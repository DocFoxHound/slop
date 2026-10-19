//! The admin panel's API: the review queue and moderation for Slop staff (`users.is_admin`).
//!
//! Every action goes through the same functions as the `slop-api review …` commands (admin.rs),
//! so the same rules apply and each decision is written to the audit log with the admin's name
//! and account ID. To anyone who isn't an admin, these routes don't exist (404).
//!
//! SECURITY §8.4 asks for hardware-key sign-in and short-lived access for staff; until passkeys
//! exist, admin rights are granted only from the server and every use is audited.

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
    releases::{self, ReleaseInfo},
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
}

/// A signed-in admin. Anyone else gets 404, so the admin API can't be discovered.
pub struct AdminUser {
    /// How the audit log names them.
    actor: String,
}

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> ApiResult<Self> {
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
        })
    }
}

/// Review and moderation refusals ("approve a release first") are the admin's to read.
fn refused(err: anyhow::Error) -> ApiError {
    ApiError::Conflict(format!("{err:#}"))
}

#[derive(Serialize, sqlx::FromRow)]
struct AppRef {
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
}

#[derive(Serialize, sqlx::FromRow)]
struct ReadyApp {
    #[sqlx(flatten)]
    #[serde(flatten)]
    app: AppRef,
    approved_releases: i64,
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
        });
    }
    let ready_to_list = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {APP_COLUMNS},
                (select count(*) from releases r where r.app_id = a.id and r.status = 'approved') as approved_releases
         from apps a join users u on u.id = a.maker_id
         where a.status = 'draft'
           and (a.stage = 'coming_soon' or exists (select 1 from releases r where r.app_id = a.id and r.status = 'approved'))
         order by a.updated_at"
    )))
    .fetch_all(&state.db)
    .await?;
    let cfg = &state.config.releases;
    Ok(Json(Queue {
        releases,
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
    admin::list_app(&state.db, &slug, &admin.actor)
        .await
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delist_app(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(slug): Path<String>,
    Json(body): Json<Reason>,
) -> ApiResult<StatusCode> {
    admin::delist_app(&state.db, &slug, &admin.actor, &body.reason)
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
    admin::set_stage(&state.db, &slug, body.stage, &admin.actor)
        .await
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}
