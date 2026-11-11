// SPDX-License-Identifier: AGPL-3.0-or-later
//! Notifications in SLAPorSLOP (the bell) and product emails (decided 2026-10-20).
//!
//! Anything that should reach a user calls `send` inside its own transaction; it only writes a
//! row. The email worker (`run_email`, only when SMTP is set up) then emails the ones the user
//! wants by email (`users.email_prefs`, per category, on unless switched off) and sends the
//! outbox. Signed-out reporters, who have no account, get their report's outcome through
//! `email` straight into the outbox.
//!
//! Only product messages: job steps, Interested apps moving on, news from the makers of
//! Interested apps (under the bell only), moderation decisions, report outcomes and changes to the
//! terms and policies. Never marketing (LEGAL-GUIDANCE TERMS-3).

use std::time::Duration as StdDuration;

use std::collections::BTreeMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
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
    ("shared", "Private apps someone shares with you"),
    (
        "teams",
        "Your teams: invitations, game nights and verification",
    ),
    (
        "apps",
        "Your apps: every release (received, approved, rejected, withdrawn), going public, and when SLAPorSLOP can't read a repository",
    ),
    ("chat", "New chat messages while you're away"),
    ("friends", "Friend requests, and when someone accepts yours"),
    (
        "security",
        "Your account's security: passkeys added, sign-ins linked, accounts combined",
    ),
    (
        "agents",
        "Your AI agent tokens: a week and a day before one expires, and when it has",
    ),
    (
        "policies",
        "Changes to SLAPorSLOP's terms and policies, before they take effect",
    ),
];

/// Kinds that show under the bell only: never emailed, never pushed to a device, and no email
/// switch on the account page. Maker news is here until the lawyer has answered whether emailing
/// it is a product message or marketing (LEGAL-GUIDANCE TERMS-3). To switch email on for a kind
/// later, move its entry to `CATEGORIES`. Moving a kind out of here makes `Notifications.tsx`
/// KINDS and the Privacy Policy false: change both, and publish a new policy version, in the same
/// change.
pub const BELL_ONLY: &[(&str, &str)] = &[("news", "News from makers of apps you're Interested in")];

/// The bell-only kinds' names, for binding.
pub fn bell_only() -> Vec<&'static str> {
    BELL_ONLY.iter().map(|(k, _)| *k).collect()
}

/// Pending notifications older than this aren't emailed (e.g. when email is switched on later).
const EMAIL_WITHIN: time::Duration = time::Duration::hours(24);
const MAX_ATTEMPTS: i32 = 5;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/notifications", get(list))
        .route("/me/notifications/read", post(mark_read))
        .route("/me/notifications/unread", post(mark_unread))
        .route("/me/notifications/{id}", delete(dismiss))
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
        .bind(clip(&n.title, 200))
        .bind(clip(&n.body, 1000))
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

pub(crate) fn clip(s: &str, max: usize) -> String {
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
            "{body}\n\n— SLAPorSLOP\n{}\n\nYou get this email because you sent a report to SLAPorSLOP.",
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
            "Hi {name},\n\n{body}\n\n— SLAPorSLOP\n{}",
            config.public_url
        ))
        .execute(conn)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Maker news (news.rs): under the bell only

/// Tells everyone with a current Interested mark and news on for this app, under the bell only:
/// written as already pushed and never emailed. Unread news from one app is one notice with a
/// count, showing the newest post. Returns how many people were told.
///
/// One statement in the poster's transaction, one row per person who is told. With the index
/// `interests_app` this is the same work as `interested::stage_moved` does for the same people,
/// and it happens at most once per app per 24 hours. Ordered by person, so two statements on the
/// same notices take their locks in one order.
pub(crate) async fn news_posted(
    conn: &mut sqlx::PgConnection,
    app_id: Uuid,
    post_id: Uuid,
    app_title: &str,
    slug: &str,
    headline: &str,
) -> sqlx::Result<u64> {
    // Open, not suspended, not the maker, and the app is in their age view (policy.rs).
    let sql = format!(
        "insert into notifications (id, user_id, category, title, body, link, collapse_key, email_status, pushed_at, news_id)
         select gen_random_uuid(), i.user_id, 'news', $2, $3, $4, $5, 'skipped', now(), $6
         from interests i
         join users u on u.id = i.user_id
         join apps a on a.id = i.app_id
         where i.app_id = $1 and i.news
           and u.closed_at is null and u.suspended_at is null
           and u.id <> a.maker_id
           and a.min_age <= {}
         order by i.user_id
         on conflict (user_id, collapse_key) where read_at is null and collapse_key is not null
         do update set count = notifications.count + 1, title = excluded.title, body = excluded.body,
                       created_at = now(), news_id = excluded.news_id",
        crate::policy::account_max_min_age_sql("u")
    );
    Ok(sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(app_id)
        .bind(clip(&format!("{app_title}: news from its maker"), 200))
        .bind(clip(headline, 1000))
        .bind(format!("/app/{slug}#news"))
        .bind(format!("news:{app_id}"))
        .bind(post_id)
        .execute(conn)
        .await?
        .rows_affected())
}

// The functions below take a post's notices in person order, as `news_posted` does, so two of
// them never wait on each other's rows in a circle. Their callers hold the post's row (or the
// app's) first.

/// A post was hidden (or is about to be removed): its notices go.
pub(crate) async fn news_withdrawn(
    conn: &mut sqlx::PgConnection,
    post_id: Uuid,
) -> sqlx::Result<()> {
    sqlx::query(
        "delete from notifications where id in (
             select id from notifications where news_id = $1 order by user_id, id for update)",
    )
    .bind(post_id)
    .execute(conn)
    .await?;
    Ok(())
}

/// A post's headline changed: its notices say the new one. Nobody is told again.
pub(crate) async fn news_reworded(
    conn: &mut sqlx::PgConnection,
    post_id: Uuid,
    headline: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        "update notifications set body = $2 where id in (
             select id from notifications where news_id = $1 order by user_id, id for update)",
    )
    .bind(post_id)
    .bind(clip(headline, 1000))
    .execute(conn)
    .await?;
    Ok(())
}

/// The app's page left the store: every notice about its news goes. Nobody is told again if it
/// comes back. Called by a staff delisting (moderation.rs) and at the end of a frozen app's grace
/// (frozen.rs); every other way out of the store is swept within the hour (news.rs `cleanup`).
pub(crate) async fn news_gone_for_app(
    conn: &mut sqlx::PgConnection,
    app_id: Uuid,
) -> sqlx::Result<()> {
    sqlx::query(
        "delete from notifications where id in (
             select id from notifications
             where news_id in (select id from app_news where app_id = $1)
             order by user_id, id for update)",
    )
    .bind(app_id)
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
    /// Unread, of every kind the caller can see.
    unread: i64,
    /// Unread per kind (only kinds with some).
    counts: BTreeMap<String, i64>,
    items: Vec<Item>,
    /// Whether there are older ones than these (ask again with `before` = the last one's id).
    more: bool,
    /// For a maker's AI agent: its own token's expiry, so it can tell its person in time.
    #[serde(skip_serializing_if = "Option::is_none")]
    your_token: Option<YourToken>,
}

#[derive(Serialize)]
struct YourToken {
    name: String,
    #[serde(with = "time::serde::rfc3339")]
    expires_at: OffsetDateTime,
    days_left: i64,
    /// What to tell the person, when it's time.
    note: String,
}

/// The categories a caller doesn't see: a maker's AI agent reads only what's about maker work,
/// never chats, friends or account security, nor news about apps the person follows (that isn't
/// maker work), and People jobs only with a token allowed to handle them.
fn hidden_for(user: &CurrentUser) -> Vec<&'static str> {
    match &user.agent {
        Some(agent) if agent.people => vec!["chat", "friends", "security", "news"],
        Some(_) => vec!["chat", "friends", "security", "jobs", "news"],
        None => Vec::new(),
    }
}

/// A kind of notification named in a request: one of `CATEGORIES` or `BELL_ONLY`.
fn known_category(category: Option<&str>) -> ApiResult<Option<&str>> {
    match category {
        None | Some("") => Ok(None),
        Some(c) if CATEGORIES.iter().chain(BELL_ONLY).any(|(k, _)| *k == c) => Ok(Some(c)),
        Some(c) => Err(ApiError::invalid(
            "category",
            format!("\"{c}\" isn't a kind of notification"),
        )),
    }
}

/// Unread notifications the caller can see (an agent's count leaves out what it can't).
pub async fn unread(db: &PgPool, user: &CurrentUser) -> sqlx::Result<i64> {
    sqlx::query_scalar(
        "select count(*) from notifications where user_id = $1 and read_at is null and not (category = any($2))",
    )
    .bind(user.id)
    .bind(hidden_for(user))
    .fetch_one(db)
    .await
}

const PAGE: i64 = 60;

#[derive(Deserialize)]
struct ListQuery {
    /// Only this kind.
    category: Option<String>,
    /// Only unread ones.
    #[serde(default)]
    unread: bool,
    /// Older than this notification (the last one of the previous page).
    before: Option<Uuid>,
    /// How many, 1 to 100 (60 when left out).
    limit: Option<i64>,
    /// The bell and the notifications page send true. Without it the answer leaves out the
    /// bell-only kinds, so a service worker (which sends no query, web/public/sw.js) can never
    /// show one on a device.
    bell: Option<bool>,
}

async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Inbox>> {
    let category = known_category(q.category.as_deref())?;
    let limit = q.limit.unwrap_or(PAGE).clamp(1, 100);
    let mut hidden = hidden_for(&user);
    if q.bell != Some(true) {
        hidden.extend(bell_only());
    }
    // Newest first; a page continues from the row it names, which must be the caller's.
    let mut items: Vec<Item> = sqlx::query_as(
        "select id, category, title, body, link, count, created_at, read_at is not null as read
         from notifications n where n.user_id = $1 and not (n.category = any($2))
           and ($3::text is null or n.category = $3)
           and (not $4 or n.read_at is null)
           and ($5::uuid is null or (n.created_at, n.id) <
                (select p.created_at, p.id from notifications p where p.id = $5 and p.user_id = $1))
         order by n.created_at desc, n.id desc limit $6",
    )
    .bind(user.id)
    .bind(&hidden)
    .bind(category)
    .bind(q.unread)
    .bind(q.before)
    .bind(limit + 1)
    .fetch_all(&state.db)
    .await?;
    let more = items.len() as i64 > limit;
    items.truncate(limit as usize);
    let counts: Vec<(String, i64)> = sqlx::query_as(
        "select category, count(*) from notifications
         where user_id = $1 and read_at is null and not (category = any($2)) group by category",
    )
    .bind(user.id)
    .bind(&hidden)
    .fetch_all(&state.db)
    .await?;
    let your_token = match &user.agent {
        Some(_) => crate::maker_agents::own_token(&state.db, user.session_id)
            .await?
            .map(|t| YourToken {
                days_left: crate::maker_agents::days_left(t.expires_at, OffsetDateTime::now_utc()),
                name: t.name,
                expires_at: t.expires_at,
                note: t.note,
            }),
        None => None,
    };
    Ok(Json(Inbox {
        unread: counts.iter().map(|(_, n)| n).sum(),
        counts: counts.into_iter().collect(),
        items,
        more,
        your_token,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadBody {
    /// These, or every unread one when empty.
    #[serde(default)]
    ids: Vec<Uuid>,
    /// With no ids: every unread one of this kind.
    #[serde(default)]
    category: Option<String>,
}

async fn mark_read(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<ReadBody>,
) -> ApiResult<StatusCode> {
    let category = known_category(body.category.as_deref())?;
    sqlx::query(
        "update notifications set read_at = now()
         where user_id = $1 and read_at is null and not (category = any($3))
           and (cardinality($2::uuid[]) > 0 and id = any($2)
                or cardinality($2::uuid[]) = 0 and ($4::text is null or category = $4))",
    )
    .bind(user.id)
    .bind(&body.ids)
    .bind(hidden_for(&user))
    .bind(category)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnreadBody {
    ids: Vec<Uuid>,
}

/// Marks notifications unread again, to come back to. One that would now be merged with a newer
/// unread one of the same thread stays on its own (only one unread per collapse key), and it
/// isn't pushed to phones again.
async fn mark_unread(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<UnreadBody>,
) -> ApiResult<StatusCode> {
    if body.ids.is_empty() || body.ids.len() > 100 {
        return Err(ApiError::invalid("ids", "1 to 100 notifications"));
    }
    let hidden = hidden_for(&user);
    let mut tx = state.db.begin().await?;
    for id in &body.ids {
        sqlx::query(
            "update notifications n set read_at = null, pushed_at = coalesce(n.pushed_at, now()),
                    collapse_key = case when exists (
                        select 1 from notifications o where o.user_id = n.user_id
                          and o.collapse_key = n.collapse_key and o.read_at is null and o.id <> n.id)
                      then null else n.collapse_key end
             where n.id = $2 and n.user_id = $1 and n.read_at is not null and not (n.category = any($3))",
        )
        .bind(user.id)
        .bind(id)
        .bind(&hidden)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Removes one of the caller's notifications from the bell (what it was about stays where it is).
async fn dismiss(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let gone = sqlx::query(
        "delete from notifications where id = $1 and user_id = $2 and not (category = any($3))",
    )
    .bind(id)
    .bind(user.id)
    .bind(hidden_for(&user))
    .execute(&state.db)
    .await?
    .rows_affected();
    if gone == 0 {
        return Err(ApiError::NotFound);
    }
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
    let transport = match lettre::AsyncSmtpTransport::<lettre::Tokio1Executor>::from_url(&smtp_url(
        &cfg.smtp_url,
    )) {
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

/// `smtp://user:password@host:587` means STARTTLS (config.rs, .env.example), but lettre reads a
/// plain `smtp://` as no TLS at all, which would send the password and every email in the clear:
/// with a password and no `tls=` of its own, the address requires STARTTLS.
fn smtp_url(raw: &str) -> String {
    match url::Url::parse(raw) {
        Ok(mut u)
            if u.scheme() == "smtp"
                && u.password().is_some()
                && !u.query_pairs().any(|(k, _)| k == "tls") =>
        {
            u.query_pairs_mut().append_pair("tls", "required");
            u.to_string()
        }
        _ => raw.to_string(),
    }
}

/// Pending notifications → outbox rows for users who want that category by email.
pub async fn queue_emails(db: &PgPool, config: &Config) -> anyhow::Result<usize> {
    let mut tx = db.begin().await?;
    // Too old to be news, bell-only kinds, or for users who don't want them: skipped.
    sqlx::query(
        "update notifications n set email_status = 'skipped'
         from users u where u.id = n.user_id and n.email_status = 'pending'
           and (n.created_at < now() - $1 or u.email is null or u.closed_at is not null
                or n.category = any($2)
                or coalesce((u.email_prefs->>n.category)::boolean, true) = false)",
    )
    .bind(EMAIL_WITHIN)
    .bind(bell_only())
    .execute(&mut *tx)
    .await?;
    let rows: Vec<(Uuid, String, String, String, String, String)> = sqlx::query_as(
        "select n.id, u.email, u.display_name, n.title, n.body, n.link from notifications n
         join users u on u.id = n.user_id where n.email_status = 'pending'
           and not (n.category = any($1))
         order by n.created_at limit 200 for update of n skip locked",
    )
    // A bell-only kind written since the skip above is still never emailed.
    .bind(bell_only())
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
            "Hi {name},\n\n{title}\n{}{}\n\nOpen it: {url}\n\n— SLAPorSLOP\n\nYou can choose which emails SLAPorSLOP sends you on your account page: {account}",
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

#[cfg(test)]
mod tests {
    use super::{BELL_ONLY, CATEGORIES, bell_only, known_category};

    #[test]
    fn bell_only_kinds_are_not_email_kinds() {
        for (kind, _) in BELL_ONLY {
            assert!(
                !CATEGORIES.iter().any(|(c, _)| c == kind),
                "{kind} would get an email switch"
            );
        }
        assert_eq!(bell_only(), ["news"]);
        assert_eq!(known_category(Some("news")).unwrap(), Some("news"));
        assert_eq!(known_category(Some("jobs")).unwrap(), Some("jobs"));
        assert!(known_category(Some("spam")).is_err());
    }
}
