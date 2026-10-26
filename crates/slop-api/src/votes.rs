// SPDX-License-Identifier: AGPL-3.0-or-later
//! Ratings: 👍 Not Slop / 👎 Slop (DESIGN §7, METRICS §1.2, §2).
//!
//! - Only players who launched the app through Slop can vote (that's `user_app_usage`), and only
//!   once it's playable: in Beta or released (owner decision 2026-10-25; Coming soon has nothing
//!   to play). One vote per account, changeable or withdrawable at any time.
//! - Each vote records the app's stage when it was cast, so a rating given in Beta says so.
//! - Every action is a `vote_events` row; `votes` holds the current vote, updated in the same
//!   transaction.
//! - Each vote is **weighted** by the voter's use at the time: playtime from Slop Pulse when the
//!   app reports it, otherwise the days they launched it. Store pages show the weighted % Not Slop
//!   with the unweighted one beside it, and both over time.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::{CurrentUser, MaybeUser},
    catalog,
    error::{ApiError, ApiResult},
    policy::ViewerPolicy,
};

/// Below this many votes, no percentage or label is shown: a handful of votes says little.
pub const MIN_VOTES: i64 = 5;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/apps/{slug}/ratings", get(ratings))
        .route("/apps/{slug}/vote", post(vote))
        .route("/apps/{slug}/ratings/breakdown", get(breakdown))
}

/// How much a vote counts (METRICS §2), from 0.1 to 1.
///
/// - With Pulse playtime: `log10(1 + minutes) / log10(601)`, so 10 hours or more counts fully
///   and anything under 2 minutes counts 0.1.
/// - Without (tools, games that don't ping): days launched, `log10(1 + days) / log10(11)`, so
///   10 days or more counts fully.
pub fn vote_weight(playtime_seconds: i64, days_launched: i32) -> f32 {
    let w = if playtime_seconds > 0 {
        let minutes = playtime_seconds as f64 / 60.0;
        if minutes < 2.0 {
            0.1
        } else {
            (1.0 + minutes).log10() / 601f64.log10()
        }
    } else {
        (1.0 + f64::from(days_launched.max(0))).log10() / 11f64.log10()
    };
    w.clamp(0.1, 1.0) as f32
}

/// The words for a % Not Slop (DESIGN §7).
pub fn label(percent: i32) -> &'static str {
    match percent {
        95.. => "Overwhelmingly Not Slop",
        80..=94 => "Very Not Slop",
        70..=79 => "Mostly Not Slop",
        40..=69 => "Mixed",
        20..=39 => "Mostly Slop",
        _ => "Overwhelmingly Slop",
    }
}

fn percent(up: f64, down: f64) -> Option<i32> {
    let total = up + down;
    (total > 0.0).then(|| (100.0 * up / total).round() as i32)
}

/// Totals at one point in time.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Tally {
    pub not_slop: i64,
    pub slop: i64,
    pub not_slop_weight: f64,
    pub slop_weight: f64,
    /// Unweighted % Not Slop; null below MIN_VOTES.
    pub percent: Option<i32>,
    /// Weighted by playtime (the one shown first); null below MIN_VOTES.
    pub percent_weighted: Option<i32>,
    pub label: Option<&'static str>,
}

impl Tally {
    pub fn new(not_slop: i64, slop: i64, not_slop_weight: f64, slop_weight: f64) -> Self {
        let enough = not_slop + slop >= MIN_VOTES;
        let percent_weighted = enough
            .then(|| percent(not_slop_weight, slop_weight))
            .flatten();
        Self {
            not_slop,
            slop,
            not_slop_weight,
            slop_weight,
            percent: enough
                .then(|| percent(not_slop as f64, slop as f64))
                .flatten(),
            percent_weighted,
            label: percent_weighted.map(label),
        }
    }
}

#[derive(Serialize)]
pub struct Month {
    /// YYYY-MM
    pub month: String,
    #[serde(flatten)]
    pub tally: Tally,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Day {
    /// YYYY-MM-DD
    pub day: String,
    /// New 👍 votes that day (including changes to 👍).
    pub not_slop: i64,
    /// New 👎 votes that day (including changes to 👎).
    pub slop: i64,
    /// Of those, how many changed an earlier vote.
    pub changed: i64,
    pub withdrawn: i64,
}

#[derive(Serialize)]
pub struct MyVote {
    /// not_slop, slop, or null.
    pub vote: Option<&'static str>,
    pub can_vote: bool,
    /// Why not, when they can't.
    pub reason: Option<&'static str>,
    /// They made it: no vote buttons for them anywhere (makers can't vote on their own apps).
    pub maker: bool,
    /// They launched it through Slop (what voting needs, besides the stage).
    pub played: bool,
    /// How much their vote counts now.
    pub weight: f32,
    /// Their written review, if any (reviews.rs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review: Option<crate::reviews::MyReview>,
}

#[derive(Serialize)]
pub struct Ratings {
    #[serde(flatten)]
    pub now: Tally,
    /// The rating at the end of each month (each account's last vote by then), oldest first.
    pub monthly: Vec<Month>,
    /// Votes cast per day, last 90 days.
    pub daily: Vec<Day>,
    /// Days in those 90 with unusual voting the abuse checks found (abuse.rs: review bombs,
    /// bursts from new accounts, swings), unless staff found nothing wrong. Shaded on the chart.
    pub unusual_days: Vec<String>,
    /// Voting is open: the app is in Beta or released.
    pub open: bool,
    /// The app's stage now: coming_soon, beta or released.
    pub stage: &'static str,
    /// Of the current votes, how many were cast while the app was in Beta.
    pub in_beta: i64,
    pub min_votes: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mine: Option<MyVote>,
    /// For apps that hired creatives through People: the rating from votes since the first
    /// release with their work (DESIGN §15.4). It never changes the rating above (HUM-13).
    pub since_new_work: Option<crate::people::SinceNewWork>,
}

/// The current rating.
pub async fn tally(db: &sqlx::PgPool, app_id: Uuid) -> sqlx::Result<Tally> {
    let (up, down, upw, downw): (i64, i64, f64, f64) = sqlx::query_as(
        "select count(*) filter (where vote = 1), count(*) filter (where vote = -1),
                coalesce(sum(weight) filter (where vote = 1), 0)::float8,
                coalesce(sum(weight) filter (where vote = -1), 0)::float8
         from votes where app_id = $1",
    )
    .bind(app_id)
    .fetch_one(db)
    .await?;
    Ok(Tally::new(up, down, upw, downw))
}

/// The rating at the end of each month since the first vote (up to 24 months), from the events:
/// each account's last vote before the month ended.
pub async fn monthly(db: &sqlx::PgPool, app_id: Uuid) -> sqlx::Result<Vec<Month>> {
    let rows: Vec<(String, i64, i64, f64, f64)> = sqlx::query_as(
        "with months as (
             select generate_series(
                 greatest(date_trunc('month', min(created_at)), date_trunc('month', now()) - interval '23 months'),
                 date_trunc('month', now()), interval '1 month') as m
             from vote_events where app_id = $1
         ), latest as (
             select distinct on (months.m, e.user_id) months.m, e.vote, e.weight
             from months join vote_events e on e.app_id = $1 and e.created_at < months.m + interval '1 month'
             order by months.m, e.user_id, e.created_at desc, e.id desc
         )
         select to_char(months.m, 'YYYY-MM'),
                count(*) filter (where l.vote = 1), count(*) filter (where l.vote = -1),
                coalesce(sum(l.weight) filter (where l.vote = 1), 0)::float8,
                coalesce(sum(l.weight) filter (where l.vote = -1), 0)::float8
         from months left join latest l on l.m = months.m
         group by months.m order by months.m",
    )
    .bind(app_id)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(month, up, down, upw, downw)| Month {
            month,
            tally: Tally::new(up, down, upw, downw),
        })
        .collect())
}

/// Days with unusual voting (see `Ratings::unusual_days`).
pub async fn unusual_days(db: &sqlx::PgPool, app_id: Uuid) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar(
        "select distinct to_char((detected_at at time zone 'UTC')::date, 'YYYY-MM-DD')
         from abuse_signals
         where subject_kind = 'app' and subject_id = $1
           and kind in ('review_bomb', 'vote_burst', 'vote_swing') and status <> 'dismissed'
           and detected_at > now() - interval '90 days'
         order by 1",
    )
    .bind(app_id)
    .fetch_all(db)
    .await
}

pub async fn daily(db: &sqlx::PgPool, app_id: Uuid) -> sqlx::Result<Vec<Day>> {
    sqlx::query_as(
        "select to_char((created_at at time zone 'UTC')::date, 'YYYY-MM-DD') as day,
                count(*) filter (where vote = 1) as not_slop,
                count(*) filter (where vote = -1) as slop,
                count(*) filter (where vote <> 0 and prev_vote is not null and prev_vote <> vote) as changed,
                count(*) filter (where vote = 0) as withdrawn
         from vote_events where app_id = $1 and created_at > now() - interval '90 days'
         group by 1 order by 1",
    )
    .bind(app_id)
    .fetch_all(db)
    .await
}

/// One group of current votes in the maker's breakdown.
#[derive(Serialize)]
pub struct Group {
    /// A version, a platform (windows, linux) or a stage (beta, released); null when it wasn't
    /// recorded.
    pub key: Option<String>,
    #[serde(flatten)]
    pub tally: Tally,
}

/// Changes of vote made while players had one version ("after 1.4, 212 players changed 👎 → 👍").
#[derive(Serialize, sqlx::FromRow)]
pub struct Impact {
    pub version: String,
    pub to_not_slop: i64,
    pub to_slop: i64,
    /// New votes (no earlier vote) cast on that version.
    pub new_votes: i64,
}

/// The maker's view of their ratings (METRICS §3, §5): current votes by the version and platform
/// each was cast on, and how each version changed people's minds.
#[derive(Serialize)]
pub struct Breakdown {
    pub by_version: Vec<Group>,
    pub by_platform: Vec<Group>,
    /// By the app's stage when each was cast: beta or released.
    pub by_stage: Vec<Group>,
    pub impact: Vec<Impact>,
}

/// Groups each account's current vote by a column of the event that set it.
async fn grouped(
    db: &sqlx::PgPool,
    app_id: Uuid,
    column: &'static str,
) -> sqlx::Result<Vec<Group>> {
    let rows: Vec<(Option<String>, i64, i64, f64, f64)> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "with current as (
             select distinct on (e.user_id) e.{column}::text as key, e.vote, e.weight
             from vote_events e where e.app_id = $1
             order by e.user_id, e.created_at desc, e.id desc
         )
         select key, count(*) filter (where vote = 1), count(*) filter (where vote = -1),
                coalesce(sum(weight) filter (where vote = 1), 0)::float8,
                coalesce(sum(weight) filter (where vote = -1), 0)::float8
         from current where vote <> 0
         group by key order by count(*) desc limit 20"
        )))
        .bind(app_id)
        .fetch_all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(key, up, down, upw, downw)| Group {
            key,
            tally: Tally::new(up, down, upw, downw),
        })
        .collect())
}

pub async fn breakdown_for(db: &sqlx::PgPool, app_id: Uuid) -> sqlx::Result<Breakdown> {
    Ok(Breakdown {
        by_version: grouped(db, app_id, "release_version").await?,
        by_platform: grouped(db, app_id, "platform").await?,
        by_stage: grouped(db, app_id, "stage").await?,
        impact: sqlx::query_as(
            "select release_version as version,
                    count(*) filter (where vote = 1 and prev_vote = -1) as to_not_slop,
                    count(*) filter (where vote = -1 and prev_vote = 1) as to_slop,
                    count(*) filter (where vote <> 0 and prev_vote is null) as new_votes
             from vote_events where app_id = $1 and release_version is not null
             group by release_version order by max(created_at) desc limit 10",
        )
        .bind(app_id)
        .fetch_all(db)
        .await?,
    })
}

/// Only the maker; anyone else gets 404.
async fn breakdown(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Breakdown>> {
    let app_id: Uuid = sqlx::query_scalar("select id from apps where slug = $1 and maker_id = $2")
        .bind(&slug)
        .bind(user.id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(breakdown_for(&state.db, app_id).await?))
}

/// Records a vote (1, -1, or 0 to withdraw) as an event plus the current vote. `at` is for the
/// simulator; real votes happen now. Returns false when nothing changed.
pub async fn cast(
    db: &sqlx::PgPool,
    user_id: Uuid,
    app_id: Uuid,
    vote: i16,
    at: Option<OffsetDateTime>,
) -> anyhow::Result<bool> {
    let mut tx = db.begin().await?;
    let usage: Option<(i64, i32, Option<String>)> = sqlx::query_as(
        "select playtime_seconds, days_launched, last_platform from user_app_usage where user_id = $1 and app_id = $2",
    )
    .bind(user_id)
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (playtime, days, platform) =
        usage.ok_or_else(|| anyhow::anyhow!("hasn't launched the app"))?;
    let prev: Option<i16> =
        sqlx::query_scalar("select vote from votes where user_id = $1 and app_id = $2 for update")
            .bind(user_id)
            .bind(app_id)
            .fetch_optional(&mut *tx)
            .await?;
    if prev.unwrap_or(0) == vote {
        return Ok(false);
    }
    let stage: String = sqlx::query_scalar("select stage::text from apps where id = $1")
        .bind(app_id)
        .fetch_one(&mut *tx)
        .await?;
    let weight = vote_weight(playtime, days);
    let at = at.unwrap_or_else(OffsetDateTime::now_utc);
    let version: Option<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select r.version {} limit 1",
        crate::releases::SERVING
    )))
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;
    sqlx::query(
        "insert into vote_events (id, user_id, app_id, vote, prev_vote, release_version, playtime_seconds, days_launched, weight, created_at, platform, stage)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12::app_stage)",
    )
    .bind(Uuid::now_v7())
    .bind(user_id)
    .bind(app_id)
    .bind(vote)
    .bind(prev)
    .bind(version)
    .bind(playtime)
    .bind(days)
    .bind(weight)
    .bind(at)
    .bind(&platform)
    .bind(&stage)
    .execute(&mut *tx)
    .await?;
    if vote == 0 {
        sqlx::query("delete from votes where user_id = $1 and app_id = $2")
            .bind(user_id)
            .bind(app_id)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query(
            "insert into votes (user_id, app_id, vote, weight, updated_at, stage) values ($1, $2, $3, $4, $5, $6::app_stage)
             on conflict (user_id, app_id) do update set vote = $3, weight = $4, updated_at = $5, stage = $6::app_stage",
        )
        .bind(user_id)
        .bind(app_id)
        .bind(vote)
        .bind(weight)
        .bind(at)
        .bind(&stage)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(true)
}

fn vote_name(v: i16) -> Option<&'static str> {
    match v {
        1 => Some("not_slop"),
        -1 => Some("slop"),
        _ => None,
    }
}

async fn my_vote(
    state: &AppState,
    user: &CurrentUser,
    app_id: Uuid,
    open: bool,
    maker: bool,
) -> ApiResult<MyVote> {
    let current: Option<i16> =
        sqlx::query_scalar("select vote from votes where user_id = $1 and app_id = $2")
            .bind(user.id)
            .bind(app_id)
            .fetch_optional(&state.db)
            .await?;
    let usage: Option<(i64, i32)> = sqlx::query_as(
        "select playtime_seconds, days_launched from user_app_usage where user_id = $1 and app_id = $2",
    )
    .bind(user.id)
    .bind(app_id)
    .fetch_optional(&state.db)
    .await?;
    let reason = if maker {
        Some(MAKER)
    } else if !open {
        Some(NOT_OPEN)
    } else if usage.is_none() {
        Some("Launch it from the Slop app first: only players can vote.")
    } else {
        None
    };
    Ok(MyVote {
        vote: current.and_then(vote_name),
        can_vote: reason.is_none(),
        reason,
        maker,
        played: usage.is_some(),
        weight: usage.map_or(0.1, |(p, d)| vote_weight(p, d)),
        review: crate::reviews::mine(&state.db, user.id, app_id).await?,
    })
}

/// Why nobody can vote yet on a Coming soon app.
const NOT_OPEN: &str = "Ratings open when it's playable: in Beta or released.";

/// Why the maker gets no vote buttons on their own app.
const MAKER: &str = "Makers can't rate their own app.";

/// The app's stage, and whether voting is open in it.
async fn stage(db: &sqlx::PgPool, app_id: Uuid) -> sqlx::Result<(&'static str, bool)> {
    let stage: String = sqlx::query_scalar("select stage::text from apps where id = $1")
        .bind(app_id)
        .fetch_one(db)
        .await?;
    Ok(match stage.as_str() {
        "released" => ("released", true),
        "beta" => ("beta", true),
        _ => ("coming_soon", false),
    })
}

async fn load(
    state: &AppState,
    user: Option<&CurrentUser>,
    app_id: Uuid,
    is_mine: bool,
) -> ApiResult<Ratings> {
    let (stage, open) = stage(&state.db, app_id).await?;
    let in_beta: i64 =
        sqlx::query_scalar("select count(*) from votes where app_id = $1 and stage = 'beta'")
            .bind(app_id)
            .fetch_one(&state.db)
            .await?;
    let mine = match user {
        Some(u) => Some(my_vote(state, u, app_id, open, is_mine).await?),
        None => None,
    };
    Ok(Ratings {
        now: tally(&state.db, app_id).await?,
        monthly: monthly(&state.db, app_id).await?,
        daily: daily(&state.db, app_id).await?,
        unusual_days: unusual_days(&state.db, app_id).await?,
        open,
        stage,
        in_beta,
        min_votes: MIN_VOTES,
        mine,
        since_new_work: crate::people::since_new_work(&state.db, app_id).await?,
    })
}

async fn ratings(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<Json<Ratings>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let (app_id, is_mine) = catalog::visible_app(&state, &slug, user.as_ref(), policy).await?;
    Ok(Json(load(&state, user.as_ref(), app_id, is_mine).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VoteBody {
    /// not_slop, slop, or null to withdraw.
    vote: Option<String>,
}

async fn vote(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(body): Json<VoteBody>,
) -> ApiResult<Json<Ratings>> {
    crate::limits::check(&state, user.id, crate::limits::Action::Vote)?;
    let policy = ViewerPolicy::resolve(&state.db, &state.config, Some(&user), &headers).await?;
    let (app_id, is_mine) = catalog::visible_app(&state, &slug, Some(&user), policy).await?;
    let value: i16 = match body.vote.as_deref() {
        Some("not_slop") => 1,
        Some("slop") => -1,
        None => 0,
        Some(_) => return Err(ApiError::invalid("vote", "not_slop, slop or null")),
    };
    if is_mine {
        return Err(ApiError::Conflict(
            "makers can't vote on their own apps".into(),
        ));
    }
    if catalog::is_private(&state.db, app_id).await? {
        return Err(ApiError::Conflict("Private Slop has no voting".into()));
    }
    let (_, open) = stage(&state.db, app_id).await?;
    let mine = my_vote(&state, &user, app_id, open, false).await?;
    if let Some(reason) = mine.reason {
        return Err(ApiError::Conflict(reason.into()));
    }
    cast(&state.db, user.id, app_id, value, None).await?;
    Ok(Json(load(&state, Some(&user), app_id, false).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_follow_metrics_section_2() {
        assert_eq!(vote_weight(60, 1), 0.1, "under 2 minutes");
        assert!(
            (vote_weight(600 * 60, 1) - 1.0).abs() < 1e-6,
            "10 hours counts fully"
        );
        assert_eq!(vote_weight(100 * 3600, 1), 1.0);
        let one_hour = vote_weight(3600, 1);
        assert!(one_hour > 0.6 && one_hour < 0.7, "{one_hour}");
        // No playtime: days launched.
        assert!((vote_weight(0, 10) - 1.0).abs() < 1e-6);
        let one_day = vote_weight(0, 1);
        assert!(one_day > 0.28 && one_day < 0.3, "{one_day}");
    }

    #[test]
    fn labels_and_minimum() {
        assert_eq!(label(100), "Overwhelmingly Not Slop");
        assert_eq!(label(94), "Very Not Slop");
        assert_eq!(label(70), "Mostly Not Slop");
        assert_eq!(label(40), "Mixed");
        assert_eq!(label(39), "Mostly Slop");
        assert_eq!(label(0), "Overwhelmingly Slop");
        assert_eq!(Tally::new(3, 1, 3.0, 1.0).percent, None);
        let t = Tally::new(4, 1, 1.0, 1.0);
        assert_eq!((t.percent, t.percent_weighted), (Some(80), Some(50)));
        assert_eq!(t.label, Some("Mixed"));
    }
}
