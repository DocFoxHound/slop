// SPDX-License-Identifier: AGPL-3.0-or-later
//! Hot (`GET /v1/shelves/hot`, DESIGN §5): the front page's rotating reel of apps taking off
//! this week. Counted per app over the last 7 days, and the 7 before for comparison, from what's
//! already public on store pages:
//!
//! - **adds**: times it was added to a library (`library_entries.added_at`; a removal takes it
//!   back);
//! - **Slaps**: accounts whose latest vote that week was 👍 Slaps (`vote_events`), so voting
//!   again or switching back and forth counts once;
//! - **players**: the most playing at once (Slop Pulse, games only).
//!
//! `activity = adds + 2 × Slaps + players`, and `score = activity + max(0, activity − last
//! week's activity)`: growth over the week before counts twice, so an app taking off beats a
//! steady one of the same size. An app needs an activity of at least `MIN_ACTIVITY` this week,
//! so one library add or one vote never makes it hot. Highest score first, then most activity.
//!
//! Only Beta and Released apps (Coming soon can't be played yet) in the store for the viewer
//! (`push_store_visible`: their age view, hidden, private and delisted apps, missed
//! deadlines), never community branches, dedicated servers or dead projects.

use axum::{Json, Router, extract::State, http::HeaderMap, routing::get};
use serde::Serialize;
use sqlx::QueryBuilder;

use crate::{
    AppState,
    auth::session::MaybeUser,
    catalog::{self, AppSummary},
    error::ApiResult,
    policy::ViewerPolicy,
};

/// Apps in the reel, at most.
pub const HOT_APPS: i64 = 10;
/// This week's activity an app needs at least.
pub const MIN_ACTIVITY: i64 = 5;

pub fn routes() -> Router<AppState> {
    Router::new().route("/shelves/hot", get(hot))
}

#[derive(Serialize, sqlx::FromRow)]
struct HotApp {
    #[serde(flatten)]
    #[sqlx(flatten)]
    app: AppSummary,
    /// Added to libraries in the last 7 days.
    adds_week: i64,
    /// Accounts whose latest vote in the last 7 days was Slaps.
    slaps_week: i64,
    /// Games: the most playing at once in the last 7 days.
    players_week: i64,
}

async fn hot(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<HotApp>>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let rules = state.config.catalog;
    // `this`: true for the last 7 days, false for the 7 before.
    let mut qb = QueryBuilder::new(
        "with votes_by_week as (
             select distinct on (e.app_id, e.user_id, w.this) e.app_id, e.vote, w.this
             from vote_events e, lateral (select e.created_at > now() - interval '7 days' as this) w
             where e.created_at > now() - interval '14 days'
             order by e.app_id, e.user_id, w.this, e.created_at desc, e.id desc),
         slaps as (
             select app_id, count(*) filter (where this and vote = 1) as week,
                    count(*) filter (where not this and vote = 1) as prev
             from votes_by_week group by app_id),
         adds as (
             select app_id, count(*) filter (where added_at > now() - interval '7 days') as week,
                    count(*) filter (where added_at <= now() - interval '7 days') as prev
             from library_entries where added_at > now() - interval '14 days' group by app_id),
         players as (
             select app_id, max(players) filter (where bucket > now() - interval '7 days') as week,
                    max(players) filter (where bucket <= now() - interval '7 days') as prev
             from pulse_counts where bucket > now() - interval '14 days' group by app_id),
         counted as (
             select a.id, coalesce(ad.week, 0) as adds_week, coalesce(sl.week, 0) as slaps_week,
                    coalesce(pl.week, 0)::int8 as players_week,
                    coalesce(ad.week, 0) + 2 * coalesce(sl.week, 0) + coalesce(pl.week, 0) as activity,
                    coalesce(ad.prev, 0) + 2 * coalesce(sl.prev, 0) + coalesce(pl.prev, 0) as before
             from apps a left join adds ad on ad.app_id = a.id left join slaps sl on sl.app_id = a.id
                  left join players pl on pl.app_id = a.id
             where a.parent_app_id is null and a.stage <> 'coming_soon'
               and not (a.stage <> 'released' and a.last_commit_at is not null
                        and a.last_commit_at < now() - make_interval(days => ",
    );
    qb.push_bind(rules.dead_after_days).push(")) and ");
    catalog::push_store_visible(&mut qb, rules, policy);
    qb.push(") ");
    catalog::push_summary_columns_with(
        &mut qb,
        &state.config,
        ", h.adds_week, h.slaps_week, h.players_week",
    );
    qb.push(" join counted h on h.id = a.id where h.activity >= ")
        .push_bind(MIN_ACTIVITY)
        .push(
            " order by h.activity + greatest(0, h.activity - h.before) desc, h.activity desc, a.slug
              limit ",
        )
        .push_bind(HOT_APPS);
    Ok(Json(qb.build_query_as().fetch_all(&state.db).await?))
}
