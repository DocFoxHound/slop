// SPDX-License-Identifier: AGPL-3.0-or-later
//! Save sync (plan phase 15): a player's saves follow them between their computers.
//!
//! - **End-to-end encrypted.** The Slop app packs and encrypts an app's save folder with a key
//!   only the player's devices have (derived from their chat backup key, slop-client
//!   `save_sync.rs`); Slop stores and returns ciphertext it can't read.
//! - **Limits:** 20 MB per app, 200 MB per player across apps (the newest of each), and the
//!   newest plus the one before are kept, so a bad save can be undone once.
//! - **No silent overwrites:** an upload names the generation it builds on; if another device
//!   uploaded since, it's refused (409) and the Slop app keeps both copies for the player.
//! - Only the player's own sessions use these routes; makers' agent tokens can't.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
};

/// Largest encrypted save per app (20 MB, plus room for the encryption's own bytes).
pub const MAX_SAVE_BYTES: usize = 20 * 1024 * 1024 + 1024;
/// All of a player's newest saves together.
pub const MAX_PLAYER_BYTES: i64 = 200 * 1024 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/saves", get(list))
        .route("/me/saves/{slug}", get(download).put(upload).delete(forget))
}

#[derive(Serialize, sqlx::FromRow)]
struct Saved {
    app_slug: String,
    generation: i32,
    size: i64,
    device: String,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

/// The player's newest save for each app, with how much of their allowance they use.
async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<serde_json::Value>> {
    let saves: Vec<Saved> = sqlx::query_as(
        "select distinct on (s.app_id) a.slug as app_slug, s.generation, length(s.body)::int8 as size, s.device, s.created_at
         from save_blobs s join apps a on a.id = s.app_id where s.user_id = $1
         order by s.app_id, s.generation desc",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    let used: i64 = saves.iter().map(|s| s.size).sum();
    Ok(Json(
        serde_json::json!({ "saves": saves, "used": used, "allowance": MAX_PLAYER_BYTES }),
    ))
}

async fn app_id(state: &AppState, slug: &str) -> ApiResult<Uuid> {
    sqlx::query_scalar("select id from apps where slug = $1")
        .bind(slug)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)
}

#[derive(Deserialize)]
struct Which {
    /// An older generation (the one before the newest); the newest when absent.
    generation: Option<i32>,
}

/// The encrypted save, with its generation and device in headers.
async fn download(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<Which>,
) -> ApiResult<Response> {
    let app = app_id(&state, &slug).await?;
    let row: Option<(i32, String, Vec<u8>)> = sqlx::query_as(
        "select generation, device, body from save_blobs where user_id = $1 and app_id = $2
           and ($3::int4 is null or generation = $3)
         order by generation desc limit 1",
    )
    .bind(user.id)
    .bind(app)
    .bind(q.generation)
    .fetch_optional(&state.db)
    .await?;
    let (generation, device, body) = row.ok_or(ApiError::NotFound)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (
                header::HeaderName::from_static("x-slop-generation"),
                generation.to_string(),
            ),
            (header::HeaderName::from_static("x-slop-device"), device),
        ],
        body,
    )
        .into_response())
}

/// Stores a new generation on top of `x-slop-base` (0 for the first). Refused with 409 when
/// another device uploaded since.
async fn upload(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
    body: Bytes,
) -> ApiResult<Json<serde_json::Value>> {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
    };
    let base: i32 = header("x-slop-base")
        .and_then(|v| v.parse().ok())
        .filter(|b| *b >= 0)
        .ok_or_else(|| {
            ApiError::invalid(
                "x-slop-base",
                "the generation this save builds on (0 for the first)",
            )
        })?;
    let device = header("x-slop-device")
        .filter(|d| {
            !d.is_empty()
                && d.len() <= 64
                && d.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        .ok_or_else(|| ApiError::invalid("x-slop-device", "this device's name"))?
        .to_string();
    if body.is_empty() || body.len() > MAX_SAVE_BYTES {
        return Err(ApiError::invalid(
            "body",
            "an encrypted save of up to 20 MB",
        ));
    }
    // Only for an app the player can see: a private or hidden one answers like a missing one.
    let policy =
        crate::policy::ViewerPolicy::resolve(&state.db, &state.config, Some(&user), &headers)
            .await?;
    let (app, _) = crate::catalog::visible_app(&state, &slug, Some(&user), policy).await?;
    let mut tx = state.db.begin().await?;
    // One upload at a time per player and app.
    sqlx::query("select pg_advisory_xact_lock(hashtextextended($1::text || $2::text, 0))")
        .bind(user.id)
        .bind(app)
        .execute(&mut *tx)
        .await?;
    let newest: i32 = sqlx::query_scalar(
        "select coalesce(max(generation), 0) from save_blobs where user_id = $1 and app_id = $2",
    )
    .bind(user.id)
    .bind(app)
    .fetch_one(&mut *tx)
    .await?;
    if newest != base {
        return Err(ApiError::Conflict(format!(
            "another computer saved since (generation {newest}); keep both and choose"
        )));
    }
    // Everything kept counts: every app's newest save and the copy before it (security review,
    // 2026-10-22: counting only the newest let an account keep twice its allowance).
    let others: i64 = sqlx::query_scalar(
        "select coalesce(sum(length(body)), 0)::int8 from save_blobs
         where user_id = $1 and (app_id <> $2 or generation = $3)",
    )
    .bind(user.id)
    .bind(app)
    .bind(newest)
    .fetch_one(&mut *tx)
    .await?;
    if others + body.len() as i64 > MAX_PLAYER_BYTES {
        return Err(ApiError::Conflict(
            "your synced saves (with the copy kept before each) are over 200 MB: stop syncing an app you don't play to make room"
                .into(),
        ));
    }
    let generation = newest + 1;
    sqlx::query("insert into save_blobs (user_id, app_id, generation, body, device) values ($1, $2, $3, $4, $5)")
        .bind(user.id)
        .bind(app)
        .bind(generation)
        .bind(body.as_ref())
        .bind(&device)
        .execute(&mut *tx)
        .await?;
    // Keep this one and the one before.
    sqlx::query(
        "delete from save_blobs where user_id = $1 and app_id = $2 and generation < $3 - 1",
    )
    .bind(user.id)
    .bind(app)
    .bind(generation)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(serde_json::json!({ "generation": generation })))
}

/// Stops syncing an app: its saves are deleted from Slop (the ones on the player's computers
/// stay).
async fn forget(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    // Nothing to forget answers the same as forgetting: no hint whether the app exists.
    let Ok(app) = app_id(&state, &slug).await else {
        return Ok(StatusCode::NO_CONTENT);
    };
    sqlx::query("delete from save_blobs where user_id = $1 and app_id = $2")
        .bind(user.id)
        .bind(app)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
