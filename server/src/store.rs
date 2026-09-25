//! Postgres persistence for uploaded videos, their battles, and transcripts.
//!
//! Schema: `pokedex/migrations/0005_battles.sql`. One `video` per upload, one `battle`
//! per video, one `transcript` row per transcription run (re-runs add rows; the newest
//! is current), and one `transcript_line` row per message. The API still speaks in
//! videos: a video's status is its newest run's status.

use analyzer::episode::Message;
use anyhow::{Context, Result};
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use serde::Serialize;
use tokio_postgres::{NoTls, Row};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Uploaded,
    Queued,
    Transcribing,
    Done,
    Failed,
}

impl Status {
    /// `None` (no run yet) is `uploaded`.
    fn parse(s: Option<&str>) -> Status {
        match s {
            Some("queued") => Status::Queued,
            Some("transcribing") => Status::Transcribing,
            Some("done") => Status::Done,
            Some("failed") => Status::Failed,
            _ => Status::Uploaded,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Video {
    pub id: String,
    /// Original filename as uploaded.
    pub name: String,
    /// File name on the video volume.
    #[serde(skip)]
    pub file: String,
    pub size: u64,
    /// Unix milliseconds.
    pub uploaded_at: i64,
    pub status: Status,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    /// Lines in the newest finished transcript.
    pub message_count: Option<usize>,
    pub error: Option<String>,
    /// Whether a browser-playable preview exists. Not stored: derived from the video volume
    /// by the API when it serves the record.
    pub has_preview: bool,
}

/// Every column `Video` needs, with timestamps as unix milliseconds. `t` is the newest
/// run of any status; `d` the newest finished one, for the line count.
const VIDEO_SELECT: &str = "
    SELECT v.id, v.name, v.file, v.size_bytes,
           (extract(epoch FROM v.uploaded_at) * 1000)::bigint,
           t.status,
           (extract(epoch FROM t.started_at) * 1000)::bigint,
           (extract(epoch FROM t.finished_at) * 1000)::bigint,
           t.error,
           d.lines
    FROM video v
    JOIN battle b ON b.video_id = v.id
    LEFT JOIN LATERAL (
        SELECT * FROM transcript WHERE battle_id = b.id ORDER BY id DESC LIMIT 1
    ) t ON true
    LEFT JOIN LATERAL (
        SELECT (SELECT count(*) FROM transcript_line l WHERE l.transcript_id = x.id) AS lines
        FROM transcript x WHERE x.battle_id = b.id AND x.status = 'done'
        ORDER BY x.id DESC LIMIT 1
    ) d ON true";

impl Video {
    fn from_row(r: &Row) -> Video {
        Video {
            id: r.get(0),
            name: r.get(1),
            file: r.get(2),
            size: r.get::<_, i64>(3) as u64,
            uploaded_at: r.get(4),
            status: Status::parse(r.get(5)),
            started_at: r.get(6),
            finished_at: r.get(7),
            error: r.get(8),
            message_count: r.get::<_, Option<i64>>(9).map(|n| n as usize),
            has_preview: false,
        }
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Clone)]
pub struct Store {
    pool: Pool,
}

impl Store {
    /// Connect, and apply any pending schema migrations first. Migrations are shared with
    /// the pokedex CLI and serialized by an advisory lock, so both may start at once.
    pub async fn connect(url: &str) -> Result<Store> {
        let owned = url.to_string();
        let applied = tokio::task::spawn_blocking(move || {
            pokedex::Db::connect(&owned).and_then(|mut db| db.migrate())
        })
        .await
        .context("migration task panicked")?
        .context("migrating the database")?;
        for name in applied {
            eprintln!("applied migration {name}");
        }

        let config: tokio_postgres::Config = url.parse().context("parsing DATABASE_URL")?;
        let manager = Manager::from_config(
            config,
            NoTls,
            ManagerConfig { recycling_method: RecyclingMethod::Fast },
        );
        let pool = Pool::builder(manager).max_size(8).build().context("building pool")?;
        // Fail at startup, not on the first request.
        let _ = pool.get().await.context("connecting to postgres")?;
        Ok(Store { pool })
    }

    async fn client(&self) -> Result<deadpool_postgres::Object> {
        self.pool.get().await.context("postgres connection")
    }

    /// Run a query whose single row and column is a JSON document; `None` if no row.
    pub async fn query_json(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> Result<Option<serde_json::Value>> {
        let c = self.client().await?;
        Ok(c.query_opt(sql, params).await?.map(|r| r.get(0)))
    }

    /// Record an upload: its video row and the battle it holds.
    pub async fn create(&self, v: &Video) -> Result<()> {
        let mut c = self.client().await?;
        let tx = c.transaction().await?;
        tx.execute(
            "INSERT INTO video (id, name, file, size_bytes, uploaded_at)
             VALUES ($1, $2, $3, $4, to_timestamp($5::bigint / 1000.0))",
            &[&v.id, &v.name, &v.file, &(v.size as i64), &v.uploaded_at],
        )
        .await?;
        tx.execute("INSERT INTO battle (video_id) VALUES ($1)", &[&v.id]).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn get(&self, id: &str) -> Result<Option<Video>> {
        let c = self.client().await?;
        let sql = format!("{VIDEO_SELECT} WHERE v.id = $1");
        Ok(c.query_opt(&sql, &[&id]).await?.as_ref().map(Video::from_row))
    }

    /// Every video, newest first.
    pub async fn list(&self) -> Result<Vec<Video>> {
        let c = self.client().await?;
        let sql = format!("{VIDEO_SELECT} ORDER BY v.uploaded_at DESC");
        Ok(c.query(&sql, &[]).await?.iter().map(Video::from_row).collect())
    }

    /// Start a new transcription run for a video, queued. Returns the run's id.
    pub async fn queue(&self, video_id: &str) -> Result<i64> {
        let c = self.client().await?;
        let row = c
            .query_one(
                "INSERT INTO transcript (battle_id)
                 SELECT id FROM battle WHERE video_id = $1 RETURNING id",
                &[&video_id],
            )
            .await?;
        Ok(row.get(0))
    }

    pub async fn start(&self, transcript_id: i64) -> Result<()> {
        let c = self.client().await?;
        c.execute(
            "UPDATE transcript SET status = 'transcribing', started_at = now() WHERE id = $1",
            &[&transcript_id],
        )
        .await?;
        Ok(())
    }

    /// Save a run's lines and mark it done, atomically.
    pub async fn finish(&self, transcript_id: i64, messages: &[Message]) -> Result<()> {
        let seq: Vec<i32> = (0..messages.len() as i32).collect();
        let t0: Vec<f64> = messages.iter().map(|m| m.t0).collect();
        let t1: Vec<f64> = messages.iter().map(|m| m.t1).collect();
        let text: Vec<&str> = messages.iter().map(|m| m.text.as_str()).collect();
        let conf: Vec<f32> = messages.iter().map(|m| m.conf).collect();
        let clean: Vec<bool> = messages.iter().map(|m| m.clean).collect();

        let mut c = self.client().await?;
        let tx = c.transaction().await?;
        tx.execute(
            "INSERT INTO transcript_line (transcript_id, seq, t0, t1, text, conf, clean)
             SELECT $1, * FROM unnest($2::int[], $3::float8[], $4::float8[], $5::text[],
                                      $6::real[], $7::bool[])",
            &[&transcript_id, &seq, &t0, &t1, &text, &conf, &clean],
        )
        .await?;
        tx.execute(
            "UPDATE transcript SET status = 'done', finished_at = now() WHERE id = $1",
            &[&transcript_id],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn fail(&self, transcript_id: i64, error: &str) -> Result<()> {
        let c = self.client().await?;
        c.execute(
            "UPDATE transcript SET status = 'failed', finished_at = now(), error = $2
             WHERE id = $1",
            &[&transcript_id, &error],
        )
        .await?;
        Ok(())
    }

    /// Fail every run left queued or transcribing, e.g. by a restart. Returns how many.
    pub async fn fail_interrupted(&self, error: &str) -> Result<u64> {
        let c = self.client().await?;
        Ok(c.execute(
            "UPDATE transcript SET status = 'failed', finished_at = now(), error = $1
             WHERE status IN ('queued', 'transcribing')",
            &[&error],
        )
        .await?)
    }

    /// The newest finished transcript's lines, in order, shaped like `Message`.
    pub async fn transcript(&self, video_id: &str) -> Result<Option<serde_json::Value>> {
        let c = self.client().await?;
        let row = c
            .query_opt(
                "SELECT coalesce(json_agg(json_build_object(
                          't0', l.t0, 't1', l.t1, 'text', l.text,
                          'conf', l.conf, 'clean', l.clean) ORDER BY l.seq)
                          FILTER (WHERE l.id IS NOT NULL), '[]')
                 FROM (SELECT t.id FROM transcript t JOIN battle b ON b.id = t.battle_id
                       WHERE b.video_id = $1 AND t.status = 'done'
                       ORDER BY t.id DESC LIMIT 1) latest
                 LEFT JOIN transcript_line l ON l.transcript_id = latest.id
                 GROUP BY latest.id",
                &[&video_id],
            )
            .await?;
        Ok(row.map(|r| r.get(0)))
    }
}
