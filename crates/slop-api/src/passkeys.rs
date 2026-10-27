// SPDX-License-Identifier: AGPL-3.0-or-later
//! Passkeys (SECURITY §4.1, §4.3): WebAuthn through webauthn-rs. Optional: a quicker,
//! phishing-proof way to sign in, never required for anything.
//!
//! - **Adding or removing one** (Account → Sign-in & security) works from any signed-in session.
//! - **Signing in** with one needs no username: the browser offers the passkeys it has for SLAPorSLOP,
//!   and the one chosen names its account. Closed accounts are refused, as with providers.
//!
//! Until 2026-10-25, sensitive actions (changing where donations go, linking sign-ins, closing the
//! account…) asked for a recent sign-in or passkey, and publishers and creatives had to use a
//! passkey every 12 hours to publish or change payouts. The owner dropped both as too much of a
//! hassle: a signed-in session is trusted for them now (SECURITY §4.3).
//!
//! SLAPorSLOP keeps only public keys and signature counters. Each ceremony's state waits in
//! `webauthn_challenges` for five minutes and is used once.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{delete, get, post},
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;
use webauthn_rs::prelude::*;

use crate::{
    AppState,
    auth::session::{self, CurrentUser},
    error::{ApiError, ApiResult},
};

const CHALLENGE_LIFETIME: time::Duration = time::Duration::minutes(5);
const MAX_PASSKEYS: i64 = 10;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/passkeys", get(list))
        .route("/me/passkeys/{id}", delete(remove))
        .route("/me/passkeys/register/start", post(register_start))
        .route("/me/passkeys/register/finish", post(register_finish))
        .route("/auth/passkey/start", post(sign_in_start))
        .route("/auth/passkey/finish", post(sign_in_finish))
}

/// SLAPorSLOP as a WebAuthn relying party: its own host, its own origin.
fn webauthn(state: &AppState) -> ApiResult<Webauthn> {
    let origin = &state.config.public_url;
    let rp_id = origin
        .host_str()
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("SLOP_PUBLIC_URL has no host")))?;
    WebauthnBuilder::new(rp_id, origin)
        .and_then(|b| b.rp_name("SLAPorSLOP").build())
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("WebAuthn setup: {e}")))
}

fn refused(e: WebauthnError) -> ApiError {
    ApiError::invalid("credential", format!("the passkey didn't check out ({e})"))
}

async fn save_challenge(
    state: &AppState,
    kind: &str,
    user_id: Option<Uuid>,
    session_id: Option<Uuid>,
    ceremony: Value,
) -> ApiResult<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into webauthn_challenges (id, kind, user_id, session_id, state, expires_at)
         values ($1, $2, $3, $4, $5, now() + $6)",
    )
    .bind(id)
    .bind(kind)
    .bind(user_id)
    .bind(session_id)
    .bind(ceremony)
    .bind(CHALLENGE_LIFETIME)
    .execute(&state.db)
    .await?;
    Ok(id)
}

/// Takes a challenge (once): its state, if it's this kind, for this user and the session that
/// started it, and not expired. A confirmation counts for the session that asked for it, not
/// for another session of the same account.
async fn take_challenge(
    state: &AppState,
    id: Uuid,
    kind: &str,
    user: Option<&CurrentUser>,
) -> ApiResult<Value> {
    let row: Option<(Option<Uuid>, Option<Uuid>, Value)> = sqlx::query_as(
        "delete from webauthn_challenges where id = $1 and kind = $2 and expires_at > now()
         returning user_id, session_id, state",
    )
    .bind(id)
    .bind(kind)
    .fetch_optional(&state.db)
    .await?;
    match row {
        Some((owner, session, ceremony))
            if owner == user.map(|u| u.id) && session == user.map(|u| u.session_id) =>
        {
            Ok(ceremony)
        }
        _ => Err(ApiError::Conflict(
            "that passkey request expired: try again".into(),
        )),
    }
}

async fn passkeys_of(state: &AppState, user_id: Uuid) -> ApiResult<Vec<(Uuid, Passkey)>> {
    let rows: Vec<(Uuid, Value)> = sqlx::query_as(
        "select id, credential from passkeys where user_id = $1 order by created_at",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;
    rows.into_iter()
        .map(|(id, v)| {
            serde_json::from_value(v)
                .map(|p| (id, p))
                .map_err(|e| ApiError::Internal(e.into()))
        })
        .collect()
}

/// Records a successful use: the new signature counter, and when.
async fn used(
    state: &AppState,
    id: Uuid,
    mut key: Passkey,
    result: &AuthenticationResult,
) -> ApiResult<()> {
    key.update_credential(result);
    sqlx::query("update passkeys set credential = $2, last_used_at = now() where id = $1")
        .bind(id)
        .bind(serde_json::to_value(&key).map_err(|e| ApiError::Internal(e.into()))?)
        .execute(&state.db)
        .await?;
    Ok(())
}

#[derive(Serialize, sqlx::FromRow)]
struct PasskeyInfo {
    id: Uuid,
    name: String,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    last_used_at: Option<OffsetDateTime>,
}

async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Vec<PasskeyInfo>>> {
    Ok(Json(
        sqlx::query_as(
            "select id, name, created_at, last_used_at from passkeys where user_id = $1 order by created_at",
        )
        .bind(user.id)
        .fetch_all(&state.db)
        .await?,
    ))
}

async fn remove(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let n = sqlx::query("delete from passkeys where id = $1 and user_id = $2")
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

#[derive(Serialize)]
struct Started<T: Serialize> {
    challenge_id: Uuid,
    options: T,
}

async fn register_start(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Started<CreationChallengeResponse>>> {
    let existing = passkeys_of(&state, user.id).await?;
    if existing.len() as i64 >= MAX_PASSKEYS {
        return Err(ApiError::Conflict(format!(
            "you can have up to {MAX_PASSKEYS} passkeys"
        )));
    }
    let exclude: Vec<CredentialID> = existing.iter().map(|(_, p)| p.cred_id().clone()).collect();
    let (options, reg) = webauthn(&state)?
        .start_passkey_registration(
            user.id,
            &user.display_name,
            &user.display_name,
            Some(exclude),
        )
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("{e}")))?;
    let id = save_challenge(
        &state,
        "register",
        Some(user.id),
        Some(user.session_id),
        serde_json::to_value(&reg).map_err(|e| ApiError::Internal(e.into()))?,
    )
    .await?;
    Ok(Json(Started {
        challenge_id: id,
        options,
    }))
}

#[derive(Deserialize)]
struct RegisterFinish {
    challenge_id: Uuid,
    /// What to call it, e.g. "MacBook" or "YubiKey".
    name: String,
    credential: RegisterPublicKeyCredential,
}

async fn register_finish(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(b): Json<RegisterFinish>,
) -> ApiResult<(StatusCode, Json<PasskeyInfo>)> {
    let name = b.name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::invalid(
            "name",
            "give it a name of up to 60 characters",
        ));
    }
    let ceremony = take_challenge(&state, b.challenge_id, "register", Some(&user)).await?;
    let reg: PasskeyRegistration =
        serde_json::from_value(ceremony).map_err(|e| ApiError::Internal(e.into()))?;
    let key = webauthn(&state)?
        .finish_passkey_registration(&b.credential, &reg)
        .map_err(refused)?;
    let id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    let info: PasskeyInfo = sqlx::query_as(
        "insert into passkeys (id, user_id, credential_id, credential, name) values ($1, $2, $3, $4, $5)
         returning id, name, created_at, last_used_at",
    )
    .bind(id)
    .bind(user.id)
    .bind(key.cred_id().as_ref())
    .bind(serde_json::to_value(&key).map_err(|e| ApiError::Internal(e.into()))?)
    .bind(name)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        if crate::error::is_unique_violation(&e) {
            ApiError::Conflict("that passkey is already on a SLAPorSLOP account".into())
        } else {
            e.into()
        }
    })?;
    crate::notify::send(
        &mut tx,
        crate::notify::Notice {
            user_id: user.id,
            category: "security",
            title: "A passkey was added to your account".into(),
            body: format!(
                "\"{name}\" can now sign in to your SLAPorSLOP account and confirm publishing and payouts. \
                 If you didn't add it, remove it under Account → Sign-ins and sign out your other devices."
            ),
            link: "/account#sign-ins".into(),
            collapse_key: None,
        },
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(info)))
}

async fn sign_in_start(
    State(state): State<AppState>,
) -> ApiResult<Json<Started<RequestChallengeResponse>>> {
    let (options, auth) = webauthn(&state)?
        .start_discoverable_authentication()
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("{e}")))?;
    let id = save_challenge(
        &state,
        "sign_in",
        None,
        None,
        serde_json::to_value(&auth).map_err(|e| ApiError::Internal(e.into()))?,
    )
    .await?;
    Ok(Json(Started {
        challenge_id: id,
        options,
    }))
}

#[derive(Deserialize)]
struct Finish {
    challenge_id: Uuid,
    credential: PublicKeyCredential,
}

async fn sign_in_finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(b): Json<Finish>,
) -> ApiResult<impl IntoResponse> {
    let ceremony = take_challenge(&state, b.challenge_id, "sign_in", None).await?;
    let auth: DiscoverableAuthentication =
        serde_json::from_value(ceremony).map_err(|e| ApiError::Internal(e.into()))?;
    let wa = webauthn(&state)?;
    // The credential names the account it was made for, but accounts can be combined since
    // (auth/merge.rs moves passkeys along): the stored credential decides whose it is, and its
    // signature is what proves it.
    let cred_id = b.credential.get_credential_id();
    let row: Option<(Uuid, Uuid, Value, Option<OffsetDateTime>)> = sqlx::query_as(
        "select p.id, p.user_id, p.credential, u.closed_at from passkeys p join users u on u.id = p.user_id
         where p.credential_id = $1",
    )
    .bind(cred_id)
    .fetch_optional(&state.db)
    .await?;
    let (id, user_id, cred, closed) =
        row.ok_or_else(|| ApiError::invalid("credential", "SLAPorSLOP doesn't know that passkey"))?;
    if closed.is_some() {
        return Err(ApiError::Conflict("that account is closed".into()));
    }
    let key: Passkey = serde_json::from_value(cred).map_err(|e| ApiError::Internal(e.into()))?;
    let result = wa
        .finish_discoverable_authentication(&b.credential, auth, &[(&key).into()])
        .map_err(refused)?;
    used(&state, id, key, &result).await?;
    let agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok());
    let cookie = session::create(&state.db, &state.config, user_id, agent).await?;
    Ok((jar.add(cookie), Json(json!({ "signed_in": true }))))
}

/// Expired ceremonies (cleanup.rs).
pub async fn cleanup(db: &sqlx::PgPool) -> sqlx::Result<u64> {
    Ok(
        sqlx::query("delete from webauthn_challenges where expires_at < now()")
            .execute(db)
            .await?
            .rows_affected(),
    )
}
