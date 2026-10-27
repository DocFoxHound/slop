// SPDX-License-Identifier: AGPL-3.0-or-later
//! Requests from police, courts and authorities (LEGAL-GUIDANCE CHAT-3), and the yearly
//! transparency report (CHAT-4).
//!
//! Staff with sensitive rights log each request as it arrives: who sent it, what kind of order,
//! what it asks, the deadline (EU production orders: 10 days, emergencies 8 hours, terrorist
//! content removal orders 1 hour). Admins are alerted as a deadline nears (ops.rs). When it's
//! answered, the log keeps what SLAPorSLOP gave, or why not, and whether the person was told; they are,
//! unless a law or the order forbids it. SLAPorSLOP can only give what it has: account details, the
//! metadata it keeps, reported messages. Chat is end-to-end encrypted (SECURITY §8.5).
//!
//! The transparency report (`/v1/transparency/{year}`, public) counts requests by country, kind
//! and outcome, and the year's reports, decisions and appeals.
//!
//! **Terrorist content removal orders** (Regulation (EU) 2021/784, CON-11) have an hour, so they
//! take one step (`removal_order`): log the order, take the content down as a moderation
//! decision (the person gets the statement of reasons and can appeal), keep a sealed copy for six
//! months (Art. 6), and record how long it took. No second approval: the hour is the law's.

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    admin::audit,
    admin_web::AdminUser,
    error::{ApiError, ApiResult},
    staff::Perm,
};

pub const KINDS: &[&str] = &[
    "subpoena",
    "court_order",
    "warrant",
    "eu_production_order",
    "preservation",
    "emergency",
    "uk_daa",
    "removal_order",
    "other",
];
const STATUSES: &[&str] = &["answered", "partly_answered", "refused", "withdrawn"];
const NOTICES: &[&str] = &["notified", "barred", "not_needed"];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/legal-requests", get(list).post(create))
        .route("/admin/legal-requests/{id}/answer", post(answer))
        .route("/admin/removal-orders", post(removal_order))
        .route("/transparency/{year}", get(transparency))
}

#[derive(Serialize, sqlx::FromRow)]
struct Request {
    id: Uuid,
    reference: String,
    #[serde(with = "time::serde::rfc3339")]
    received_at: OffsetDateTime,
    country: String,
    authority: String,
    kind: String,
    legal_basis: String,
    asked: String,
    subject_user_id: Option<Uuid>,
    subject_name: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    deadline: Option<OffsetDateTime>,
    status: String,
    response: Option<String>,
    user_notice: String,
    created_by: String,
    answered_by: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    answered_at: Option<OffsetDateTime>,
}

const COLUMNS: &str = "r.id, r.reference, r.received_at, r.country, r.authority, r.kind, r.legal_basis, r.asked,
    r.subject_user_id, u.display_name as subject_name, r.deadline, r.status, r.response, r.user_notice,
    r.created_by, r.answered_by, r.answered_at";

/// Open ones first, by deadline; then the rest, newest first.
async fn list(State(state): State<AppState>, admin: AdminUser) -> ApiResult<Json<Vec<Request>>> {
    admin.need(Perm::Sensitive)?;
    let rows = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {COLUMNS} from legal_requests r left join users u on u.id = r.subject_user_id
         order by r.status <> 'open', r.deadline nulls last, r.received_at desc limit 500"
    )))
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewRequest {
    #[serde(with = "time::serde::rfc3339")]
    received_at: OffsetDateTime,
    country: String,
    authority: String,
    kind: String,
    #[serde(default)]
    legal_basis: String,
    asked: String,
    subject_user_id: Option<Uuid>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    deadline: Option<OffsetDateTime>,
}

fn text(field: &'static str, v: &str, max: usize, empty_ok: bool) -> ApiResult<String> {
    let v = v.trim();
    if (!empty_ok && v.is_empty()) || v.chars().count() > max {
        return Err(ApiError::invalid(field, format!("1 to {max} characters")));
    }
    Ok(v.to_string())
}

async fn create(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(b): Json<NewRequest>,
) -> ApiResult<Json<Value>> {
    admin.need(Perm::Sensitive)?;
    let country = b.country.trim().to_ascii_uppercase();
    if country.len() != 2 || !country.bytes().all(|c| c.is_ascii_uppercase()) {
        return Err(ApiError::invalid("country", "a two-letter country code"));
    }
    if !KINDS.contains(&b.kind.as_str()) {
        return Err(ApiError::invalid("kind", KINDS.join(", ")));
    }
    let authority = text("authority", &b.authority, 200, false)?;
    let legal_basis = text("legal_basis", &b.legal_basis, 500, true)?;
    let asked = text("asked", &b.asked, 4000, false)?;
    // A removal order has an hour, an emergency eight, a production order ten days.
    let deadline = b.deadline.or(match b.kind.as_str() {
        "removal_order" => Some(b.received_at + time::Duration::hours(1)),
        "emergency" => Some(b.received_at + time::Duration::hours(8)),
        "eu_production_order" => Some(b.received_at + time::Duration::days(10)),
        _ => None,
    });
    let id = Uuid::now_v7();
    // The ID's random end: its start is the time, which two requests can share.
    let reference = format!("LR-{}", id.simple().to_string()[22..].to_ascii_uppercase());
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "insert into legal_requests (id, reference, received_at, country, authority, kind, legal_basis, asked,
                                     subject_user_id, deadline, user_notice, created_by)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(id)
    .bind(&reference)
    .bind(b.received_at)
    .bind(&country)
    .bind(&authority)
    .bind(&b.kind)
    .bind(&legal_basis)
    .bind(&asked)
    .bind(b.subject_user_id)
    .bind(deadline)
    .bind(if b.subject_user_id.is_some() { "pending" } else { "not_needed" })
    .bind(&admin.actor)
    .execute(&mut *tx)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(d) if d.code().as_deref() == Some("23503") => {
            ApiError::invalid("subject_user_id", "no such account")
        }
        _ => e.into(),
    })?;
    audit(
        &mut tx,
        &admin.actor,
        "legal_request.logged",
        &reference,
        json!({ "kind": b.kind, "country": country }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(Json(
        json!({ "id": id, "reference": reference, "deadline": deadline.map(|d| d.to_string()) }),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Answer {
    status: String,
    response: String,
    user_notice: String,
}

async fn answer(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(b): Json<Answer>,
) -> ApiResult<Json<Value>> {
    admin.need(Perm::Sensitive)?;
    if !STATUSES.contains(&b.status.as_str()) {
        return Err(ApiError::invalid("status", STATUSES.join(", ")));
    }
    if !NOTICES.contains(&b.user_notice.as_str()) {
        return Err(ApiError::invalid("user_notice", NOTICES.join(", ")));
    }
    let response = text("response", &b.response, 4000, false)?;
    let mut tx = state.db.begin().await?;
    let reference: Option<String> = sqlx::query_scalar(
        "update legal_requests set status = $2, response = $3, user_notice = $4, answered_by = $5, answered_at = now()
         where id = $1 and status = 'open' returning reference",
    )
    .bind(id)
    .bind(&b.status)
    .bind(&response)
    .bind(&b.user_notice)
    .bind(&admin.actor)
    .fetch_optional(&mut *tx)
    .await?;
    let reference =
        reference.ok_or_else(|| ApiError::Conflict("that request isn't open".into()))?;
    audit(
        &mut tx,
        &admin.actor,
        "legal_request.answered",
        &reference,
        json!({ "status": b.status, "user_notice": b.user_notice }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(Json(json!({ "reference": reference })))
}

/// Open requests whose deadline is within a day (or past), for the admins' alerts (ops.rs).
pub async fn due_soon(db: &sqlx::PgPool) -> sqlx::Result<Vec<(String, String, OffsetDateTime)>> {
    sqlx::query_as(
        "select reference, kind, deadline from legal_requests
         where status = 'open' and deadline < now() + interval '1 day' order by deadline",
    )
    .fetch_all(db)
    .await
}

#[derive(Serialize)]
struct Count {
    key: String,
    count: i64,
}

async fn counts(db: &sqlx::PgPool, sql: &'static str, year: i32) -> sqlx::Result<Vec<Count>> {
    let rows: Vec<(String, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(year)
        .fetch_all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(key, count)| Count { key, count })
        .collect())
}

/// The transparency report for a year (CHAT-4): counts only, never who.
async fn transparency(
    State(state): State<AppState>,
    Path(year): Path<i32>,
) -> ApiResult<Json<Value>> {
    let now = OffsetDateTime::now_utc().year();
    if !(2026..=now).contains(&year) {
        return Err(ApiError::NotFound);
    }
    let db = &state.db;
    Ok(Json(json!({
        "year": year,
        "complete": year < now,
        "legal_requests": {
            "by_country": counts(db, "select country, count(*) from legal_requests where extract(year from received_at at time zone 'UTC') = $1 group by 1 order by 2 desc, 1", year).await?,
            "by_kind": counts(db, "select kind, count(*) from legal_requests where extract(year from received_at at time zone 'UTC') = $1 group by 1 order by 2 desc, 1", year).await?,
            "by_outcome": counts(db, "select status, count(*) from legal_requests where extract(year from received_at at time zone 'UTC') = $1 group by 1 order by 2 desc, 1", year).await?,
        },
        "reports": {
            "by_reason": counts(db, "select reason, count(*) from reports where extract(year from created_at at time zone 'UTC') = $1 group by 1 order by 2 desc, 1", year).await?,
            "by_outcome": counts(db, "select status, count(*) from reports where extract(year from created_at at time zone 'UTC') = $1 group by 1 order by 2 desc, 1", year).await?,
            "median_hours_to_decide": sqlx::query_scalar::<_, Option<f64>>(
                "select percentile_cont(0.5) within group (order by extract(epoch from decided_at - created_at) / 3600)
                 from reports where decided_at is not null and extract(year from created_at at time zone 'UTC') = $1",
            )
            .bind(year)
            .fetch_one(db)
            .await?
            .map(|h| (h * 10.0).round() / 10.0),
        },
        "decisions": {
            "by_action": counts(db, "select action, count(*) from moderation_decisions where extract(year from created_at at time zone 'UTC') = $1 group by 1 order by 2 desc, 1", year).await?,
            "automated": sqlx::query_scalar::<_, i64>("select count(*) from moderation_decisions where automated and extract(year from created_at at time zone 'UTC') = $1").bind(year).fetch_one(db).await?,
        },
        "appeals": {
            "by_outcome": counts(db, "select appeal_status, count(*) from moderation_decisions where appealed_at is not null and extract(year from appealed_at at time zone 'UTC') = $1 group by 1 order by 2 desc, 1", year).await?,
        },
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemovalOrder {
    #[serde(with = "time::serde::rfc3339")]
    received_at: OffsetDateTime,
    country: String,
    authority: String,
    /// The order's own reference and what it names.
    order: String,
    /// app (its address), app_review (its ID), profile (the creative's handle), team (its address).
    subject_kind: String,
    subject: String,
}

/// Takes content down on a removal order, in one step (see the module comment).
async fn removal_order(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(b): Json<RemovalOrder>,
) -> ApiResult<Json<Value>> {
    admin.need(Perm::Sensitive)?;
    let country = b.country.trim().to_ascii_uppercase();
    if country.len() != 2 || !country.bytes().all(|c| c.is_ascii_uppercase()) {
        return Err(ApiError::invalid("country", "a two-letter country code"));
    }
    let authority = text("authority", &b.authority, 200, false)?;
    let order = text("order", &b.order, 4000, false)?;
    let subject = b.subject.trim();
    // What it is, whose, what to call it, the action, and the content as it stands.
    let found: Option<(Uuid, Uuid, String, Value)> = match b.subject_kind.as_str() {
        "app" => sqlx::query_as(
            "select a.id, a.maker_id, 'App: ' || a.title,
                    jsonb_build_object('app', to_jsonb(a), 'pictures', (select to_jsonb(ms) from media_sets ms where ms.id = a.media_set_id))
             from apps a where a.slug = $1",
        )
        .bind(subject)
        .fetch_optional(&state.db)
        .await?,
        "app_review" => sqlx::query_as(
            "select r.id, r.user_id, 'Review of ' || a.title, to_jsonb(r)
             from app_reviews r join apps a on a.id = r.app_id where r.id::text = $1",
        )
        .bind(subject)
        .fetch_optional(&state.db)
        .await?,
        "profile" => sqlx::query_as(
            "select c.user_id, c.user_id, 'People profile: ' || c.name, to_jsonb(c) from creatives c where c.handle = lower($1)",
        )
        .bind(subject)
        .fetch_optional(&state.db)
        .await?,
        "team" => sqlx::query_as(
            "select t.id, t.created_by, 'Team: ' || t.name, to_jsonb(t) from teams t where t.slug = $1",
        )
        .bind(subject)
        .fetch_optional(&state.db)
        .await?,
        _ => return Err(ApiError::invalid("subject_kind", "app, app_review, profile or team")),
    };
    let (subject_id, owner, label, snapshot) =
        found.ok_or_else(|| ApiError::invalid("subject", "nothing on SLAPorSLOP by that name"))?;
    let action = match b.subject_kind.as_str() {
        "app" => "delist_app",
        "app_review" => "hide_review",
        "profile" => "hide_profile",
        _ => "hide_team",
    };
    let request_id = Uuid::now_v7();
    let reference = format!(
        "LR-{}",
        request_id.simple().to_string()[22..].to_ascii_uppercase()
    );
    let mut tx = state.db.begin().await?;
    let law = format!(
        "Regulation (EU) 2021/784 (terrorist content online): removal order from {authority}, {country}"
    );
    let decision = crate::moderation::decide(
        &mut tx,
        &crate::moderation::NewDecision {
            user_id: owner,
            subject_kind: &b.subject_kind,
            subject_id,
            subject_label: &label,
            action,
            rule: "illegal",
            law: Some(&law),
            facts: &format!("A removal order from a competent authority named this content as terrorist content: {order}"),
            decided_by: &admin.actor,
        },
    )
    .await
    .map_err(|e| ApiError::Conflict(format!("{e:#}")))?.id;
    let removed_at = OffsetDateTime::now_utc();
    let minutes = ((removed_at - b.received_at).whole_seconds().max(0) + 59) / 60;
    sqlx::query(
        "insert into legal_requests (id, reference, received_at, country, authority, kind, legal_basis, asked,
                                     subject_user_id, deadline, status, response, user_notice, created_by,
                                     answered_by, answered_at)
         values ($1, $2, $3, $4, $5, 'removal_order', 'Regulation (EU) 2021/784 Art. 3', $6, $7, $8, 'answered', $9,
                 'notified', $10, $10, $11)",
    )
    .bind(request_id)
    .bind(&reference)
    .bind(b.received_at)
    .bind(&country)
    .bind(&authority)
    .bind(&order)
    .bind(owner)
    .bind(b.received_at + time::Duration::hours(1))
    .bind(format!("Removed {label} {minutes} minutes after the order arrived."))
    .bind(&admin.actor)
    .bind(removed_at)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "insert into removed_content (id, legal_request_id, decision_id, subject_kind, subject_id, snapshot, preserve_until)
         values ($1, $2, $3, $4, $5, $6, now() + interval '6 months')",
    )
    .bind(Uuid::now_v7())
    .bind(request_id)
    .bind(decision)
    .bind(&b.subject_kind)
    .bind(subject_id)
    .bind(&snapshot)
    .execute(&mut *tx)
    .await?;
    audit(
        &mut tx,
        &admin.actor,
        "legal_request.removal_order",
        &reference,
        json!({ "subject": label, "minutes": minutes }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(Json(
        json!({ "reference": reference, "minutes": minutes, "label": label }),
    ))
}
