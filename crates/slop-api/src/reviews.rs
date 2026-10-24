// SPDX-License-Identifier: AGPL-3.0-or-later
//! Written reviews on apps, and public profiles (DESIGN §7; LEGAL §7; PRIV-6; REV-2).
//!
//! - A review goes with a vote: only players who can vote can write one, and it shows only while
//!   they have a vote on the app. Editing keeps one review per player per app.
//! - Reviews are never hidden for being negative (REV-2), only by a moderation decision
//!   (moderation.rs, kind `app_review`).
//! - The app's maker can reply under a review, once (editing replaces it). The reply shows with
//!   the review, marked when the review was changed after it; the reviewer is told. Staff can
//!   remove a reply (moderation.rs `remove_reply`), and the maker can't post it again.
//! - Profiles show a player's current votes and reviews, **public by default**. In the UK
//!   under-18 view they're private by default (PRIV-6). A private profile's reviews still appear
//!   on app pages, as "A player", with no link.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, put},
};
use serde::{Deserialize, Serialize};
use sqlx::QueryBuilder;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::{CurrentUser, MaybeUser},
    catalog::{self, AppSummary},
    error::{ApiError, ApiResult},
    policy::ViewerPolicy,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/apps/{slug}/reviews", get(list))
        .route("/apps/{slug}/review", put(write).delete(remove))
        .route(
            "/apps/{slug}/reviews/{id}/reply",
            put(write_reply).delete(remove_reply),
        )
        .route("/users/{id}", get(profile))
}

/// SQL: whether user `u`'s profile is public: their choice, else public unless their country's
/// default is the under-18 view (PRIV-6).
pub(crate) const PUBLIC_SQL: &str = "coalesce(u.profile_public,
     not coalesce((select cp.under18_view_default from country_policies cp where cp.country_code = u.country_code), true))";

pub async fn is_public(db: &sqlx::PgPool, user_id: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select {PUBLIC_SQL} from users u where u.id = $1"
    )))
    .bind(user_id)
    .fetch_one(db)
    .await
}

#[derive(Serialize, sqlx::FromRow)]
struct Review {
    id: Uuid,
    /// The author's profile, when it's public.
    author_id: Option<Uuid>,
    /// Their name when the profile is public, else "A player".
    author: String,
    /// not_slop or slop
    vote: String,
    body: String,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    updated_at: OffsetDateTime,
    edited: bool,
    mine: bool,
    /// The app maker's reply, unless staff removed it.
    reply: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    reply_at: Option<OffsetDateTime>,
    /// The review changed after the maker replied.
    reply_outdated: bool,
}

#[derive(Deserialize)]
struct ListQuery {
    /// not_slop or slop
    vote: Option<String>,
    offset: Option<i64>,
}

#[derive(Serialize)]
struct ReviewPage {
    reviews: Vec<Review>,
    not_slop: i64,
    slop: i64,
    next_offset: Option<i64>,
    /// The viewer made the app, so they can reply.
    can_reply: bool,
}

async fn list(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<ReviewPage>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let (app_id, is_mine) = catalog::visible_app(&state, &slug, user.as_ref(), policy).await?;
    let offset = q.offset.unwrap_or(0).clamp(0, 10_000);
    let vote: Option<i16> = match q.vote.as_deref() {
        Some("not_slop") => Some(1),
        Some("slop") => Some(-1),
        None | Some("") => None,
        Some(_) => return Err(ApiError::invalid("vote", "not_slop or slop")),
    };
    let viewer = user.as_ref().map(|u| u.id);
    let mut qb = QueryBuilder::new("select r.id, case when ");
    qb.push(PUBLIC_SQL)
        .push(" then u.id end as author_id, case when ")
        .push(PUBLIC_SQL)
        .push(
            " then u.display_name else 'A player' end as author,
               case when v.vote = 1 then 'not_slop' else 'slop' end as vote, r.body, r.created_at, r.updated_at,
               r.updated_at > r.created_at + interval '1 minute' as edited, coalesce(r.user_id = ",
        )
        .push_bind(viewer)
        .push(
            ", false) as mine,
               case when r.reply_hidden_at is null then r.reply end as reply,
               case when r.reply_hidden_at is null then r.reply_at end as reply_at,
               coalesce(r.reply_hidden_at is null and r.updated_at > r.reply_at, false) as reply_outdated
             from app_reviews r join votes v on v.user_id = r.user_id and v.app_id = r.app_id
             join users u on u.id = r.user_id
             where r.hidden_at is null and u.closed_at is null and u.suspended_at is null and r.app_id = ",
        )
        .push_bind(app_id);
    if let Some(v) = vote {
        qb.push(" and v.vote = ").push_bind(v);
    }
    qb.push(" order by (r.user_id = ")
        .push_bind(viewer)
        .push(") is true desc, r.updated_at desc limit 21 offset ")
        .push_bind(offset);
    let mut reviews: Vec<Review> = qb.build_query_as().fetch_all(&state.db).await?;
    let next_offset = (reviews.len() > 20).then_some(offset + 20);
    reviews.truncate(20);
    let (not_slop, slop): (i64, i64) = sqlx::query_as(
        "select count(*) filter (where v.vote = 1), count(*) filter (where v.vote = -1)
         from app_reviews r join votes v on v.user_id = r.user_id and v.app_id = r.app_id
         join users u on u.id = r.user_id
         where r.app_id = $1 and r.hidden_at is null and u.closed_at is null and u.suspended_at is null",
    )
    .bind(app_id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(ReviewPage {
        reviews,
        not_slop,
        slop,
        next_offset,
        can_reply: is_mine,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteBody {
    body: String,
}

async fn write(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(b): Json<WriteBody>,
) -> ApiResult<StatusCode> {
    crate::limits::check(&state, user.id, crate::limits::Action::Review)?;
    let policy = ViewerPolicy::resolve(&state.db, &state.config, Some(&user), &headers).await?;
    let (app_id, _) = catalog::visible_app(&state, &slug, Some(&user), policy).await?;
    if catalog::is_private(&state.db, app_id).await? {
        return Err(ApiError::Conflict("Private Slop has no reviews".into()));
    }
    let body = b.body.trim();
    if body.is_empty() || body.chars().count() > 4000 {
        return Err(ApiError::invalid("body", "1 to 4,000 characters"));
    }
    let voted: bool = sqlx::query_scalar(
        "select exists (select 1 from votes where user_id = $1 and app_id = $2)",
    )
    .bind(user.id)
    .bind(app_id)
    .fetch_one(&state.db)
    .await?;
    if !voted {
        return Err(ApiError::Conflict(
            "vote 👍 or 👎 first: a review goes with your vote".into(),
        ));
    }
    // A review hidden by a moderation decision stays hidden; editing doesn't bring it back.
    sqlx::query(
        "insert into app_reviews (id, user_id, app_id, body) values ($1, $2, $3, $4)
         on conflict (user_id, app_id) do update set body = $4, updated_at = now()",
    )
    .bind(Uuid::now_v7())
    .bind(user.id)
    .bind(app_id)
    .bind(body)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    sqlx::query("delete from app_reviews r using apps a where a.id = r.app_id and a.slug = $2 and r.user_id = $1")
        .bind(user.id)
        .bind(&slug)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The viewer's own review of an app, for the rating panel.
pub async fn mine(
    db: &sqlx::PgPool,
    user_id: Uuid,
    app_id: Uuid,
) -> sqlx::Result<Option<MyReview>> {
    sqlx::query_as("select body, hidden_at is not null as hidden from app_reviews where user_id = $1 and app_id = $2")
        .bind(user_id)
        .bind(app_id)
        .fetch_optional(db)
        .await
}

#[derive(Serialize, sqlx::FromRow)]
pub struct MyReview {
    pub body: String,
    /// Hidden by a moderation decision (see the account page).
    pub hidden: bool,
}

#[derive(Serialize)]
struct Profile {
    id: Uuid,
    name: String,
    #[serde(with = "time::serde::rfc3339")]
    joined: OffsetDateTime,
    /// Votes and reviews are shown (the user's choice).
    public: bool,
    is_me: bool,
    /// Their People page, when they have a live one.
    creative_handle: Option<String>,
    /// Their profile picture (avatars.rs); null: the letter.
    avatar_url: Option<String>,
    /// Apps they make, in the store.
    apps: Vec<AppSummary>,
    votes: Vec<ProfileVote>,
}

#[derive(Serialize, sqlx::FromRow)]
struct ProfileVote {
    #[sqlx(flatten)]
    #[serde(flatten)]
    app: AppSummary,
    /// not_slop or slop
    my_vote: String,
    review: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    voted_at: OffsetDateTime,
}

async fn profile(
    State(state): State<AppState>,
    MaybeUser(viewer): MaybeUser,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Profile>> {
    let row: Option<(String, OffsetDateTime, bool)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select u.display_name, u.created_at, {PUBLIC_SQL} from users u
         where u.id = $1 and u.closed_at is null and u.suspended_at is null"
    )))
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let (name, joined, public) = row.ok_or(ApiError::NotFound)?;
    let is_me = viewer.as_ref().is_some_and(|v| v.id == id);
    let policy = ViewerPolicy::resolve(&state.db, &state.config, viewer.as_ref(), &headers).await?;
    let creative_handle: Option<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select c.handle from creatives c where c.user_id = $1 and {}",
        crate::people::OPEN_SQL
    )))
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let mut qb = QueryBuilder::new("");
    catalog::push_summary_columns(&mut qb, &state.config);
    qb.push(" where a.maker_id = ").push_bind(id).push(" and ");
    catalog::push_store_visible(&mut qb, state.config.catalog, policy);
    qb.push(" order by a.updated_at desc limit 24");
    let apps: Vec<AppSummary> = qb.build_query_as().fetch_all(&state.db).await?;
    let votes = if public || is_me {
        let mut qb = QueryBuilder::new("");
        catalog::push_summary_columns_with(
            &mut qb,
            &state.config,
            ", case when v.vote = 1 then 'not_slop' else 'slop' end as my_vote,
               (select r.body from app_reviews r where r.user_id = v.user_id and r.app_id = v.app_id and r.hidden_at is null) as review,
               v.updated_at as voted_at",
        );
        qb.push(" join votes v on v.app_id = a.id where v.user_id = ")
            .push_bind(id)
            .push(" and ");
        catalog::push_store_visible(&mut qb, state.config.catalog, policy);
        qb.push(" order by v.updated_at desc limit 100");
        qb.build_query_as().fetch_all(&state.db).await?
    } else {
        Vec::new()
    };
    Ok(Json(Profile {
        id,
        name,
        joined,
        public,
        is_me,
        creative_handle,
        avatar_url: crate::avatars::url_of(&state, id).await?,
        apps,
        votes,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyBody {
    body: String,
}

/// The app's maker replies under a review (or changes their reply). The reviewer is told.
async fn write_reply(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
    Json(b): Json<ReplyBody>,
) -> ApiResult<StatusCode> {
    crate::limits::check(&state, user.id, crate::limits::Action::Review)?;
    let (app_id, _) = crate::updates::owned(&state, &slug, &user).await?;
    let body = b.body.trim();
    if body.is_empty() || body.chars().count() > 2000 {
        return Err(ApiError::invalid("body", "1 to 2,000 characters"));
    }
    let mut tx = state.db.begin().await?;
    let row: Option<(Uuid, Option<OffsetDateTime>, String)> = sqlx::query_as(
        "select r.user_id, r.reply_hidden_at, a.title from app_reviews r join apps a on a.id = r.app_id
         where r.id = $1 and r.app_id = $2 and r.hidden_at is null for update of r",
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (reviewer, removed, title) = row.ok_or(ApiError::NotFound)?;
    if removed.is_some() {
        return Err(ApiError::Conflict(
            "your reply was removed by a moderation decision; you can appeal it from your account"
                .into(),
        ));
    }
    sqlx::query("update app_reviews set reply = $2, reply_at = now() where id = $1")
        .bind(id)
        .bind(body)
        .execute(&mut *tx)
        .await?;
    if reviewer != user.id {
        crate::notify::send(
            &mut tx,
            crate::notify::Notice {
                user_id: reviewer,
                category: "apps",
                title: format!("{title}'s maker replied to your review"),
                body: body.chars().take(200).collect(),
                link: format!("/app/{slug}#reviews"),
                collapse_key: Some(format!("review-reply:{id}")),
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_reply(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let (app_id, _) = crate::updates::owned(&state, &slug, &user).await?;
    let done = sqlx::query(
        "update app_reviews set reply = null, reply_at = null
         where id = $1 and app_id = $2 and reply is not null and reply_hidden_at is null",
    )
    .bind(id)
    .bind(app_id)
    .execute(&state.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}
