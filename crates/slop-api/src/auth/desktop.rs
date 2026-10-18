//! Desktop sign-in (SECURITY §4.1-4.2, RFC 8252).
//!
//! 1. The app creates a sign-in request with a PKCE challenge and the loopback port it listens
//!    on, then opens the returned approval page in the system browser.
//! 2. On the website, the user signs in (or signs up) as usual and approves the device.
//! 3. The browser is sent to `http://127.0.0.1:<port>/callback?code=…&state=…`, where the app
//!    is listening.
//! 4. The app swaps the code and its PKCE verifier for an access token (15 minutes) and a
//!    refresh token. Refresh tokens rotate on every use; reusing one revokes the session.
//!
//! The app never sees the provider password or the website cookie, and the website never hands
//! a token to anything but the app's own loopback address.

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::session::{self, CurrentUser};
use crate::{
    AppState,
    error::{ApiError, ApiResult},
    tokens::{hash_token, new_token, pkce_challenge},
};

const REQUEST_LIFETIME: Duration = Duration::minutes(10);
pub const ACCESS_TOKEN_LIFETIME: Duration = Duration::minutes(15);
pub const REFRESH_TOKEN_LIFETIME: Duration = Duration::days(30);

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/desktop/requests", post(create_request))
        .route("/auth/desktop/requests/{id}", get(get_request))
        .route("/auth/desktop/requests/{id}/approve", post(approve))
        .route("/auth/desktop/token", post(token))
}

fn is_b64url(value: &str, len: std::ops::RangeInclusive<usize>) -> bool {
    len.contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewRequest {
    code_challenge: String,
    redirect_port: u16,
    state: String,
    device_name: String,
}

#[derive(Serialize)]
struct CreatedRequest {
    id: Uuid,
    /// The page to open in the system browser.
    approve_url: String,
    #[serde(with = "time::serde::rfc3339")]
    expires_at: OffsetDateTime,
}

async fn create_request(
    State(state): State<AppState>,
    Json(req): Json<NewRequest>,
) -> ApiResult<Json<CreatedRequest>> {
    if !is_b64url(&req.code_challenge, 43..=43) {
        return Err(ApiError::invalid(
            "code_challenge",
            "must be a PKCE S256 challenge",
        ));
    }
    if !is_b64url(&req.state, 16..=128) {
        return Err(ApiError::invalid(
            "state",
            "must be 16 to 128 URL-safe characters",
        ));
    }
    if req.redirect_port < 1024 {
        return Err(ApiError::invalid("redirect_port", "must be 1024 or higher"));
    }
    let device_name = super::validate_display_name(&req.device_name)
        .map_err(|_| ApiError::invalid("device_name", "use 1 to 40 characters"))?;

    let id = Uuid::now_v7();
    sqlx::query("delete from desktop_auth_requests where expires_at < now()")
        .execute(&state.db)
        .await?;
    let expires_at: OffsetDateTime = sqlx::query_scalar(
        "insert into desktop_auth_requests (id, code_challenge, redirect_port, state, device_name, expires_at)
         values ($1, $2, $3, $4, $5, now() + $6) returning expires_at",
    )
    .bind(id)
    .bind(&req.code_challenge)
    .bind(i32::from(req.redirect_port))
    .bind(&req.state)
    .bind(&device_name)
    .bind(REQUEST_LIFETIME)
    .fetch_one(&state.db)
    .await?;

    let approve_url = state
        .config
        .public_url
        .join(&format!("/desktop-signin/{id}"))
        .expect("valid public URL")
        .into();
    Ok(Json(CreatedRequest {
        id,
        approve_url,
        expires_at,
    }))
}

#[derive(Serialize)]
struct RequestInfo {
    device_name: String,
    #[serde(with = "time::serde::rfc3339")]
    expires_at: OffsetDateTime,
}

/// What the approval page shows. Only for a signed-in user, and only while still pending.
async fn get_request(
    State(state): State<AppState>,
    _user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<RequestInfo>> {
    let row: Option<(String, OffsetDateTime)> = sqlx::query_as(
        "select device_name, expires_at from desktop_auth_requests
         where id = $1 and approved_by is null and expires_at > now()",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let (device_name, expires_at) = row.ok_or(ApiError::NotFound)?;
    Ok(Json(RequestInfo {
        device_name,
        expires_at,
    }))
}

#[derive(Serialize)]
struct Approved {
    /// Where the browser goes next: the app's loopback listener.
    redirect_url: String,
}

/// The signed-in user approves the device. Cookie-authenticated, so the CSRF layer applies.
async fn approve(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Approved>> {
    let code = new_token();
    let row: Option<(i32, String)> = sqlx::query_as(
        "update desktop_auth_requests set approved_by = $2, code_hash = $3
         where id = $1 and approved_by is null and expires_at > now()
         returning redirect_port, state",
    )
    .bind(id)
    .bind(user.id)
    .bind(hash_token(&code))
    .fetch_optional(&state.db)
    .await?;
    let (port, app_state) = row.ok_or(ApiError::NotFound)?;

    let mut url =
        url::Url::parse(&format!("http://127.0.0.1:{port}/callback")).expect("valid loopback URL");
    url.query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", &app_state);
    Ok(Json(Approved {
        redirect_url: url.into(),
    }))
}

#[derive(Deserialize)]
#[serde(tag = "grant_type", rename_all = "snake_case", deny_unknown_fields)]
enum TokenRequest {
    AuthorizationCode { code: String, code_verifier: String },
    RefreshToken { refresh_token: String },
}

#[derive(Serialize, Debug)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    /// Seconds until the access token expires.
    pub expires_in: i64,
    pub token_type: &'static str,
}

async fn token(
    State(state): State<AppState>,
    Json(req): Json<TokenRequest>,
) -> ApiResult<Json<TokenResponse>> {
    match req {
        TokenRequest::AuthorizationCode {
            code,
            code_verifier,
        } => redeem_code(&state, &code, &code_verifier).await.map(Json),
        TokenRequest::RefreshToken { refresh_token } => {
            refresh(&state, &refresh_token).await.map(Json)
        }
    }
}

async fn redeem_code(state: &AppState, code: &str, verifier: &str) -> ApiResult<TokenResponse> {
    let mut tx = state.db.begin().await?;
    // Single use: the row is deleted whether or not the verifier matches.
    let row: Option<(Uuid, String, String)> = sqlx::query_as(
        "delete from desktop_auth_requests
         where code_hash = $1 and approved_by is not null and expires_at > now()
         returning approved_by, code_challenge, device_name",
    )
    .bind(hash_token(code))
    .fetch_optional(&mut *tx)
    .await?;
    let Some((user_id, challenge, device_name)) = row else {
        return Err(ApiError::Unauthorized);
    };
    if pkce_challenge(verifier) != challenge {
        tx.commit().await?;
        return Err(ApiError::Unauthorized);
    }

    let session_id = Uuid::now_v7();
    sqlx::query(
        "insert into sessions (id, user_id, kind, user_agent, expires_at)
         values ($1, $2, 'desktop', $3, now() + $4)",
    )
    .bind(session_id)
    .bind(user_id)
    .bind(&device_name)
    .bind(session::DESKTOP_ABSOLUTE_LIFETIME)
    .execute(&mut *tx)
    .await?;
    let tokens = issue(&mut tx, session_id).await?;
    tx.commit().await?;
    Ok(tokens)
}

async fn refresh(state: &AppState, refresh_token: &str) -> ApiResult<TokenResponse> {
    let mut tx = state.db.begin().await?;
    let live = session::live_session_sql("$2", "$3");
    let row: Option<(Uuid, bool, bool)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select r.session_id, r.used_at is not null, ({live})
         from refresh_tokens r join sessions s on s.id = r.session_id
         where r.token_hash = $1 and r.expires_at > now()
         for update of r"
    )))
    .bind(hash_token(refresh_token))
    .bind(session::IDLE_TIMEOUT)
    .bind(session::DESKTOP_IDLE_TIMEOUT)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((session_id, already_used, live)) = row else {
        return Err(ApiError::Unauthorized);
    };
    if already_used {
        // A used refresh token came back: it was copied. End the whole session.
        tracing::warn!(%session_id, "refresh token reuse; revoking session");
        sqlx::query("update sessions set revoked_at = now() where id = $1 and revoked_at is null")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Err(ApiError::Unauthorized);
    }
    if !live {
        return Err(ApiError::Unauthorized);
    }

    sqlx::query("update refresh_tokens set used_at = now() where token_hash = $1")
        .bind(hash_token(refresh_token))
        .execute(&mut *tx)
        .await?;
    sqlx::query("update sessions set last_seen_at = now() where id = $1")
        .bind(session_id)
        .execute(&mut *tx)
        .await?;
    let tokens = issue(&mut tx, session_id).await?;
    tx.commit().await?;
    Ok(tokens)
}

async fn issue(tx: &mut sqlx::PgConnection, session_id: Uuid) -> ApiResult<TokenResponse> {
    let access_token = new_token();
    let refresh_token = new_token();
    sqlx::query("delete from access_tokens where session_id = $1 and expires_at < now()")
        .bind(session_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("insert into access_tokens (token_hash, session_id, expires_at) values ($1, $2, now() + $3)")
        .bind(hash_token(&access_token))
        .bind(session_id)
        .bind(ACCESS_TOKEN_LIFETIME)
        .execute(&mut *tx)
        .await?;
    sqlx::query("insert into refresh_tokens (token_hash, session_id, expires_at) values ($1, $2, now() + $3)")
        .bind(hash_token(&refresh_token))
        .bind(session_id)
        .bind(REFRESH_TOKEN_LIFETIME)
        .execute(&mut *tx)
        .await?;
    Ok(TokenResponse {
        access_token,
        refresh_token,
        expires_in: ACCESS_TOKEN_LIFETIME.whole_seconds(),
        token_type: "Bearer",
    })
}
