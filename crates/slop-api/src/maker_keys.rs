// SPDX-License-Identifier: AGPL-3.0-or-later
//! Makers' signing keys (SECURITY §5.3): for builds a maker makes and attaches by hand, outside
//! GitHub Actions (on GitLab, Bitbucket, a self-hosted server, or their own computer). They
//! register an Ed25519 SSH public key, sign each file with
//! `ssh-keygen -Y sign -f <key> -n slop-release <file>`, and attach the `.sig` beside it.
//!
//! Adding a key needs a recent sign-in or passkey (step-up), and the key only works 72 hours
//! after it's added; the maker is told straight away, so a key added by someone with a stolen
//! session can be removed before it signs anything. Removing one works at once.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get},
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::{self, CurrentUser},
    error::{ApiError, ApiResult},
};

/// How long a new key waits before it can sign.
pub const WAIT: time::Duration = time::Duration::hours(72);
const MAX_KEYS: i64 = 5;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/signing-keys", get(list).post(add))
        .route("/me/signing-keys/{id}", delete(remove))
}

#[derive(Serialize, sqlx::FromRow)]
struct KeyInfo {
    id: Uuid,
    name: String,
    fingerprint: String,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    /// When it starts working (72 hours after it was added).
    #[serde(with = "time::serde::rfc3339")]
    active_from: OffsetDateTime,
    active: bool,
}

const COLUMNS: &str =
    "id, name, fingerprint, created_at, active_from, active_from <= now() as active";

async fn list(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Vec<KeyInfo>>> {
    Ok(Json(
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select {COLUMNS} from maker_keys where user_id = $1 and revoked_at is null order by created_at"
        )))
        .bind(user.id)
        .fetch_all(&state.db)
        .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewKey {
    name: String,
    /// The `.pub` file's line: `ssh-ed25519 AAAA… comment`.
    public_key: String,
}

async fn add(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(b): Json<NewKey>,
) -> ApiResult<(StatusCode, Json<KeyInfo>)> {
    session::require_recent_sign_in(&state.db, &user).await?;
    let name = b.name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::invalid(
            "name",
            "give it a name of up to 60 characters",
        ));
    }
    let (key, public_key) = slop_manifest::sshsig::parse_public_key(b.public_key.trim()).map_err(|_| {
        ApiError::invalid(
            "public_key",
            "paste an Ed25519 public key (the line in id_ed25519.pub, starting with ssh-ed25519)",
        )
    })?;
    let fingerprint = slop_manifest::sshsig::fingerprint(&key);
    let mut tx = state.db.begin().await?;
    // Re-adding a key the user already has (to rename it) isn't another key.
    let count: i64 = sqlx::query_scalar(
        "select count(*) from maker_keys where user_id = $1 and revoked_at is null and fingerprint <> $2",
    )
    .bind(user.id)
    .bind(&fingerprint)
    .fetch_one(&mut *tx)
    .await?;
    if count >= MAX_KEYS {
        return Err(ApiError::Conflict(format!(
            "you can have up to {MAX_KEYS} signing keys"
        )));
    }
    let info: KeyInfo = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "insert into maker_keys (id, user_id, public_key, fingerprint, name, active_from)
         values ($1, $2, $3, $4, $5, now() + $6)
         on conflict (user_id, fingerprint) do update set revoked_at = null, name = excluded.name,
             active_from = now() + $6, created_at = now()
         returning {COLUMNS}"
    )))
    .bind(Uuid::now_v7())
    .bind(user.id)
    .bind(&public_key)
    .bind(&fingerprint)
    .bind(name)
    .bind(WAIT)
    .fetch_one(&mut *tx)
    .await?;
    crate::notify::send(
        &mut tx,
        crate::notify::Notice {
            user_id: user.id,
            category: "apps",
            title: "A signing key was added to your account".into(),
            body: format!(
                "\"{name}\" ({fingerprint}) can sign your releases from {}. If you didn't add it, remove it now under Account → Signing keys and sign out your other devices.",
                info.active_from.date()
            ),
            link: "/account#signing-keys".into(),
            collapse_key: None,
        },
    )
    .await?;
    crate::admin::audit(
        &mut tx,
        &user.id.to_string(),
        "maker_key.add",
        &fingerprint,
        serde_json::json!({ "name": name }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(info)))
}

async fn remove(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let n = sqlx::query(
        "update maker_keys set revoked_at = now() where id = $1 and user_id = $2 and revoked_at is null",
    )
    .bind(id)
    .bind(user.id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}
