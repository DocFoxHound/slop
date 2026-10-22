// SPDX-License-Identifier: AGPL-3.0-or-later
//! Donation holds: staff stop donations to one app (DESIGN §12, PAYMENTS §5.5).
//!
//! When a complaint says an app shouldn't be making money (someone else's work, a trademark, a
//! law), Slop can stop the money without taking the app down:
//!
//! - **Paused for review** (`review`): quick and reversible, while Slop looks into it. A
//!   moderator or finance staff member places it on their own word.
//! - **Turned off** (`off`): Slop found a problem the maker has to fix. A sensitive action, so it
//!   goes through two-person approval (staff.rs).
//!
//! Either way the app stays in the store, new donations stop (also its share of donations to
//! community builds), monthly server donations are paused at Stripe (no charge while held, not
//! cancelled), and the maker is told the reason. Donors see only "Donations to this app are
//! paused". When the maker has fixed it they ask for a review here, with a message; staff lift
//! the hold or keep it and say why. Every step is in the audit log.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState, admin,
    admin_web::AdminUser,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
    notify::{self, Notice},
    people::stripe::{self, Mode, call},
    staff::Perm,
};

/// A maker can ask again this long after their last request was answered.
const ASK_AGAIN_AFTER: time::Duration = time::Duration::hours(24);

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/donation-holds", get(list))
        .route("/admin/apps/{slug}/donation-hold", post(place))
        .route("/admin/donation-holds/{id}/lift", post(lift_route))
        .route("/admin/donation-hold-requests/{id}/answer", post(answer))
        .route("/apps/{slug}/donations/hold", get(mine))
        .route("/apps/{slug}/donations/hold/request", post(ask))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Review,
    Off,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Review => "review",
            Self::Off => "off",
        }
    }

    fn words(self) -> &'static str {
        match self {
            Self::Review => "paused while Slop reviews it",
            Self::Off => "turned off until the problem is fixed",
        }
    }
}

/// Moderators and finance staff look after holds.
fn need_holds(admin: &AdminUser) -> ApiResult<()> {
    admin
        .need(Perm::Moderate)
        .or_else(|_| admin.need(Perm::Finance))
}

#[derive(Serialize, sqlx::FromRow)]
struct Request {
    id: Uuid,
    message: String,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    outcome: Option<String>,
    answer: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    answered_at: Option<OffsetDateTime>,
}

#[derive(Serialize, sqlx::FromRow)]
struct Hold {
    id: Uuid,
    kind: String,
    reason: String,
    ground: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    placed_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    lifted_at: Option<OffsetDateTime>,
    lift_note: Option<String>,
    #[sqlx(skip)]
    requests: Vec<Request>,
}

async fn requests_of(db: &sqlx::PgPool, hold: Uuid) -> sqlx::Result<Vec<Request>> {
    sqlx::query_as(
        "select id, message, created_at, outcome, answer, answered_at from donation_hold_requests
         where hold_id = $1 order by created_at",
    )
    .bind(hold)
    .fetch_all(db)
    .await
}

// ---------------------------------------------------------------------------------------------
// The maker's side

#[derive(Serialize)]
struct Mine {
    /// The hold in force, if any.
    open: Option<Hold>,
    /// When they may ask for a review next (null: now, or nothing to ask about).
    #[serde(with = "time::serde::rfc3339::option")]
    can_ask_at: Option<OffsetDateTime>,
}

async fn mine(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Mine>> {
    let (app_id, _) = crate::donations::my_app(&state, &user, &slug).await?;
    let open: Option<Hold> = sqlx::query_as(
        "select id, kind, reason, ground, placed_at, lifted_at, lift_note from donation_holds
         where app_id = $1 and lifted_at is null",
    )
    .bind(app_id)
    .fetch_optional(&state.db)
    .await?;
    let Some(mut open) = open else {
        return Ok(Json(Mine {
            open: None,
            can_ask_at: None,
        }));
    };
    open.requests = requests_of(&state.db, open.id).await?;
    let can_ask_at = next_ask(&open.requests);
    Ok(Json(Mine {
        open: Some(open),
        can_ask_at,
    }))
}

/// None when the maker may ask now.
fn next_ask(requests: &[Request]) -> Option<OffsetDateTime> {
    let last = requests.last()?;
    match last.answered_at {
        None => Some(OffsetDateTime::now_utc() + ASK_AGAIN_AFTER), // one waiting already
        Some(at) => Some(at + ASK_AGAIN_AFTER).filter(|t| *t > OffsetDateTime::now_utc()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AskBody {
    /// What they changed.
    message: String,
}

async fn ask(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<AskBody>,
) -> ApiResult<StatusCode> {
    let (app_id, title) = crate::donations::my_app(&state, &user, &slug).await?;
    let message = b.message.trim();
    if message.is_empty() || message.chars().count() > 4000 {
        return Err(ApiError::invalid(
            "message",
            "say what you changed, in up to 4000 characters",
        ));
    }
    let hold: Option<Uuid> =
        sqlx::query_scalar("select id from donation_holds where app_id = $1 and lifted_at is null")
            .bind(app_id)
            .fetch_optional(&state.db)
            .await?;
    let Some(hold) = hold else {
        return Err(ApiError::Conflict(
            "donations to this app aren't paused".into(),
        ));
    };
    let requests = requests_of(&state.db, hold).await?;
    if let Some(last) = requests.last() {
        if last.answered_at.is_none() {
            return Err(ApiError::Conflict(
                "your last request is still waiting for an answer".into(),
            ));
        }
        if next_ask(&requests).is_some() {
            return Err(ApiError::Conflict(
                "you can ask again a day after the last answer".into(),
            ));
        }
    }
    let mut tx = state.db.begin().await?;
    sqlx::query("insert into donation_hold_requests (id, hold_id, message) values ($1, $2, $3)")
        .bind(Uuid::now_v7())
        .bind(hold)
        .bind(message)
        .execute(&mut *tx)
        .await?;
    // Tell the staff who can answer.
    let staff: Vec<Uuid> = sqlx::query_scalar(
        "select distinct u.id from users u join staff_roles r on r.user_id = u.id
         where u.is_admin and u.closed_at is null and r.role in ('owner', 'moderator', 'finance')",
    )
    .fetch_all(&mut *tx)
    .await?;
    for id in staff {
        notify::send(
            &mut tx,
            Notice {
                user_id: id,
                category: "ops",
                title: "A maker asks for donations back".into(),
                body: format!("{title}: the maker says they've fixed the problem."),
                link: "/admin?tab=money".into(),
                collapse_key: Some("donation-hold-requests".into()),
            },
        )
        .await?;
    }
    admin::audit(
        &mut tx,
        &user.display_name,
        "donations.hold_review_asked",
        &slug,
        json!({ "hold": hold }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------------------------
// Staff

#[derive(Deserialize)]
struct ListQuery {
    /// open (default) or all.
    show: Option<String>,
}

#[derive(Serialize, sqlx::FromRow)]
struct Row {
    id: Uuid,
    slug: String,
    title: String,
    maker: String,
    kind: String,
    reason: String,
    ground: Option<String>,
    placed_by: String,
    #[serde(with = "time::serde::rfc3339")]
    placed_at: OffsetDateTime,
    lifted_by: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    lifted_at: Option<OffsetDateTime>,
    lift_note: Option<String>,
    active_server_donations: i64,
    #[sqlx(skip)]
    requests: Vec<Request>,
}

async fn list(
    State(state): State<AppState>,
    admin: AdminUser,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<Row>>> {
    admin.need(Perm::Read)?;
    let all = q.show.as_deref() == Some("all");
    let mut rows: Vec<Row> = sqlx::query_as(
        "select h.id, a.slug, a.title, u.display_name as maker, h.kind, h.reason, h.ground, h.placed_by,
                h.placed_at, h.lifted_by, h.lifted_at, h.lift_note,
                (select count(*) from server_donations s where s.app_id = a.id and s.status = 'active') as active_server_donations
         from donation_holds h join apps a on a.id = h.app_id join users u on u.id = a.maker_id
         where $1 or h.lifted_at is null
         -- Waiting requests first, then newest.
         order by exists (select 1 from donation_hold_requests r where r.hold_id = h.id and r.outcome is null) desc,
                  h.placed_at desc
         limit 500",
    )
    .bind(all)
    .fetch_all(&state.db)
    .await?;
    for r in &mut rows {
        r.requests = requests_of(&state.db, r.id).await?;
    }
    Ok(Json(rows))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaceBody {
    kind: Kind,
    /// What's wrong and what would fix it; the maker reads this.
    reason: String,
    /// A Content Policy rule, law or complaint reference.
    ground: Option<String>,
}

fn check_words(b: &PlaceBody) -> ApiResult<()> {
    let reason = b.reason.trim();
    if reason.is_empty() || reason.chars().count() > 4000 {
        return Err(ApiError::invalid(
            "reason",
            "tell the maker what's wrong and what would fix it (up to 4000 characters)",
        ));
    }
    if b.ground.as_deref().is_some_and(|g| g.chars().count() > 300) {
        return Err(ApiError::invalid("ground", "up to 300 characters"));
    }
    Ok(())
}

async fn place(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(slug): Path<String>,
    Json(b): Json<PlaceBody>,
) -> ApiResult<axum::response::Response> {
    use axum::response::IntoResponse;
    check_words(&b)?;
    let ground = b.ground.as_deref().map(str::trim).filter(|g| !g.is_empty());
    match b.kind {
        Kind::Review => {
            need_holds(&admin)?;
            let v = apply(
                &state,
                &slug,
                Kind::Review,
                b.reason.trim(),
                ground,
                &admin.actor,
            )
            .await?;
            Ok(Json(v).into_response())
        }
        Kind::Off => {
            admin.need(Perm::Sensitive)?;
            let title: String = sqlx::query_scalar("select title from apps where slug = $1")
                .bind(&slug)
                .fetch_optional(&state.db)
                .await?
                .ok_or(ApiError::NotFound)?;
            Ok(crate::staff::request(
                &state,
                &admin,
                "donations.off",
                &format!("Turn off donations to {title}"),
                json!({ "slug": slug, "reason": b.reason.trim(), "ground": ground }),
            )
            .await?
            .into_response())
        }
    }
}

#[derive(Deserialize)]
struct OffPayload {
    slug: String,
    reason: String,
    ground: Option<String>,
}

/// staff.rs runs an approved "donations.off" here.
pub async fn off_stored(state: &AppState, payload: &Value, actor: &str) -> ApiResult<Value> {
    let p: OffPayload = serde_json::from_value(payload.clone())
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("stored action is damaged: {e}")))?;
    apply(
        state,
        &p.slug,
        Kind::Off,
        &p.reason,
        p.ground.as_deref(),
        actor,
    )
    .await
}

/// Places a hold, or changes the one in force (its kind or reason).
async fn apply(
    state: &AppState,
    slug: &str,
    kind: Kind,
    reason: &str,
    ground: Option<&str>,
    actor: &str,
) -> ApiResult<Value> {
    let (app_id, title, maker): (Uuid, String, Uuid) =
        sqlx::query_as("select id, title, maker_id from apps where slug = $1")
            .bind(slug)
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::NotFound)?;
    let existing: Option<(Uuid, String)> = sqlx::query_as(
        "select id, kind from donation_holds where app_id = $1 and lifted_at is null",
    )
    .bind(app_id)
    .fetch_optional(&state.db)
    .await?;
    // A new hold stops monthly charges at Stripe first; if Stripe refuses, nothing changes here.
    let donors = if existing.is_none() {
        let donors = set_collection(state, app_id, true).await?;
        expire_open_checkouts(state, app_id).await;
        donors
    } else {
        Vec::new()
    };
    let mut tx = state.db.begin().await?;
    let id = match &existing {
        Some((id, _)) => {
            sqlx::query(
                "update donation_holds set kind = $2, reason = $3, ground = $4 where id = $1",
            )
            .bind(id)
            .bind(kind.name())
            .bind(reason)
            .bind(ground)
            .execute(&mut *tx)
            .await?;
            *id
        }
        None => {
            let id = Uuid::now_v7();
            sqlx::query(
                "insert into donation_holds (id, app_id, kind, reason, ground, placed_by) values ($1, $2, $3, $4, $5, $6)",
            )
            .bind(id)
            .bind(app_id)
            .bind(kind.name())
            .bind(reason)
            .bind(ground)
            .bind(actor)
            .execute(&mut *tx)
            .await?;
            id
        }
    };
    let changed = existing.is_some();
    notify::send(
        &mut tx,
        Notice {
            user_id: maker,
            category: "donations",
            title: format!("Donations to {title} are {}", kind.words()),
            body: format!(
                "{reason}\n\nYour app stays in the store. When you've fixed this, ask for a review on the app's Donations page."
            ),
            link: format!("/publish/{slug}/manage#donations"),
            collapse_key: Some(format!("donation-hold:{id}")),
        },
    )
    .await?;
    for donor in &donors {
        notify::send(
            &mut tx,
            Notice {
                user_id: *donor,
                category: "donations",
                title: format!("Your server donation to {title} is paused"),
                body: format!(
                    "Donations to {title} are paused, so you won't be charged until they restart. You can cancel any time from your donations page."
                ),
                link: "/account#donations".into(),
                collapse_key: None,
            },
        )
        .await?;
    }
    admin::audit(
        &mut tx,
        actor,
        if changed {
            "donations.hold_changed"
        } else {
            "donations.hold_placed"
        },
        slug,
        json!({ "hold": id, "kind": kind.name(), "reason": reason, "ground": ground, "server_donations_paused": donors.len() }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(json!({ "hold": id }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LiftBody {
    /// Why, for the maker and the record.
    note: String,
}

async fn lift_route(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(b): Json<LiftBody>,
) -> ApiResult<StatusCode> {
    need_lift(&state, &admin, id).await?;
    let note = b.note.trim();
    if note.is_empty() || note.chars().count() > 4000 {
        return Err(ApiError::invalid(
            "note",
            "say why, in up to 4000 characters",
        ));
    }
    lift(&state, id, &admin.actor, note).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Lifting a pause for review is a moderator's or finance's call; lifting "turned off" is a
/// sensitive one.
async fn need_lift(state: &AppState, admin: &AdminUser, hold: Uuid) -> ApiResult<()> {
    let kind: Option<String> =
        sqlx::query_scalar("select kind from donation_holds where id = $1 and lifted_at is null")
            .bind(hold)
            .fetch_optional(&state.db)
            .await?;
    match kind.as_deref() {
        None => Err(ApiError::NotFound),
        Some("off") => admin.need(Perm::Sensitive),
        Some(_) => need_holds(admin),
    }
}

async fn lift(state: &AppState, hold: Uuid, actor: &str, note: &str) -> ApiResult<()> {
    let (app_id, slug, title, maker): (Uuid, String, String, Uuid) = sqlx::query_as(
        "select a.id, a.slug, a.title, a.maker_id from donation_holds h join apps a on a.id = h.app_id
         where h.id = $1 and h.lifted_at is null",
    )
    .bind(hold)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    let donors = set_collection(state, app_id, false).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "update donation_holds set lifted_at = now(), lifted_by = $2, lift_note = $3 where id = $1",
    )
    .bind(hold)
    .bind(actor)
    .bind(note)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "update donation_hold_requests set outcome = 'lifted', answer = $2, answered_by = $3, answered_at = now()
         where hold_id = $1 and outcome is null",
    )
    .bind(hold)
    .bind(note)
    .bind(actor)
    .execute(&mut *tx)
    .await?;
    // Simulated monthly donations skipped their charges while held; the next is a month on.
    sqlx::query(
        "update server_donations set next_charge_at = now() + interval '30 days'
         where app_id = $1 and simulated and status = 'active' and next_charge_at < now()",
    )
    .bind(app_id)
    .execute(&mut *tx)
    .await?;
    notify::send(
        &mut tx,
        Notice {
            user_id: maker,
            category: "donations",
            title: format!("Donations to {title} are back on"),
            body: note.to_string(),
            link: format!("/publish/{slug}/manage#donations"),
            collapse_key: Some(format!("donation-hold:{hold}")),
        },
    )
    .await?;
    for donor in &donors {
        notify::send(
            &mut tx,
            Notice {
                user_id: *donor,
                category: "donations",
                title: format!("Your server donation to {title} has restarted"),
                body: format!(
                    "Donations to {title} are back on, so your monthly server donation continues from its next date. You can cancel any time from your donations page."
                ),
                link: "/account#donations".into(),
                collapse_key: None,
            },
        )
        .await?;
    }
    admin::audit(
        &mut tx,
        actor,
        "donations.hold_lifted",
        &slug,
        json!({ "hold": hold, "note": note }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswerBody {
    /// true: lift the hold. false: keep it, and `answer` says what's still wrong.
    lift: bool,
    answer: String,
}

async fn answer(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(b): Json<AnswerBody>,
) -> ApiResult<StatusCode> {
    let text = b.answer.trim();
    if text.is_empty() || text.chars().count() > 4000 {
        return Err(ApiError::invalid(
            "answer",
            "tell the maker why, in up to 4000 characters",
        ));
    }
    let hold: Uuid = sqlx::query_scalar(
        "select hold_id from donation_hold_requests where id = $1 and outcome is null",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    if b.lift {
        need_lift(&state, &admin, hold).await?;
        return lift(&state, hold, &admin.actor, text)
            .await
            .map(|()| StatusCode::NO_CONTENT);
    }
    need_holds(&admin)?;
    let (slug, title, maker): (String, String, Uuid) = sqlx::query_as(
        "select a.slug, a.title, a.maker_id from donation_holds h join apps a on a.id = h.app_id where h.id = $1",
    )
    .bind(hold)
    .fetch_one(&state.db)
    .await?;
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "update donation_hold_requests set outcome = 'kept', answer = $2, answered_by = $3, answered_at = now()
         where id = $1",
    )
    .bind(id)
    .bind(text)
    .bind(&admin.actor)
    .execute(&mut *tx)
    .await?;
    notify::send(
        &mut tx,
        Notice {
            user_id: maker,
            category: "donations",
            title: format!("Donations to {title} stay paused for now"),
            body: format!("{text}\n\nYou can ask again once you've made the changes."),
            link: format!("/publish/{slug}/manage#donations"),
            collapse_key: Some(format!("donation-hold:{hold}")),
        },
    )
    .await?;
    admin::audit(
        &mut tx,
        &admin.actor,
        "donations.hold_kept",
        &slug,
        json!({ "hold": hold, "request": id, "answer": text }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Pauses (or resumes) collection of an app's monthly server donations at Stripe, so donors
/// aren't charged while held. Returns the donors, to tell them.
/// Closes Checkout pages opened before the hold, so nobody finishes paying into a paused app
/// (security review, 2026-10-22). Best effort: one that can't be closed is logged, and stays
/// recorded if paid, for staff to refund.
async fn expire_open_checkouts(state: &AppState, app_id: Uuid) {
    let Mode::Stripe(stripe) = stripe::mode(state) else {
        return;
    };
    let open: Vec<(String, String)> = match sqlx::query_as(
        "select checkout_session_id, stripe_account_id from donations
         where app_id = $1 and status = 'pending' and checkout_session_id is not null and not simulated
         union all
         select checkout_session_id, stripe_account_id from server_donations
         where app_id = $1 and status = 'pending' and checkout_session_id is not null and not simulated",
    )
    .bind(app_id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(error = %err, "open checkouts of a paused app");
            return;
        }
    };
    for (session, acct) in open {
        if let Err(err) = call(
            state,
            stripe,
            reqwest::Method::POST,
            &format!("v1/checkout/sessions/{session}/expire"),
            Some(&acct),
            &[],
        )
        .await
        {
            tracing::warn!(error = %err, %session, "couldn't close a Checkout page of a paused app");
        }
    }
}

async fn set_collection(state: &AppState, app_id: Uuid, pause: bool) -> ApiResult<Vec<Uuid>> {
    let subs: Vec<(Uuid, String, Option<String>, bool)> = sqlx::query_as(
        "select donor_id, stripe_account_id, stripe_subscription_id, simulated from server_donations
         where app_id = $1 and status = 'active'",
    )
    .bind(app_id)
    .fetch_all(&state.db)
    .await?;
    if let Mode::Stripe(stripe) = stripe::mode(state) {
        for (_, acct, sub, simulated) in &subs {
            let (Some(sub), false) = (sub, simulated) else {
                continue;
            };
            let form: &[(&str, String)] = if pause {
                &[("pause_collection[behavior]", "void".to_string())]
            } else {
                &[("pause_collection", String::new())]
            };
            call(
                state,
                stripe,
                reqwest::Method::POST,
                &format!("v1/subscriptions/{sub}"),
                Some(acct),
                form,
            )
            .await
            .map_err(ApiError::Internal)?;
        }
    }
    Ok(subs.into_iter().map(|(donor, ..)| donor).collect())
}
