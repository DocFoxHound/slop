// SPDX-License-Identifier: AGPL-3.0-or-later
//! Reports, moderation decisions and appeals (DESIGN §16; LEGAL-GUIDANCE CON-3, CON-5, CON-8,
//! CON-9, CON-16).
//!
//! - **Reports:** anyone can report an app, a People profile, a review, or (as one of its two
//!   sides) a job message. Each gets a reference number as the acknowledgement. Urgent and serious
//!   reasons are due within 24 hours; the rest within 7 days. Signed-out reporters give a name and
//!   email (optional for child abuse). Copyright reports collect what a DMCA notice must contain.
//! - **Decisions:** every restriction (taking an app out of the store, hiding a review or
//!   profile, a warning, suspending an account) is a decision with a statement of reasons: the
//!   facts, the rule (a Content Policy section), the law where it's the law, whether automation
//!   was used, and how to appeal. The affected user sees it on their account page. Reporters see
//!   the outcome of their reports.
//! - **Appeals:** within six months. A person keeps or reverses the decision, and says why. An
//!   appeal against a copyright decision is a counter-notice, with the statements §512(g) needs.
//!
//! Decisions are made by people. Nothing here is automated, and the statement says so.

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{
    AppState, admin,
    admin_web::AdminUser,
    auth::session::{CurrentUser, MaybeUser},
    catalog,
    error::{ApiError, ApiResult},
    policy::ViewerPolicy,
};

/// How long the affected user has to appeal (DSA Art. 20(1): at least six months).
pub const APPEAL_WINDOW: Duration = Duration::days(183);
/// Reports one account may send in a day.
const REPORTS_PER_DAY: i64 = 20;
/// Upheld copyright decisions at which the repeat-infringer policy ends an account (CON-4).
pub const COPYRIGHT_STRIKES: i64 = 3;

pub struct Reason {
    pub code: &'static str,
    pub label: &'static str,
    /// What it can be reported on: app, review, profile, message.
    pub kinds: &'static [&'static str],
    /// 0 urgent, 1 serious, 2 everything else.
    pub priority: i16,
    /// The Content Policy rule it usually breaks.
    pub rule: Option<&'static str>,
}

const ALL: &[&str] = &[
    "app",
    "review",
    "profile",
    "message",
    "app_review",
    "team",
    "chat_message",
    "bounty",
];

pub const REASONS: &[Reason] = &[
    Reason {
        code: "child_abuse",
        label: "Child sexual abuse material",
        kinds: ALL,
        priority: 0,
        rule: Some("child_abuse"),
    },
    Reason {
        code: "threat",
        label: "A threat to someone's life or safety",
        kinds: &[
            "review",
            "app_review",
            "profile",
            "team",
            "message",
            "chat_message",
            "bounty",
        ],
        priority: 0,
        rule: Some("threat"),
    },
    Reason {
        code: "intimate_image",
        label: "An intimate image shared without consent (including AI-made)",
        kinds: ALL,
        priority: 0,
        rule: Some("intimate_image"),
    },
    Reason {
        code: "malware",
        label: "Malware, hidden mining or data theft",
        kinds: &["app"],
        priority: 1,
        rule: Some("malware"),
    },
    Reason {
        code: "source_mismatch",
        label: "The build doesn't match its source",
        kinds: &["app"],
        priority: 1,
        rule: Some("source_mismatch"),
    },
    Reason {
        code: "outside_code",
        label: "Runs code or mods from outside SLAPorSLOP",
        kinds: &["app"],
        priority: 1,
        rule: Some("outside_code"),
    },
    Reason {
        code: "scam",
        label: "A scam or fraud",
        kinds: ALL,
        priority: 1,
        rule: Some("scam"),
    },
    Reason {
        code: "illegal",
        label: "Illegal content (including terrorist content)",
        kinds: ALL,
        priority: 1,
        rule: Some("illegal"),
    },
    Reason {
        code: "sexual",
        label: "Sexual content",
        kinds: ALL,
        priority: 1,
        rule: Some("sexual"),
    },
    Reason {
        code: "copyright",
        label: "Copyright or trademark infringement",
        kinds: &["app", "review", "app_review", "profile", "team"],
        priority: 1,
        rule: Some("copyright"),
    },
    Reason {
        code: "harassment",
        label: "Harassment, hate or bullying",
        kinds: &[
            "review",
            "app_review",
            "profile",
            "team",
            "message",
            "chat_message",
            "bounty",
        ],
        priority: 2,
        rule: Some("harassment"),
    },
    Reason {
        code: "private_info",
        label: "Someone's private information",
        kinds: &[
            "review",
            "app_review",
            "profile",
            "team",
            "message",
            "chat_message",
            "bounty",
        ],
        priority: 2,
        rule: Some("private_info"),
    },
    Reason {
        code: "license",
        label: "Breaks an open-source licence",
        kinds: &["app"],
        priority: 2,
        rule: Some("license"),
    },
    Reason {
        code: "gambling",
        label: "Pay-to-win, loot boxes or gambling",
        kinds: &["app"],
        priority: 2,
        rule: Some("gambling"),
    },
    Reason {
        code: "misleading",
        label: "A misleading page",
        kinds: &["app", "profile", "team"],
        priority: 2,
        rule: Some("misleading"),
    },
    Reason {
        code: "fake_review",
        label: "A fake, paid or off-topic review",
        kinds: &["review", "app_review"],
        priority: 2,
        rule: Some("fake_review"),
    },
    Reason {
        code: "spam",
        label: "Spam",
        kinds: &[
            "bounty",
            "review",
            "app_review",
            "profile",
            "team",
            "message",
            "chat_message",
        ],
        priority: 2,
        rule: Some("spam"),
    },
    Reason {
        code: "broken",
        label: "The build is broken",
        kinds: &["app"],
        priority: 2,
        rule: None,
    },
    Reason {
        code: "other",
        label: "Something else",
        kinds: ALL,
        priority: 2,
        rule: None,
    },
];

/// Content Policy sections. Each title is a heading in `legal/content.md`.
pub const RULES: &[(&str, &str)] = &[
    ("malware", "Malware and data theft"),
    ("source_mismatch", "Builds must match their source"),
    ("outside_code", "Code from outside SLAPorSLOP"),
    ("scam", "Scams and fraud"),
    ("illegal", "Illegal content"),
    ("child_abuse", "Child sexual abuse material"),
    ("intimate_image", "Intimate images shared without consent"),
    ("sexual", "Sexual content"),
    ("threat", "Threats and violence"),
    ("harassment", "Harassment, hate and bullying"),
    ("private_info", "Sharing private information"),
    ("copyright", "Copyright and trademarks"),
    ("license", "Open-source licences"),
    ("gambling", "Pay-to-win, loot boxes and gambling"),
    ("misleading", "Misleading pages"),
    ("fake_review", "Fake, paid or off-topic reviews"),
    ("extortion", "Reviews used as leverage"),
    ("false_fact", "False statements of fact"),
    ("spam", "Spam"),
];

pub fn rule_title(code: &str) -> &'static str {
    RULES
        .iter()
        .find(|(c, _)| *c == code)
        .map_or("Content Policy", |(_, t)| t)
}

fn reason(code: &str) -> Option<&'static Reason> {
    REASONS.iter().find(|r| r.code == code)
}

fn reason_label(code: &str) -> &'static str {
    reason(code).map_or("Other", |r| r.label)
}

pub fn action_label(action: &str) -> &'static str {
    match action {
        "delist_app" => "Taken out of the store",
        "hide_review" => "Review hidden",
        "hide_profile" => "Profile hidden",
        "hide_team" => "Team hidden",
        "remove_reply" => "Reply removed",
        "hide_bounty" => "Bounty hidden",
        "suspend_account" => "Account suspended",
        "warn" => "Warning",
        _ => "Decision",
    }
}

/// What a moderator can do about each kind of thing.
fn actions_for(kind: &str) -> &'static [&'static str] {
    match kind {
        "app" => &["delist_app", "warn", "suspend_account"],
        "review" => &["hide_review", "warn", "suspend_account"],
        // A review and the maker's reply under it: either can be the problem.
        "app_review" => &["hide_review", "remove_reply", "warn", "suspend_account"],
        "profile" => &["hide_profile", "warn", "suspend_account"],
        "team" => &["hide_team", "warn", "suspend_account"],
        "bounty" => &["hide_bounty", "warn", "suspend_account"],
        _ => &["warn", "suspend_account"],
    }
}

/// A short reference people can quote: R-XXXX-XXXX for reports, D-… for decisions.
fn reference(prefix: char) -> String {
    const ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
    let bytes: [u8; 8] = rand::random();
    let chars: String = bytes
        .iter()
        .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
        .collect();
    format!("{prefix}-{}-{}", &chars[..4], &chars[4..])
}

fn text(field: &'static str, value: &str, max: usize, required: bool) -> ApiResult<String> {
    let value = value.trim();
    if required && value.is_empty() {
        return Err(ApiError::invalid(field, "this can't be empty"));
    }
    if value.chars().count() > max {
        return Err(ApiError::invalid(
            field,
            format!("{max} characters at most"),
        ));
    }
    Ok(value.to_string())
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/reports/reasons", get(reasons))
        .route("/reports", post(report))
        .route("/me/reports", get(my_reports))
        .route("/me/decisions", get(my_decisions))
        .route("/me/decisions/{id}/appeal", post(appeal))
        .route("/admin/reports/{id}/act", post(act))
        .route("/admin/reports/{id}/dismiss", post(dismiss))
        .route("/admin/decisions/{id}/appeal", post(decide_appeal))
}

// ---------------------------------------------------------------------------------------------
// Reporting

#[derive(Deserialize)]
struct ReasonsQuery {
    kind: String,
}

#[derive(Serialize)]
struct ReasonView {
    code: &'static str,
    label: &'static str,
    urgent: bool,
}

async fn reasons(Query(q): Query<ReasonsQuery>) -> Json<Vec<ReasonView>> {
    Json(
        REASONS
            .iter()
            .filter(|r| r.kinds.contains(&q.kind.as_str()))
            .map(|r| ReasonView {
                code: r.code,
                label: r.label,
                urgent: r.priority < 2,
            })
            .collect(),
    )
}

/// What a copyright notice must contain (17 U.S.C. §512(c)(3)).
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CopyrightNotice {
    /// The work that's infringed.
    work: String,
    /// Who owns it, if the reporter is acting for them.
    owner: String,
    address: String,
    phone: String,
    /// "I believe in good faith that this use isn't authorised…"
    good_faith: bool,
    /// "…the information is accurate and, under penalty of perjury, I'm authorised to act…"
    accurate_under_penalty: bool,
    /// The reporter's full name, typed as their signature.
    signature: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReportBody {
    /// app, review, profile or message.
    kind: String,
    /// An app's address, a creative's handle, a review's job ID or a message ID.
    subject: String,
    reason: String,
    details: String,
    /// "I believe this report is accurate and complete" (DSA Art. 16(2)(d)).
    good_faith: bool,
    name: Option<String>,
    email: Option<String>,
    copyright: Option<CopyrightNotice>,
    /// A chat message (kind chat_message): the reporter's decrypted copy and its franking key,
    /// checked against what SLAPorSLOP stored when it was sent (chat.rs).
    chat: Option<crate::chat::ChatEvidence>,
}

#[derive(Serialize)]
struct Acknowledgement {
    reference: String,
    #[serde(with = "time::serde::rfc3339")]
    due_at: OffsetDateTime,
}

/// What's being reported: (id, the account it's about, a label, a quoted message).
struct Subject {
    id: Uuid,
    user_id: Option<Uuid>,
    label: String,
    quoted: Option<String>,
}

async fn find_subject(
    state: &AppState,
    user: Option<&CurrentUser>,
    headers: &HeaderMap,
    kind: &str,
    subject: &str,
    chat: Option<&crate::chat::ChatEvidence>,
) -> ApiResult<Subject> {
    let subject = subject.trim();
    match kind {
        "app" => {
            let policy = ViewerPolicy::resolve(&state.db, &state.config, user, headers).await?;
            let (id, _) = catalog::visible_app(state, subject, user, policy).await?;
            let (maker, title): (Uuid, String) =
                sqlx::query_as("select maker_id, title from apps where id = $1")
                    .bind(id)
                    .fetch_one(&state.db)
                    .await?;
            Ok(Subject {
                id,
                user_id: Some(maker),
                label: format!("App: {title}"),
                quoted: None,
            })
        }
        "profile" => {
            let row: Option<(Uuid, String, String)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "select user_id, name, handle from creatives c where handle = $1 and {}",
                crate::people::VISIBLE_SQL
            )))
            .bind(subject.to_ascii_lowercase())
            .fetch_optional(&state.db)
            .await?;
            let (id, name, handle) = row.ok_or(ApiError::NotFound)?;
            Ok(Subject {
                id,
                user_id: Some(id),
                label: format!("People profile: {name} (@{handle})"),
                quoted: None,
            })
        }
        "review" => {
            let job_id: Uuid = subject.parse().map_err(|_| ApiError::NotFound)?;
            let row: Option<(Uuid, String, String)> = sqlx::query_as(
                "select r.client_id, u.display_name, c.name from creative_reviews r
                 join users u on u.id = r.client_id join creatives c on c.user_id = r.creative_id
                 where r.job_id = $1 and r.hidden_at is null",
            )
            .bind(job_id)
            .fetch_optional(&state.db)
            .await?;
            let (client, client_name, creative_name) = row.ok_or(ApiError::NotFound)?;
            Ok(Subject {
                id: job_id,
                user_id: Some(client),
                label: format!("Review of {creative_name} by {client_name}"),
                quoted: None,
            })
        }
        "team" => {
            let row: Option<(Uuid, Uuid, String)> = sqlx::query_as(
                "select id, created_by, name from teams where slug = $1 and hidden_reason is null",
            )
            .bind(subject.to_ascii_lowercase())
            .fetch_optional(&state.db)
            .await?;
            let (id, owner, name) = row.ok_or(ApiError::NotFound)?;
            Ok(Subject {
                id,
                user_id: Some(owner),
                label: format!("Team: {name}"),
                quoted: None,
            })
        }
        "bounty" => {
            let id: Uuid = subject.parse().map_err(|_| ApiError::NotFound)?;
            let row: Option<(Uuid, String)> = sqlx::query_as(
                "select created_by, title from bounties where id = $1 and status <> 'hidden'",
            )
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
            let (author, title) = row.ok_or(ApiError::NotFound)?;
            Ok(Subject {
                id,
                user_id: Some(author),
                label: format!("Bounty: {title}"),
                quoted: None,
            })
        }
        "app_review" => {
            let id: Uuid = subject.parse().map_err(|_| ApiError::NotFound)?;
            let row: Option<(Uuid, String, String)> = sqlx::query_as(
                "select r.user_id, u.display_name, a.title from app_reviews r join users u on u.id = r.user_id
                 join apps a on a.id = r.app_id where r.id = $1 and r.hidden_at is null",
            )
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
            let (author, name, title) = row.ok_or(ApiError::NotFound)?;
            Ok(Subject {
                id,
                user_id: Some(author),
                label: format!("Review of {title} by {name}"),
                quoted: None,
            })
        }
        "message" => {
            // Only the two sides of a job see its messages, so only they can report one.
            let user = user.ok_or(ApiError::Unauthorized)?;
            let id: Uuid = subject.parse().map_err(|_| ApiError::NotFound)?;
            let row: Option<(Uuid, Option<String>, String)> = sqlx::query_as(
                "select m.author_id, m.body, coalesce(j.offer->>'title', 'a job for ' || a.title)
                 from job_messages m join jobs j on j.id = m.job_id join apps a on a.id = j.app_id
                 where m.id = $1 and $2 in (j.client_id, j.creative_id)",
            )
            .bind(id)
            .bind(user.id)
            .fetch_optional(&state.db)
            .await?;
            let (author, body, title) = row.ok_or(ApiError::NotFound)?;
            if author == user.id {
                return Err(ApiError::invalid("subject", "that's your own message"));
            }
            let body = body.ok_or_else(|| {
                ApiError::Conflict(
                    "that message's text has been deleted; report the job with your own copy instead"
                        .into(),
                )
            })?;
            Ok(Subject {
                id,
                user_id: Some(author),
                label: format!("Message in the job \"{title}\""),
                quoted: Some(body),
            })
        }
        "chat_message" => {
            // Only someone in the chat can report it, with their own decrypted copy: SLAPorSLOP can't
            // read chats, but franking proves the sender sent exactly this.
            let user = user.ok_or(ApiError::Unauthorized)?;
            let evidence = chat.ok_or_else(|| {
                ApiError::invalid("chat", "a chat report carries the reported message")
            })?;
            let (sender, chat_kind, name) =
                crate::chat::check_evidence(&state.db, user.id, evidence).await?;
            let text = serde_json::from_str::<serde_json::Value>(&evidence.plaintext)
                .ok()
                .and_then(|v| v["text"].as_str().map(String::from))
                .unwrap_or_else(|| evidence.plaintext.clone());
            let attachments = serde_json::from_str::<serde_json::Value>(&evidence.plaintext)
                .ok()
                .and_then(|v| v["attachments"].as_array().map(|a| a.len()))
                .unwrap_or(0);
            let mut quoted: String = text.chars().take(3800).collect();
            if attachments > 0 {
                quoted.push_str(&format!(
                    "\n[{attachments} attachment(s): the reporter can send them on request]"
                ));
            }
            Ok(Subject {
                id: evidence.conversation_id,
                user_id: Some(sender),
                label: format!(
                    "Message from {name} in a {} chat",
                    match chat_kind.as_str() {
                        "direct" => "direct",
                        "team" => "team",
                        _ => "group",
                    }
                ),
                quoted: Some(quoted),
            })
        }
        _ => Err(ApiError::invalid(
            "kind",
            "app, review, app_review, profile, message, chat_message or bounty",
        )),
    }
}

async fn report(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Json(body): Json<ReportBody>,
) -> ApiResult<Json<Acknowledgement>> {
    if let Some(u) = &user {
        crate::limits::check(&state, u.id, crate::limits::Action::Report)?;
    }
    let reason = reason(&body.reason)
        .filter(|r| r.kinds.contains(&body.kind.as_str()))
        .ok_or_else(|| ApiError::invalid("reason", "choose a reason"))?;
    let details = text("details", &body.details, 4000, true)?;
    if !body.good_faith {
        return Err(ApiError::invalid(
            "good_faith",
            "confirm the report is accurate and made in good faith",
        ));
    }
    // A name and an email for signed-out reporters, except for child abuse reports (DSA Art. 16(2)(c)).
    let (name, email) = match &user {
        Some(_) => (None, None),
        None => {
            let name = text("name", body.name.as_deref().unwrap_or(""), 100, false)?;
            let email = text("email", body.email.as_deref().unwrap_or(""), 254, false)?;
            if reason.code != "child_abuse" {
                if name.is_empty() {
                    return Err(ApiError::invalid("name", "add your name"));
                }
                if !email.contains('@') {
                    return Err(ApiError::invalid(
                        "email",
                        "add an email we can reach you at",
                    ));
                }
            }
            (
                Some(name).filter(|n| !n.is_empty()),
                Some(email).filter(|e| !e.is_empty()),
            )
        }
    };
    let copyright = if reason.code == "copyright" {
        let c = body.copyright.ok_or_else(|| {
            ApiError::invalid("copyright", "a copyright report needs the notice details")
        })?;
        for (field, value, max) in [
            ("copyright.work", &c.work, 1000),
            ("copyright.address", &c.address, 300),
            ("copyright.phone", &c.phone, 40),
            ("copyright.signature", &c.signature, 100),
        ] {
            text(field, value, max, true)?;
        }
        text("copyright.owner", &c.owner, 200, false)?;
        if !c.good_faith || !c.accurate_under_penalty {
            return Err(ApiError::invalid(
                "copyright.statements",
                "a copyright notice needs both statements",
            ));
        }
        Some(sqlx::types::Json(c))
    } else {
        None
    };
    if let Some(u) = &user {
        let today: i64 = sqlx::query_scalar(
            "select count(*) from reports where reporter_id = $1 and created_at > now() - interval '1 day'",
        )
        .bind(u.id)
        .fetch_one(&state.db)
        .await?;
        if today >= REPORTS_PER_DAY {
            return Err(ApiError::Conflict(
                "you've sent a lot of reports today; try again tomorrow, or use the contact page"
                    .into(),
            ));
        }
    }
    let subject = find_subject(
        &state,
        user.as_ref(),
        &headers,
        &body.kind,
        &body.subject,
        body.chat.as_ref(),
    )
    .await?;
    // A reviewer can report the maker's reply under their own review, so only then is reporting
    // your own review allowed.
    let reply_under_own_review = body.kind == "app_review"
        && sqlx::query_scalar::<_, bool>(
            "select exists (select 1 from app_reviews where id = $1 and reply is not null and reply_hidden_at is null)",
        )
        .bind(subject.id)
        .fetch_one(&state.db)
        .await?;
    if user.as_ref().is_some_and(|u| Some(u.id) == subject.user_id) && !reply_under_own_review {
        return Err(ApiError::invalid("subject", "you can't report your own"));
    }
    let due = if reason.priority < 2 {
        Duration::hours(24)
    } else {
        Duration::days(7)
    };
    let preserve = (reason.code == "child_abuse").then(|| Duration::days(366));
    // Signed-out reports have no account to limit (security review, 2026-10-22): the same
    // address reporting the same thing again, or a flood about one subject, joins the report
    // already waiting instead of adding to the queue and to outcome emails. Signed-in reports
    // are limited per account above.
    if user.is_none() {
        let waiting: Option<(String, OffsetDateTime)> = sqlx::query_as(
            "select reference, due_at from reports
             where reporter_id is null and subject_kind = $1 and subject_id = $2 and status = 'open'
               and (lower(reporter_email) = lower($3)
                    or (select count(*) from reports r2 where r2.reporter_id is null and r2.subject_kind = $1
                          and r2.subject_id = $2 and r2.created_at > now() - interval '1 day') >= 10)
             order by created_at limit 1",
        )
        .bind(&body.kind)
        .bind(subject.id)
        .bind(email.as_deref().unwrap_or(""))
        .fetch_optional(&state.db)
        .await?;
        if let Some((reference, due_at)) = waiting {
            return Ok(Json(Acknowledgement { reference, due_at }));
        }
    }
    let reference = reference('R');
    let due_at: OffsetDateTime = sqlx::query_scalar(
        "insert into reports (id, reference, subject_kind, subject_id, subject_user_id, subject_label, quoted,
                              reason, details, reporter_id, reporter_name, reporter_email, copyright,
                              priority, due_at, preserve_until)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, now() + $15, now() + $16)
         returning due_at",
    )
    .bind(Uuid::now_v7())
    .bind(&reference)
    .bind(&body.kind)
    .bind(subject.id)
    .bind(subject.user_id)
    .bind(&subject.label)
    .bind(&subject.quoted)
    .bind(reason.code)
    .bind(&details)
    .bind(user.as_ref().map(|u| u.id))
    .bind(name)
    .bind(email)
    .bind(copyright)
    .bind(reason.priority)
    .bind(due)
    .bind(preserve)
    .fetch_one(&state.db)
    .await
    .map_err(|err| {
        if crate::error::is_unique_violation(&err) {
            ApiError::Conflict(
                "you've already reported this, and it's waiting for review".into(),
            )
        } else {
            err.into()
        }
    })?;
    // A report that the app is malicious, or isn't its source, re-scans its live release now.
    if body.kind == "app" && matches!(reason.code, "malware" | "source_mismatch") {
        crate::rescan::request(&state.db, subject.id, reason.code).await?;
    }
    Ok(Json(Acknowledgement { reference, due_at }))
}

#[derive(Serialize, sqlx::FromRow)]
struct MyReport {
    reference: String,
    subject_kind: String,
    subject_label: String,
    reason: String,
    #[sqlx(skip)]
    reason_label: &'static str,
    status: String,
    outcome: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    decided_at: Option<OffsetDateTime>,
}

/// The reports the user sent, and what happened to them (DSA Art. 16(5)).
async fn my_reports(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Vec<MyReport>>> {
    let mut rows: Vec<MyReport> = sqlx::query_as(
        "select reference, subject_kind, subject_label, reason, status, outcome, created_at, decided_at
         from reports where reporter_id = $1 order by created_at desc limit 200",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    for r in &mut rows {
        r.reason_label = reason_label(&r.reason);
    }
    Ok(Json(rows))
}

// ---------------------------------------------------------------------------------------------
// Decisions

/// A decision to record. `decide` also carries it out; `record` is for a restriction already
/// made by another path (a review dispute).
pub(crate) struct NewDecision<'a> {
    pub user_id: Uuid,
    pub subject_kind: &'a str,
    pub subject_id: Uuid,
    pub subject_label: &'a str,
    pub action: &'a str,
    pub rule: &'a str,
    pub law: Option<&'a str>,
    pub facts: &'a str,
    pub decided_by: &'a str,
}

pub(crate) async fn record(
    tx: &mut sqlx::PgConnection,
    d: &NewDecision<'_>,
) -> anyhow::Result<Uuid> {
    anyhow::ensure!(
        RULES.iter().any(|(c, _)| *c == d.rule),
        "choose the rule it breaks"
    );
    anyhow::ensure!(!d.facts.trim().is_empty(), "say what happened (the facts)");
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into moderation_decisions (id, reference, user_id, subject_kind, subject_id, subject_label,
                                           action, rule, law, facts, decided_by)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(id)
    .bind(reference('D'))
    .bind(d.user_id)
    .bind(d.subject_kind)
    .bind(d.subject_id)
    .bind(d.subject_label)
    .bind(d.action)
    .bind(d.rule)
    .bind(d.law.filter(|l| !l.trim().is_empty()))
    .bind(d.facts.trim())
    .bind(d.decided_by)
    .execute(&mut *tx)
    .await?;
    crate::notify::send(
        tx,
        crate::notify::Notice {
            user_id: d.user_id,
            category: "moderation",
            title: format!("{}: {}", action_label(d.action), d.subject_label),
            body: format!(
                "Under the Content Policy rule \"{}\". See why, and how to appeal.",
                rule_title(d.rule)
            ),
            link: "/account#decisions".into(),
            collapse_key: None,
        },
    )
    .await?;
    admin::audit(
        tx,
        d.decided_by,
        &format!("moderation.{}", d.action),
        &d.subject_id.to_string(),
        json!({ "decision": id, "user": d.user_id, "rule": d.rule, "facts": d.facts }),
    )
    .await?;
    Ok(id)
}

/// A decision carried out and recorded.
pub(crate) struct Decided {
    pub id: Uuid,
    /// Saved cards of the pledges it released, for `bounties::forget_cards` once the
    /// transaction commits.
    pub cards: Vec<String>,
}

/// Carries out a decision and records it.
pub(crate) async fn decide(
    tx: &mut sqlx::PgConnection,
    d: &NewDecision<'_>,
) -> anyhow::Result<Decided> {
    let applied = apply(
        tx,
        d.action,
        d.subject_kind,
        d.subject_id,
        d.user_id,
        d.rule,
        false,
    )
    .await?;
    Ok(Decided {
        id: record(tx, d).await?,
        cards: applied.cards,
    })
}

/// What carrying out or undoing a decision left to do, or to say.
#[derive(Default)]
struct Applied {
    /// Saved cards to forget at Stripe once the transaction commits.
    cards: Vec<String>,
    /// Why an app put back on appeal stays out of the store for now, for the maker and staff.
    stays_out: Option<String>,
}

/// A bounty put back on appeal runs for the time it had left, but at least this long.
const RESTORED_BOUNTY_RUNS: Duration = Duration::days(7);

/// Puts a decision into effect, or undoes it when an appeal reverses it.
async fn apply(
    tx: &mut sqlx::PgConnection,
    action: &str,
    kind: &str,
    subject_id: Uuid,
    user_id: Uuid,
    rule: &str,
    undo: bool,
) -> anyhow::Result<Applied> {
    let why = rule_title(rule);
    let mut out = Applied::default();
    match (action, kind) {
        ("delist_app", "app") if undo => {
            // Back in the store only if it passes the checks any listing does. Frozen: it goes
            // back when the freeze ends, like an app delisted for being frozen (frozen.rs).
            // Otherwise it waits under Ready to list for a reviewer.
            let stage: Option<crate::catalog::AppStage> = sqlx::query_scalar(
                "select stage from apps where id = $1 and status = 'delisted' for update",
            )
            .bind(subject_id)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(stage) = stage {
                match admin::not_listable(tx, subject_id, stage).await? {
                    None => {
                        sqlx::query(
                            "update apps set status = 'listed', relist_after_appeal = false, updated_at = now()
                             where id = $1",
                        )
                        .bind(subject_id)
                        .execute(&mut *tx)
                        .await?;
                    }
                    Some(blocked) => {
                        let frozen = matches!(blocked, admin::NotListable::Frozen(_));
                        sqlx::query(
                            "update apps set frozen_delisted = frozen_delisted or $2, relist_after_appeal = not $2
                             where id = $1",
                        )
                        .bind(subject_id)
                        .bind(frozen)
                        .execute(&mut *tx)
                        .await?;
                        out.stays_out = Some(blocked.for_maker());
                    }
                }
            }
        }
        ("delist_app", "app") => {
            let n = sqlx::query(
                "update apps set status = 'delisted', updated_at = now() where id = $1 and status = 'listed'",
            )
            .bind(subject_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            anyhow::ensure!(
                n == 1,
                "the app isn't in the store, so there's nothing to take out"
            );
        }
        ("hide_review", "review") => {
            sqlx::query(
                "update creative_reviews set hidden_at = case when $2 then null else now() end,
                        hidden_reason = case when $2 then null else $3 end
                 where job_id = $1",
            )
            .bind(subject_id)
            .bind(undo)
            .bind(why)
            .execute(&mut *tx)
            .await?;
            if undo {
                sqlx::query(
                    "update review_disputes set status = 'kept',
                            decision_reason = decision_reason || ' (reversed on appeal)'
                     where job_id = $1 and status = 'removed'",
                )
                .bind(subject_id)
                .execute(&mut *tx)
                .await?;
            }
        }
        ("hide_review", "app_review") => {
            sqlx::query(
                "update app_reviews set hidden_at = case when $2 then null else now() end,
                        hidden_reason = case when $2 then null else $3 end
                 where id = $1",
            )
            .bind(subject_id)
            .bind(undo)
            .bind(why)
            .execute(&mut *tx)
            .await?;
        }
        ("hide_bounty", "bounty") if undo => {
            // Back as it was, for the time it had left (at least a week). A delivery withdrawn
            // while it was hidden leaves it open; one being paid has nothing left to charge.
            let title: Option<String> = sqlx::query_scalar(
                "update bounties set
                        status = case
                            when hidden_from = 'claimed' and not exists (
                                select 1 from bounty_claims c where c.bounty_id = $1 and c.status = 'pending')
                            then 'open'
                            when hidden_from = 'paying' then 'paid'
                            else coalesce(hidden_from, 'open') end,
                        expires_at = now() + greatest(coalesce(hidden_left, interval '0'), $2),
                        hidden_reason = null, hidden_from = null, hidden_left = null
                 where id = $1 and status = 'hidden'
                 returning title",
            )
            .bind(subject_id)
            .bind(RESTORED_BOUNTY_RUNS)
            .fetch_optional(&mut *tx)
            .await?;
            let Some(title) = title else {
                return Ok(out);
            };
            // Their pledges stay released (the cards were let go): they can pledge again.
            let mut backers: Vec<Uuid> = sqlx::query_scalar(
                "update bounty_pledges set released_by_hide = false
                 where bounty_id = $1 and released_by_hide returning user_id",
            )
            .bind(subject_id)
            .fetch_all(&mut *tx)
            .await?;
            backers.sort();
            backers.dedup();
            for backer in backers {
                crate::notify::send(
                    tx,
                    crate::notify::Notice {
                        user_id: backer,
                        category: "donations",
                        title: format!("\"{title}\" is back"),
                        body: "The bounty was restored on appeal. Your pledge was released when it \
                               was hidden and won't be charged: pledge again if you still want to back it."
                            .into(),
                        link: format!("/bounties/{subject_id}"),
                        collapse_key: Some(format!("bounty:{subject_id}")),
                    },
                )
                .await?;
            }
        }
        ("hide_bounty", "bounty") => {
            // Hiding ends it: nothing more is charged, and every pledge is released, its saved
            // card forgotten at Stripe once the decision commits. What it was is kept for an
            // appeal.
            sqlx::query(
                "update bounties set hidden_from = status, hidden_left = greatest(expires_at - now(), interval '0'),
                        status = 'hidden', hidden_reason = $2, expires_at = now()
                 where id = $1 and status <> 'hidden'",
            )
            .bind(subject_id)
            .bind(why)
            .execute(&mut *tx)
            .await?;
            let cards: Vec<Option<String>> = sqlx::query_scalar(
                "update bounty_pledges set status = 'released', released_by_hide = true
                 where bounty_id = $1 and status in ('saving', 'active', 'due')
                 returning payment_method_id",
            )
            .bind(subject_id)
            .fetch_all(&mut *tx)
            .await?;
            out.cards = cards.into_iter().flatten().collect();
        }
        ("remove_reply", "app_review") => {
            sqlx::query(
                "update app_reviews set reply_hidden_at = case when $2 then null else now() end,
                        reply_hidden_reason = case when $2 then null else $3 end
                 where id = $1",
            )
            .bind(subject_id)
            .bind(undo)
            .bind(why)
            .execute(&mut *tx)
            .await?;
        }
        ("hide_team", "team") => {
            sqlx::query(
                "update teams set hidden_reason = case when $2 then null else $3 end where id = $1",
            )
            .bind(subject_id)
            .bind(undo)
            .bind(why)
            .execute(&mut *tx)
            .await?;
        }
        ("hide_profile", "profile") => {
            sqlx::query(
                "update creatives set hidden_reason = case when $2 then null else $3 end, updated_at = now()
                 where user_id = $1",
            )
            .bind(subject_id)
            .bind(undo)
            .bind(why)
            .execute(&mut *tx)
            .await?;
        }
        ("suspend_account", _) => {
            // Admin rights end with a suspension and don't come back on their own.
            sqlx::query(
                "update users set suspended_at = case when $2 then null else now() end,
                        is_admin = is_admin and $2
                 where id = $1",
            )
            .bind(user_id)
            .bind(undo)
            .execute(&mut *tx)
            .await?;
        }
        ("warn", _) => {}
        _ => anyhow::bail!("that action doesn't apply to a {kind}"),
    }
    Ok(out)
}

#[derive(Serialize, sqlx::FromRow)]
pub struct DecisionView {
    id: Uuid,
    reference: String,
    subject_kind: String,
    subject_label: String,
    action: String,
    #[sqlx(skip)]
    action_label: &'static str,
    rule: String,
    #[sqlx(skip)]
    rule_title: &'static str,
    law: Option<String>,
    facts: String,
    automated: bool,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    appeal: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    appealed_at: Option<OffsetDateTime>,
    appeal_status: Option<String>,
    appeal_reason: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    appeal_decided_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    reversed_at: Option<OffsetDateTime>,
    /// Appeals are open for six months and only once.
    #[sqlx(skip)]
    can_appeal: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    #[sqlx(skip)]
    appeal_until: Option<OffsetDateTime>,
    /// Whether the user hadn't seen it before this request.
    #[sqlx(default)]
    new: bool,
}

const DECISION_COLUMNS: &str =
    "id, reference, subject_kind, subject_label, action, rule, law, facts, automated,
     created_at, appeal, appealed_at, appeal_status, appeal_reason, appeal_decided_at, reversed_at";

fn finish(d: &mut DecisionView) {
    d.action_label = action_label(&d.action);
    d.rule_title = rule_title(&d.rule);
    let until = d.created_at + APPEAL_WINDOW;
    d.appeal_until = Some(until);
    d.can_appeal = d.appeal_status.is_none() && OffsetDateTime::now_utc() < until;
}

/// Decisions about the user's content or account: the statements of reasons.
async fn my_decisions(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Vec<DecisionView>>> {
    let mut rows: Vec<DecisionView> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {DECISION_COLUMNS}, seen_at is null as new from moderation_decisions
         where user_id = $1 order by created_at desc"
    )))
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    for d in &mut rows {
        finish(d);
    }
    sqlx::query(
        "update moderation_decisions set seen_at = now() where user_id = $1 and seen_at is null",
    )
    .bind(user.id)
    .execute(&state.db)
    .await?;
    Ok(Json(rows))
}

/// Decisions the user hasn't opened yet (for the notice on every page).
pub async fn unseen(db: &sqlx::PgPool, user_id: Uuid) -> sqlx::Result<i64> {
    sqlx::query_scalar(
        "select count(*) from moderation_decisions where user_id = $1 and seen_at is null",
    )
    .bind(user_id)
    .fetch_one(db)
    .await
}

/// A counter-notice (17 U.S.C. §512(g)(3)).
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CounterNotice {
    name: String,
    address: String,
    phone: String,
    /// "…under penalty of perjury, I believe in good faith the material was removed by mistake…"
    mistake_under_penalty: bool,
    /// "I consent to the jurisdiction of the federal district court…, and will accept service…"
    consent_to_jurisdiction: bool,
    signature: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppealBody {
    appeal: String,
    counter_notice: Option<CounterNotice>,
}

async fn appeal(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<AppealBody>,
) -> ApiResult<Json<DecisionView>> {
    let text_ = text("appeal", &body.appeal, 4000, true)?;
    let rule: Option<String> =
        sqlx::query_scalar("select rule from moderation_decisions where id = $1 and user_id = $2")
            .bind(id)
            .bind(user.id)
            .fetch_optional(&state.db)
            .await?;
    let rule = rule.ok_or(ApiError::NotFound)?;
    let counter = if rule == "copyright" {
        let c = body.counter_notice.ok_or_else(|| {
            ApiError::invalid(
                "counter_notice",
                "an appeal against a copyright decision is a counter-notice: add its details",
            )
        })?;
        for (field, value, max) in [
            ("counter_notice.name", &c.name, 100),
            ("counter_notice.address", &c.address, 300),
            ("counter_notice.phone", &c.phone, 40),
            ("counter_notice.signature", &c.signature, 100),
        ] {
            text(field, value, max, true)?;
        }
        if !c.mistake_under_penalty || !c.consent_to_jurisdiction {
            return Err(ApiError::invalid(
                "counter_notice.statements",
                "a counter-notice needs both statements",
            ));
        }
        Some(sqlx::types::Json(c))
    } else {
        None
    };
    let n = sqlx::query(
        "update moderation_decisions set appeal = $3, counter_notice = $4, appealed_at = now(), appeal_status = 'pending'
         where id = $1 and user_id = $2 and appeal_status is null and created_at > now() - $5",
    )
    .bind(id)
    .bind(user.id)
    .bind(&text_)
    .bind(counter)
    .bind(APPEAL_WINDOW)
    .execute(&state.db)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(ApiError::Conflict(
            "this decision can't be appealed: it has been appealed already, or six months have passed"
                .into(),
        ));
    }
    let mut d: DecisionView = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {DECISION_COLUMNS} from moderation_decisions where id = $1"
    )))
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    finish(&mut d);
    Ok(Json(d))
}

// ---------------------------------------------------------------------------------------------
// The admin side

#[derive(Serialize)]
pub struct Account {
    id: Uuid,
    name: String,
    closed: bool,
    suspended: bool,
    /// Decisions about them that weren't reversed.
    decisions: i64,
    /// Upheld copyright decisions (the repeat-infringer policy, CON-4).
    copyright_strikes: i64,
}

async fn account(db: &sqlx::PgPool, id: Uuid) -> sqlx::Result<Account> {
    let (name, closed, suspended, decisions, copyright_strikes): (String, bool, bool, i64, i64) =
        sqlx::query_as(
            "select u.display_name, u.closed_at is not null, u.suspended_at is not null,
                    (select count(*) from moderation_decisions d where d.user_id = u.id and d.reversed_at is null),
                    (select count(*) from moderation_decisions d where d.user_id = u.id and d.reversed_at is null
                       and d.rule = 'copyright')
             from users u where u.id = $1",
        )
        .bind(id)
        .fetch_one(db)
        .await?;
    Ok(Account {
        id,
        name,
        closed,
        suspended,
        decisions,
        copyright_strikes,
    })
}

#[derive(Serialize)]
pub struct QueuedReport {
    id: Uuid,
    reference: String,
    subject_kind: String,
    subject_label: String,
    /// Where an admin can look at it.
    link: Option<String>,
    quoted: Option<String>,
    account: Option<Account>,
    reason: String,
    reason_label: &'static str,
    details: String,
    /// Who sent it: an account name, or a signed-out reporter's name and email.
    reporter: String,
    copyright: Option<serde_json::Value>,
    priority: i16,
    #[serde(with = "time::serde::rfc3339")]
    due_at: OffsetDateTime,
    overdue: bool,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    /// Other open reports about the same thing.
    same_subject: i64,
    actions: &'static [&'static str],
    /// The rule the reason points to, to start from.
    suggested_rule: Option<&'static str>,
    preserve: bool,
}

#[derive(sqlx::FromRow)]
struct ReportRow {
    id: Uuid,
    reference: String,
    subject_kind: String,
    subject_id: Uuid,
    subject_user_id: Option<Uuid>,
    subject_label: String,
    quoted: Option<String>,
    reason: String,
    details: String,
    reporter: String,
    copyright: Option<serde_json::Value>,
    priority: i16,
    due_at: OffsetDateTime,
    created_at: OffsetDateTime,
    same_subject: i64,
    preserve: bool,
}

async fn link(db: &sqlx::PgPool, kind: &str, id: Uuid) -> sqlx::Result<Option<String>> {
    Ok(match kind {
        "app" => sqlx::query_scalar::<_, String>("select slug from apps where id = $1")
            .bind(id)
            .fetch_optional(db)
            .await?
            .map(|s| format!("/app/{s}")),
        "profile" => {
            sqlx::query_scalar::<_, String>("select handle from creatives where user_id = $1")
                .bind(id)
                .fetch_optional(db)
                .await?
                .map(|h| format!("/people/{h}"))
        }
        "team" => sqlx::query_scalar::<_, String>("select slug from teams where id = $1")
            .bind(id)
            .fetch_optional(db)
            .await?
            .map(|s| format!("/teams/{s}")),
        "app_review" => sqlx::query_scalar::<_, String>(
            "select a.slug from app_reviews r join apps a on a.id = r.app_id where r.id = $1",
        )
        .bind(id)
        .fetch_optional(db)
        .await?
        .map(|s| format!("/app/{s}#review-{id}")),
        "bounty" => Some(format!("/bounties/{id}")),
        "review" => sqlx::query_scalar::<_, String>(
            "select c.handle from creative_reviews r join creatives c on c.user_id = r.creative_id
             where r.job_id = $1",
        )
        .bind(id)
        .fetch_optional(db)
        .await?
        .map(|h| format!("/people/{h}#review-{id}")),
        _ => None,
    })
}

/// Open reports, urgent first, then by when they're due.
pub async fn queued(db: &sqlx::PgPool) -> ApiResult<Vec<QueuedReport>> {
    let rows: Vec<ReportRow> = sqlx::query_as(
        "select r.id, r.reference, r.subject_kind, r.subject_id, r.subject_user_id, r.subject_label, r.quoted,
                r.reason, r.details,
                coalesce(u.display_name || ' (account)',
                         coalesce(r.reporter_name, 'No name') || ' <' || coalesce(r.reporter_email, 'no email') || '>, signed out')
                  as reporter,
                r.copyright, r.priority, r.due_at, r.created_at,
                (select count(*) from reports o where o.subject_kind = r.subject_kind and o.subject_id = r.subject_id
                   and o.status = 'open' and o.id <> r.id) as same_subject,
                r.preserve_until is not null as preserve
         from reports r left join users u on u.id = r.reporter_id
         where r.status = 'open' order by r.priority, r.due_at limit 200",
    )
    .fetch_all(db)
    .await?;
    let now = OffsetDateTime::now_utc();
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(QueuedReport {
            link: link(db, &r.subject_kind, r.subject_id).await?,
            account: match r.subject_user_id {
                Some(id) => Some(account(db, id).await?),
                None => None,
            },
            actions: actions_for(&r.subject_kind),
            suggested_rule: reason(&r.reason).and_then(|x| x.rule),
            reason_label: reason_label(&r.reason),
            overdue: r.due_at < now,
            id: r.id,
            reference: r.reference,
            subject_kind: r.subject_kind,
            subject_label: r.subject_label,
            quoted: r.quoted,
            reason: r.reason,
            details: r.details,
            reporter: r.reporter,
            copyright: r.copyright,
            priority: r.priority,
            due_at: r.due_at,
            created_at: r.created_at,
            same_subject: r.same_subject,
            preserve: r.preserve,
        });
    }
    Ok(out)
}

#[derive(Serialize)]
pub struct QueuedAppeal {
    #[serde(flatten)]
    decision: DecisionView,
    decided_by: String,
    counter_notice: Option<serde_json::Value>,
    account: Account,
    link: Option<String>,
}

/// Appeals waiting for a person, oldest first.
pub async fn queued_appeals(db: &sqlx::PgPool) -> ApiResult<Vec<QueuedAppeal>> {
    #[derive(sqlx::FromRow)]
    struct Row {
        #[sqlx(flatten)]
        decision: DecisionView,
        decided_by: String,
        counter_notice: Option<serde_json::Value>,
        user_id: Uuid,
        subject_id: Uuid,
    }
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {DECISION_COLUMNS}, decided_by, counter_notice, user_id, subject_id
         from moderation_decisions where appeal_status = 'pending' order by appealed_at limit 200"
    )))
    .fetch_all(db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for mut r in rows {
        finish(&mut r.decision);
        out.push(QueuedAppeal {
            link: link(db, &r.decision.subject_kind, r.subject_id).await?,
            account: account(db, r.user_id).await?,
            decided_by: r.decided_by,
            counter_notice: r.counter_notice,
            decision: r.decision,
        });
    }
    Ok(out)
}

fn refused(err: anyhow::Error) -> ApiError {
    ApiError::Conflict(format!("{err:#}"))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ActBody {
    action: String,
    rule: String,
    law: Option<String>,
    /// The facts, for the statement of reasons the user gets.
    facts: String,
    /// What the reporter is told.
    outcome: String,
    /// Where it was passed on (NCMEC, the NCA, the police) and its reference.
    external_reference: Option<String>,
}

/// Acts on a report: carries out the decision, sends the statement of reasons, and closes every
/// open report about the same thing.
async fn act(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(b): Json<ActBody>,
) -> ApiResult<axum::response::Response> {
    admin.need(crate::staff::Perm::Moderate)?;
    if b.action == "suspend_account" {
        // Suspending someone is sensitive: it waits for enough staff (staff.rs).
        admin.need(crate::staff::Perm::Sensitive)?;
        validate_act(&b)?;
        let label: Option<String> = sqlx::query_scalar(
            "select subject_label from reports where id = $1 and status = 'open'",
        )
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
        let label = label.ok_or_else(|| ApiError::Conflict("that report isn't open".into()))?;
        return Ok(crate::staff::request(
            &state,
            &admin,
            "report.act",
            &format!(
                "Suspend the account behind report about {label} ({})",
                b.rule
            ),
            json!({ "report_id": id, "body": b }),
        )
        .await?
        .into_response());
    }
    Ok(Json(act_on_report(&state, id, &b, &admin.actor).await?).into_response())
}

fn validate_act(b: &ActBody) -> ApiResult<()> {
    text("facts", &b.facts, 4000, true)?;
    text("outcome", &b.outcome, 2000, true)?;
    text("law", b.law.as_deref().unwrap_or(""), 300, false)?;
    text(
        "external_reference",
        b.external_reference.as_deref().unwrap_or(""),
        200,
        false,
    )?;
    Ok(())
}

/// A suspension through a report, once approved (staff.rs).
pub(crate) async fn act_on_report_stored(
    state: &AppState,
    payload: &serde_json::Value,
    actor: &str,
) -> ApiResult<serde_json::Value> {
    let id: Uuid = serde_json::from_value(payload["report_id"].clone())
        .map_err(|e| ApiError::Internal(e.into()))?;
    let b: ActBody = serde_json::from_value(payload["body"].clone())
        .map_err(|e| ApiError::Internal(e.into()))?;
    act_on_report(state, id, &b, actor).await
}

/// Decides an open report: the decision, closing the reports about the same thing, and telling
/// the reporters.
async fn act_on_report(
    state: &AppState,
    id: Uuid,
    b: &ActBody,
    actor: &str,
) -> ApiResult<serde_json::Value> {
    let facts = text("facts", &b.facts, 4000, true)?;
    let outcome = text("outcome", &b.outcome, 2000, true)?;
    let law = text("law", b.law.as_deref().unwrap_or(""), 300, false)?;
    let external = text(
        "external_reference",
        b.external_reference.as_deref().unwrap_or(""),
        200,
        false,
    )?;
    let mut tx = state.db.begin().await?;
    let row: Option<(String, Uuid, Option<Uuid>, String)> = sqlx::query_as(
        "select subject_kind, subject_id, subject_user_id, subject_label from reports
         where id = $1 and status = 'open' for update",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let (kind, subject_id, user_id, label) =
        row.ok_or_else(|| ApiError::Conflict("that report isn't open".into()))?;
    if !actions_for(&kind).contains(&b.action.as_str()) {
        return Err(ApiError::invalid(
            "action",
            "that action doesn't apply here",
        ));
    }
    let user_id = user_id
        .context("the report isn't about an account")
        .map_err(refused)?;
    // Removing a reply is a decision about the app's maker, who wrote it.
    let user_id = if b.action == "remove_reply" {
        sqlx::query_scalar::<_, Uuid>(
            "select a.maker_id from app_reviews r join apps a on a.id = r.app_id
             where r.id = $1 and r.reply is not null and r.reply_hidden_at is null",
        )
        .bind(subject_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::invalid("action", "there's no reply to remove"))?
    } else {
        user_id
    };
    let decision = decide(
        &mut tx,
        &NewDecision {
            user_id,
            subject_kind: if b.action == "suspend_account" {
                "account"
            } else {
                &kind
            },
            subject_id: if b.action == "suspend_account" {
                user_id
            } else {
                subject_id
            },
            subject_label: &label,
            action: &b.action,
            rule: &b.rule,
            law: Some(&law),
            facts: &facts,
            decided_by: actor,
        },
    )
    .await
    .map_err(refused)?;
    let closed: Vec<Answered> = sqlx::query_as(
        "update reports set status = 'actioned', outcome = $3, decided_by = $4, decided_at = now(), decision_id = $5,
                external_reference = coalesce(nullif($6, ''), external_reference)
         where status = 'open' and (id = $1 or (subject_kind = $7 and subject_id = $2))
         returning reference, subject_label, reporter_id, reporter_email",
    )
    .bind(id)
    .bind(subject_id)
    .bind(&outcome)
    .bind(actor)
    .bind(decision.id)
    .bind(&external)
    .bind(&kind)
    .fetch_all(&mut *tx)
    .await?;
    tell_reporters(&mut tx, &state.config, &closed, &outcome).await?;
    tx.commit().await?;
    crate::bounties::forget_cards(state, &decision.cards).await;
    Ok(json!({ "decision": decision.id }))
}

/// A report that was just answered.
#[derive(sqlx::FromRow)]
struct Answered {
    reference: String,
    subject_label: String,
    reporter_id: Option<Uuid>,
    reporter_email: Option<String>,
}

/// Tells each reporter what was decided (DSA Art. 16(5)): in SLAPorSLOP, or by email when they
/// reported signed out.
async fn tell_reporters(
    tx: &mut sqlx::PgConnection,
    config: &crate::config::Config,
    answered: &[Answered],
    outcome: &str,
) -> sqlx::Result<()> {
    for a in answered {
        let title = format!("Your report {} was answered", a.reference);
        if let Some(user_id) = a.reporter_id {
            crate::notify::send(
                tx,
                crate::notify::Notice {
                    user_id,
                    category: "moderation",
                    title,
                    body: format!("{}: {outcome}", a.subject_label),
                    link: "/account#reports".into(),
                    collapse_key: None,
                },
            )
            .await?;
        } else if let Some(email) = &a.reporter_email {
            crate::notify::email(
                tx,
                config,
                email,
                &title,
                &format!(
                    "You reported {} to SLAPorSLOP.\n\nWhat we decided: {outcome}",
                    a.subject_label
                ),
            )
            .await?;
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DismissBody {
    outcome: String,
    external_reference: Option<String>,
}

/// Closes a report without action, telling the reporter why.
async fn dismiss(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(b): Json<DismissBody>,
) -> ApiResult<axum::http::StatusCode> {
    admin.need(crate::staff::Perm::Moderate)?;
    let outcome = text("outcome", &b.outcome, 2000, true)?;
    let external = text(
        "external_reference",
        b.external_reference.as_deref().unwrap_or(""),
        200,
        false,
    )?;
    let mut tx = state.db.begin().await?;
    let closed: Vec<Answered> = sqlx::query_as(
        "update reports set status = 'dismissed', outcome = $2, decided_by = $3, decided_at = now(),
                external_reference = nullif($4, '')
         where id = $1 and status = 'open'
         returning reference, subject_label, reporter_id, reporter_email",
    )
    .bind(id)
    .bind(&outcome)
    .bind(&admin.actor)
    .bind(&external)
    .fetch_all(&mut *tx)
    .await?;
    if closed.is_empty() {
        return Err(ApiError::Conflict("that report isn't open".into()));
    }
    tell_reporters(&mut tx, &state.config, &closed, &outcome).await?;
    admin::audit(
        &mut tx,
        &admin.actor,
        "report.dismiss",
        &id.to_string(),
        json!({ "outcome": outcome }),
    )
    .await?;
    tx.commit().await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppealDecision {
    reverse: bool,
    reason: String,
}

#[derive(Serialize)]
struct AppealDecided {
    /// Reversed, but the app can't go back in the store yet: why, as the maker is told.
    stays_out: Option<String>,
}

/// Keeps or reverses a decision on appeal, with the reason the user is shown.
async fn decide_appeal(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(b): Json<AppealDecision>,
) -> ApiResult<Json<AppealDecided>> {
    admin.need(crate::staff::Perm::Moderate)?;
    let reason = text("reason", &b.reason, 2000, true)?;
    let mut tx = state.db.begin().await?;
    let row: Option<(String, String, Uuid, Uuid, String, String)> = sqlx::query_as(
        "select action, subject_kind, subject_id, user_id, rule, subject_label from moderation_decisions
         where id = $1 and appeal_status = 'pending' for update",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let (action, kind, subject_id, user_id, rule, label) =
        row.ok_or_else(|| ApiError::Conflict("that appeal isn't pending".into()))?;
    let applied = if b.reverse {
        apply(&mut tx, &action, &kind, subject_id, user_id, &rule, true)
            .await
            .map_err(refused)?
    } else {
        Applied::default()
    };
    crate::notify::send(
        &mut tx,
        crate::notify::Notice {
            user_id,
            category: "moderation",
            title: format!(
                "Your appeal about {label}: {}",
                if b.reverse {
                    "the decision is reversed"
                } else {
                    "the decision stands"
                }
            ),
            body: match &applied.stays_out {
                Some(why) => format!("{reason}\n\n{why}"),
                None => reason.clone(),
            },
            link: "/account#decisions".into(),
            collapse_key: None,
        },
    )
    .await?;
    sqlx::query(
        "update moderation_decisions set appeal_status = $2, appeal_reason = $3, appeal_decided_by = $4,
                appeal_decided_at = now(), reversed_at = case when $5 then now() end, seen_at = null
         where id = $1",
    )
    .bind(id)
    .bind(if b.reverse { "reversed" } else { "upheld" })
    .bind(&reason)
    .bind(&admin.actor)
    .bind(b.reverse)
    .execute(&mut *tx)
    .await?;
    admin::audit(
        &mut tx,
        &admin.actor,
        if b.reverse {
            "appeal.reverse"
        } else {
            "appeal.uphold"
        },
        &id.to_string(),
        json!({ "reason": reason, "stays_out": applied.stays_out }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(AppealDecided {
        stays_out: applied.stays_out,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rule_is_a_section_of_the_content_policy() {
        let policy = include_str!("../legal/content.md");
        for (code, title) in RULES {
            assert!(
                policy.contains(&format!("\n### {title}\n")),
                "{code}: {title}"
            );
        }
        for r in REASONS {
            if let Some(rule) = r.rule {
                assert!(RULES.iter().any(|(c, _)| *c == rule), "{}", r.code);
            }
        }
    }

    #[test]
    fn references_are_short_and_readable() {
        let r = reference('R');
        assert_eq!(r.len(), 11);
        assert!(r.starts_with("R-"));
        assert!(!r.contains(['0', 'O', '1', 'I']));
    }
}
