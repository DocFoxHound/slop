// SPDX-License-Identifier: AGPL-3.0-or-later
//! Running Slop with an AI agent (decided 2026-10-20). An admin makes an admin API token (Admin →
//! AI reviewers) and gives it to an agent, which then watches the system and does what the
//! admin asks: approvals, rejections, reports, warnings, suspensions.
//!
//! - **REST:** everything under `/v1/admin/…` takes `Authorization: Bearer slopadm_…`. This
//!   module adds what an agent needs beyond the review queue: `GET /v1/admin/status` (one look
//!   at the system's health and what's waiting), finding and reading accounts, and deciding
//!   about an account directly (a warning or a suspension, with a Content Policy rule and the
//!   facts, like any moderation decision), or lifting a suspension.
//! - **MCP:** `POST /v1/admin/mcp` speaks the Model Context Protocol (JSON-RPC over HTTP), so an
//!   agent such as Claude can connect with the token and get these as tools. Each tool calls the
//!   REST API above through Slop's own router (built for the call), so it gets exactly the same
//!   checks, limits and audit log entries as any other client.
//!
//! The tools cover the whole admin panel (decided 2026-10-22: the owner runs Slop from their
//! phone through an agent), except the payments record and making tokens, which stay with a
//! person signed in. Staff changes an agent asks for wait for a person to approve them on the
//! website (staff.rs).
//!
//! What a token may do is its scope: `read` looks and leaves notes, `review` also approves and
//! rejects and fixes store pages, profiles, shelves and freezes, `moderate` also suspends and
//! reinstates accounts, withdraws releases, takes apps out of the store, turns donations off,
//! publishes legal documents and asks for staff changes. Every action is in the audit log under
//! the token's name and its admin.

use axum::{
    Json, Router,
    body::Body,
    extract::{ConnectInfo, Path, Query, Request, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    AppState, admin,
    admin_web::AdminUser,
    error::{ApiError, ApiResult},
    staff::Perm,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/status", get(status))
        .route("/admin/users", get(find_users))
        .route("/admin/users/{id}", get(user_details))
        .route("/admin/users/{id}/decide", post(decide_user))
        .route("/admin/users/{id}/lift-suspension", post(lift_suspension))
        .route("/admin/users/{id}/passkey-grace", post(passkey_grace))
        .route("/admin/mcp", post(mcp).get(mcp_get))
}

// ---------------------------------------------------------------------------------------------
// Status

#[derive(Serialize)]
struct Waiting {
    releases_to_review: i64,
    releases_failed_this_week: i64,
    changes: i64,
    pictures: i64,
    portfolios: i64,
    reports_urgent: i64,
    reports_serious: i64,
    reports_other: i64,
    reports_overdue: i64,
    appeals: i64,
    live_releases_flagged_by_rescan: i64,
    abuse_signals: i64,
    apps_ready_to_list: i64,
}

#[derive(Serialize)]
struct Status {
    /// Open alerts (ops.rs): stale backups, failed deploys, full disks, scanner failures…
    alerts: Vec<Value>,
    waiting: Waiting,
    #[serde(with = "time::serde::rfc3339::option")]
    last_backup: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    last_restore_check: Option<OffsetDateTime>,
    /// The commit running, from the deploy folder.
    deployed: Option<String>,
    scanning: bool,
    signing_key: bool,
    github_app: bool,
    /// Accounts, apps in the store, releases approved in the last day.
    accounts: i64,
    apps_listed: i64,
    approved_last_day: i64,
}

async fn count(db: &sqlx::PgPool, sql: &'static str) -> sqlx::Result<i64> {
    sqlx::query_scalar(sql).fetch_one(db).await
}

async fn status(State(state): State<AppState>, _admin: AdminUser) -> ApiResult<Json<Status>> {
    let db = &state.db;
    let alerts: Vec<(String, String, String, OffsetDateTime)> = sqlx::query_as(
        "select key, title, detail, first_seen from ops_alerts where resolved_at is null order by first_seen desc limit 50",
    )
    .fetch_all(db)
    .await?;
    let ops = &state.config.ops;
    let cfg = &state.config.releases;
    Ok(Json(Status {
        alerts: alerts
            .into_iter()
            .map(|(key, title, detail, since)| {
                json!({ "key": key, "title": title, "detail": detail, "since": since.unix_timestamp() })
            })
            .collect(),
        waiting: Waiting {
            releases_to_review: count(db, "select count(*) from releases where status = 'awaiting_review'").await?,
            releases_failed_this_week: count(db, "select count(*) from releases where status = 'failed' and created_at > now() - interval '7 days'").await?,
            changes: count(db, "select count(*) from app_changes where status = 'pending'").await.unwrap_or(0),
            pictures: count(db, "select count(*) from media_sets where status = 'pending'").await.unwrap_or(0),
            portfolios: count(db, "select count(*) from portfolio_sets where status = 'pending'").await.unwrap_or(0),
            reports_urgent: count(db, "select count(*) from reports where status = 'open' and priority = 0").await?,
            reports_serious: count(db, "select count(*) from reports where status = 'open' and priority = 1").await?,
            reports_other: count(db, "select count(*) from reports where status = 'open' and priority = 2").await?,
            reports_overdue: count(db, "select count(*) from reports where status = 'open' and due_at < now()").await?,
            appeals: count(db, "select count(*) from moderation_decisions where appeal_status = 'pending'").await.unwrap_or(0),
            live_releases_flagged_by_rescan: count(db, "select count(*) from release_files f join releases r on r.id = f.release_id where f.rescan_flagged_at is not null and r.status = 'approved'").await?,
            abuse_signals: count(db, "select count(*) from abuse_signals where status = 'open'").await?,
            apps_ready_to_list: count(db, "select count(*) from apps a where a.status = 'draft' and a.visibility = 'public' and exists (select 1 from releases r where r.app_id = a.id and r.status = 'approved')").await?,
        },
        last_backup: ops.backup_dir.as_ref().and_then(|d| crate::ops::stamp_in(&d.join("last-success"))),
        last_restore_check: ops.backup_dir.as_ref().and_then(|d| crate::ops::stamp_in(&d.join("last-restore-check"))),
        deployed: ops.deploy_dir.as_ref().and_then(|d| {
            std::fs::read_link(d.join("current"))
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().chars().take(12).collect()))
        }),
        scanning: cfg.scanner.is_on(),
        signing_key: cfg.signing_key.is_some(),
        github_app: cfg.github_app.is_some(),
        accounts: count(db, "select count(*) from users where closed_at is null").await?,
        apps_listed: count(db, "select count(*) from apps where status = 'listed'").await?,
        approved_last_day: count(db, "select count(*) from releases where status = 'approved' and reviewed_at > now() - interval '1 day'").await?,
    }))
}

// ---------------------------------------------------------------------------------------------
// Accounts

#[derive(Deserialize)]
struct FindQuery {
    /// Part of a display name or People handle, or an account ID.
    q: String,
}

#[derive(Serialize, sqlx::FromRow)]
struct Found {
    id: Uuid,
    display_name: String,
    handle: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    suspended: bool,
    closed: bool,
    is_admin: bool,
}

async fn find_users(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(q): Query<FindQuery>,
) -> ApiResult<Json<Vec<Found>>> {
    let text = q.q.trim();
    if text.is_empty() || text.chars().count() > 100 {
        return Err(ApiError::invalid("q", "give 1 to 100 characters"));
    }
    let id: Option<Uuid> = text.parse().ok();
    let pattern = format!(
        "%{}%",
        text.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    Ok(Json(
        sqlx::query_as(
            "select u.id, u.display_name, c.handle, u.created_at, u.suspended_at is not null as suspended,
                    u.closed_at is not null as closed, u.is_admin
             from users u left join creatives c on c.user_id = u.id
             where u.id = $1 or u.display_name ilike $2 or c.handle ilike $2
             order by u.created_at desc limit 20",
        )
        .bind(id)
        .bind(&pattern)
        .fetch_all(&state.db)
        .await?,
    ))
}

#[derive(Serialize)]
struct UserDetails {
    id: Uuid,
    display_name: String,
    country_code: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    suspended_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    closed_at: Option<OffsetDateTime>,
    is_admin: bool,
    /// Sign-in providers linked (not the accounts themselves).
    sign_ins: Vec<String>,
    apps: Vec<Value>,
    reports_about: Vec<Value>,
    decisions: Vec<Value>,
}

/// (display name, country, joined, suspended, closed, admin)
type AccountRow = (
    String,
    Option<String>,
    OffsetDateTime,
    Option<OffsetDateTime>,
    Option<OffsetDateTime>,
    bool,
);
/// (id, reference, action, rule, about, decided, reversed)
type DecisionRow = (
    Uuid,
    String,
    String,
    String,
    String,
    OffsetDateTime,
    Option<OffsetDateTime>,
);

/// One account, for a decision. Reading it is logged (METRICS §3: staff access is logged).
async fn user_details(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<UserDetails>> {
    admin.need(Perm::Moderate)?;
    let db = &state.db;
    let row: Option<AccountRow> =
        sqlx::query_as(
            "select display_name, country_code, created_at, suspended_at, closed_at, is_admin from users where id = $1",
        )
        .bind(id)
        .fetch_optional(db)
        .await?;
    let (display_name, country_code, created_at, suspended_at, closed_at, is_admin) =
        row.ok_or(ApiError::NotFound)?;
    let sign_ins = sqlx::query_scalar(
        "select provider from identities where user_id = $1 order by created_at",
    )
    .bind(id)
    .fetch_all(db)
    .await?;
    let apps: Vec<(String, String, String, String)> = sqlx::query_as(
        "select slug, title, status::text, visibility from apps where maker_id = $1 order by created_at desc limit 50",
    )
    .bind(id)
    .fetch_all(db)
    .await?;
    let reports: Vec<(String, String, String, String, OffsetDateTime)> = sqlx::query_as(
        "select reference, subject_kind, reason, status, created_at from reports where subject_user_id = $1
         order by created_at desc limit 30",
    )
    .bind(id)
    .fetch_all(db)
    .await?;
    let decisions: Vec<DecisionRow> = sqlx::query_as(
        "select id, reference, action, rule, subject_label, created_at, reversed_at from moderation_decisions
         where user_id = $1 order by created_at desc limit 30",
    )
    .bind(id)
    .fetch_all(db)
    .await?;
    let mut tx = db.begin().await?;
    admin::audit(
        &mut tx,
        &admin.actor,
        "user.view",
        &id.to_string(),
        json!({}),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(Json(UserDetails {
        id,
        display_name,
        country_code,
        created_at,
        suspended_at,
        closed_at,
        is_admin,
        sign_ins,
        apps: apps
            .into_iter()
            .map(|(slug, title, status, visibility)| json!({ "slug": slug, "title": title, "status": status, "visibility": visibility }))
            .collect(),
        reports_about: reports
            .into_iter()
            .map(|(reference, kind, reason, status, at)| {
                json!({ "reference": reference, "about": kind, "reason": reason, "status": status, "at": at.unix_timestamp() })
            })
            .collect(),
        decisions: decisions
            .into_iter()
            .map(|(id, reference, action, rule, label, at, reversed)| {
                json!({ "id": id, "reference": reference, "action": action, "rule": rule, "about": label,
                        "at": at.unix_timestamp(), "reversed": reversed.is_some() })
            })
            .collect(),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecideUser {
    /// warn or suspend_account.
    action: String,
    /// A Content Policy rule (GET /v1/admin/queue lists them under `rules`).
    rule: String,
    /// The facts, for the statement of reasons the person gets (DSA Art. 17).
    facts: String,
    law: Option<String>,
}

/// A warning or suspension without a report: recorded, explained to the person and appealable
/// like any moderation decision. A suspension is sensitive: it waits for enough staff (staff.rs).
async fn decide_user(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(b): Json<DecideUser>,
) -> ApiResult<Response> {
    match b.action.as_str() {
        "warn" => admin.need(Perm::Moderate)?,
        "suspend_account" => admin.need(Perm::Sensitive)?,
        _ => return Err(ApiError::invalid("action", "warn or suspend_account")),
    }
    if !crate::moderation::RULES
        .iter()
        .any(|(code, _)| *code == b.rule)
    {
        return Err(ApiError::invalid(
            "rule",
            "use one of the Content Policy rules",
        ));
    }
    let facts = b.facts.trim();
    if facts.is_empty() || facts.chars().count() > 4000 {
        return Err(ApiError::invalid(
            "facts",
            "state the facts in up to 4000 characters",
        ));
    }
    if id == admin.user_id {
        return Err(ApiError::invalid("action", "not on your own account"));
    }
    let name: String =
        sqlx::query_scalar("select display_name from users where id = $1 and closed_at is null")
            .bind(id)
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::NotFound)?;
    if b.action == "suspend_account" {
        return Ok(crate::staff::request(
            &state,
            &admin,
            "account.suspend",
            &format!("Suspend {name} ({})", b.rule),
            json!({ "user_id": id, "rule": b.rule, "facts": facts, "law": b.law }),
        )
        .await?
        .into_response());
    }
    let d = decide_account(
        &state,
        id,
        "warn",
        &b.rule,
        facts,
        b.law.as_deref(),
        &admin.actor,
    )
    .await?;
    Ok(Json(d).into_response())
}

/// Records and carries out a decision about an account.
async fn decide_account(
    state: &AppState,
    id: Uuid,
    action: &str,
    rule: &str,
    facts: &str,
    law: Option<&str>,
    actor: &str,
) -> ApiResult<Value> {
    let mut tx = state.db.begin().await?;
    let row: Option<(String, Option<OffsetDateTime>, Option<OffsetDateTime>)> = sqlx::query_as(
        "select display_name, suspended_at, closed_at from users where id = $1 for update",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let (name, suspended, closed) = row.ok_or(ApiError::NotFound)?;
    if closed.is_some() {
        return Err(ApiError::Conflict("that account is closed".into()));
    }
    if action == "suspend_account" && suspended.is_some() {
        return Err(ApiError::Conflict(
            "that account is already suspended".into(),
        ));
    }
    let decision = crate::moderation::decide(
        &mut tx,
        &crate::moderation::NewDecision {
            user_id: id,
            subject_kind: "account",
            subject_id: id,
            subject_label: &name,
            action,
            rule,
            law: law.filter(|l| !l.trim().is_empty()),
            facts,
            decided_by: actor,
        },
    )
    .await
    .map_err(|e| ApiError::Conflict(format!("{e:#}")))?;
    tx.commit().await?;
    Ok(json!({ "decision": decision }))
}

/// A suspension, once approved (staff.rs).
pub(crate) async fn suspend_stored(state: &AppState, p: &Value, actor: &str) -> ApiResult<Value> {
    let id: Uuid =
        serde_json::from_value(p["user_id"].clone()).map_err(|e| ApiError::Internal(e.into()))?;
    decide_account(
        state,
        id,
        "suspend_account",
        p["rule"].as_str().unwrap_or_default(),
        p["facts"].as_str().unwrap_or_default(),
        p["law"].as_str(),
        actor,
    )
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Lift {
    /// Why, for the audit log; the person is told their account is back.
    reason: String,
}

/// Ends a suspension early (not an appeal: that's /admin/decisions/{id}/appeal). Sensitive: it
/// waits for enough staff (staff.rs).
async fn lift_suspension(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(b): Json<Lift>,
) -> ApiResult<crate::staff::Outcome> {
    admin.need(Perm::Sensitive)?;
    let reason = b.reason.trim();
    if !(10..=1000).contains(&reason.chars().count()) {
        return Err(ApiError::invalid(
            "reason",
            "say why in 10 to 1000 characters",
        ));
    }
    let name: Option<String> = sqlx::query_scalar(
        "select display_name from users where id = $1 and suspended_at is not null",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let name = name.ok_or_else(|| ApiError::Conflict("that account isn't suspended".into()))?;
    crate::staff::request(
        &state,
        &admin,
        "account.lift",
        &format!("Lift {name}'s suspension: {reason}"),
        json!({ "user_id": id, "reason": reason }),
    )
    .await
}

/// Lets someone who lost their passkeys publish without one for a week (passkeys.rs), after
/// staff checked it's really them. Sensitive: someone who stole a sign-in could ask for it too,
/// so it waits for enough staff (staff.rs), like a suspension.
async fn passkey_grace(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(b): Json<Lift>,
) -> ApiResult<crate::staff::Outcome> {
    admin.need(Perm::Sensitive)?;
    let reason = b.reason.trim();
    if !(10..=1000).contains(&reason.chars().count()) {
        return Err(ApiError::invalid(
            "reason",
            "say how you checked it's them, in 10 to 1000 characters",
        ));
    }
    let name: Option<String> =
        sqlx::query_scalar("select display_name from users where id = $1 and closed_at is null")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    let name = name.ok_or(ApiError::NotFound)?;
    crate::staff::request(
        &state,
        &admin,
        "account.passkey_grace",
        &format!("Let {name} publish without a passkey for a week: {reason}"),
        json!({ "user_id": id, "reason": reason }),
    )
    .await
}

/// A passkey grace, once approved (staff.rs).
pub(crate) async fn passkey_grace_stored(
    state: &AppState,
    p: &Value,
    actor: &str,
) -> ApiResult<Value> {
    let id: Uuid =
        serde_json::from_value(p["user_id"].clone()).map_err(|e| ApiError::Internal(e.into()))?;
    let reason = p["reason"].as_str().unwrap_or_default();
    let mut tx = state.db.begin().await?;
    let until = crate::passkeys::grace(&mut tx, id).await?;
    crate::notify::send(
        &mut tx,
        crate::notify::Notice {
            user_id: id,
            category: "apps",
            title: "Add a new passkey this week".into(),
            body: format!(
                "Slop lets you publish and manage payouts without a passkey until {}. Add a new one under \
                 Account → Sign-ins before then. If you didn't ask for this, tell Slop at once.",
                until.date()
            ),
            link: "/account#sign-ins".into(),
            collapse_key: None,
        },
    )
    .await?;
    admin::audit(
        &mut tx,
        actor,
        "account.passkey_grace",
        &id.to_string(),
        json!({ "reason": reason, "until": until.to_string() }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(json!({ "until": until.to_string() }))
}

/// Lifting a suspension, once approved (staff.rs).
pub(crate) async fn lift_stored(state: &AppState, p: &Value, actor: &str) -> ApiResult<Value> {
    let id: Uuid =
        serde_json::from_value(p["user_id"].clone()).map_err(|e| ApiError::Internal(e.into()))?;
    let reason = p["reason"].as_str().unwrap_or_default();
    let mut tx = state.db.begin().await?;
    let lifted = sqlx::query(
        "update users set suspended_at = null where id = $1 and suspended_at is not null",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if lifted == 0 {
        return Err(ApiError::Conflict("that account isn't suspended".into()));
    }
    sqlx::query(
        "update moderation_decisions set reversed_at = now()
         where user_id = $1 and action = 'suspend_account' and reversed_at is null",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    crate::notify::send(
        &mut tx,
        crate::notify::Notice {
            user_id: id,
            category: "moderation",
            title: "Your account is no longer suspended".into(),
            body: "Slop lifted the suspension. Everything works as before.".into(),
            link: "/account#decisions".into(),
            collapse_key: None,
        },
    )
    .await?;
    admin::audit(
        &mut tx,
        actor,
        "moderation.lift_suspension",
        &id.to_string(),
        json!({ "reason": reason }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------------------------
// MCP (Model Context Protocol, JSON-RPC 2.0 over HTTP)

const PROTOCOL: &str = "2025-06-18";

/// A tool: one call to the admin API. `{name}` in the path is filled from the arguments of the
/// same name; the other arguments are the query (GET) or the JSON body.
pub(crate) struct Tool {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) method: Method,
    pub(crate) path: &'static str,
    /// JSON Schema properties, and which are required.
    pub(crate) properties: fn() -> Value,
    pub(crate) required: &'static [&'static str],
}

pub(crate) fn s(desc: &str) -> Value {
    json!({ "type": "string", "description": desc })
}

const TOOLS: &[Tool] = &[
    Tool {
        name: "slop_status",
        description: "Slop's health and what's waiting for a person: open alerts (stale backups, failed deploys, scanner failures, overdue reports…), queue counts, last backup, the running commit. Start here.",
        method: Method::GET,
        path: "/v1/admin/status",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "review_guide",
        description: "How Slop reviews releases, changes, pictures and portfolios, and what to look for. Read before deciding anything.",
        method: Method::GET,
        path: "/v1/admin/review/guide",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "review_queue",
        description: "Everything waiting for a review decision, each with the app's store page, its checks (scans, Microsoft Defender, signatures), pictures, earlier notes and the exact requests that decide it.",
        method: Method::GET,
        path: "/v1/admin/review",
        properties: || json!({ "kind": s("Only this kind: release, change, media, listing, portfolio, problem or review_dispute") }),
        required: &[],
    },
    Tool {
        name: "admin_queue",
        description: "The admin queue as the website shows it: releases, reports (urgent first), appeals, live releases flagged by a re-scan, apps ready for the store, and the Content Policy rules decisions cite.",
        method: Method::GET,
        path: "/v1/admin/queue",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "add_note",
        description: "Leave a review note for a person instead of deciding (kind: release, change, media, listing, portfolio, problem, review_dispute).",
        method: Method::POST,
        path: "/v1/admin/review/{kind}/{id}/notes",
        properties: || json!({ "kind": s("What it's about"), "id": s("Its ID"), "verdict": s("pass, concern or unsure"), "note": s("The note, up to 4000 characters") }),
        required: &["kind", "id", "note"],
    },
    Tool {
        name: "approve_release",
        description: "Approve and sign a release that's awaiting review. Needs a review or moderate token.",
        method: Method::POST,
        path: "/v1/admin/releases/{id}/approve",
        properties: || json!({ "id": s("The release ID") }),
        required: &["id"],
    },
    Tool {
        name: "reject_release",
        description: "Reject a release, with the reason the maker is shown.",
        method: Method::POST,
        path: "/v1/admin/releases/{id}/reject",
        properties: || json!({ "id": s("The release ID"), "reason": s("Shown to the maker") }),
        required: &["id", "reason"],
    },
    Tool {
        name: "withdraw_release",
        description: "Withdraw an approved release (the kill switch): it stops launching for everyone. Needs a moderate token.",
        method: Method::POST,
        path: "/v1/admin/releases/{id}/revoke",
        properties: || json!({ "id": s("The release ID"), "reason": s("Shown to the maker and players") }),
        required: &["id", "reason"],
    },
    Tool {
        name: "clear_scan_finding",
        description: "Clear a scan finding or Microsoft Defender result on a release file as a false positive (before approval), with a logged reason of 10+ characters.",
        method: Method::POST,
        path: "/v1/admin/releases/{id}/files/{platform}/clear",
        properties: || json!({ "id": s("The release ID"), "platform": s("windows, linux, linux-deb or linux-rpm"), "reason": s("Why it's a false positive") }),
        required: &["id", "platform", "reason"],
    },
    Tool {
        name: "keep_rescanned_release",
        description: "Keep a live release that a re-scan flagged, as a false positive, with a logged reason of 10+ characters.",
        method: Method::POST,
        path: "/v1/admin/releases/{id}/files/{platform}/rescan-clear",
        properties: || json!({ "id": s("The release ID"), "platform": s("The file's platform"), "reason": s("Why it's a false positive") }),
        required: &["id", "platform", "reason"],
    },
    Tool {
        name: "decide_change",
        description: "Approve or turn down a maker's change request (switching the release players get, or moving the repository).",
        method: Method::POST,
        path: "/v1/admin/changes/{id}/{decision}",
        properties: || json!({ "id": s("The change ID"), "decision": s("approve or reject"), "reason": s("Required to reject: shown to the maker") }),
        required: &["id", "decision"],
    },
    Tool {
        name: "decide_pictures",
        description: "Approve or turn down an app's new store-page pictures.",
        method: Method::POST,
        path: "/v1/admin/media/{id}/{decision}",
        properties: || json!({ "id": s("The picture set ID"), "decision": s("approve or reject"), "reason": s("Required to reject: shown to the maker") }),
        required: &["id", "decision"],
    },
    Tool {
        name: "decide_portfolio",
        description: "Approve or turn down a creative's People portfolio.",
        method: Method::POST,
        path: "/v1/admin/portfolio/{id}/{decision}",
        properties: || json!({ "id": s("The portfolio set ID"), "decision": s("approve or reject"), "reason": s("Required to reject: shown to the creative") }),
        required: &["id", "decision"],
    },
    Tool {
        name: "list_app",
        description: "Put an app that's ready into the store.",
        method: Method::POST,
        path: "/v1/admin/apps/{slug}/list",
        properties: || json!({ "slug": s("The app's address") }),
        required: &["slug"],
    },
    Tool {
        name: "delist_app",
        description: "Take an app out of the store, citing a Content Policy rule; the maker gets a statement of reasons and can appeal. Needs a moderate token.",
        method: Method::POST,
        path: "/v1/admin/apps/{slug}/delist",
        properties: || json!({ "slug": s("The app's address"), "rule": s("A Content Policy rule code"), "reason": s("The facts, for the maker") }),
        required: &["slug", "rule", "reason"],
    },
    Tool {
        name: "act_on_report",
        description: "Decide an open report: delist_app, hide_review, remove_reply (the app maker's reply under a review), hide_profile, hide_team, warn or suspend_account (suspending needs a moderate token). The person gets a statement of reasons; the reporter is told the outcome.",
        method: Method::POST,
        path: "/v1/admin/reports/{id}/act",
        properties: || json!({ "id": s("The report ID"), "action": s("What to do"), "rule": s("A Content Policy rule code"), "facts": s("The facts, for the statement of reasons"), "outcome": s("What the reporter is told"), "law": s("Optional: the law, if it's illegal content"), "external_reference": s("Optional: where it was passed on (NCMEC, NCA, police) and its reference") }),
        required: &["id", "action", "rule", "facts", "outcome"],
    },
    Tool {
        name: "dismiss_report",
        description: "Close a report without action, telling the reporter why.",
        method: Method::POST,
        path: "/v1/admin/reports/{id}/dismiss",
        properties: || json!({ "id": s("The report ID"), "outcome": s("What the reporter is told") }),
        required: &["id", "outcome"],
    },
    Tool {
        name: "decide_appeal",
        description: "Keep or reverse a moderation decision someone appealed, with the reason they're shown.",
        method: Method::POST,
        path: "/v1/admin/decisions/{id}/appeal",
        properties: || json!({ "id": s("The decision ID"), "reverse": { "type": "boolean", "description": "true reverses it" }, "reason": s("Shown to the person") }),
        required: &["id", "reverse", "reason"],
    },
    Tool {
        name: "close_abuse_signal",
        description: "Close a pattern the abuse checks found (admin_queue lists them under `abuse`): acted=true when you dealt with it through the moderation tools, false when it's nothing. The note is kept.",
        method: Method::POST,
        path: "/v1/admin/abuse/{id}/dismiss",
        properties: || json!({ "id": s("The signal ID"), "note": s("What you found"), "acted": { "type": "boolean", "description": "true if it was acted on" } }),
        required: &["id", "note"],
    },
    Tool {
        name: "find_users",
        description: "Find accounts by part of a display name or People handle, or by account ID.",
        method: Method::GET,
        path: "/v1/admin/users",
        properties: || json!({ "q": s("Name, handle or ID") }),
        required: &["q"],
    },
    Tool {
        name: "user_details",
        description: "One account: when it joined, suspension, its apps, reports about it and decisions about it. Reading it is logged.",
        method: Method::GET,
        path: "/v1/admin/users/{id}",
        properties: || json!({ "id": s("The account ID") }),
        required: &["id"],
    },
    Tool {
        name: "moderate_user",
        description: "Warn or suspend an account without a report, citing a Content Policy rule and the facts. The person gets a statement of reasons and can appeal. Suspending needs a moderate token.",
        method: Method::POST,
        path: "/v1/admin/users/{id}/decide",
        properties: || json!({ "id": s("The account ID"), "action": s("warn or suspend_account"), "rule": s("A Content Policy rule code"), "facts": s("The facts, for the statement of reasons"), "law": s("Optional: the law, if it's illegal") }),
        required: &["id", "action", "rule", "facts"],
    },
    Tool {
        name: "lift_suspension",
        description: "End an account's suspension early, with a logged reason (10+ characters). Needs a moderate token.",
        method: Method::POST,
        path: "/v1/admin/users/{id}/lift-suspension",
        properties: || json!({ "id": s("The account ID"), "reason": s("Why") }),
        required: &["id", "reason"],
    },
    Tool {
        name: "passkey_grace",
        description: "Let someone who lost their passkeys publish and manage payouts without one for a week, after checking it's really them (say how in the reason, 10+ characters). Waits for staff approval like other sensitive actions. Needs a moderate token.",
        method: Method::POST,
        path: "/v1/admin/users/{id}/passkey-grace",
        properties: || json!({ "id": s("The account ID"), "reason": s("How you checked it's them") }),
        required: &["id", "reason"],
    },
    Tool {
        name: "pending_actions",
        description: "Sensitive actions (withdrawals, delistings, suspensions, role changes) waiting for more staff approvals. Only people approve them, signed in on the website; tell your admin what's waiting.",
        method: Method::GET,
        path: "/v1/admin/actions",
        properties: || json!({ "status": s("pending (the default) or all") }),
        required: &[],
    },
    Tool {
        name: "audit_log",
        description: "Decisions and actions, newest first, 100 at a time, with who made them. Search with q; page with before (the last id you got).",
        method: Method::GET,
        path: "/v1/admin/audit",
        properties: || json!({ "q": s("Optional: part of who did it, what, or to what (an app address, an account ID)"), "before": { "type": "integer", "description": "Optional: entries older than this id" } }),
        required: &[],
    },
    // Apps
    Tool {
        name: "find_apps",
        description: "Find apps by part of the title, address or maker's name, optionally only one status.",
        method: Method::GET,
        path: "/v1/admin/apps",
        properties: || json!({ "q": s("Optional: part of the title, address or maker's name"), "status": s("Optional: draft, listed or delisted") }),
        required: &[],
    },
    Tool {
        name: "app_record",
        description: "Everything about one app: its store page, releases, donations and pauses, reports, decisions and full history.",
        method: Method::GET,
        path: "/v1/admin/apps/{slug}/record",
        properties: || json!({ "slug": s("The app's address") }),
        required: &["slug"],
    },
    Tool {
        name: "edit_store_page",
        description: "Fix an app's store page directly: title, summary, description (Markdown), minimum age, content labels. Give only what changes, and a reason the maker is told. Needs a review or moderate token.",
        method: Method::PATCH,
        path: "/v1/admin/apps/{slug}/listing",
        properties: || json!({ "slug": s("The app's address"), "title": s("Optional"), "summary": s("Optional"), "description_md": s("Optional"), "min_age": { "type": "integer", "description": "Optional: 0, 3, 7, 12, 16 or 18" }, "content_labels": { "type": "array", "items": { "type": "string" }, "description": "Optional: the whole new list" }, "reason": s("Why, for the maker") }),
        required: &["slug", "reason"],
    },
    Tool {
        name: "set_app_stage",
        description: "Move an app between coming_soon, beta and released.",
        method: Method::POST,
        path: "/v1/admin/apps/{slug}/stage",
        properties: || json!({ "slug": s("The app's address"), "stage": s("coming_soon, beta or released") }),
        required: &["slug", "stage"],
    },
    Tool {
        name: "freeze_app",
        description: "Stop an app taking new releases while you look into it. It stays in the store and installed copies keep working; the maker is told the reason. Needs a review or moderate token.",
        method: Method::POST,
        path: "/v1/admin/apps/{slug}/freeze",
        properties: || json!({ "slug": s("The app's address"), "reason": s("What's wrong and what would fix it, for the maker") }),
        required: &["slug", "reason"],
    },
    Tool {
        name: "unfreeze_app",
        description: "Lift a freeze staff placed. For an automatic freeze (Slop lost its repository) it checks the repository again now instead. Returns the freeze still in place, or null.",
        method: Method::POST,
        path: "/v1/admin/apps/{slug}/unfreeze",
        properties: || json!({ "slug": s("The app's address"), "note": s("For the maker and the record") }),
        required: &["slug", "note"],
    },
    // Donations
    Tool {
        name: "donation_pauses",
        description: "Apps with donations paused, with makers' requests to lift them (open ones first).",
        method: Method::GET,
        path: "/v1/admin/donation-holds",
        properties: || json!({ "show": s("open (the default) or all") }),
        required: &[],
    },
    Tool {
        name: "pause_donations",
        description: "Pause an app's donations: kind review (for now, while you look) or off (until the maker fixes something; a sensitive action, needs a moderate token). Monthly donors aren't charged meanwhile. The maker is told the reason and can ask for it to be lifted.",
        method: Method::POST,
        path: "/v1/admin/apps/{slug}/donation-hold",
        properties: || json!({ "slug": s("The app's address"), "kind": s("review or off"), "reason": s("What's wrong and what would fix it, for the maker"), "ground": s("Optional: a Content Policy rule, law or complaint reference") }),
        required: &["slug", "kind", "reason"],
    },
    Tool {
        name: "lift_donation_pause",
        description: "Lift a donation pause. Monthly donations restart from their next date.",
        method: Method::POST,
        path: "/v1/admin/donation-holds/{id}/lift",
        properties: || json!({ "id": s("The pause ID"), "note": s("For the maker and the record") }),
        required: &["id", "note"],
    },
    Tool {
        name: "answer_donation_request",
        description: "Answer a maker who asked for their donation pause to be lifted: lift=true lifts it, false keeps it and the answer says what's still wrong.",
        method: Method::POST,
        path: "/v1/admin/donation-hold-requests/{id}/answer",
        properties: || json!({ "id": s("The request ID"), "lift": { "type": "boolean", "description": "true lifts the pause" }, "answer": s("Shown to the maker") }),
        required: &["id", "lift", "answer"],
    },
    // People
    Tool {
        name: "user_profile",
        description: "What staff can fix about an account: its display name, its People page (name, headline, about, location, links) and the teams it runs.",
        method: Method::GET,
        path: "/v1/admin/users/{id}/profile",
        properties: || json!({ "id": s("The account ID") }),
        required: &["id"],
    },
    Tool {
        name: "edit_profile",
        description: "Fix an account's display name or People page (for impersonation, slurs, scam links): give only what changes, and a reason the person is told. Needs a review or moderate token.",
        method: Method::PATCH,
        path: "/v1/admin/users/{id}/profile",
        properties: || json!({ "id": s("The account ID"), "display_name": s("Optional"), "creative": { "type": "object", "description": "Optional: their People page", "properties": { "name": s("Optional"), "headline": s("Optional"), "about_md": s("Optional"), "location": s("Optional"), "remove_links": { "type": "boolean", "description": "Optional: true removes every link" } } }, "reason": s("Why, for the person") }),
        required: &["id", "reason"],
    },
    Tool {
        name: "edit_team",
        description: "Fix a team's name or description, with a reason its owners and admins are told.",
        method: Method::PATCH,
        path: "/v1/admin/teams/{slug}",
        properties: || json!({ "slug": s("The team's address"), "name": s("Optional"), "about": s("Optional"), "reason": s("Why, for the team") }),
        required: &["slug", "reason"],
    },
    // The front page
    Tool {
        name: "featured_apps",
        description: "The Featured carousel at the top of the front page, in order.",
        method: Method::GET,
        path: "/v1/admin/featured",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "set_featured_apps",
        description: "Replace the Featured carousel (up to 8 apps, in order, each with a line of up to 160 characters saying why).",
        method: Method::PUT,
        path: "/v1/admin/featured",
        properties: || json!({ "apps": { "type": "array", "items": { "type": "object", "properties": { "slug": s("App address"), "blurb": s("Why it's featured") }, "required": ["slug"] } } }),
        required: &["apps"],
    },
    Tool {
        name: "store_shelves",
        description: "The shelves staff made (each with its apps), and the automatic front-page shelves with whether each is hidden.",
        method: Method::GET,
        path: "/v1/admin/shelves",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "create_shelf",
        description: "Make a shelf: an address, a title, a line saying what ties the apps together, and app addresses in order (up to 24). on_front=false keeps it off the front page (it still has its own page).",
        method: Method::POST,
        path: "/v1/admin/shelves",
        properties: || json!({ "slug": s("Its address: lowercase letters, digits and dashes"), "title": s("Up to 60 characters"), "blurb": s("Optional: up to 160 characters"), "on_front": { "type": "boolean", "description": "Optional, default true" }, "apps": { "type": "array", "items": { "type": "string" }, "description": "App addresses, in order" } }),
        required: &["slug", "title", "apps"],
    },
    Tool {
        name: "update_shelf",
        description: "Replace a shelf's title, line, front-page setting and apps (send all of them).",
        method: Method::PUT,
        path: "/v1/admin/shelves/{slug}",
        properties: || json!({ "slug": s("The shelf's address"), "title": s("Up to 60 characters"), "blurb": s("Optional"), "on_front": { "type": "boolean", "description": "Optional, default true" }, "apps": { "type": "array", "items": { "type": "string" }, "description": "App addresses, in order" } }),
        required: &["slug", "title", "apps"],
    },
    Tool {
        name: "delete_shelf",
        description: "Remove a shelf staff made.",
        method: Method::DELETE,
        path: "/v1/admin/shelves/{slug}",
        properties: || json!({ "slug": s("The shelf's address") }),
        required: &["slug"],
    },
    Tool {
        name: "order_shelves",
        description: "Put the shelves staff made in a new order on the front page.",
        method: Method::PUT,
        path: "/v1/admin/shelves/order",
        properties: || json!({ "slugs": { "type": "array", "items": { "type": "string" }, "description": "Every shelf's address, in order" } }),
        required: &["slugs"],
    },
    Tool {
        name: "hide_front_shelf",
        description: "Hide or show one of the automatic front-page shelves (store_shelves lists their keys).",
        method: Method::POST,
        path: "/v1/admin/front-shelves/{key}",
        properties: || json!({ "key": s("popular, upcoming, categories, not_slop, barely_seen, glow_ups, community, fresh or fresh_slop"), "hidden": { "type": "boolean", "description": "true hides it" } }),
        required: &["key", "hidden"],
    },
    // Legal documents
    Tool {
        name: "legal_documents",
        description: "Slop's legal documents (terms, privacy, content, publisher, people): every version with when it took or takes effect, and the newest text.",
        method: Method::GET,
        path: "/v1/admin/legal",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "publish_legal_document",
        description: "Publish a new version of a legal document: the whole new text and a one-line summary of what changed, which everyone is told. It takes effect 15 days later. A sensitive action: needs a moderate token, and waits for a second person once Slop requires two. Only change legal text when your admin asked for exactly this.",
        method: Method::POST,
        path: "/v1/admin/legal/{kind}",
        properties: || json!({ "kind": s("terms, privacy, content, publisher or people"), "summary": s("What changed, in plain words"), "body_md": s("The whole new text, Markdown") }),
        required: &["kind", "summary", "body_md"],
    },
    Tool {
        name: "withdraw_legal_document",
        description: "Withdraw an announced version published from the admin panel, before it takes effect.",
        method: Method::POST,
        path: "/v1/admin/legal/{kind}/{version}/withdraw",
        properties: || json!({ "kind": s("The document"), "version": s("The announced version, like 2026-10-22"), "reason": s("Why, for the record") }),
        required: &["kind", "version", "reason"],
    },
    // Staff
    Tool {
        name: "staff",
        description: "Slop's staff and their roles, and how many approvals sensitive actions need.",
        method: Method::GET,
        path: "/v1/admin/staff",
        properties: || json!({}),
        required: &[],
    },
    Tool {
        name: "change_staff_role",
        description: "Ask to give or take a staff role (owner, moderator, reviewer, support, finance, security); giving one makes the account staff. From an agent this always waits for a person to approve it signed in on the website (tell your admin). Needs a moderate token and an admin who may manage staff.",
        method: Method::POST,
        path: "/v1/admin/staff/{user_id}/roles",
        properties: || json!({ "user_id": s("The account ID"), "role": s("The role"), "grant": { "type": "boolean", "description": "true gives it, false takes it away" } }),
        required: &["user_id", "role", "grant"],
    },
    Tool {
        name: "remove_from_staff",
        description: "Ask to take someone off staff entirely (every role, and their admin API tokens are revoked). From an agent this always waits for a person to approve it on the website.",
        method: Method::POST,
        path: "/v1/admin/staff/{user_id}/remove",
        properties: || json!({ "user_id": s("The account ID"), "reason": s("Why, 10+ characters") }),
        required: &["user_id", "reason"],
    },
    Tool {
        name: "cancel_pending_action",
        description: "Withdraw a sensitive action you asked for that's still waiting for approval.",
        method: Method::POST,
        path: "/v1/admin/actions/{id}/cancel",
        properties: || json!({ "id": s("The pending action's ID") }),
        required: &["id"],
    },
];

fn tool_list(tools: &[Tool]) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": { "type": "object", "properties": (t.properties)(), "required": t.required },
                })
            })
            .collect(),
    )
}

/// Builds the admin API request a tool call stands for.
pub(crate) fn tool_request(
    tool: &Tool,
    args: &serde_json::Map<String, Value>,
) -> Result<(Method, String, Option<Value>), String> {
    let mut rest = args.clone();
    for r in tool.required {
        if !args.contains_key(*r) {
            return Err(format!("missing argument: {r}"));
        }
    }
    let mut path = String::new();
    let mut tail = tool.path;
    while let Some(start) = tail.find('{') {
        let end = tail[start..].find('}').ok_or("bad tool path")? + start;
        path.push_str(&tail[..start]);
        let name = &tail[start + 1..end];
        let value = rest
            .remove(name)
            .ok_or_else(|| format!("missing argument: {name}"))?;
        let text = match value {
            Value::String(s) => s,
            other => other.to_string(),
        };
        if name == "decision" && text != "approve" && text != "reject" {
            return Err("decision must be approve or reject".into());
        }
        if text.is_empty() || text == "." || text == ".." {
            return Err(format!("{name} isn't a valid value"));
        }
        // URL-unreserved characters stay as they are (app addresses have dashes, and an
        // app-scoped token compares them as written); everything else, `/` and `?` included,
        // is escaped, so an argument stays one path segment.
        path.push_str(&crate::slop_build::path_segment(&text));
        tail = &tail[end + 1..];
    }
    path.push_str(tail);
    if tool.method == Method::GET {
        if !rest.is_empty() {
            let mut q = url::form_urlencoded::Serializer::new(String::new());
            for (k, v) in &rest {
                q.append_pair(
                    k,
                    &match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    },
                );
            }
            path = format!("{path}?{}", q.finish());
        }
        Ok((Method::GET, path, None))
    } else {
        // Approvals and listings take no body. Any other tool with fields beyond its path sends
        // a JSON body even when the optional ones are all left out, since its route reads one
        // (a job's decline with no note, say).
        let has_body = !tool.path.contains("{decision}")
            && (tool.properties)()
                .as_object()
                .is_some_and(|p| p.keys().any(|k| !tool.path.contains(&format!("{{{k}}}"))));
        let body = (has_body || !rest.is_empty()).then_some(Value::Object(rest));
        Ok((tool.method.clone(), path, body))
    }
}

#[derive(Deserialize)]
struct RpcRequest {
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// MCP over plain HTTP POST (JSON responses, no server-sent events).
async fn mcp(State(state): State<AppState>, _admin: AdminUser, request: Request) -> Response {
    serve_mcp(
        &state,
        request,
        TOOLS,
        "slop-admin",
        "Slop's admin API. Call slop_status first, read review_guide before deciding, and only act when the admin you work for asked you to. Every action is in the audit log under your token's name.",
    )
    .await
}

/// Answers one MCP request with `tools`, each calling Slop's API with the caller's credentials.
pub(crate) async fn serve_mcp(
    state: &AppState,
    request: Request,
    tools: &'static [Tool],
    server: &str,
    instructions: &str,
) -> Response {
    let headers = request.headers().clone();
    let connect = request
        .extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .cloned();
    let Ok(bytes) = axum::body::to_bytes(request.into_body(), 256 * 1024).await else {
        return Json(rpc_error(Value::Null, -32700, "request too large")).into_response();
    };
    let Ok(req) = serde_json::from_slice::<RpcRequest>(&bytes) else {
        return Json(rpc_error(Value::Null, -32700, "not a JSON-RPC request")).into_response();
    };
    let id = req.id.clone().unwrap_or(Value::Null);
    // Notifications get no answer.
    if req.id.is_none() {
        return StatusCode::ACCEPTED.into_response();
    }
    let reply = match req.method.as_str() {
        "initialize" => rpc_result(
            id,
            json!({
                "protocolVersion": PROTOCOL,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": server, "version": env!("CARGO_PKG_VERSION") },
                "instructions": instructions,
            }),
        ),
        "ping" => rpc_result(id, json!({})),
        "tools/list" => rpc_result(id, json!({ "tools": tool_list(tools) })),
        "tools/call" => {
            let name = req.params["name"].as_str().unwrap_or("");
            let empty = serde_json::Map::new();
            let args = req.params["arguments"].as_object().unwrap_or(&empty);
            match tools.iter().find(|t| t.name == name) {
                None => rpc_error(id, -32602, "unknown tool"),
                Some(tool) => match call_tool(state, tool, args, &headers, connect).await {
                    Ok((ok, text)) => rpc_result(
                        id,
                        json!({ "content": [{ "type": "text", "text": text }], "isError": !ok }),
                    ),
                    Err(message) => rpc_result(
                        id,
                        json!({ "content": [{ "type": "text", "text": message }], "isError": true }),
                    ),
                },
            }
        }
        _ => rpc_error(id, -32601, "method not found"),
    };
    Json(reply).into_response()
}

/// MCP clients may open a GET stream for server messages; this server sends none.
async fn mcp_get(_admin: AdminUser) -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

/// Calls the admin API through Slop's router with the caller's own credentials.
async fn call_tool(
    state: &AppState,
    tool: &Tool,
    args: &serde_json::Map<String, Value>,
    headers: &HeaderMap,
    connect: Option<ConnectInfo<std::net::SocketAddr>>,
) -> Result<(bool, String), String> {
    let (method, path, body) = tool_request(tool, args)?;
    // A fresh router for the call (cheap): kept in the state it would be a reference cycle.
    let router = crate::router(state.clone()).map_err(|e| e.to_string())?;
    let mut builder = Request::builder().method(method).uri(&path);
    for name in [
        header::AUTHORIZATION,
        header::COOKIE,
        header::ORIGIN,
        header::HOST,
    ] {
        if let Some(v) = headers.get(&name) {
            builder = builder.header(name, v);
        }
    }
    if let Some(v) = headers.get("sec-fetch-site") {
        builder = builder.header("sec-fetch-site", v);
    }
    let request = match body {
        Some(b) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string())),
        None => builder.body(Body::empty()),
    }
    .map_err(|e| e.to_string())?;
    let mut request = request;
    if let Some(c) = connect {
        request.extensions_mut().insert(c);
    }
    let res = router.oneshot(request).await.map_err(|e| e.to_string())?;
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 8 * 1024 * 1024)
        .await
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let text = if text.is_empty() {
        format!("Done (HTTP {}).", status.as_u16())
    } else {
        text
    };
    Ok((status.is_success(), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: Value) -> serde_json::Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn tool_calls_become_admin_requests() {
        let tool = TOOLS.iter().find(|t| t.name == "reject_release").unwrap();
        let (m, path, body) =
            tool_request(tool, &args(json!({ "id": "abc", "reason": "no" }))).unwrap();
        assert_eq!(
            (m, path.as_str()),
            (Method::POST, "/v1/admin/releases/abc/reject")
        );
        assert_eq!(body, Some(json!({ "reason": "no" })));
        let tool = TOOLS.iter().find(|t| t.name == "find_users").unwrap();
        let (_, path, body) = tool_request(tool, &args(json!({ "q": "Mia & co" }))).unwrap();
        assert_eq!(path, "/v1/admin/users?q=Mia+%26+co");
        assert!(body.is_none());
        let tool = TOOLS.iter().find(|t| t.name == "decide_change").unwrap();
        assert!(tool_request(tool, &args(json!({ "id": "x", "decision": "../tokens" }))).is_err());
        let (_, path, body) =
            tool_request(tool, &args(json!({ "id": "x/y", "decision": "approve" }))).unwrap();
        assert_eq!(path, "/v1/admin/changes/x%2Fy/approve");
        assert!(body.is_none());
        assert!(tool_request(tool, &args(json!({ "id": "x" }))).is_err());
    }

    #[test]
    fn every_tool_has_a_schema() {
        for t in TOOLS {
            assert!((t.properties)().is_object(), "{}", t.name);
            for r in t.required {
                assert!((t.properties)().get(*r).is_some(), "{}: {r}", t.name);
            }
        }
    }
}
