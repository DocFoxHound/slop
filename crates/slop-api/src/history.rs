// SPDX-License-Identifier: AGPL-3.0-or-later
//! History for makers (DESIGN §11, METRICS §3): for each UTC day, how many accounts played an
//! app, how many played it for the first time and how many times it was added to a library,
//! with the day's busiest moment from Slop Pulse.
//!
//! Counts only (`app_daily_counts`): no account ids and no breakdowns. The numbers are written
//! where the thing happens (pulse.rs launch, library.rs add and remove); this module only reads
//! them, for the app's maker. `apps.history_from` and `apps.players_from` say from which day the
//! numbers are real, so a day that was never counted is `null`, not zero.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use serde::{Deserialize, Serialize};

use crate::{
    AppState,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
    limits, updates,
};

pub fn routes() -> Router<AppState> {
    Router::new().route("/apps/{slug}/history", get(history))
}

#[derive(Deserialize)]
struct Range {
    /// 30 (default), 90 or 365.
    days: Option<String>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Day {
    /// YYYY-MM-DD (UTC).
    pub day: String,
    /// Accounts that started the app that day. None: not counted that day.
    pub players: Option<i32>,
    /// Accounts that started it for the first time. None: not counted that day.
    pub first_plays: Option<i32>,
    /// Times it was added to a library. None: not counted that day.
    pub library_adds: Option<i32>,
    /// The most playing at once (Slop Pulse). None: not counted that day.
    pub peak: Option<i32>,
}

#[derive(Serialize)]
pub struct History {
    /// The first UTC day anything was counted for this app.
    pub history_from: String,
    /// The first UTC day players per day were counted.
    pub players_from: String,
    /// Every UTC day in the range, oldest first, ending today.
    pub days: Vec<Day>,
}

/// Only the maker; anyone else gets 404. The limit is charged first, so it says nothing about
/// any app.
async fn history(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(range): Query<Range>,
) -> ApiResult<Json<History>> {
    limits::check(&state, user.id, limits::Action::History)?;
    let (app_id, _) = updates::owned(&state, &slug, &user).await?;
    let days: i32 = match range.days.as_deref() {
        None | Some("30") => 30,
        Some("90") => 90,
        Some("365") => 365,
        Some(_) => return Err(ApiError::invalid("days", "30, 90 or 365")),
    };
    let (private, server, history_from, players_from): (bool, bool, String, String) =
        sqlx::query_as(
            "select visibility = 'private', branch_kind is not distinct from 'server',
                    to_char(history_from, 'YYYY-MM-DD'), to_char(players_from, 'YYYY-MM-DD')
             from apps where id = $1",
        )
        .bind(app_id)
        .fetch_one(&state.db)
        .await?;
    // Nothing is counted for either (pulse.rs, library.rs), so there is nothing to show.
    if private {
        return Err(ApiError::Conflict("Private apps have no history".into()));
    }
    if server {
        return Err(ApiError::Conflict(
            "Dedicated servers have no history".into(),
        ));
    }
    // Every day of the range, with or without a row. Before the day counting began the answer
    // is null. The busiest moment is the same reading of pulse_counts as pulse.rs.
    let days: Vec<Day> = sqlx::query_as(
        "select to_char(d.day, 'YYYY-MM-DD') as day,
                case when d.day >= a.players_from then coalesce(c.players, 0) end as players,
                case when d.day >= a.history_from then coalesce(c.first_plays, 0) end as first_plays,
                case when d.day >= a.history_from then coalesce(c.library_adds, 0) end as library_adds,
                case when d.day >= a.history_from then coalesce(p.peak, 0) end as peak
         from apps a
         cross join (select generate_series((now() at time zone 'UTC')::date - ($2::int - 1),
                                            (now() at time zone 'UTC')::date,
                                            interval '1 day')::date as day) d
         left join app_daily_counts c on c.app_id = a.id and c.day = d.day
         left join (select (bucket at time zone 'UTC')::date as day, max(players) as peak
                    from pulse_counts
                    where app_id = $1
                      and bucket >= ((now() at time zone 'UTC')::date - ($2::int - 1))::timestamp at time zone 'UTC'
                    group by 1) p on p.day = d.day
         where a.id = $1
         order by d.day",
    )
    .bind(app_id)
    .bind(days)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(History {
        history_from,
        players_from,
        days,
    }))
}
