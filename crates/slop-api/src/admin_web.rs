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

/// A signed-in admin, or an admin's API token. Anyone else gets 404, so the admin API can't be
/// discovered.
pub struct AdminUser {
    /// How the audit log names them.
    pub actor: String,
    pub user_id: Uuid,
    /// Set when the request came with an admin API token rather than a session.
    pub token: Option<crate::review::TokenUse>,
}

impl AdminUser {
    /// Approving, rejecting and moderating: not for `read` tokens.
    pub fn may_decide(&self) -> ApiResult<()> {
        match &self.token {
            Some(t) if !t.can_decide => Err(ApiError::Forbidden),
            _ => Ok(()),
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
    /// Drafts that could go in the store: Coming soon pages, or apps with an approved release.
    ready_to_list: Vec<ReadyApp>,
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
                a.visibility = 'private' as going_public
         from apps a join users u on u.id = a.maker_id
         where a.status = 'draft' and (a.visibility = 'public' or a.public_requested_at is not null)
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
        checks: ServerChecks {
            scanning: cfg.scanner.is_on(),
            maker_signatures_required: cfg.require_maker_signature,
            signing_key: cfg.signing_key.is_some(),
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
             order by a.updated_at desc limit 200"
    )))
    .bind(q.status)
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

async fn audit_log(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> ApiResult<Json<Vec<AuditEntry>>> {
    Ok(Json(
        sqlx::query_as("select id, at, actor, action, subject, detail from audit_log order by id desc limit 200")
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
    admin.may_decide()?;
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
    admin.may_decide()?;
    admin::reject(&state.db, id, &admin.actor, &body.reason)
        .await
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<Reason>,
) -> ApiResult<StatusCode> {
    admin.may_decide()?;
    admin::revoke(&state.db, id, &admin.actor, &body.reason)
        .await
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_app(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    admin.may_decide()?;
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
) -> ApiResult<StatusCode> {
    admin.may_decide()?;
    admin::delist_app(&state.db, &slug, &admin.actor, &body.rule, &body.reason)
        .await
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_stage(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(slug): Path<String>,
    Json(body): Json<Stage>,
) -> ApiResult<StatusCode> {
    admin.may_decide()?;
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
    admin.may_decide()?;
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
    admin.may_decide()?;
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
    admin.may_decide()?;
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
    admin.may_decide()?;
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
    admin.may_decide()?;
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
    admin.may_decide()?;
    crate::people::portfolio::reject_set(&state.db, id, &admin.actor, &body.reason)
        .await
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}
