//! Website sessions (SECURITY §4.2): an opaque random token in an `HttpOnly`, `SameSite=Lax`
//! cookie, stored hashed. Sessions expire 30 days after sign-in, or after 7 days unused.

use axum::{extract::FromRequestParts, http::request::Parts};
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
/// `last_seen_at` is only written when it is older than this, to avoid a write per request.
const TOUCH_INTERVAL: Duration = Duration::minutes(5);

/// The signed-in user. Extracting it fails with 401 when there is no valid session.
#[derive(Clone, Debug)]
pub struct CurrentUser {
    pub id: Uuid,
    pub session_id: Uuid,
    pub display_name: String,
    pub country_code: String,
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

async fn lookup(state: &AppState, jar: &CookieJar) -> ApiResult<Option<CurrentUser>> {
    let Some(cookie) = jar.get(cookie_name(&state.config)) else {
        return Ok(None);
    };
    let user: Option<(CurrentUser, bool)> =
        sqlx::query_as::<_, (Uuid, Uuid, String, String, bool)>(
            "select u.id, s.id, u.display_name, u.country_code, s.last_seen_at < now() - $2
         from sessions s join users u on u.id = s.user_id
         where s.token_hash = $1 and s.revoked_at is null
           and s.expires_at > now() and s.last_seen_at > now() - $3",
        )
        .bind(hash_token(cookie.value()))
        .bind(TOUCH_INTERVAL)
        .bind(IDLE_TIMEOUT)
        .fetch_optional(&state.db)
        .await?
        .map(|(id, session_id, display_name, country_code, stale)| {
            (
                CurrentUser {
                    id,
                    session_id,
                    display_name,
                    country_code,
                },
                stale,
            )
        });

    let Some((user, stale)) = user else {
        return Ok(None);
    };
    if stale {
        sqlx::query("update sessions set last_seen_at = now() where id = $1")
            .bind(user.session_id)
            .execute(&state.db)
            .await?;
    }
    Ok(Some(user))
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> ApiResult<Self> {
        let jar = CookieJar::from_headers(&parts.headers);
        lookup(state, &jar).await?.ok_or(ApiError::Unauthorized)
    }
}

impl FromRequestParts<AppState> for MaybeUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> ApiResult<Self> {
        let jar = CookieJar::from_headers(&parts.headers);
        Ok(Self(lookup(state, &jar).await?))
    }
}
