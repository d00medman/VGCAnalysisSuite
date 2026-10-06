//! Who a request acts as. Spec: `specs/001-trainer-table`.
//!
//! Handlers that touch battle-side data take a `Trainer` argument; resolving it is the
//! only place that knows how a trainer is identified. Today the only resolver is the dev
//! stub, on when `DEV_AUTH=1`: the trainer comes from a `dev_trainer=<id>` cookie, else the
//! oldest admin. A cookie rather than a header, because the browser fetches `<video>` and
//! `<img>` sources itself and can't add headers to them. With the stub off every request
//! is refused (401) until real sign-in exists, and the `/api/dev` routes aren't mounted.

use crate::store::Trainer;
use crate::{ApiError, ApiResult, Shared};
use axum::async_trait;
use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{header, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

pub const DEV_COOKIE: &str = "dev_trainer";

fn unauthorized(why: &str) -> ApiError {
    ApiError(StatusCode::UNAUTHORIZED, why.into())
}

/// The value of cookie `name` in the request, if present.
fn cookie<'a>(parts: &'a Parts, name: &str) -> Option<&'a str> {
    parts
        .headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

#[async_trait]
impl FromRequestParts<Shared> for Trainer {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, s: &Shared) -> ApiResult<Trainer> {
        if !s.dev_auth {
            return Err(unauthorized("sign-in required"));
        }
        let found = match cookie(parts, DEV_COOKIE) {
            // A stale or malformed cookie is refused, never replaced by someone else.
            Some(v) => match v.parse::<i64>() {
                Ok(id) => s.store.trainer(id).await?,
                Err(_) => None,
            },
            None => s.store.default_trainer().await?,
        };
        found.ok_or_else(|| unauthorized("unknown trainer"))
    }
}

/// Routes that need a trainer but no battle data, plus the dev stub's routes when it's on.
pub fn routes(dev_auth: bool) -> Router<Shared> {
    let r = Router::new().route("/api/me", get(me));
    if dev_auth {
        r.route("/api/dev/trainers", get(list_trainers).post(create_trainer))
    } else {
        r
    }
}

async fn me(trainer: Trainer) -> Json<Trainer> {
    Json(trainer)
}

async fn list_trainers(State(s): State<Shared>) -> ApiResult<Json<Vec<Trainer>>> {
    Ok(Json(s.store.trainers().await?))
}

#[derive(Deserialize)]
struct NewTrainer {
    display_name: String,
    role: Option<String>,
}

async fn create_trainer(
    State(s): State<Shared>,
    Json(t): Json<NewTrainer>,
) -> ApiResult<(StatusCode, Json<Trainer>)> {
    let name = t.display_name.trim();
    if name.is_empty() || name.chars().count() > 40 {
        return Err(ApiError(StatusCode::BAD_REQUEST, "display name must be 1-40 characters".into()));
    }
    let role = t.role.as_deref().unwrap_or("user");
    if !matches!(role, "user" | "admin") {
        return Err(ApiError(StatusCode::BAD_REQUEST, "role must be user or admin".into()));
    }
    Ok((StatusCode::CREATED, Json(s.store.create_trainer(name, role).await?)))
}
