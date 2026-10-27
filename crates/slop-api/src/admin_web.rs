// SPDX-License-Identifier: AGPL-3.0-or-later
//! The admin panel's API: the review queue and moderation for SLAPorSLOP staff (`users.is_admin`).
//!
//! Every action goes through the same functions as the `slop-api review …` commands (admin.rs),
//! so the same rules apply and each decision is written to the audit log with the admin's name
//! and account ID. To anyone who isn't an admin, these routes don't exist (404).
//!
//! SECURITY §8.4 asks for hardware-key sign-in and short-lived access for staff; until passkeys
//! exist, admin rights are granted only from the server and every use is audited.
//!
//! Admins can also make API tokens for AI reviewers (review.rs). A token acts for its admin on
//! these routes only, is named in the audit log, and with the `read` scope can't decide anything.

use axum::{
    Json, Router,
    extract::{FromRequestParts, Path, Query, State},
    http::{StatusCode, request::Parts},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, QueryBuilder};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    admin::{self, ReviewRules},
    auth::session::CurrentUser,
    catalog::{AppStage, AppStatus},
    error::{ApiError, ApiResult},
    manage::{self, ChangeInfo},
    media,
    releases::{self, ReleaseInfo},
    review::{self, Note},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/queue", get(queue))
        .route("/admin/apps", get(apps))
        .route("/admin/audit", get(audit_log))
        .route("/admin/releases/{id}/approve", post(approve))
        .route("/admin/releases/{id}/reject", post(reject))
        .route("/admin/releases/{id}/revoke", post(revoke))
        .route(
            "/admin/releases/{id}/files/{platform}/rescan-clear",
            post(clear_rescan),
        )
        .route(
            "/admin/releases/{id}/files/{platform}/clear",
            post(clear_scan),
        )
        .route("/admin/apps/{slug}/list", post(list_app))
        .route("/admin/apps/{slug}/delist", post(delist_app))
        .route("/admin/apps/{slug}/stage", post(set_stage))
        .route("/admin/changes/{id}/approve", post(approve_change))
        .route("/admin/changes/{id}/reject", post(reject_change))
        .route("/admin/media/{id}/approve", post(approve_media))
        .route("/admin/media/{id}/reject", post(reject_media))
        .route("/admin/portfolio/{id}/approve", post(approve_portfolio))
        .route("/admin/portfolio/{id}/reject", post(reject_portfolio))
}

/// A signed-in staff member (`users.is_admin`), or a staff member's API token. Anyone else gets
/// 404, so the admin API can't be discovered. What they may do is `need`: their roles
/// (staff.rs), capped by the token's level when it's a token.
pub struct AdminUser {
    /// How the audit log names them.
    pub actor: String,
    pub user_id: Uuid,
    /// Set when the request came with an admin API token rather than a session.
    pub token: Option<crate::review::TokenUse>,
    /// Their staff roles (staff.rs).
    pub roles: Vec<crate::staff::Role>,
    /// The website session, when signed in themselves (not a token).
    pub session_id: Option<Uuid>,
}

impl AdminUser {
    /// Refuses (403) what neither their roles nor their token's level allow.
    pub fn need(&self, perm: crate::staff::Perm) -> ApiResult<()> {
        use crate::staff::Perm;
        if !self.roles.iter().any(|r| r.allows(perm)) && perm != Perm::Read {
            return Err(ApiError::Forbidden);
        }
        if let Some(t) = &self.token {
            let ok = match perm {
                Perm::Read => true,
                Perm::Review | Perm::Moderate => t.can_decide,
                Perm::Sensitive => t.can_moderate,
                // Tokens never manage staff, tokens or money.
                Perm::Security | Perm::Finance => false,
            };
            if !ok {
                return Err(ApiError::Forbidden);
            }
        }
        Ok(())
    }

    /// Managing tokens: only an admin signed in themselves.
    pub fn signed_in(&self) -> ApiResult<()> {
        match self.token {
            Some(_) => Err(ApiError::NotFound),
            None => Ok(()),
        }
    }
}

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> ApiResult<Self> {
        if let Some(found) = crate::review::admin_from_token(state, &parts.headers).await? {
            return Ok(found);
        }
        let user = CurrentUser::from_request_parts(parts, state)
            .await
            .map_err(|_| ApiError::NotFound)?;
        let is_admin: bool = sqlx::query_scalar("select is_admin from users where id = $1")
            .bind(user.id)
            .fetch_one(&state.db)
            .await?;
        if !is_admin {
            return Err(ApiError::NotFound);
        }
        Ok(Self {
            actor: format!("{} ({})", user.display_name, user.id),
            user_id: user.id,
            token: None,
            roles: crate::staff::roles_of(&state.db, user.id).await?,
            session_id: Some(user.session_id),
        })
    }
}

/// Review and moderation refusals ("approve a release first") are the admin's to read.
fn refused(err: anyhow::Error) -> ApiError {
    ApiError::Conflict(format!("{err:#}"))
}

#[derive(Serialize, sqlx::FromRow)]
pub(crate) struct AppRef {
    slug: String,
    title: String,
    stage: AppStage,
    status: AppStatus,
    repo_url: String,
    maker: String,
}

#[derive(Serialize)]
struct QueuedRelease {
    app: AppRef,
    #[serde(flatten)]
    release: ReleaseInfo,
    notes: Vec<Note>,
}

#[derive(Serialize)]
pub(crate) struct QueuedMedia {
    app: AppRef,
    #[serde(flatten)]
    pub set: media::MediaSet,
    /// What the store shows now, to compare with.
    pub live: Option<media::MediaSet>,
    pub notes: Vec<Note>,
    #[serde(skip)]
    pub app_id: Uuid,
}

#[derive(Serialize)]
struct QueuedChange {
    app: AppRef,
    #[serde(flatten)]
    change: ChangeInfo,
    /// The version players get now, for a change of release.
    players_get: Option<String>,
    notes: Vec<Note>,
}

#[derive(Serialize, sqlx::FromRow)]
struct ReadyApp {
    #[sqlx(flatten)]
    #[serde(flatten)]
    app: AppRef,
    approved_releases: i64,
    /// New pictures wait for review before the app can be listed.
    pictures_pending: bool,
    /// A private app its owner asked to make public: listing it makes it public (SECURITY §12.3),
    /// after SLAPorSLOP checks the repository is public again.
    going_public: bool,
    /// Taken out of the store, then reinstated on appeal while it couldn't be listed.
    after_appeal: bool,
    /// When the app last changed (its newest approved release, its page, or the appeal): how
    /// long it has waited, for the review queue.
    #[serde(with = "time::serde::rfc3339")]
    waiting_since: OffsetDateTime,
    #[sqlx(skip)]
    notes: Vec<Note>,
    #[sqlx(skip)]
    #[serde(skip)]
    id: Uuid,
}

#[derive(Serialize)]
struct ServerChecks {
    scanning: bool,
    maker_signatures_required: bool,
    /// Whether releases can be signed at all (SLOP_SIGNING_KEY).
    signing_key: bool,
    /// SLAPorSLOP's GitHub App: without it, private GitHub repositories can't be published or shared,
    /// and every GitHub request shares one small allowance.
    github_app: bool,
    /// The server has a GitHub token (a higher allowance than none at all).
    github_token: bool,
}

#[derive(Serialize)]
struct Queue {
    /// Releases being checked, waiting for review, or whose checks failed; oldest first.
    releases: Vec<QueuedRelease>,
    /// Makers' changes that would have players download something different; oldest first.
    changes: Vec<QueuedChange>,
    /// New store-page pictures; oldest first.
    media: Vec<QueuedMedia>,
    /// New People portfolios; oldest first. A profile goes live with its first approved one.
    portfolios: Vec<crate::people::portfolio::QueuedPortfolio>,
    /// Problems with People jobs that the two sides haven't settled; oldest first.
    problems: Vec<crate::people::mediation::QueuedProblem>,
    /// Creatives' disputes about reviews; oldest first.
    review_disputes: Vec<crate::people::mediation::QueuedDispute>,
    /// Open reports: urgent first, then by when they're due (moderation.rs).
    reports: Vec<crate::moderation::QueuedReport>,
    /// Appeals against moderation decisions; oldest first.
    appeals: Vec<crate::moderation::QueuedAppeal>,
    /// The Content Policy rules a decision can cite: (code, title).
    rules: &'static [(&'static str, &'static str)],
    /// Drafts that could go in the store (Coming soon pages, or apps with an approved release),
    /// and apps reinstated on appeal that wait to go back.
    ready_to_list: Vec<ReadyApp>,
    /// Live releases a re-scan flagged (rescan.rs): withdraw them or keep them.
    rescans: Vec<FlaggedFile>,
    /// Patterns the abuse checks found (abuse.rs), for a person to look at.
    abuse: Vec<crate::abuse::Signal>,
    checks: ServerChecks,
}

const APP_COLUMNS: &str = "a.slug, a.title, a.stage, a.status, a.repo_url, u.display_name as maker";

async fn queue(State(state): State<AppState>, _admin: AdminUser) -> ApiResult<Json<Queue>> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "select r.id, a.slug from releases r join apps a on a.id = r.app_id
         where r.status in ('fetching', 'awaiting_review', 'failed')
         order by r.created_at limit 100",
    )
    .fetch_all(&state.db)
    .await?;
    let mut releases = Vec::with_capacity(rows.len());
    for (id, slug) in rows {
        let app: AppRef = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select {APP_COLUMNS} from apps a join users u on u.id = a.maker_id where a.slug = $1"
        )))
        .bind(&slug)
        .fetch_one(&state.db)
        .await?;
        releases.push(QueuedRelease {
            app,
            release: releases::load_release(&state, id).await?,
            notes: review::notes(&state.db, "release", id).await?,
        });
    }
    let mut ready_to_list: Vec<ReadyApp> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {APP_COLUMNS},
                (select count(*) from releases r where r.app_id = a.id and r.status = 'approved') as approved_releases,
                exists (select 1 from media_sets ms where ms.app_id = a.id and ms.status = 'pending') as pictures_pending,
                a.visibility = 'private' as going_public,
                a.relist_after_appeal as after_appeal,
                greatest(a.updated_at, (select max(r.reviewed_at) from releases r where r.app_id = a.id and r.status = 'approved')) as waiting_since
         from apps a join users u on u.id = a.maker_id
         where (a.status = 'draft' or (a.status = 'delisted' and a.relist_after_appeal))
           and (a.visibility = 'public' or a.public_requested_at is not null)
           and (a.stage = 'coming_soon' or exists (select 1 from releases r where r.app_id = a.id and r.status = 'approved'))
         order by a.updated_at"
    )))
    .fetch_all(&state.db)
    .await?;
    for app in &mut ready_to_list {
        app.id = sqlx::query_scalar("select id from apps where slug = $1")
            .bind(&app.app.slug)
            .fetch_one(&state.db)
            .await?;
        app.notes = review::notes(&state.db, "listing", app.id).await?;
    }
    let mut conn = state.db.acquire().await?;
    let pending: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "select id, app_id from app_changes where status = 'pending' order by created_at limit 100",
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut changes = Vec::with_capacity(pending.len());
    for (id, app_id) in pending {
        let app: AppRef = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select {APP_COLUMNS} from apps a join users u on u.id = a.maker_id where a.id = $1"
        )))
        .bind(app_id)
        .fetch_one(&mut *conn)
        .await?;
        let change: ChangeInfo = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select {} where c.id = $1",
            manage::change_columns()
        )))
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
        let players_get = manage::serving(&mut conn, app_id).await?.map(|r| r.version);
        changes.push(QueuedChange {
            app,
            change,
            players_get,
            notes: review::notes(&state.db, "change", id).await?,
        });
    }
    let media = queued_media(&state).await?;
    let cfg = &state.config.releases;
    Ok(Json(Queue {
        releases,
        changes,
        media,
        portfolios: crate::people::portfolio::queued(&state).await?,
        problems: crate::people::mediation::queued_problems(&state.db).await?,
        review_disputes: crate::people::mediation::queued_disputes(&state.db).await?,
        reports: crate::moderation::queued(&state.db).await?,
        appeals: crate::moderation::queued_appeals(&state.db).await?,
        rules: crate::moderation::RULES,
        ready_to_list,
        abuse: crate::abuse::open(&state.db).await?,
        rescans: sqlx::query_as(
            "select r.id as release_id, r.version, f.platform, f.file_name, f.rescan_result, f.rescan_report,
                    f.rescan_flagged_at, a.slug, a.title
             from release_files f join releases r on r.id = f.release_id join apps a on a.id = r.app_id
             where f.rescan_flagged_at is not null and r.status = 'approved'
             order by f.rescan_flagged_at",
        )
        .fetch_all(&state.db)
        .await?,
        checks: ServerChecks {
            scanning: cfg.scanner.is_on(),
            maker_signatures_required: cfg.require_maker_signature,
            signing_key: cfg.signing_key.is_some(),
            github_app: cfg.github_app.is_some(),
            github_token: cfg.github_token.as_deref().is_some_and(|t| !t.is_empty()),
        },
    }))
}

#[derive(Serialize, Deserialize, sqlx::FromRow)]
struct ApprovedRelease {
    id: Uuid,
    version: String,
    #[serde(with = "time::serde::rfc3339::option")]
    reviewed_at: Option<OffsetDateTime>,
}

/// The newest release the maker submitted, whatever became of it.
#[derive(Serialize)]
struct NewestRelease {
    version: String,
    status: String,
    /// Why it failed, was rejected or was withdrawn.
    status_reason: Option<String>,
}

/// What can need staff about an app, most urgent first: a code and its condition on the facts
/// row `x` (static SQL). An app shows its codes as `flags`; the Apps tab says each in words
/// (web/src/pages/admin/apps.ts) and sorts by the first.
const FLAGS: &[(&str, &str)] = &[
    // A re-scan of a live release found something new (rescan.rs).
    ("rescan_flagged", "x.rescan_flags > 0"),
    // The file players get changed on the maker's page, or can't be fetched.
    ("download_lost", "x.downloads_lost > 0"),
    // SLAPorSLOP lost access to its repository, or staff froze it (admin_tools.rs).
    ("frozen", "x.frozen_at is not null"),
    ("reports", "x.open_reports > 0"),
    // The newest release couldn't be fetched or scanned.
    ("release_failed", "x.newest_status = 'failed'"),
    ("release_waiting", "x.pending_releases > 0"),
    // Listed, but its Coming soon or Beta time ran out, so the store hides it (catalog.rs).
    (
        "past_deadline",
        "x.status = 'listed' and x.stage_deadline < now()",
    ),
    (
        "deadline_soon",
        "x.status = 'listed' and x.stage_deadline >= now() and x.stage_deadline < now() + interval '14 days'",
    ),
    ("donations_paused", "x.donations_paused"),
    // The store's own "dead project": not released, and no commit for a long time.
    (
        "no_commits",
        "x.stage <> 'released' and x.last_commit_at < now() - x.dead_after",
    ),
    // In the store a long time, playable, and nobody has played it lately.
    (
        "no_players",
        "x.status = 'listed' and x.stage <> 'coming_soon' and x.created_at < now() - x.dead_after
         and coalesce(x.last_played_at, '-infinity') < now() - x.dead_after",
    ),
    // Enough votes to count, and most of them say Slop.
    ("low_rating", "x.not_slop_percent < 40"),
];

#[derive(sqlx::FromRow)]
struct AppFacts {
    slug: String,
    title: String,
    stage: AppStage,
    status: AppStatus,
    repo_url: String,
    maker: String,
    visibility: String,
    icon_url: Option<String>,
    thumbnail_url: Option<String>,
    frozen_at: Option<OffsetDateTime>,
    frozen_reason: Option<String>,
    stage_deadline: Option<OffsetDateTime>,
    last_commit_at: Option<OffsetDateTime>,
    updated_at: OffsetDateTime,
    newest_version: Option<String>,
    newest_status: Option<String>,
    newest_reason: Option<String>,
    serving_version: Option<String>,
    pending_releases: i64,
    rescan_flags: i64,
    downloads_lost: i64,
    open_reports: i64,
    donations_paused: bool,
    players: i64,
    last_played_at: Option<OffsetDateTime>,
    vote_count: i64,
    not_slop_percent: Option<i32>,
    releases: sqlx::types::Json<Vec<ApprovedRelease>>,
    flags: Vec<String>,
}

#[derive(Serialize)]
struct ModeratedApp {
    #[serde(flatten)]
    app: AppRef,
    /// Approved releases, newest first, for withdrawing one.
    releases: Vec<ApprovedRelease>,
    /// Releases still being checked or waiting for review: what listing a Beta or Released app
    /// is waiting on when it has no approved release yet.
    pending_releases: i64,
    /// What needs staff, most urgent first (`FLAGS`); empty for a healthy app.
    flags: Vec<String>,
    visibility: String,
    icon_url: Option<String>,
    thumbnail_url: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    frozen_at: Option<OffsetDateTime>,
    frozen_reason: Option<String>,
    /// When its Coming soon or Beta time runs out (null once released).
    #[serde(with = "time::serde::rfc3339::option")]
    stage_deadline: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    last_commit_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    updated_at: OffsetDateTime,
    newest_release: Option<NewestRelease>,
    /// The version players get (releases::SERVING), if any.
    serving_version: Option<String>,
    /// Live release files a re-scan flagged, and ones whose download changed or went away.
    rescan_flags: i64,
    downloads_lost: i64,
    open_reports: i64,
    donations_paused: bool,
    /// Accounts that have launched it, and when one last played.
    players: i64,
    #[serde(with = "time::serde::rfc3339::option")]
    last_played_at: Option<OffsetDateTime>,
    vote_count: i64,
    /// Weighted % Slaps, as the store shows it; null below votes::MIN_VOTES.
    not_slop_percent: Option<i32>,
}

impl From<AppFacts> for ModeratedApp {
    fn from(f: AppFacts) -> Self {
        let newest_release = match (f.newest_version, f.newest_status) {
            (Some(version), Some(status)) => Some(NewestRelease {
                version,
                status,
                status_reason: f.newest_reason,
            }),
            _ => None,
        };
        ModeratedApp {
            app: AppRef {
                slug: f.slug,
                title: f.title,
                stage: f.stage,
                status: f.status,
                repo_url: f.repo_url,
                maker: f.maker,
            },
            releases: f.releases.0,
            pending_releases: f.pending_releases,
            flags: f.flags,
            visibility: f.visibility,
            icon_url: f.icon_url,
            thumbnail_url: f.thumbnail_url,
            frozen_at: f.frozen_at,
            frozen_reason: f.frozen_reason,
            stage_deadline: f.stage_deadline,
            last_commit_at: f.last_commit_at,
            updated_at: f.updated_at,
            newest_release,
            serving_version: f.serving_version,
            rescan_flags: f.rescan_flags,
            downloads_lost: f.downloads_lost,
            open_reports: f.open_reports,
            donations_paused: f.donations_paused,
            players: f.players,
            last_played_at: f.last_played_at,
            vote_count: f.vote_count,
            not_slop_percent: f.not_slop_percent,
        }
    }
}

#[derive(Deserialize, Clone, Copy, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
enum AppsSort {
    /// Most recently changed first.
    #[default]
    Updated,
    /// Most urgent flag first (`FLAGS`), then most recently changed.
    Urgent,
    Title,
}

#[derive(Deserialize)]
struct AppsQuery {
    status: Option<AppStatus>,
    /// Part of the title, address or maker's name.
    q: Option<String>,
    /// Only apps with at least one flag.
    #[serde(default)]
    attention: bool,
    #[serde(default)]
    sort: AppsSort,
    /// A page: up to 200 apps (the default), after skipping `offset`.
    limit: Option<i64>,
    offset: Option<i64>,
}

/// `text` as an ILIKE pattern that matches it anywhere, literally.
pub(crate) fn contains_pattern(text: &str) -> String {
    format!(
        "%{}%",
        text.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}

/// Every app (optionally by status, or only those needing staff), with what an admin would want
/// to know at a glance: for listing, delisting, stages and withdrawals. One query: the facts are
/// index lookups per app, and when the page doesn't depend on them only the page's apps get them.
async fn apps(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(q): Query<AppsQuery>,
) -> ApiResult<Json<Vec<ModeratedApp>>> {
    let limit = q.limit.unwrap_or(200).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let rules = state.config.catalog;
    let pattern =
        q.q.as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(contains_pattern);
    // (the order of the whole list, the same order on `apps a` alone)
    let (order, base_order) = match q.sort {
        AppsSort::Updated => ("updated_at desc, slug", "a.updated_at desc, a.slug"),
        AppsSort::Title => ("lower(title), slug", "lower(a.title), a.slug"),
        AppsSort::Urgent => ("urgency nulls last, updated_at desc, slug", ""),
    };
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
        "with base as (
             select a.id, a.slug, a.title, a.stage, a.status, a.repo_url, u.display_name as maker,
                    a.visibility, a.frozen_at, a.frozen_reason, a.last_commit_at, a.created_at,
                    a.updated_at, a.media_set_id, a.pinned_release_id,
                    case a.stage
                        when 'coming_soon' then a.stage_entered_at + make_interval(days => ",
    );
    qb.push_bind(rules.coming_soon_days)
        .push(") when 'beta' then a.stage_entered_at + make_interval(days => ")
        .push_bind(rules.beta_days)
        .push(
            ") end as stage_deadline
             from apps a join users u on u.id = a.maker_id
             where (",
        )
        .push_bind(q.status)
        .push("::app_status is null or a.status = ")
        .push_bind(q.status)
        .push(") and (")
        .push_bind(pattern.clone())
        .push("::text is null or a.title ilike ")
        .push_bind(pattern.clone())
        .push(" or a.slug ilike ")
        .push_bind(pattern.clone())
        .push(" or u.display_name ilike ")
        .push_bind(pattern)
        .push(")");
    // Sorting by date or title needs no facts: just the page's apps go on.
    if !q.attention && q.sort != AppsSort::Urgent {
        qb.push(format!(" order by {base_order} limit "))
            .push_bind(offset + limit);
    }
    qb.push(
        "), x as (
             select b.*, make_interval(days => ",
    )
    .push_bind(rules.dead_after_days)
    .push(
        ") as dead_after,
                ms.icon->>'url' as icon_url, ms.thumbnail->>'url' as thumbnail_url,
                nr.version as newest_version, nr.status::text as newest_status, nr.status_reason as newest_reason,
                sv.version as serving_version,
                (select count(*) from releases r
                 where r.app_id = b.id and r.status in ('fetching', 'awaiting_review')) as pending_releases,
                fl.rescan_flags, fl.downloads_lost,
                (select count(*) from reports p
                 where p.subject_kind = 'app' and p.subject_id = b.id and p.status = 'open') as open_reports,
                exists (select 1 from donation_holds h where h.app_id = b.id and h.lifted_at is null) as donations_paused,
                pl.players, pl.last_played_at, vt.vote_count, vt.not_slop_percent,
                coalesce((select json_agg(json_build_object('id', r.id, 'version', r.version, 'reviewed_at', r.reviewed_at)
                                          order by r.reviewed_at desc)
                          from (select id, version, reviewed_at from releases r
                                where r.app_id = b.id and r.status = 'approved'
                                order by reviewed_at desc limit 10) r), '[]') as releases
             from base b
             left join media_sets ms on ms.id = b.media_set_id
             left join lateral (select version, status, status_reason from releases r
                                where r.app_id = b.id order by created_at desc limit 1) nr on true
             left join lateral (select r.id, r.version from releases r
                                where r.app_id = b.id and r.status = 'approved' and r.signed_manifest is not null
                                order by coalesce(r.id = b.pinned_release_id, false) desc, r.reviewed_at desc
                                limit 1) sv on true
             left join lateral (select count(*) filter (where f.rescan_flagged_at is not null) as rescan_flags,
                                       count(*) filter (where f.release_id = sv.id
                                                          and f.rescan_result in ('changed', 'unreachable')) as downloads_lost
                                from release_files f join releases r on r.id = f.release_id
                                where r.app_id = b.id and r.status = 'approved') fl on true
             left join lateral (select count(*) as players, max(g.last_played_at) as last_played_at
                                from user_app_usage g where g.app_id = b.id) pl on true
             left join lateral (select count(*) as vote_count,
                                       case when count(*) >= ",
    )
    .push_bind(crate::votes::MIN_VOTES)
    .push(
        " and sum(v.weight) > 0
                                            then round(100 * coalesce(sum(v.weight) filter (where v.vote = 1), 0)
                                                       / sum(v.weight))::int4 end as not_slop_percent
                                from votes v where v.app_id = b.id) vt on true
         ), y as (
             select x.*, array_remove(array[",
    );
    for (i, (code, condition)) in FLAGS.iter().enumerate() {
        if i > 0 {
            qb.push(", ");
        }
        qb.push(format!("case when {condition} then '{code}' end"));
    }
    let codes: Vec<String> = FLAGS.iter().map(|(code, _)| format!("'{code}'")).collect();
    qb.push(format!(
        "]::text[], null) as flags from x
         )
         select *, array_position(array[{}]::text[], flags[1]) as urgency from y where (not ",
        codes.join(", ")
    ))
    .push_bind(q.attention)
    .push(format!(
        " or cardinality(flags) > 0) order by {order} limit "
    ))
    .push_bind(limit)
    .push(" offset ")
    .push_bind(offset);
    let rows: Vec<AppFacts> = qb.build_query_as().fetch_all(&state.db).await?;
    Ok(Json(rows.into_iter().map(ModeratedApp::from).collect()))
}

#[derive(Serialize, sqlx::FromRow)]
struct AuditEntry {
    id: i64,
    #[serde(with = "time::serde::rfc3339")]
    at: OffsetDateTime,
    actor: String,
    action: String,
    subject: String,
    detail: serde_json::Value,
}

#[derive(Deserialize)]
struct AuditQuery {
    /// Part of who did it, what, or to what.
    q: Option<String>,
    /// Entries older than this one (the last `id` of the previous page).
    before: Option<i64>,
}

/// The audit log, newest first, 100 at a time.
async fn audit_log(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Json<Vec<AuditEntry>>> {
    let pattern =
        q.q.as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(contains_pattern);
    Ok(Json(
        sqlx::query_as(
            "select id, at, actor, action, subject, detail from audit_log
             where ($1::bigint is null or id < $1)
               and ($2::text is null or actor ilike $2 or action ilike $2 or subject ilike $2)
             order by id desc limit 100",
        )
        .bind(q.before)
        .bind(pattern)
        .fetch_all(&state.db)
        .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reason {
    reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Stage {
    stage: AppStage,
}

async fn approve(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    admin.need(crate::staff::Perm::Review)?;
    let cfg = &state.config;
    admin::approve(
        &state.db,
        cfg.releases.signing_key.as_ref(),
        ReviewRules::from_config(cfg),
        id,
        &admin.actor,
    )
    .await
    .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn reject(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<Reason>,
) -> ApiResult<StatusCode> {
    admin.need(crate::staff::Perm::Review)?;
    admin::reject(&state.db, id, &admin.actor, &body.reason)
        .await
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn clear_scan(
    State(state): State<AppState>,
    admin: AdminUser,
    Path((id, platform)): Path<(Uuid, String)>,
    Json(body): Json<Reason>,
) -> ApiResult<StatusCode> {
    admin.need(crate::staff::Perm::Review)?;
    admin::clear_scan(&state.db, id, &platform, &admin.actor, &body.reason)
        .await
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, sqlx::FromRow)]
struct FlaggedFile {
    release_id: Uuid,
    version: String,
    platform: String,
    file_name: String,
    rescan_result: Option<String>,
    rescan_report: Option<serde_json::Value>,
    #[serde(with = "time::serde::rfc3339")]
    rescan_flagged_at: time::OffsetDateTime,
    slug: String,
    title: String,
}

/// Keeps a live release a re-scan flagged: the reviewer judged it a false positive (logged).
async fn clear_rescan(
    State(state): State<AppState>,
    admin: AdminUser,
    Path((id, platform)): Path<(Uuid, String)>,
    Json(body): Json<Reason>,
) -> ApiResult<StatusCode> {
    admin.need(crate::staff::Perm::Review)?;
    let reason = body.reason.trim();
    if !(10..=1000).contains(&reason.chars().count()) {
        return Err(ApiError::invalid(
            "reason",
            "say why in 10 to 1000 characters",
        ));
    }
    let mut tx = state.db.begin().await?;
    let cleared = sqlx::query(
        "update release_files set rescan_flagged_at = null, rescan_cleared_reason = $3
         where release_id = $1 and platform = $2 and rescan_flagged_at is not null",
    )
    .bind(id)
    .bind(&platform)
    .bind(reason)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if cleared == 0 {
        return Err(ApiError::NotFound);
    }
    admin::audit(
        &mut tx,
        &admin.actor,
        "release.rescan_cleared",
        &format!("{id} {platform}"),
        serde_json::json!({ "reason": reason }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<Reason>,
) -> ApiResult<crate::staff::Outcome> {
    admin.need(crate::staff::Perm::Sensitive)?;
    let reason = body.reason.trim();
    if reason.is_empty() {
        return Err(ApiError::invalid("reason", "say why"));
    }
    let what: Option<(String, String)> = sqlx::query_as(
        "select a.title, r.version from releases r join apps a on a.id = r.app_id
         where r.id = $1 and r.status = 'approved'",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let (title, version) =
        what.ok_or_else(|| ApiError::Conflict("only an approved release can be withdrawn".into()))?;
    crate::staff::request(
        &state,
        &admin,
        "release.revoke",
        &format!("Withdraw {title} {version}: {reason}"),
        serde_json::json!({ "release_id": id, "reason": reason }),
    )
    .await
}

async fn list_app(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    admin.need(crate::staff::Perm::Review)?;
    let going_public: Option<(String,)> = sqlx::query_as(
        "select repo_url from apps where slug = $1 and visibility = 'private' and public_requested_at is not null",
    )
    .bind(&slug)
    .fetch_optional(&state.db)
    .await?;
    match going_public {
        Some((repo,)) => {
            // Check again now that the repository is public (SECURITY §12.3).
            match crate::frozen::check(&state, &repo, true).await {
                crate::frozen::Access::Readable => {}
                crate::frozen::Access::Lost(reason) => {
                    return Err(ApiError::Conflict(format!(
                        "it can't go public yet: {reason}"
                    )));
                }
                crate::frozen::Access::Unknown => {
                    return Err(ApiError::Conflict(
                        "SLAPorSLOP couldn't confirm the repository is public; try again shortly"
                            .into(),
                    ));
                }
            }
            admin::make_public(&state.db, &slug, &admin.actor)
                .await
                .map_err(refused)?;
        }
        None => admin::list_app(&state.db, &slug, &admin.actor)
            .await
            .map_err(refused)?,
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Delist {
    /// The Content Policy rule (moderation.rs RULES).
    rule: String,
    /// The facts, for the maker's statement of reasons.
    reason: String,
}

async fn delist_app(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(slug): Path<String>,
    Json(body): Json<Delist>,
) -> ApiResult<crate::staff::Outcome> {
    admin.need(crate::staff::Perm::Sensitive)?;
    if !crate::moderation::RULES
        .iter()
        .any(|(code, _)| *code == body.rule)
    {
        return Err(ApiError::invalid(
            "rule",
            "use one of the Content Policy rules",
        ));
    }
    if body.reason.trim().is_empty() {
        return Err(ApiError::invalid("reason", "state the facts"));
    }
    let title: String =
        sqlx::query_scalar("select title from apps where slug = $1 and status = 'listed'")
            .bind(&slug)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| ApiError::Conflict("that app isn't in the store".into()))?;
    crate::staff::request(
        &state,
        &admin,
        "app.delist",
        &format!("Take {title} out of the store ({})", body.rule),
        serde_json::json!({ "slug": slug, "rule": body.rule, "reason": body.reason.trim() }),
    )
    .await
}

async fn set_stage(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(slug): Path<String>,
    Json(body): Json<Stage>,
) -> ApiResult<StatusCode> {
    admin.need(crate::staff::Perm::Review)?;
    admin::set_stage(&state.db, &slug, body.stage, &admin.actor)
        .await
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn approve_change(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    admin.need(crate::staff::Perm::Review)?;
    manage::approve_change(&state.db, id, &admin.actor)
        .await
        .map_err(refused)?;
    state.watch_wakeup.notify_one();
    Ok(StatusCode::NO_CONTENT)
}

async fn reject_change(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<Reason>,
) -> ApiResult<StatusCode> {
    admin.need(crate::staff::Perm::Review)?;
    manage::reject_change(&state.db, id, &admin.actor, &body.reason)
        .await
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Pictures waiting for review, each with what the store shows now. Oldest first.
pub(crate) async fn queued_media(state: &AppState) -> ApiResult<Vec<QueuedMedia>> {
    let rows: Vec<(Uuid, Uuid, Option<Uuid>, Vec<String>)> = sqlx::query_as(
        "select ms.id, ms.app_id, a.media_set_id, a.media_order from media_sets ms join apps a on a.id = ms.app_id
         where ms.status = 'pending' order by ms.created_at limit 100",
    )
    .fetch_all(&state.db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, app_id, live, order) in rows {
        let app: AppRef = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select {APP_COLUMNS} from apps a join users u on u.id = a.maker_id where a.id = $1"
        )))
        .bind(app_id)
        .fetch_one(&state.db)
        .await?;
        let mut set = media::load_set(&state.db, id).await?;
        media::arrange(&mut set.gallery, &order);
        let live = match live {
            Some(l) => {
                let mut l = media::load_set(&state.db, l).await?;
                media::arrange(&mut l.gallery, &order);
                Some(l)
            }
            None => None,
        };
        out.push(QueuedMedia {
            app,
            app_id,
            set,
            live,
            notes: review::notes(&state.db, "media", id).await?,
        });
    }
    Ok(out)
}

async fn approve_media(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    admin.need(crate::staff::Perm::Review)?;
    media::approve_set(&state.db, id, &admin.actor)
        .await
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn reject_media(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<Reason>,
) -> ApiResult<StatusCode> {
    admin.need(crate::staff::Perm::Review)?;
    media::reject_set(&state.db, id, &admin.actor, &body.reason)
        .await
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn approve_portfolio(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    admin.need(crate::staff::Perm::Review)?;
    crate::people::portfolio::approve_set(&state.db, id, &admin.actor)
        .await
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn reject_portfolio(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<Reason>,
) -> ApiResult<StatusCode> {
    admin.need(crate::staff::Perm::Review)?;
    crate::people::portfolio::reject_set(&state.db, id, &admin.actor, &body.reason)
        .await
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}
