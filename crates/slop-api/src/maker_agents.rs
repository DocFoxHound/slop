// SPDX-License-Identifier: AGPL-3.0-or-later
//! Makers' AI agents (DESIGN §8, plan phase 10). A maker makes a token (Account → AI agents) and
//! gives it to their coding agent, which can then publish and fix their apps from the editor:
//! create and edit store pages, submit releases with notes, read Slop's checks of each release,
//! and get a ready-made fix for every problem found (fixes.rs).
//!
//! The thinking happens in the maker's agent, never on Slop's servers: Slop gives it tools, data
//! and instructions, and calls no AI model.
//!
//! - **Tokens** (`slopmk_…`) are sessions of kind `agent`, so they show on the maker's devices
//!   page and sign out like any session. Each has a scope (`read`, or `publish` to also change
//!   store pages and submit releases), an optional list of apps, and expires within 30 days.
//! - **Only maker work:** a token may call the routes in `ALLOWED` and nothing else: never
//!   money, sign-ins, tokens, chat, People or account settings. Releases still go through Slop's
//!   scans and a person's review.
//! - **Passkeys:** a token made within 12 hours of confirming with a passkey counts as
//!   passkey-proven for its lifetime (passkeys.rs `require`), so the agent can publish.
//! - **MCP:** `POST /v1/maker/mcp` speaks the Model Context Protocol, with the tools in `TOOLS`,
//!   each calling the API above with the token.

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
    agent::{Tool, s},
    auth::session::{self, CurrentUser},
    error::{ApiError, ApiResult},
    tokens::{hash_token, new_token},
};

/// Maker tokens start with this, so they're recognisable (and secret scanners can find them).
pub const PREFIX: &str = "slopmk_";
/// The longest a token lives.
pub const MAX_DAYS: i64 = 30;
/// Live tokens per maker.
const MAX_TOKENS: i64 = 10;

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
    /// May handle People jobs (requests, quotes, changes, the job conversation).
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
}

/// (method, path after `/v1`, needs `publish`, what `{slug}` is). `*` is any one segment.
const ALLOWED: &[(&str, &str, bool, Slug)] = &[
    ("GET", "/me", false, Slug::Any),
    ("GET", "/me/apps", false, Slug::Any),
    ("GET", "/me/notifications", false, Slug::Any),
    ("GET", "/categories", false, Slug::Any),
    ("GET", "/runtimes", false, Slug::Any),
    ("GET", "/maker/guide", false, Slug::Any),
    ("GET", "/maker/publishing-guide", false, Slug::Any),
    ("GET", "/maker/publishing-guide/*", false, Slug::Any),
    ("POST", "/maker/mcp", false, Slug::Any),
    ("GET", "/maker/mcp", false, Slug::Any),
    ("GET", "/apps/{slug}", false, Slug::Any),
    ("GET", "/apps/{slug}/manage", false, Slug::Own),
    ("GET", "/apps/{slug}/releases", false, Slug::Own),
    ("GET", "/apps/{slug}/fixes", false, Slug::Own),
    ("GET", "/apps/{slug}/reviews", false, Slug::Own),
    ("GET", "/apps/{slug}/ratings", false, Slug::Own),
    ("GET", "/apps/{slug}/slop-build", false, Slug::Own),
    ("GET", "/apps/{slug}/github-releases", false, Slug::Own),
    ("GET", "/apps/{slug}/update-settings", false, Slug::Own),
    ("GET", "/apps/{slug}/media", false, Slug::Own),
    ("GET", "/apps/{slug}/offers", false, Slug::Own),
    ("POST", "/apps", true, Slug::Any),
    ("POST", "/apps/{slug}/branches", true, Slug::Any),
    ("PATCH", "/apps/{slug}", true, Slug::Own),
    ("POST", "/apps/{slug}/changes", true, Slug::Own),
    ("POST", "/apps/{slug}/changes/*/cancel", true, Slug::Own),
    ("POST", "/apps/{slug}/releases", true, Slug::Own),
    ("POST", "/apps/{slug}/releases/from-github", true, Slug::Own),
    ("PATCH", "/apps/{slug}/releases/*", true, Slug::Own),
    ("POST", "/apps/{slug}/repo-check", true, Slug::Own),
    ("POST", "/apps/{slug}/media/check", true, Slug::Own),
    ("POST", "/apps/{slug}/offers", true, Slug::Own),
    ("PUT", "/apps/{slug}/reviews/*/reply", true, Slug::Own),
    ("GET", "/apps/{slug}/bounties", false, Slug::Any),
    ("GET", "/apps/{slug}/bug-reports", false, Slug::Own),
    ("GET", "/apps/{slug}/release-feedback", false, Slug::Own),
    ("GET", "/apps/{slug}/demo", false, Slug::Own),
    ("POST", "/apps/{slug}/demo", true, Slug::Own),
    ("DELETE", "/apps/{slug}/demo", true, Slug::Own),
    ("GET", "/bug-reports/*", false, Slug::Any),
    // Each route checks the report is on the user's own app.
    ("PATCH", "/bug-reports/*", true, Slug::Any),
    ("GET", "/bounties", false, Slug::Any),
    ("GET", "/bounties/*", false, Slug::Any),
    // Claiming pays the maker; it's checked by the app's maker or the backers.
    ("POST", "/bounties/*/claims", true, Slug::Any),
    ("DELETE", "/apps/{slug}/reviews/*/reply", true, Slug::Own),
    ("POST", "/apps/{slug}/offers/*/answer", true, Slug::Own),
    ("POST", "/apps/{slug}/offers/*/withdraw", true, Slug::Own),
    // People: never accepting agreements or changes, paying, signing off, delivering,
    // withdrawing or cancelling. Those stay with the person.
    ("GET", "/me/jobs", false, Slug::People),
    ("GET", "/me/creative", false, Slug::People),
    ("GET", "/people/skills", false, Slug::People),
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
];

/// The allowed route a request matches: whether it needs `publish`, how its slug is checked,
/// and the slug.
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
            (Slug::Any | Slug::People, _) | (Slug::Own, None) => Ok(()),
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

/// The maker a `slopmk_` token signs in as, with its limits.
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
         where s.token_hash = $1 and s.kind = 'agent' and u.closed_at is null and {live}"
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

// ---------------------------------------------------------------------------------------------
// Tokens

#[derive(Serialize)]
struct TokenInfo {
    id: Uuid,
    name: String,
    scope: String,
    /// App addresses; empty: all the maker's apps.
    apps: Vec<String>,
    /// Also handles People jobs.
    people: bool,
    /// Made within 12 hours of a passkey confirmation, so it can publish (passkeys.rs).
    passkey_proved: bool,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    expires_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    last_used_at: OffsetDateTime,
}

const TOKEN_COLUMNS: &str = "s.id, m.name, m.scope,
    array(select a.slug from apps a where a.id = any(m.app_ids) order by a.slug), m.people,
    s.passkey_at is not null, s.created_at, s.expires_at, s.last_seen_at";

type TokenRow = (
    Uuid,
    String,
    String,
    Vec<String>,
    bool,
    bool,
    OffsetDateTime,
    OffsetDateTime,
    OffsetDateTime,
);

fn info(r: TokenRow) -> TokenInfo {
    TokenInfo {
        id: r.0,
        name: r.1,
        scope: r.2,
        apps: r.3,
        people: r.4,
        passkey_proved: r.5,
        created_at: r.6,
        expires_at: r.7,
        last_used_at: r.8,
    }
}

async fn list(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Vec<TokenInfo>>> {
    let rows: Vec<TokenRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {TOKEN_COLUMNS} from sessions s join maker_tokens m on m.session_id = s.id
         where s.user_id = $1 and s.kind = 'agent' and s.revoked_at is null and s.expires_at > now()
         order by s.created_at desc"
    )))
    .bind(user.id)
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
    /// 1 to 30; 30 by default.
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
    /// Shown once; Slop keeps only its hash.
    secret: String,
    mcp_url: String,
    /// Connects Claude Code to Slop with this token.
    claude_command: String,
}

async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<NewToken>,
) -> ApiResult<(StatusCode, Json<Created>)> {
    session::require_recent_sign_in(&state.db, &user).await?;
    let name = body.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::invalid("name", "1 to 60 characters"));
    }
    if body.scope != "read" && body.scope != "publish" {
        return Err(ApiError::invalid("scope", "read or publish"));
    }
    let days = body.days.unwrap_or(MAX_DAYS);
    if !(1..=MAX_DAYS).contains(&days) {
        return Err(ApiError::invalid("days", "1 to 30"));
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
    let live: i64 = sqlx::query_scalar(
        "select count(*) from sessions where user_id = $1 and kind = 'agent' and revoked_at is null and expires_at > now()",
    )
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;
    if live >= MAX_TOKENS {
        return Err(ApiError::Conflict(format!(
            "you have {MAX_TOKENS} agent tokens; remove one first"
        )));
    }
    let secret = format!("{PREFIX}{}", new_token());
    let id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    // A passkey confirmation in the last 12 hours carries over to the token (passkeys.rs).
    sqlx::query(
        "insert into sessions (id, user_id, token_hash, user_agent, expires_at, kind, passkey_at)
         values ($1, $2, $3, $4, now() + $5, 'agent',
                 (select passkey_at from sessions where id = $6 and passkey_at > now() - $7))",
    )
    .bind(id)
    .bind(user.id)
    .bind(hash_token(&secret))
    .bind(format!("AI agent: {name}"))
    .bind(Duration::days(days))
    .bind(user.session_id)
    .bind(crate::passkeys::PROOF_WINDOW)
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
            claude_command: format!(
                "claude mcp add --transport http slop {mcp_url} --header \"Authorization: Bearer {secret}\""
            ),
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
// Guides: short, so an agent reads only what it needs

const GUIDE: &str = include_str!("maker_guide.md");
const PUBLISHING: &str = include_str!("../../../web/public/publishing.md");

async fn guide(_user: CurrentUser) -> impl IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/markdown; charset=utf-8",
        )],
        GUIDE,
    )
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
        "summary": s("One line for tiles, up to 160 characters"),
        "description_md": s("The store page's description, Markdown, up to 20,000 characters: what it is, how to play, what's different. Plain words; no claims it's \"safe\""),
        "repo_url": s("The public repository, e.g. https://github.com/owner/repo"),
        "license_spdx": s("The repository's licence as an SPDX ID, e.g. MIT, GPL-3.0-or-later"),
        "min_age": { "type": "integer", "description": "Minimum age: 0, 7, 12, 16 or 18" },
        "content_labels": { "type": "array", "items": { "type": "string" }, "description": "Any of romance, suggestive, violence, horror, strong_language, gambling_themes" },
        "tags": { "type": "array", "items": { "type": "string" }, "description": "Tag slugs from list_categories" },
        "platforms": { "type": "array", "items": { "type": "string" }, "description": "windows and/or linux: every release must include exactly these" },
        "online": { "type": "array", "items": { "type": "string" }, "description": "How it goes online, if it does (see the guide section \"Online play and mods\")" },
        "online_hosts": { "type": "array", "items": { "type": "string" }, "description": "The hosts it connects to" },
        "sandbox_ready": { "type": "boolean", "description": "Works in sandbox mode: local saves only in SLOP_SAVE_DIR" },
        "mods_folder": { "type": ["object", "null"], "description": "Where it reads mods from: {base: game|saves, path, layout: folder|flat}; null if it reads SLOP_MODS or has no mods" },
        "bug_reports": { "type": "boolean", "description": "Take bug reports from players on Slop (on by default)" },
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
    m.insert("kind".into(), s("game or app"));
    m.insert("stage".into(), s("coming_soon, beta or released"));
    fields
}

fn slug_only() -> Value {
    json!({ "slug": s("The app's store address") })
}

fn release_fields() -> Value {
    json!({
        "slug": s("The app's store address"),
        "version": s("The version, e.g. 1.2.0"),
        "commit_sha": s("The full 40-character commit SHA it was built from"),
        "files": { "type": "array", "description": "One per platform: {platform: windows|linux, kind: archive|installer, url: the file on your release page, exec: the program inside an archive, args: []}", "items": { "type": "object" } },
        "notes": s("What changed, for players: Markdown, up to 5,000 characters"),
        "runtimes": { "type": "array", "items": { "type": "string" }, "description": "Shared runtimes the Windows build needs, by ID from list_runtimes; leave out to keep the last release's" },
    })
}

pub(crate) const TOOLS: &[Tool] = &[
    Tool {
        name: "maker_guide",
        description: "How to publish and fix an app on Slop with these tools, and Slop's rules in short. Read first.",
        method: Method::GET,
        path: "/v1/maker/guide",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "publishing_guide",
        description: "One section of Slop's full publishing guide (e.g. \"Saves and sandbox mode\", \"Slop Pulse (player counts)\", \"Online play and mods\", \"Option B: Slop Build\"). Without a section: the list of sections. Read only the section you need.",
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
        description: "The notifications of the person you work for, newest first: releases approved or rejected, reviews, offered fixes, job requests and messages. One call to see what needs attention; check it now and then rather than polling every tool.",
        method: Method::GET,
        path: "/v1/me/notifications",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "my_apps",
        description: "Your apps on Slop, including drafts, with their status.",
        method: Method::GET,
        path: "/v1/me/apps",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "app_overview",
        description: "An app's management page: its store page fields, stage, what's waiting for review, and what's needed next.",
        method: Method::GET,
        path: "/v1/apps/{slug}/manage",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "list_releases",
        description: "An app's releases, newest first, each with its status (fetching, awaiting_review, approved, rejected, failed, revoked) and the reason, its files and Slop's checks: the source check (licence, saves, online, code loading, purchases), Slop Build's checks (secrets, vulnerable packages) and file scans.",
        method: Method::GET,
        path: "/v1/apps/{slug}/releases",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "get_fixes",
        description: "Every problem Slop's checks found in the app's newest release, each with why it matters and exact instructions to fix it in the repository. Fix them, push, and submit a new release.",
        method: Method::GET,
        path: "/v1/apps/{slug}/fixes",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "list_reviews",
        description: "Players' reviews of the app (Slop or Not Slop, with text), newest first.",
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
        description: "For each recent release: % Not Slop from votes cast on it, reviews written while it was current with their most common phrases (praised and criticised), and its bug reports. Use it to see what an update changed for players.",
        method: Method::GET,
        path: "/v1/apps/{slug}/release-feedback",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "browser_demo",
        description: "Whether the app can have a browser demo now (new releases only, for a few days), and the demo's state: fetching, live (with its address and plays), failed (why) or ended.",
        method: Method::GET,
        path: "/v1/apps/{slug}/demo",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "host_browser_demo",
        description: "Hosts a browser demo of a newly released game: give the address of a web build (a .zip with index.html at its top, e.g. Godot's Web export) attached to a release on the app's repository. Slop scans and hosts it for its first days, with no network access. Check browser_demo afterwards.",
        method: Method::POST,
        path: "/v1/apps/{slug}/demo",
        properties: || json!({ "slug": s("The app's store address"), "url": s("The web build's address on the release page") }),
        required: &["slug", "url"],
    },
    Tool {
        name: "list_bounties",
        description: "Open fix bounties: what players will pay for, biggest first. With an app's address: that app's bounties. A bounty is paid to whoever delivers it in an approved release of the app or of a community build of it.",
        method: Method::GET,
        path: "/v1/bounties",
        properties: || json!({ "slug": s("Optional: an app's store address, for its bounties only") }),
        required: &[],
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
        description: "Slop's categories and their tags, for a store page's tags.",
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
        description: "How to add Slop Build to the app's repository: the workflow file to commit, with the detected project type. With it, every push builds, signs and submits a release by itself.",
        method: Method::GET,
        path: "/v1/apps/{slug}/slop-build",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "github_releases",
        description: "The releases on the app's GitHub repository that can be submitted with submit_github_release.",
        method: Method::GET,
        path: "/v1/apps/{slug}/github-releases",
        properties: slug_only,
        required: &["slug"],
    },
    Tool {
        name: "create_store_page",
        description: "Creates a draft store page. Write the description yourself from the repository (README, gameplay, controls). Needs a publish token for all your apps.",
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
        description: "Submits a release whose files are attached to a release on the app's repository. Slop downloads, hashes and scans them, then a person reviews it. Check its progress with list_releases.",
        method: Method::POST,
        path: "/v1/apps/{slug}/releases",
        properties: release_fields,
        required: &["slug", "version", "commit_sha", "files"],
    },
    Tool {
        name: "submit_github_release",
        description: "Submits a release from the app's GitHub releases by its tag; its notes default to the GitHub release's text.",
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
        name: "check_repository",
        description: "Runs Slop's source check on the repository's newest commit now, without a release: licence, saves, online code, purchases. Use it after fixing, before releasing.",
        method: Method::POST,
        path: "/v1/apps/{slug}/repo-check",
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
                "summary": s("One line, up to 160 characters"),
                "repo_url": s("Your fork's or mod's public repository"),
                "license_spdx": s("Its licence, as an SPDX ID (a fork keeps the original's terms)"),
                "kind": s("addon (a mod), mod (a modded version), fix, translation, port or other"),
                "note": s("What it changes from the original, for players: up to 500 characters"),
                "platforms": { "type": "array", "items": { "type": "string" }, "description": "windows and/or linux; leave out for a mod" },
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
        name: "offer_fix_to_original",
        description: "Offers your community build's changes back to the original app: open the pull request on the original's repository yourself (e.g. with gh pr create), then register it here so the original's maker is told on Slop.",
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
];

async fn mcp(State(state): State<AppState>, user: CurrentUser, request: Request) -> Response {
    if user.agent.is_none() {
        // The website's own session can't drive the agent endpoint.
        return ApiError::Forbidden.into_response();
    }
    crate::agent::serve_mcp(
        &state,
        request,
        TOOLS,
        "slop-maker",
        "Slop, the store for open-source AI-made apps and games, for the maker you work for. Call maker_guide first. Do the writing and fixing yourself in the repository; Slop checks, a person reviews. Never claim an app is \"safe\": say what Slop checked.",
    )
    .await
}

async fn mcp_get(_user: CurrentUser) -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
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
        for (m, p) in [
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
        ] {
            assert_eq!(route(&m, p), None, "{m} {p}");
        }
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
            !saves.contains("## Slop in your code"),
            "stops at the next section"
        );
    }

    #[test]
    fn every_tool_is_an_allowed_route() {
        for t in TOOLS {
            let path = ["{section}", "{id}", "{slug}", "{original}"]
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
