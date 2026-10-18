//! Identity: sign-in with GitHub, sign-up, sign-out and the devices page (SECURITY §4).

pub mod desktop;
pub mod github;
pub mod session;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header::USER_AGENT},
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get, post},
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{
    AppState,
    error::{ApiError, ApiResult, is_unique_violation},
    tokens::{hash_token, new_token, pkce_challenge},
};
use session::{CurrentUser, MaybeUser, build_cookie, removal_cookie};

const OAUTH_COOKIE: &str = "slop_oauth_state";
const SIGNUP_COOKIE: &str = "slop_signup";
const OAUTH_FLOW_LIFETIME: Duration = Duration::minutes(10);
const SIGNUP_LIFETIME: Duration = Duration::minutes(30);

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/providers", get(providers))
        .route("/auth/github/start", get(github_start))
        .route("/auth/github/callback", get(github_callback))
        .route("/auth/signup", get(signup_info).post(signup))
        .route("/auth/signout", post(signout))
        .route("/countries", get(countries))
        .route("/me", get(me))
        .route("/me/sessions", get(list_sessions))
        .route("/me/sessions/{id}", delete(revoke_session))
        .merge(desktop::routes())
}

/// Only same-site relative paths, so sign-in can't be used as an open redirect.
fn safe_return_to(value: Option<&str>) -> String {
    match value {
        Some(path)
            if path.starts_with('/')
                && !path.starts_with("//")
                && !path.contains('\\')
                && !path.chars().any(char::is_control)
                && path.len() <= 512 =>
        {
            path.to_string()
        }
        _ => "/".to_string(),
    }
}

fn callback_url(state: &AppState) -> String {
    state
        .config
        .public_url
        .join("/v1/auth/github/callback")
        .expect("valid public URL")
        .into()
}

#[derive(Serialize)]
struct Providers {
    github: bool,
}

/// Which sign-in providers this server has set up.
async fn providers(State(state): State<AppState>) -> Json<Providers> {
    Json(Providers {
        github: state.config.github.is_some(),
    })
}

#[derive(Deserialize)]
struct StartQuery {
    return_to: Option<String>,
}

async fn github_start(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(query): Query<StartQuery>,
) -> ApiResult<(CookieJar, Redirect)> {
    let github = state
        .config
        .github
        .as_ref()
        .ok_or(ApiError::Unavailable("GitHub sign-in isn't set up"))?;

    let oauth_state = new_token();
    let verifier = new_token();
    sqlx::query("delete from oauth_flows where expires_at < now()")
        .execute(&state.db)
        .await?;
    sqlx::query(
        "insert into oauth_flows (state_hash, provider, pkce_verifier, return_to, expires_at)
         values ($1, 'github', $2, $3, now() + $4)",
    )
    .bind(hash_token(&oauth_state))
    .bind(&verifier)
    .bind(safe_return_to(query.return_to.as_deref()))
    .bind(OAUTH_FLOW_LIFETIME)
    .execute(&state.db)
    .await?;

    let url = github::authorize_url(
        github,
        &callback_url(&state),
        &oauth_state,
        &pkce_challenge(&verifier),
    );
    let jar = jar.add(build_cookie(
        &state.config,
        OAUTH_COOKIE,
        oauth_state,
        OAUTH_FLOW_LIFETIME,
    ));
    Ok((jar, Redirect::to(url.as_str())))
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

async fn github_callback(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Query(query): Query<CallbackQuery>,
) -> ApiResult<Response> {
    let github = state
        .config
        .github
        .as_ref()
        .ok_or(ApiError::Unavailable("GitHub sign-in isn't set up"))?;
    let jar = jar.remove(removal_cookie(&state.config, OAUTH_COOKIE));

    if query.error.is_some() {
        return Ok((jar, Redirect::to("/signin?error=cancelled")).into_response());
    }
    let (Some(code), Some(query_state)) = (query.code, query.state) else {
        return Err(ApiError::invalid("code", "missing code or state"));
    };
    // The state must match the cookie of the browser that started the flow (login CSRF).
    let cookie_state = CookieJar::from_headers(&headers)
        .get(OAUTH_COOKIE)
        .map(|c| c.value().to_string());
    if cookie_state.as_deref() != Some(query_state.as_str()) {
        return Err(ApiError::invalid(
            "state",
            "sign-in expired or was started in another browser",
        ));
    }
    let flow: Option<(String, String)> = sqlx::query_as(
        "delete from oauth_flows where state_hash = $1 and provider = 'github' and expires_at > now()
         returning pkce_verifier, return_to",
    )
    .bind(hash_token(&query_state))
    .fetch_optional(&state.db)
    .await?;
    let Some((verifier, return_to)) = flow else {
        return Err(ApiError::invalid(
            "state",
            "sign-in expired, please try again",
        ));
    };

    let profile = async {
        let token =
            github::exchange_code(&state.http, github, &code, &verifier, &callback_url(&state))
                .await?;
        let user = github::fetch_user(&state.http, github, &token).await?;
        let email = github::fetch_primary_email(&state.http, github, &token).await?;
        anyhow::Ok((user, email))
    }
    .await;
    let (user, email) = match profile {
        Ok(profile) => profile,
        Err(err) => {
            tracing::warn!(error = ?err, "GitHub sign-in failed");
            return Ok((jar, Redirect::to("/signin?error=provider")).into_response());
        }
    };
    let subject = user.id.to_string();

    let existing: Option<Uuid> = sqlx::query_scalar(
        "update identities set last_sign_in_at = now()
         where provider = 'github' and provider_subject = $1 returning user_id",
    )
    .bind(&subject)
    .fetch_optional(&state.db)
    .await?;

    if let Some(user_id) = existing {
        let user_agent = headers.get(USER_AGENT).and_then(|v| v.to_str().ok());
        let cookie = session::create(&state.db, &state.config, user_id, user_agent).await?;
        return Ok((jar.add(cookie), Redirect::to(&return_to)).into_response());
    }

    // New to Slop: hold the provider details until the user finishes sign-up.
    let signup_token = new_token();
    let suggested_name: String = user
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or(user.login)
        .chars()
        .take(40)
        .collect();
    sqlx::query("delete from pending_signups where expires_at < now()")
        .execute(&state.db)
        .await?;
    sqlx::query(
        "insert into pending_signups
             (token_hash, provider, provider_subject, suggested_name, email, return_to, expires_at)
         values ($1, 'github', $2, $3, $4, $5, now() + $6)",
    )
    .bind(hash_token(&signup_token))
    .bind(&subject)
    .bind(&suggested_name)
    .bind(&email)
    .bind(&return_to)
    .bind(SIGNUP_LIFETIME)
    .execute(&state.db)
    .await?;

    let jar = jar.add(build_cookie(
        &state.config,
        SIGNUP_COOKIE,
        signup_token,
        SIGNUP_LIFETIME,
    ));
    Ok((jar, Redirect::to("/signup")).into_response())
}

#[derive(Serialize, sqlx::FromRow)]
struct Country {
    code: String,
    name: String,
    min_age: i16,
}

async fn signup_countries(state: &AppState) -> ApiResult<Vec<Country>> {
    Ok(sqlx::query_as(
        "select country_code as code, name, min_age from country_policies
         where signups_allowed order by name",
    )
    .fetch_all(&state.db)
    .await?)
}

async fn countries(State(state): State<AppState>) -> ApiResult<Json<Vec<Country>>> {
    Ok(Json(signup_countries(&state).await?))
}

#[derive(Serialize)]
struct SignupInfo {
    suggested_name: String,
    email: Option<String>,
    countries: Vec<Country>,
}

async fn signup_info(State(state): State<AppState>, jar: CookieJar) -> ApiResult<Json<SignupInfo>> {
    let token = jar.get(SIGNUP_COOKIE).ok_or(ApiError::NotFound)?;
    let pending: Option<(String, Option<String>)> = sqlx::query_as(
        "select suggested_name, email from pending_signups where token_hash = $1 and expires_at > now()",
    )
    .bind(hash_token(token.value()))
    .fetch_optional(&state.db)
    .await?;
    let (suggested_name, email) = pending.ok_or(ApiError::NotFound)?;
    Ok(Json(SignupInfo {
        suggested_name,
        email,
        countries: signup_countries(&state).await?,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignupRequest {
    display_name: String,
    country_code: String,
    /// The user confirms they are at least their country's minimum age (16 everywhere today).
    age_confirmed: bool,
}

#[derive(Serialize)]
struct SignupResponse {
    user: Me,
    return_to: String,
}

pub fn validate_display_name(name: &str) -> ApiResult<String> {
    let name = name.trim();
    let len = name.chars().count();
    if len == 0 || len > 40 {
        return Err(ApiError::invalid("display_name", "use 1 to 40 characters"));
    }
    if name.chars().any(char::is_control) {
        return Err(ApiError::invalid(
            "display_name",
            "contains characters that aren't allowed",
        ));
    }
    Ok(name.to_string())
}

async fn signup(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(req): Json<SignupRequest>,
) -> ApiResult<(StatusCode, CookieJar, Json<SignupResponse>)> {
    let token = jar
        .get(SIGNUP_COOKIE)
        .ok_or(ApiError::Unauthorized)?
        .value()
        .to_string();
    let display_name = validate_display_name(&req.display_name)?;
    let country_code = req.country_code.trim().to_ascii_uppercase();
    let min_age: Option<i16> = sqlx::query_scalar(
        "select min_age from country_policies where country_code = $1 and signups_allowed",
    )
    .bind(&country_code)
    .fetch_optional(&state.db)
    .await?;
    let Some(min_age) = min_age else {
        return Err(ApiError::invalid(
            "country_code",
            "Slop isn't available in this country yet",
        ));
    };
    if !req.age_confirmed {
        return Err(ApiError::invalid(
            "age_confirmed",
            format!("you must be {min_age} or older to use Slop"),
        ));
    }

    let mut tx = state.db.begin().await?;
    let pending: Option<(String, String, Option<String>, String)> = sqlx::query_as(
        "delete from pending_signups where token_hash = $1 and expires_at > now()
         returning provider, provider_subject, email, return_to",
    )
    .bind(hash_token(&token))
    .fetch_optional(&mut *tx)
    .await?;
    let (provider, subject, email, return_to) = pending.ok_or(ApiError::Unauthorized)?;

    let user_id = Uuid::now_v7();
    sqlx::query(
        "insert into users (id, display_name, email, country_code, age_16_confirmed_at)
         values ($1, $2, $3, $4, now())",
    )
    .bind(user_id)
    .bind(&display_name)
    .bind(&email)
    .bind(&country_code)
    .execute(&mut *tx)
    .await?;
    sqlx::query("insert into identities (provider, provider_subject, user_id) values ($1, $2, $3)")
        .bind(&provider)
        .bind(&subject)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| {
            if is_unique_violation(&err) {
                ApiError::Conflict("this sign-in already has a Slop account".into())
            } else {
                err.into()
            }
        })?;
    tx.commit().await?;

    let user_agent = headers.get(USER_AGENT).and_then(|v| v.to_str().ok());
    let cookie = session::create(&state.db, &state.config, user_id, user_agent).await?;
    let jar = jar
        .remove(removal_cookie(&state.config, SIGNUP_COOKIE))
        .add(cookie);
    let user = Me {
        id: user_id,
        display_name,
        email,
        country_code,
    };
    Ok((
        StatusCode::CREATED,
        jar,
        Json(SignupResponse { user, return_to }),
    ))
}

async fn signout(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    jar: CookieJar,
) -> ApiResult<(StatusCode, CookieJar)> {
    if let Some(user) = user {
        sqlx::query("update sessions set revoked_at = now() where id = $1")
            .bind(user.session_id)
            .execute(&state.db)
            .await?;
    }
    let jar = jar.remove(removal_cookie(
        &state.config,
        session::cookie_name(&state.config),
    ));
    Ok((StatusCode::NO_CONTENT, jar))
}

#[derive(Serialize, sqlx::FromRow)]
struct Me {
    id: Uuid,
    display_name: String,
    email: Option<String>,
    country_code: String,
}

async fn me(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Me>> {
    let me =
        sqlx::query_as("select id, display_name, email, country_code from users where id = $1")
            .bind(user.id)
            .fetch_one(&state.db)
            .await?;
    Ok(Json(me))
}

#[derive(Serialize, sqlx::FromRow)]
struct SessionInfo {
    id: Uuid,
    /// "web" or "desktop".
    kind: String,
    user_agent: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    last_seen_at: OffsetDateTime,
    current: bool,
}

async fn list_sessions(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Vec<SessionInfo>>> {
    let sql = format!(
        "select s.id, s.kind, s.user_agent, s.created_at, s.last_seen_at, s.id = $4 as current
         from sessions s where s.user_id = $3 and {}
         order by s.last_seen_at desc",
        session::live_session_sql("$1", "$2")
    );
    let sessions = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(session::IDLE_TIMEOUT)
        .bind(session::DESKTOP_IDLE_TIMEOUT)
        .bind(user.id)
        .bind(user.session_id)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(sessions))
}

async fn revoke_session(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let revoked = sqlx::query(
        "update sessions set revoked_at = now() where id = $1 and user_id = $2 and revoked_at is null",
    )
    .bind(id)
    .bind(user.id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if revoked == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_to_rejects_offsite_paths() {
        assert_eq!(safe_return_to(Some("/apps/x?y=1")), "/apps/x?y=1");
        for bad in [
            "https://evil.example",
            "//evil.example",
            "/\\evil.example",
            "apps",
            "/a\nb",
        ] {
            assert_eq!(safe_return_to(Some(bad)), "/", "{bad}");
        }
        assert_eq!(safe_return_to(None), "/");
    }
}
