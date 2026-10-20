//! Identity: sign-in with GitHub, sign-up, sign-out and the devices page (SECURITY §4).

pub mod desktop;
pub mod github;
pub mod merge;
pub mod oauth;
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
use oauth::Provider;
use session::{CurrentUser, MaybeUser, build_cookie, removal_cookie};

const OAUTH_COOKIE: &str = "slop_oauth_state";
const SIGNUP_COOKIE: &str = "slop_signup";
const OAUTH_FLOW_LIFETIME: Duration = Duration::minutes(10);
const SIGNUP_LIFETIME: Duration = Duration::minutes(30);

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/providers", get(providers))
        .route("/auth/{provider}/start", get(start))
        .route("/auth/{provider}/link", get(link_start))
        .route("/auth/{provider}/callback", get(callback))
        .route("/me/identities", get(identities))
        .route("/me/identities/{provider}/{subject}", delete(unlink))
        .merge(merge::routes())
        .route("/auth/signup", get(signup_info).post(signup))
        .route("/auth/signout", post(signout))
        .route("/countries", get(countries))
        .route("/me", get(me).patch(crate::pulse::update_settings))
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

#[derive(Serialize)]
struct Providers {
    github: bool,
    google: bool,
    gitlab: bool,
    bitbucket: bool,
}

/// Which sign-in providers this server has set up.
async fn providers(State(state): State<AppState>) -> Json<Providers> {
    Json(Providers {
        github: Provider::Github.enabled(&state),
        google: Provider::Google.enabled(&state),
        gitlab: Provider::Gitlab.enabled(&state),
        bitbucket: Provider::Bitbucket.enabled(&state),
    })
}

#[derive(Deserialize)]
struct StartQuery {
    return_to: Option<String>,
}

fn provider_from(name: &str, state: &AppState) -> ApiResult<Provider> {
    let p = Provider::parse(name).ok_or(ApiError::NotFound)?;
    if !p.enabled(state) {
        return Err(ApiError::Unavailable(
            "that sign-in isn't set up on this server",
        ));
    }
    Ok(p)
}

/// Starts an OAuth flow; `link` ties it to the signed-in account and session.
async fn begin(
    state: &AppState,
    jar: CookieJar,
    provider: Provider,
    return_to: Option<&str>,
    link: Option<&CurrentUser>,
) -> ApiResult<(CookieJar, Redirect)> {
    let oauth_state = new_token();
    let verifier = new_token();
    sqlx::query("delete from oauth_flows where expires_at < now()")
        .execute(&state.db)
        .await?;
    sqlx::query(
        "insert into oauth_flows (state_hash, provider, pkce_verifier, return_to, expires_at, link_user_id, link_session_id)
         values ($1, $2, $3, $4, now() + $5, $6, $7)",
    )
    .bind(hash_token(&oauth_state))
    .bind(provider.name())
    .bind(&verifier)
    .bind(safe_return_to(return_to))
    .bind(OAUTH_FLOW_LIFETIME)
    .bind(link.map(|u| u.id))
    .bind(link.map(|u| u.session_id))
    .execute(&state.db)
    .await?;
    let url = oauth::authorize_url(state, provider, &oauth_state, &pkce_challenge(&verifier))
        .ok_or(ApiError::Unavailable(
            "that sign-in isn't set up on this server",
        ))?;
    let jar = jar.add(build_cookie(
        &state.config,
        OAUTH_COOKIE,
        oauth_state,
        OAUTH_FLOW_LIFETIME,
    ));
    Ok((jar, Redirect::to(url.as_str())))
}

async fn start(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(provider): Path<String>,
    Query(query): Query<StartQuery>,
) -> ApiResult<(CookieJar, Redirect)> {
    let provider = provider_from(&provider, &state)?;
    begin(&state, jar, provider, query.return_to.as_deref(), None).await
}

/// Linking another sign-in to the signed-in account (SECURITY §4.1): only from a signed-in
/// session, after a recent sign-in, and only by signing in to the account being linked.
async fn link_start(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    jar: CookieJar,
    Path(provider_name): Path<String>,
    Query(query): Query<StartQuery>,
) -> ApiResult<Response> {
    let provider = provider_from(&provider_name, &state)?;
    let back = safe_return_to(Some(
        query.return_to.as_deref().unwrap_or("/account#sign-ins"),
    ));
    let fresh = match &user {
        Some(u) => session::require_recent_sign_in(&state.db, u).await.is_ok(),
        None => false,
    };
    if !fresh {
        // Sign in (again) first, then come straight back here to link.
        let here: String = format!(
            "/v1/auth/{provider_name}/link?return_to={}",
            url::form_urlencoded::byte_serialize(back.as_bytes()).collect::<String>()
        );
        let to = format!(
            "/signin?reason=link&return_to={}",
            url::form_urlencoded::byte_serialize(here.as_bytes()).collect::<String>()
        );
        return Ok(Redirect::to(&to).into_response());
    }
    let user = user.expect("checked above");
    Ok(begin(&state, jar, provider, Some(&back), Some(&user))
        .await?
        .into_response())
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

fn with_param(path: &str, param: &str) -> String {
    let (base, hash) = path
        .split_once('#')
        .map_or((path, None), |(b, h)| (b, Some(h)));
    let sep = if base.contains('?') { '&' } else { '?' };
    match hash {
        Some(h) => format!("{base}{sep}{param}#{h}"),
        None => format!("{base}{sep}{param}"),
    }
}

async fn callback(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    MaybeUser(signed_in): MaybeUser,
    Path(provider): Path<String>,
    Query(query): Query<CallbackQuery>,
) -> ApiResult<Response> {
    let provider = provider_from(&provider, &state)?;
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
    let flow: Option<(String, String, Option<Uuid>, Option<Uuid>)> = sqlx::query_as(
        "delete from oauth_flows where state_hash = $1 and provider = $2 and expires_at > now()
         returning pkce_verifier, return_to, link_user_id, link_session_id",
    )
    .bind(hash_token(&query_state))
    .bind(provider.name())
    .fetch_optional(&state.db)
    .await?;
    let Some((verifier, return_to, link_user, link_session)) = flow else {
        return Err(ApiError::invalid(
            "state",
            "sign-in expired, please try again",
        ));
    };

    let profile = match oauth::profile(&state, provider, &code, &verifier).await {
        Ok(profile) => profile,
        Err(err) => {
            tracing::warn!(error = ?err, provider = provider.name(), "sign-in failed");
            let back = if link_user.is_some() {
                return_to.as_str()
            } else {
                "/signin"
            };
            return Ok((jar, Redirect::to(&with_param(back, "error=provider"))).into_response());
        }
    };
    let existing: Option<Uuid> = sqlx::query_scalar(
        "update identities set last_sign_in_at = now(), login = coalesce($3, login)
         where provider = $1 and provider_subject = $2 returning user_id",
    )
    .bind(provider.name())
    .bind(&profile.subject)
    .bind(&profile.login)
    .fetch_optional(&state.db)
    .await?;

    // Linking: still the same account and session that started it.
    if let Some(into) = link_user {
        let same = signed_in
            .as_ref()
            .is_some_and(|u| u.id == into && Some(u.session_id) == link_session);
        if !same {
            return Ok((jar, Redirect::to("/signin?error=link_expired")).into_response());
        }
        return match existing {
            None => {
                sqlx::query(
                    "insert into identities (provider, provider_subject, user_id, login) values ($1, $2, $3, $4)",
                )
                .bind(provider.name())
                .bind(&profile.subject)
                .bind(into)
                .bind(&profile.login)
                .execute(&state.db)
                .await?;
                Ok((
                    jar,
                    Redirect::to(&with_param(
                        &return_to,
                        &format!("linked={}", provider.name()),
                    )),
                )
                    .into_response())
            }
            Some(owner) if owner == into => {
                Ok((jar, Redirect::to(&with_param(&return_to, "linked=already"))).into_response())
            }
            Some(other) => {
                // That sign-in has its own Slop account: offer to combine the two.
                let token = new_token();
                sqlx::query("delete from pending_merges where expires_at < now()")
                    .execute(&state.db)
                    .await?;
                sqlx::query(
                    "insert into pending_merges (token_hash, into_user_id, from_user_id, session_id, provider, provider_subject, expires_at)
                     values ($1, $2, $3, $4, $5, $6, now() + $7)",
                )
                .bind(hash_token(&token))
                .bind(into)
                .bind(other)
                .bind(link_session)
                .bind(provider.name())
                .bind(&profile.subject)
                .bind(merge::CONFIRM_WITHIN)
                .execute(&state.db)
                .await?;
                let jar = jar.add(build_cookie(
                    &state.config,
                    merge::COOKIE,
                    token,
                    merge::CONFIRM_WITHIN,
                ));
                Ok((jar, Redirect::to("/account/combine")).into_response())
            }
        };
    }

    if let Some(user_id) = existing {
        // A closed account's sign-in stays refused (DESIGN §4).
        let closed: bool =
            sqlx::query_scalar("select closed_at is not null from users where id = $1")
                .bind(user_id)
                .fetch_one(&state.db)
                .await?;
        if closed {
            return Ok((jar, Redirect::to("/signin?error=closed")).into_response());
        }
        let user_agent = headers.get(USER_AGENT).and_then(|v| v.to_str().ok());
        let cookie = session::create(&state.db, &state.config, user_id, user_agent).await?;
        return Ok((jar.add(cookie), Redirect::to(&return_to)).into_response());
    }

    // New to Slop: hold the provider details until the user finishes sign-up.
    let signup_token = new_token();
    let suggested_name: String = profile.name.chars().take(40).collect();
    sqlx::query("delete from pending_signups where expires_at < now()")
        .execute(&state.db)
        .await?;
    sqlx::query(
        "insert into pending_signups
             (token_hash, provider, provider_subject, suggested_name, email, return_to, expires_at, login)
         values ($1, $2, $3, $4, $5, $6, now() + $7, $8)",
    )
    .bind(hash_token(&signup_token))
    .bind(provider.name())
    .bind(&profile.subject)
    .bind(&suggested_name)
    .bind(&profile.email)
    .bind(&return_to)
    .bind(SIGNUP_LIFETIME)
    .bind(&profile.login)
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
struct LinkedIdentity {
    provider: String,
    provider_subject: String,
    login: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    last_sign_in_at: OffsetDateTime,
}

/// The sign-ins linked to the account (Account → Sign-ins).
async fn identities(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Vec<LinkedIdentity>>> {
    Ok(Json(
        sqlx::query_as(
            "select provider, provider_subject, login, created_at, last_sign_in_at from identities
             where user_id = $1 order by created_at",
        )
        .bind(user.id)
        .fetch_all(&state.db)
        .await?,
    ))
}

/// Unlinks a sign-in. The last one stays (the account needs a way in), and it needs a recent
/// sign-in, so a stolen session can't lock the owner out.
async fn unlink(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((provider, subject)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    session::require_recent_sign_in(&state.db, &user).await?;
    let mut tx = state.db.begin().await?;
    let count: i64 = sqlx::query_scalar("select count(*) from identities where user_id = $1")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await?;
    let done = sqlx::query(
        "delete from identities where user_id = $1 and provider = $2 and provider_subject = $3",
    )
    .bind(user.id)
    .bind(&provider)
    .bind(&subject)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if done == 0 {
        return Err(ApiError::NotFound);
    }
    if count <= 1 {
        return Err(ApiError::Conflict(
            "that's your only way to sign in: link another first".into(),
        ));
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
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
    /// The user accepts the Terms of Service in effect (legal.rs).
    #[serde(default)]
    accept_terms: bool,
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

    if !req.accept_terms {
        return Err(ApiError::invalid(
            "accept_terms",
            "accept the Terms of Service to create an account",
        ));
    }

    let mut tx = state.db.begin().await?;
    type Pending = (String, String, Option<String>, String, Option<String>);
    let pending: Option<Pending> = sqlx::query_as(
        "delete from pending_signups where token_hash = $1 and expires_at > now()
         returning provider, provider_subject, email, return_to, login",
    )
    .bind(hash_token(&token))
    .fetch_optional(&mut *tx)
    .await?;
    let (provider, subject, email, return_to, login) = pending.ok_or(ApiError::Unauthorized)?;

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
    sqlx::query("insert into identities (provider, provider_subject, user_id, login) values ($1, $2, $3, $4)")
        .bind(&provider)
        .bind(&subject)
        .bind(user_id)
        .bind(&login)
        .execute(&mut *tx)
        .await
        .map_err(|err| {
            if is_unique_violation(&err) {
                ApiError::Conflict("this sign-in already has a Slop account".into())
            } else {
                err.into()
            }
        })?;
    if let Some(version) = crate::legal::current_version(&mut tx, "terms").await? {
        crate::legal::record_acceptance(&mut tx, user_id, "terms", &version).await?;
    }
    tx.commit().await?;

    let user_agent = headers.get(USER_AGENT).and_then(|v| v.to_str().ok());
    let cookie = session::create(&state.db, &state.config, user_id, user_agent).await?;
    let jar = jar
        .remove(removal_cookie(&state.config, SIGNUP_COOKIE))
        .add(cookie);
    // Recommendations default to on, except where the under-18 view applies (PRIV-6).
    let under18: Option<bool> = sqlx::query_scalar(
        "select under18_view_default from country_policies where country_code = $1",
    )
    .bind(&country_code)
    .fetch_optional(&state.db)
    .await?;
    let user = Me {
        id: user_id,
        display_name,
        email,
        country_code,
        is_admin: false,
        share_playtime: true,
        personalize: under18 == Some(false),
        adult_confirmed: false,
        suspended: false,
        new_decisions: 0,
        unread_notifications: 0,
        profile_public: under18 == Some(false),
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
    /// Slop staff: the UI shows the admin panel.
    is_admin: bool,
    /// "Share my playtime" (PRIV-9).
    share_playtime: bool,
    /// "Recommendations based on my activity" (LEGAL §7), with its default applied.
    #[sqlx(skip)]
    personalize: bool,
    /// Confirmed 18+ for People (LEGAL §6), so the UI only asks once.
    adult_confirmed: bool,
    /// Suspended by a moderation decision (moderation.rs).
    suspended: bool,
    /// Moderation decisions about the user they haven't opened yet.
    #[sqlx(skip)]
    new_decisions: i64,
    /// Unread notifications (notify.rs), for the bell.
    #[sqlx(skip)]
    unread_notifications: i64,
    /// Votes and reviews on the public profile (reviews.rs), with its default applied.
    #[sqlx(skip)]
    profile_public: bool,
}

async fn me(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: axum::http::HeaderMap,
) -> ApiResult<Json<Me>> {
    let mut me: Me = sqlx::query_as(
        "select id, display_name, email, country_code, is_admin, share_playtime,
                adult_confirmed_at is not null as adult_confirmed, suspended_at is not null as suspended
         from users where id = $1",
    )
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;
    let policy =
        crate::policy::ViewerPolicy::resolve(&state.db, &state.config, Some(&user), &headers)
            .await?;
    me.personalize = crate::foryou::personalized(&state.db, user.id, policy).await?;
    me.new_decisions = crate::moderation::unseen(&state.db, user.id).await?;
    me.unread_notifications = crate::notify::unread(&state.db, user.id).await?;
    me.profile_public = crate::reviews::is_public(&state.db, user.id).await?;
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
