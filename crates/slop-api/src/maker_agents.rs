// SPDX-License-Identifier: AGPL-3.0-or-later
//! Makers' AI agents (DESIGN §8, plan phase 10). A maker makes a token (Account → AI agents) and
//! gives it to their coding agent, which can then publish and fix their apps from the editor:
//! create and edit store pages, submit releases with notes, read SLAPorSLOP's checks of each release,
//! and get a ready-made fix for every problem found (fixes.rs).
//!
//! The thinking happens in the maker's agent, never on SLAPorSLOP's servers: SLAPorSLOP gives it tools, data
//! and instructions, and calls no AI model.
//!
//! - **Tokens** (`slopmk_…`) are sessions of kind `agent`, so they show on the maker's devices
//!   page and sign out like any session. Each has a scope (`read`, or `publish` to also change
//!   store pages and submit releases), an optional list of apps, and expires when its owner
//!   chose: a day to a year (`MAX_DAYS`). It has no idle limit (auth/session.rs), so it works
//!   until then. The owner is told a week and a day before it expires, and when it has
//!   (`warn_expiring`, hourly), and the agent sees its expiry in `whats_new` and `maker_guide`
//!   so it can tell them too. A replacement is a new token with the same settings; the old one
//!   keeps working until its own expiry or until it's removed.
//! - **Only maker work:** a token may call the routes in `ALLOWED` and nothing else: never
//!   money, sign-ins, tokens, chat, People or account settings. Releases still go through SLAPorSLOP's
//!   scans and a person's review.
//! - **Making one** needs only a signed-in session, not a recent sign-in: what a token can do is
//!   limited enough that the owner chose not to ask "confirm it's you" for it. A `publish` token
//!   needs the Publisher Agreement accepted (`terms_required` until it is), since the agent
//!   can't accept it; if a new version takes effect later, the agent's tools get
//!   `terms_required` until the owner accepts it on the AI agents page.
//! - **Writing to SLAPorSLOP's admins** (admin_messages.rs) works with any token, read or publish, for
//!   some apps or all: it's limited per account instead (once an hour, five a day).
//! - **MCP:** `POST /v1/maker/mcp` (and `/mcp`, the short address for AI tools) speaks the Model
//!   Context Protocol, with the tools in `TOOLS`, each calling the API above with the token.
//! - **Approved in the browser:** an AI tool can also connect without anyone handling a token
//!   (agent_oauth.rs): what the person approves is a session like these, with an access token
//!   (`access_tokens`, also `slopmk_…`) the tool refreshes itself.

use axum::{
    Json, Router,
    extract::{Path, Request, State},
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{
    AppState,
    agent::{Prompt, Tool, s},
    auth::session::{self, CurrentUser},
    error::{ApiError, ApiResult},
    tokens::{hash_token, new_token},
};

/// Maker tokens start with this, so they're recognisable (and secret scanners can find them).
pub const PREFIX: &str = "slopmk_";
/// The longest a token lives: a year.
pub const MAX_DAYS: i64 = 365;
/// A token's lifetime when none is given.
const DEFAULT_DAYS: i64 = 30;
/// Tokens that expire or expired within this are listed (expired ones until cleanup deletes them).
const LISTED_AFTER_EXPIRY: Duration = crate::cleanup::ENDED_SESSION_RETENTION;
/// Live tokens per maker (approved AI tools included).
pub const MAX_TOKENS: i64 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Read,
    Publish,
}

/// What an agent's token may do.
#[derive(Clone, Debug)]
pub struct AgentToken {
    pub scope: Scope,
    /// The apps it may touch; empty: all the maker's apps, and creating new ones.
    pub app_ids: Vec<Uuid>,
    /// People access: jobs (requests, quotes, changes, the job conversation), the person's own
    /// People page and portfolio, and the day's shelves.
    pub people: bool,
}

/// How a route's `{slug}` relates to the token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slug {
    /// The token's own app: it must be one of the token's apps.
    Own,
    /// Any app the maker can see: reading it, or making a new app from it (a community build or
    /// a mod). Making something new needs a token for all the maker's apps.
    Any,
    /// People work (`*` is a job): only with a token allowed to handle People jobs. Each route
    /// checks the user is a side of the job.
    People,
    /// Something on one of the maker's apps found by its own ID (`*` is a bug report): the route
    /// checks the app is one of the token's (bug_reports.rs `load`), so a token for some apps
    /// works on those apps' reports.
    Checked,
    /// Claiming a bounty: the route checks the app it's claimed for is one of the token's apps
    /// (it's named in the body).
    Claim,
}

/// (method, path after `/v1`, needs `publish`, what `{slug}` is). `*` is any one segment.
const ALLOWED: &[(&str, &str, bool, Slug)] = &[
    ("GET", "/me", false, Slug::Any),
    ("GET", "/me/apps", false, Slug::Any),
    ("GET", "/me/notifications", false, Slug::Any),
    ("GET", "/me/next-steps", false, Slug::Any),
    ("GET", "/categories", false, Slug::Any),
    ("GET", "/runtimes", false, Slug::Any),
    ("GET", "/maker/guide", false, Slug::Any),
    ("GET", "/maker/publishing-guide", false, Slug::Any),
    ("GET", "/maker/publishing-guide/*", false, Slug::Any),
    ("POST", "/maker/mcp", false, Slug::Any),
    ("GET", "/maker/mcp", false, Slug::Any),
    // The same endpoint at the short address AI tools are given (agent_oauth.rs).
    ("POST", "/mcp", false, Slug::Any),
    ("GET", "/mcp", false, Slug::Any),
    ("GET", "/apps/{slug}", false, Slug::Any),
    ("GET", "/apps/{slug}/manage", false, Slug::Own),
    ("GET", "/apps/{slug}/releases", false, Slug::Own),
    ("GET", "/apps/{slug}/fixes", false, Slug::Own),
    // A dry run: it reads the repository and stores nothing (fixes.rs).
    ("GET", "/apps/{slug}/source-check", false, Slug::Own),
    ("GET", "/apps/{slug}/reviews", false, Slug::Own),
    ("GET", "/apps/{slug}/ratings", false, Slug::Own),
    ("GET", "/apps/{slug}/history", false, Slug::Own),
    ("GET", "/apps/{slug}/slop-build", false, Slug::Own),
    ("GET", "/apps/{slug}/github-releases", false, Slug::Own),
    ("GET", "/apps/{slug}/update-settings", false, Slug::Own),
    ("GET", "/apps/{slug}/media", false, Slug::Own),
    ("GET", "/apps/{slug}/offers", false, Slug::Own),
    ("POST", "/apps", true, Slug::Any),
    ("POST", "/apps/{slug}/branches", true, Slug::Any),
    ("POST", "/apps/{slug}/servers", true, Slug::Own),
    ("PATCH", "/apps/{slug}", true, Slug::Own),
    ("POST", "/apps/{slug}/changes", true, Slug::Own),
    ("POST", "/apps/{slug}/changes/*/cancel", true, Slug::Own),
    ("POST", "/apps/{slug}/releases", true, Slug::Own),
    ("POST", "/apps/{slug}/releases/from-github", true, Slug::Own),
    ("PATCH", "/apps/{slug}/releases/*", true, Slug::Own),
    ("POST", "/apps/{slug}/releases/*/withdraw", true, Slug::Own),
    ("POST", "/apps/{slug}/repo-check", true, Slug::Own),
    ("POST", "/apps/{slug}/stage", true, Slug::Own),
    ("PUT", "/apps/{slug}/commit-status", true, Slug::Own),
    ("POST", "/apps/{slug}/media/check", true, Slug::Own),
    ("POST", "/apps/{slug}/offers", true, Slug::Own),
    ("PUT", "/apps/{slug}/reviews/*/reply", true, Slug::Own),
    ("GET", "/apps/{slug}/bounties", false, Slug::Any),
    ("GET", "/apps/{slug}/bug-reports", false, Slug::Own),
    ("GET", "/apps/{slug}/release-feedback", false, Slug::Own),
    ("GET", "/apps/{slug}/demo", false, Slug::Own),
    ("POST", "/apps/{slug}/demo", true, Slug::Own),
    ("DELETE", "/apps/{slug}/demo", true, Slug::Own),
    // Each route checks the report is on the user's own app, and one of the token's apps.
    ("GET", "/bug-reports/*", false, Slug::Checked),
    ("PATCH", "/bug-reports/*", true, Slug::Checked),
    ("GET", "/bounties", false, Slug::Any),
    ("GET", "/bounties/*", false, Slug::Any),
    // Claiming pays the maker; it's checked by the app's maker or the backers.
    ("POST", "/bounties/*/claims", true, Slug::Claim),
    ("DELETE", "/apps/{slug}/reviews/*/reply", true, Slug::Own),
    // News on the app's store page (news.rs): the maker's own posts, with the hidden ones.
    ("GET", "/apps/{slug}/news", false, Slug::Own),
    ("POST", "/apps/{slug}/news", true, Slug::Own),
    ("PUT", "/apps/{slug}/news/*", true, Slug::Own),
    ("DELETE", "/apps/{slug}/news/*", true, Slug::Own),
    ("POST", "/apps/{slug}/offers/*/answer", true, Slug::Own),
    ("POST", "/apps/{slug}/offers/*/withdraw", true, Slug::Own),
    // People: never accepting agreements or changes, paying, signing off, delivering,
    // withdrawing or cancelling. Those stay with the person.
    ("GET", "/me/jobs", false, Slug::People),
    ("GET", "/me/creative", false, Slug::People),
    // The portfolio: where it stands, and reading it again from the repository, which queues
    // what's new for a person's review, like an app's pictures. Changing a repository the person
    // set (portfolio.rs refuses it from a token), the works' order and the page itself stay
    // with the person.
    ("GET", "/me/creative/portfolio", false, Slug::People),
    ("POST", "/me/creative/portfolio/check", true, Slug::People),
    ("GET", "/jobs/*", false, Slug::People),
    ("GET", "/jobs/*/conversation", false, Slug::People),
    ("POST", "/jobs/*/offer", true, Slug::People),
    ("POST", "/jobs/*/decline", true, Slug::People),
    ("POST", "/jobs/*/change/request", true, Slug::People),
    ("POST", "/jobs/*/change/offer", true, Slug::People),
    ("POST", "/jobs/*/change/decline", true, Slug::People),
    ("POST", "/jobs/*/conversation/request", true, Slug::People),
    ("POST", "/jobs/*/conversation/open", true, Slug::People),
    ("POST", "/jobs/*/conversation/close", true, Slug::People),
    ("POST", "/jobs/*/messages", true, Slug::People),
    // The People directory and its pages are public: any token may look, so a maker's agent can
    // find someone to hire. Asking for a quote stays with the person (`/people/*/hire` isn't here).
    ("GET", "/people", false, Slug::Any),
    ("GET", "/people/skills", false, Slug::Any),
    // The day's shelves include "Suggested for you", which says what the viewer has hired for:
    // only with People access. It's here, before the page, so the page's `*` doesn't take it.
    ("GET", "/people/discover", false, Slug::People),
    ("GET", "/people/*", false, Slug::Any),
    // Writing to SLAPorSLOP's admins: any token, since it changes nothing of the maker's. It has its
    // own limits per account (admin_messages.rs).
    ("GET", "/staff-contacts", false, Slug::Any),
    ("GET", "/admin-messages", false, Slug::Any),
    ("POST", "/admin-messages", false, Slug::Any),
    ("POST", "/admin-messages/*/reply", false, Slug::Any),
];

/// The allowed route a request matches: whether it needs `publish`, how its slug is checked,
/// and the slug (for a bug report, the report's ID).
fn route<'a>(method: &Method, path: &'a str) -> Option<(bool, Slug, Option<&'a str>)> {
    // Handlers inside the `/v1` router see the path without it.
    let path = path.strip_prefix("/v1").unwrap_or(path);
    let parts: Vec<&str> = path.split('/').collect();
    ALLOWED.iter().find_map(|(m, pattern, publish, kind)| {
        if *m != method.as_str() {
            return None;
        }
        let want: Vec<&str> = pattern.split('/').collect();
        if want.len() != parts.len() {
            return None;
        }
        let mut slug = None;
        for (w, p) in want.iter().zip(&parts) {
            match *w {
                "{slug}" if !p.is_empty() => slug = Some(*p),
                "*" if !p.is_empty() => {}
                w if w == *p && !w.starts_with('{') && w != "*" => {}
                _ => return None,
            }
        }
        Some((*publish, *kind, slug))
    })
}

impl AgentToken {
    /// Refuses anything outside maker work, the token's scope, or its apps.
    pub async fn allows(&self, state: &AppState, method: &Method, path: &str) -> ApiResult<()> {
        let Some((publish, kind, slug)) = route(method, path) else {
            return Err(ApiError::Forbidden);
        };
        if publish && self.scope != Scope::Publish {
            return Err(ApiError::Forbidden);
        }
        if kind == Slug::People {
            return if self.people {
                Ok(())
            } else {
                Err(ApiError::Forbidden)
            };
        }
        if self.app_ids.is_empty() {
            return Ok(());
        }
        match (kind, slug) {
            // A token for some apps only can't make new ones.
            (Slug::Any, _) if publish => Err(ApiError::Forbidden),
            (Slug::Any | Slug::People | Slug::Checked | Slug::Claim, _) | (Slug::Own, None) => {
                Ok(())
            }
            (Slug::Own, Some(slug)) => {
                let id: Option<Uuid> = sqlx::query_scalar("select id from apps where slug = $1")
                    .bind(slug)
                    .fetch_optional(&state.db)
                    .await?;
                match id {
                    Some(id) if self.app_ids.contains(&id) => Ok(()),
                    _ => Err(ApiError::NotFound),
                }
            }
        }
    }
}

/// The maker a `slopmk_` token signs in as, with its limits: a token made on the website (the
/// session's own), or an approved AI tool's access token (agent_oauth.rs).
pub async fn lookup(state: &AppState, token: &str) -> ApiResult<Option<CurrentUser>> {
    let live = session::live_session_sql("$2", "$3");
    type Row = (
        Uuid,
        Uuid,
        String,
        String,
        bool,
        String,
        Vec<Uuid>,
        bool,
        bool,
    );
    let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select u.id, s.id, u.display_name, u.country_code, u.suspended_at is not null, m.scope,
                m.app_ids, m.people, s.last_seen_at < now() - $4
         from sessions s join users u on u.id = s.user_id join maker_tokens m on m.session_id = s.id
         where s.id = coalesce(
                   (select t.session_id from access_tokens t where t.token_hash = $1 and t.expires_at > now()),
                   (select x.id from sessions x where x.token_hash = $1))
           and s.kind = 'agent' and u.closed_at is null and {live}"
    )))
    .bind(hash_token(token))
    .bind(session::IDLE_TIMEOUT)
    .bind(session::DESKTOP_IDLE_TIMEOUT)
    .bind(session::TOUCH_INTERVAL)
    .fetch_optional(&state.db)
    .await?;
    let Some((
        id,
        session_id,
        display_name,
        country_code,
        suspended,
        scope,
        app_ids,
        people,
        stale,
    )) = row
    else {
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
        agent: Some(AgentToken {
            scope: if scope == "publish" {
                Scope::Publish
            } else {
                Scope::Read
            },
            app_ids,
            people,
        }),
    }))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/maker-tokens", get(list).post(create))
        .route("/me/maker-tokens/{id}", delete(revoke))
        .route("/maker/guide", get(guide))
        .route("/maker/publishing-guide", get(guide_sections))
        .route("/maker/publishing-guide/{section}", get(guide_section))
        .route("/maker/mcp", post(mcp).get(mcp_get))
}

/// The MCP endpoint at `/mcp`, outside `/v1`: the address AI tools are given.
pub fn short_mcp_routes() -> Router<AppState> {
    Router::new().route("/mcp", post(mcp).get(mcp_get))
}

// ---------------------------------------------------------------------------------------------
// Tokens

#[derive(Serialize)]
struct TokenInfo {
    id: Uuid,
    name: String,
    scope: String,
    /// App addresses; empty: all the maker's apps.
    apps: Vec<String>,
    /// Has People access.
    people: bool,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    expires_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    last_used_at: OffsetDateTime,
    /// Past its expiry: it no longer works (listed for a week, to be replaced or removed).
    expired: bool,
    /// Approved in the browser (agent_oauth.rs): the AI tool's name for itself, and where the
    /// approval sent the browser (a website's host, or null for this computer). Null for a
    /// token made here.
    connected: Option<Connected>,
}

#[derive(Serialize)]
struct Connected {
    client: String,
    redirect_host: Option<String>,
}

const TOKEN_COLUMNS: &str = "s.id, m.name, m.scope,
    array(select a.slug from apps a where a.id = any(m.app_ids) order by a.slug), m.people,
    s.created_at, s.expires_at, s.last_seen_at, s.expires_at <= now(),
    exists (select 1 from agent_grants g where g.session_id = s.id),
    (select c.name from agent_grants g join agent_clients c on c.id = g.client_id where g.session_id = s.id),
    (select g.redirect_uri from agent_grants g where g.session_id = s.id)";

type TokenRow = (
    Uuid,
    String,
    String,
    Vec<String>,
    bool,
    OffsetDateTime,
    OffsetDateTime,
    OffsetDateTime,
    bool,
    bool,
    Option<String>,
    Option<String>,
);

fn info(r: TokenRow) -> TokenInfo {
    TokenInfo {
        id: r.0,
        name: r.1,
        scope: r.2,
        apps: r.3,
        people: r.4,
        created_at: r.5,
        expires_at: r.6,
        last_used_at: r.7,
        expired: r.8,
        connected: r.9.then(|| Connected {
            client: r.10.unwrap_or_else(|| "AI tool".into()),
            redirect_host: r
                .11
                .and_then(|u| match crate::agent_oauth::check_return(&u) {
                    Ok(crate::agent_oauth::Return::Web(host)) => Some(host),
                    _ => None,
                }),
        }),
    }
}

/// The caller's tokens: live ones, and ones that expired in the last week (so the page can say so
/// and offer a replacement).
async fn list(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Vec<TokenInfo>>> {
    let rows: Vec<TokenRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {TOKEN_COLUMNS} from sessions s join maker_tokens m on m.session_id = s.id
         where s.user_id = $1 and s.kind = 'agent' and s.revoked_at is null
           and s.expires_at > now() - $2
         order by s.created_at desc"
    )))
    .bind(user.id)
    .bind(LISTED_AFTER_EXPIRY)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.into_iter().map(info).collect()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewToken {
    name: String,
    /// `read` or `publish`.
    scope: String,
    /// App addresses it may touch; empty or absent: all the maker's apps.
    #[serde(default)]
    apps: Vec<String>,
    /// Days it lasts: 1 to 365; 30 by default.
    #[serde(default)]
    days: Option<i64>,
    /// Also handle People jobs: requests, quotes, changes and the job conversation.
    #[serde(default)]
    people: bool,
}

#[derive(Serialize)]
struct Created {
    #[serde(flatten)]
    token: TokenInfo,
    /// Shown once; SLAPorSLOP keeps only its hash.
    secret: String,
    /// The MCP address. The website shows how to connect each kind of agent with the token kept
    /// in an environment variable, so the token itself never goes into a command or a chat.
    mcp_url: String,
}

async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<NewToken>,
) -> ApiResult<(StatusCode, Json<Created>)> {
    // A signed-in session is enough (no step-up): a token lasts at most a year, is listed with its
    // expiry and signed out with the maker's devices, and can't pay, accept agreements, change
    // payouts, tokens or the account. Agent sessions can't make tokens (`ALLOWED`).
    let name = body.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::invalid("name", "1 to 60 characters"));
    }
    if body.scope != "read" && body.scope != "publish" {
        return Err(ApiError::invalid("scope", "read or publish"));
    }
    let days = body.days.unwrap_or(DEFAULT_DAYS);
    if !(1..=MAX_DAYS).contains(&days) {
        return Err(ApiError::invalid("days", format!("1 to {MAX_DAYS}")));
    }
    // An agent that publishes acts under the Publisher Agreement, so the owner accepts it here,
    // where they can (the website shows it and makes the token once it's accepted); the agent
    // can't accept agreements.
    if body.scope == "publish" {
        crate::legal::require(&state.db, user.id, "publisher").await?;
    }
    let mut app_ids = Vec::new();
    for slug in &body.apps {
        let id: Option<Uuid> =
            sqlx::query_scalar("select id from apps where slug = $1 and maker_id = $2")
                .bind(slug)
                .bind(user.id)
                .fetch_optional(&state.db)
                .await?;
        app_ids.push(
            id.ok_or_else(|| ApiError::invalid("apps", format!("{slug} isn't one of your apps")))?,
        );
    }
    let secret = format!("{PREFIX}{}", new_token());
    let id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    // Counted with the account locked, so requests at the same moment can't pass the limit
    // together.
    sqlx::query("select 1 from users where id = $1 for update")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    let live: i64 = sqlx::query_scalar(
        "select count(*) from sessions where user_id = $1 and kind = 'agent' and revoked_at is null and expires_at > now()",
    )
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    if live >= MAX_TOKENS {
        return Err(ApiError::Conflict(format!(
            "you have {MAX_TOKENS} agent tokens; remove one first"
        )));
    }
    sqlx::query(
        "insert into sessions (id, user_id, token_hash, user_agent, expires_at, kind)
         values ($1, $2, $3, $4, now() + $5, 'agent')",
    )
    .bind(id)
    .bind(user.id)
    .bind(hash_token(&secret))
    .bind(format!("AI agent: {name}"))
    .bind(Duration::days(days))
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "insert into maker_tokens (session_id, name, scope, app_ids, people) values ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(&name)
    .bind(&body.scope)
    .bind(&app_ids)
    .bind(body.people)
    .execute(&mut *tx)
    .await?;
    let row: TokenRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {TOKEN_COLUMNS} from sessions s join maker_tokens m on m.session_id = s.id where s.id = $1"
    )))
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    let site = state.config.public_url.as_str().trim_end_matches('/');
    let mcp_url = format!("{site}/v1/maker/mcp");
    Ok((
        StatusCode::CREATED,
        Json(Created {
            token: info(row),
            mcp_url,
            secret,
        }),
    ))
}

async fn revoke(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let done = sqlx::query(
        "update sessions set revoked_at = now()
         where id = $1 and user_id = $2 and kind = 'agent' and revoked_at is null",
    )
    .bind(id)
    .bind(user.id)
    .execute(&state.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------------------------
// Expiry: the owner is told in time, and so is the agent

/// The owner is warned this long before a token expires (`maker_tokens.expiry_warning` 1), then
/// this long before (2), then when it has expired (3).
const WARN_WEEK: Duration = Duration::days(7);
const WARN_DAY: Duration = Duration::days(1);
/// Tokens this short aren't warned about: they were made to be short.
const WARNED_LIFETIME: Duration = Duration::days(8);

/// Whole days left before `expires_at`, rounded up (0 once it has passed).
pub fn days_left(expires_at: OffsetDateTime, now: OffsetDateTime) -> i64 {
    let secs = (expires_at - now).whole_seconds();
    if secs <= 0 {
        0
    } else {
        (secs + 86_399) / 86_400
    }
}

/// "in 3 days", "within a day", "has expired".
fn when(expires_at: OffsetDateTime, now: OffsetDateTime) -> String {
    match days_left(expires_at, now) {
        0 => "has expired".into(),
        1 => "expires within a day".into(),
        n => format!("expires in {n} days"),
    }
}

/// The calling agent's own token: its name and expiry, and a line saying what to tell its person.
pub(crate) struct OwnToken {
    pub name: String,
    pub expires_at: OffsetDateTime,
    pub note: String,
}

pub(crate) async fn own_token(
    db: &sqlx::PgPool,
    session_id: Uuid,
) -> sqlx::Result<Option<OwnToken>> {
    let row: Option<(String, OffsetDateTime, bool)> = sqlx::query_as(
        "select m.name, s.expires_at, exists (select 1 from agent_grants g where g.session_id = s.id)
         from maker_tokens m join sessions s on s.id = m.session_id
         where m.session_id = $1",
    )
    .bind(session_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|(name, expires_at, connected)| {
        let now = OffsetDateTime::now_utc();
        let what = if connected {
            format!("This connection (\"{name}\", approved in the browser)")
        } else {
            format!("This token (\"{name}\")")
        };
        let mut note = format!("{what} {} ({}).", when(expires_at, now), expires_at.date());
        if connected && days_left(expires_at, now) <= 7 {
            note.push_str(
                " When it ends, the person you work for approves you again in their browser: you'll \
                 be asked to sign in to SLAPorSLOP (in Claude Code, /mcp; in Codex, codex mcp login).",
            );
        } else if days_left(expires_at, now) <= 7 {
            note.push_str(
                " Tell the person you work for to make a replacement on SLAPorSLOP under Account → AI \
                 agents (Make a replacement fills in the same settings) and set it up on their \
                 computer the same way, before this one stops working.",
            );
        }
        OwnToken {
            name,
            expires_at,
            note,
        }
    }))
}

/// Hourly (cleanup.rs): tells owners a week and a day before a token expires, and when it has.
/// Each warning is sent once (`maker_tokens.expiry_warning`); a server that was down sends only
/// the latest one due. Nothing for a token removed, made for a week or less, or already replaced
/// (a newer live token with the same name that lasts longer). Returns the warnings sent.
pub async fn warn_expiring(db: &sqlx::PgPool) -> anyhow::Result<usize> {
    let mut tx = db.begin().await?;
    let due: Vec<(Uuid, Uuid, String, OffsetDateTime, i16, bool, bool)> = sqlx::query_as(
        "select m.session_id, s.user_id, m.name, s.expires_at,
                (case when s.expires_at <= now() then 3
                      when s.expires_at <= now() + $1 then 2
                      else 1 end)::int2,
                exists (select 1 from sessions r join maker_tokens rm on rm.session_id = r.id
                        where r.user_id = s.user_id and r.kind = 'agent' and r.id <> s.id
                          and rm.name = m.name and r.revoked_at is null
                          and r.created_at > s.created_at and r.expires_at > s.expires_at),
                exists (select 1 from agent_grants g where g.session_id = s.id)
         from maker_tokens m join sessions s on s.id = m.session_id
         where s.kind = 'agent' and s.revoked_at is null
           and s.expires_at - s.created_at > $3
           and s.expires_at > now() - $4
           and s.expires_at <= now() + $2
           and m.expiry_warning < (case when s.expires_at <= now() then 3
                                        when s.expires_at <= now() + $1 then 2
                                        else 1 end)
         for update of m skip locked",
    )
    .bind(WARN_DAY)
    .bind(WARN_WEEK)
    .bind(WARNED_LIFETIME)
    .bind(LISTED_AFTER_EXPIRY)
    .fetch_all(&mut *tx)
    .await?;
    let now = OffsetDateTime::now_utc();
    let mut sent = 0;
    for (id, user_id, name, expires_at, stage, replaced, connected) in due {
        sqlx::query("update maker_tokens set expiry_warning = $2 where session_id = $1")
            .bind(id)
            .bind(stage)
            .execute(&mut *tx)
            .await?;
        if replaced {
            continue;
        }
        let (title, body) = if connected {
            // Approved in the browser (agent_oauth.rs): nothing to set up, only to approve again.
            if stage == 3 {
                (
                    format!("Your AI tool \"{name}\" was disconnected"),
                    "The time you chose for it is up, so it can't reach SLAPorSLOP any more. To connect \
                     it again, let it sign in to SLAPorSLOP when it asks (in Claude Code: /mcp) and \
                     approve it in your browser."
                        .to_string(),
                )
            } else {
                (
                    format!(
                        "Your AI tool \"{name}\" disconnects {}",
                        match days_left(expires_at, now) {
                            1 => "within a day".to_string(),
                            n => format!("in {n} days"),
                        }
                    ),
                    format!(
                        "On {} the time you chose for it is up. After that, let it sign in to \
                         SLAPorSLOP again when it asks, and approve it in your browser.",
                        expires_at.date()
                    ),
                )
            }
        } else if stage == 3 {
            (
                format!("Your AI agent token \"{name}\" has expired"),
                "Your AI agent can't reach SLAPorSLOP with it any more. Make a replacement on Account → AI \
                 agents: it fills in the same settings. Then set it up on your computer the same way \
                 as before (never paste it into an AI chat)."
                    .to_string(),
            )
        } else {
            (
                format!("Your AI agent token \"{name}\" {}", when(expires_at, now)),
                format!(
                    "It stops working on {}. Make a replacement on Account → AI agents: it fills in \
                     the same settings, and the old one keeps working until then. Then set it up on \
                     your computer the same way as before (never paste it into an AI chat).",
                    expires_at.date()
                ),
            )
        };
        crate::notify::send(
            &mut tx,
            crate::notify::Notice {
                user_id,
                category: "agents",
                title,
                body,
                link: "/account/agents".into(),
                collapse_key: None,
            },
        )
        .await?;
        sent += 1;
    }
    tx.commit().await?;
    Ok(sent)
}

// ---------------------------------------------------------------------------------------------
// Guides: short, so an agent reads only what it needs

const GUIDE: &str = include_str!("maker_guide.md");
const PUBLISHING: &str = include_str!("../../../web/public/publishing.md");

/// The guide, with the agent's own token's expiry at the end, so it can tell its person in time.
async fn guide(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Response> {
    let mut text = GUIDE.to_string();
    if user.agent.is_some()
        && let Some(own) = own_token(&state.db, user.session_id).await?
    {
        text.push_str(&format!("\n## Your token\n\n{}\n", own.note));
    }
    Ok((
        [(
            axum::http::header::CONTENT_TYPE,
            "text/markdown; charset=utf-8",
        )],
        text,
    )
        .into_response())
}

/// The publishing guide's `##` and `###` sections: (heading, its text up to the next heading of
/// the same or a higher level). Lines in code blocks never count as headings.
fn sections() -> Vec<(String, String)> {
    let lines: Vec<&str> = PUBLISHING.lines().collect();
    let mut headings = Vec::new();
    let mut in_code = false;
    for (i, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
        } else if !in_code {
            if let Some(t) = line.strip_prefix("### ") {
                headings.push((i, 3, t));
            } else if let Some(t) = line.strip_prefix("## ") {
                headings.push((i, 2, t));
            }
        }
    }
    headings
        .iter()
        .enumerate()
        .map(|(n, &(start, level, title))| {
            let end = headings[n + 1..]
                .iter()
                .find(|&&(_, l, _)| l <= level)
                .map_or(lines.len(), |&(i, _, _)| i);
            (title.trim().to_string(), lines[start..end].join("\n"))
        })
        .collect()
}

async fn guide_sections(_user: CurrentUser) -> Json<Value> {
    Json(json!({
        "sections": sections().into_iter().map(|(t, _)| t).collect::<Vec<_>>(),
        "read_one": "GET /v1/maker/publishing-guide/{section}, with the section's name",
    }))
}

async fn guide_section(_user: CurrentUser, Path(section): Path<String>) -> ApiResult<Response> {
    let want = section.trim().to_lowercase();
    let found = sections()
        .into_iter()
        .find(|(t, _)| t.to_lowercase() == want)
        .ok_or(ApiError::NotFound)?;
    Ok((
        [(
            axum::http::header::CONTENT_TYPE,
            "text/markdown; charset=utf-8",
        )],
        found.1,
    )
        .into_response())
}

// ---------------------------------------------------------------------------------------------
// MCP

fn store_page_fields() -> Value {
    json!({
        "title": s("Name, 1–80 characters"),
        "summary": s("One line for tiles, up to 200 characters"),
        "description_md": s("The store page's description, Markdown, up to 20,000 characters: what it is, how to play, what's different. Plain words; no claims it's \"safe\""),
        "repo_url": s("The public repository, e.g. https://github.com/owner/repo"),
        "license_spdx": s("The repository's licence as an SPDX ID, e.g. MIT, GPL-3.0-or-later"),
        "min_age": { "type": "integer", "description": "Minimum age: 0, 3, 7, 12, 16 or 18" },
        "content_labels": { "type": "array", "items": { "type": "string" }, "description": "Any of romance, suggestive, violence, horror, strong_language, gambling_themes" },
        "tags": { "type": "array", "items": { "type": "string" }, "description": "Up to 10 tag slugs from list_categories. A category with max_age (Kids & Family) lists only apps rated at most that and without its excluded_labels: don't tag an older app with it" },
        "platforms": { "type": "array", "items": { "type": "string" }, "description": "Any of windows, linux (a .tar.gz), linux-deb and linux-rpm: every release must include exactly these. linux needs linux-deb and/or linux-rpm with it, and they need linux. Or [\"browser\"] alone, for a game that plays on your own website: nothing to download, and each release is its play address (submit_play_address). Can't change between the two once it has releases" },
        "online": { "type": "array", "items": { "type": "string" }, "description": "How it goes online, if it does: any of own_servers (servers you run), player_servers (servers players host), peer_to_peer, other_services (someone else's: a login, a leaderboard, an AI service). Empty: it doesn't go online. See the guide section \"Online play and mods\"" },
        "online_hosts": { "type": "array", "items": { "type": "string" }, "description": "The hosts it connects to" },
        "sandbox_ready": { "type": "boolean", "description": "Works in sandbox mode: local saves only in SLOP_SAVE_DIR. Not for browser games" },
        "mods_folder": { "type": ["object", "null"], "description": "Where it reads mods from: {base: game|saves, path, layout: folder|flat}; null if it reads SLOP_MODS or has no mods. Not for browser games" },
        "bug_reports": { "type": "boolean", "description": "Take bug reports from players on SLAPorSLOP (on by default)" },
        "accessibility": { "type": "array", "items": { "type": "string" }, "description": "Accessibility features it really has (players confirm or dispute them): remap_controls, colorblind, subtitles, text_size, one_handed, difficulty, no_flashing, screen_reader, controller. Check the code before claiming one" },
    })
}

fn new_app_fields() -> Value {
    let mut fields = store_page_fields();
    let m = fields.as_object_mut().expect("object");
    m.insert(
        "slug".into(),
        s("The store address: lowercase letters, digits and dashes, e.g. pixel-dungeon"),
    );
    m.insert(
        "kind".into(),
        s("game or app; only a game can play in the browser"),
    );
    m.insert("stage".into(), s("coming_soon, beta or released"));
    fields
}

fn slug_only() -> Value {
    json!({ "slug": s("The app's store address") })
}

/// A browser game's release: its play address instead of files.
fn play_address_fields() -> Value {
    json!({
        "slug": s("The browser game's store address"),
        "play_address": s("The https:// address the game plays at on your own website, e.g. https://you.github.io/game/. It must load as a web page, on a public site"),
        "version": s("The version, e.g. 1.2.0: a new one for each address you give"),
        "commit_sha": s("The full 40-character commit SHA your site is built from; leave out on GitHub to use the default branch's newest"),
        "notes": s("What changed, for players: Markdown, up to 5,000 characters"),
        "rating": { "type": "object", "description": "This release's age rating, when it differs from the store page's: {min_age: 0, 3, 7, 12, 16 or 18, content_labels: [...]}" },
    })
}

fn release_fields() -> Value {
    json!({
        "slug": s("The app's store address"),
        "version": s("The version, e.g. 1.2.0"),
        "commit_sha": s("The full 40-character commit SHA it was built from"),
        "files": { "type": "array", "description": "One for each platform on the store page: {platform, kind, url: the file attached to a release on the app's repository, exec: the program to start inside an archive (e.g. bin/game.exe), args: []}. windows: kind archive, a .zip. linux: kind archive, a .tar.gz. linux-deb: kind installer, a .deb. linux-rpm: kind installer, an .rpm. A mod: platform any, kind archive, a .zip, no exec", "items": { "type": "object" } },
        "notes": s("What changed, for players: Markdown, up to 5,000 characters"),
        "runtimes": { "type": "array", "items": { "type": "string" }, "description": "Shared runtimes the Windows build needs, by ID from list_runtimes; leave out to keep the last release's" },
    })
}

pub(crate) const TOOLS: &[Tool] = &[
    Tool {
        name: "maker_guide",
        description: "How to publish and fix an app on SLAPorSLOP with these tools, and SLAPorSLOP's rules in short. Read first.",
        method: Method::GET,
        path: "/v1/maker/guide",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "publishing_guide",
        description: "One section of SLAPorSLOP's full publishing guide (e.g. \"Saves and sandbox mode\", \"Slop Pulse (player counts)\", \"Online play and mods\", \"Option B: Slop Build\"). Without a section: the list of sections. Read only the section you need.",
        method: Method::GET,
        path: "/v1/maker/publishing-guide/{section}",
        properties: || json!({ "section": s("The section's heading, exactly as listed") }),
        required: &["section"],
    },
    Tool {
        name: "list_guide_sections",
        description: "The publishing guide's section headings.",
        method: Method::GET,
        path: "/v1/maker/publishing-guide",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "whats_new",
        description: "The notifications of the person you work for, newest first: releases approved or rejected, reviews, offered fixes, job requests and messages. One call to see what needs attention; check it now and then rather than polling every tool. Reading them here doesn't mark them read. `your_token` says when your own token expires: within 7 days, tell your person to make a replacement on Account → AI agents.",
        method: Method::GET,
        path: "/v1/me/notifications",
        properties: || {
            json!({
                "unread": { "type": "boolean", "description": "Only the ones the person hasn't read yet" },
                "limit": { "type": "integer", "description": "How many, 1 to 100; 60 when left out" },
            })
        },
        required: &[],
    },
    Tool {
        name: "whats_left",
        description: "Everything still open across your person's apps, agreements and People page, each step saying who does it (`who`: you = the person, assistant = you with these tools, slaporslop = a check or review to wait for), with a link. `summary` is a sentence to pass on. Call it when you start, and end every session with it, telling your person their steps and links.",
        method: Method::GET,
        path: "/v1/me/next-steps",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "my_apps",
        description: "Your apps on SLAPorSLOP, including drafts, with their status.",
        method: Method::GET,
        path: "/v1/me/apps",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "app_overview",
        description: "An app's management page: the release players get now (`serving`) and whether it's held there (`held`), the approved releases they could get (`approved`), releases waiting for review (`waiting`), changes asked for (`changes`), a freeze if SLAPorSLOP can't read the repository, and whether the app can move to its next stage. The store page's own text is get_store_page.",
        method: Method::GET,
        path: "/v1/apps/{slug}/manage",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "get_store_page",
        description: "An app's store page as players see it: title, summary, description, tags, age rating and content labels, platforms, online, licence, repository, stage and rating. Your maker's own apps (drafts too), or any app in the store, e.g. the original before you register a community build of it. Read it before update_store_page, so you change only what should change.",
        method: Method::GET,
        path: "/v1/apps/{slug}",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "list_releases",
        description: "An app's releases, newest first, each with its status (fetching, awaiting_review, approved, rejected, failed, revoked, withdrawn) and the reason, its files and SLAPorSLOP's checks: the source check (licence, saves, online, code loading, purchases), Slop Build's checks (secrets, vulnerable packages) and file scans. `progress` says where it is, file by file: `summary` is a sentence to pass on (\"Checking file 2 of 4: Linux (.deb), scanning for malware… about 13 minutes left\"), with `current` (the file being checked and its step), `files_checked` of `files_total`, `remaining_seconds`, and each file's `check` (waiting, checking, checked). Files take minutes each to scan, so check back every few minutes, not every few seconds. A browser game's release has no files: `play_address`, and `address_checks` (https, public_host and public_host_skipped, bad_links, status, final_address, redirects, hops, content_type, ok, problem); its `progress.stage` is checking_address while SLAPorSLOP checks it, which takes seconds. Each release is long: to follow the one you just submitted, ask for `limit` 1.",
        method: Method::GET,
        path: "/v1/apps/{slug}/releases",
        properties: || {
            json!({
                "slug": s("The app's store address"),
                "limit": { "type": "integer", "description": "How many, newest first: 1 for only the newest. 1 to 50; 50 when left out" },
            })
        },
        required: &["slug"],
    },
    Tool {
        name: "get_fixes",
        description: "Every problem SLAPorSLOP's checks found in the app's newest release, each with why it matters and exact instructions to fix it in the repository. Fix them, push, and submit a new release (a browser game: give its play address again). `recommendations` lists what's worth adding that never blocks a release (e.g. the app has no icon yet), with the same kind of instructions.",
        method: Method::GET,
        path: "/v1/apps/{slug}/fixes",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "check_source",
        description: "A dry run of SLAPorSLOP's source check, before submitting a release: reads the app's repository at a commit that's been pushed and returns the fixes a release there would get, in get_fixes' form (licence problems, purchases, code loaded from outside SLAPorSLOP, going online without the store page saying so, saves outside SLOP_SAVE_DIR, Slop Pulse). Nothing is saved, submitted or sent for review. `status` is must_fix, should_fix, looks_good, or not_checked (`detail` says why the source couldn't be read, e.g. the commit isn't pushed). It can take a minute on a large repository. It doesn't run what only a release gets: Slop Build's own checks (secrets, packages with known vulnerabilities), the scan of the files, a person's review.",
        method: Method::GET,
        path: "/v1/apps/{slug}/source-check",
        properties: || {
            json!({
                "slug": s("The app's store address"),
                "commit_sha": s("The full 40-character commit SHA to check, in lowercase (git rev-parse HEAD prints it); leave out on GitHub to check the default branch's newest"),
            })
        },
        required: &["slug"],
    },
    Tool {
        name: "hold_at_release",
        description: "Chooses which approved release players get. A rollback, when the newest release turned out broken: give the earlier release's ID from app_overview's `approved`, and players are moved back to it and held there while you fix things. Give `release_id` null to stop holding, so players follow the newest approved release again. It's applied at once when the release would be approved without a person (app_overview's `auto_approval_ready`); otherwise it waits for a person at SLAPorSLOP, and `status_reason` says why. A new request replaces one still waiting. Only when your maker asks: it changes what every player gets. Not for browser games, which always open their newest approved address.",
        method: Method::POST,
        path: "/v1/apps/{slug}/changes",
        properties: || {
            json!({
                "slug": s("The app's store address"),
                "kind": { "type": "string", "enum": ["release"], "description": "Always release" },
                "release_id": { "type": ["string", "null"], "description": "The approved release to hold players at, from app_overview's `approved`; null to follow the newest approved release again" },
                "note": s("Why, for SLAPorSLOP's reviewer: up to 500 characters"),
            })
        },
        required: &["slug", "kind", "release_id"],
    },
    Tool {
        name: "cancel_pending_change",
        description: "Cancels a change that's still waiting for a person (status pending in app_overview's `changes`): a hold_at_release that wasn't applied at once, or a move to another repository your maker asked for on the website.",
        method: Method::POST,
        path: "/v1/apps/{slug}/changes/{id}/cancel",
        properties: || {
            json!({
                "slug": s("The app's store address"),
                "id": s("The change's ID, from app_overview's `changes`"),
            })
        },
        required: &["slug", "id"],
    },
    Tool {
        name: "release_settings",
        description: "How the app's releases reach SLAPorSLOP and players: whether SLAPorSLOP watches the repository for new GitHub releases (`watch_releases`, with the last tag seen and any problem), whether an approved release goes to players by itself (`auto_update`), and whether the next release would be approved without a person (`auto_approval_ready`; `auto_approval_note` says why not). Switching these is for your maker, on the app's Manage page.",
        method: Method::GET,
        path: "/v1/apps/{slug}/update-settings",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "app_ratings",
        description: "The app's rating: Slaps and Slop votes now, the rating at the end of each month, votes per day for the last 90 days (with days of unusual voting SLAPorSLOP's abuse checks found), and how many votes it needs before a rating shows (`min_votes`). release_feedback says how each release landed.",
        method: Method::GET,
        path: "/v1/apps/{slug}/ratings",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "get_history",
        description: "How the app did each day, for the last 30, 90 or 365 days (`days`; 30 when left out), oldest first: `players` (accounts that started it that UTC day), `first_plays` (accounts that started it for the first time), `library_adds` (times it was added to a library; an account that removes it and adds it again on a later day counts again) and `peak` (the most playing at once, from Slop Pulse; it counts the maker too while they share their playtime). Counts only, never who. `null` means not counted that day, which is not zero: players per day are counted from `players_from`, the rest from `history_from`. The maker's own plays and adds aren't counted in the other three. Plays by accounts that switched off \"Share my playtime\" aren't counted either. Don't add `players` up across days: an account counts on every day it plays. SLAPorSLOP doesn't count installs; first plays are the closest number. Not for Private apps or dedicated servers.",
        method: Method::GET,
        path: "/v1/apps/{slug}/history",
        properties: || {
            json!({
                "slug": s("The app's store address"),
                "days": { "type": "integer", "description": "30, 90 or 365; 30 when left out" },
            })
        },
        required: &["slug"],
    },
    Tool {
        name: "list_reviews",
        description: "Players' reviews of the app (Slaps or Slop, with text), newest first.",
        method: Method::GET,
        path: "/v1/apps/{slug}/reviews",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "reply_to_review",
        description: "Replies publicly under a player's review of the app, as its maker (a new reply replaces the old one). Be brief, polite and specific: thank them, say what was fixed or is planned. Never argue, never ask for a better vote, never offer anything for one.",
        method: Method::PUT,
        path: "/v1/apps/{slug}/reviews/{id}/reply",
        properties: || {
            json!({
                "slug": s("The app's store address"),
                "id": s("The review's ID, from list_reviews"),
                "body": s("The reply, up to 2,000 characters"),
            })
        },
        required: &["slug", "id", "body"],
    },
    Tool {
        name: "remove_review_reply",
        description: "Takes down your maker's reply under a player's review (the review stays). Only when your maker asks.",
        method: Method::DELETE,
        path: "/v1/apps/{slug}/reviews/{id}/reply",
        properties: || {
            json!({
                "slug": s("The app's store address"),
                "id": s("The review's ID, from list_reviews"),
            })
        },
        required: &["slug", "id"],
    },
    Tool {
        name: "list_news",
        description: "The news your maker posted on the app's store page, newest first, each with its ID (`hidden` ones were removed by a moderation decision and can't be changed; `can_appeal` says whether your maker can still appeal that). `posting` says whether a new post is allowed now: one per app per day, and only while the app's page is in the store. When `more` is true there are older posts: ask again with `before` set to the last ID you got.",
        method: Method::GET,
        path: "/v1/apps/{slug}/news",
        properties: || {
            json!({
                "slug": s("The app's store address"),
                "before": s("The ID of the last post you already have, for the older ones"),
                "limit": { "type": "integer", "description": "How many, 1 to 50; 10 when left out" },
            })
        },
        required: &["slug"],
    },
    Tool {
        name: "post_news",
        description: "Posts short news on the app's store page, as its maker. People who marked the app Interested are told unless they switched news off, so it can't be unsent, and only one post per app per day is allowed. Only when your maker asks. Plain, short, no hype: what changed, or what's coming and when. Say \"scanned\" or \"checked\", never \"safe\". Never offer anything for votes or reviews.",
        method: Method::POST,
        path: "/v1/apps/{slug}/news",
        properties: || {
            json!({
                "slug": s("The app's store address"),
                "headline": s("One line, up to 100 characters: it's what people see in their notifications"),
                "body": s("The news, up to 2,000 characters. Simple Markdown (bold, lists, links); no pictures"),
            })
        },
        required: &["slug", "headline", "body"],
    },
    Tool {
        name: "edit_news",
        description: "Changes a news post's headline and text (send both). Nobody is told again. Only when your maker asks.",
        method: Method::PUT,
        path: "/v1/apps/{slug}/news/{id}",
        properties: || {
            json!({
                "slug": s("The app's store address"),
                "id": s("The post's ID, from list_news"),
                "headline": s("One line, up to 100 characters"),
                "body": s("The news, up to 2,000 characters"),
            })
        },
        required: &["slug", "id", "headline", "body"],
    },
    Tool {
        name: "remove_news",
        description: "Deletes a news post from the app's store page. It doesn't give the day's post back. Only when your maker asks.",
        method: Method::DELETE,
        path: "/v1/apps/{slug}/news/{id}",
        properties: || {
            json!({
                "slug": s("The app's store address"),
                "id": s("The post's ID, from list_news"),
            })
        },
        required: &["slug", "id"],
    },
    Tool {
        name: "list_bug_reports",
        description: "Players' bug reports on the app, most \"me too\" first: what's wrong, the version and platform, and crash output when the player sent it. status: open (default), fixed, wont_fix, duplicate or all. File the ones worth tracking as issues on the repository yourself (e.g. gh issue create), without the player's name or anything personal from the log.",
        method: Method::GET,
        path: "/v1/apps/{slug}/bug-reports",
        properties: || json!({ "slug": s("The app's store address"), "status": s("open, fixed, wont_fix, duplicate or all") }),
        required: &["slug"],
    },
    Tool {
        name: "update_bug_report",
        description: "Marks a bug report fixed (say in which version), wont_fix or duplicate, or open again, with a short note the player sees. The player, and everyone who said \"me too\" when it's fixed, is told.",
        method: Method::PATCH,
        path: "/v1/bug-reports/{id}",
        properties: || {
            json!({
                "id": s("The report's ID"),
                "status": s("open, fixed, wont_fix or duplicate"),
                "maker_note": s("For the player: up to 2,000 characters"),
            })
        },
        required: &["id"],
    },
    Tool {
        name: "release_feedback",
        description: "For each recent release: % Slaps from votes cast on it, reviews written while it was current with their most common phrases (praised and criticised), and its bug reports. Use it to see what an update changed for players.",
        method: Method::GET,
        path: "/v1/apps/{slug}/release-feedback",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "browser_demo",
        description: "Whether the app can have a browser demo now (new releases only, for a few days), and the demo's state: fetching, live (with its address and plays), failed (why) or ended. Not for browser games: they already play in the browser.",
        method: Method::GET,
        path: "/v1/apps/{slug}/demo",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "host_browser_demo",
        description: "Hosts a browser demo of a newly released game: give the address of a web build (a .zip with index.html at its top, e.g. Godot's Web export) attached to a release on the app's repository. SLAPorSLOP scans and hosts it for its first days, with no network access. Check browser_demo afterwards. Not for browser games: they already play in the browser, on their own site.",
        method: Method::POST,
        path: "/v1/apps/{slug}/demo",
        properties: || json!({ "slug": s("The app's store address"), "url": s("The web build's address on the release page") }),
        required: &["slug", "url"],
    },
    Tool {
        name: "end_browser_demo",
        description: "Ends the app's browser demo now and deletes its files, before its days are up. Only when your maker asks.",
        method: Method::DELETE,
        path: "/v1/apps/{slug}/demo",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "list_bounties",
        description: "Open fix bounties across the store: what players will pay for, biggest first (app_bounties lists one app's). A bounty is paid to whoever delivers it in an approved release of the app or of a community build of it.",
        method: Method::GET,
        path: "/v1/bounties",
        properties: || json!({ "slug": s("Optional: an app's store address, for its bounties only") }),
        required: &[],
    },
    Tool {
        name: "app_bounties",
        description: "One app's fix bounties (yours, or one you'd build a community version of), open ones first, biggest first.",
        method: Method::GET,
        path: "/v1/apps/{slug}/bounties",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "get_bounty",
        description: "One bounty: what's wanted, how much is pledged, and deliveries so far.",
        method: Method::GET,
        path: "/v1/bounties/{id}",
        properties: || json!({ "id": s("The bounty's ID") }),
        required: &["id"],
    },
    Tool {
        name: "claim_bounty",
        description: "Says your maker's newest approved release delivers a bounty. Only when it truly does what the bounty asks: the app's maker or the backers check it before anyone is charged. Your maker needs a donation account (their Stripe account) to be paid.",
        method: Method::POST,
        path: "/v1/bounties/{id}/claims",
        properties: || {
            json!({
                "id": s("The bounty's ID"),
                "app_slug": s("Your maker's app it's delivered in: the bounty's app or a community build of it"),
                "note": s("What was done, for the backers: up to 2,000 characters"),
            })
        },
        required: &["id", "app_slug", "note"],
    },
    Tool {
        name: "list_categories",
        description: "SLAPorSLOP's categories and their tags, for a store page's tags. A category's max_age and excluded_labels, when present, say which apps it lists.",
        method: Method::GET,
        path: "/v1/categories",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "list_runtimes",
        description: "The shared Windows runtimes a release can name (Visual C++, .NET Desktop, DirectX).",
        method: Method::GET,
        path: "/v1/runtimes",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "slop_build_setup",
        description: "How to add Slop Build to the app's repository: the workflow file to commit, with the detected project type. With it, every push builds, signs and submits a release by itself. Not for browser games: Slop Build makes downloads (give a browser game's address with submit_play_address).",
        method: Method::GET,
        path: "/v1/apps/{slug}/slop-build",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "github_releases",
        description: "The releases on the app's GitHub repository that can be submitted with submit_github_release. Not for browser games.",
        method: Method::GET,
        path: "/v1/apps/{slug}/github-releases",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "create_store_page",
        description: "Creates a draft store page. Write the description yourself from the repository (README, gameplay, controls). Needs a publish token for all your apps. For a game that plays on your own website, platforms [\"browser\"] (a public game: no sandbox_ready or mods_folder), then give its address with submit_play_address.",
        method: Method::POST,
        path: "/v1/apps",
        properties: new_app_fields,
        required: &[
            "slug",
            "title",
            "summary",
            "kind",
            "stage",
            "repo_url",
            "license_spdx",
            "min_age",
            "platforms",
        ],
    },
    Tool {
        name: "update_store_page",
        description: "Changes store page fields. On a listed app some changes wait for a person's review.",
        method: Method::PATCH,
        path: "/v1/apps/{slug}",
        properties: || {
            let mut f = store_page_fields();
            f.as_object_mut()
                .expect("object")
                .insert("slug".into(), s("The app's store address"));
            f
        },
        required: &["slug"],
    },
    Tool {
        name: "submit_release",
        description: "Submits a release of a Windows or Linux app whose files are attached to a release on the app's repository. SLAPorSLOP downloads, hashes and scans them, then a person reviews it. Check its progress with list_releases. A browser game uses submit_play_address instead.",
        method: Method::POST,
        path: "/v1/apps/{slug}/releases",
        properties: release_fields,
        required: &["slug", "version", "commit_sha", "files"],
    },
    Tool {
        name: "submit_play_address",
        description: "Submits a browser game's release: the https address it plays at on your own website (its store page's platforms are [\"browser\"]). SLAPorSLOP checks the address (https, a public site, the bad-link lists, and that the page loads as a web page), in seconds, then a person opens it and plays it: every new address waits for a person, and only the same address as the one Play opens now can be approved automatically. SLAPorSLOP checks the address, not what the site serves: never say it's scanned. Check it with list_releases.",
        method: Method::POST,
        path: "/v1/apps/{slug}/releases",
        properties: play_address_fields,
        required: &["slug", "play_address", "version"],
    },
    Tool {
        name: "submit_github_release",
        description: "Submits a release from the app's GitHub releases by its tag; its notes default to the GitHub release's text. Not for browser games.",
        method: Method::POST,
        path: "/v1/apps/{slug}/releases/from-github",
        properties: || {
            json!({
                "slug": s("The app's store address"),
                "tag": s("The GitHub release's tag"),
                "files": { "type": "array", "items": { "type": "object" }, "description": "Which asset is which platform, as github_releases shows" },
                "notes": s("What changed, for players: Markdown, up to 5,000 characters"),
            })
        },
        required: &["slug", "tag"],
    },
    Tool {
        name: "set_release_notes",
        description: "Sets a release's notes: what changed, for players. Write them from the commits since the last release.",
        method: Method::PATCH,
        path: "/v1/apps/{slug}/releases/{id}",
        properties: || {
            json!({
                "slug": s("The app's store address"),
                "id": s("The release's ID, from list_releases"),
                "notes": s("Markdown, up to 5,000 characters"),
            })
        },
        required: &["slug", "id", "notes"],
    },
    Tool {
        name: "withdraw_release",
        description: "Withdraws a release that hasn't been reviewed yet (fetching or awaiting_review), so it never reaches players: one your maker didn't publish, or one sent by mistake. Only when your maker asks. The app's next release is then reviewed by a person.",
        method: Method::POST,
        path: "/v1/apps/{slug}/releases/{id}/withdraw",
        properties: || {
            json!({
                "slug": s("The app's store address"),
                "id": s("The release's ID, from list_releases"),
            })
        },
        required: &["slug", "id"],
    },
    Tool {
        name: "move_to_stage",
        description: "Moves the app on to its next stage: `beta` (playable, still changing; players can rate it and donate) or `released` (finished). Straight from coming_soon to released is fine. Only forward, and only once a release (a browser game: its play address) has been approved; it happens at once and everyone who marked the app Interested is told. Only when your maker asks: it's their decision.",
        method: Method::POST,
        path: "/v1/apps/{slug}/stage",
        properties: || {
            json!({
                "slug": s("The app's store address"),
                "stage": s("beta or released"),
            })
        },
        required: &["slug", "stage"],
    },
    Tool {
        name: "set_commit_status",
        description: "Switches SLAPorSLOP's result on GitHub on or off for one of your maker's apps. With it on, each release sent from then on gets a commit status named SLAPorSLOP on its commit: being checked, waiting for review, passed, not approved, couldn't be checked, or withdrawn. Anyone who can see the commit on GitHub sees it, also when a release isn't approved; it never says why. Only when your maker asks: tell them that first. Only for apps whose app_overview has `commit_status`. If its `permission` is false, your maker must accept the App's Commit statuses permission on GitHub (`github_url`, then Configure, then Review request); afterwards call this again with on true, which checks again. Statuses already on GitHub stay when it's switched off, except that a passed one is corrected if SLAPorSLOP later withdraws that release.",
        method: Method::PUT,
        path: "/v1/apps/{slug}/commit-status",
        properties: || {
            json!({
                "slug": s("The app's store address"),
                "on": { "type": "boolean", "description": "true switches it on (or checks again), false switches it off" },
            })
        },
        required: &["slug", "on"],
    },
    Tool {
        name: "check_repository",
        description: "Checks now that SLAPorSLOP can still read the app's repository, instead of waiting for the daily check. A freeze for lost access is lifted once SLAPorSLOP can read it (not one SLAPorSLOP's staff placed). Returns `frozen` (the freeze still in place, or null) and `recommendations`: what's worth adding that never blocks anything (e.g. no icon yet), with exact instructions. It doesn't check the code (SLAPorSLOP's source check runs on each release) or read the pictures (check_pictures does).",
        method: Method::POST,
        path: "/v1/apps/{slug}/repo-check",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "check_pictures",
        description: "Reads the app's .slop/media.json (icon, thumbnail, screenshots, videos) on the repository's default branch now, as the owner's \"Check my repository now\" does. New pictures wait for a person's review before they show. Returns what the store shows (`live`), what's waiting (`pending`), the last `problem` (`icon_problem` when it was the icon), and `recommendations` such as a missing icon. Push the pictures first; see the guide section \"Pictures and videos\".",
        method: Method::POST,
        path: "/v1/apps/{slug}/media/check",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "list_pictures",
        description: "The app's pictures as SLAPorSLOP has them, without reading the repository again: what the store shows (`live`), what waits for a person's review (`pending`), a set that was turned down and why (`rejected`), and the last `problem`. After you push new pictures, check_pictures reads them.",
        method: Method::GET,
        path: "/v1/apps/{slug}/media",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "register_community_build",
        description: "Registers a community build or a mod of another app (its original), from your own fork or repository. It starts as a draft; submit a release for it like any app, and it's checked and reviewed the same way. Mods (kind addon) are one .zip for every platform (platform any) with nothing to start. Read the guide sections \"Publishing a community build\" or \"Publishing a mod\" first.",
        method: Method::POST,
        path: "/v1/apps/{original}/branches",
        properties: || {
            json!({
                "original": s("The original app's store address"),
                "slug": s("The new store address for your build or mod"),
                "title": s("Its name"),
                "summary": s("One line, up to 200 characters"),
                "repo_url": s("Your fork's or mod's public repository"),
                "license_spdx": s("Its licence, as an SPDX ID (a fork keeps the original's terms)"),
                "kind": s("addon (a mod), mod (a modded version), fix, translation, port or other"),
                "note": s("What it changes from the original, for players: up to 500 characters"),
                "platforms": { "type": "array", "items": { "type": "string" }, "description": "windows and/or linux (with linux-deb and/or linux-rpm); leave out for a mod. A community build can't play in the browser, and a browser game can't have community builds or mods, yet" },
                "description_md": s("Its store page text, Markdown; the note when empty"),
                "min_age": { "type": "integer", "description": "At least the original's; the original's when left out" },
                "content_labels": { "type": "array", "items": { "type": "string" } },
                "online": { "type": "array", "items": { "type": "string" } },
                "online_hosts": { "type": "array", "items": { "type": "string" } },
                "sandbox_ready": { "type": "boolean" },
            })
        },
        required: &[
            "original",
            "slug",
            "title",
            "summary",
            "repo_url",
            "license_spdx",
            "kind",
            "note",
        ],
    },
    Tool {
        name: "add_dedicated_server",
        description: "Adds a dedicated server to one of your games: a program players run on their own computer or a host so others can play. It's shown on the game's store page, not listed on its own, and lives in its own repository (never the game's). It starts as a draft: submit a release for it like any app, and it's checked and reviewed the same way. Not for servers only you run: for those, add own_servers to the game's online. A game can have several. Read the guide section \"Dedicated servers\" first.",
        method: Method::POST,
        path: "/v1/apps/{game}/servers",
        properties: || {
            json!({
                "game": s("Your game's store address"),
                "slug": s("The server's own address (it shows its game's page), e.g. the game's plus -server"),
                "title": s("Its name, e.g. the game's name plus Dedicated server, or Lobby server"),
                "summary": s("One line for players: what it is and what it hosts"),
                "repo_url": s("The server's own public repository"),
                "license_spdx": s("Its licence, as an SPDX ID"),
                "platforms": { "type": "array", "items": { "type": "string" }, "description": "windows and/or linux (with linux-deb and/or linux-rpm), as for any app" },
                "description_md": s("More for the people hosting it, Markdown: what it loads and from where, ports, settings"),
                "min_age": { "type": "integer", "description": "At least the game's; the game's when left out" },
                "content_labels": { "type": "array", "items": { "type": "string" } },
                "online": { "type": "array", "items": { "type": "string" } },
                "online_hosts": { "type": "array", "items": { "type": "string" } },
                "sandbox_ready": { "type": "boolean" },
            })
        },
        required: &[
            "game",
            "slug",
            "title",
            "summary",
            "repo_url",
            "license_spdx",
            "platforms",
        ],
    },
    Tool {
        name: "offer_fix_to_original",
        description: "Offers your community build's changes back to the original app: open the pull request on the original's repository yourself (e.g. with gh pr create), then register it here so the original's maker is told on SLAPorSLOP.",
        method: Method::POST,
        path: "/v1/apps/{slug}/offers",
        properties: || {
            json!({
                "slug": s("Your community build's store address"),
                "pull_request_url": s("The pull or merge request on the original's repository"),
                "message": s("What it fixes or adds, for the original's maker: up to 2,000 characters"),
            })
        },
        required: &["slug", "pull_request_url", "message"],
    },
    Tool {
        name: "list_offers",
        description: "Fixes offered to your app by community builds, and the ones your community build offered, with their status.",
        method: Method::GET,
        path: "/v1/apps/{slug}/offers",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "answer_offer",
        description: "Accepts or declines a fix offered to your app (merging the pull request is done on the repository). The community build's maker is told.",
        method: Method::POST,
        path: "/v1/apps/{slug}/offers/{id}/answer",
        properties: || {
            json!({
                "slug": s("Your app's store address"),
                "id": s("The offer's ID, from list_offers"),
                "decision": s("accepted or declined"),
                "message": s("A short reply, up to 2,000 characters"),
            })
        },
        required: &["slug", "id", "decision"],
    },
    Tool {
        name: "find_people",
        description: "Searches People: the artists, composers, writers and other creatives who take paid work on SLAPorSLOP, for when your maker needs art, music, sound, writing or another skill. Each has their page's address (`handle`), a headline, skills, how much AI they use (`ai_use`), whether they're taking work, their lowest price, and their rating from past clients. Read one with get_person. Hiring is for your maker: give them the link to the person's page (`/people/` plus the handle), where they press Hire. You can add the request to that link, so the form opens filled in for them to check and send (maker_guide, \"Finding someone to hire\").",
        method: Method::GET,
        path: "/v1/people",
        properties: || {
            json!({
                "skill": s("One skill, by its slug from list_skills (e.g. pixel-art, music)"),
                "q": s("Words in their name, headline or page address"),
                "taking_work": { "type": "boolean", "description": "Only people taking work now" },
                "ai": s("none, assisted or heavy: only people who use AI exactly that much"),
                "studios": { "type": "boolean", "description": "Only studios (teams)" },
                "sort": s("rating or clients; left out, the order is shuffled daily so nobody is buried"),
                "limit": { "type": "integer", "description": "How many, 1 to 200; 20 is plenty to choose from. 200 when left out" },
            })
        },
        required: &[],
    },
    Tool {
        name: "get_person",
        description: "One person's People page: what they do (`about_md`), their services with prices and delivery days, portfolio works, how they use AI, turnaround, reviews from past clients and the projects they worked on. With People access, your own person's page (by their handle, from my_people_page) also says where it stands and previews a portfolio that's still in review.",
        method: Method::GET,
        path: "/v1/people/{handle}",
        properties: || json!({ "handle": s("The page's address, from find_people") }),
        required: &["handle"],
    },
    Tool {
        name: "list_skills",
        description: "The skills People pages can list, each with its slug and name: for find_people's `skill`, and for a People page's `skills`.",
        method: Method::GET,
        path: "/v1/people/skills",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "my_people_page",
        description: "Your person's own People page as they saved it: handle, headline, about, skills, services and prices, links, portfolio repository, and where the page stands (`status`: draft, waiting for review, turned down, live or hidden). 404 when they have none yet. Changing it is for the person (you can prepare a link that fills the editor in: see the portfolio guide). Needs a token with People access.",
        method: Method::GET,
        path: "/v1/me/creative",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "my_portfolio",
        description: "Your person's portfolio as SLAPorSLOP has it: the repository, the approved one that shows on their page (`live`), one waiting for a person's review (`pending`), the newest one turned down with the reviewer's reason (`rejected`), when it was last read (`checked_at`) and what was wrong then (`problem`: one line for each problem, naming the work and the field). Needs a token with People access.",
        method: Method::GET,
        path: "/v1/me/creative/portfolio",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "check_my_portfolio",
        description: "\"Check my portfolio now\": reads `.slop/portfolio.json` at the newest commit of your person's portfolio repository and checks every work. `result` says what happened: `pending` (it's new and now waits for a person's review, replacing one that was still waiting), `unchanged` (the same as the one approved or waiting), `none` (no portfolio.json there) or `problem` (`problem` lists every one: fix them, push, and check again). Dry-run first without sending anything: GET /v1/check/portfolio?url=…. A portfolio that was turned down isn't taken again unchanged. `repo_url` only when the page has no repository yet (it must be a public GitHub repository of their linked GitHub account); changing it is for the person. Needs a publish token with People access.",
        method: Method::POST,
        path: "/v1/me/creative/portfolio/check",
        properties: || {
            json!({
                "repo_url": s("The portfolio repository, only while none is set, e.g. https://github.com/you/portfolio"),
            })
        },
        required: &[],
    },
    Tool {
        name: "my_jobs",
        description: "People jobs you're a side of (as the creative or the client): requests waiting for your quote, offers, work in progress, with each job's step and what it waits for. Needs a token with People access.",
        method: Method::GET,
        path: "/v1/me/jobs",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "get_job",
        description: "One job: the request, the offer and agreed terms, changes, notes, and what each side can do next.",
        method: Method::GET,
        path: "/v1/jobs/{id}",
        properties: || json!({ "id": s("The job's ID") }),
        required: &["id"],
    },
    Tool {
        name: "read_job_conversation",
        description: "The job's conversation so far (recorded: both sides can download it). Only while it's open.",
        method: Method::GET,
        path: "/v1/jobs/{id}/conversation",
        properties: || json!({ "id": s("The job's ID") }),
        required: &["id"],
    },
    Tool {
        name: "send_job_message",
        description: "Sends a message in the job's open conversation, as the person you work for. Say you're their AI assistant when it matters. Never agree to terms in messages: terms are only what offers and changes say.",
        method: Method::POST,
        path: "/v1/jobs/{id}/messages",
        properties: || {
            json!({
                "id": s("The job's ID"),
                "body": s("The message, up to 4,000 characters"),
                "recording_ack": { "type": "boolean", "description": "True once the person you work for accepted that the conversation is recorded" },
            })
        },
        required: &["id", "body"],
    },
    Tool {
        name: "send_quote",
        description: "As the creative: answers a request with an offer (the terms the client accepts or not). Only send what the person you work for decided: price, dates and rights are binding once the client accepts.",
        method: Method::POST,
        path: "/v1/jobs/{id}/offer",
        properties: || {
            json!({
                "id": s("The job's ID"),
                "title": s("What's being made, up to 100 characters"),
                "scope_md": s("Exactly what's included, Markdown"),
                "price_minor": { "type": "integer", "description": "The price in the currency's smallest unit (cents)" },
                "currency": s("e.g. usd, eur, gbp"),
                "payment": s("upfront, split (half now, half on delivery) or on_delivery"),
                "due_date": s("YYYY-MM-DD"),
                "sign_off": { "type": "boolean", "description": "The client signs off on a sketch or demo first" },
                "rights": s("transfer (the client owns it) or licence (for this project)"),
                "open_source": { "type": "boolean", "description": "May be published under the project's open-source licence" },
                "credit": { "type": "boolean", "description": "Credited on the project's page" },
                "ai_use": s("none, assisted or heavy: how much AI the creative uses for this job"),
                "refund_policy": s("Optional refund terms"),
            })
        },
        required: &[
            "id",
            "title",
            "price_minor",
            "currency",
            "payment",
            "due_date",
            "rights",
            "open_source",
            "credit",
            "ai_use",
        ],
    },
    Tool {
        name: "decline_job",
        description: "Declines a request (as the creative) or turns down an offer (as the client), with a short note why.",
        method: Method::POST,
        path: "/v1/jobs/{id}/decline",
        properties: || json!({ "id": s("The job's ID"), "note": s("Why, briefly") }),
        required: &["id"],
    },
    Tool {
        name: "propose_change",
        description: "As the creative: offers a Change to Agreement (new work, extra price, new date), answering the client's request or on your own.",
        method: Method::POST,
        path: "/v1/jobs/{id}/change/offer",
        properties: || {
            json!({
                "id": s("The job's ID"),
                "title": s("What changes"),
                "scope_md": s("Exactly what the change includes"),
                "extra_price_minor": { "type": "integer", "description": "Extra price in cents; 0 for none" },
                "new_due_date": s("YYYY-MM-DD"),
            })
        },
        required: &["id", "title", "extra_price_minor", "new_due_date"],
    },
    Tool {
        name: "request_change",
        description: "As the client: asks the creative for a change to the agreed work.",
        method: Method::POST,
        path: "/v1/jobs/{id}/change/request",
        properties: || json!({ "id": s("The job's ID"), "note": s("What you'd like changed") }),
        required: &["id", "note"],
    },
    Tool {
        name: "decline_change",
        description: "Declines a requested or offered change; the work stays as agreed.",
        method: Method::POST,
        path: "/v1/jobs/{id}/change/decline",
        properties: || json!({ "id": s("The job's ID"), "note": s("Why, briefly") }),
        required: &["id"],
    },
    Tool {
        name: "open_job_conversation",
        description: "As the creative: opens the job's conversation (the client asked for one, or you want one).",
        method: Method::POST,
        path: "/v1/jobs/{id}/conversation/open",
        properties: || json!({ "id": s("The job's ID") }),
        required: &["id"],
    },
    Tool {
        name: "ask_for_job_conversation",
        description: "As the client: asks the creative to open a conversation.",
        method: Method::POST,
        path: "/v1/jobs/{id}/conversation/request",
        properties: || json!({ "id": s("The job's ID"), "note": s("What you'd like to talk about") }),
        required: &["id"],
    },
    Tool {
        name: "close_job_conversation",
        description: "Closes the job's conversation. Its text stays downloadable for 30 days.",
        method: Method::POST,
        path: "/v1/jobs/{id}/conversation/close",
        properties: || json!({ "id": s("The job's ID") }),
        required: &["id"],
    },
    Tool {
        name: "withdraw_offer",
        description: "Withdraws a fix your community build offered.",
        method: Method::POST,
        path: "/v1/apps/{slug}/offers/{id}/withdraw",
        properties: || {
            json!({
                "slug": s("Your community build's store address"),
                "id": s("The offer's ID"),
            })
        },
        required: &["slug", "id"],
    },
    Tool {
        name: "list_staff_contacts",
        description: "SLAPorSLOP's staff you can address a message to, by id and roles (e.g. someone with the reviewer role for a question about a release review; a name only when they chose it), and when your maker's account may post next. Leave out `to` in send_admin_message to reach every admin, which is usually right.",
        method: Method::GET,
        path: "/v1/staff-contacts",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "send_admin_message",
        description: ADMIN_MESSAGE_RULES,
        method: Method::POST,
        path: "/v1/admin-messages",
        properties: || {
            json!({
                "category": s("report (an error, or a bug in SLAPorSLOP itself), suggestion, or other"),
                "subject": s("One line, up to 120 characters: what it's about"),
                "body": s("Plain text, up to 2,000 characters: what happened, where (the tool or request, the app's address, the release's ID), what you expected, and what you suggest"),
                "to": s("Optional: a staff member's id from list_staff_contacts; leave out to reach every admin"),
            })
        },
        required: &["category", "subject", "body"],
    },
    Tool {
        name: "list_my_admin_messages",
        description: "The threads your maker's account started with SLAPorSLOP's admins, newest first, with every message, the admins' replies (signed with their names), whether each is open or closed, and when the account may post next. Check it when whats_new says the admins answered.",
        method: Method::GET,
        path: "/v1/admin-messages",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "reply_to_admin_message",
        description: "Adds a follow-up to one of your maker's open threads with SLAPorSLOP's admins, e.g. an answer to their question. It counts like a new message: once an hour, 5 a day for the whole account. Plain text, up to 2,000 characters; never secrets or players' personal data.",
        method: Method::POST,
        path: "/v1/admin-messages/{id}/reply",
        properties: || {
            json!({
                "id": s("The thread's id, from list_my_admin_messages"),
                "body": s("Plain text, up to 2,000 characters"),
            })
        },
        required: &["id", "body"],
    },
];

/// The makers' and creatives' MCP server.
const SERVER: crate::agent::Server = crate::agent::Server {
    name: "slop-maker",
    instructions: "SLAPorSLOP, the store for open-source AI-made apps and games, for the maker you work for. Call maker_guide first. Do the writing and fixing yourself in the repository; SLAPorSLOP checks, a person reviews. Never claim an app is \"safe\": say what SLAPorSLOP checked.",
    tools: TOOLS,
    // Not here though they only add: a quote and a change to a job's terms (send_quote,
    // propose_change, request_change) commit the person once the other side accepts, so the AI
    // tool should ask first.
    additive: &[
        "create_store_page",
        "submit_release",
        "submit_play_address",
        "submit_github_release",
        "check_repository",
        "check_pictures",
        "check_my_portfolio",
        "register_community_build",
        "add_dedicated_server",
        "offer_fix_to_original",
        "claim_bounty",
        "send_job_message",
        "open_job_conversation",
        "ask_for_job_conversation",
        "send_admin_message",
        "reply_to_admin_message",
    ],
    prompts: PROMPTS,
    // Results on GitHub exist only where the owner switched them on (commit_status.rs).
    hidden: |c| {
        if crate::commit_status::available(c) {
            &[]
        } else {
            &["set_commit_status"]
        }
    },
};

/// What a person can pick in their AI tool instead of knowing what to ask for.
const PROMPTS: &[Prompt] = &[
    Prompt {
        name: "publish_project",
        title: "Publish this project",
        description: "Gets the project you have open onto SLAPorSLOP: store page, pictures, builds and the first release.",
        arguments: &[(
            "address",
            "The store address you'd like, e.g. pixel-dungeon (optional)",
            false,
        )],
        text: "Help me publish the project in this folder on SLAPorSLOP.
I'd like its store address to be {address}.

1. Call maker_guide and whats_left first, and follow the guide.
2. Check the repository is public and has an OSI licence file. If something SLAPorSLOP doesn't allow is in the way (closed-source parts, purchases, a self-updater), tell me before going on.
3. Write the store page from the code and the README and create it. Don't invent features.
4. Add the pictures (an icon, a thumbnail, screenshots), push, and have SLAPorSLOP read them.
5. Run check_source on what's pushed and fix what it lists, so the first release doesn't come back for it.
6. Set up releases with Slop Build, or submit the builds I already have. A game that plays on my own website is a browser game: give its address instead.
7. Follow the first release until SLAPorSLOP's checks are done, and fix whatever get_fixes lists.

Ask me before anything that's mine to decide: the name, the age rating, the stage. Finish with whats_left, and tell me in plain words which steps are mine, with their links.",
    },
    Prompt {
        name: "fix_last_release",
        title: "Fix my last release",
        description: "Reads what SLAPorSLOP's checks found in your newest release, fixes it in the repository and releases again.",
        arguments: &[(
            "app",
            "The app's store address (optional when you have one app)",
            false,
        )],
        text: "My newest release on SLAPorSLOP needs fixing.
The app's store address is {app}.

1. If I didn't say which app, find it with my_apps (ask me when there's more than one).
2. Call list_releases with limit 1 for where the release stands, then get_fixes for every problem and how to fix it.
3. Fix the `must` problems in the repository, then the `should` ones. Tell me about any that needs my decision.
4. Commit and push, then run check_source on the new commit to see the source problems are gone.
5. Release again with a new version, with notes for players on what changed.
6. Follow the new release with list_releases (limit 1) every few minutes until its checks are done.

Tell me what you changed and what's still waiting, for a person at SLAPorSLOP or for me.",
    },
    Prompt {
        name: "whats_left",
        title: "What's left to do?",
        description: "Everything still open across your apps, agreements and People page, and who does each step.",
        arguments: &[],
        text: "Call whats_left and whats_new on SLAPorSLOP. Tell me in plain words what's waiting for me (with the links), what's waiting for SLAPorSLOP, and what you can do yourself. Then do your own steps, and tell me what you did.",
    },
    Prompt {
        name: "player_feedback",
        title: "What are players saying?",
        description: "Sums up reviews, bug reports and how each release landed, and suggests what to fix first.",
        arguments: &[(
            "app",
            "The app's store address (optional when you have one app)",
            false,
        )],
        text: "Tell me what players think of my app on SLAPorSLOP.
The app's store address is {app}.

If I didn't say which app, find it with my_apps (ask me when there's more than one). Read release_feedback, list_reviews, list_bug_reports and app_ratings. Then tell me, briefly: what players like, what they complain about most, which bugs the most players hit, and whether the last release made things better or worse. Suggest what to fix first, and where it is in the code. Don't reply to reviews or change bug reports unless I ask.",
    },
    Prompt {
        name: "set_up_people_page",
        title: "Set up my People page",
        description: "For artists, composers and other creatives: builds your portfolio repository and prepares your People page.",
        arguments: &[(
            "work",
            "What you do, e.g. pixel art and animation (optional)",
            false,
        )],
        text: "Help me set up my People page and portfolio on SLAPorSLOP, so makers can hire me.
What I do: {work}.

Read {site}/portfolio.md and follow its checklist for agents. Ask me what I make, what work I want to be hired for and which pieces I'm proudest of. Build the portfolio in a public GitHub repository of mine, and prepare my People page as one link that fills the editor in. Once my page exists and the portfolio is pushed, have it checked with check_my_portfolio if your token has People access (otherwise I press Check my portfolio now), and fix what it lists. Never decide for me whether I'm 18 or older, whether I work as a business, my prices, or anything about agreements and payments. Finish by telling me the steps that are mine, with their links.",
    },
    Prompt {
        name: "find_someone_to_hire",
        title: "Find someone to hire",
        description: "Searches People for artists, composers or other creatives who fit what your project needs.",
        arguments: &[(
            "need",
            "What you need, e.g. a chiptune soundtrack or pixel art for a platformer",
            true,
        )],
        text: "I'm looking for someone to hire on SLAPorSLOP's People for my project. I need: {need}.

Call list_skills, then find_people with the skill that fits (limit 20, taking work), and read the pages of the ones that fit best with get_person. Show me up to five: who they are, what they offer and at what price, how much AI they use, what past clients said, and the link to their page ({site}/people/ plus their handle). Don't rank them by anything I didn't ask for. Asking for a quote is mine to do: I press Hire on their page. Once I've picked someone, offer to write the request with me and give me the link that opens their page with it filled in (maker_guide, \"Finding someone to hire\"): I check it and send it myself.",
    },
];

/// What a maker's agent is told before writing to SLAPorSLOP's admins.
const ADMIN_MESSAGE_RULES: &str = "Writes to SLAPorSLOP's admins: an error you hit on SLAPorSLOP, a bug in SLAPorSLOP itself, or a suggestion (not your maker's own app's bugs: fix those in its repository). Your maker's whole account may post once an hour and 5 times a day (a new thread or a reply, all its tokens together), so write only when it matters. Be brief and thorough in one message, so you don't spend your maker's tokens on follow-ups: what happened, where, what you expected, what you suggest. Plain text only, under 2,000 characters, at most 3 links. Only staff and makers with an app in the store, with accounts at least 7 days old, can write, with a token at least an hour old. Never include secrets, tokens or players' personal data. The admins answer in the thread: you'll see it in whats_new and list_my_admin_messages.";

/// The MCP path a request came in on, for the discovery documents (agent_oauth.rs).
fn mcp_path(request: &Request) -> &'static str {
    if request.uri().path() == "/mcp" {
        "/mcp"
    } else {
        "/v1/maker/mcp"
    }
}

async fn mcp(
    State(state): State<AppState>,
    user: Result<CurrentUser, ApiError>,
    request: Request,
) -> Response {
    let user = match user {
        Ok(user) => user,
        // No working token: say where an AI tool finds out how to get one (RFC 9728).
        Err(ApiError::Unauthorized) => {
            let had_token = request
                .headers()
                .contains_key(axum::http::header::AUTHORIZATION);
            return crate::agent_oauth::unauthorized(&state, mcp_path(&request), had_token);
        }
        Err(err) => return err.into_response(),
    };
    if user.agent.is_none() {
        // The website's own session can't drive the agent endpoint.
        return ApiError::Forbidden.into_response();
    }
    crate::agent::serve_mcp(&state, request, &SERVER).await
}

async fn mcp_get(
    State(state): State<AppState>,
    user: Result<CurrentUser, ApiError>,
    request: Request,
) -> Response {
    match user {
        Ok(_) => StatusCode::METHOD_NOT_ALLOWED.into_response(),
        Err(ApiError::Unauthorized) => {
            let had_token = request
                .headers()
                .contains_key(axum::http::header::AUTHORIZATION);
            crate::agent_oauth::unauthorized(&state, mcp_path(&request), had_token)
        }
        Err(err) => err.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_maker_routes_match() {
        let get = Method::GET;
        assert_eq!(
            route(&get, "/v1/apps/pong/releases"),
            Some((false, Slug::Own, Some("pong")))
        );
        assert_eq!(
            route(&get, "/v1/apps/pong/history"),
            Some((false, Slug::Own, Some("pong")))
        );
        assert_eq!(
            route(&Method::PATCH, "/v1/apps/pong"),
            Some((true, Slug::Own, Some("pong")))
        );
        assert_eq!(
            route(&Method::PATCH, "/v1/apps/pong/releases/abc"),
            Some((true, Slug::Own, Some("pong")))
        );
        assert_eq!(
            route(&Method::POST, "/v1/apps"),
            Some((true, Slug::Any, None))
        );
        assert_eq!(
            route(&Method::POST, "/v1/apps/pong/branches"),
            Some((true, Slug::Any, Some("pong")))
        );
        assert_eq!(
            route(&Method::PUT, "/v1/apps/pong/commit-status"),
            Some((true, Slug::Own, Some("pong")))
        );
        // News: reading needs any token of the maker's, the rest a publish token.
        for (m, p, publish) in [
            (Method::GET, "/v1/apps/pong/news", false),
            (Method::POST, "/v1/apps/pong/news", true),
            (Method::PUT, "/v1/apps/pong/news/n1", true),
            (Method::DELETE, "/v1/apps/pong/news/n1", true),
        ] {
            assert_eq!(
                route(&m, p),
                Some((publish, Slug::Own, Some("pong"))),
                "{m} {p}"
            );
        }
        for (m, p) in [
            // The news switch on an Interested mark is the person's.
            (Method::PUT, "/v1/me/interested/pong/news"),
            (Method::PUT, "/v1/me/interested/pong"),
            (Method::GET, "/v1/me/export"),
            (Method::POST, "/v1/me/maker-tokens"),
            (Method::POST, "/v1/apps/pong/donate"),
            (Method::POST, "/v1/apps/pong/vote"),
            (Method::GET, "/v1/admin/status"),
            (Method::DELETE, "/v1/apps/pong"),
            (Method::GET, "/v1/apps/pong/releases/x/download/windows"),
            (Method::POST, "/v1/apps/pong/changes//cancel"),
            (Method::GET, "/v2/apps/pong"),
            (Method::GET, "/v1/apps//releases"),
            (Method::GET, "/v1/admin/ai-messages"),
            (Method::POST, "/v1/admin-messages/x/close"),
            (Method::DELETE, "/v1/admin-messages/x"),
        ] {
            assert_eq!(route(&m, p), None, "{m} {p}");
        }
    }

    #[test]
    fn writing_to_the_admins_is_open_to_every_token() {
        for (m, p) in [
            (Method::GET, "/v1/staff-contacts"),
            (Method::GET, "/v1/admin-messages"),
            (Method::POST, "/v1/admin-messages"),
            (Method::POST, "/v1/admin-messages/x/reply"),
        ] {
            assert_eq!(route(&m, p), Some((false, Slug::Any, None)), "{m} {p}");
        }
    }

    #[test]
    fn any_token_looks_at_people_and_only_looks() {
        for p in ["/v1/people", "/v1/people/skills", "/v1/people/vera"] {
            assert_eq!(
                route(&Method::GET, p),
                Some((false, Slug::Any, None)),
                "{p}"
            );
        }
        // What's personal needs People access: the creative's own page and portfolio, and the
        // day's shelves (they say what the viewer has hired for).
        for p in [
            "/v1/me/creative",
            "/v1/me/creative/portfolio",
            "/v1/people/discover",
        ] {
            assert_eq!(
                route(&Method::GET, p),
                Some((false, Slug::People, None)),
                "{p}"
            );
        }
        // Having the portfolio read again sends what's new for review: a publish token's.
        assert_eq!(
            route(&Method::POST, "/v1/me/creative/portfolio/check"),
            Some((true, Slug::People, None))
        );
        for (m, p) in [
            (Method::POST, "/v1/people/vera/hire"),
            (Method::GET, "/v1/people/vera/hire"),
            (Method::PUT, "/v1/people/reviews/x/reply"),
            (Method::PUT, "/v1/me/creative"),
            (Method::PUT, "/v1/me/creative/portfolio/order"),
            (Method::POST, "/v1/me/creative/stripe"),
            (Method::PUT, "/v1/apps/pong/update-settings"),
        ] {
            assert_eq!(route(&m, p), None, "{m} {p}");
        }
    }

    #[test]
    fn the_tools_and_prompts_hold_together() {
        crate::agent::check_server(&SERVER);
    }

    #[test]
    fn tool_calls_become_maker_requests() {
        let call = |name: &str, args: Value| {
            let tool = TOOLS.iter().find(|t| t.name == name).unwrap();
            crate::agent::tool_request(tool, args.as_object().unwrap())
        };
        // A rollback names the release; following the newest again is a null, which is still sent.
        let (m, path, body) = call(
            "hold_at_release",
            json!({ "slug": "pong", "kind": "release", "release_id": null }),
        )
        .unwrap();
        assert_eq!((m, path.as_str()), (Method::POST, "/v1/apps/pong/changes"));
        assert_eq!(body, Some(json!({ "kind": "release", "release_id": null })));
        assert!(
            call(
                "hold_at_release",
                json!({ "slug": "pong", "kind": "release" })
            )
            .is_err()
        );
        // Taking something away sends no body.
        for (name, args, want) in [
            (
                "remove_review_reply",
                json!({ "slug": "pong", "id": "r1" }),
                "/v1/apps/pong/reviews/r1/reply",
            ),
            (
                "end_browser_demo",
                json!({ "slug": "pong" }),
                "/v1/apps/pong/demo",
            ),
            (
                "remove_news",
                json!({ "slug": "pong", "id": "n1" }),
                "/v1/apps/pong/news/n1",
            ),
        ] {
            let (m, path, body) = call(name, args).unwrap();
            assert_eq!((m, path.as_str(), body), (Method::DELETE, want, None));
        }
        let (_, path, body) = call(
            "cancel_pending_change",
            json!({ "slug": "pong", "id": "c1" }),
        )
        .unwrap();
        assert_eq!(
            (path.as_str(), body),
            ("/v1/apps/pong/changes/c1/cancel", None)
        );
        let (_, path, _) = call(
            "find_people",
            json!({ "skill": "pixel-art", "taking_work": true, "limit": 5 }),
        )
        .unwrap();
        assert_eq!(path, "/v1/people?limit=5&skill=pixel-art&taking_work=true");
        let (_, path, _) = call("list_releases", json!({ "slug": "pong", "limit": 1 })).unwrap();
        assert_eq!(path, "/v1/apps/pong/releases?limit=1");
        let (m, path, body) = call("get_history", json!({ "slug": "pong", "days": 90 })).unwrap();
        assert_eq!(
            (m, path.as_str(), body),
            (Method::GET, "/v1/apps/pong/history?days=90", None)
        );
        // News: the headline and text are the body; the post's ID goes in the path.
        let news =
            json!({ "slug": "pong", "headline": "Beta on Friday", "body": "Saves work now." });
        let (m, path, body) = call("post_news", news.clone()).unwrap();
        assert_eq!((m, path.as_str()), (Method::POST, "/v1/apps/pong/news"));
        assert_eq!(
            body,
            Some(json!({ "headline": "Beta on Friday", "body": "Saves work now." }))
        );
        let mut edit = news;
        edit["id"] = json!("n1");
        let (m, path, body) = call("edit_news", edit).unwrap();
        assert_eq!((m, path.as_str()), (Method::PUT, "/v1/apps/pong/news/n1"));
        assert_eq!(
            body,
            Some(json!({ "headline": "Beta on Friday", "body": "Saves work now." }))
        );
        let (m, path, body) = call("list_news", json!({ "slug": "pong" })).unwrap();
        assert_eq!(
            (m, path.as_str(), body),
            (Method::GET, "/v1/apps/pong/news", None)
        );
        // Older posts: the page after the last one it has.
        let (m, path, body) = call(
            "list_news",
            json!({ "slug": "pong", "before": "n9", "limit": 50 }),
        )
        .unwrap();
        assert_eq!(
            (m, path.as_str(), body),
            (Method::GET, "/v1/apps/pong/news?before=n9&limit=50", None)
        );
        let (_, path, _) = call("get_person", json!({ "handle": "../me/creative" })).unwrap();
        assert_eq!(path, "/v1/people/..%2Fme%2Fcreative");
    }

    #[test]
    fn guide_sections_are_found() {
        let all = sections();
        let names: Vec<&str> = all.iter().map(|(t, _)| t.as_str()).collect();
        for want in [
            "Saves and sandbox mode",
            "Online play and mods",
            "Slop Pulse (player counts)",
        ] {
            assert!(names.contains(&want), "{want} in {names:?}");
        }
        let (_, saves) = all
            .iter()
            .find(|(t, _)| t == "Saves and sandbox mode")
            .unwrap();
        assert!(saves.contains("SLOP_SAVE_DIR"));
        assert!(
            !saves.contains("## SLAPorSLOP in your code"),
            "stops at the next section"
        );
    }

    #[test]
    fn every_tool_is_an_allowed_route() {
        for t in TOOLS {
            let path = [
                "{section}",
                "{id}",
                "{slug}",
                "{original}",
                "{game}",
                "{handle}",
            ]
            .iter()
            .fold(t.path.to_string(), |p, v| p.replace(v, "x"));
            assert!(
                route(&t.method, &path).is_some(),
                "{}: {} {path}",
                t.name,
                t.method
            );
            for r in t.required {
                assert!((t.properties)().get(*r).is_some(), "{}: {r}", t.name);
            }
        }
    }
}
