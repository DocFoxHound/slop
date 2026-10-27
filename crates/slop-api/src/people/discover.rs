// SPDX-License-Identifier: AGPL-3.0-or-later
//! The People front page's shelves (`GET /v1/people/discover`), like the store's: a different
//! mix every day, so a visitor finds someone new and nobody is buried (DESIGN §15.1).
//!
//! None of them is ordered by how well a creative does (ratings, clients): ranking on
//! performance needs the lawyer first (LEGAL-GUIDANCE HUM-10). Each is a filter, then either the
//! day's shuffle or how new the page is.

use axum::{Json, extract::State};
use serde::Serialize;
use sqlx::{Postgres, QueryBuilder};
use time::OffsetDateTime;

use super::{Card, CardRow, OPEN_SQL, SKILLS, Skill, push_card_select, skill};
use crate::{AppState, error::ApiResult};

/// People on each shelf.
const SHELF: i64 = 8;
/// How long a page counts as new on People.
const NEW_DAYS: i32 = 60;

/// When a creative's page first went live: their first approved portfolio.
const WENT_LIVE_SQL: &str =
    ", (select min(coalesce(p.reviewed_at, p.created_at)) from portfolio_sets p
      where p.creative_id = c.user_id and p.status = 'approved') as went_live";

#[derive(Serialize)]
pub(super) struct Discover {
    /// Can be hired now (taking work, payments set up), shuffled for the day.
    taking_work: Vec<Card>,
    /// Pages that went live in the last 60 days, newest first.
    new: Vec<Card>,
    /// One skill a day, in turn, with the day's shuffle of the people who offer it.
    spotlight: Option<Spotlight>,
    /// Studios (teams), shuffled for the day.
    studios: Vec<Card>,
}

#[derive(Serialize)]
struct Spotlight {
    skill: Skill,
    people: Vec<Card>,
}

/// Up to a shelf of creatives in the directory that `filter` pushes the condition for (SQL over
/// `c`), in the day's shuffle. `salt` gives each shelf its own order, so the same people don't
/// lead every shelf.
async fn shuffled(
    state: &AppState,
    filter: impl FnOnce(&mut QueryBuilder<Postgres>),
    salt: &str,
) -> ApiResult<Vec<Card>> {
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("select * from (");
    push_card_select(&mut qb, "");
    qb.push(" where ").push(OPEN_SQL).push(" and ");
    filter(&mut qb);
    qb.push(") x order by md5(handle || ")
        .push_bind(format!("{}{salt}", OffsetDateTime::now_utc().date()))
        .push(") limit ")
        .push_bind(SHELF);
    rows(state, qb).await
}

async fn rows(state: &AppState, mut qb: QueryBuilder<Postgres>) -> ApiResult<Vec<Card>> {
    let rows: Vec<CardRow> = qb.build_query_as().fetch_all(&state.db).await?;
    Ok(rows
        .into_iter()
        .map(|r| r.into_card(&state.config))
        .collect())
}

pub(super) async fn discover(State(state): State<AppState>) -> ApiResult<Json<Discover>> {
    let taking_work = shuffled(
        &state,
        |qb| {
            qb.push("c.taking_work and c.stripe_status = 'active'");
        },
        "taking",
    )
    .await?;

    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("select * from (");
    push_card_select(&mut qb, WENT_LIVE_SQL);
    qb.push(" where ").push(OPEN_SQL);
    qb.push(") x where went_live > now() - make_interval(days => ")
        .push_bind(NEW_DAYS)
        .push(") order by went_live desc, handle limit ")
        .push_bind(SHELF);
    let new = rows(&state, qb).await?;

    // The skills someone offers, in the directory's order; today's is the next one in turn.
    let offered: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select distinct s from creatives c, unnest(c.skills) s where {OPEN_SQL}"
    )))
    .fetch_all(&state.db)
    .await?;
    let offered: Vec<&str> = SKILLS
        .iter()
        .map(|(s, _)| *s)
        .filter(|s| offered.iter().any(|o| o == s))
        .collect();
    let spotlight = match offered.len() {
        0 => None,
        n => {
            let today = OffsetDateTime::now_utc().date().to_julian_day();
            let slug = offered[today.rem_euclid(n as i32) as usize];
            Some(Spotlight {
                skill: skill(slug),
                people: shuffled(
                    &state,
                    |qb| {
                        qb.push_bind(slug.to_string()).push(" = any(c.skills)");
                    },
                    slug,
                )
                .await?,
            })
        }
    };

    let studios = shuffled(
        &state,
        |qb| {
            qb.push("c.kind = 'studio'");
        },
        "studios",
    )
    .await?;
    Ok(Json(Discover {
        taking_work,
        new,
        spotlight,
        studios,
    }))
}
