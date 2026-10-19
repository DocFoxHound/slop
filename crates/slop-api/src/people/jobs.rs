//! Jobs: request → offer → accept & pay → deliver → complete (DESIGN §15.2, PAYMENTS §7).
//!
//! The steps and their short notes are the **contract record** (HUM-16, HUM-17), not a chat.
//! Slop never decides disputes; it records what each side did.

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use time::{Date, Duration, OffsetDateTime, format_description::well_known::Iso8601};
use url::Url;
use uuid::Uuid;

use super::{Service, Skill, check_price, require_adult, skill, stripe};
use crate::{
    AppState,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
    updates::owned,
};

/// The Service Agreement template (versioned: change the version with the text).
pub const TEMPLATE: &str = include_str!("service_agreement.md");
pub const TEMPLATE_VERSION: &str = "2026-10-19";
/// The client has this long after delivery to accept or ask for a revision (DESIGN §15.2).
pub const COMPLETION_WINDOW: Duration = Duration::days(7);
const MAX_OPEN_REQUESTS: i64 = 3;
const DEFAULT_REFUNDS: &str =
    "Full refund before work starts. After that, refunds cover the parts not yet delivered.";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/people/{handle}/hire", post(hire))
        .route("/me/jobs", get(my_jobs))
        .route("/jobs/{id}", get(job))
        .route("/jobs/{id}/offer", post(offer))
        .route("/jobs/{id}/decline", post(decline))
        .route("/jobs/{id}/changes", post(ask_changes))
        .route("/jobs/{id}/accept", post(accept))
        .route("/jobs/{id}/pay", post(pay))
        .route("/jobs/{id}/payment/check", post(check_payment))
        .route("/jobs/{id}/deliver", post(deliver))
        .route("/jobs/{id}/revision", post(revision))
        .route("/jobs/{id}/complete", post(complete))
        .route("/jobs/{id}/cancel", post(cancel))
        .route("/jobs/{id}/withdraw", post(withdraw))
        .route("/jobs/{id}/review", put(review))
}

// ---------------------------------------------------------------------------------------------
// The offer

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Offer {
    pub title: String,
    #[serde(default)]
    pub scope_md: String,
    pub price_minor: i64,
    pub currency: String,
    /// upfront, split (half now, half on delivery) or on_delivery.
    pub payment: String,
    /// YYYY-MM-DD.
    pub due_date: String,
    pub revisions: i16,
    /// transfer ("You own it") or licence ("Licence for this project").
    pub rights: String,
    /// May be published under the project's open-source licence.
    pub open_source: bool,
    /// Credited on the project's page.
    pub credit: bool,
    /// none, assisted or heavy, for this job.
    pub ai_use: String,
    #[serde(default)]
    pub refund_policy: String,
}

fn clean_offer(mut o: Offer, today: Date) -> ApiResult<Offer> {
    o.title = o.title.trim().to_string();
    if o.title.is_empty() || o.title.chars().count() > 100 || o.title.chars().any(char::is_control)
    {
        return Err(ApiError::invalid(
            "title",
            "one line of up to 100 characters",
        ));
    }
    o.scope_md = o.scope_md.trim().to_string();
    if o.scope_md.chars().count() > 4000 {
        return Err(ApiError::invalid("scope_md", "up to 4,000 characters"));
    }
    check_price("price_minor", o.price_minor, &o.currency)?;
    if !["upfront", "split", "on_delivery"].contains(&o.payment.as_str()) {
        return Err(ApiError::invalid(
            "payment",
            "upfront, split or on_delivery",
        ));
    }
    let due = Date::parse(&o.due_date, &Iso8601::DATE)
        .map_err(|_| ApiError::invalid("due_date", "a date, YYYY-MM-DD"))?;
    if due < today || due > today + Duration::days(730) {
        return Err(ApiError::invalid(
            "due_date",
            "from today to two years ahead",
        ));
    }
    if !(0..=10).contains(&o.revisions) {
        return Err(ApiError::invalid("revisions", "0 to 10"));
    }
    if !["transfer", "licence"].contains(&o.rights.as_str()) {
        return Err(ApiError::invalid("rights", "transfer or licence"));
    }
    if !["none", "assisted", "heavy"].contains(&o.ai_use.as_str()) {
        return Err(ApiError::invalid("ai_use", "none, assisted or heavy"));
    }
    o.refund_policy = o.refund_policy.trim().to_string();
    if o.refund_policy.is_empty() {
        o.refund_policy = DEFAULT_REFUNDS.into();
    }
    if o.refund_policy.chars().count() > 500 {
        return Err(ApiError::invalid("refund_policy", "up to 500 characters"));
    }
    Ok(o)
}

pub fn money(minor: i64, currency: &str) -> String {
    let symbol = match currency {
        "eur" => "€",
        "gbp" => "£",
        _ => "$",
    };
    let whole = minor / 100;
    let cents = minor % 100;
    let grouped = {
        let s = whole.to_string();
        let mut out = String::new();
        for (i, c) in s.chars().enumerate() {
            if i > 0 && (s.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(c);
        }
        out
    };
    if cents == 0 {
        format!("{symbol}{grouped}")
    } else {
        format!("{symbol}{grouped}.{cents:02}")
    }
}

/// The payments an offer leads to: (part, due on, amount).
pub fn schedule(o: &Offer) -> Vec<(&'static str, &'static str, i64)> {
    match o.payment.as_str() {
        "upfront" => vec![("full", "accept", o.price_minor)],
        "on_delivery" => vec![("full", "delivery", o.price_minor)],
        _ => {
            let first = (o.price_minor + 1) / 2;
            vec![
                ("deposit", "accept", first),
                ("final", "delivery", o.price_minor - first),
            ]
        }
    }
}

fn payment_words(o: &Offer) -> String {
    let price = |m| money(m, &o.currency);
    match o.payment.as_str() {
        "upfront" => format!("all {} when you accept", price(o.price_minor)),
        "on_delivery" => format!("all {} when the work is delivered", price(o.price_minor)),
        _ => {
            let s = schedule(o);
            format!(
                "{} when you accept, {} when the work is delivered",
                price(s[0].2),
                price(s[1].2)
            )
        }
    }
}

fn ai_words(ai: &str) -> &'static str {
    match ai {
        "none" => "No AI: made entirely by hand.",
        "assisted" => "AI-assisted: AI used for references or drafts, finished by hand.",
        _ => "Mostly AI-generated, then edited.",
    }
}

/// The plain-language summary both sides see (DESIGN §15.3).
pub fn summary(o: &Offer) -> Vec<String> {
    vec![
        format!("What: {}", o.title),
        format!("Price: {}", money(o.price_minor, &o.currency)),
        format!("Payment: {}", payment_words(o)),
        format!("Due: {}", o.due_date),
        format!(
            "Revisions: {}",
            match o.revisions {
                0 => "none included".to_string(),
                1 => "1 included".to_string(),
                n => format!("{n} included"),
            }
        ),
        match o.rights.as_str() {
            "transfer" => "Who owns it: you do, once it's fully paid (copyright is transferred)".into(),
            _ => "Who owns it: the creative; you get a licence to use and change it in this project, forever".into(),
        },
        if o.open_source {
            "Open source: may be published under your project's open-source licence".into()
        } else {
            "Open source: no; it stays all rights reserved, even in a public repository".into()
        },
        format!("Credit: {}", if o.credit { "shown on your project's page" } else { "not shown" }),
        format!("AI use: {}", ai_words(&o.ai_use)),
        format!("Refunds: {}", o.refund_policy),
    ]
}

pub struct Parties<'a> {
    pub job_id: Uuid,
    pub creative_name: &'a str,
    pub creative_handle: &'a str,
    pub trader: bool,
    pub client_name: &'a str,
    pub project_title: &'a str,
    pub project_slug: &'a str,
    /// The job's skill, for the credit's wording.
    pub skill: &'a str,
    pub withdrawal_days: Option<i16>,
}

/// Fills in the Service Agreement. The result is what both sides agree to, word for word.
pub fn agreement(o: &Offer, p: &Parties) -> String {
    let rights = match o.rights.as_str() {
        "transfer" => format!(
            "Once the client has paid in full, the creative **transfers the copyright** in the work to the client, \
             with the right to use, change and publish it anywhere. This is a transfer of copyright, not a \"work made \
             for hire\". The creative may still show the work in their portfolio, credited to {}.",
            p.project_title
        ),
        _ => format!(
            "The creative keeps the copyright. Once the client has paid in full, the creative gives the client a \
             **worldwide, permanent, non-exclusive licence** to use, change and publish the work in {} \
             (including its updates, ports and marketing). The creative may show the work in their portfolio.",
            p.project_title
        ),
    };
    let open_source = if o.open_source {
        "**Open source:** the client may publish the work in the project's repository **under the project's \
         open-source licence**, which lets anyone reuse it under that licence."
            .to_string()
    } else {
        "**Open source:** the work may be stored in the project's public repository, but it **isn't** covered by \
         the project's open-source licence: it stays all rights reserved except for the rights above. The client \
         marks it as such in the repository."
            .to_string()
    };
    let credit = if o.credit {
        format!(
            "The client agrees that Slop shows \"{} by {}\" on the project's page.",
            super::credit_role(p.skill),
            p.creative_name
        )
    } else {
        "The work isn't credited on the project's page.".to_string()
    };
    let withdrawal = match p.withdrawal_days {
        Some(days) => format!(
            "**Your right to cancel.** As a consumer you may cancel this agreement within {days} days of accepting it, \
             without giving a reason, using the \"Withdraw from this contract\" button on the job page. If you asked \
             the creative to start within that time, you pay for the work done until you cancel, and you lose the \
             right once the work is fully done. The creative refunds you within 14 days of cancelling."
        ),
        None => String::new(),
    };
    let trader = if p.trader {
        "The creative offers this service **as a business (trader)**, so consumer law applies to this agreement where the client is a consumer."
    } else {
        "The creative offers this service **as a private individual, not a business**, so consumer protection law (such as the right to cancel) usually doesn't apply."
    };
    let scope = if o.scope_md.is_empty() {
        String::new()
    } else {
        format!("**Scope:**\n\n{}", o.scope_md)
    };
    let fields: [(&str, String); 21] = [
        ("template", TEMPLATE_VERSION.to_string()),
        ("job_id", p.job_id.to_string()),
        ("creative_name", p.creative_name.to_string()),
        ("creative_handle", p.creative_handle.to_string()),
        ("client_name", p.client_name.to_string()),
        ("trader_line", trader.to_string()),
        ("title", o.title.clone()),
        ("project_title", p.project_title.to_string()),
        ("project_slug", p.project_slug.to_string()),
        ("scope", scope),
        ("due_date", o.due_date.clone()),
        ("revisions", o.revisions.to_string()),
        ("ai_use_line", ai_words(&o.ai_use).to_string()),
        ("price", money(o.price_minor, &o.currency)),
        ("payment_line", payment_words(o) + "."),
        ("rights_line", rights),
        ("open_source_line", open_source),
        ("credit_line", credit),
        ("refund_policy", o.refund_policy.clone()),
        ("withdrawal_line", withdrawal),
        ("unused", String::new()),
    ];
    let mut text = TEMPLATE.to_string();
    for (k, v) in fields {
        text = text.replace(&format!("{{{{{k}}}}}"), &v);
    }
    // Empty optional paragraphs leave blank runs; tidy them.
    while text.contains("\n\n\n") {
        text = text.replace("\n\n\n", "\n\n");
    }
    text
}

// ---------------------------------------------------------------------------------------------
// Loading a job

#[derive(sqlx::FromRow)]
struct JobRow {
    id: Uuid,
    creative_id: Uuid,
    client_id: Uuid,
    skill: String,
    status: String,
    request_md: String,
    wanted_by: Option<Date>,
    client_is_business: bool,
    #[sqlx(json(nullable))]
    offer: Option<Offer>,
    offer_count: i32,
    agreement_text: Option<String>,
    agreement_sha256: Option<String>,
    agreement_template: Option<String>,
    accepted_at: Option<OffsetDateTime>,
    withdrawal_until: Option<OffsetDateTime>,
    start_now_ack_at: Option<OffsetDateTime>,
    delivered_at: Option<OffsetDateTime>,
    revisions_used: i16,
    completed_at: Option<OffsetDateTime>,
    completed_automatically: bool,
    credited_at: Option<OffsetDateTime>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    creative_name: String,
    creative_handle: String,
    trader: bool,
    creative_ai_use: String,
    stripe_account_id: Option<String>,
    client_name: String,
    client_country: String,
    project_title: String,
    project_slug: String,
    project_stage: String,
}

const JOB_SELECT: &str = "select j.id, j.creative_id, j.client_id, j.app_id, j.skill, j.status, j.request_md,
        j.wanted_by, j.client_is_business, j.offer, j.offer_count, j.agreement_text, j.agreement_sha256,
        j.agreement_template, j.accepted_at, j.withdrawal_until, j.start_now_ack_at, j.delivered_at,
        j.revisions_used, j.completed_at, j.completed_automatically, j.credited_at, j.created_at, j.updated_at,
        c.name as creative_name, c.handle as creative_handle, c.trader, c.ai_use as creative_ai_use,
        c.stripe_account_id, u.display_name as client_name, u.country_code as client_country,
        a.title as project_title, a.slug as project_slug, a.stage::text as project_stage
    from jobs j join creatives c on c.user_id = j.creative_id join users u on u.id = j.client_id
    join apps a on a.id = j.app_id";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Client,
    Creative,
}

/// The job, if the caller is one of its two sides. Anyone else gets 404.
async fn load(state: &AppState, id: Uuid, user: &CurrentUser) -> ApiResult<(JobRow, Role)> {
    let row: Option<JobRow> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("{JOB_SELECT} where j.id = $1")))
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    match row {
        Some(j) if j.client_id == user.id => Ok((j, Role::Client)),
        Some(j) if j.creative_id == user.id => Ok((j, Role::Creative)),
        _ => Err(ApiError::NotFound),
    }
}

#[derive(Serialize, sqlx::FromRow)]
struct PaymentView {
    part: String,
    due_on: String,
    amount_minor: i32,
    currency: String,
    /// waiting (not due yet), due or paid.
    status: String,
    #[serde(with = "time::serde::rfc3339::option")]
    paid_at: Option<OffsetDateTime>,
    simulated: bool,
}

#[derive(Serialize, sqlx::FromRow)]
struct EventView {
    kind: String,
    /// Null: Slop did it.
    actor: Option<String>,
    note: Option<String>,
    links: Vec<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

#[derive(Serialize)]
struct Agreement {
    text: String,
    sha256: Option<String>,
    template: String,
    #[serde(with = "time::serde::rfc3339::option")]
    accepted_at: Option<OffsetDateTime>,
}

#[derive(Serialize)]
struct Withdrawal {
    #[serde(with = "time::serde::rfc3339")]
    until: OffsetDateTime,
    open: bool,
    started_early: bool,
}

#[derive(Serialize)]
struct MyReview {
    vote: &'static str,
    body: String,
    reply: Option<String>,
}

#[derive(Serialize)]
pub struct JobView {
    id: Uuid,
    status: String,
    /// What's happening, in words for the viewer.
    status_text: String,
    role: &'static str,
    creative: Value,
    client_name: String,
    project: Value,
    skill: Skill,
    request_md: String,
    wanted_by: Option<String>,
    client_is_business: bool,
    offer: Option<Offer>,
    offer_count: i32,
    summary: Vec<String>,
    agreement: Option<Agreement>,
    payments: Vec<PaymentView>,
    paid_minor: i64,
    refunded_minor: i64,
    /// For the creative: about what they'll receive after Slop's fee (Stripe's card fee is extra).
    creative_receives_minor: Option<i64>,
    withdrawal: Option<Withdrawal>,
    /// Before acceptance: the consumer cancellation period that will apply, in days.
    withdrawal_days: Option<i16>,
    revisions_left: i16,
    #[serde(with = "time::serde::rfc3339::option")]
    auto_complete_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    completed_at: Option<OffsetDateTime>,
    completed_automatically: bool,
    credited: bool,
    events: Vec<EventView>,
    /// What the viewer can do now.
    actions: Vec<&'static str>,
    payment_mode: &'static str,
    review: Option<MyReview>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

fn status_text(j: &JobRow, role: Role, due: Option<&PaymentView>) -> String {
    let c = &j.creative_name;
    let me_client = role == Role::Client;
    match j.status.as_str() {
        "requested" if me_client => format!("Waiting for {c} to send an offer."),
        "requested" => "They're waiting for your offer.".into(),
        "offered" if me_client => {
            format!("{c} sent an offer. Accept it, ask for changes, or decline.")
        }
        "offered" => "Offer sent. Waiting for the client.".into(),
        "awaiting_payment" if me_client => match due {
            Some(p) => format!(
                "Accepted. Pay {} to start the work.",
                money(p.amount_minor.into(), &p.currency)
            ),
            None => "Accepted.".into(),
        },
        "awaiting_payment" => "Accepted. Waiting for the first payment before you start.".into(),
        "in_progress" if me_client => format!("{c} is working on it."),
        "in_progress" => "In progress. Mark it delivered when it's ready.".into(),
        "delivered" if me_client => match due {
            Some(p) => format!(
                "Delivered. If it looks good, pay the remaining {} to finish.",
                money(p.amount_minor.into(), &p.currency)
            ),
            None => "Delivered. Say it looks good, or ask for a revision.".into(),
        },
        "delivered" => "Delivered. Waiting for the client to accept it.".into(),
        "completed" if due.is_some() => "Completed. A payment is still owed.".into(),
        "completed" => "Completed.".into(),
        "declined" => "Declined.".into(),
        "cancelled" => format!("Cancelled. Any refund comes from {c} through Stripe."),
        "withdrawn" => format!("Withdrawn by the client. {c} refunds what's owed within 14 days."),
        other => other.to_string(),
    }
}

async fn view(state: &AppState, j: JobRow, role: Role, viewer: Uuid) -> ApiResult<JobView> {
    let payments: Vec<PaymentView> = sqlx::query_as(
        "select jp.part, jp.due_on, jp.amount_minor, jp.currency, jp.status, jp.paid_at,
                coalesce(bool_or(p.simulated), false) as simulated
         from job_payments jp left join payments p on p.job_payment_id = jp.id and p.kind = 'people_job'
         where jp.job_id = $1 group by jp.id order by jp.due_on, jp.part",
    )
    .bind(j.id)
    .fetch_all(&state.db)
    .await?;
    let (paid, refunded): (i64, i64) = sqlx::query_as(
        "select coalesce(sum(p.amount_minor) filter (where p.kind = 'people_job'), 0)::int8,
                coalesce(sum(p.amount_minor) filter (where p.kind = 'refund'), 0)::int8
         from payments p left join job_payments jp on jp.id = p.job_payment_id
         left join payments o on o.id = p.refund_of left join job_payments ojp on ojp.id = o.job_payment_id
         where jp.job_id = $1 or ojp.job_id = $1",
    )
    .bind(j.id)
    .fetch_one(&state.db)
    .await?;
    let events: Vec<EventView> = sqlx::query_as(
        "select e.kind, case when e.actor_id = $2 then 'You' else u.display_name end as actor,
                e.note, e.links, e.created_at
         from job_events e left join users u on u.id = e.actor_id
         where e.job_id = $1 order by e.created_at, e.id",
    )
    .bind(j.id)
    .bind(viewer)
    .fetch_all(&state.db)
    .await?;
    let review: Option<(i16, String, Option<String>)> =
        sqlx::query_as("select vote, body, reply from creative_reviews where job_id = $1")
            .bind(j.id)
            .fetch_optional(&state.db)
            .await?;
    let now = OffsetDateTime::now_utc();
    let due = payments.iter().find(|p| p.status == "due");
    let active = ["awaiting_payment", "in_progress", "delivered"].contains(&j.status.as_str());
    let withdrawal = j.withdrawal_until.map(|until| Withdrawal {
        until,
        open: until > now && active,
        started_early: j.start_now_ack_at.is_some(),
    });
    let revisions_left = j
        .offer
        .as_ref()
        .map_or(0, |o| (o.revisions - j.revisions_used).max(0));
    let mut actions: Vec<&'static str> = Vec::new();
    match (role, j.status.as_str()) {
        (Role::Creative, "requested") => actions.extend(["offer", "decline"]),
        (Role::Creative, "offered") => actions.extend(["offer", "decline"]),
        (Role::Client, "requested") => actions.push("decline"),
        (Role::Client, "offered") => actions.extend(["accept", "ask_changes", "decline"]),
        (Role::Client, "awaiting_payment") => actions.extend(["pay", "cancel"]),
        (Role::Creative, "awaiting_payment") => actions.push("cancel"),
        (Role::Creative, "in_progress") => actions.extend(["deliver", "cancel"]),
        (Role::Client, "delivered") => {
            actions.push(if due.is_some() { "pay" } else { "complete" });
            if revisions_left > 0 {
                actions.push("revision");
            }
        }
        (Role::Creative, "delivered") => actions.push("cancel"),
        (Role::Client, "completed") => {
            if due.is_some() {
                actions.push("pay");
            }
            actions.push("review");
        }
        _ => {}
    }
    if role == Role::Client && withdrawal.as_ref().is_some_and(|w| w.open) {
        actions.push("withdraw");
    }
    let summary_lines = j.offer.as_ref().map(summary).unwrap_or_default();
    let withdrawal_days = if j.status == "offered" {
        withdrawal_days(state, &j).await?
    } else {
        None
    };
    let agreement = match (&j.agreement_text, &j.offer) {
        (Some(text), _) => Some(Agreement {
            text: text.clone(),
            sha256: j.agreement_sha256.clone(),
            template: j.agreement_template.clone().unwrap_or_default(),
            accepted_at: j.accepted_at,
        }),
        // Before acceptance: the text the client would agree to.
        (None, Some(o)) if j.status == "offered" => Some(Agreement {
            text: agreement(o, &parties(state, &j).await?),
            sha256: None,
            template: TEMPLATE_VERSION.into(),
            accepted_at: None,
        }),
        _ => None,
    };
    Ok(JobView {
        status_text: status_text(&j, role, due),
        id: j.id,
        status: j.status.clone(),
        role: match role {
            Role::Client => "client",
            Role::Creative => "creative",
        },
        creative: json!({ "name": j.creative_name, "handle": j.creative_handle, "trader": j.trader, "ai_use": j.creative_ai_use }),
        client_name: j.client_name.clone(),
        project: json!({ "title": j.project_title, "slug": j.project_slug, "stage": j.project_stage }),
        skill: skill(&j.skill),
        request_md: j.request_md.clone(),
        wanted_by: j.wanted_by.map(|d| d.to_string()),
        client_is_business: j.client_is_business,
        creative_receives_minor: match (role, &j.offer) {
            (Role::Creative, Some(o)) => Some(o.price_minor - stripe::fee(o.price_minor)),
            _ => None,
        },
        offer: j.offer.clone(),
        offer_count: j.offer_count,
        summary: summary_lines,
        agreement,
        payments,
        paid_minor: paid,
        // Refunds are stored as negative amounts.
        refunded_minor: -refunded,
        withdrawal,
        withdrawal_days,
        revisions_left,
        auto_complete_at: (j.status == "delivered")
            .then(|| j.delivered_at.map(|d| d + COMPLETION_WINDOW))
            .flatten(),
        completed_at: j.completed_at,
        completed_automatically: j.completed_automatically,
        credited: j.credited_at.is_some(),
        events,
        actions,
        payment_mode: stripe::mode(state).name(),
        review: review.map(|(vote, body, reply)| MyReview {
            vote: if vote > 0 { "not_slop" } else { "slop" },
            body,
            reply,
        }),
        created_at: j.created_at,
    })
}

async fn parties<'a>(state: &AppState, j: &'a JobRow) -> ApiResult<Parties<'a>> {
    Ok(Parties {
        job_id: j.id,
        creative_name: &j.creative_name,
        creative_handle: &j.creative_handle,
        trader: j.trader,
        client_name: &j.client_name,
        project_title: &j.project_title,
        project_slug: &j.project_slug,
        skill: &j.skill,
        withdrawal_days: withdrawal_days(state, j).await?,
    })
}

/// The consumer withdrawal period, when it applies: a trader creative, a client who isn't
/// hiring as a business, in a country with such a right (HUM-2).
async fn withdrawal_days(state: &AppState, j: &JobRow) -> ApiResult<Option<i16>> {
    if !j.trader || j.client_is_business {
        return Ok(None);
    }
    Ok(sqlx::query_scalar(
        "select consumer_withdrawal_days from country_policies where country_code = $1",
    )
    .bind(&j.client_country)
    .fetch_optional(&state.db)
    .await?
    .flatten())
}

async fn reload(state: &AppState, id: Uuid, user: &CurrentUser) -> ApiResult<Json<JobView>> {
    let (j, role) = load(state, id, user).await?;
    Ok(Json(view(state, j, role, user.id).await?))
}

async fn event(
    conn: &mut sqlx::PgConnection,
    job: Uuid,
    actor: Option<Uuid>,
    kind: &str,
    note: Option<&str>,
    links: &[String],
    offer: Option<&Offer>,
) -> sqlx::Result<()> {
    sqlx::query(
        "insert into job_events (id, job_id, actor_id, kind, note, links, offer) values ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(Uuid::now_v7())
    .bind(job)
    .bind(actor)
    .bind(kind)
    .bind(note.filter(|n| !n.is_empty()))
    .bind(links)
    .bind(offer.map(sqlx::types::Json))
    .execute(&mut *conn)
    .await?;
    sqlx::query("update jobs set updated_at = now() where id = $1")
        .bind(job)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

fn note(field: &'static str, text: &str, max: usize, required: bool) -> ApiResult<String> {
    let t = text.trim();
    if required && t.is_empty() {
        return Err(ApiError::invalid(field, "say what you need"));
    }
    if t.chars().count() > max {
        return Err(ApiError::invalid(field, format!("up to {max} characters")));
    }
    Ok(t.to_string())
}

fn wrong_step(what: &str) -> ApiError {
    ApiError::Conflict(format!("the job isn't at the step where you can {what}"))
}

/// Changes the status only if it's still `from` (two people may press buttons at once).
async fn set_status(
    conn: &mut sqlx::PgConnection,
    id: Uuid,
    from: &[&str],
    to: &str,
) -> ApiResult<()> {
    let from: Vec<String> = from.iter().map(|s| s.to_string()).collect();
    let done = sqlx::query(
        "update jobs set status = $3, updated_at = now() where id = $1 and status = any($2)",
    )
    .bind(id)
    .bind(&from)
    .bind(to)
    .execute(&mut *conn)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::Conflict("the job changed; reload it".into()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Steps

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HireBody {
    /// One of the caller's apps (any stage, public or private).
    app_slug: String,
    skill: String,
    /// The package hired, if any; its details go into the record.
    service_id: Option<String>,
    request_md: String,
    wanted_by: Option<String>,
    #[serde(default)]
    client_is_business: bool,
    #[serde(default)]
    confirm_adult: bool,
}

async fn hire(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(handle): Path<String>,
    Json(body): Json<HireBody>,
) -> ApiResult<Json<JobView>> {
    require_adult(&state, &user, body.confirm_adult).await?;
    /// (id, skills, live, can be hired, services)
    type Hireable = (
        Uuid,
        Vec<String>,
        bool,
        bool,
        sqlx::types::Json<Vec<Service>>,
    );
    let row: Option<Hireable> = sqlx::query_as(
        "select user_id, skills, (portfolio_set_id is not null and hidden_reason is null),
                (taking_work and stripe_status = 'active'), services
         from creatives where handle = $1",
    )
    .bind(handle.to_ascii_lowercase())
    .fetch_optional(&state.db)
    .await?;
    let Some((creative_id, skills, live, can_hire, services)) = row else {
        return Err(ApiError::NotFound);
    };
    if !live {
        return Err(ApiError::NotFound);
    }
    if creative_id == user.id {
        return Err(ApiError::Conflict("you can't hire yourself".into()));
    }
    if !can_hire {
        return Err(ApiError::Conflict(
            "they aren't taking paid work right now".into(),
        ));
    }
    if !skills.contains(&body.skill) {
        return Err(ApiError::invalid("skill", "pick one of their skills"));
    }
    let service = match body.service_id.as_deref() {
        Some(id) => Some(services.0.into_iter().find(|s| s.id == id).ok_or_else(|| {
            ApiError::invalid("service_id", "that service isn't offered any more")
        })?),
        None => None,
    };
    let (app_id, _) = owned(&state, &body.app_slug, &user)
        .await
        .map_err(|_| ApiError::invalid("app_slug", "pick one of your projects"))?;
    let request = note("request_md", &body.request_md, 4000, true)?;
    let wanted_by = match body.wanted_by.as_deref().filter(|d| !d.is_empty()) {
        Some(d) => Some(
            Date::parse(d, &Iso8601::DATE)
                .map_err(|_| ApiError::invalid("wanted_by", "a date, YYYY-MM-DD"))?,
        ),
        None => None,
    };
    let open: i64 = sqlx::query_scalar(
        "select count(*) from jobs where client_id = $1 and creative_id = $2 and status in ('requested', 'offered')",
    )
    .bind(user.id)
    .bind(creative_id)
    .fetch_one(&state.db)
    .await?;
    if open >= MAX_OPEN_REQUESTS {
        return Err(ApiError::Conflict(format!(
            "you already have {open} open requests with them; wait for an answer first"
        )));
    }
    let id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "insert into jobs (id, creative_id, client_id, app_id, skill, status, request_md, wanted_by, client_is_business)
         values ($1, $2, $3, $4, $5, 'requested', $6, $7, $8)",
    )
    .bind(id)
    .bind(creative_id)
    .bind(user.id)
    .bind(app_id)
    .bind(&body.skill)
    .bind(&request)
    .bind(wanted_by)
    .bind(body.client_is_business)
    .execute(&mut *tx)
    .await?;
    let service_note = service.map(|s| {
        format!(
            "Package: {} (from {}, about {} days)",
            s.name,
            money(s.price_minor, &s.currency),
            s.days
        )
    });
    event(
        &mut tx,
        id,
        Some(user.id),
        "requested",
        service_note.as_deref(),
        &[],
        None,
    )
    .await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

#[derive(Serialize)]
struct JobCard {
    id: Uuid,
    status: String,
    status_text: String,
    needs_you: bool,
    other_name: String,
    creative_handle: String,
    project_title: String,
    project_slug: String,
    skill: Skill,
    title: String,
    price: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    updated_at: OffsetDateTime,
}

#[derive(Serialize)]
struct MyJobs {
    as_client: Vec<JobCard>,
    as_creative: Vec<JobCard>,
    /// How many jobs wait on the caller.
    waiting_on_you: usize,
}

async fn my_jobs(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<MyJobs>> {
    let rows: Vec<JobRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{JOB_SELECT} where j.client_id = $1 or j.creative_id = $1 order by j.updated_at desc limit 200"
    )))
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    let due: Vec<(Uuid, i32, String)> = sqlx::query_as(
        "select jp.job_id, jp.amount_minor, jp.currency from job_payments jp join jobs j on j.id = jp.job_id
         where jp.status = 'due' and j.client_id = $1",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    let reviewed: Vec<Uuid> =
        sqlx::query_scalar("select job_id from creative_reviews where client_id = $1")
            .bind(user.id)
            .fetch_all(&state.db)
            .await?;
    let mut out = MyJobs {
        as_client: vec![],
        as_creative: vec![],
        waiting_on_you: 0,
    };
    for j in rows {
        let role = if j.client_id == user.id {
            Role::Client
        } else {
            Role::Creative
        };
        let due_here = due
            .iter()
            .find(|(id, _, _)| *id == j.id)
            .map(|(_, amount, currency)| PaymentView {
                part: String::new(),
                due_on: String::new(),
                amount_minor: *amount,
                currency: currency.clone(),
                status: "due".into(),
                paid_at: None,
                simulated: false,
            });
        let needs_you = match (role, j.status.as_str()) {
            (Role::Creative, "requested" | "in_progress") => true,
            (Role::Client, "offered" | "awaiting_payment" | "delivered") => true,
            (Role::Client, "completed") => due_here.is_some() || !reviewed.contains(&j.id),
            _ => false,
        };
        let card = JobCard {
            id: j.id,
            status: j.status.clone(),
            status_text: status_text(&j, role, due_here.as_ref()),
            needs_you,
            other_name: match role {
                Role::Client => j.creative_name.clone(),
                Role::Creative => j.client_name.clone(),
            },
            creative_handle: j.creative_handle.clone(),
            project_title: j.project_title.clone(),
            project_slug: j.project_slug.clone(),
            skill: skill(&j.skill),
            title: j.offer.as_ref().map_or_else(
                || j.request_md.chars().take(80).collect(),
                |o| o.title.clone(),
            ),
            price: j.offer.as_ref().map(|o| money(o.price_minor, &o.currency)),
            updated_at: j.updated_at,
        };
        if needs_you {
            out.waiting_on_you += 1;
        }
        match role {
            Role::Client => out.as_client.push(card),
            Role::Creative => out.as_creative.push(card),
        }
    }
    Ok(Json(out))
}

async fn job(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<JobView>> {
    reload(&state, id, &user).await
}

async fn offer(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(o): Json<Offer>,
) -> ApiResult<Json<JobView>> {
    let (j, role) = load(&state, id, &user).await?;
    if role != Role::Creative || !["requested", "offered"].contains(&j.status.as_str()) {
        return Err(wrong_step("send an offer"));
    }
    let o = clean_offer(o, OffsetDateTime::now_utc().date())?;
    let mut tx = state.db.begin().await?;
    set_status(&mut tx, id, &["requested", "offered"], "offered").await?;
    sqlx::query(
        "update jobs set offer = $2, offer_count = offer_count + 1, offered_at = now() where id = $1",
    )
    .bind(id)
    .bind(sqlx::types::Json(&o))
    .execute(&mut *tx)
    .await?;
    event(&mut tx, id, Some(user.id), "offered", None, &[], Some(&o)).await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteBody {
    #[serde(default)]
    note: String,
}

async fn decline(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<NoteBody>,
) -> ApiResult<Json<JobView>> {
    let (_, _) = load(&state, id, &user).await?;
    let text = note("note", &body.note, 1000, false)?;
    let mut tx = state.db.begin().await?;
    set_status(&mut tx, id, &["requested", "offered"], "declined").await?;
    sqlx::query("update jobs set ended_at = now(), ended_by = $2 where id = $1")
        .bind(id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    event(
        &mut tx,
        id,
        Some(user.id),
        "declined",
        Some(&text),
        &[],
        None,
    )
    .await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

async fn ask_changes(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<NoteBody>,
) -> ApiResult<Json<JobView>> {
    let (_, role) = load(&state, id, &user).await?;
    if role != Role::Client {
        return Err(wrong_step("ask for changes"));
    }
    let text = note("note", &body.note, 1000, true)?;
    let mut tx = state.db.begin().await?;
    set_status(&mut tx, id, &["offered"], "requested").await?;
    event(
        &mut tx,
        id,
        Some(user.id),
        "changes_requested",
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
struct AcceptBody {
    /// The offer the client saw (it may have changed since).
    offer_count: i32,
    /// "Start now. I understand I lose my right to cancel once the work is fully done." Only
    /// asked when a withdrawal right applies; never pre-ticked (HUM-2).
    #[serde(default)]
    start_now: bool,
    #[serde(default)]
    confirm_adult: bool,
}

async fn accept(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<AcceptBody>,
) -> ApiResult<Json<JobView>> {
    let (j, role) = load(&state, id, &user).await?;
    if role != Role::Client || j.status != "offered" {
        return Err(wrong_step("accept"));
    }
    if body.offer_count != j.offer_count {
        return Err(ApiError::Conflict(
            "the offer changed; read the new one first".into(),
        ));
    }
    require_adult(&state, &user, body.confirm_adult).await?;
    let o = j.offer.clone().ok_or_else(|| wrong_step("accept"))?;
    let p = parties(&state, &j).await?;
    let text = agreement(&o, &p);
    let sha = hex::encode(Sha256::digest(text.as_bytes()));
    let now = OffsetDateTime::now_utc();
    let withdrawal_until = p.withdrawal_days.map(|d| now + Duration::days(d.into()));
    let plan = schedule(&o);
    let due_now = plan.iter().any(|(_, on, _)| *on == "accept");
    let mut tx = state.db.begin().await?;
    set_status(
        &mut tx,
        id,
        &["offered"],
        if due_now {
            "awaiting_payment"
        } else {
            "in_progress"
        },
    )
    .await?;
    sqlx::query(
        "update jobs set agreement_text = $2, agreement_sha256 = $3, agreement_template = $4, accepted_at = $5,
                withdrawal_until = $6, start_now_ack_at = $7, started_at = $8
         where id = $1",
    )
    .bind(id)
    .bind(&text)
    .bind(&sha)
    .bind(TEMPLATE_VERSION)
    .bind(now)
    .bind(withdrawal_until)
    .bind((withdrawal_until.is_some() && body.start_now).then_some(now))
    .bind((!due_now).then_some(now))
    .execute(&mut *tx)
    .await?;
    for (part, on, amount) in plan {
        sqlx::query(
            "insert into job_payments (id, job_id, part, due_on, amount_minor, currency, status)
             values ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(Uuid::now_v7())
        .bind(id)
        .bind(part)
        .bind(on)
        .bind(amount as i32)
        .bind(&o.currency)
        .bind(if on == "accept" { "due" } else { "waiting" })
        .execute(&mut *tx)
        .await?;
    }
    event(
        &mut tx,
        id,
        Some(user.id),
        "accepted",
        Some(&format!("Agreement SHA-256 {sha}")),
        &[],
        None,
    )
    .await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

#[derive(Serialize)]
struct PayAnswer {
    /// Stripe Checkout, to send the client to. None when the payment was simulated.
    url: Option<String>,
    job: JobView,
}

async fn pay(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<PayAnswer>> {
    let (j, role) = load(&state, id, &user).await?;
    if role != Role::Client
        || !["awaiting_payment", "delivered", "completed"].contains(&j.status.as_str())
    {
        return Err(wrong_step("pay"));
    }
    let due: Option<(Uuid, i32, String, String, Option<String>)> = sqlx::query_as(
        "select id, amount_minor, currency, part, checkout_session_id from job_payments
         where job_id = $1 and status = 'due' order by due_on, part limit 1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let Some((payment_id, amount, currency, part, session)) = due else {
        return Err(ApiError::Conflict("nothing is due right now".into()));
    };
    let title = j
        .offer
        .as_ref()
        .map_or("Commissioned work", |o| o.title.as_str());
    let label = match part.as_str() {
        "deposit" => "first half",
        "final" => "second half",
        _ => "payment",
    };
    let url = match stripe::mode(&state) {
        stripe::Mode::Off => {
            return Err(ApiError::Unavailable(
                "payments aren't switched on yet on this server",
            ));
        }
        stripe::Mode::Simulated => {
            payment_succeeded(
                &state.db,
                Paid {
                    job_payment_id: payment_id,
                    stripe_account_id: j.stripe_account_id.clone(),
                    object_id: format!("sim_{}", Uuid::now_v7().simple()),
                    payer: Some(user.id),
                    amount_minor: amount.into(),
                    fee_minor: stripe::fee(amount.into()),
                    simulated: true,
                },
            )
            .await?;
            None
        }
        stripe::Mode::Stripe(cfg) => {
            let account = j
                .stripe_account_id
                .clone()
                .ok_or_else(|| ApiError::Conflict("they can't take payments right now".into()))?;
            let due = stripe::Due {
                job_payment_id: payment_id,
                job_id: id,
                amount_minor: amount.into(),
                currency,
                description: format!(
                    "{} by @{} — {label} (People job via Slop)",
                    title, j.creative_handle
                ),
                account,
                session,
            };
            Some(
                stripe::checkout_url(&state, cfg, &due, user.id)
                    .await
                    .map_err(|e| ApiError::Conflict(format!("{e:#}")))?,
            )
        }
    };
    let (j, role) = load(&state, id, &user).await?;
    Ok(Json(PayAnswer {
        url,
        job: view(&state, j, role, user.id).await?,
    }))
}

/// After Stripe sends the client back: reads the session in case the webhook hasn't arrived.
async fn check_payment(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<JobView>> {
    let (j, _) = load(&state, id, &user).await?;
    if let (stripe::Mode::Stripe(cfg), Some(account)) = (stripe::mode(&state), &j.stripe_account_id)
    {
        let sessions: Vec<String> = sqlx::query_scalar(
            "select checkout_session_id from job_payments
             where job_id = $1 and status = 'due' and checkout_session_id is not null",
        )
        .bind(id)
        .fetch_all(&state.db)
        .await?;
        for s in sessions {
            stripe::sync_session(&state, cfg, account, &s).await?;
        }
    }
    reload(&state, id, &user).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeliverBody {
    #[serde(default)]
    note: String,
    /// Where the files are (any https links), up to 5.
    #[serde(default)]
    links: Vec<String>,
}

async fn deliver(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<DeliverBody>,
) -> ApiResult<Json<JobView>> {
    let (_, role) = load(&state, id, &user).await?;
    if role != Role::Creative {
        return Err(wrong_step("deliver"));
    }
    let text = note("note", &body.note, 2000, false)?;
    if body.links.len() > 5 {
        return Err(ApiError::invalid("links", "up to 5 links"));
    }
    let mut links = Vec::new();
    for l in &body.links {
        let url: Url = l
            .trim()
            .parse()
            .map_err(|_| ApiError::invalid("links", "links must be full https:// addresses"))?;
        if url.scheme() != "https" || url.as_str().len() > 500 {
            return Err(ApiError::invalid(
                "links",
                "links must be https:// addresses",
            ));
        }
        links.push(url.to_string());
    }
    if text.is_empty() && links.is_empty() {
        return Err(ApiError::invalid(
            "links",
            "add a link to the files, or a note saying where they are",
        ));
    }
    let mut tx = state.db.begin().await?;
    set_status(&mut tx, id, &["in_progress"], "delivered").await?;
    sqlx::query("update jobs set delivered_at = now() where id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("update job_payments set status = 'due' where job_id = $1 and due_on = 'delivery' and status = 'waiting'")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    event(
        &mut tx,
        id,
        Some(user.id),
        "delivered",
        Some(&text),
        &links,
        None,
    )
    .await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

async fn revision(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<NoteBody>,
) -> ApiResult<Json<JobView>> {
    let (j, role) = load(&state, id, &user).await?;
    let left = j
        .offer
        .as_ref()
        .map_or(0, |o| o.revisions - j.revisions_used);
    if role != Role::Client || j.status != "delivered" || left <= 0 {
        return Err(wrong_step("ask for a revision"));
    }
    let text = note("note", &body.note, 2000, true)?;
    let mut tx = state.db.begin().await?;
    set_status(&mut tx, id, &["delivered"], "in_progress").await?;
    sqlx::query(
        "update jobs set revisions_used = revisions_used + 1, delivered_at = null where id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    // Due again at the next delivery.
    sqlx::query("update job_payments set status = 'waiting' where job_id = $1 and due_on = 'delivery' and status = 'due'")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    event(
        &mut tx,
        id,
        Some(user.id),
        "revision_requested",
        Some(&text),
        &[],
        None,
    )
    .await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

/// "Looks good" when nothing more is owed (otherwise paying is the "looks good").
async fn complete(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<JobView>> {
    let (_, role) = load(&state, id, &user).await?;
    if role != Role::Client {
        return Err(wrong_step("accept the work"));
    }
    let owed: bool = sqlx::query_scalar(
        "select exists (select 1 from job_payments where job_id = $1 and status = 'due')",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    if owed {
        return Err(ApiError::Conflict("pay the rest to finish the job".into()));
    }
    let mut tx = state.db.begin().await?;
    finish(&mut tx, id, Some(user.id), false, OffsetDateTime::now_utc()).await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

async fn finish(
    conn: &mut sqlx::PgConnection,
    id: Uuid,
    actor: Option<Uuid>,
    automatically: bool,
    at: OffsetDateTime,
) -> ApiResult<()> {
    set_status(conn, id, &["delivered"], "completed").await?;
    sqlx::query("update jobs set completed_at = $3, completed_automatically = $2 where id = $1")
        .bind(id)
        .bind(automatically)
        .bind(at)
        .execute(&mut *conn)
        .await?;
    event(
        conn,
        id,
        actor,
        if automatically {
            "completed_automatically"
        } else {
            "completed"
        },
        None,
        &[],
        None,
    )
    .await?;
    maybe_credit(conn, id, at).await?;
    Ok(())
}

/// Grants the credit and badge once a job is completed, fully paid, and at least 10 (HUM-14).
pub async fn maybe_credit(
    conn: &mut sqlx::PgConnection,
    id: Uuid,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query(
        "update jobs j set credited_at = $3
         where j.id = $1 and j.status = 'completed' and j.credited_at is null
           and not exists (select 1 from job_payments where job_id = j.id and status <> 'paid')
           and (select coalesce(sum(amount_minor), 0) from job_payments where job_id = j.id) >= $2",
    )
    .bind(id)
    .bind(super::MIN_PRICE_MINOR)
    .bind(at)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

async fn cancel(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<NoteBody>,
) -> ApiResult<Json<JobView>> {
    let (j, role) = load(&state, id, &user).await?;
    let paid_any: bool = sqlx::query_scalar(
        "select exists (select 1 from job_payments where job_id = $1 and status = 'paid')",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    let allowed = match role {
        Role::Creative => {
            ["awaiting_payment", "in_progress", "delivered"].contains(&j.status.as_str())
        }
        // Before paying anything, the client can still back out.
        Role::Client => j.status == "awaiting_payment" && !paid_any,
    };
    if !allowed {
        return Err(wrong_step("cancel"));
    }
    let text = note("note", &body.note, 1000, role == Role::Creative)?;
    let mut tx = state.db.begin().await?;
    set_status(
        &mut tx,
        id,
        &["awaiting_payment", "in_progress", "delivered"],
        "cancelled",
    )
    .await?;
    sqlx::query("update jobs set ended_at = now(), ended_by = $2 where id = $1")
        .bind(id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("update job_payments set status = 'waiting' where job_id = $1 and status = 'due'")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    event(
        &mut tx,
        id,
        Some(user.id),
        "cancelled",
        Some(&text),
        &[],
        None,
    )
    .await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

/// The EU/UK consumer "Withdraw from this contract" button (HUM-3).
async fn withdraw(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<JobView>> {
    let (j, role) = load(&state, id, &user).await?;
    let open = j
        .withdrawal_until
        .is_some_and(|u| u > OffsetDateTime::now_utc());
    if role != Role::Client || !open {
        return Err(wrong_step("withdraw"));
    }
    let mut tx = state.db.begin().await?;
    set_status(
        &mut tx,
        id,
        &["awaiting_payment", "in_progress", "delivered"],
        "withdrawn",
    )
    .await?;
    sqlx::query(
        "update jobs set withdrawn_at = now(), ended_at = now(), ended_by = $2 where id = $1",
    )
    .bind(id)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("update job_payments set status = 'waiting' where job_id = $1 and status = 'due'")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    event(&mut tx, id, Some(user.id), "withdrawn", None, &[], None).await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewBody {
    /// not_slop or slop.
    vote: String,
    #[serde(default)]
    body: String,
}

/// Only the client of a completed job can review it (HUM-11). Saying it again replaces it.
async fn review(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<ReviewBody>,
) -> ApiResult<Json<JobView>> {
    let (j, role) = load(&state, id, &user).await?;
    if role != Role::Client || j.status != "completed" {
        return Err(ApiError::Conflict(
            "you can review once the job is complete".into(),
        ));
    }
    let vote: i16 = match body.vote.as_str() {
        "not_slop" => 1,
        "slop" => -1,
        _ => return Err(ApiError::invalid("vote", "not_slop or slop")),
    };
    let text = note("body", &body.body, 2000, false)?;
    sqlx::query(
        "insert into creative_reviews (job_id, creative_id, client_id, vote, body) values ($1, $2, $3, $4, $5)
         on conflict (job_id) do update set vote = $4, body = $5, updated_at = now()",
    )
    .bind(id)
    .bind(j.creative_id)
    .bind(user.id)
    .bind(vote)
    .bind(&text)
    .execute(&state.db)
    .await?;
    reload(&state, id, &user).await
}

// ---------------------------------------------------------------------------------------------
// Payments arriving, refunds, disputes (stripe.rs and the simulated mode call these)

pub struct Paid {
    pub job_payment_id: Uuid,
    pub stripe_account_id: Option<String>,
    /// The Stripe charge (or a simulated ID). Unique: recording it twice does nothing.
    pub object_id: String,
    pub payer: Option<Uuid>,
    pub amount_minor: i64,
    pub fee_minor: i64,
    pub simulated: bool,
}

/// Records a payment and moves the job on: work starts after the first payment, and paying
/// the final part after delivery accepts the work.
pub async fn payment_succeeded(db: &sqlx::PgPool, p: Paid) -> anyhow::Result<()> {
    payment_succeeded_at(db, p, None).await
}

/// Like `payment_succeeded`, at a given time (the simulator).
pub async fn payment_succeeded_at(
    db: &sqlx::PgPool,
    p: Paid,
    at: Option<OffsetDateTime>,
) -> anyhow::Result<()> {
    let at = at.unwrap_or_else(OffsetDateTime::now_utc);
    let mut tx = db.begin().await?;
    let row: Option<(Uuid, String, String, i32, String)> = sqlx::query_as(
        "select jp.job_id, jp.status, jp.due_on, jp.amount_minor, jp.currency from job_payments jp
         where jp.id = $1 for update",
    )
    .bind(p.job_payment_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (job_id, status, due_on, amount, currency) =
        row.ok_or_else(|| anyhow::anyhow!("no such job payment"))?;
    let inserted = sqlx::query(
        "insert into payments (id, kind, job_payment_id, stripe_account_id, stripe_object_id, payer_id,
                               amount_minor, slop_fee_minor, currency, simulated, created_at)
         values ($1, 'people_job', $2, $3, $4, $5, $6, $7, $8, $9, $10)
         on conflict (stripe_object_id) do nothing",
    )
    .bind(Uuid::now_v7())
    .bind(p.job_payment_id)
    .bind(&p.stripe_account_id)
    .bind(&p.object_id)
    .bind(p.payer)
    .bind(p.amount_minor as i32)
    .bind(p.fee_minor as i32)
    .bind(&currency)
    .bind(p.simulated)
    .bind(at)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 || status == "paid" {
        tx.commit().await?;
        return Ok(());
    }
    anyhow::ensure!(
        i64::from(amount) == p.amount_minor,
        "paid {} but {} was due",
        p.amount_minor,
        amount
    );
    sqlx::query("update job_payments set status = 'paid', paid_at = $2 where id = $1")
        .bind(p.job_payment_id)
        .bind(at)
        .execute(&mut *tx)
        .await?;
    let job_status: String = sqlx::query_scalar("select status from jobs where id = $1 for update")
        .bind(job_id)
        .fetch_one(&mut *tx)
        .await?;
    let note = money(p.amount_minor, &currency);
    sqlx::query("insert into job_events (id, job_id, actor_id, kind, note, created_at) values ($1, $2, $3, 'paid', $4, $5)")
        .bind(Uuid::now_v7())
        .bind(job_id)
        .bind(p.payer)
        .bind(&note)
        .bind(at)
        .execute(&mut *tx)
        .await?;
    let accept_left: bool = sqlx::query_scalar(
        "select exists (select 1 from job_payments where job_id = $1 and due_on = 'accept' and status <> 'paid')",
    )
    .bind(job_id)
    .fetch_one(&mut *tx)
    .await?;
    if job_status == "awaiting_payment" && !accept_left {
        sqlx::query("update jobs set status = 'in_progress', started_at = $2, updated_at = $2 where id = $1")
            .bind(job_id)
            .bind(at)
            .execute(&mut *tx)
            .await?;
    } else if job_status == "delivered" && due_on == "delivery" {
        sqlx::query("update jobs set status = 'completed', completed_at = $2, updated_at = $2 where id = $1")
            .bind(job_id)
            .bind(at)
            .execute(&mut *tx)
            .await?;
        sqlx::query("insert into job_events (id, job_id, actor_id, kind, created_at) values ($1, $2, $3, 'completed', $4)")
            .bind(Uuid::now_v7())
            .bind(job_id)
            .bind(p.payer)
            .bind(at)
            .execute(&mut *tx)
            .await?;
    }
    maybe_credit(&mut tx, job_id, at).await?;
    tx.commit().await?;
    Ok(())
}

/// Stripe says a charge has `refunded_total` refunded: records the new part of it.
pub async fn refund_recorded(
    db: &sqlx::PgPool,
    charge: &str,
    refunded_total: i64,
) -> anyhow::Result<()> {
    let mut tx = db.begin().await?;
    let original: Option<(Uuid, Uuid, String, Option<String>)> = sqlx::query_as(
        "select p.id, jp.job_id, p.currency, p.stripe_account_id from payments p
         join job_payments jp on jp.id = p.job_payment_id
         where p.stripe_object_id = $1 and p.kind = 'people_job' for update of p",
    )
    .bind(charge)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((id, job_id, currency, account)) = original else {
        return Ok(());
    };
    let so_far: i64 = sqlx::query_scalar(
        "select coalesce(-sum(amount_minor), 0)::int8 from payments where refund_of = $1 and kind = 'refund'",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    let new = refunded_total - so_far;
    if new > 0 {
        sqlx::query(
            "insert into payments (id, kind, refund_of, stripe_account_id, stripe_object_id, amount_minor, currency)
             values ($1, 'refund', $2, $3, $4, $5, $6) on conflict (stripe_object_id) do nothing",
        )
        .bind(Uuid::now_v7())
        .bind(id)
        .bind(account)
        .bind(format!("{charge}:refunded:{refunded_total}"))
        .bind(-(new as i32))
        .bind(&currency)
        .execute(&mut *tx)
        .await?;
        event(
            &mut tx,
            job_id,
            None,
            "refunded",
            Some(&money(new, &currency)),
            &[],
            None,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn dispute_recorded(
    db: &sqlx::PgPool,
    charge: &str,
    dispute: &str,
    amount: i64,
) -> anyhow::Result<()> {
    let mut tx = db.begin().await?;
    let original: Option<(Uuid, Uuid, String)> = sqlx::query_as(
        "select p.id, jp.job_id, p.currency from payments p join job_payments jp on jp.id = p.job_payment_id
         where p.stripe_object_id = $1 and p.kind = 'people_job'",
    )
    .bind(charge)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((id, job_id, currency)) = original else {
        return Ok(());
    };
    let done = sqlx::query(
        "insert into payments (id, kind, refund_of, stripe_object_id, amount_minor, currency)
         values ($1, 'dispute', $2, $3, $4, $5) on conflict (stripe_object_id) do nothing",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(dispute)
    .bind(-(amount as i32))
    .bind(&currency)
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() > 0 {
        event(
            &mut tx,
            job_id,
            None,
            "disputed",
            Some(&money(amount, &currency)),
            &[],
            None,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Completes delivered jobs whose 7 days have passed (hourly, cleanup.rs).
pub async fn auto_complete(db: &sqlx::PgPool, now: OffsetDateTime) -> anyhow::Result<u64> {
    let ids: Vec<Uuid> =
        sqlx::query_scalar("select id from jobs where status = 'delivered' and delivered_at < $1")
            .bind(now - COMPLETION_WINDOW)
            .fetch_all(db)
            .await?;
    let mut n = 0;
    for id in ids {
        let mut tx = db.begin().await?;
        match finish(&mut tx, id, None, true, now).await {
            Ok(()) => {
                tx.commit().await?;
                n += 1;
            }
            Err(ApiError::Conflict(_)) => {}
            Err(e) => return Err(anyhow::anyhow!("{e}")),
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer() -> Offer {
        Offer {
            title: "Character sprite sheet".into(),
            scope_md: "Four directions, 8 frames each.".into(),
            price_minor: 12_001,
            currency: "usd".into(),
            payment: "split".into(),
            due_date: "2026-12-01".into(),
            revisions: 2,
            rights: "transfer".into(),
            open_source: false,
            credit: true,
            ai_use: "none".into(),
            refund_policy: String::new(),
        }
    }

    #[test]
    fn half_now_half_on_delivery_adds_up() {
        let s = schedule(&offer());
        assert_eq!(
            s,
            vec![("deposit", "accept", 6_001), ("final", "delivery", 6_000)]
        );
        assert_eq!(money(12_001, "usd"), "$120.01");
        assert_eq!(money(1_234_500, "gbp"), "£12,345");
    }

    #[test]
    fn offers_are_checked() {
        let today = Date::from_calendar_date(2026, time::Month::October, 19).unwrap();
        assert!(clean_offer(offer(), today).is_ok());
        let mut o = offer();
        o.price_minor = 500;
        assert!(clean_offer(o, today).is_err());
        let mut o = offer();
        o.due_date = "2026-10-01".into();
        assert!(clean_offer(o, today).is_err());
        let mut o = offer();
        o.rights = "work for hire".into();
        assert!(clean_offer(o, today).is_err());
        assert_eq!(
            clean_offer(offer(), today).unwrap().refund_policy,
            DEFAULT_REFUNDS
        );
    }

    #[test]
    fn the_agreement_says_what_was_agreed() {
        let p = Parties {
            job_id: Uuid::nil(),
            creative_name: "Vera",
            creative_handle: "vera",
            trader: true,
            client_name: "Octo",
            project_title: "Pixel Dungeon",
            project_slug: "pixel-dungeon",
            skill: "pixel-art",
            withdrawal_days: Some(14),
        };
        let text = agreement(&offer(), &p);
        assert!(!text.contains("{{"), "{text}");
        assert!(text.contains("transfers the copyright"));
        assert!(text.contains("isn't** covered by the project's open-source licence"));
        assert!(text.contains("$60.01 when you accept, $60 when the work is delivered"));
        assert!(text.contains("within 14 days"));
        assert!(text.contains("Slop isn't a party"));
        assert!(text.contains("\"Art by Vera\""));
        let mut o = offer();
        o.rights = "licence".into();
        o.open_source = true;
        let text = agreement(
            &o,
            &Parties {
                withdrawal_days: None,
                trader: false,
                ..p
            },
        );
        assert!(text.contains("non-exclusive licence"));
        assert!(text.contains("under the project's open-source licence"));
        assert!(!text.contains("Your right to cancel"));
        assert!(text.contains("private individual"));
    }
}
