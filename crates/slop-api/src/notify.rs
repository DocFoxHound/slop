//! Notifications in Slop (the bell) and product emails (decided 2026-10-20).
//!
//! Anything that should reach a user calls `send` inside its own transaction; it only writes a
//! row. The email worker (`run_email`, only when SMTP is set up) then emails the ones the user
//! wants by email (`users.email_prefs`, per category, on unless switched off) and sends the
//! outbox. Signed-out reporters, who have no account, get their report's outcome through
//! `email` straight into the outbox.
//!
//! Only product messages: job steps, Interested apps moving on, moderation decisions and report
//! outcomes. Never marketing (LEGAL-GUIDANCE TERMS-3).

use std::time::Duration as StdDuration;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::CurrentUser,
    config::Config,
    error::{ApiError, ApiResult},
};

pub const CATEGORIES: &[(&str, &str)] = &[
    (
        "jobs",
        "People jobs: offers, payments, deliveries, messages and problems",
    ),
    (
        "interested",
        "Apps you're Interested in reaching beta or release",
    ),
    (
        "moderation",
        "Decisions about your content, appeals and your reports",
    ),
    (
        "donations",
        "Donations: receipts, server donation reminders and changes to where yours are paid",
    ),
    ("shared", "Private Slop someone shares with you"),
    (
        "teams",
        "Your teams: invitations, game nights and verification",
    ),
    (
        "apps",
        "Your apps: going public, and when Slop can't read a repository",
    ),
    ("chat", "New chat messages while you're away"),
    ("friends", "Friend requests, and when someone accepts yours"),
];

/// Pending notifications older than this aren't emailed (e.g. when email is switched on later).
const EMAIL_WITHIN: time::Duration = time::Duration::hours(24);
const MAX_ATTEMPTS: i32 = 5;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/notifications", get(list))
        .route("/me/notifications/read", post(mark_read))
        .route("/me/email-prefs", get(prefs).put(set_prefs))
}

/// A notification to write.
pub(crate) struct Notice<'a> {
    pub user_id: Uuid,
    pub category: &'a str,
    pub title: String,
    pub body: String,
    pub link: String,
    /// Unread notifications with the same key are merged into one.
    pub collapse_key: Option<String>,
}

/// Writes a notification (or adds to an unread one with the same collapse key). Closed accounts
/// get nothing.
pub(crate) async fn send(conn: &mut sqlx::PgConnection, n: Notice<'_>) -> sqlx::Result<()> {
    let open: bool = sqlx::query_scalar(
        "select exists (select 1 from users where id = $1 and closed_at is null)",
    )
    .bind(n.user_id)
    .fetch_one(&mut *conn)
    .await?;
    if !open {
        return Ok(());
    }
    if let Some(key) = &n.collapse_key {
        let merged = sqlx::query(
            "update notifications set count = count + 1, title = $3, body = $4, created_at = now(),
                    pushed_at = case when pushed_at < now() - interval '2 minutes' then null else pushed_at end
             where user_id = $1 and collapse_key = $2 and read_at is null",
        )
        .bind(n.user_id)
        .bind(key)
        .bind(&n.title)
        .bind(&n.body)
        .execute(&mut *conn)
        .await?
        .rows_affected();
        if merged > 0 {
            return Ok(());
        }
    }
    sqlx::query(
        "insert into notifications (id, user_id, category, title, body, link, collapse_key)
         values ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(Uuid::now_v7())
    .bind(n.user_id)
    .bind(n.category)
    .bind(clip(&n.title, 200))
    .bind(clip(&n.body, 1000))
    .bind(&n.link)
    .bind(&n.collapse_key)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}

/// Queues an email to someone without an account (a signed-out reporter). Does nothing when
/// email isn't set up.
pub(crate) async fn email(
    conn: &mut sqlx::PgConnection,
    config: &Config,
    to: &str,
    subject: &str,
    body: &str,
) -> sqlx::Result<()> {
    if config.email.is_none() || !to.contains('@') {
        return Ok(());
    }
    sqlx::query("insert into email_outbox (id, to_address, subject, body) values ($1, $2, $3, $4)")
        .bind(Uuid::now_v7())
        .bind(to)
        .bind(subject)
        .bind(format!(
            "{body}\n\n— Slop\n{}\n\nYou get this email because you sent a report to Slop.",
            config.public_url
        ))
        .execute(conn)
        .await?;
    Ok(())
}

/// Queues an email the user gets whatever their email choices: confirmations and reminders the
/// law asks for (server donations: PAY-15, PAY-16, PAY-17). Nothing without an email or SMTP.
pub(crate) async fn email_user(
    conn: &mut sqlx::PgConnection,
    config: &Config,
    user_id: Uuid,
    subject: &str,
    body: &str,
) -> sqlx::Result<()> {
    if config.email.is_none() {
        return Ok(());
    }
    let row: Option<(String, String)> = sqlx::query_as(
        "select email, display_name from users where id = $1 and email is not null and closed_at is null",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((to, name)) = row else {
        return Ok(());
    };
    sqlx::query("insert into email_outbox (id, to_address, subject, body) values ($1, $2, $3, $4)")
        .bind(Uuid::now_v7())
        .bind(to)
        .bind(subject)
        .bind(format!(
            "Hi {name},\n\n{body}\n\n— Slop\n{}",
            config.public_url
        ))
        .execute(conn)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The bell

#[derive(Serialize, sqlx::FromRow)]
struct Item {
    id: Uuid,
    category: String,
    title: String,
    body: String,
    link: String,
    count: i32,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    read: bool,
}

#[derive(Serialize)]
struct Inbox {
    unread: i64,
    items: Vec<Item>,
}

pub async fn unread(db: &PgPool, user_id: Uuid) -> sqlx::Result<i64> {
    sqlx::query_scalar("select count(*) from notifications where user_id = $1 and read_at is null")
        .bind(user_id)
        .fetch_one(db)
        .await
}

async fn list(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Inbox>> {
    let items = sqlx::query_as(
        "select id, category, title, body, link, count, created_at, read_at is not null as read
         from notifications where user_id = $1 order by created_at desc limit 60",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(Inbox {
        unread: unread(&state.db, user.id).await?,
        items,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadBody {
    /// These, or every unread one when empty.
    #[serde(default)]
    ids: Vec<Uuid>,
}

async fn mark_read(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<ReadBody>,
) -> ApiResult<StatusCode> {
    sqlx::query(
        "update notifications set read_at = now()
         where user_id = $1 and read_at is null and (cardinality($2::uuid[]) = 0 or id = any($2))",
    )
    .bind(user.id)
    .bind(&body.ids)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct Pref {
    category: &'static str,
    label: &'static str,
    email: bool,
}

#[derive(Serialize)]
struct Prefs {
    /// Whether this server sends email at all.
    email_on: bool,
    has_email: bool,
    categories: Vec<Pref>,
}

async fn load_prefs(state: &AppState, user_id: Uuid) -> ApiResult<Prefs> {
    let (prefs, email): (Value, Option<String>) =
        sqlx::query_as("select email_prefs, email from users where id = $1")
            .bind(user_id)
            .fetch_one(&state.db)
            .await?;
    Ok(Prefs {
        email_on: state.config.email.is_some(),
        has_email: email.is_some(),
        categories: CATEGORIES
            .iter()
            .map(|(category, label)| Pref {
                category,
                label,
                email: prefs
                    .get(*category)
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            })
            .collect(),
    })
}

async fn prefs(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Prefs>> {
    Ok(Json(load_prefs(&state, user.id).await?))
}

async fn set_prefs(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<Map<String, Value>>,
) -> ApiResult<Json<Prefs>> {
    let mut clean = Map::new();
    for (k, v) in body {
        if !CATEGORIES.iter().any(|(c, _)| *c == k) {
            return Err(ApiError::invalid(
                "category",
                format!("\"{k}\" isn't a kind of email"),
            ));
        }
        let on = v
            .as_bool()
            .ok_or_else(|| ApiError::invalid("category", "true or false"))?;
        clean.insert(k, Value::Bool(on));
    }
    sqlx::query("update users set email_prefs = email_prefs || $2 where id = $1")
        .bind(user.id)
        .bind(Value::Object(clean))
        .execute(&state.db)
        .await?;
    Ok(Json(load_prefs(&state, user.id).await?))
}

// ---------------------------------------------------------------------------------------------
// Email

/// Turns pending notifications into emails (or skips them), then sends the outbox. Runs every
/// 20 seconds when SMTP is set up.
pub async fn run_email(state: AppState) {
    let Some(cfg) = state.config.email.clone() else {
        tracing::info!("SLOP_SMTP_URL / SLOP_EMAIL_FROM not set: notifications aren't emailed");
        return;
    };
    let transport =
        match lettre::AsyncSmtpTransport::<lettre::Tokio1Executor>::from_url(&cfg.smtp_url) {
            Ok(t) => t.build(),
            Err(err) => {
                tracing::error!(error = %err, "SLOP_SMTP_URL isn't a valid SMTP URL; email is off");
                return;
            }
        };
    loop {
        if let Err(err) = queue_emails(&state.db, &state.config).await {
            tracing::warn!(error = ?err, "queueing notification emails failed");
        }
        if let Err(err) = send_outbox(&state.db, &transport, &cfg.from).await {
            tracing::warn!(error = ?err, "sending emails failed");
        }
        tokio::time::sleep(StdDuration::from_secs(20)).await;
    }
}

/// Pending notifications → outbox rows for users who want that category by email.
pub async fn queue_emails(db: &PgPool, config: &Config) -> anyhow::Result<usize> {
    let mut tx = db.begin().await?;
    // Too old to be news, or for users who don't want them: skipped.
    sqlx::query(
        "update notifications n set email_status = 'skipped'
         from users u where u.id = n.user_id and n.email_status = 'pending'
           and (n.created_at < now() - $1 or u.email is null or u.closed_at is not null
                or coalesce((u.email_prefs->>n.category)::boolean, true) = false)",
    )
    .bind(EMAIL_WITHIN)
    .execute(&mut *tx)
    .await?;
    let rows: Vec<(Uuid, String, String, String, String, String)> = sqlx::query_as(
        "select n.id, u.email, u.display_name, n.title, n.body, n.link from notifications n
         join users u on u.id = n.user_id where n.email_status = 'pending'
         order by n.created_at limit 200 for update of n skip locked",
    )
    .fetch_all(&mut *tx)
    .await?;
    for (id, to, name, title, body, link) in &rows {
        let url = config
            .public_url
            .join(link)
            .map(String::from)
            .unwrap_or_default();
        let account = config
            .public_url
            .join("/account#notifications")
            .map(String::from)
            .unwrap_or_default();
        let text = format!(
            "Hi {name},\n\n{title}\n{}{}\n\nOpen it: {url}\n\n— Slop\n\nYou can choose which emails Slop sends you on your account page: {account}",
            if body.is_empty() { "" } else { "\n" },
            body
        );
        sqlx::query(
            "insert into email_outbox (id, to_address, subject, body) values ($1, $2, $3, $4)",
        )
        .bind(Uuid::now_v7())
        .bind(to)
        .bind(title)
        .bind(text)
        .execute(&mut *tx)
        .await?;
        sqlx::query("update notifications set email_status = 'queued' where id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(rows.len())
}

async fn send_outbox(
    db: &PgPool,
    transport: &lettre::AsyncSmtpTransport<lettre::Tokio1Executor>,
    from: &str,
) -> anyhow::Result<()> {
    use lettre::AsyncTransport;
    let rows: Vec<(Uuid, String, String, String)> = sqlx::query_as(
        "select id, to_address, subject, body from email_outbox
         where sent_at is null and failed_at is null order by created_at limit 50",
    )
    .fetch_all(db)
    .await?;
    for (id, to, subject, body) in rows {
        let result = async {
            let message = lettre::Message::builder()
                .from(from.parse()?)
                .to(to.parse()?)
                .subject(subject)
                .header(lettre::message::header::ContentType::TEXT_PLAIN)
                .body(body)?;
            transport.send(message).await?;
            anyhow::Ok(())
        }
        .await;
        match result {
            Ok(()) => {
                sqlx::query("update email_outbox set sent_at = now(), attempts = attempts + 1 where id = $1")
                    .bind(id)
                    .execute(db)
                    .await?;
            }
            Err(err) => {
                sqlx::query(
                    "update email_outbox set attempts = attempts + 1, last_error = $2,
                            failed_at = case when attempts + 1 >= $3 then now() end
                     where id = $1",
                )
                .bind(id)
                .bind(format!("{err:#}"))
                .bind(MAX_ATTEMPTS)
                .execute(db)
                .await?;
            }
        }
    }
    Ok(())
}

/// Hourly: read notifications go after 180 days, any after a year; sent emails after 30 days.
pub async fn cleanup(db: &PgPool) -> sqlx::Result<()> {
    sqlx::query(
        "delete from notifications where (read_at < now() - interval '180 days') or created_at < now() - interval '1 year'",
    )
    .execute(db)
    .await?;
    sqlx::query(
        "delete from email_outbox where coalesce(sent_at, failed_at) < now() - interval '30 days'",
    )
    .execute(db)
    .await?;
    Ok(())
}
