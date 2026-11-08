// SPDX-License-Identifier: AGPL-3.0-or-later
//! Branches and community builds (DESIGN §9).
//!
//! Anyone can fork an app and register a branch: a mod, a fix, a translation or a port. A branch
//! is an app of its own (its author, its fork, its releases, which are reviewed and signed like
//! any other, its votes and its own donate button), linked to the app it branches from. It isn't
//! listed in the store on its own: it shows on the original's page as a **Community** build,
//! with what platforms it adds, and in the "Community branches" shelf. Installing one needs an
//! "I understand" first: made by its author, not the publisher; checked the same way; may lag
//! behind official updates.
//!
//! A **mod** (kind `addon`) is a branch too: anyone can publish one for any app, from their own
//! repository, and it's checked, reviewed and signed the same way. Its releases are one `.zip`
//! for every platform (`any`) with nothing to start; the SLAPorSLOP app installs it for the game, which
//! loads it. Apps may only load mods approved on SLAPorSLOP (Content Policy, "Code from outside SLAPorSLOP").
//! Mods show beside community builds on the original's page, never in the store on their own.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, PgPool, QueryBuilder};
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::CurrentUser,
    catalog::{self, AppSummary},
    error::{ApiError, ApiResult, is_unique_violation},
    policy::ViewerPolicy,
};

pub const KINDS: &[(&str, &str)] = &[
    ("addon", "Mod"),
    ("mod", "Modded version"),
    ("fix", "Fix"),
    ("translation", "Translation"),
    ("port", "Port"),
    ("other", "Other"),
    ("server", "Dedicated server"),
];

/// SQL: app `a` is a dedicated server linked to its game (docs/DEDICATED-SERVERS.md), which isn't
/// a community build or a mod: it's listed on its game's page and players run it themselves.
pub(crate) const IS_SERVER: &str = "a.branch_kind is not distinct from 'server'";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/apps/{slug}/branches", post(register))
        .route("/apps/{slug}/servers", post(add_server))
}

/// Whether this is a community build of a game whose original counts players with Slop Pulse.
/// Such builds must include Pulse too (DESIGN §9), so player counts and playtime-weighted votes
/// mean the same on both; a reviewer can't list one before its first Pulse handshake. Never for
/// a browser game, which can't send one (and has no community builds yet, `register`), and never
/// for a mod or a dedicated server, which players don't play.
pub async fn needs_pulse(conn: &mut PgConnection, app_id: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "select exists (select 1 from apps b join apps p on p.id = b.parent_app_id
                        where b.id = $1 and p.kind = 'game' and b.branch_kind not in ('addon', 'server')
                          and not ('browser' = any(b.platforms))
                          and exists (select 1 from pulse_sessions s where s.app_id = p.id and s.started_at is not null))",
    )
    .bind(app_id)
    .fetch_one(conn)
    .await
}

/// Whether the app has ever sent a Pulse handshake (a launch through SLAPorSLOP that called `start`).
pub async fn has_pulse(conn: &mut PgConnection, app_id: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "select exists (select 1 from pulse_sessions where app_id = $1 and started_at is not null)",
    )
    .bind(app_id)
    .fetch_one(conn)
    .await
}

#[derive(Serialize)]
pub struct BranchInfo {
    /// The app it branches from.
    parent_slug: String,
    parent_title: String,
    parent_maker: String,
    kind: String,
    kind_label: &'static str,
    note: Option<String>,
}

/// For a branch's own page: what it branches from. A dedicated server isn't a branch (`server_for`).
pub async fn info(db: &PgPool, app_id: Uuid) -> sqlx::Result<Option<BranchInfo>> {
    let row: Option<(String, String, String, String, Option<String>)> = sqlx::query_as(
        "select p.slug, p.title, u.display_name, a.branch_kind, a.branch_note
         from apps a join apps p on p.id = a.parent_app_id join users u on u.id = p.maker_id
         where a.id = $1 and a.branch_kind <> 'server'",
    )
    .bind(app_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(
        |(parent_slug, parent_title, parent_maker, kind, note)| BranchInfo {
            kind_label: KINDS
                .iter()
                .find(|(k, _)| *k == kind)
                .map_or("Other", |(_, l)| l),
            parent_slug,
            parent_title,
            parent_maker,
            kind,
            note,
        },
    ))
}

/// The game a dedicated server is for.
#[derive(Serialize, sqlx::FromRow)]
pub struct ServerFor {
    slug: String,
    title: String,
}

/// For a dedicated server's own data: its game (its store address shows the game's page).
pub async fn server_for(db: &PgPool, app_id: Uuid) -> sqlx::Result<Option<ServerFor>> {
    sqlx::query_as(
        "select p.slug, p.title from apps a join apps p on p.id = a.parent_app_id
         where a.id = $1 and a.branch_kind = 'server'",
    )
    .bind(app_id)
    .fetch_optional(db)
    .await
}

/// A dedicated server, for its game's page.
#[derive(Serialize, sqlx::FromRow)]
pub struct Server {
    #[sqlx(flatten)]
    #[serde(flatten)]
    app: AppSummary,
    #[serde(skip)]
    app_id: Uuid,
    platforms: Vec<String>,
    /// The release players get, when it has one.
    #[sqlx(skip)]
    release: Option<ServerRelease>,
}

#[derive(Serialize)]
pub struct ServerRelease {
    id: Uuid,
    version: String,
    files: Vec<ServerFile>,
}

/// A server's file, for downloading it without the desktop app (a host that can't run it):
/// its size and SHA-256 to check it with.
#[derive(Serialize, sqlx::FromRow)]
pub struct ServerFile {
    platform: String,
    file_name: String,
    size_bytes: Option<i64>,
    sha256: Option<String>,
    /// How many parts it's split into (0: one file).
    parts: i32,
}

/// For a game's page: its dedicated servers that this viewer can see. Everyone sees the ones in
/// the store, or shared with them with a private game; the game's maker also sees drafts, with
/// their status.
pub async fn servers(
    state: &AppState,
    game_id: Uuid,
    viewer: Option<Uuid>,
    policy: ViewerPolicy,
) -> sqlx::Result<Vec<Server>> {
    let mut qb = QueryBuilder::new("");
    catalog::push_summary_columns_with(&mut qb, &state.config, ", a.id as app_id, a.platforms");
    qb.push(" where a.parent_app_id = ")
        .push_bind(game_id)
        .push(" and ")
        .push(IS_SERVER)
        .push(" and ((");
    catalog::push_store_visible(&mut qb, state.config.catalog, policy);
    qb.push(") or ");
    catalog::push_shared_with(&mut qb, viewer);
    qb.push(" or a.maker_id = ")
        .push_bind(viewer)
        .push(") order by a.title limit 20");
    let mut servers: Vec<Server> = qb.build_query_as().fetch_all(&state.db).await?;
    for server in &mut servers {
        let release: Option<(Uuid, String)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select r.id, r.version {} limit 1",
            crate::releases::SERVING_SIGNED
        )))
        .bind(server.app_id)
        .fetch_optional(&state.db)
        .await?;
        if let Some((id, version)) = release {
            let files = sqlx::query_as(
                "select platform, file_name, size_bytes, sha256,
                        coalesce(jsonb_array_length(parts), 0)::int as parts
                 from release_files where release_id = $1 order by platform",
            )
            .bind(id)
            .fetch_all(&state.db)
            .await?;
            server.release = Some(ServerRelease { id, version, files });
        }
    }
    Ok(servers)
}

#[derive(Serialize, sqlx::FromRow)]
pub struct CommunityBuild {
    #[sqlx(flatten)]
    #[serde(flatten)]
    app: AppSummary,
    author: String,
    branch_kind: String,
    branch_note: Option<String>,
    platforms: Vec<String>,
    /// A mod whose newest approved release has code, or goes online (DESIGN §9).
    mod_code: bool,
    mod_online: bool,
    /// Platforms it has that the original doesn't: the gaps it fills (never for mods).
    #[sqlx(skip)]
    adds: Vec<String>,
}

/// For an app's page: its branches in the store, the ones filling platform gaps first.
pub async fn community(
    state: &AppState,
    app_id: Uuid,
    parent_platforms: &[String],
    policy: ViewerPolicy,
) -> sqlx::Result<Vec<CommunityBuild>> {
    let mut qb = QueryBuilder::new("");
    catalog::push_summary_columns_with(
        &mut qb,
        &state.config,
        ", (select u.display_name from users u where u.id = a.maker_id) as author, a.branch_kind, a.branch_note, a.platforms,
           coalesce((select (r.repo_scan ->> 'mod_code')::boolean from releases r
                     where r.app_id = a.id and r.status = 'approved' order by r.reviewed_at desc limit 1), false) as mod_code,
           a.branch_kind = 'addon' and (cardinality(a.online) > 0 or coalesce((select (r.repo_scan ->> 'mod_online')::boolean from releases r
                     where r.app_id = a.id and r.status = 'approved' order by r.reviewed_at desc limit 1), false)) as mod_online",
    );
    qb.push(" where a.parent_app_id = ")
        .push_bind(app_id)
        .push(" and not ")
        .push(IS_SERVER)
        .push(" and ");
    catalog::push_store_visible(&mut qb, state.config.catalog, policy);
    qb.push(" order by a.title limit 50");
    let mut builds: Vec<CommunityBuild> = qb.build_query_as().fetch_all(&state.db).await?;
    for b in &mut builds {
        b.adds = b
            .platforms
            .iter()
            .filter(|p| *p != "any" && !parent_platforms.contains(p))
            .cloned()
            .collect();
    }
    builds.sort_by(|a, b| {
        a.adds
            .is_empty()
            .cmp(&b.adds.is_empty())
            .then(b.app.vote_count.cmp(&a.app.vote_count))
    });
    Ok(builds)
}

/// The original's age rating and content labels: a branch or mod can add to them, never go
/// below them, since it carries the original's content.
pub(crate) struct RatingFloor {
    min_age: i16,
    labels: Vec<String>,
}

impl RatingFloor {
    pub(crate) fn check_age(&self, min_age: i16) -> ApiResult<()> {
        if min_age < self.min_age {
            return Err(ApiError::invalid(
                "min_age",
                match self.min_age {
                    0 => "rate it at least Everyone, like the original".to_string(),
                    n => format!("rate it at least {n}+, like the original"),
                },
            ));
        }
        Ok(())
    }

    pub(crate) fn check_labels(&self, labels: &[String]) -> ApiResult<()> {
        let missing: Vec<&str> = self
            .labels
            .iter()
            .filter(|l| !labels.contains(l))
            .map(String::as_str)
            .collect();
        if !missing.is_empty() {
            return Err(ApiError::invalid(
                "content_labels",
                format!("keep the original's labels: {}", missing.join(", ")),
            ));
        }
        Ok(())
    }
}

/// The original's rating, when `app_id` is a branch or mod.
pub(crate) async fn rating_floor(
    conn: &mut PgConnection,
    app_id: Uuid,
) -> sqlx::Result<Option<RatingFloor>> {
    let row: Option<(i16, Vec<String>)> = sqlx::query_as(
        "select p.min_age, p.content_labels from apps a join apps p on p.id = a.parent_app_id where a.id = $1",
    )
    .bind(app_id)
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|(min_age, labels)| RatingFloor { min_age, labels }))
}

/// Registering a branch asks what creating an app asks (catalog.rs `NewApp`), so branches and
/// mods are held to the same rules; what isn't given comes from the original.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Register {
    slug: String,
    title: String,
    summary: String,
    repo_url: String,
    license_spdx: String,
    kind: String,
    /// What it changes from the original. A dedicated server (`server`) changes nothing, so it
    /// needs none.
    #[serde(default)]
    note: String,
    /// Not needed for a mod (`addon`): its releases are one .zip for every platform.
    #[serde(default)]
    platforms: Vec<String>,
    /// The store page text; the note when empty.
    #[serde(default)]
    description_md: String,
    /// Age rating and content labels: at least the original's (the original's when left out).
    min_age: Option<i16>,
    content_labels: Option<Vec<String>>,
    /// What it connects to online; the original's when left out.
    online: Option<Vec<String>>,
    online_hosts: Option<Vec<String>>,
    /// Works in sandbox mode: local saves only in SLOP_SAVE_DIR.
    #[serde(default)]
    sandbox_ready: bool,
}

/// A dedicated server for the maker's own game (docs/DEDICATED-SERVERS.md): `Register` without
/// the kind, which is `server`, or a note, since it changes nothing in the game.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewServer {
    slug: String,
    title: String,
    summary: String,
    repo_url: String,
    license_spdx: String,
    platforms: Vec<String>,
    #[serde(default)]
    description_md: String,
    min_age: Option<i16>,
    content_labels: Option<Vec<String>>,
    online: Option<Vec<String>>,
    online_hosts: Option<Vec<String>>,
    #[serde(default)]
    sandbox_ready: bool,
}

/// Adds a dedicated server to the maker's game. Like a branch, it starts as a draft with its own
/// repository: the maker submits a release, which is checked and reviewed like any other.
async fn add_server(
    state: State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    parent_slug: Path<String>,
    Json(s): Json<NewServer>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let b = Register {
        slug: s.slug,
        title: s.title,
        summary: s.summary,
        repo_url: s.repo_url,
        license_spdx: s.license_spdx,
        kind: "server".into(),
        note: String::new(),
        platforms: s.platforms,
        description_md: s.description_md,
        min_age: s.min_age,
        content_labels: s.content_labels,
        online: s.online,
        online_hosts: s.online_hosts,
        sandbox_ready: s.sandbox_ready,
    };
    register(state, user, headers, parent_slug, Json(b)).await
}

/// Registers a branch of an app the author can see. It starts as a draft: the author submits a
/// release, which is checked and reviewed like any other, and SLAPorSLOP lists it.
async fn register(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(parent_slug): Path<String>,
    Json(b): Json<Register>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    // A branch is a new app: the same limit applies.
    crate::limits::check(&state, user.id, crate::limits::Action::NewApp)?;
    let policy = ViewerPolicy::resolve(&state.db, &state.config, Some(&user), &headers).await?;
    let (parent, is_mine) = catalog::visible_app(&state, &parent_slug, Some(&user), policy).await?;
    let server = b.kind == "server";
    let is_branch: bool =
        sqlx::query_scalar("select parent_app_id is not null from apps where id = $1")
            .bind(parent)
            .fetch_one(&state.db)
            .await?;
    if is_branch {
        return Err(ApiError::Conflict(if server {
            "add a dedicated server to the game itself, not to a community build or mod of it"
                .into()
        } else {
            "branch the original app, not a branch of it".into()
        }));
    }
    // A dedicated server comes from the game's own maker, who decides what players host
    // (docs/DEDICATED-SERVERS.md); anyone else publishes theirs as an app of its own.
    if server && !is_mine {
        return Err(ApiError::invalid(
            "kind",
            "only the game's maker can add its dedicated servers",
        ));
    }
    // Private apps have no community builds or mods. A private game can have a dedicated server,
    // shared with the same people (catalog.rs `push_shared_with`).
    if !server && catalog::is_private(&state.db, parent).await? {
        return Err(ApiError::NotFound);
    }
    // A browser game runs on its maker's website: there's no game to install a mod into, or a
    // build of it to change (not yet, DESIGN §9), or a server to host.
    if catalog::plays_in_browser(&state.db, parent).await? {
        return Err(ApiError::Conflict(if server {
            "a browser game can't have a dedicated server: it plays on its maker's website".into()
        } else {
            "a browser game can't have community builds or mods yet: it plays on its maker's website"
                .into()
        }));
    }
    crate::legal::require(&state.db, user.id, "publisher").await?;
    catalog::validate_slug(&b.slug)?;
    let title = catalog::validate_text("title", &b.title, 80, false)?;
    let summary = catalog::validate_text("summary", &b.summary, 200, false)?;
    let note = catalog::validate_text("note", &b.note, 500, server)?;
    let description = catalog::validate_text("description_md", &b.description_md, 20_000, true)?;
    let repo_url = catalog::validate_repo_url_in(&state, &b.repo_url)?;
    let license = catalog::validate_license(&b.license_spdx)?;
    if !KINDS.iter().any(|(k, _)| *k == b.kind) {
        return Err(ApiError::invalid(
            "kind",
            "addon, mod, fix, translation, port, other or server",
        ));
    }
    // A mod is one .zip for every platform; other branches are builds of the app.
    let platforms = if b.kind == "addon" {
        vec!["any".to_string()]
    } else {
        catalog::validate_platforms(&b.platforms)?
    };
    if platforms == ["browser"] {
        return Err(ApiError::invalid(
            "platforms",
            if server {
                "a dedicated server is a download players run: choose Windows, Linux or both"
            } else {
                "a community build can't play in the browser yet: choose Windows, Linux or both"
            },
        ));
    }
    let same_repo: bool =
        sqlx::query_scalar("select lower(repo_url) = lower($2) from apps where id = $1")
            .bind(parent)
            .bind(&repo_url)
            .fetch_one(&state.db)
            .await?;
    if same_repo {
        return Err(ApiError::invalid(
            "repo_url",
            if server {
                "a dedicated server lives in its own repository, not the game's"
            } else {
                "a branch or mod builds from your own repository, not the original one"
            },
        ));
    }
    let (parent_age, parent_labels, parent_online, parent_hosts): (
        i16,
        Vec<String>,
        Vec<String>,
        Vec<String>,
    ) = sqlx::query_as(
        "select min_age, content_labels, online, online_hosts from apps where id = $1",
    )
    .bind(parent)
    .fetch_one(&state.db)
    .await?;
    let floor = RatingFloor {
        min_age: parent_age,
        labels: parent_labels.clone(),
    };
    let min_age = catalog::validate_min_age(b.min_age.unwrap_or(parent_age))?;
    floor.check_age(min_age)?;
    let content_labels =
        catalog::validate_content_labels(b.content_labels.as_ref().unwrap_or(&parent_labels))?;
    floor.check_labels(&content_labels)?;
    let online = catalog::validate_online(b.online.as_ref().unwrap_or(&parent_online))?;
    let online_hosts = if online.is_empty() {
        Vec::new()
    } else {
        catalog::validate_online_hosts(b.online_hosts.as_ref().unwrap_or(&parent_hosts))?
    };
    // The fork must be the author's own.
    crate::repo_owner::check(&state, user.id, &repo_url, None).await?;
    let id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    // The branch takes the original's kind, tags and mods folder (a fork reads mods the same way); it's Released from the start, and can be
    // listed once it has an approved release, checked and reviewed like any other. A dedicated
    // server is an app (not a game players play), with no mods folder and the game's visibility:
    // a private game's server is shared with the game's testers.
    sqlx::query(
        "insert into apps (id, slug, title, summary, description_md, kind, stage, maker_id, repo_url, license_spdx,
                           min_age, platforms, parent_app_id, branch_kind, branch_note,
                           content_labels, online, online_hosts, sandbox_ready, mods_folder, visibility)
         select $1, $2, $3, $4, coalesce(nullif($12, ''), nullif($5, ''), $4),
                case when $10 = 'server' then 'app'::app_kind else p.kind end, 'released', $6, $7, $8, $13, $9, p.id, $10,
                nullif($5, ''), $14, $15, $16, $17, case when $10 in ('addon', 'server') then null else p.mods_folder end,
                case when $10 = 'server' then p.visibility else 'public' end
         from apps p where p.id = $11",
    )
    .bind(id)
    .bind(&b.slug)
    .bind(&title)
    .bind(&summary)
    .bind(&note)
    .bind(user.id)
    .bind(&repo_url)
    .bind(&license)
    .bind(&platforms)
    .bind(&b.kind)
    .bind(parent)
    .bind(&description)
    .bind(min_age)
    .bind(&content_labels)
    .bind(&online)
    .bind(&online_hosts)
    .bind(b.sandbox_ready)
    .execute(&mut *tx)
    .await
    .map_err(|err| {
        if is_unique_violation(&err) {
            ApiError::invalid("slug", "that address is taken")
        } else {
            err.into()
        }
    })?;
    // A dedicated server carries the Dedicated Server tag, not its game's: it isn't the game.
    sqlx::query(if server {
        "insert into app_tags (app_id, tag_id) select $1, id from tags where slug = 'dedicated-server' and $2::uuid is not null"
    } else {
        "insert into app_tags (app_id, tag_id) select $1, tag_id from app_tags where app_id = $2"
    })
    .bind(id)
    .bind(parent)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "slug": b.slug })),
    ))
}
