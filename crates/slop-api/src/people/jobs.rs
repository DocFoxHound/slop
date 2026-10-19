//! Jobs (DESIGN §15.2–15.6, PAYMENTS §7):
//!
//! request (or inquiry) → offer → accept & pay → [sign-off on the direction] → deliver a preview
//! → accept & pay the rest → final files → completed.
//!
//! - **Exact wording.** The client acknowledges, on the request and again on accepting, that the
//!   creative delivers exactly what the agreement says, nothing more or less.
//! - **Changes are new work.** After acceptance, any change is a Change to Agreement: its own
//!   document with a price and a date, written by the creative and accepted by the client.
//! - **Conversations are recorded** and opened by the creative. The client may ask for one at
//!   any time; the creative decides whether and when (LEGAL-GUIDANCE HUM-17).
//! - The steps, notes, documents and conversations are the contract record (HUM-16), downloadable
//!   by both sides. Slop never decides disputes; mediation (mediation.rs) only suggests.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::header,
    response::IntoResponse,
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
pub const TEMPLATE_VERSION: &str = "2026-10-19b";
/// The client has this long after delivery to accept it or ask for a change (DESIGN §15.2).
pub const COMPLETION_WINDOW: Duration = Duration::days(7);
const MAX_OPEN_REQUESTS: i64 = 3;
const DEFAULT_REFUNDS: &str =
    "Full refund before work starts. After that, refunds cover the parts not yet delivered.";
/// What the client acknowledges before sending a request and again before accepting (DESIGN §15.2).
pub const EXACT_WORDING: &str = "The creative will deliver exactly what the agreement says: nothing more and nothing less. \
     Anything I don't write down isn't part of the work, and any change after accepting is new work with its own price and date.";
/// Shown above every job conversation and acknowledged before writing (DESIGN §15.5).
pub const RECORDING_NOTICE: &str = "This conversation is recorded, so you both have proof of what was said. Download your copy: \
     Slop keeps the text only while the conversation is open and for 30 days after it closes. After that Slop keeps only a \
     fingerprint of each message, which proves a downloaded copy hasn't been changed. Slop staff see the conversation only if \
     one of you submits your copy for a problem or a review dispute.";
/// How long Slop keeps a closed conversation's text for the two sides to download.
pub const CONVERSATION_KEPT: Duration = Duration::days(30);

/// Submitting a conversation copy can be larger than other requests (lib.rs gives it 2 MB).
pub fn evidence_routes() -> Router<AppState> {
    Router::new().route("/jobs/{id}/evidence", post(submit_evidence))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/people/{handle}/hire", post(hire))
        .route("/me/jobs", get(my_jobs))
        .route("/jobs/{id}", get(job))
        .route("/jobs/{id}/record", get(record))
        .route("/jobs/{id}/offer", post(offer))
        .route("/jobs/{id}/decline", post(decline))
        .route("/jobs/{id}/accept", post(accept))
        .route("/jobs/{id}/pay", post(pay))
        .route("/jobs/{id}/payment/check", post(check_payment))
        .route("/jobs/{id}/sign-off/send", post(send_for_sign_off))
        .route("/jobs/{id}/sign-off", post(sign_off))
        .route("/jobs/{id}/deliver", post(deliver))
        .route("/jobs/{id}/accept-delivery", post(accept_delivery))
        .route("/jobs/{id}/final-files", post(final_files))
        .route("/jobs/{id}/cancel", post(cancel))
        .route("/jobs/{id}/withdraw", post(withdraw))
        .route("/jobs/{id}/review", put(review))
        .route("/jobs/{id}/change/request", post(request_change))
        .route("/jobs/{id}/change/offer", post(offer_change))
        .route("/jobs/{id}/change/decline", post(decline_change))
        .route("/jobs/{id}/change/accept", post(accept_change))
        .route(
            "/jobs/{id}/conversation/request",
            post(request_conversation),
        )
        .route("/jobs/{id}/conversation/open", post(open_conversation))
        .route("/jobs/{id}/conversation/close", post(close_conversation))
        .route("/jobs/{id}/messages", post(post_message))
        .route("/jobs/{id}/conversation", get(conversation_copy))
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
    /// The client signs off on the direction (a sketch, a demo) before the rest is made.
    #[serde(default)]
    pub sign_off: bool,
    /// No longer used: changes are new work (Change to Agreement). Kept so older offers read.
    #[serde(default, skip_serializing)]
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

fn parse_due(field: &'static str, text: &str, today: Date) -> ApiResult<Date> {
    let due = Date::parse(text, &Iso8601::DATE)
        .map_err(|_| ApiError::invalid(field, "a date, YYYY-MM-DD"))?;
    if due < today || due > today + Duration::days(730) {
        return Err(ApiError::invalid(field, "from today to two years ahead"));
    }
    Ok(due)
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
    parse_due("due_date", &o.due_date, today)?;
    o.revisions = 0;
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
        "on_delivery" => format!(
            "all {} when you accept the delivered preview",
            price(o.price_minor)
        ),
        _ => {
            let s = schedule(o);
            format!(
                "{} when you accept, {} when you accept the delivered preview",
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
        format!("What: {} — exactly as written, nothing more or less", o.title),
        format!("Price: {}", money(o.price_minor, &o.currency)),
        format!("Payment: {}", payment_words(o)),
        format!("Due: {}", o.due_date),
        if o.sign_off {
            "Sign-off: you approve the direction first; changing it afterwards is new work".into()
        } else {
            "Sign-off: none; the creative goes straight to the finished work".into()
        },
        "Changes after you accept: new work, with its own price and date".into(),
        "Before the last payment you get a preview; the final files come after it".into(),
        match o.rights.as_str() {
            "transfer" => "Who owns it: you do, once it's fully paid (copyright is transferred)".into(),
            _ => "Who owns it: the creative; you get a licence to use and change it in this project, forever".into(),
        },
        if o.open_source {
            "Open source: may be published under your project's open-source licence".into()
        } else {
            "Open source: no; it stays all rights reserved, even in a public repository".into()
        },
        format!(
            "Credit: {}",
            if o.credit { "shown on your project's page" } else { "not shown" }
        ),
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
    let sign_off = if o.sign_off {
        "**Sign-off.** Before making the finished work, the creative sends the direction (for example a sketch, a \
         rough cut or a sample) for the client to sign off on the job page. Once signed off, the direction is part of \
         this agreement: changing it is a Change to Agreement (section 2)."
    } else {
        ""
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
    let fields: [(&str, String); 20] = [
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
        ("ai_use_line", ai_words(&o.ai_use).to_string()),
        ("sign_off_line", sign_off.to_string()),
        ("price", money(o.price_minor, &o.currency)),
        ("payment_line", payment_words(o) + "."),
        ("rights_line", rights),
        ("open_source_line", open_source),
        ("credit_line", credit),
        ("refund_policy", o.refund_policy.clone()),
        ("withdrawal_line", withdrawal),
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

/// A Change to Agreement, as both sides see and sign it.
pub struct ChangeTerms<'a> {
    pub number: i32,
    pub title: &'a str,
    pub scope_md: &'a str,
    pub extra_price_minor: i64,
    pub currency: &'a str,
    pub new_due_date: &'a str,
    pub requested_note: Option<&'a str>,
}

pub fn change_document(c: &ChangeTerms, p: &Parties, agreement_sha: &str) -> String {
    let price = if c.extra_price_minor > 0 {
        format!(
            "**Extra price:** {}, paid to the creative when this change is accepted.",
            money(c.extra_price_minor, c.currency)
        )
    } else {
        "**Extra price:** none.".to_string()
    };
    let asked = c
        .requested_note
        .filter(|n| !n.is_empty())
        .map(|n| {
            format!(
                "\nThe client asked for this change:\n\n> {}\n",
                n.replace('\n', "\n> ")
            )
        })
        .unwrap_or_default();
    format!(
        "# Change to Agreement {n}\n\nJob {job} · changes the Service Agreement with SHA-256 `{agreement_sha}`.\n\n\
         Between **{creative}** (@{handle}, \"the creative\") and **{client}** (\"the client\"), for **{project}**.\n\n\
         This changes the Service Agreement **only as written here**; everything else stays as agreed. The work below is \
         new work: the creative delivers exactly what's written, nothing more or less.\n{asked}\n\
         ## The change\n\n**{title}**\n\n{scope}\n\n- {price}\n- **New due date:** {due}.\n\n\
         Sending this is the creative's signature; pressing **Accept** is the client's.\n",
        n = c.number,
        job = p.job_id,
        creative = p.creative_name,
        handle = p.creative_handle,
        client = p.client_name,
        project = p.project_title,
        title = c.title,
        scope = c.scope_md,
        due = c.new_due_date,
    )
}

// ---------------------------------------------------------------------------------------------
// Loading a job

#[derive(sqlx::FromRow)]
pub(crate) struct JobRow {
    pub id: Uuid,
    pub creative_id: Uuid,
    pub client_id: Uuid,
    skill: String,
    pub status: String,
    request_md: String,
    wanted_by: Option<Date>,
    client_is_business: bool,
    inquiry: bool,
    #[sqlx(json(nullable))]
    pub offer: Option<Offer>,
    offer_count: i32,
    agreement_text: Option<String>,
    agreement_sha256: Option<String>,
    agreement_template: Option<String>,
    accepted_at: Option<OffsetDateTime>,
    due_date: Option<Date>,
    withdrawal_until: Option<OffsetDateTime>,
    start_now_ack_at: Option<OffsetDateTime>,
    signed_off_at: Option<OffsetDateTime>,
    delivered_at: Option<OffsetDateTime>,
    final_files_at: Option<OffsetDateTime>,
    completed_at: Option<OffsetDateTime>,
    completed_automatically: bool,
    credited_at: Option<OffsetDateTime>,
    conversation_open: bool,
    conversation_requested_at: Option<OffsetDateTime>,
    conversation_request_note: Option<String>,
    conversation_closed_at: Option<OffsetDateTime>,
    client_recording_ack_at: Option<OffsetDateTime>,
    creative_recording_ack_at: Option<OffsetDateTime>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    pub creative_name: String,
    pub creative_handle: String,
    trader: bool,
    creative_ai_use: String,
    stripe_account_id: Option<String>,
    pub client_name: String,
    client_country: String,
    pub project_title: String,
    pub project_slug: String,
    project_stage: String,
}

const JOB_SELECT: &str = "select j.id, j.creative_id, j.client_id, j.app_id, j.skill, j.status, j.request_md,
        j.wanted_by, j.client_is_business, j.inquiry, j.offer, j.offer_count, j.agreement_text, j.agreement_sha256,
        j.agreement_template, j.accepted_at, j.due_date, j.withdrawal_until, j.start_now_ack_at, j.signed_off_at,
        j.delivered_at, j.final_files_at, j.completed_at, j.completed_automatically, j.credited_at,
        j.conversation_open, j.conversation_requested_at, j.conversation_request_note, j.conversation_closed_at,
        j.client_recording_ack_at, j.creative_recording_ack_at, j.created_at, j.updated_at,
        c.name as creative_name, c.handle as creative_handle, c.trader, c.ai_use as creative_ai_use,
        c.stripe_account_id, u.display_name as client_name, u.country_code as client_country,
        a.title as project_title, a.slug as project_slug, a.stage::text as project_stage
    from jobs j join creatives c on c.user_id = j.creative_id join users u on u.id = j.client_id
    join apps a on a.id = j.app_id";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Role {
    Client,
    Creative,
}

pub(crate) async fn load_any(db: &sqlx::PgPool, id: Uuid) -> ApiResult<Option<JobRow>> {
    Ok(
        sqlx::query_as(sqlx::AssertSqlSafe(format!("{JOB_SELECT} where j.id = $1")))
            .bind(id)
            .fetch_optional(db)
            .await?,
    )
}

/// The job, if the caller is one of its two sides. Anyone else gets 404.
pub(crate) async fn load(
    state: &AppState,
    id: Uuid,
    user: &CurrentUser,
) -> ApiResult<(JobRow, Role)> {
    match load_any(&state.db, id).await? {
        Some(j) if j.client_id == user.id => Ok((j, Role::Client)),
        Some(j) if j.creative_id == user.id => Ok((j, Role::Creative)),
        _ => Err(ApiError::NotFound),
    }
}

const ACTIVE: &[&str] = &[
    "awaiting_payment",
    "in_progress",
    "awaiting_sign_off",
    "delivered",
    "final_files",
];
const ENDED: &[&str] = &["declined", "cancelled", "withdrawn"];

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
    /// The change this pays for.
    change_number: Option<i32>,
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

#[derive(Serialize, sqlx::FromRow)]
struct MessageView {
    id: Uuid,
    author: String,
    mine: bool,
    /// Null once Slop has deleted it (30 days after the conversation closed).
    body: Option<String>,
    sha256: String,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

#[derive(Serialize, sqlx::FromRow)]
pub(crate) struct ChangeView {
    id: Uuid,
    number: i32,
    status: String,
    requested_note: Option<String>,
    title: Option<String>,
    scope_md: Option<String>,
    extra_price_minor: Option<i32>,
    /// YYYY-MM-DD.
    new_due_date: Option<String>,
    decline_note: Option<String>,
    text: Option<String>,
    sha256: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    decided_at: Option<OffsetDateTime>,
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
struct Conversation {
    open: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    requested_at: Option<OffsetDateTime>,
    request_note: Option<String>,
    /// When Slop deletes the text (a closed conversation), so both sides know to download it.
    #[serde(with = "time::serde::rfc3339::option")]
    text_deleted_after: Option<OffsetDateTime>,
    /// The viewer acknowledged that it's recorded.
    acknowledged: bool,
    notice: &'static str,
    messages: Vec<MessageView>,
}

#[derive(Serialize)]
struct MyReview {
    vote: &'static str,
    body: String,
    reply: Option<String>,
    hidden: bool,
    mediator_note: Option<String>,
    /// The creative's dispute about it, if any: pending, removed, noted or kept.
    dispute: Option<Value>,
}

/// For the creative: how this client has paid before (facts, not opinions).
#[derive(Serialize, sqlx::FromRow)]
pub struct ClientRecord {
    pub completed_jobs: i64,
    pub paid_in_full: i64,
    pub owing: i64,
}

pub async fn client_record(db: &sqlx::PgPool, client: Uuid) -> sqlx::Result<ClientRecord> {
    sqlx::query_as(
        "select count(*) as completed_jobs,
                count(*) filter (where not exists (select 1 from job_payments p where p.job_id = j.id and p.status <> 'paid')) as paid_in_full,
                count(*) filter (where exists (select 1 from job_payments p where p.job_id = j.id and p.status = 'due')) as owing
         from jobs j where j.client_id = $1 and j.status in ('completed', 'final_files')",
    )
    .bind(client)
    .fetch_one(db)
    .await
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
    inquiry: bool,
    offer: Option<Offer>,
    offer_count: i32,
    summary: Vec<String>,
    agreement: Option<Agreement>,
    /// Changes to Agreement, oldest first.
    changes: Vec<ChangeView>,
    due_date: Option<String>,
    payments: Vec<PaymentView>,
    paid_minor: i64,
    refunded_minor: i64,
    /// For the creative: about what they'll receive after Slop's fee (Stripe's card fee is extra).
    creative_receives_minor: Option<i64>,
    withdrawal: Option<Withdrawal>,
    /// Before acceptance: the consumer cancellation period that will apply, in days.
    withdrawal_days: Option<i16>,
    #[serde(with = "time::serde::rfc3339::option")]
    signed_off_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    final_files_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    auto_complete_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    completed_at: Option<OffsetDateTime>,
    completed_automatically: bool,
    credited: bool,
    conversation: Conversation,
    problem: Option<Value>,
    /// For the creative: the client's payment record.
    client_record: Option<ClientRecord>,
    events: Vec<EventView>,
    /// What the viewer can do now.
    actions: Vec<&'static str>,
    payment_mode: &'static str,
    review: Option<MyReview>,
    exact_wording: &'static str,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

fn status_text(j: &JobRow, role: Role, due: Option<&PaymentView>) -> String {
    let c = &j.creative_name;
    let me_client = role == Role::Client;
    let owed = |p: &PaymentView| money(p.amount_minor.into(), &p.currency);
    match j.status.as_str() {
        "requested" if me_client && j.inquiry => {
            format!(
                "You asked {c} to look at your request. They can open a conversation or send an offer."
            )
        }
        "requested" if me_client => format!("Waiting for {c} to send an offer."),
        "requested" if j.inquiry => {
            "They'd like your view before an offer: open a conversation, or send an offer.".into()
        }
        "requested" => "They're waiting for your offer.".into(),
        "offered" if me_client => {
            format!("{c} sent an offer. Read it carefully: it's exactly what you'll get.")
        }
        "offered" => "Offer sent. Waiting for the client.".into(),
        "awaiting_payment" if me_client => match due {
            Some(p) => format!("Pay {} to start the work.", owed(p)),
            None => "Accepted.".into(),
        },
        "awaiting_payment" => "Waiting for the client's payment before you start.".into(),
        "in_progress" if me_client => format!("{c} is working on it."),
        "in_progress" => {
            if j.offer.as_ref().is_some_and(|o| o.sign_off) && j.signed_off_at.is_none() {
                "In progress. Send the direction for sign-off first.".into()
            } else {
                "In progress. Deliver it when it's ready.".into()
            }
        }
        "awaiting_sign_off" if me_client => {
            format!("{c} sent the direction. Sign off on it, or ask for a change.")
        }
        "awaiting_sign_off" => "Waiting for the client to sign off on the direction.".into(),
        "delivered" if me_client => match due {
            Some(p) => format!(
                "Delivered (a preview). If it matches the agreement, pay the remaining {} and {c} sends the final files.",
                owed(p)
            ),
            None => "Delivered. If it matches the agreement, accept it.".into(),
        },
        "delivered" => "Delivered. Waiting for the client to accept.".into(),
        "final_files" if me_client => {
            format!("Paid in full. Waiting for {c} to send the final files.")
        }
        "final_files" => "Paid in full. Send the final files.".into(),
        "completed" if due.is_some() => "Completed. A payment is still owed.".into(),
        "completed" => "Completed.".into(),
        "declined" => "Declined.".into(),
        "cancelled" => format!("Cancelled. Any refund comes from {c} through Stripe."),
        "withdrawn" => format!("Withdrawn by the client. {c} refunds what's owed within 14 days."),
        other => other.to_string(),
    }
}

// Row tuples straight from SQL, read once below.
#[allow(clippy::type_complexity)]
async fn view(state: &AppState, j: JobRow, role: Role, viewer: Uuid) -> ApiResult<JobView> {
    let payments: Vec<PaymentView> = sqlx::query_as(
        "select jp.part, jp.due_on, jp.amount_minor, jp.currency, jp.status, jp.paid_at,
                coalesce(bool_or(p.simulated), false) as simulated, ch.number as change_number
         from job_payments jp left join payments p on p.job_payment_id = jp.id and p.kind = 'people_job'
         left join job_changes ch on ch.id = jp.change_id
         where jp.job_id = $1 group by jp.id, ch.number order by coalesce(ch.number, 0), jp.due_on, jp.part",
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
    let messages: Vec<MessageView> = sqlx::query_as(
        "select m.id, u.display_name as author, m.author_id = $2 as mine, m.body, m.sha256, m.created_at
         from job_messages m join users u on u.id = m.author_id where m.job_id = $1 order by m.created_at, m.id",
    )
    .bind(j.id)
    .bind(viewer)
    .fetch_all(&state.db)
    .await?;
    let changes: Vec<ChangeView> = sqlx::query_as(
        "select id, number, status, requested_note, title, scope_md, extra_price_minor, new_due_date::text, decline_note,
                text, sha256, created_at, decided_at
         from job_changes where job_id = $1 order by number",
    )
    .bind(j.id)
    .fetch_all(&state.db)
    .await?;
    let review: Option<(i16, String, Option<String>, bool, Option<String>)> = sqlx::query_as(
        "select vote, body, reply, hidden_at is not null, mediator_note from creative_reviews where job_id = $1",
    )
    .bind(j.id)
    .fetch_optional(&state.db)
    .await?;
    let dispute: Option<Value> = sqlx::query_scalar(
        "select json_build_object('reason', reason, 'statement', statement, 'client_statement', client_statement,
                                  'status', status, 'decision_reason', decision_reason)
         from review_disputes where job_id = $1",
    )
    .bind(j.id)
    .fetch_optional(&state.db)
    .await?;
    let problem = super::mediation::problem_view(&state.db, j.id, viewer).await?;
    let now = OffsetDateTime::now_utc();
    let due = payments.iter().find(|p| p.status == "due");
    let active = ACTIVE.contains(&j.status.as_str());
    let ended = ENDED.contains(&j.status.as_str());
    let withdrawal = j.withdrawal_until.map(|until| Withdrawal {
        until,
        open: until > now && active,
        started_early: j.start_now_ack_at.is_some(),
    });
    let open_change = changes
        .iter()
        .find(|c| c.status == "requested" || c.status == "offered");
    let needs_sign_off = j.offer.as_ref().is_some_and(|o| o.sign_off) && j.signed_off_at.is_none();
    let mut actions: Vec<&'static str> = Vec::new();
    match (role, j.status.as_str()) {
        (Role::Creative, "requested" | "offered") => actions.extend(["offer", "decline"]),
        (Role::Client, "requested") => actions.push("decline"),
        (Role::Client, "offered") => actions.extend(["accept", "decline"]),
        (Role::Client, "awaiting_payment") => actions.extend(["pay", "cancel"]),
        (Role::Creative, "awaiting_payment") => actions.push("cancel"),
        (Role::Creative, "in_progress") => {
            actions.push(if needs_sign_off {
                "send_for_sign_off"
            } else {
                "deliver"
            });
            actions.push("cancel");
        }
        (Role::Client, "awaiting_sign_off") => actions.push("sign_off"),
        (Role::Creative, "awaiting_sign_off" | "delivered") => actions.push("cancel"),
        (Role::Client, "delivered") => actions.push(if due.is_some() {
            "pay"
        } else {
            "accept_delivery"
        }),
        (Role::Creative, "final_files") => actions.push("final_files"),
        (Role::Client, "completed") => {
            if due.is_some() {
                actions.push("pay");
            }
            actions.push("review");
        }
        _ => {}
    }
    // Changes to Agreement: while there's work under way.
    let changeable = ["in_progress", "awaiting_sign_off", "delivered"].contains(&j.status.as_str());
    match (role, open_change.map(|c| c.status.as_str())) {
        (Role::Client, None) if changeable => actions.push("request_change"),
        (Role::Creative, None) if changeable => actions.push("propose_change"),
        (Role::Creative, Some("requested")) => actions.extend(["offer_change", "decline_change"]),
        (Role::Creative, Some("offered")) => actions.push("withdraw_change"),
        (Role::Client, Some("offered")) => actions.extend(["accept_change", "decline_change"]),
        _ => {}
    }
    // Conversations: the client asks, the creative opens.
    match role {
        Role::Client if !j.conversation_open && j.conversation_requested_at.is_none() => {
            actions.push("request_conversation")
        }
        Role::Creative if !j.conversation_open => actions.push("open_conversation"),
        Role::Creative => actions.push("close_conversation"),
        _ => {}
    }
    if j.conversation_open {
        actions.push("message");
    }
    if role == Role::Client && withdrawal.as_ref().is_some_and(|w| w.open) {
        actions.push("withdraw");
    }
    let problem_open = problem.as_ref().is_some_and(|p| p["status"] == "open");
    if j.accepted_at.is_some() && !ended && !problem_open {
        actions.push("open_problem");
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
    let acknowledged = match role {
        Role::Client => j.client_recording_ack_at.is_some(),
        Role::Creative => j.creative_recording_ack_at.is_some(),
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
        inquiry: j.inquiry,
        creative_receives_minor: match (role, &j.offer) {
            (Role::Creative, Some(o)) => Some(o.price_minor - stripe::fee(o.price_minor)),
            _ => None,
        },
        offer: j.offer.clone(),
        offer_count: j.offer_count,
        summary: summary_lines,
        agreement,
        changes,
        due_date: j
            .due_date
            .map(|d| d.to_string())
            .or_else(|| j.offer.as_ref().map(|o| o.due_date.clone())),
        payments,
        paid_minor: paid,
        // Refunds are stored as negative amounts.
        refunded_minor: -refunded,
        withdrawal,
        withdrawal_days,
        signed_off_at: j.signed_off_at,
        final_files_at: j.final_files_at,
        auto_complete_at: (j.status == "delivered")
            .then(|| j.delivered_at.map(|d| d + COMPLETION_WINDOW))
            .flatten(),
        completed_at: j.completed_at,
        completed_automatically: j.completed_automatically,
        credited: j.credited_at.is_some(),
        conversation: Conversation {
            open: j.conversation_open,
            requested_at: j.conversation_requested_at,
            request_note: j.conversation_request_note.clone(),
            text_deleted_after: (!j.conversation_open)
                .then(|| j.conversation_closed_at.map(|c| c + CONVERSATION_KEPT))
                .flatten(),
            acknowledged,
            notice: RECORDING_NOTICE,
            messages,
        },
        problem,
        client_record: match role {
            Role::Creative => Some(client_record(&state.db, j.client_id).await?),
            Role::Client => None,
        },
        events,
        actions,
        payment_mode: stripe::mode(state).name(),
        review: review.map(|(vote, body, reply, hidden, mediator_note)| MyReview {
            vote: if vote > 0 { "not_slop" } else { "slop" },
            body,
            reply,
            hidden,
            mediator_note,
            dispute,
        }),
        exact_wording: EXACT_WORDING,
        created_at: j.created_at,
    })
}

pub(crate) async fn parties<'a>(state: &AppState, j: &'a JobRow) -> ApiResult<Parties<'a>> {
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

pub(crate) async fn reload(
    state: &AppState,
    id: Uuid,
    user: &CurrentUser,
) -> ApiResult<Json<JobView>> {
    let (j, role) = load(state, id, user).await?;
    Ok(Json(view(state, j, role, user.id).await?))
}

pub(crate) async fn event(
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

pub(crate) fn note(
    field: &'static str,
    text: &str,
    max: usize,
    required: bool,
) -> ApiResult<String> {
    let t = text.trim();
    if required && t.is_empty() {
        return Err(ApiError::invalid(field, "write it out"));
    }
    if t.chars().count() > max {
        return Err(ApiError::invalid(field, format!("up to {max} characters")));
    }
    Ok(t.to_string())
}

pub(crate) fn wrong_step(what: &str) -> ApiError {
    ApiError::Conflict(format!("the job isn't at the step where you can {what}"))
}

/// Changes the status only if it's still one of `from` (two people may press buttons at once).
pub(crate) async fn set_status(
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

fn https_links(links: &[String]) -> ApiResult<Vec<String>> {
    if links.len() > 5 {
        return Err(ApiError::invalid("links", "up to 5 links"));
    }
    links
        .iter()
        .map(|l| {
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
            Ok(url.to_string())
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Request (or inquiry), offer, accept

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
    /// "Ask about this first": ask the creative to review the request and talk before an offer.
    #[serde(default)]
    inquiry: bool,
    /// The client read EXACT_WORDING (an unticked box).
    #[serde(default)]
    exact_wording_ack: bool,
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
    if !body.exact_wording_ack {
        return Err(ApiError::invalid(
            "exact_wording_ack",
            "confirm you understand the creative delivers exactly what's written",
        ));
    }
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
    // Pay what you owe before hiring anyone again (DESIGN §15.6).
    let owing: Option<(String, i64, String)> = sqlx::query_as(
        "select c.name, sum(p.amount_minor)::int8, p.currency from jobs j join job_payments p on p.job_id = j.id
         join creatives c on c.user_id = j.creative_id
         where j.client_id = $1 and j.status in ('completed', 'final_files') and p.status = 'due'
         group by c.name, p.currency limit 1",
    )
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?;
    if let Some((name, amount, currency)) = owing {
        return Err(ApiError::Conflict(format!(
            "you still owe {name} {} for finished work; pay it from your jobs before hiring again",
            money(amount, &currency)
        )));
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
        "insert into jobs (id, creative_id, client_id, app_id, skill, status, request_md, wanted_by,
                           client_is_business, inquiry, request_ack_at, conversation_requested_at, conversation_request_note)
         values ($1, $2, $3, $4, $5, 'requested', $6, $7, $8, $9, now(), $10, $11)",
    )
    .bind(id)
    .bind(creative_id)
    .bind(user.id)
    .bind(app_id)
    .bind(&body.skill)
    .bind(&request)
    .bind(wanted_by)
    .bind(body.client_is_business)
    .bind(body.inquiry)
    .bind(body.inquiry.then(OffsetDateTime::now_utc))
    .bind(body.inquiry.then_some(
        "Please review my request: your opinion, how long it would take, and anything unclear.",
    ))
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
        if body.inquiry { "inquiry" } else { "requested" },
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
    inquiry: bool,
    conversation_requested: bool,
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
    // Open changes waiting on someone: (job, status).
    let changes: Vec<(Uuid, String)> = sqlx::query_as(
        "select ch.job_id, ch.status from job_changes ch join jobs j on j.id = ch.job_id
         where ch.status in ('requested', 'offered') and (j.client_id = $1 or j.creative_id = $1)",
    )
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
                change_number: None,
            });
        let change = changes
            .iter()
            .find(|(id, _)| *id == j.id)
            .map(|(_, s)| s.as_str());
        let needs_you = match (role, j.status.as_str()) {
            (Role::Creative, "requested" | "in_progress" | "final_files") => true,
            (Role::Client, "offered" | "awaiting_payment" | "awaiting_sign_off" | "delivered") => {
                true
            }
            (Role::Client, "completed") => due_here.is_some() || !reviewed.contains(&j.id),
            _ => false,
        } || matches!(
            (role, change),
            (Role::Creative, Some("requested")) | (Role::Client, Some("offered"))
        ) || (role == Role::Creative
            && !j.conversation_open
            && j.conversation_requested_at.is_some());
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
            inquiry: j.inquiry,
            conversation_requested: !j.conversation_open && j.conversation_requested_at.is_some(),
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
pub(crate) struct NoteBody {
    #[serde(default)]
    pub note: String,
}

/// Before acceptance: the client withdraws the request or turns the offer down; the creative
/// declines the work. A note explains why; the creative can still send a new offer.
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
    sqlx::query(
        "update jobs set ended_at = now(), ended_by = $2, conversation_open = false,
                conversation_closed_at = case when conversation_open then now() else conversation_closed_at end
         where id = $1",
    )
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptBody {
    /// The offer the client saw (it may have changed since).
    offer_count: i32,
    /// "Start now. I understand I lose my right to cancel once the work is fully done." Only
    /// asked when a withdrawal right applies; never pre-ticked (HUM-2).
    #[serde(default)]
    start_now: bool,
    /// The client read EXACT_WORDING again, for these exact terms.
    #[serde(default)]
    exact_terms_ack: bool,
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
    if !body.exact_terms_ack {
        return Err(ApiError::invalid(
            "exact_terms_ack",
            "confirm you understand you'll get exactly what the agreement says",
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
    let due_date = Date::parse(&o.due_date, &Iso8601::DATE).ok();
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
                withdrawal_until = $6, start_now_ack_at = $7, started_at = $8, exact_terms_ack_at = $5, due_date = $9
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
    .bind(due_date)
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

// ---------------------------------------------------------------------------------------------
// Paying

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
    if role != Role::Client || ENDED.contains(&j.status.as_str()) {
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
        "change" => "change to agreement",
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

// ---------------------------------------------------------------------------------------------
// Sign-off, delivery, final files

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FilesBody {
    #[serde(default)]
    note: String,
    /// Where the files are (any https links), up to 5.
    #[serde(default)]
    links: Vec<String>,
}

fn files(body: &FilesBody) -> ApiResult<(String, Vec<String>)> {
    let text = note("note", &body.note, 2000, false)?;
    let links = https_links(&body.links)?;
    if text.is_empty() && links.is_empty() {
        return Err(ApiError::invalid(
            "links",
            "add a link to the files, or a note saying where they are",
        ));
    }
    Ok((text, links))
}

/// The creative sends the direction (a sketch, a sample) for the client to sign off.
async fn send_for_sign_off(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<FilesBody>,
) -> ApiResult<Json<JobView>> {
    let (j, role) = load(&state, id, &user).await?;
    let wanted = j.offer.as_ref().is_some_and(|o| o.sign_off) && j.signed_off_at.is_none();
    if role != Role::Creative || !wanted {
        return Err(wrong_step("send the direction for sign-off"));
    }
    let (text, links) = files(&body)?;
    let mut tx = state.db.begin().await?;
    set_status(&mut tx, id, &["in_progress"], "awaiting_sign_off").await?;
    event(
        &mut tx,
        id,
        Some(user.id),
        "sent_for_sign_off",
        Some(&text),
        &links,
        None,
    )
    .await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

/// The client signs off on the direction: from now on, changing it is a Change to Agreement.
async fn sign_off(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<JobView>> {
    let (_, role) = load(&state, id, &user).await?;
    if role != Role::Client {
        return Err(wrong_step("sign off"));
    }
    let mut tx = state.db.begin().await?;
    set_status(&mut tx, id, &["awaiting_sign_off"], "in_progress").await?;
    sqlx::query("update jobs set signed_off_at = now() where id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    event(&mut tx, id, Some(user.id), "signed_off", None, &[], None).await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

/// Delivery. When money is still owed it's a preview; the final files follow the last payment.
async fn deliver(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<FilesBody>,
) -> ApiResult<Json<JobView>> {
    let (j, role) = load(&state, id, &user).await?;
    if role != Role::Creative {
        return Err(wrong_step("deliver"));
    }
    if j.offer.as_ref().is_some_and(|o| o.sign_off) && j.signed_off_at.is_none() {
        return Err(ApiError::Conflict(
            "the client signs off on the direction first".into(),
        ));
    }
    let (text, links) = files(&body)?;
    let mut tx = state.db.begin().await?;
    set_status(&mut tx, id, &["in_progress"], "delivered").await?;
    sqlx::query("update jobs set delivered_at = now() where id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let owed = sqlx::query(
        "update job_payments set status = 'due' where job_id = $1 and due_on = 'delivery' and status = 'waiting'",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    event(
        &mut tx,
        id,
        Some(user.id),
        if owed > 0 {
            "delivered_preview"
        } else {
            "delivered"
        },
        Some(&text),
        &links,
        None,
    )
    .await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

/// Accepting a delivery when nothing more is owed (otherwise paying is the acceptance).
async fn accept_delivery(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<JobView>> {
    let (_, role) = load(&state, id, &user).await?;
    if role != Role::Client {
        return Err(wrong_step("accept the delivery"));
    }
    let owed: bool = sqlx::query_scalar(
        "select exists (select 1 from job_payments where job_id = $1 and status = 'due')",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    if owed {
        return Err(ApiError::Conflict(
            "pay what's owed to accept the delivery".into(),
        ));
    }
    let mut tx = state.db.begin().await?;
    finish(&mut tx, id, Some(user.id), false, OffsetDateTime::now_utc()).await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

/// The creative sends the final files after the last payment, which completes the job.
async fn final_files(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<FilesBody>,
) -> ApiResult<Json<JobView>> {
    let (_, role) = load(&state, id, &user).await?;
    if role != Role::Creative {
        return Err(wrong_step("send the final files"));
    }
    let (text, links) = files(&body)?;
    let now = OffsetDateTime::now_utc();
    let mut tx = state.db.begin().await?;
    send_final_files(&mut tx, id, user.id, &text, &links, now).await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

pub async fn send_final_files(
    conn: &mut sqlx::PgConnection,
    id: Uuid,
    creative: Uuid,
    note: &str,
    links: &[String],
    at: OffsetDateTime,
) -> ApiResult<()> {
    set_status(conn, id, &["final_files"], "completed").await?;
    sqlx::query("update jobs set final_files_at = $2, completed_at = coalesce(completed_at, $2) where id = $1")
        .bind(id)
        .bind(at)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "insert into job_events (id, job_id, actor_id, kind, note, links, created_at) values ($1, $2, $3, 'final_files', $4, $5, $6)",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(creative)
    .bind(Some(note).filter(|n| !n.is_empty()))
    .bind(links)
    .bind(at)
    .execute(&mut *conn)
    .await?;
    maybe_credit(conn, id, at).await?;
    Ok(())
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

// ---------------------------------------------------------------------------------------------
// Cancelling, withdrawing, reviewing

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
        Role::Creative => [
            "awaiting_payment",
            "in_progress",
            "awaiting_sign_off",
            "delivered",
        ]
        .contains(&j.status.as_str()),
        // Before paying anything, the client can still back out.
        Role::Client => j.status == "awaiting_payment" && !paid_any,
    };
    if !allowed {
        return Err(wrong_step("cancel"));
    }
    let text = note("note", &body.note, 1000, role == Role::Creative)?;
    let mut tx = state.db.begin().await?;
    end_job(&mut tx, id, user.id, "cancelled", Some(&text)).await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

/// Ends an active job (cancelled or withdrawn): nothing more is due, open changes lapse.
pub(crate) async fn end_job(
    conn: &mut sqlx::PgConnection,
    id: Uuid,
    by: Uuid,
    status: &str,
    text: Option<&str>,
) -> ApiResult<()> {
    set_status(conn, id, ACTIVE, status).await?;
    sqlx::query(
        "update jobs set ended_at = now(), ended_by = $2, withdrawn_at = case when $3 = 'withdrawn' then now() end,
                conversation_closed_at = case when conversation_open then now() else conversation_closed_at end,
                conversation_open = false
         where id = $1",
    )
    .bind(id)
    .bind(by)
    .bind(status)
    .execute(&mut *conn)
    .await?;
    sqlx::query("update job_payments set status = 'waiting' where job_id = $1 and status = 'due'")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "update job_changes set status = 'withdrawn', decided_at = now() where job_id = $1 and status in ('requested', 'offered')",
    )
    .bind(id)
    .execute(&mut *conn)
    .await?;
    event(conn, id, Some(by), status, text, &[], None).await?;
    Ok(())
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
    end_job(&mut tx, id, user.id, "withdrawn", None).await?;
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
    if role != Role::Client || !["completed", "final_files"].contains(&j.status.as_str()) {
        return Err(ApiError::Conflict(
            "you can review once the work is delivered and accepted".into(),
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
// Changes to Agreement: new work, its own document, price and date

async fn request_change(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<NoteBody>,
) -> ApiResult<Json<JobView>> {
    let (j, role) = load(&state, id, &user).await?;
    if role != Role::Client
        || !["in_progress", "awaiting_sign_off", "delivered"].contains(&j.status.as_str())
    {
        return Err(wrong_step("ask for a change"));
    }
    let text = note("note", &body.note, 2000, true)?;
    let mut tx = state.db.begin().await?;
    let number = next_change_number(&mut tx, id).await?;
    let inserted = sqlx::query(
        "insert into job_changes (id, job_id, number, requested_note, status) values ($1, $2, $3, $4, 'requested')
         on conflict do nothing",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(number)
    .bind(&text)
    .execute(&mut *tx)
    .await?;
    if inserted.rows_affected() == 0 {
        return Err(ApiError::Conflict(
            "there's already a change waiting for an answer".into(),
        ));
    }
    event(
        &mut tx,
        id,
        Some(user.id),
        "change_requested",
        Some(&text),
        &[],
        None,
    )
    .await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

async fn next_change_number(conn: &mut sqlx::PgConnection, job: Uuid) -> sqlx::Result<i32> {
    sqlx::query_scalar("select coalesce(max(number), 0) + 1 from job_changes where job_id = $1")
        .bind(job)
        .fetch_one(conn)
        .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeOffer {
    pub title: String,
    #[serde(default)]
    pub scope_md: String,
    /// Zero for no charge.
    pub extra_price_minor: i64,
    /// YYYY-MM-DD.
    pub new_due_date: String,
}

/// The creative writes the Change to Agreement: answering the client's request, or on their own.
async fn offer_change(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(c): Json<ChangeOffer>,
) -> ApiResult<Json<JobView>> {
    let (j, role) = load(&state, id, &user).await?;
    if role != Role::Creative
        || !["in_progress", "awaiting_sign_off", "delivered"].contains(&j.status.as_str())
    {
        return Err(wrong_step("send a change to the agreement"));
    }
    let o = j
        .offer
        .as_ref()
        .ok_or_else(|| wrong_step("send a change"))?;
    let title = note("title", &c.title, 100, true)?;
    let scope = note("scope_md", &c.scope_md, 4000, false)?;
    if c.extra_price_minor != 0 {
        check_price("extra_price_minor", c.extra_price_minor, &o.currency)?;
    }
    let due = parse_due(
        "new_due_date",
        &c.new_due_date,
        OffsetDateTime::now_utc().date(),
    )?;
    let mut tx = state.db.begin().await?;
    let open: Option<(Uuid, String)> = sqlx::query_as(
        "select id, status from job_changes where job_id = $1 and status in ('requested', 'offered') for update",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let change_id = match open {
        Some((cid, status)) if status == "requested" => cid,
        Some(_) => {
            return Err(ApiError::Conflict(
                "a change is already waiting for the client".into(),
            ));
        }
        None => {
            let cid = Uuid::now_v7();
            let number = next_change_number(&mut tx, id).await?;
            sqlx::query(
                "insert into job_changes (id, job_id, number, status) values ($1, $2, $3, 'requested')",
            )
            .bind(cid)
            .bind(id)
            .bind(number)
            .execute(&mut *tx)
            .await?;
            cid
        }
    };
    sqlx::query(
        "update job_changes set status = 'offered', title = $2, scope_md = $3, extra_price_minor = $4,
                new_due_date = $5, offered_at = now() where id = $1",
    )
    .bind(change_id)
    .bind(&title)
    .bind(&scope)
    .bind(c.extra_price_minor as i32)
    .bind(due)
    .execute(&mut *tx)
    .await?;
    event(
        &mut tx,
        id,
        Some(user.id),
        "change_offered",
        Some(&format!(
            "{title} · {} · due {due}",
            if c.extra_price_minor > 0 {
                money(c.extra_price_minor, &o.currency)
            } else {
                "no extra charge".into()
            }
        )),
        &[],
        None,
    )
    .await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

/// The creative declines a requested change (the work stays as agreed) or withdraws their
/// offered one; the client declines an offered change.
async fn decline_change(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<NoteBody>,
) -> ApiResult<Json<JobView>> {
    let (_, role) = load(&state, id, &user).await?;
    let text = note("note", &body.note, 1000, false)?;
    let mut tx = state.db.begin().await?;
    let open: Option<(Uuid, String)> = sqlx::query_as(
        "select id, status from job_changes where job_id = $1 and status in ('requested', 'offered') for update",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((change_id, status)) = open else {
        return Err(ApiError::Conflict("there's no change waiting".into()));
    };
    let new_status = match (role, status.as_str()) {
        (Role::Creative, "requested") | (Role::Client, "offered") => "declined",
        (Role::Creative, "offered") => "withdrawn",
        _ => return Err(wrong_step("answer this change")),
    };
    sqlx::query(
        "update job_changes set status = $2, decline_note = nullif($3, ''), decided_at = now() where id = $1",
    )
    .bind(change_id)
    .bind(new_status)
    .bind(&text)
    .execute(&mut *tx)
    .await?;
    event(
        &mut tx,
        id,
        Some(user.id),
        if new_status == "declined" {
            "change_declined"
        } else {
            "change_withdrawn"
        },
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
struct AcceptChange {
    /// The change the client read.
    number: i32,
    #[serde(default)]
    exact_terms_ack: bool,
}

/// The client accepts a Change to Agreement: it's new work. Anything delivered goes back to
/// in progress, the new price is due now, and the due date moves.
async fn accept_change(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<AcceptChange>,
) -> ApiResult<Json<JobView>> {
    let (j, role) = load(&state, id, &user).await?;
    if role != Role::Client {
        return Err(wrong_step("accept a change"));
    }
    if !body.exact_terms_ack {
        return Err(ApiError::invalid(
            "exact_terms_ack",
            "confirm you understand you'll get exactly what the change says",
        ));
    }
    let change: Option<(Uuid, String, String, i32, Date, Option<String>)> = sqlx::query_as(
        "select id, title, scope_md, extra_price_minor, new_due_date, requested_note from job_changes
         where job_id = $1 and number = $2 and status = 'offered'",
    )
    .bind(id)
    .bind(body.number)
    .fetch_optional(&state.db)
    .await?;
    let Some((change_id, title, scope, extra, due, requested)) = change else {
        return Err(ApiError::Conflict(
            "that change isn't waiting for you (it may have changed); reload".into(),
        ));
    };
    let o = j
        .offer
        .clone()
        .ok_or_else(|| wrong_step("accept a change"))?;
    let p = parties(&state, &j).await?;
    let text = change_document(
        &ChangeTerms {
            number: body.number,
            title: &title,
            scope_md: &scope,
            extra_price_minor: extra.into(),
            currency: &o.currency,
            new_due_date: &due.to_string(),
            requested_note: requested.as_deref(),
        },
        &p,
        j.agreement_sha256.as_deref().unwrap_or_default(),
    );
    let sha = hex::encode(Sha256::digest(text.as_bytes()));
    let mut tx = state.db.begin().await?;
    let done = sqlx::query(
        "update job_changes set status = 'accepted', text = $2, sha256 = $3, decided_at = now()
         where id = $1 and status = 'offered'",
    )
    .bind(change_id)
    .bind(&text)
    .bind(&sha)
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::Conflict("the change changed; reload it".into()));
    }
    // New work: delivered or waiting for sign-off goes back to in progress, and what was due on
    // delivery waits for the next delivery.
    sqlx::query(
        "update job_payments set status = 'waiting' where job_id = $1 and due_on = 'delivery' and status = 'due'",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    let next = if extra > 0 {
        "awaiting_payment"
    } else {
        "in_progress"
    };
    set_status(
        &mut tx,
        id,
        &["in_progress", "awaiting_sign_off", "delivered"],
        next,
    )
    .await?;
    sqlx::query("update jobs set due_date = $2, delivered_at = null where id = $1")
        .bind(id)
        .bind(due)
        .execute(&mut *tx)
        .await?;
    if extra > 0 {
        sqlx::query(
            "insert into job_payments (id, job_id, part, due_on, amount_minor, currency, status, change_id)
             values ($1, $2, 'change', 'accept', $3, $4, 'due', $5)",
        )
        .bind(Uuid::now_v7())
        .bind(id)
        .bind(extra)
        .bind(&o.currency)
        .bind(change_id)
        .execute(&mut *tx)
        .await?;
    }
    event(
        &mut tx,
        id,
        Some(user.id),
        "change_accepted",
        Some(&format!(
            "Change to Agreement {} · SHA-256 {sha}",
            body.number
        )),
        &[],
        None,
    )
    .await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

// ---------------------------------------------------------------------------------------------
// Conversations: recorded, opened by the creative

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConversationRequest {
    #[serde(default)]
    note: String,
}

/// The client asks for a conversation, at any time. The creative decides.
async fn request_conversation(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<ConversationRequest>,
) -> ApiResult<Json<JobView>> {
    let (j, role) = load(&state, id, &user).await?;
    if role != Role::Client || j.conversation_open {
        return Err(wrong_step("ask for a conversation"));
    }
    if j.conversation_requested_at.is_some() {
        return Err(ApiError::Conflict(
            "you've already asked; the creative decides when to open it".into(),
        ));
    }
    let text = note("note", &body.note, 300, false)?;
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "update jobs set conversation_requested_at = now(), conversation_request_note = nullif($2, '') where id = $1",
    )
    .bind(id)
    .bind(&text)
    .execute(&mut *tx)
    .await?;
    event(
        &mut tx,
        id,
        Some(user.id),
        "conversation_requested",
        Some(&text),
        &[],
        None,
    )
    .await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

async fn open_conversation(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<JobView>> {
    let (j, role) = load(&state, id, &user).await?;
    if role != Role::Creative || j.conversation_open {
        return Err(wrong_step("open a conversation"));
    }
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "update jobs set conversation_open = true, conversation_requested_at = null, conversation_request_note = null,
                conversation_closed_at = null where id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    event(
        &mut tx,
        id,
        Some(user.id),
        "conversation_opened",
        None,
        &[],
        None,
    )
    .await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

async fn close_conversation(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<JobView>> {
    let (j, role) = load(&state, id, &user).await?;
    if role != Role::Creative || !j.conversation_open {
        return Err(wrong_step("close the conversation"));
    }
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "update jobs set conversation_open = false, conversation_closed_at = now() where id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    event(
        &mut tx,
        id,
        Some(user.id),
        "conversation_closed",
        None,
        &[],
        None,
    )
    .await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageBody {
    body: String,
    /// "I understand this conversation is recorded" (asked once per job, unticked).
    #[serde(default)]
    recording_ack: bool,
}

async fn post_message(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<MessageBody>,
) -> ApiResult<Json<JobView>> {
    let (j, role) = load(&state, id, &user).await?;
    if !j.conversation_open {
        return Err(ApiError::Conflict(
            "the conversation isn't open; the creative opens it".into(),
        ));
    }
    let acked = match role {
        Role::Client => j.client_recording_ack_at.is_some(),
        Role::Creative => j.creative_recording_ack_at.is_some(),
    };
    if !acked && !body.recording_ack {
        return Err(ApiError::invalid(
            "recording_ack",
            "confirm you understand this conversation is recorded",
        ));
    }
    let text = note("body", &body.body, 4000, true)?;
    let mut tx = state.db.begin().await?;
    if !acked {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "update jobs set {} = now() where id = $1",
            match role {
                Role::Client => "client_recording_ack_at",
                Role::Creative => "creative_recording_ack_at",
            }
        )))
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    // Each fingerprint chains to the one before, so a copy can't drop or reorder messages either.
    sqlx::query("select 1 from jobs where id = $1 for update")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let prev: Option<String> = sqlx::query_scalar(
        "select sha256 from job_messages where job_id = $1 order by created_at desc, id desc limit 1",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let message = Uuid::now_v7();
    let sha = message_fingerprint(prev.as_deref(), message, user.id, &text);
    sqlx::query(
        "insert into job_messages (id, job_id, author_id, body, sha256) values ($1, $2, $3, $4, $5)",
    )
    .bind(message)
    .bind(id)
    .bind(user.id)
    .bind(&text)
    .bind(&sha)
    .execute(&mut *tx)
    .await?;
    sqlx::query("update jobs set updated_at = now() where id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    reload(&state, id, &user).await
}

/// A message's fingerprint: SHA-256 of the previous fingerprint, the message ID, its author and
/// its text. It proves a copy is unchanged without Slop keeping the text.
pub fn message_fingerprint(prev: Option<&str>, id: Uuid, author: Uuid, body: &str) -> String {
    let mut h = Sha256::new();
    h.update(prev.unwrap_or("").as_bytes());
    h.update(b"\n");
    h.update(id.to_string().as_bytes());
    h.update(b"\n");
    h.update(author.to_string().as_bytes());
    h.update(b"\n");
    h.update(body.as_bytes());
    hex::encode(h.finalize())
}

/// A side's own copy of the conversation (JSON), as downloaded.
#[derive(Serialize, Deserialize)]
pub struct ConversationCopy {
    pub job_id: Uuid,
    pub messages: Vec<CopiedMessage>,
}

#[derive(Serialize, Deserialize)]
pub struct CopiedMessage {
    pub id: Uuid,
    pub author_id: Uuid,
    pub author: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub body: String,
    pub sha256: String,
}

async fn conversation_copy(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    load(&state, id, &user).await?;
    let rows: Vec<(Uuid, Uuid, String, OffsetDateTime, Option<String>, String)> = sqlx::query_as(
        "select m.id, m.author_id, u.display_name, m.created_at, m.body, m.sha256 from job_messages m
         join users u on u.id = m.author_id where m.job_id = $1 order by m.created_at, m.id",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    if rows.iter().any(|r| r.4.is_none()) {
        return Err(ApiError::Conflict(
            "Slop has already deleted this conversation's text; use the copy you downloaded".into(),
        ));
    }
    let copy = ConversationCopy {
        job_id: id,
        messages: rows
            .into_iter()
            .map(
                |(mid, author_id, author, created_at, body, sha256)| CopiedMessage {
                    id: mid,
                    author_id,
                    author,
                    created_at,
                    body: body.unwrap_or_default(),
                    sha256,
                },
            )
            .collect(),
    };
    Ok((
        [
            (header::CONTENT_TYPE, "application/json".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"slop-conversation-{id}.json\""),
            ),
        ],
        Json(copy),
    ))
}

/// Checks a copy against Slop's fingerprints: (matching messages, messages that don't match or
/// aren't Slop's). Order and gaps matter: each fingerprint includes the one before.
pub async fn verify_copy(
    db: &sqlx::PgPool,
    job: Uuid,
    copy: &ConversationCopy,
) -> sqlx::Result<(i32, i32)> {
    let stored: Vec<(Uuid, String)> = sqlx::query_as(
        "select id, sha256 from job_messages where job_id = $1 order by created_at, id",
    )
    .bind(job)
    .fetch_all(db)
    .await?;
    let mut prev: Option<String> = None;
    let (mut good, mut bad) = (0, 0);
    for m in &copy.messages {
        let sha = message_fingerprint(prev.as_deref(), m.id, m.author_id, &m.body);
        if stored.iter().any(|(id, s)| *id == m.id && *s == sha) {
            good += 1;
        } else {
            bad += 1;
        }
        prev = Some(sha);
    }
    Ok((good, bad))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceBody {
    /// The downloaded conversation copy (its JSON).
    copy: ConversationCopy,
}

#[derive(Serialize)]
struct EvidenceAnswer {
    verified_messages: i32,
    unverified_messages: i32,
}

/// A side submits their copy for a problem or a review dispute, so a mediator can read it.
async fn submit_evidence(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<EvidenceBody>,
) -> ApiResult<Json<EvidenceAnswer>> {
    load(&state, id, &user).await?;
    let open: bool = sqlx::query_scalar(
        "select exists (select 1 from job_problems where job_id = $1 and status = 'open')
             or exists (select 1 from review_disputes where job_id = $1 and status = 'pending')",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    if !open {
        return Err(ApiError::Conflict(
            "copies are only for an open problem or a review dispute".into(),
        ));
    }
    if body.copy.job_id != id || body.copy.messages.len() > 2000 {
        return Err(ApiError::invalid(
            "copy",
            "that isn't this job's conversation",
        ));
    }
    let (good, bad) = verify_copy(&state.db, id, &body.copy).await?;
    let transcript = serde_json::to_string(&body.copy).map_err(anyhow::Error::from)?;
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "insert into job_evidence (id, job_id, submitted_by, transcript, verified_messages, unverified_messages)
         values ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(user.id)
    .bind(&transcript)
    .bind(good)
    .bind(bad)
    .execute(&mut *tx)
    .await?;
    event(
        &mut tx,
        id,
        Some(user.id),
        "conversation_submitted",
        Some(&format!(
            "{good} messages match Slop's fingerprints, {bad} don't"
        )),
        &[],
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(EvidenceAnswer {
        verified_messages: good,
        unverified_messages: bad,
    }))
}

/// Deletes the text of conversations closed over 30 days ago, keeping the fingerprints; closes
/// conversations left open on jobs that ended or completed over 30 days ago (cleanup.rs).
pub async fn forget_conversations(db: &sqlx::PgPool, now: OffsetDateTime) -> sqlx::Result<u64> {
    sqlx::query(
        "update jobs set conversation_open = false, conversation_closed_at = $1
         where conversation_open and (ended_at < $1 - $2 or (status = 'completed' and completed_at < $1 - $2))",
    )
    .bind(now)
    .bind(CONVERSATION_KEPT)
    .execute(db)
    .await?;
    Ok(sqlx::query(
        "update job_messages m set body = null from jobs j
         where j.id = m.job_id and m.body is not null and not j.conversation_open and j.conversation_closed_at < $1 - $2",
    )
    .bind(now)
    .bind(CONVERSATION_KEPT)
    .execute(db)
    .await?
    .rows_affected())
}

// ---------------------------------------------------------------------------------------------
// The record: everything, downloadable by both sides (and by mediators for a problem or dispute)

// Row tuples straight from SQL, read once below.
#[allow(clippy::type_complexity)]
pub(crate) async fn record_text(db: &sqlx::PgPool, j: &JobRow) -> ApiResult<String> {
    let mut out = format!(
        "# Job record: {}\n\nJob {} · {} (@{}) for {} · project {} ({})\nRequested {} · status {}\n\n",
        j.offer.as_ref().map_or("request", |o| o.title.as_str()),
        j.id,
        j.creative_name,
        j.creative_handle,
        j.client_name,
        j.project_title,
        j.project_slug,
        j.created_at.date(),
        j.status
    );
    out.push_str(&format!("## Request\n\n{}\n\n", j.request_md));
    if let Some(text) = &j.agreement_text {
        out.push_str(&format!(
            "## Service Agreement (accepted {}, SHA-256 {})\n\n{text}\n\n",
            j.accepted_at.map(|a| a.to_string()).unwrap_or_default(),
            j.agreement_sha256.clone().unwrap_or_default()
        ));
    }
    let changes: Vec<(i32, String, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "select number, status, requested_note, text, decline_note from job_changes where job_id = $1 order by number",
    )
    .bind(j.id)
    .fetch_all(db)
    .await?;
    for (n, status, asked, text, declined) in changes {
        out.push_str(&format!("## Change {n} ({status})\n\n"));
        if let Some(a) = asked {
            out.push_str(&format!("Asked: {a}\n\n"));
        }
        if let Some(t) = text {
            out.push_str(&format!("{t}\n\n"));
        }
        if let Some(d) = declined {
            out.push_str(&format!("Note: {d}\n\n"));
        }
    }
    let events: Vec<(OffsetDateTime, Option<String>, String, Option<String>, Vec<String>)> =
        sqlx::query_as(
            "select e.created_at, u.display_name, e.kind, e.note, e.links from job_events e
             left join users u on u.id = e.actor_id where e.job_id = $1 order by e.created_at, e.id",
        )
        .bind(j.id)
        .fetch_all(db)
        .await?;
    out.push_str("## Timeline\n\n");
    for (at, who, kind, note, links) in events {
        out.push_str(&format!(
            "- {at} · {} · {kind}{}{}\n",
            who.unwrap_or_else(|| "Slop".into()),
            note.map(|n| format!(": {n}")).unwrap_or_default(),
            if links.is_empty() {
                String::new()
            } else {
                format!(" ({})", links.join(", "))
            }
        ));
    }
    let messages: Vec<(OffsetDateTime, String, Option<String>, String)> = sqlx::query_as(
        "select m.created_at, u.display_name, m.body, m.sha256 from job_messages m join users u on u.id = m.author_id
         where m.job_id = $1 order by m.created_at, m.id",
    )
    .bind(j.id)
    .fetch_all(db)
    .await?;
    if !messages.is_empty() {
        out.push_str("\n## Conversation (recorded)\n\n");
        for (at, who, body, sha) in messages {
            match body {
                Some(b) => out.push_str(&format!("**{who}** · {at}\n\n{b}\n\n")),
                None => out.push_str(&format!(
                    "**{who}** · {at} · deleted from Slop; fingerprint `{sha}`\n\n"
                )),
            }
        }
    }
    let payments: Vec<(String, i32, String, String)> = sqlx::query_as(
        "select part, amount_minor, currency, status from job_payments where job_id = $1 order by due_on, part",
    )
    .bind(j.id)
    .fetch_all(db)
    .await?;
    if !payments.is_empty() {
        out.push_str("\n## Payments\n\n");
        for (part, amount, currency, status) in payments {
            out.push_str(&format!(
                "- {part}: {} · {status}\n",
                money(amount.into(), &currency)
            ));
        }
    }
    out.push_str(&super::mediation::problems_text(db, j.id).await?);
    Ok(out)
}

async fn record(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let (j, _) = load(&state, id, &user).await?;
    let text = record_text(&state.db, &j).await?;
    Ok((
        [
            (
                header::CONTENT_TYPE,
                "text/markdown; charset=utf-8".to_string(),
            ),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"slop-job-{id}.md\""),
            ),
        ],
        text,
    ))
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

/// Records a payment and moves the job on: work starts after the payments due on accepting,
/// and paying what's due on delivery accepts it (the creative then sends the final files).
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
    sqlx::query(
        "insert into job_events (id, job_id, actor_id, kind, note, created_at) values ($1, $2, $3, 'paid', $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(job_id)
    .bind(p.payer)
    .bind(money(p.amount_minor, &currency))
    .bind(at)
    .execute(&mut *tx)
    .await?;
    let accept_left: bool = sqlx::query_scalar(
        "select exists (select 1 from job_payments where job_id = $1 and due_on = 'accept' and status <> 'paid')",
    )
    .bind(job_id)
    .fetch_one(&mut *tx)
    .await?;
    let delivery_left: bool = sqlx::query_scalar(
        "select exists (select 1 from job_payments where job_id = $1 and due_on = 'delivery' and status <> 'paid')",
    )
    .bind(job_id)
    .fetch_one(&mut *tx)
    .await?;
    if job_status == "awaiting_payment" && !accept_left {
        sqlx::query(
            "update jobs set status = 'in_progress', started_at = coalesce(started_at, $2), updated_at = $2 where id = $1",
        )
        .bind(job_id)
        .bind(at)
        .execute(&mut *tx)
        .await?;
    } else if due_on == "delivery"
        && !delivery_left
        && ["delivered", "completed"].contains(&job_status.as_str())
    {
        // Paying for the preview accepts the delivery; the creative now sends the final files.
        sqlx::query(
            "update jobs set status = 'final_files', completed_at = coalesce(completed_at, $2), updated_at = $2 where id = $1",
        )
        .bind(job_id)
        .bind(at)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "insert into job_events (id, job_id, actor_id, kind, created_at) values ($1, $2, $3, 'accepted_delivery', $4)",
        )
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

/// Where Stripe isn't used (the sandbox, development): records an agreed refund as simulated,
/// against the job's payments, newest first.
pub async fn simulated_refund(
    conn: &mut sqlx::PgConnection,
    job: Uuid,
    amount: i64,
) -> anyhow::Result<()> {
    let paid: Vec<(Uuid, i32, String, i64)> = sqlx::query_as(
        "select p.id, p.amount_minor, p.currency,
                coalesce((select -sum(r.amount_minor) from payments r where r.refund_of = p.id and r.kind = 'refund'), 0)::int8
         from payments p join job_payments jp on jp.id = p.job_payment_id
         where jp.job_id = $1 and p.kind = 'people_job' order by p.created_at desc",
    )
    .bind(job)
    .fetch_all(&mut *conn)
    .await?;
    let mut left = amount;
    for (id, paid, currency, refunded) in paid {
        let room = i64::from(paid) - refunded;
        let take = room.min(left);
        if take <= 0 {
            continue;
        }
        sqlx::query(
            "insert into payments (id, kind, refund_of, stripe_object_id, amount_minor, currency, simulated)
             values ($1, 'refund', $2, $3, $4, $5, true)",
        )
        .bind(Uuid::now_v7())
        .bind(id)
        .bind(format!("sim_refund_{}", Uuid::now_v7().simple()))
        .bind(-(take as i32))
        .bind(&currency)
        .execute(&mut *conn)
        .await?;
        event(
            conn,
            job,
            None,
            "refunded",
            Some(&money(take, &currency)),
            &[],
            None,
        )
        .await?;
        left -= take;
        if left == 0 {
            break;
        }
    }
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

/// Completes delivered jobs whose 7 days have passed (hourly, cleanup.rs). Anything still owed
/// stays owed, and the final files wait for it.
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
            sign_off: true,
            revisions: 0,
            rights: "transfer".into(),
            open_source: false,
            credit: true,
            ai_use: "none".into(),
            refund_policy: String::new(),
        }
    }

    fn parties() -> Parties<'static> {
        Parties {
            job_id: Uuid::nil(),
            creative_name: "Vera",
            creative_handle: "vera",
            trader: true,
            client_name: "Octo",
            project_title: "Pixel Dungeon",
            project_slug: "pixel-dungeon",
            skill: "pixel-art",
            withdrawal_days: Some(14),
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
        // Older offers with revisions still read; revisions no longer exist.
        let old: Offer = serde_json::from_value(serde_json::json!({
            "title": "x", "price_minor": 1000, "currency": "usd", "payment": "split", "due_date": "2026-12-01",
            "revisions": 2, "rights": "transfer", "open_source": false, "credit": true, "ai_use": "none"
        }))
        .unwrap();
        assert!(!old.sign_off);
        assert_eq!(clean_offer(old, today).unwrap().revisions, 0);
    }

    #[test]
    fn the_agreement_binds_both_sides_to_the_words() {
        let text = agreement(&offer(), &parties());
        assert!(!text.contains("{{"), "{text}");
        assert!(text.contains("nothing more and nothing less"));
        assert!(text.contains("Change to Agreement"));
        assert!(text.contains("**Sign-off.**"));
        assert!(text.contains("is a preview"));
        assert!(text.contains("transfers the copyright"));
        assert!(text.contains("isn't** covered by the project's open-source licence"));
        assert!(text.contains("$60.01 when you accept, $60 when you accept the delivered preview"));
        assert!(text.contains("within 14 days"));
        assert!(text.contains("job conversations"));
        assert!(text.contains("Slop isn't a party"));
        assert!(text.contains("\"Art by Vera\""));
        let mut o = offer();
        o.rights = "licence".into();
        o.open_source = true;
        o.sign_off = false;
        let text = agreement(
            &o,
            &Parties {
                withdrawal_days: None,
                trader: false,
                ..parties()
            },
        );
        assert!(text.contains("non-exclusive licence"));
        assert!(text.contains("under the project's open-source licence"));
        assert!(!text.contains("Your right to cancel"));
        assert!(!text.contains("**Sign-off.**"));
        assert!(text.contains("private individual"));
    }

    #[test]
    fn a_change_is_its_own_document() {
        let doc = change_document(
            &ChangeTerms {
                number: 2,
                title: "Add a running animation",
                scope_md: "8 frames, 4 directions.",
                extra_price_minor: 4_000,
                currency: "usd",
                new_due_date: "2026-12-20",
                requested_note: Some("Can the knight run too?"),
            },
            &parties(),
            "abc123",
        );
        assert!(doc.starts_with("# Change to Agreement 2"));
        assert!(doc.contains("SHA-256 `abc123`"));
        assert!(doc.contains("$40, paid to the creative when this change is accepted"));
        assert!(doc.contains("> Can the knight run too?"));
        assert!(doc.contains("**New due date:** 2026-12-20"));
        assert!(doc.contains("only as written here"));
    }
}
