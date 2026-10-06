//! API tests against a real Postgres, driving the router without a socket.
//!
//! Like the pokedex's integration tests, each test migrates its own throwaway schema, so
//! tests are independent and run in parallel. Needs a running Postgres:
//! `docker compose up -d postgres` from the repo root, or point
//! `POKEDEX_TEST_DATABASE_URL` elsewhere. No video files or ffmpeg are involved: rows are
//! inserted through the store.

use super::*;
use axum::http::Request as HttpRequest;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};

const DEFAULT_URL: &str = "postgres://pokedex:pokedex@127.0.0.1:5432/pokedex";

struct TestApp {
    app: Router,
    store: Store,
    base_url: String,
    schema: String,
    video_dir: PathBuf,
}

impl TestApp {
    async fn new(dev_auth: bool) -> TestApp {
        static N: AtomicUsize = AtomicUsize::new(0);
        let base_url = std::env::var("POKEDEX_TEST_DATABASE_URL").unwrap_or(DEFAULT_URL.into());
        let schema = format!("srvtest_{}_{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed));

        let (client, conn) = tokio_postgres::connect(&base_url, tokio_postgres::NoTls)
            .await
            .unwrap_or_else(|e| {
                panic!("connecting to {base_url}: {e}\n(start it with `docker compose up -d postgres`)")
            });
        tokio::spawn(conn);
        client.batch_execute(&format!("CREATE SCHEMA {schema}")).await.unwrap();

        let url = format!("{base_url}?options=-c%20search_path%3D{schema}");
        let store = Store::connect(&url).await.unwrap();
        let video_dir = std::env::temp_dir().join(&schema);
        std::fs::create_dir_all(&video_dir).unwrap();
        let atlas = Arc::new(Atlas::load(std::path::Path::new(analyzer::DEFAULT_ATLAS)).unwrap());
        let state = Arc::new(AppState::new(
            store.clone(),
            atlas,
            PathBuf::from("/nonexistent/ffmpeg"),
            video_dir.clone(),
            dev_auth,
        ));
        TestApp { app: app(state), store, base_url, schema, video_dir }
    }

    /// Send a request, as `trainer` (the dev cookie) if given. Returns the status and the
    /// body parsed as JSON, or `Null` if it isn't JSON.
    async fn call(&self, method: &str, uri: &str, trainer: Option<&str>, body: Body) -> (StatusCode, Value) {
        let mut req = HttpRequest::builder().method(method).uri(uri);
        if let Some(t) = trainer {
            req = req.header(header::COOKIE, format!("theme=dark; {}={t}", auth::DEV_COOKIE));
        }
        if method == "POST" || method == "PUT" {
            req = req.header(header::CONTENT_TYPE, "application/json");
        }
        let res = self.app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    async fn get(&self, uri: &str, trainer: Option<&str>) -> (StatusCode, Value) {
        self.call("GET", uri, trainer, Body::empty()).await
    }

    /// A video owned by `trainer`, with a finished transcript of two lines. Returns the
    /// first line's id.
    async fn video(&self, id: &str, trainer: i64) -> i64 {
        let v = Video {
            id: id.into(),
            name: format!("{id}.MP4"),
            file: format!("{id}.mp4"),
            size: 100,
            uploaded_at: now_ms(),
            status: Status::Uploaded,
            started_at: None,
            finished_at: None,
            message_count: None,
            error: None,
            has_preview: false,
        };
        self.store.create(&v, trainer).await.unwrap();
        std::fs::write(self.video_dir.join(&v.file), b"not a video").unwrap();
        let run = self.store.queue(id, trainer).await.unwrap().unwrap();
        let line = |t: f64, text: &str| Message { t0: t, t1: t + 1.0, text: text.into(), conf: 0.9, clean: true };
        self.store.finish(run, &[line(0.0, "Go! Pikachu!"), line(2.0, "Pikachu used Thunderbolt!")]).await.unwrap();
        let lines = self.store.transcript(id, trainer).await.unwrap().unwrap();
        lines[0]["id"].as_i64().unwrap()
    }
}

impl Drop for TestApp {
    fn drop(&mut self) {
        let (url, schema) = (self.base_url.clone(), self.schema.clone());
        // Sync client on its own thread: Drop can't await, and this may run inside the runtime.
        let _ = std::thread::spawn(move || {
            if let Ok(mut db) = pokedex::Db::connect(&url) {
                let _ = db.client().batch_execute(&format!("DROP SCHEMA {schema} CASCADE"));
            }
        })
        .join();
        let _ = std::fs::remove_dir_all(&self.video_dir);
    }
}

#[tokio::test]
async fn health_answers() {
    let t = TestApp::new(false).await;
    let res = t.app.clone().oneshot(HttpRequest::get("/api/health").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

/// Criteria 5 and 8: with the stub off there is no way in, and the pokedex stays public.
#[tokio::test]
async fn without_dev_auth_battle_routes_refuse_and_pokedex_is_public() {
    let t = TestApp::new(false).await;
    let admin = t.store.create_trainer("Ash", "admin").await.unwrap();
    t.video("v1", admin.id).await;
    let id = admin.id.to_string();

    for (uri, trainer) in [("/api/videos", None), ("/api/videos/v1", Some(id.as_str())), ("/api/me", None)] {
        assert_eq!(t.get(uri, trainer).await.0, StatusCode::UNAUTHORIZED, "{uri}");
    }
    assert_eq!(t.get("/api/dev/trainers", None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(t.get("/api/pokedex/regulations", None).await.0, StatusCode::OK);
}

/// Criterion 5, with the stub on: the pokedex needs no trainer, and ignores a bad cookie.
#[tokio::test]
async fn pokedex_ignores_the_trainer() {
    let t = TestApp::new(true).await;
    assert_eq!(t.get("/api/pokedex/regulations", None).await.0, StatusCode::OK);
    assert_eq!(t.get("/api/pokedex/regulations", Some("9999")).await.0, StatusCode::OK);
}

/// Criterion 4: every battle-side route treats another trainer's video as missing.
#[tokio::test]
async fn trainers_cannot_see_or_touch_each_others_battles() {
    let t = TestApp::new(true).await;
    let a = t.store.create_trainer("Ash", "admin").await.unwrap().id;
    let b = t.store.create_trainer("Misty", "user").await.unwrap().id;
    t.video("va", a).await;
    let b_line = t.video("vb", b).await;
    let (a, b) = (a.to_string(), b.to_string());
    let (a, b) = (Some(a.as_str()), Some(b.as_str()));

    let (status, list) = t.get("/api/videos", a).await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<&str> = list.as_array().unwrap().iter().map(|v| v["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["va"]);

    for uri in ["/api/videos/vb", "/api/videos/vb/file", "/api/videos/vb/preview", "/api/videos/vb/live.jpg"] {
        assert_eq!(t.get(uri, a).await.0, StatusCode::NOT_FOUND, "{uri}");
    }
    let (status, _) = t.call("POST", "/api/videos/vb/transcribe", a, Body::empty()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let mark = || Body::from(json!({ "ends_turn": true }).to_string());
    let turn_end = format!("/api/videos/vb/lines/{b_line}/turn-end");
    assert_eq!(t.call("PUT", &turn_end, a, mark()).await.0, StatusCode::NOT_FOUND);

    // Nothing above changed B's battle, and B still has full access to it.
    let (status, detail) = t.get("/api/videos/vb", b).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["video"]["status"], "done");
    assert_eq!(detail["messages"][0]["ends_turn"], false);
    assert_eq!(t.get("/api/videos/vb/file", b).await.0, StatusCode::OK);
    assert_eq!(t.call("PUT", &turn_end, b, mark()).await.0, StatusCode::NO_CONTENT);
}

/// Criterion 3: an upload belongs to whoever uploaded it.
#[tokio::test]
async fn uploads_belong_to_the_uploader() {
    let t = TestApp::new(true).await;
    let a = t.store.create_trainer("Ash", "admin").await.unwrap().id;
    let b = t.store.create_trainer("Misty", "user").await.unwrap().id;

    let (status, v) = t.call("POST", "/api/videos?name=clip.MP4", Some(&b.to_string()), Body::from("bytes")).await;
    assert_eq!(status, StatusCode::CREATED);
    let id = v["id"].as_str().unwrap();
    assert!(t.store.get(id, b).await.unwrap().is_some());
    assert!(t.store.get(id, a).await.unwrap().is_none());
}

/// The stub: who a request acts as, and the dev routes. Covers criterion 9's defaults.
#[tokio::test]
async fn dev_stub_picks_the_trainer_from_the_cookie() {
    let t = TestApp::new(true).await;

    // A fresh database has no trainers; startup gives the stub an admin to act as.
    assert_eq!(t.get("/api/me", None).await.0, StatusCode::UNAUTHORIZED);
    t.store.ensure_dev_trainer().await.unwrap();
    t.store.ensure_dev_trainer().await.unwrap();
    let (status, me) = t.get("/api/me", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!((me["display_name"].as_str(), me["role"].as_str()), (Some("Dev"), Some("admin")));

    let new = |body: Value| Body::from(body.to_string());
    let (status, misty) = t.call("POST", "/api/dev/trainers", None, new(json!({ "display_name": " Misty " }))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!((misty["display_name"].as_str(), misty["role"].as_str()), (Some("Misty"), Some("user")));
    for bad in [json!({ "display_name": "Brock", "role": "owner" }), json!({ "display_name": "  " })] {
        assert_eq!(t.call("POST", "/api/dev/trainers", None, new(bad)).await.0, StatusCode::BAD_REQUEST);
    }
    let (_, all) = t.get("/api/dev/trainers", None).await;
    assert_eq!(all.as_array().unwrap().len(), 2);

    let misty_id = misty["id"].to_string();
    assert_eq!(t.get("/api/me", Some(&misty_id)).await.1["display_name"], "Misty");
    // A stale or garbled cookie is refused, never swapped for the default trainer.
    assert_eq!(t.get("/api/me", Some("9999")).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(t.get("/api/me", Some("abc")).await.0, StatusCode::UNAUTHORIZED);
}
