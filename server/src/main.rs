//! HTTP API over the analyzer: upload a recording, transcribe it, browse past transcripts.
//!
//! Videos live on disk under `VIDEO_DIR`; metadata and transcripts live in Postgres (`store`).
//! Transcriptions run one at a time — each decode already uses every core — so extra
//! requests wait as `queued`. Live progress for the running job is held in memory and
//! served from `GET /api/videos/:id`; only finished transcripts are persisted.
//!
//! Playback: recordings are HEVC, which most desktop browsers cannot play, so each
//! transcription also writes an H.264 preview (`<id>.preview.mp4`) from the same decode, and
//! a live snapshot of the frame being read (`<id>.live.jpg`) while it runs. Browsers that can
//! play HEVC use the original directly. All three are served with HTTP Range support so the
//! player can seek.

mod dex;
mod store;

use analyzer::atlas::Atlas;
use analyzer::episode::Message;
use analyzer::decode::Extras;
use analyzer::transcribe::{transcribe_with, Status as Progress};
use anyhow::Context;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::extract::Request;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use store::{now_ms, Status, Store, Video};
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;
use tower::ServiceExt;
use tower_http::services::ServeFile;

/// Live state of a queued or running transcription.
#[derive(Clone, Default, Serialize)]
struct Live {
    progress: Option<Progress>,
    fraction: Option<f64>,
    /// Wall-clock seconds since decoding started.
    elapsed: f64,
    messages: Vec<Message>,
}

struct AppState {
    store: Store,
    atlas: Arc<Atlas>,
    ffmpeg: PathBuf,
    video_dir: PathBuf,
    live: Mutex<HashMap<String, Arc<Mutex<Live>>>>,
    /// One transcription at a time.
    worker: Semaphore,
}

type Shared = Arc<AppState>;

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        eprintln!("error: {e:#}");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
    }
}

type ApiResult<T> = Result<T, ApiError>;

fn not_found(id: &str) -> ApiError {
    ApiError(StatusCode::NOT_FOUND, format!("no video {id}"))
}

fn env_or(key: &str, default: impl Into<String>) -> String {
    std::env::var(key).unwrap_or_else(|_| default.into())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let bind = env_or("BIND", "0.0.0.0:8080");
    let database_url = env_or("DATABASE_URL", "postgres://pokedex:pokedex@127.0.0.1:5432/pokedex");
    // Default is anchored to this crate's directory, not the working directory, so a local
    // run behaves the same wherever it is started from. The container sets VIDEO_DIR.
    let video_dir = PathBuf::from(env_or("VIDEO_DIR", concat!(env!("CARGO_MANIFEST_DIR"), "/data/videos")));
    let ffmpeg = std::env::var_os("ANALYZER_FFMPEG")
        .map(PathBuf::from)
        .unwrap_or_else(|| analyzer::decode::ffmpeg_path(None));
    let atlas_path = PathBuf::from(env_or("ANALYZER_ATLAS", analyzer::DEFAULT_ATLAS));

    tokio::fs::create_dir_all(&video_dir)
        .await
        .with_context(|| format!("creating {}", video_dir.display()))?;
    let atlas = Arc::new(Atlas::load(&atlas_path)?);
    let store = Store::connect(&database_url).await?;

    // A restart kills any in-flight decode; say so rather than leaving it "transcribing".
    store.fail_interrupted("interrupted: server restarted during transcription").await?;

    eprintln!("atlas  {} ({} templates)", atlas_path.display(), atlas.templates.len());
    eprintln!("ffmpeg {}", ffmpeg.display());
    eprintln!("videos {}", video_dir.display());

    let state = Arc::new(AppState {
        store,
        atlas,
        ffmpeg,
        video_dir,
        live: Mutex::new(HashMap::new()),
        worker: Semaphore::new(1),
    });

    let app = Router::new()
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/videos", get(list_videos))
        .route(
            "/api/videos",
            // Recordings are ~1 GB; the body is streamed to disk, never buffered.
            post(upload_video).layer(DefaultBodyLimit::disable()),
        )
        .route("/api/videos/:id", get(get_video))
        .route("/api/videos/:id/transcribe", post(start_transcription))
        .route("/api/videos/:id/file", get(serve_original))
        .route("/api/videos/:id/preview", get(serve_preview))
        .route("/api/videos/:id/live.jpg", get(serve_snapshot))
        .route("/api/videos/:id/lines/:line/turn-end", put(set_turn_end))
        .merge(dex::routes())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    eprintln!("listening on {bind}");
    axum::serve(listener, app).await?;
    Ok(())
}

fn preview_path(s: &AppState, id: &str) -> PathBuf {
    s.video_dir.join(format!("{id}.preview.mp4"))
}

/// Where ffmpeg writes the preview during a run; renamed to `preview_path` only on success,
/// so a half-written preview is never served.
fn preview_part_path(s: &AppState, id: &str) -> PathBuf {
    s.video_dir.join(format!("{id}.preview.part.mp4"))
}

fn snapshot_path(s: &AppState, id: &str) -> PathBuf {
    s.video_dir.join(format!("{id}.live.jpg"))
}

async fn annotate(s: &AppState, mut v: Video) -> Video {
    v.has_preview = tokio::fs::try_exists(preview_path(s, &v.id)).await.unwrap_or(false);
    v
}

/// Look up a video by the id in the URL. Paths on disk are only ever built from ids that
/// exist in the database, never from the raw URL segment.
async fn lookup(s: &AppState, id: &str) -> ApiResult<Video> {
    Ok(s.store.get(id).await?.ok_or_else(|| not_found(id))?)
}

async fn list_videos(State(s): State<Shared>) -> ApiResult<Json<Vec<Video>>> {
    let mut out = Vec::new();
    for v in s.store.list().await? {
        out.push(annotate(&s, v).await);
    }
    Ok(Json(out))
}

/// Serve a file with Range support (seeking), or 404 if it does not exist yet.
async fn serve_file(path: PathBuf, req: Request, no_store: bool) -> ApiResult<Response> {
    if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return Err(ApiError(StatusCode::NOT_FOUND, "not available".into()));
    }
    let res = ServeFile::new(&path)
        .oneshot(req)
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let mut res = res.map(Body::new);
    if no_store {
        res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    Ok(res)
}

async fn serve_original(State(s): State<Shared>, Path(id): Path<String>, req: Request) -> ApiResult<Response> {
    let v = lookup(&s, &id).await?;
    serve_file(s.video_dir.join(&v.file), req, false).await
}

async fn serve_preview(State(s): State<Shared>, Path(id): Path<String>, req: Request) -> ApiResult<Response> {
    let v = lookup(&s, &id).await?;
    serve_file(preview_path(&s, &v.id), req, false).await
}

async fn serve_snapshot(State(s): State<Shared>, Path(id): Path<String>, req: Request) -> ApiResult<Response> {
    let v = lookup(&s, &id).await?;
    serve_file(snapshot_path(&s, &v.id), req, true).await
}

#[derive(Deserialize)]
struct UploadQuery {
    name: String,
}

/// `POST /api/videos?name=<filename>` with the raw file as the request body.
async fn upload_video(
    State(s): State<Shared>,
    Query(q): Query<UploadQuery>,
    body: Body,
) -> ApiResult<(StatusCode, Json<Video>)> {
    let name = q.name.trim().chars().take(255).collect::<String>();
    if name.is_empty() {
        return Err(ApiError(StatusCode::BAD_REQUEST, "missing ?name=".into()));
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    // The on-disk name never comes from user input beyond a short alphanumeric extension.
    let ext = std::path::Path::new(&name)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| !e.is_empty() && e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_else(|| "bin".into());
    let file = format!("{id}.{ext}");
    let part = s.video_dir.join(format!("{file}.part"));

    let size = match write_body(&part, body).await {
        Ok(n) => n,
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(ApiError(StatusCode::BAD_REQUEST, format!("upload failed: {e:#}")));
        }
    };
    if size == 0 {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(ApiError(StatusCode::BAD_REQUEST, "empty upload".into()));
    }
    tokio::fs::rename(&part, s.video_dir.join(&file))
        .await
        .context("finalising upload")?;

    let video = Video {
        id,
        name,
        file,
        size,
        uploaded_at: now_ms(),
        status: Status::Uploaded,
        started_at: None,
        finished_at: None,
        message_count: None,
        error: None,
        has_preview: false,
    };
    s.store.create(&video).await?;
    eprintln!("uploaded {} ({} bytes) as {}", video.name, video.size, video.id);
    Ok((StatusCode::CREATED, Json(video)))
}

async fn write_body(path: &std::path::Path, body: Body) -> anyhow::Result<u64> {
    let mut out = tokio::fs::File::create(path).await?;
    let mut stream = body.into_data_stream();
    let mut size = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        size += chunk.len() as u64;
        out.write_all(&chunk).await?;
    }
    out.flush().await?;
    Ok(size)
}

#[derive(Serialize)]
struct VideoDetail {
    video: Video,
    /// Present while queued or transcribing.
    live: Option<Live>,
    /// Messages so far while running; the saved transcript once done.
    messages: serde_json::Value,
}

async fn get_video(State(s): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<VideoDetail>> {
    let video = annotate(&s, lookup(&s, &id).await?).await;
    let live = s.live.lock().unwrap().get(&id).map(|l| l.lock().unwrap().clone());
    let messages = match (&live, video.status) {
        (Some(l), _) => serde_json::to_value(&l.messages).context("encoding messages")?,
        (None, Status::Done) => s.store.transcript(&id).await?.unwrap_or_default(),
        _ => serde_json::Value::Array(vec![]),
    };
    Ok(Json(VideoDetail { video, live, messages }))
}

#[derive(Deserialize)]
struct TurnEnd {
    ends_turn: bool,
}

/// `PUT /api/videos/:id/lines/:line/turn-end` with `{"ends_turn": bool}`: mark or unmark a
/// transcript line as the last line of its turn.
async fn set_turn_end(
    State(s): State<Shared>,
    Path((id, line)): Path<(String, i64)>,
    Json(body): Json<TurnEnd>,
) -> ApiResult<StatusCode> {
    if s.store.set_turn_end(&id, line, body.ends_turn).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError(StatusCode::NOT_FOUND, format!("no line {line} in video {id}")))
    }
}

async fn start_transcription(
    State(s): State<Shared>,
    Path(id): Path<String>,
) -> ApiResult<(StatusCode, Json<Video>)> {
    let video = s.store.get(&id).await?.ok_or_else(|| not_found(&id))?;
    let live = Arc::new(Mutex::new(Live::default()));
    {
        let mut map = s.live.lock().unwrap();
        if map.contains_key(&id) {
            return Err(ApiError(StatusCode::CONFLICT, "already transcribing".into()));
        }
        map.insert(id.clone(), Arc::clone(&live));
    }
    let transcript_id = match s.store.queue(&id).await {
        Ok(t) => t,
        Err(e) => {
            s.live.lock().unwrap().remove(&id);
            return Err(e.into());
        }
    };
    tokio::spawn(run_job(Arc::clone(&s), video.clone(), transcript_id, live));
    let video = s.store.get(&id).await?.ok_or_else(|| not_found(&id))?;
    Ok((StatusCode::ACCEPTED, Json(video)))
}

async fn run_job(s: Shared, video: Video, transcript_id: i64, live: Arc<Mutex<Live>>) {
    let id = video.id.clone();
    let result = async {
        let _permit = s.worker.acquire().await.context("worker closed")?;
        s.store.start(transcript_id).await?;
        eprintln!("transcribing {} ({})", video.name, id);
        let (atlas, ffmpeg, path) = (Arc::clone(&s.atlas), s.ffmpeg.clone(), s.video_dir.join(&video.file));
        let extras = Extras {
            preview: Some(preview_part_path(&s, &id)),
            snapshot: Some(snapshot_path(&s, &id)),
        };
        let live_w = Arc::clone(&live);
        let started = std::time::Instant::now();
        tokio::task::spawn_blocking(move || {
            let live_m = Arc::clone(&live_w);
            transcribe_with(
                &ffmpeg,
                &atlas,
                &path,
                None,
                None,
                &extras,
                |p| {
                    let mut l = live_w.lock().unwrap();
                    l.progress = Some(*p);
                    l.fraction = p.fraction();
                    l.elapsed = started.elapsed().as_secs_f64();
                },
                |m| live_m.lock().unwrap().messages.push(m.clone()),
            )
        })
        .await
        .context("transcription task panicked")?
    }
    .await;

    // The preview is complete only if ffmpeg exited cleanly, which `result` reflects.
    let part = preview_part_path(&s, &id);
    if result.is_ok() {
        if let Err(e) = tokio::fs::rename(&part, preview_path(&s, &id)).await {
            eprintln!("preview for {id} not kept: {e}");
        }
    } else {
        let _ = tokio::fs::remove_file(&part).await;
    }
    let _ = tokio::fs::remove_file(snapshot_path(&s, &id)).await;

    let saved = match &result {
        Ok(messages) => {
            eprintln!("done {} ({}): {} messages", video.name, id, messages.len());
            s.store.finish(transcript_id, messages).await
        }
        Err(e) => {
            eprintln!("failed {} ({}): {e:#}", video.name, id);
            s.store.fail(transcript_id, &format!("{e:#}")).await
        }
    };
    if let Err(e) = saved {
        eprintln!("could not save result for {id}: {e:#}");
    }
    s.live.lock().unwrap().remove(&id);
}
