// SPDX-License-Identifier: AGPL-3.0-or-later
//! Problems with jobs, and disputes about reviews (DESIGN §15.6, LEGAL §4.1, §4.5).
//!
//! **Problems.** Either side opens one, states their case, the other answers, and either
//! proposes outcomes (a refund, cancelling, redoing a part, something else). An outcome both
//! accept is recorded and carried out where SLAPorSLOP can (cancelling; a simulated refund in the
//! sandbox). A SLAPorSLOP mediator can read the job's record and **suggest** an outcome, which binds
//! no one: SLAPorSLOP doesn't decide disputes. Unresolved problems close with the record available to
//! both, for a card dispute or a court.
//!
//! **Review disputes.** A creative can dispute a review that breaks the rules, isn't about the
//! job, is extortion, or states something the job record shows is false. A mediator decides:
//! remove it, keep it with a factual note, or keep it as it is. Never because it's negative
//! (HUM-12, HUM-18); every decision has a written reason (DSA Art. 17).

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;

use super::jobs::{self, JobView, Role, end_job, load, load_any, note, reload, wrong_step};
use crate::{
    AppState, admin,
    admin_web::AdminUser,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/jobs/{id}/problem", post(open_problem))
        .route("/jobs/{id}/problem/reply", post(reply))
        .route("/jobs/{id}/problem/propose", post(propose))
        .route("/jobs/{id}/problem/proposals/{pid}/{answer}", post(answer))
        .route("/jobs/{id}/problem/withdraw", post(withdraw_problem))
        .route("/people/reviews/{job_id}/dispute", post(dispute_review))
        .route("/jobs/{id}/review/dispute-answer", put(answer_dispute))
        .route("/admin/problems/{id}/suggest", post(suggest))
        .route("/admin/problems/{id}/close", post(close))
        .route("/admin/review-disputes/{job_id}/decide", post(decide))
        .route("/admin/jobs/{id}/record", get(admin_record))
}

// ---------------------------------------------------------------------------------------------
// Problems

/// The job's open (or latest) problem, as the viewer sees it.
// Row tuples straight from SQL, read once below.
#[allow(clippy::type_complexity)]
pub async fn problem_view(
    db: &sqlx::PgPool,
    job: Uuid,
    viewer: Uuid,
) -> sqlx::Result<Option<Value>> {
    let row: Option<(Uuid, Uuid, String, Option<String>, String, Option<String>, Option<String>, OffsetDateTime)> =
        sqlx::query_as(
            "select id, opened_by, statement, reply, status, mediator_suggestion, closing_note, created_at
             from job_problems where job_id = $1 order by created_at desc limit 1",
        )
        .bind(job)
        .fetch_optional(db)
        .await?;
    let Some((id, opened_by, statement, reply, status, suggestion, closing, created)) = row else {
        return Ok(None);
    };
    let proposals: Vec<(Uuid, bool, String, Option<i32>, String, String)> = sqlx::query_as(
        "select id, proposed_by = $2, kind, amount_minor, note, status from job_problem_proposals
         where problem_id = $1 order by created_at",
    )
    .bind(id)
    .bind(viewer)
    .fetch_all(db)
    .await?;
    Ok(Some(json!({
        "id": id,
        "opened_by_me": opened_by == viewer,
        "statement": statement,
        "reply": reply,
        "status": status,
        "mediator_suggestion": suggestion,
        "closing_note": closing,
        "created_at": created.format(&time::format_description::well_known::Rfc3339).unwrap_or_default(),
        "proposals": proposals.into_iter().map(|(pid, mine, kind, amount, note, status)| json!({
            "id": pid, "mine": mine, "kind": kind, "amount_minor": amount, "note": note, "status": status,
        })).collect::<Vec<_>>(),
    })))
}

/// For the downloadable record.
// Row tuples straight from SQL, read once below.
#[allow(clippy::type_complexity)]
pub async fn problems_text(db: &sqlx::PgPool, job: Uuid) -> ApiResult<String> {
    let rows: Vec<(
        Uuid,
        String,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        "select id, statement, reply, status, mediator_suggestion, closing_note from job_problems
         where job_id = $1 order by created_at",
    )
    .bind(job)
    .fetch_all(db)
    .await?;
    let mut out = String::new();
    for (id, statement, reply, status, suggestion, closing) in rows {
        out.push_str(&format!(
            "\n## Problem ({status})\n\nOpened with: {statement}\n\n"
        ));
        if let Some(r) = reply {
            out.push_str(&format!("Answer: {r}\n\n"));
        }
        let proposals: Vec<(String, Option<i32>, String, String)> = sqlx::query_as(
            "select kind, amount_minor, note, status from job_problem_proposals where problem_id = $1 order by created_at",
        )
        .bind(id)
        .fetch_all(db)
        .await?;
        for (kind, amount, note, status) in proposals {
            out.push_str(&format!(
                "- Proposal ({status}): {kind}{} — {note}\n",
                amount
                    .map(|a| format!(" {}", f64::from(a) / 100.0))
                    .unwrap_or_default()
            ));
        }
        if let Some(s) = suggestion {
            out.push_str(&format!("\nMediator's suggestion (not binding): {s}\n"));
        }
        if let Some(c) = closing {
            out.push_str(&format!("\nClosed: {c}\n"));
        }
    }
    Ok(out)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Statement {
    statement: String,
}

async fn open_problem(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<Statement>,
) -> ApiResult<Json<JobView>> {
    let (j, _) = load(&state, id, &user).await?;
    if j.offer.is_none() || ["requested", "offered", "declined"].contains(&j.status.as_str()) {
        return Err(wrong_step("open a problem (that's for accepted jobs)"));
    }
    let text = note("statement", &body.statement, 4000, true)?;
    let mut tx = state.db.begin().await?;
    let inserted = sqlx::query(
        "insert into job_problems (id, job_id, opened_by, statement) values ($1, $2, $3, $4) on conflict do nothing",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(user.id)
    .bind(&text)
    .execute(&mut *tx)
    .await?;
    if inserted.rows_affected() == 0 {
        return Err(ApiError::Conflict(
            "a problem is already open on this job".into(),
        ));
    }
    jobs::event(
        &mut tx,
        id,
        Some(user.id),
        "problem_opened",
        Some(&text),
        &[],
        None,
    )
    .await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

async fn open_problem_of(db: &sqlx::PgPool, job: Uuid) -> ApiResult<(Uuid, Uuid)> {
    sqlx::query_as("select id, opened_by from job_problems where job_id = $1 and status = 'open'")
        .bind(job)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| ApiError::Conflict("there's no open problem on this job".into()))
}

async fn reply(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<Statement>,
) -> ApiResult<Json<JobView>> {
    load(&state, id, &user).await?;
    let (problem, opened_by) = open_problem_of(&state.db, id).await?;
    if opened_by == user.id {
        return Err(ApiError::Conflict(
            "the other side answers your statement".into(),
        ));
    }
    let text = note("statement", &body.statement, 4000, true)?;
    let mut tx = state.db.begin().await?;
    sqlx::query("update job_problems set reply = $2, replied_at = now() where id = $1")
        .bind(problem)
        .bind(&text)
        .execute(&mut *tx)
        .await?;
    jobs::event(
        &mut tx,
        id,
        Some(user.id),
        "problem_answered",
        Some(&text),
        &[],
        None,
    )
    .await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    /// refund, cancel, redo or other.
    pub kind: String,
    pub amount_minor: Option<i64>,
    pub note: String,
}

async fn propose(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(p): Json<Proposal>,
) -> ApiResult<Json<JobView>> {
    let (j, _) = load(&state, id, &user).await?;
    let (problem, _) = open_problem_of(&state.db, id).await?;
    if !["refund", "cancel", "redo", "other"].contains(&p.kind.as_str()) {
        return Err(ApiError::invalid("kind", "refund, cancel, redo or other"));
    }
    let text = note("note", &p.note, 1000, true)?;
    let amount = match (p.kind.as_str(), p.amount_minor) {
        ("refund", Some(a)) if a > 0 => {
            // What was paid, less what's been refunded already: refunds Stripe reported (stored
            // as negative amounts), or refunds agreed in earlier problems that the creative may
            // not have made yet, whichever is more.
            let left: i64 = sqlx::query_scalar(
                "select (select coalesce(sum(p.amount_minor), 0) from payments p
                         join job_payments jp on jp.id = p.job_payment_id
                         where jp.job_id = $1 and p.kind = 'people_job')
                      - greatest(
                          (select coalesce(-sum(r.amount_minor), 0) from payments r
                           join payments p on p.id = r.refund_of join job_payments jp on jp.id = p.job_payment_id
                           where jp.job_id = $1 and r.kind = 'refund'),
                          (select coalesce(sum(pp.amount_minor), 0) from job_problem_proposals pp
                           join job_problems pr on pr.id = pp.problem_id
                           where pr.job_id = $1 and pp.kind = 'refund' and pp.status = 'accepted'))",
            )
            .bind(j.id)
            .fetch_one(&state.db)
            .await?;
            if a > left {
                return Err(ApiError::invalid(
                    "amount_minor",
                    "a refund can't be more than what was paid and not yet refunded",
                ));
            }
            Some(a as i32)
        }
        ("refund", _) => return Err(ApiError::invalid("amount_minor", "how much to refund")),
        _ => None,
    };
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "insert into job_problem_proposals (id, problem_id, proposed_by, kind, amount_minor, note) values ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::now_v7())
    .bind(problem)
    .bind(user.id)
    .bind(&p.kind)
    .bind(amount)
    .bind(&text)
    .execute(&mut *tx)
    .await?;
    jobs::event(
        &mut tx,
        id,
        Some(user.id),
        "problem_proposal",
        Some(&format!("{}: {text}", p.kind)),
        &[],
        None,
    )
    .await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

/// Accept, reject or withdraw a proposal. Accepting one the other side made resolves the
/// problem and carries it out where SLAPorSLOP can.
async fn answer(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, pid, answer)): Path<(Uuid, Uuid, String)>,
) -> ApiResult<Json<JobView>> {
    let (j, role) = load(&state, id, &user).await?;
    let (problem, _) = open_problem_of(&state.db, id).await?;
    let row: Option<(Uuid, String, Option<i32>, String)> = sqlx::query_as(
        "select proposed_by, kind, amount_minor, note from job_problem_proposals
         where id = $1 and problem_id = $2 and status = 'pending'",
    )
    .bind(pid)
    .bind(problem)
    .fetch_optional(&state.db)
    .await?;
    let Some((by, kind, amount, text)) = row else {
        return Err(ApiError::Conflict("that proposal isn't open".into()));
    };
    let mine = by == user.id;
    let status = match (answer.as_str(), mine) {
        ("accept", false) => "accepted",
        ("reject", false) => "rejected",
        ("withdraw", true) => "withdrawn",
        _ => {
            return Err(ApiError::Conflict(
                "you can't answer your own proposal that way".into(),
            ));
        }
    };
    let mut tx = state.db.begin().await?;
    sqlx::query("update job_problem_proposals set status = $2, decided_at = now() where id = $1")
        .bind(pid)
        .bind(status)
        .execute(&mut *tx)
        .await?;
    jobs::event(
        &mut tx,
        id,
        Some(user.id),
        &format!("problem_proposal_{status}"),
        Some(&text),
        &[],
        None,
    )
    .await?;
    if status == "accepted" {
        sqlx::query(
            "update job_problems set status = 'resolved', closing_note = $2, closed_at = now() where id = $1",
        )
        .bind(problem)
        .bind(format!("Both agreed: {kind} — {text}"))
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "update job_problem_proposals set status = 'withdrawn', decided_at = now() where problem_id = $1 and status = 'pending'",
        )
        .bind(problem)
        .execute(&mut *tx)
        .await?;
        carry_out(&state, &mut tx, &j, user.id, role, &kind, amount).await?;
    }
    tx.commit().await?;
    reload(&state, id, &user).await
}

async fn carry_out(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    j: &jobs::JobRow,
    by: Uuid,
    _role: Role,
    kind: &str,
    amount: Option<i32>,
) -> ApiResult<()> {
    match kind {
        "cancel" => {
            if [
                "awaiting_payment",
                "in_progress",
                "awaiting_sign_off",
                "delivered",
                "final_files",
            ]
            .contains(&j.status.as_str())
            {
                end_job(
                    tx,
                    j.id,
                    by,
                    "cancelled",
                    Some("Cancelled as agreed in a problem."),
                )
                .await?;
            }
        }
        "redo" => {
            // The agreed part is redone at no charge: back to in progress, nothing due until delivery.
            if ["delivered", "final_files", "completed"].contains(&j.status.as_str()) {
                sqlx::query(
                    "update job_payments set status = 'waiting' where job_id = $1 and due_on = 'delivery' and status = 'due'",
                )
                .bind(j.id)
                .execute(&mut *tx)
                .await?;
                jobs::set_status(
                    tx,
                    j.id,
                    &["delivered", "final_files", "completed"],
                    "in_progress",
                )
                .await?;
                sqlx::query("update jobs set delivered_at = null where id = $1")
                    .bind(j.id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        "refund" => {
            // Refunds are the creative's to issue in Stripe. Without Stripe (sandbox,
            // development), the agreed refund is recorded as simulated.
            if let (super::stripe::Mode::Simulated, Some(a)) = (super::stripe::mode(state), amount)
            {
                jobs::simulated_refund(tx, j.id, a.into())
                    .await
                    .map_err(ApiError::Internal)?;
            }
        }
        _ => {}
    }
    Ok(())
}

async fn withdraw_problem(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<JobView>> {
    load(&state, id, &user).await?;
    let (problem, opened_by) = open_problem_of(&state.db, id).await?;
    if opened_by != user.id {
        return Err(ApiError::Conflict(
            "only whoever opened it can withdraw it".into(),
        ));
    }
    let mut tx = state.db.begin().await?;
    sqlx::query("update job_problems set status = 'withdrawn', closed_at = now() where id = $1")
        .bind(problem)
        .execute(&mut *tx)
        .await?;
    jobs::event(
        &mut tx,
        id,
        Some(user.id),
        "problem_withdrawn",
        None,
        &[],
        None,
    )
    .await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

// ---------------------------------------------------------------------------------------------
// Review disputes

pub const DISPUTE_REASONS: &[(&str, &str)] = &[
    (
        "rules",
        "Breaks the rules: insults, threats or private information",
    ),
    ("not_about_job", "Isn't about the work on this job"),
    (
        "extortion",
        "Was used as a threat (\"do more for free or I'll leave a bad review\")",
    ),
    (
        "false_fact",
        "States something the job record shows is false",
    ),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DisputeBody {
    reason: String,
    statement: String,
}

async fn dispute_review(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(job_id): Path<Uuid>,
    Json(body): Json<DisputeBody>,
) -> ApiResult<StatusCode> {
    let review: Option<(Uuid, OffsetDateTime)> =
        sqlx::query_as("select creative_id, updated_at from creative_reviews where job_id = $1")
            .bind(job_id)
            .fetch_optional(&state.db)
            .await?;
    match review {
        Some((creative, _)) if creative == user.id => {}
        _ => return Err(ApiError::NotFound),
    }
    if !DISPUTE_REASONS.iter().any(|(r, _)| *r == body.reason) {
        return Err(ApiError::invalid("reason", "pick one of the reasons"));
    }
    let text = note("statement", &body.statement, 2000, true)?;
    let mut tx = state.db.begin().await?;
    // One dispute per review; a review changed after a decision can be disputed again.
    let done = sqlx::query(
        "insert into review_disputes (job_id, reason, statement) values ($1, $2, $3)
         on conflict (job_id) do update set reason = $2, statement = $3, client_statement = null, status = 'pending',
             decision_reason = null, decided_by = null, decided_at = null, created_at = now()
         where review_disputes.status <> 'pending'
           and (select updated_at from creative_reviews where job_id = $1) > review_disputes.decided_at",
    )
    .bind(job_id)
    .bind(&body.reason)
    .bind(&text)
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::Conflict(
            "this review has already been disputed; a decided review can be disputed again only if it changes".into(),
        ));
    }
    // The reviewer hears of it, so they can give their side before a mediator decides.
    let (client, creative, title): (Uuid, String, String) = sqlx::query_as(
        "select r.client_id, c.name, coalesce(j.offer->>'title', 'the job for ' || a.title)
         from creative_reviews r join jobs j on j.id = r.job_id join apps a on a.id = j.app_id
         join creatives c on c.user_id = r.creative_id where r.job_id = $1",
    )
    .bind(job_id)
    .fetch_one(&mut *tx)
    .await?;
    crate::notify::send(
        &mut tx,
        crate::notify::Notice {
            user_id: client,
            category: "jobs",
            title: format!("{creative} disputed your review of “{title}”"),
            body: "A mediator decides whether it breaks the rules. You can give your side on the job page."
                .into(),
            link: format!("/jobs/{job_id}"),
            collapse_key: None,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The reviewer's side of a dispute, while it's pending.
async fn answer_dispute(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<Statement>,
) -> ApiResult<Json<JobView>> {
    let (_, role) = load(&state, id, &user).await?;
    if role != Role::Client {
        return Err(ApiError::NotFound);
    }
    let text = note("statement", &body.statement, 2000, true)?;
    let done = sqlx::query(
        "update review_disputes set client_statement = $2 where job_id = $1 and status = 'pending'",
    )
    .bind(id)
    .bind(&text)
    .execute(&state.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::Conflict(
            "there's no pending dispute about your review".into(),
        ));
    }
    reload(&state, id, &user).await
}

// ---------------------------------------------------------------------------------------------
// Mediators (admins and AI reviewers with admin tokens)

#[derive(Serialize)]
pub struct QueuedProblem {
    pub id: Uuid,
    pub job_id: Uuid,
    pub creative: String,
    pub client: String,
    pub title: String,
    pub statement: String,
    pub opened_by_creative: bool,
    pub reply: Option<String>,
    pub mediator_suggestion: Option<String>,
    pub proposals: Value,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub notes: Vec<crate::review::Note>,
}

pub async fn queued_problems(db: &sqlx::PgPool) -> ApiResult<Vec<QueuedProblem>> {
    type Row = (
        Uuid,
        Uuid,
        String,
        String,
        Option<String>,
        String,
        bool,
        Option<String>,
        Option<String>,
        OffsetDateTime,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "select p.id, p.job_id, c.name, u.display_name, j.offer->>'title', p.statement, p.opened_by = j.creative_id,
                p.reply, p.mediator_suggestion, p.created_at
         from job_problems p join jobs j on j.id = p.job_id join creatives c on c.user_id = j.creative_id
         join users u on u.id = j.client_id where p.status = 'open' order by p.created_at",
    )
    .fetch_all(db)
    .await?;
    let mut out = Vec::new();
    for (
        id,
        job_id,
        creative,
        client,
        title,
        statement,
        by_creative,
        reply,
        suggestion,
        created_at,
    ) in rows
    {
        let proposals: Value = sqlx::query_scalar(
            "select coalesce(json_agg(json_build_object('kind', kind, 'amount_minor', amount_minor, 'note', note,
                    'status', status) order by created_at), '[]') from job_problem_proposals where problem_id = $1",
        )
        .bind(id)
        .fetch_one(db)
        .await?;
        out.push(QueuedProblem {
            id,
            job_id,
            creative,
            client,
            title: title.unwrap_or_else(|| "Commissioned work".into()),
            statement,
            opened_by_creative: by_creative,
            reply,
            mediator_suggestion: suggestion,
            proposals,
            created_at,
            notes: crate::review::notes(db, "problem", id).await?,
        });
    }
    Ok(out)
}

#[derive(Serialize)]
pub struct QueuedDispute {
    pub job_id: Uuid,
    pub creative: String,
    pub creative_handle: String,
    pub client: String,
    pub vote: &'static str,
    pub review: String,
    pub reason: String,
    pub reason_text: &'static str,
    pub statement: String,
    pub client_statement: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub notes: Vec<crate::review::Note>,
}

pub async fn queued_disputes(db: &sqlx::PgPool) -> ApiResult<Vec<QueuedDispute>> {
    type Row = (
        Uuid,
        String,
        String,
        String,
        i16,
        String,
        String,
        String,
        Option<String>,
        OffsetDateTime,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "select d.job_id, c.name, c.handle, u.display_name, r.vote, r.body, d.reason, d.statement, d.client_statement, d.created_at
         from review_disputes d join creative_reviews r on r.job_id = d.job_id
         join creatives c on c.user_id = r.creative_id join users u on u.id = r.client_id
         where d.status = 'pending' order by d.created_at",
    )
    .fetch_all(db)
    .await?;
    let mut out = Vec::new();
    for (
        job_id,
        creative,
        handle,
        client,
        vote,
        review,
        reason,
        statement,
        client_statement,
        created_at,
    ) in rows
    {
        out.push(QueuedDispute {
            job_id,
            creative,
            creative_handle: handle,
            client,
            vote: if vote > 0 { "not_slop" } else { "slop" },
            review,
            reason_text: DISPUTE_REASONS
                .iter()
                .find(|(r, _)| *r == reason)
                .map_or("", |(_, t)| t),
            reason,
            statement,
            client_statement,
            created_at,
            notes: crate::review::notes(db, "review_dispute", job_id).await?,
        });
    }
    Ok(out)
}

/// The job record, for a mediator: only while the job has a problem or a review dispute, and
/// every look is written to the audit log (the conversation is private otherwise).
async fn admin_record(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<String> {
    // Private conversations: for the staff who handle problems, and only while one is open
    // (security review, 2026-10-22).
    admin.need(crate::staff::Perm::Moderate)?;
    let reason: Option<String> = sqlx::query_scalar(
        "select case when exists (select 1 from job_problems where job_id = $1 and status = 'open') then 'problem'
                     when exists (select 1 from review_disputes where job_id = $1 and status = 'pending') then 'review dispute' end",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    let Some(reason) = reason else {
        return Err(ApiError::NotFound);
    };
    let j = load_any(&state.db, id).await?.ok_or(ApiError::NotFound)?;
    let mut tx = state.db.begin().await?;
    admin::audit(
        &mut tx,
        &admin.actor,
        "job.record_read",
        &id.to_string(),
        json!({ "because": reason }),
    )
    .await?;
    tx.commit().await?;
    let mut text = jobs::record_text(&state.db, &j).await?;
    let evidence: Vec<(String, i32, i32, String)> = sqlx::query_as(
        "select u.display_name, e.verified_messages, e.unverified_messages, e.transcript from job_evidence e
         join users u on u.id = e.submitted_by where e.job_id = $1 order by e.created_at",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    for (who, good, bad, transcript) in evidence {
        text.push_str(&format!(
            "\n## Conversation copy submitted by {who} ({good} messages match SLAPorSLOP's fingerprints, {bad} don't)\n\n"
        ));
        if let Ok(copy) = serde_json::from_str::<jobs::ConversationCopy>(&transcript) {
            for m in copy.messages {
                text.push_str(&format!(
                    "**{}** · {}\n\n{}\n\n",
                    m.author, m.created_at, m.body
                ));
            }
        }
    }
    Ok(text)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Suggestion {
    suggestion: String,
}

/// A mediator's suggestion: shown to both sides, binds no one.
async fn suggest(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<Suggestion>,
) -> ApiResult<StatusCode> {
    admin.need(crate::staff::Perm::Moderate)?;
    let text = note("suggestion", &body.suggestion, 4000, true)?;
    let mut tx = state.db.begin().await?;
    let job: Option<Uuid> = sqlx::query_scalar(
        "update job_problems set mediator_suggestion = $2, suggested_by = $3, suggested_at = now()
         where id = $1 and status = 'open' returning job_id",
    )
    .bind(id)
    .bind(&text)
    .bind(&admin.actor)
    .fetch_optional(&mut *tx)
    .await?;
    let job = job.ok_or_else(|| ApiError::Conflict("that problem isn't open".into()))?;
    jobs::event(
        &mut tx,
        job,
        None,
        "mediator_suggestion",
        Some(&text),
        &[],
        None,
    )
    .await?;
    admin::audit(
        &mut tx,
        &admin.actor,
        "problem.suggest",
        &job.to_string(),
        json!({ "problem": id }),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseBody {
    note: String,
}

/// Closes a problem the two sides didn't settle. Their record stays available to both.
async fn close(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<CloseBody>,
) -> ApiResult<StatusCode> {
    admin.need(crate::staff::Perm::Moderate)?;
    let text = note("note", &body.note, 2000, true)?;
    let mut tx = state.db.begin().await?;
    let job: Option<Uuid> = sqlx::query_scalar(
        "update job_problems set status = 'unresolved', closing_note = $2, closed_at = now()
         where id = $1 and status = 'open' returning job_id",
    )
    .bind(id)
    .bind(&text)
    .fetch_optional(&mut *tx)
    .await?;
    let job = job.ok_or_else(|| ApiError::Conflict("that problem isn't open".into()))?;
    jobs::event(
        &mut tx,
        job,
        None,
        "problem_closed_unresolved",
        Some(&text),
        &[],
        None,
    )
    .await?;
    admin::audit(
        &mut tx,
        &admin.actor,
        "problem.close",
        &job.to_string(),
        json!({ "problem": id, "note": text }),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    /// remove, note or keep.
    outcome: String,
    /// The written reason, shown to both sides (DSA Art. 17).
    reason: String,
    /// For "note": the factual note shown under the review.
    note: Option<String>,
}

async fn decide(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(job_id): Path<Uuid>,
    Json(d): Json<Decision>,
) -> ApiResult<StatusCode> {
    admin.need(crate::staff::Perm::Moderate)?;
    let reason = note("reason", &d.reason, 1000, true)?;
    let (status, hidden, mediator_note) = match d.outcome.as_str() {
        "remove" => ("removed", true, None),
        "note" => {
            let n = note("note", d.note.as_deref().unwrap_or(""), 1000, true)?;
            ("noted", false, Some(n))
        }
        "keep" => ("kept", false, None),
        _ => return Err(ApiError::invalid("outcome", "remove, note or keep")),
    };
    let mut tx = state.db.begin().await?;
    let done = sqlx::query(
        "update review_disputes set status = $2, decision_reason = $3, decided_by = $4, decided_at = now()
         where job_id = $1 and status = 'pending'",
    )
    .bind(job_id)
    .bind(status)
    .bind(&reason)
    .bind(&admin.actor)
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::Conflict("that dispute isn't pending".into()));
    }
    if hidden {
        // A removed review is a moderation decision: the reviewer gets the statement of reasons
        // and can appeal (CON-9).
        let (client, dispute_reason, label): (Uuid, String, String) = sqlx::query_as(
            "select r.client_id, d.reason, 'Review of ' || c.name || ' by ' || u.display_name
             from creative_reviews r join review_disputes d on d.job_id = r.job_id
             join creatives c on c.user_id = r.creative_id join users u on u.id = r.client_id
             where r.job_id = $1",
        )
        .bind(job_id)
        .fetch_one(&mut *tx)
        .await?;
        let rule = match dispute_reason.as_str() {
            "not_about_job" => "fake_review",
            "extortion" => "extortion",
            "false_fact" => "false_fact",
            _ => "harassment",
        };
        crate::moderation::record(
            &mut tx,
            &crate::moderation::NewDecision {
                user_id: client,
                subject_kind: "review",
                subject_id: job_id,
                subject_label: &label,
                action: "hide_review",
                rule,
                law: None,
                facts: &reason,
                decided_by: &admin.actor,
            },
        )
        .await?;
    }
    sqlx::query(
        "update creative_reviews set hidden_at = case when $2 then now() end, hidden_reason = case when $2 then $3 end,
                mediator_note = $4 where job_id = $1",
    )
    .bind(job_id)
    .bind(hidden)
    .bind(&reason)
    .bind(mediator_note)
    .execute(&mut *tx)
    .await?;
    jobs::event(
        &mut tx,
        job_id,
        None,
        &format!("review_dispute_{status}"),
        Some(&reason),
        &[],
        None,
    )
    .await?;
    admin::audit(
        &mut tx,
        &admin.actor,
        "review_dispute.decide",
        &job_id.to_string(),
        json!({ "outcome": status, "reason": reason }),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
