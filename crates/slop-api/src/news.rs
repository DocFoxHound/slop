// SPDX-License-Identifier: AGPL-3.0-or-later
//! Maker news (DESIGN §8; owner decision 2026-11-10): the maker of an app whose own page is in the
//! store posts short news on that page. A post is a headline (one line, up to 100 characters,
//! which is also what the bell shows) and up to 2,000 characters of simple Markdown, which the web
//! shows only through the chat renderer (no pictures, no raw HTML).
//!
//! - One new post per app per 24 hours, also after a post is removed or hidden
//!   (`apps.news_posted_at`), and a per-account limit on posting and editing (limits.rs `News`).
//!   Editing is free and tells nobody again.
//! - People who marked the app Interested, and didn't switch news off for it, are told under the
//!   bell only: never by email or on a device (notify.rs `news_posted`, `BELL_ONLY`).
//! - Staff can hide a post (moderation.rs `hide_news`). It leaves the page and the bell, stays in
//!   the maker's list for the appeal, and can't be edited, removed or posted again. Hidden posts
//!   are deleted 213 days after they were hidden, unless an appeal about one is still waiting
//!   (`cleanup`).
//! - Reading is public: anyone who may see the app's store page reads its posts. A Private app
//!   has no store page, so only its maker reads its old posts.
//! - Locks: every change takes the post's row before its notices, and its notices in person
//!   order (notify.rs), so a maker's change and a staff decision never wait on each other in a
//!   circle. Posting holds the app's row while it checks the store, so a delisting under way is
//!   seen.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, put},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::QueryBuilder;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::{CurrentUser, MaybeUser},
    catalog,
    config::Config,
    error::{ApiError, ApiResult},
    limits::{self, Action},
    notify,
    policy::ViewerPolicy,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/apps/{slug}/news", get(list).post(post))
        .route("/apps/{slug}/news/{id}", put(edit).delete(remove))
}

const NOT_IN_STORE: &str =
    "news can be posted or changed only while the app's page is in the store";
const HIDDEN: &str =
    "this post was removed by a moderation decision; you can appeal it from your account";
/// The same, once the decision can't be appealed any more (appealed already, or six months on).
const HIDDEN_DECIDED: &str =
    "this post was removed by a moderation decision; you can see the decision on your account";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewsBody {
    headline: String,
    body: String,
}

#[derive(Serialize, sqlx::FromRow)]
struct Post {
    id: Uuid,
    headline: String,
    body: String,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    /// Changed more than a minute after it was posted.
    edited: bool,
    /// Removed by a moderation decision: only ever true for the app's maker.
    hidden: bool,
    /// The rule it was hidden under, with `hidden` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    hidden_reason: Option<String>,
    /// With `hidden` only: whether a decision that hides it can still be appealed.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[sqlx(default)]
    can_appeal: Option<bool>,
}

/// An `app_news` row as a `Post`.
const POST_COLUMNS: &str = "id, headline, body, created_at,
    updated_at > created_at + interval '1 minute' as edited, hidden_at is not null as hidden,
    case when hidden_at is not null then hidden_reason end as hidden_reason";

/// Whether the app's own page is in the store (listed, public, inside its stage deadline, not
/// frozen, not a dedicated server), and when its maker may post news next while that's still to
/// come.
async fn page_state(
    conn: &mut sqlx::PgConnection,
    config: &Config,
    app_id: Uuid,
) -> sqlx::Result<(bool, Option<OffsetDateTime>)> {
    // The full view, so an 18+ app can post too.
    let mut qb = QueryBuilder::new("select (");
    catalog::push_store_visible(&mut qb, config.catalog, ViewerPolicy::FULL_VIEW);
    qb.push(
        ") and a.frozen_at is null and a.branch_kind is distinct from 'server',
         case when a.news_posted_at > now() - interval '24 hours'
              then a.news_posted_at + interval '24 hours' end
         from apps a where a.id = ",
    )
    .push_bind(app_id);
    qb.build_query_as().fetch_one(conn).await
}

/// The headline and text, trimmed, or what to fix.
fn checked(b: &NewsBody) -> ApiResult<(&str, &str)> {
    let headline = b.headline.trim();
    let n = headline.chars().count();
    if n == 0 || n > 100 || headline.contains(['\n', '\r']) {
        return Err(ApiError::invalid(
            "headline",
            "1 to 100 characters, on one line",
        ));
    }
    // Plain text: it's also what the bell shows. No tabs or other control characters, and none
    // of the marks that turn the text's direction around (they can make it read differently).
    let hidden =
        |c: char| c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}');
    if headline.chars().any(hidden) {
        return Err(ApiError::invalid(
            "headline",
            "one line without tabs or hidden characters",
        ));
    }
    let body = b.body.trim();
    let n = body.chars().count();
    if n == 0 || n > 2000 {
        return Err(ApiError::invalid("body", "1 to 2,000 characters"));
    }
    // Line breaks and tabs are fine; a NUL would reach the database.
    if body
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(ApiError::invalid(
            "body",
            "without hidden control characters",
        ));
    }
    Ok((headline, body))
}

/// The post of the app that staff hid with this exact text, if any: that text can't come back as
/// a new post or through an edit. (This stops accidents, not intent: the report and the appeal
/// are for that.)
async fn same_as_hidden(
    conn: &mut sqlx::PgConnection,
    app_id: Uuid,
    headline: &str,
    body: &str,
) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar(
        "select id from app_news
         where app_id = $1 and hidden_at is not null and headline = $2 and body = $3
         order by hidden_at desc limit 1",
    )
    .bind(app_id)
    .bind(headline)
    .bind(body)
    .fetch_optional(conn)
    .await
}

/// Whether a standing decision that hides this post can still be appealed by its maker (once,
/// within six months: moderation.rs `appeal`).
async fn can_appeal(conn: &mut sqlx::PgConnection, post_id: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "select exists (select 1 from moderation_decisions
                        where subject_kind = 'news' and subject_id = $1 and action = 'hide_news'
                          and reversed_at is null and appeal_status is null
                          and created_at > now() - $2)",
    )
    .bind(post_id)
    .bind(crate::moderation::APPEAL_WINDOW)
    .fetch_one(conn)
    .await
}

/// The refusal for a post staff hid: it offers the appeal only while one can be made.
async fn hidden_refusal(conn: &mut sqlx::PgConnection, post_id: Uuid) -> ApiResult<ApiError> {
    let message = if can_appeal(conn, post_id).await? {
        HIDDEN
    } else {
        HIDDEN_DECIDED
    };
    Ok(ApiError::Conflict(message.into()))
}

#[derive(Deserialize)]
struct ListQuery {
    /// The ID of the last post already shown.
    before: Option<String>,
    /// How many, 1 to 50 (10 when left out).
    limit: Option<i64>,
}

#[derive(Serialize)]
struct Posting {
    allowed: bool,
    /// not_in_store or wait
    why_not: Option<&'static str>,
    #[serde(with = "time::serde::rfc3339::option")]
    next_post_at: Option<OffsetDateTime>,
}

#[derive(Serialize)]
struct NewsPage {
    news: Vec<Post>,
    /// Whether there are older ones (ask again with `before` = the last one's ID).
    more: bool,
    /// For the app's maker only: whether a new post is allowed now, and if not, why.
    #[serde(skip_serializing_if = "Option::is_none")]
    posting: Option<Posting>,
}

/// The app's news, newest first, for anyone who may see its page. Its maker also gets the hidden
/// posts and whether they may post now.
async fn list(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<NewsPage>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let (app_id, is_mine) = catalog::visible_app(&state, &slug, user.as_ref(), policy).await?;
    // A Private app has no store page and no news: the people it's shared with see none, as on
    // the web (news written while it was public stays for when it's public again).
    if !is_mine && catalog::is_private(&state.db, app_id).await? {
        return Ok(Json(NewsPage {
            news: Vec::new(),
            more: false,
            posting: None,
        }));
    }
    let before = match q.before.as_deref() {
        None | Some("") => None,
        Some(id) => {
            Some(Uuid::parse_str(id).map_err(|_| ApiError::invalid("before", "a news post's ID"))?)
        }
    };
    let limit = q.limit.unwrap_or(10).clamp(1, 50);
    // A page continues from the post it names, which must be one the caller can see.
    let mut news: Vec<Post> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {POST_COLUMNS} from app_news n
         where n.app_id = $1 and ($2 or n.hidden_at is null)
           and ($3::uuid is null
                or (n.created_at, n.id) < (select b.created_at, b.id from app_news b
                                           where b.id = $3 and b.app_id = $1
                                             and ($2 or b.hidden_at is null)))
         order by n.created_at desc, n.id desc limit $4"
    )))
    .bind(app_id)
    .bind(is_mine)
    .bind(before)
    .bind(limit + 1)
    .fetch_all(&state.db)
    .await?;
    let more = news.len() as i64 > limit;
    news.truncate(limit as usize);
    let posting = if is_mine {
        let mut conn = state.db.acquire().await?;
        // Only the maker gets hidden posts; there are few of them.
        for p in news.iter_mut().filter(|p| p.hidden) {
            p.can_appeal = Some(can_appeal(&mut conn, p.id).await?);
        }
        Some(match page_state(&mut conn, &state.config, app_id).await? {
            (false, _) => Posting {
                allowed: false,
                why_not: Some("not_in_store"),
                next_post_at: None,
            },
            (true, Some(next)) => Posting {
                allowed: false,
                why_not: Some("wait"),
                next_post_at: Some(next),
            },
            (true, None) => Posting {
                allowed: true,
                why_not: None,
                next_post_at: None,
            },
        })
    } else {
        None
    };
    Ok(Json(NewsPage {
        news,
        more,
        posting,
    }))
}

/// The maker posts news. Everyone Interested with news on is told under the bell.
async fn post(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<NewsBody>,
) -> ApiResult<(StatusCode, Json<Post>)> {
    limits::check(&state, user.id, Action::News)?;
    let (app_id, _) = crate::updates::owned(&state, &slug, &user).await?;
    let (headline, body) = checked(&b)?;
    let mut tx = state.db.begin().await?;
    // The app's row first: a delisting under way (which takes its news out of the bells) finishes
    // before the store is checked, and two posts at once are one after the other.
    sqlx::query("select 1 from apps where id = $1 for no key update")
        .bind(app_id)
        .execute(&mut *tx)
        .await?;
    if !page_state(&mut tx, &state.config, app_id).await?.0 {
        return Err(ApiError::Conflict(NOT_IN_STORE.into()));
    }
    if let Some(hidden) = same_as_hidden(&mut tx, app_id, headline, body).await? {
        return Err(hidden_refusal(&mut tx, hidden).await?);
    }
    // The day's slot, on the row this transaction holds.
    let title: Option<String> = sqlx::query_scalar(
        "update apps set news_posted_at = now()
         where id = $1 and (news_posted_at is null or news_posted_at <= now() - interval '24 hours')
         returning title",
    )
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(title) = title else {
        let (next, now): (OffsetDateTime, OffsetDateTime) = sqlx::query_as(
            "select news_posted_at + interval '24 hours', now() from apps where id = $1",
        )
        .bind(app_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ApiError::NotFound)?;
        return Err(not_yet(
            now,
            next,
            "This app posted news in the last 24 hours.",
        ));
    };
    let id = Uuid::now_v7();
    let post: Post = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "insert into app_news (id, app_id, headline, body) values ($1, $2, $3, $4)
         returning {POST_COLUMNS}"
    )))
    .bind(id)
    .bind(app_id)
    .bind(headline)
    .bind(body)
    .fetch_one(&mut *tx)
    .await?;
    let told = notify::news_posted(&mut tx, app_id, id, &title, &slug, headline).await?;
    // No headline here: the post is the subject, and a removed post leaves no text behind.
    crate::admin::audit(
        &mut tx,
        &user.id.to_string(),
        "news.post",
        &id.to_string(),
        json!({ "app": slug, "told": told, "by_agent": user.agent.is_some() }),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(post)))
}

/// The maker changes a post (both fields). Nobody is told again; the bell's notices of this post
/// show the new headline.
async fn edit(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
    Json(b): Json<NewsBody>,
) -> ApiResult<Json<Post>> {
    limits::check(&state, user.id, Action::News)?;
    let (app_id, _) = crate::updates::owned(&state, &slug, &user).await?;
    let (headline, body) = checked(&b)?;
    let mut tx = state.db.begin().await?;
    let row: Option<(String, bool)> = sqlx::query_as(
        "select headline, hidden_at is not null from app_news where id = $1 and app_id = $2 for update",
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (was, hidden) = row.ok_or(ApiError::NotFound)?;
    if hidden {
        return Err(hidden_refusal(&mut tx, id).await?);
    }
    if !page_state(&mut tx, &state.config, app_id).await?.0 {
        return Err(ApiError::Conflict(NOT_IN_STORE.into()));
    }
    // Nor can an edit bring back the text of a post staff hid.
    if let Some(hidden) = same_as_hidden(&mut tx, app_id, headline, body).await? {
        return Err(hidden_refusal(&mut tx, hidden).await?);
    }
    let post: Post = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "update app_news set headline = $2, body = $3,
                updated_at = case when headline = $2 and body = $3 then updated_at else now() end
         where id = $1 returning {POST_COLUMNS}"
    )))
    .bind(id)
    .bind(headline)
    .bind(body)
    .fetch_one(&mut *tx)
    .await?;
    if was != headline {
        notify::news_reworded(&mut tx, id, headline).await?;
    }
    crate::admin::audit(
        &mut tx,
        &user.id.to_string(),
        "news.edit",
        &id.to_string(),
        json!({ "app": slug, "by_agent": user.agent.is_some() }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(post))
}

/// The maker removes a post, whatever the app's state now; its notices go too. It doesn't give
/// the day's post back. A post staff hid stays for the appeal.
async fn remove(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let (app_id, _) = crate::updates::owned(&state, &slug, &user).await?;
    let mut tx = state.db.begin().await?;
    let hidden: Option<bool> = sqlx::query_scalar(
        "select hidden_at is not null from app_news where id = $1 and app_id = $2 for update",
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;
    match hidden {
        None => return Err(ApiError::NotFound),
        Some(true) => return Err(hidden_refusal(&mut tx, id).await?),
        Some(false) => {}
    }
    notify::news_withdrawn(&mut tx, id).await?;
    sqlx::query("delete from app_news where id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    crate::admin::audit(
        &mut tx,
        &user.id.to_string(),
        "news.remove",
        &id.to_string(),
        json!({ "app": slug, "by_agent": user.agent.is_some() }),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Hourly (cleanup.rs). Notices about the news of an app whose page left the store go: a staff
/// delisting and the end of a frozen app's grace take them at once, this catches every other way
/// out (a stage deadline passed, the maker made it private or took it out). So do notices about an
/// app the person may no longer see (its age rating went up, or their account's country changed;
/// the same rule as when they were told, policy.rs). And posts staff hid go 213 days after they
/// were hidden, long after their appeal window, unless an appeal about one is still waiting.
/// Returns (notices, posts).
pub async fn cleanup(state: &AppState) -> sqlx::Result<(u64, u64)> {
    // Notices are taken in person order, as everywhere else (notify.rs).
    let mut qb = QueryBuilder::new(
        "delete from notifications where id in (
             select n.id from notifications n join app_news p on p.id = n.news_id
             where not exists (select 1 from apps a where a.id = p.app_id and ",
    );
    catalog::push_store_visible_as(&mut qb, state.config.catalog, ViewerPolicy::FULL_VIEW, "a");
    qb.push(") order by n.user_id, n.id for update of n)");
    let mut notices = qb.build().execute(&state.db).await?.rows_affected();
    let out_of_view = format!(
        "delete from notifications where id in (
             select n.id from notifications n
             join app_news p on p.id = n.news_id
             join apps a on a.id = p.app_id
             join users u on u.id = n.user_id
             where a.min_age > {}
             order by n.user_id, n.id for update of n)",
        crate::policy::account_max_min_age_sql("u")
    );
    notices += sqlx::query(sqlx::AssertSqlSafe(out_of_view))
        .execute(&state.db)
        .await?
        .rows_affected();
    let posts = sqlx::query(
        "delete from app_news p where p.hidden_at < now() - interval '213 days'
           and not exists (select 1 from moderation_decisions d
                           where d.subject_kind = 'news' and d.subject_id = p.id
                             and d.appeal_status = 'pending')",
    )
    .execute(&state.db)
    .await?
    .rows_affected();
    Ok((notices, posts))
}

// ---------------------------------------------------------------------------------------------
// The 24 hours, in words (as admin_messages.rs says its limits)

/// "47 minutes", "3 hours 5 minutes".
fn in_words(seconds: u64) -> String {
    let minutes = seconds.div_ceil(60).max(1);
    let (h, m) = (minutes / 60, minutes % 60);
    let unit = |n: u64, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    match (h, m) {
        (0, m) => unit(m, "minute", "minutes"),
        (h, 0) => unit(h, "hour", "hours"),
        (h, m) => format!(
            "{} {}",
            unit(h, "hour", "hours"),
            unit(m, "minute", "minutes")
        ),
    }
}

fn stamp(t: OffsetDateTime) -> String {
    t.replace_nanosecond(0)
        .unwrap_or(t)
        .format(&Rfc3339)
        .unwrap_or_default()
}

/// A 429 that says when posting is allowed again, and why.
fn not_yet(now: OffsetDateTime, next: OffsetDateTime, why: &str) -> ApiError {
    let seconds = u64::try_from((next - now).whole_seconds().max(1)).unwrap_or(1);
    ApiError::NotYet {
        message: format!(
            "{why} The next post is allowed at {} (in {}).",
            stamp(next),
            in_words(seconds)
        ),
        retry_after: seconds,
    }
}

#[cfg(test)]
mod tests {
    use time::{Duration, macros::datetime};

    use super::{ApiError, in_words, not_yet};

    #[test]
    fn the_wait_is_said_in_words() {
        assert_eq!(in_words(60), "1 minute");
        assert_eq!(in_words(3600), "1 hour");
        assert_eq!(in_words(23 * 3600 + 12 * 60), "23 hours 12 minutes");
        let now = datetime!(2026-11-10 16:56:00.4 UTC);
        let next = datetime!(2026-11-11 16:08:00.4 UTC);
        assert_eq!(next - now, Duration::hours(23) + Duration::minutes(12));
        let ApiError::NotYet {
            message,
            retry_after,
        } = not_yet(now, next, "This app posted news in the last 24 hours.")
        else {
            panic!("not a 429 with a time");
        };
        assert_eq!(
            message,
            "This app posted news in the last 24 hours. The next post is allowed at \
             2026-11-11T16:08:00Z (in 23 hours 12 minutes)."
        );
        assert_eq!(retry_after, 23 * 3600 + 12 * 60);
        let ApiError::NotYet { message, .. } = not_yet(now, now + Duration::seconds(20), "Wait.")
        else {
            panic!("not a 429 with a time");
        };
        assert!(message.ends_with("(in 1 minute)."), "{message}");
    }
}
