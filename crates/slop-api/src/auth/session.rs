//! Sessions (SECURITY §4.2).
//!
//! - **Website:** an opaque random token in an `HttpOnly`, `SameSite=Lax` cookie, stored hashed.
//!   Expires 30 days after sign-in, or after 7 days unused.
//! - **Desktop app:** a short-lived bearer access token plus a rotating refresh token (see
//!   `desktop.rs`). Expires 180 days after sign-in, or after 30 days unused.

use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, header::AUTHORIZATION, request::Parts},
};
use axum_extra::extract::{
    CookieJar,
    cookie::{Cookie, SameSite},
};
use sqlx::PgPool;
use time::Duration;
use uuid::Uuid;

use crate::{
    AppState,
    config::Config,
    error::{ApiError, ApiResult},
    tokens::{hash_token, new_token},
};

pub const ABSOLUTE_LIFETIME: Duration = Duration::days(30);
pub const IDLE_TIMEOUT: Duration = Duration::days(7);
pub const DESKTOP_ABSOLUTE_LIFETIME: Duration = Duration::days(180);
pub const DESKTOP_IDLE_TIMEOUT: Duration = Duration::days(30);

/// SQL condition for a live session `s`, given `$idle` and `$desktop_idle` placeholders.
pub fn live_session_sql(idle: &str, desktop_idle: &str) -> String {
    format!(
        "s.revoked_at is null and s.expires_at > now()
         and s.last_seen_at > now() - case s.kind when 'desktop' then {desktop_idle} else {idle} end"
    )
}
/// `last_seen_at` is only written when it is older than this, to avoid a write per request.
const TOUCH_INTERVAL: Duration = Duration::minutes(5);

/// The signed-in user. Extracting it fails with 401 when there is no valid session.
#[derive(Clone, Debug)]
pub struct CurrentUser {
    pub id: Uuid,
    pub session_id: Uuid,
    pub display_name: String,
    pub country_code: String,
    /// Suspended by a moderation decision: may read, appeal, download their data, sign out or
    /// close the account, and nothing else (`allowed_while_suspended`).
    pub suspended: bool,
}

/// The signed-in user, if any. Never fails because of a missing or stale session.
#[derive(Clone, Debug)]
pub struct MaybeUser(pub Option<CurrentUser>);

pub fn cookie_name(config: &Config) -> &'static str {
    if config.secure_cookies() {
        "__Host-slop_session"
    } else {
        "slop_session"
    }
}

/// Creates a session and returns the cookie that carries it.
pub async fn create(
    db: &PgPool,
    config: &Config,
    user_id: Uuid,
    user_agent: Option<&str>,
) -> ApiResult<Cookie<'static>> {
    let token = new_token();
    let user_agent = user_agent.map(|ua| ua.chars().take(200).collect::<String>());
    sqlx::query(
        "insert into sessions (id, user_id, token_hash, user_agent, expires_at)
         values ($1, $2, $3, $4, now() + $5)",
    )
    .bind(Uuid::now_v7())
    .bind(user_id)
    .bind(hash_token(&token))
    .bind(user_agent)
    .bind(ABSOLUTE_LIFETIME)
    .execute(db)
    .await?;
    Ok(build_cookie(
        config,
        cookie_name(config),
        token,
        ABSOLUTE_LIFETIME,
    ))
}

/// A cookie scoped the way every Slop auth cookie is: whole site, `HttpOnly`, `SameSite=Lax`.
pub fn build_cookie(
    config: &Config,
    name: &'static str,
    value: String,
    max_age: Duration,
) -> Cookie<'static> {
    Cookie::build((name, value))
        .path("/")
        .http_only(true)
        .secure(config.secure_cookies())
        .same_site(SameSite::Lax)
        .max_age(max_age)
        .build()
}

pub fn removal_cookie(config: &Config, name: &'static str) -> Cookie<'static> {
    build_cookie(config, name, String::new(), Duration::ZERO)
}

/// The bearer token from an `Authorization: Bearer …` header.
fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

/// Finds the session for a request: a desktop bearer token if present, else the website cookie.
async fn lookup(state: &AppState, headers: &HeaderMap) -> ApiResult<Option<CurrentUser>> {
    let live = live_session_sql("$2", "$3");
    let (sql, token) = if let Some(token) = bearer(headers) {
        (
            format!(
                "select u.id, s.id, u.display_name, u.country_code, s.last_seen_at < now() - $4,
                        u.suspended_at is not null
                 from access_tokens t join sessions s on s.id = t.session_id join users u on u.id = s.user_id
                 where t.token_hash = $1 and t.expires_at > now() and s.kind = 'desktop' and u.closed_at is null
                   and {live}"
            ),
            token.to_string(),
        )
    } else {
        let jar = CookieJar::from_headers(headers);
        let Some(cookie) = jar.get(cookie_name(&state.config)) else {
            return Ok(None);
        };
        (
            format!(
                "select u.id, s.id, u.display_name, u.country_code, s.last_seen_at < now() - $4,
                        u.suspended_at is not null
                 from sessions s join users u on u.id = s.user_id
                 where s.token_hash = $1 and s.kind = 'web' and u.closed_at is null and {live}"
            ),
            cookie.value().to_string(),
        )
    };

    let row: Option<(Uuid, Uuid, String, String, bool, bool)> =
        sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(hash_token(&token))
            .bind(IDLE_TIMEOUT)
            .bind(DESKTOP_IDLE_TIMEOUT)
            .bind(TOUCH_INTERVAL)
            .fetch_optional(&state.db)
            .await?;
    let Some((id, session_id, display_name, country_code, stale, suspended)) = row else {
        return Ok(None);
    };
    if stale {
        sqlx::query("update sessions set last_seen_at = now() where id = $1")
            .bind(session_id)
            .execute(&state.db)
            .await?;
    }
    Ok(Some(CurrentUser {
        id,
        session_id,
        display_name,
        country_code,
        suspended,
    }))
}

/// What a suspended account may still do besides reading: appeal, accept terms, download its
/// data, sign out (here or on other devices) and close the account.
pub fn allowed_while_suspended(method: &axum::http::Method, path: &str) -> bool {
    let path = path.strip_prefix("/v1").unwrap_or(path);
    method == axum::http::Method::GET
        || method == axum::http::Method::HEAD
        || path == "/auth/signout"
        || path == "/me/close"
        || path == "/me/legal/accept"
        || path.starts_with("/me/sessions/")
        || (path.starts_with("/me/decisions/") && path.ends_with("/appeal"))
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> ApiResult<Self> {
        let user = lookup(state, &parts.headers)
            .await?
            .ok_or(ApiError::Unauthorized)?;
        if user.suspended && !allowed_while_suspended(&parts.method, parts.uri.path()) {
            return Err(ApiError::Suspended);
        }
        Ok(user)
    }
}

impl FromRequestParts<AppState> for MaybeUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> ApiResult<Self> {
        Ok(Self(lookup(state, &parts.headers).await?))
    }
}
