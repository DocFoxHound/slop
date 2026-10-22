// SPDX-License-Identifier: AGPL-3.0-or-later
//! The admin panel's views across the whole store: one app's full record, staff edits to a
//! store page, and the money overview. The review queue and moderation are in admin_web.rs,
//! accounts in agent.rs, donation holds in donation_holds.rs.
//!
//! Staff edits go straight onto the store page (no review: staff are the reviewers), need a
//! reason, tell the maker what changed and why, and are in the audit log with the old values.

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, QueryBuilder};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState, admin,
    admin_web::AdminUser,
    catalog::{validate_content_labels, validate_min_age, validate_text},
    error::{ApiError, ApiResult},
    notify::{self, Notice},
    staff::Perm,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/apps/{slug}/record", get(record))
        .route(
            "/admin/apps/{slug}/listing",
            axum::routing::patch(edit_listing),
        )
        .route("/admin/money", get(money))
}

// ---------------------------------------------------------------------------------------------
// One app

#[derive(Serialize, sqlx::FromRow)]
struct AppRow {
    id: Uuid,
    slug: String,
    title: String,
    summary: String,
    description_md: String,
    status: String,
    stage: String,
    visibility: String,
    repo_url: String,
    license_spdx: String,
    min_age: i16,
    content_labels: Vec<String>,
    platforms: Vec<String>,
    maker_id: Uuid,
    maker: String,
    parent: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    frozen_at: Option<OffsetDateTime>,
    frozen_reason: Option<String>,
    /// Set when staff froze it by hand (admin_tools.rs).
    frozen_by: Option<String>,
    donations_on: bool,
    donation_account: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    updated_at: OffsetDateTime,
}

#[derive(Serialize, sqlx::FromRow)]
struct ReleaseRow {
    id: Uuid,
    version: String,
    status: String,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    reviewed_at: Option<OffsetDateTime>,
}

#[derive(Serialize, sqlx::FromRow)]
struct MoneyTotal {
    currency: String,
    donations: i64,
    donated_minor: i64,
    slop_fee_minor: i64,
    refunded_minor: i64,
}

#[derive(Serialize, sqlx::FromRow)]
struct ReportRow {
    id: Uuid,
    reference: String,
    reason: String,
    details: String,
    status: String,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

#[derive(Serialize, sqlx::FromRow)]
struct DecisionRow {
    id: Uuid,
    reference: String,
    action: String,
    rule: String,
    facts: String,
    decided_by: String,
    appeal_status: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    reversed_at: Option<OffsetDateTime>,
}

#[derive(Serialize, sqlx::FromRow)]
struct AuditRow {
    id: i64,
    #[serde(with = "time::serde::rfc3339")]
    at: OffsetDateTime,
    actor: String,
    action: String,
    detail: Value,
}

#[derive(Serialize)]
struct Record {
    app: AppRow,
    releases: Vec<ReleaseRow>,
    donations: Vec<MoneyTotal>,
    active_server_donations: i64,
    reports: Vec<ReportRow>,
    decisions: Vec<DecisionRow>,
    /// Everything staff and the system did to it, newest first.
    audit: Vec<AuditRow>,
}

/// Everything about one app on one page.
async fn record(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Record>> {
    admin.need(Perm::Read)?;
    let db = &state.db;
    let app: AppRow = sqlx::query_as(
        "select a.id, a.slug, a.title, a.summary, a.description_md, a.status::text as status, a.stage::text as stage,
                a.visibility, a.repo_url, a.license_spdx, a.min_age, a.content_labels, a.platforms,
                a.maker_id, u.display_name as maker, p.slug as parent, a.frozen_at, a.frozen_reason, a.frozen_by, a.donations_on,
                d.stripe_status as donation_account, a.created_at, a.updated_at
         from apps a join users u on u.id = a.maker_id
         left join apps p on p.id = a.parent_app_id
         left join donation_accounts d on d.user_id = a.maker_id
         where a.slug = $1",
    )
    .bind(&slug)
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;
    let releases = sqlx::query_as(
        "select id, version, status::text as status, created_at, reviewed_at from releases
         where app_id = $1 order by created_at desc limit 30",
    )
    .bind(app.id)
    .fetch_all(db)
    .await?;
    let donations = sqlx::query_as(
        "select currency, count(*) as donations, coalesce(sum(donation_minor), 0)::bigint as donated_minor,
                coalesce(sum(slop_fee_minor), 0)::bigint as slop_fee_minor,
                coalesce(sum(refunded_minor), 0)::bigint as refunded_minor
         from donations where app_id = $1 and status <> 'pending' and status <> 'expired'
         group by currency order by currency",
    )
    .bind(app.id)
    .fetch_all(db)
    .await?;
    let active_server_donations = sqlx::query_scalar(
        "select count(*) from server_donations where app_id = $1 and status = 'active'",
    )
    .bind(app.id)
    .fetch_one(db)
    .await?;
    let reports = sqlx::query_as(
        "select id, reference, reason, details, status, created_at from reports
         where subject_kind = 'app' and subject_id = $1 order by created_at desc limit 50",
    )
    .bind(app.id)
    .fetch_all(db)
    .await?;
    let decisions = sqlx::query_as(
        "select id, reference, action, rule, facts, decided_by, appeal_status, created_at, reversed_at
         from moderation_decisions where subject_kind = 'app' and subject_id = $1
         order by created_at desc limit 50",
    )
    .bind(app.id)
    .fetch_all(db)
    .await?;
    let audit = sqlx::query_as(
        "select id, at, actor, action, detail from audit_log where subject = $1 order by id desc limit 100",
    )
    .bind(&slug)
    .fetch_all(db)
    .await?;
    Ok(Json(Record {
        app,
        releases,
        donations,
        active_server_donations,
        reports,
        decisions,
        audit,
    }))
}

/// (app, maker, title, summary, description, age, content labels)
type ListingNow = (Uuid, Uuid, String, String, String, i16, Vec<String>);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListingEdit {
    title: Option<String>,
    summary: Option<String>,
    description_md: Option<String>,
    min_age: Option<i16>,
    content_labels: Option<Vec<String>>,
    /// Why, for the maker and the audit log.
    reason: String,
}

/// Staff fix a store page directly: a misleading title, a missing content label, a rule-breaking
/// description. The maker is told what changed and why.
async fn edit_listing(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(slug): Path<String>,
    Json(b): Json<ListingEdit>,
) -> ApiResult<Json<Value>> {
    admin.need(Perm::Moderate)?;
    let reason = b.reason.trim();
    if reason.is_empty() || reason.chars().count() > 2000 {
        return Err(ApiError::invalid(
            "reason",
            "say why, for the maker, in up to 2000 characters",
        ));
    }
    let mut tx = state.db.begin().await?;
    let before: Option<ListingNow> = sqlx::query_as(
        "select id, maker_id, title, summary, description_md, min_age, content_labels from apps
         where slug = $1 for update",
    )
    .bind(&slug)
    .fetch_optional(&mut *tx)
    .await?;
    let (app_id, maker, title, summary, description, min_age, labels) =
        before.ok_or(ApiError::NotFound)?;
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("update apps set updated_at = now()");
    let mut changed = serde_json::Map::new();
    let mut fields: Vec<&str> = Vec::new();
    if let Some(v) = &b.title {
        let v = validate_text("title", v, 80, false)?;
        if v != title {
            changed.insert("title".into(), json!({ "from": title, "to": v }));
            fields.push("title");
            qb.push(", title = ").push_bind(v);
        }
    }
    if let Some(v) = &b.summary {
        let v = validate_text("summary", v, 200, false)?;
        if v != summary {
            changed.insert("summary".into(), json!({ "from": summary, "to": v }));
            fields.push("summary");
            qb.push(", summary = ").push_bind(v);
        }
    }
    if let Some(v) = &b.description_md {
        let v = validate_text("description_md", v, 20_000, true)?;
        if v != description {
            changed.insert(
                "description_md".into(),
                json!({ "from": description, "to": v }),
            );
            fields.push("description");
            qb.push(", description_md = ").push_bind(v);
        }
    }
    if let Some(v) = b.min_age {
        let v = validate_min_age(v)?;
        if v != min_age {
            changed.insert("min_age".into(), json!({ "from": min_age, "to": v }));
            fields.push("age rating");
            qb.push(", min_age = ").push_bind(v);
        }
    }
    if let Some(v) = &b.content_labels {
        let v = validate_content_labels(v)?;
        if v != labels {
            changed.insert("content_labels".into(), json!({ "from": labels, "to": v }));
            fields.push("content labels");
            qb.push(", content_labels = ").push_bind(v);
        }
    }
    if fields.is_empty() {
        return Err(ApiError::invalid("title", "nothing changed"));
    }
    qb.push(" where id = ").push_bind(app_id);
    qb.build().execute(&mut *tx).await?;
    let now_title: String = sqlx::query_scalar("select title from apps where id = $1")
        .bind(app_id)
        .fetch_one(&mut *tx)
        .await?;
    notify::send(
        &mut tx,
        Notice {
            user_id: maker,
            category: "apps",
            title: format!("Slop changed {now_title}'s store page"),
            body: format!("Changed: {}.\n\n{reason}", fields.join(", ")),
            link: format!("/app/{slug}"),
            collapse_key: None,
        },
    )
    .await?;
    admin::audit(
        &mut tx,
        &admin.actor,
        "app.listing_edited",
        &slug,
        json!({ "changed": changed, "reason": reason }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(Json(json!({ "changed": fields })))
}

// ---------------------------------------------------------------------------------------------
// Money

#[derive(Serialize, sqlx::FromRow)]
struct MonthRow {
    month: String,
    kind: String,
    currency: String,
    payments: i64,
    amount_minor: i64,
    slop_fee_minor: i64,
}

#[derive(Serialize, sqlx::FromRow)]
struct PaymentRow {
    id: Uuid,
    kind: String,
    amount_minor: i32,
    slop_fee_minor: i32,
    currency: String,
    stripe_account_id: Option<String>,
    stripe_object_id: Option<String>,
    simulated: bool,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

#[derive(Serialize)]
struct Money {
    /// The last 12 months: payments and Slop's fees, by kind and currency.
    months: Vec<MonthRow>,
    /// The newest 100 entries in the payments record.
    recent: Vec<PaymentRow>,
    active_server_donations: i64,
    open_holds: i64,
    hold_requests_waiting: i64,
    /// Webhook events received in the last day: Stripe is talking to Slop.
    stripe_events_today: i64,
    /// stripe, simulated or off.
    payments: &'static str,
}

/// The money overview: finance staff only (payment records name Stripe accounts).
async fn money(State(state): State<AppState>, admin: AdminUser) -> ApiResult<Json<Money>> {
    admin.need(Perm::Finance)?;
    let db = &state.db;
    let months = sqlx::query_as(
        "select to_char(date_trunc('month', created_at), 'YYYY-MM') as month, kind, currency,
                count(*) as payments, coalesce(sum(amount_minor), 0)::bigint as amount_minor,
                coalesce(sum(slop_fee_minor), 0)::bigint as slop_fee_minor
         from payments where created_at > date_trunc('month', now()) - interval '11 months'
         group by 1, 2, 3 order by 1 desc, 2, 3",
    )
    .fetch_all(db)
    .await?;
    let recent = sqlx::query_as(
        "select id, kind, amount_minor, slop_fee_minor, currency, stripe_account_id, stripe_object_id, simulated, created_at
         from payments order by created_at desc limit 100",
    )
    .fetch_all(db)
    .await?;
    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        "select (select count(*) from server_donations where status = 'active'),
                (select count(*) from donation_holds where lifted_at is null),
                (select count(*) from donation_hold_requests where outcome is null),
                (select count(*) from stripe_events where received_at > now() - interval '1 day')",
    )
    .fetch_one(db)
    .await?;
    Ok(Json(Money {
        months,
        recent,
        active_server_donations: counts.0,
        open_holds: counts.1,
        hold_requests_waiting: counts.2,
        stripe_events_today: counts.3,
        payments: crate::people::stripe::mode(&state).name(),
    }))
}
