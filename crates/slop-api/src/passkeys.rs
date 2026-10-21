// SPDX-License-Identifier: AGPL-3.0-or-later
//! Passkeys (SECURITY §4.1, §4.3): WebAuthn through webauthn-rs.
//!
//! - **Adding one** (Account → Sign-ins) needs a recent sign-in, like linking a provider.
//! - **Signing in** with one needs no username: the browser offers the passkeys it has for Slop,
//!   and the one chosen names its account. Closed accounts are refused, as with providers.
//! - **Step-up:** a sensitive action that asks for a recent sign-in (changing where donations
//!   go, managing tokens…) also accepts confirming with a passkey in the last ten minutes.
//!
//! - **Required for publishers and creatives** (`require`): 30 days after someone first publishes
//!   or sets up as a creative, publishing and payout changes need a passkey used in the last 12
//!   hours (signing in with it, or confirming with it). Reminders go a week and a day before.
//!   Staff can give a week's grace to someone who lost theirs (`grace`, agent.rs).
//!
//! Slop keeps only public keys and signature counters. Each ceremony's state waits in
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
/// Time to add a passkey after first publishing or setting up as a creative.
pub const GRACE: time::Duration = time::Duration::days(30);
/// How long proving a passkey counts for publishing and payouts.
pub const PROOF_WINDOW: time::Duration = time::Duration::hours(12);
/// What staff can give someone who lost their passkeys, after checking it's them.
pub const SUPPORT_GRACE: time::Duration = time::Duration::days(7);
const MAX_PASSKEYS: i64 = 10;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/passkeys", get(list))
        .route("/me/passkeys/{id}", delete(remove))
        .route("/me/passkeys/register/start", post(register_start))
        .route("/me/passkeys/register/finish", post(register_finish))
        .route("/me/step-up/start", post(step_up_start))
        .route("/me/step-up/finish", post(step_up_finish))
        .route("/auth/passkey/start", post(sign_in_start))
        .route("/auth/passkey/finish", post(sign_in_finish))
}

/// Slop as a WebAuthn relying party: its own host, its own origin.
fn webauthn(state: &AppState) -> ApiResult<Webauthn> {
    let origin = &state.config.public_url;
    let rp_id = origin
        .host_str()
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("SLOP_PUBLIC_URL has no host")))?;
    WebauthnBuilder::new(rp_id, origin)
        .and_then(|b| b.rp_name("Slop").build())
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

/// Takes a challenge (once): its state, if it's this kind, for this user, and not expired.
async fn take_challenge(
    state: &AppState,
    id: Uuid,
    kind: &str,
    user_id: Option<Uuid>,
) -> ApiResult<Value> {
    let row: Option<(Option<Uuid>, Value)> = sqlx::query_as(
        "delete from webauthn_challenges where id = $1 and kind = $2 and expires_at > now()
         returning user_id, state",
    )
    .bind(id)
    .bind(kind)
    .fetch_optional(&state.db)
    .await?;
    match row {
        Some((owner, ceremony)) if owner == user_id => Ok(ceremony),
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
    session::require_recent_sign_in(&state.db, &user).await?;
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
    session::require_recent_sign_in(&state.db, &user).await?;
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
    let ceremony = take_challenge(&state, b.challenge_id, "register", Some(user.id)).await?;
    let reg: PasskeyRegistration =
        serde_json::from_value(ceremony).map_err(|e| ApiError::Internal(e.into()))?;
    let key = webauthn(&state)?
        .finish_passkey_registration(&b.credential, &reg)
        .map_err(refused)?;
    let id = Uuid::now_v7();
    let info: PasskeyInfo = sqlx::query_as(
        "insert into passkeys (id, user_id, credential_id, credential, name) values ($1, $2, $3, $4, $5)
         returning id, name, created_at, last_used_at",
    )
    .bind(id)
    .bind(user.id)
    .bind(key.cred_id().as_ref())
    .bind(serde_json::to_value(&key).map_err(|e| ApiError::Internal(e.into()))?)
    .bind(name)
    .fetch_one(&state.db)
    .await
    .map_err(|e| {
        if crate::error::is_unique_violation(&e) {
            ApiError::Conflict("that passkey is already on a Slop account".into())
        } else {
            e.into()
        }
    })?;
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
        row.ok_or_else(|| ApiError::invalid("credential", "Slop doesn't know that passkey"))?;
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
    // Signed in with a passkey: that counts as proving one (`require`).
    sqlx::query("update sessions set passkey_at = now() where token_hash = $1")
        .bind(crate::tokens::hash_token(cookie.value()))
        .execute(&state.db)
        .await?;
    Ok((jar.add(cookie), Json(json!({ "signed_in": true }))))
}

async fn step_up_start(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Started<RequestChallengeResponse>>> {
    let keys: Vec<Passkey> = passkeys_of(&state, user.id)
        .await?
        .into_iter()
        .map(|(_, k)| k)
        .collect();
    if keys.is_empty() {
        return Err(ApiError::Conflict(
            "add a passkey first, or sign in again to confirm".into(),
        ));
    }
    let (options, auth) = webauthn(&state)?
        .start_passkey_authentication(&keys)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("{e}")))?;
    let id = save_challenge(
        &state,
        "step_up",
        Some(user.id),
        Some(user.session_id),
        serde_json::to_value(&auth).map_err(|e| ApiError::Internal(e.into()))?,
    )
    .await?;
    Ok(Json(Started {
        challenge_id: id,
        options,
    }))
}

async fn step_up_finish(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(b): Json<Finish>,
) -> ApiResult<StatusCode> {
    let ceremony = take_challenge(&state, b.challenge_id, "step_up", Some(user.id)).await?;
    let auth: PasskeyAuthentication =
        serde_json::from_value(ceremony).map_err(|e| ApiError::Internal(e.into()))?;
    let result = webauthn(&state)?
        .finish_passkey_authentication(&b.credential, &auth)
        .map_err(refused)?;
    let keys = passkeys_of(&state, user.id).await?;
    let (id, key) = keys
        .into_iter()
        .find(|(_, k)| k.cred_id() == result.cred_id())
        .ok_or_else(|| ApiError::invalid("credential", "Slop doesn't know that passkey"))?;
    used(&state, id, key, &result).await?;
    sqlx::query("update sessions set stepped_up_at = now(), passkey_at = now() where id = $1")
        .bind(user.session_id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Publishing and payout changes (SECURITY §4.1): fine before the user's deadline (started here
/// the first time, with a notice), during a grace staff gave, or with a passkey proved by this
/// session in the last 12 hours; otherwise `PasskeyRequired`.
pub async fn require(state: &AppState, user: &CurrentUser) -> ApiResult<()> {
    let (due, grace, has, proved): (Option<OffsetDateTime>, Option<OffsetDateTime>, bool, bool) =
        sqlx::query_as(
            "select u.passkey_due_at, u.passkey_grace_until,
                    exists (select 1 from passkeys p where p.user_id = u.id),
                    coalesce((select s.passkey_at > now() - $3 from sessions s where s.id = $2), false)
             from users u where u.id = $1",
        )
        .bind(user.id)
        .bind(user.session_id)
        .bind(PROOF_WINDOW)
        .fetch_one(&state.db)
        .await?;
    let now = OffsetDateTime::now_utc();
    match due {
        None => {
            let mut tx = state.db.begin().await?;
            let started: Option<OffsetDateTime> = sqlx::query_scalar(
                "update users set passkey_due_at = now() + $2 where id = $1 and passkey_due_at is null
                 returning passkey_due_at",
            )
            .bind(user.id)
            .bind(GRACE)
            .fetch_optional(&mut *tx)
            .await?;
            if let (Some(at), false) = (started, has) {
                crate::notify::send(
                    &mut tx,
                    crate::notify::Notice {
                        user_id: user.id,
                        category: "apps",
                        title: "Add a passkey to your account".into(),
                        body: format!(
                            "From {}, publishing on Slop and changing where money goes need a passkey, so someone \
                             who gets into your sign-in elsewhere can't do either. Add one under Account → Sign-ins.",
                            at.date()
                        ),
                        link: "/account#sign-ins".into(),
                        collapse_key: Some("passkey".into()),
                    },
                )
                .await?;
            }
            tx.commit().await?;
            Ok(())
        }
        Some(due) if due > now => Ok(()),
        _ if grace.is_some_and(|g| g > now) => Ok(()),
        _ if has && proved => Ok(()),
        _ => Err(ApiError::PasskeyRequired(has)),
    }
}

/// Reminders a week and a day before the deadline, to publishers and creatives with no passkey.
pub async fn remind(db: &sqlx::PgPool) -> anyhow::Result<usize> {
    let due: Vec<(Uuid, OffsetDateTime, i16)> = sqlx::query_as(
        "select u.id, u.passkey_due_at,
                (case when u.passkey_due_at < now() + interval '1 day' then 2 else 1 end)::int2
         from users u
         where u.passkey_due_at > now() and u.passkey_due_at < now() + interval '7 days'
           and u.closed_at is null
           and not exists (select 1 from passkeys p where p.user_id = u.id)
           and u.passkey_reminders < case when u.passkey_due_at < now() + interval '1 day' then 2 else 1 end",
    )
    .fetch_all(db)
    .await?;
    for (user_id, at, step) in &due {
        let mut tx = db.begin().await?;
        sqlx::query("update users set passkey_reminders = $2 where id = $1")
            .bind(user_id)
            .bind(step)
            .execute(&mut *tx)
            .await?;
        crate::notify::send(
            &mut tx,
            crate::notify::Notice {
                user_id: *user_id,
                category: "apps",
                title: if *step == 2 {
                    "Tomorrow: publishing needs a passkey".into()
                } else {
                    "In a week: publishing needs a passkey".into()
                },
                body: format!(
                    "From {}, publishing on Slop and changing where money goes need a passkey. Add one under \
                     Account → Sign-ins: it takes a minute.",
                    at.date()
                ),
                link: "/account#sign-ins".into(),
                collapse_key: Some("passkey".into()),
            },
        )
        .await?;
        tx.commit().await?;
    }
    Ok(due.len())
}

/// Lets someone who lost their passkeys publish without one for a week (staff, after checking
/// it's them; agent.rs).
pub async fn grace(conn: &mut sqlx::PgConnection, user_id: Uuid) -> sqlx::Result<OffsetDateTime> {
    sqlx::query_scalar(
        "update users set passkey_grace_until = now() + $2 where id = $1 returning passkey_grace_until",
    )
    .bind(user_id)
    .bind(SUPPORT_GRACE)
    .fetch_one(conn)
    .await
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
