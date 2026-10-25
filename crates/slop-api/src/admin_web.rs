// SPDX-License-Identifier: AGPL-3.0-or-later
//! The admin panel's API: the review queue and moderation for Slop staff (`users.is_admin`).
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

    /// A sign-in, or a passkey confirmation, from the last few minutes (SECURITY §4.3), and a
    /// person rather than a token. Only making an admin API token (review.rs) still asks: every
    /// other step-up went on 2026-10-25, the owner's decision.
    pub async fn confirm_recent(&self, db: &sqlx::PgPool) -> ApiResult<()> {
        let Some(session) = self.session_id else {
            return Err(ApiError::Forbidden);
        };
        let fresh: bool = sqlx::query_scalar(
            "select created_at > now() - $2 or coalesce(stepped_up_at > now() - $2, false) from sessions where id = $1",
        )
        .bind(session)
        .bind(crate::auth::session::STEP_UP_WINDOW)
        .fetch_one(db)
        .await?;
        if fresh {
            Ok(())
        } else {
            Err(ApiError::StepUpRequired)
        }
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
    /// Private Slop its owner asked to make public: listing it makes it public (SECURITY §12.3),
    /// after Slop checks the repository is public again.
    going_public: bool,
    /// Taken out of the store, then reinstated on appeal while it couldn't be listed.
    after_appeal: bool,
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
    /// Slop's GitHub App: without it, private GitHub repositories can't be published or shared,
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
                a.relist_after_appeal as after_appeal
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

#[derive(Serialize, sqlx::FromRow)]
struct ApprovedRelease {
    id: Uuid,
    version: String,
    #[serde(with = "time::serde::rfc3339::option")]
    reviewed_at: Option<OffsetDateTime>,
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
}

#[derive(Deserialize)]
struct AppsQuery {
    status: Option<AppStatus>,
    /// Part of the title, address or maker's name.
    q: Option<String>,
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

/// Every app (optionally by status), for listing, delisting, stages and withdrawals.
async fn apps(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(q): Query<AppsQuery>,
) -> ApiResult<Json<Vec<ModeratedApp>>> {
    let rows: Vec<(Uuid, AppRef)> = sqlx::query_as::<
        _,
        (Uuid, String, String, AppStage, AppStatus, String, String),
    >(sqlx::AssertSqlSafe(format!(
        "select a.id, {APP_COLUMNS} from apps a join users u on u.id = a.maker_id
             where ($1::app_status is null or a.status = $1)
               and ($2::text is null or a.title ilike $2 or a.slug ilike $2 or u.display_name ilike $2)
             order by a.updated_at desc limit 200"
    )))
    .bind(q.status)
    .bind(
        q.q.as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(contains_pattern),
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|(id, slug, title, stage, status, repo_url, maker)| {
        (
            id,
            AppRef {
                slug,
                title,
                stage,
                status,
                repo_url,
                maker,
            },
        )
    })
    .collect();
    let mut out = Vec::with_capacity(rows.len());
    for (id, app) in rows {
        let releases = sqlx::query_as(
            "select id, version, reviewed_at from releases
             where app_id = $1 and status = 'approved' order by reviewed_at desc limit 10",
        )
        .bind(id)
        .fetch_all(&state.db)
        .await?;
        let pending_releases = sqlx::query_scalar(
            "select count(*) from releases
             where app_id = $1 and status in ('fetching', 'awaiting_review')",
        )
        .bind(id)
        .fetch_one(&state.db)
        .await?;
        out.push(ModeratedApp {
            app,
            releases,
            pending_releases,
        });
    }
    Ok(Json(out))
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
                        "Slop couldn't confirm the repository is public; try again shortly".into(),
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
