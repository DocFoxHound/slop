// SPDX-License-Identifier: AGPL-3.0-or-later
//! The People front page's shelves (`GET /v1/people/discover`), like the store's: a different
//! mix every day, so a visitor finds someone new and nobody is buried (DESIGN §15.1).
//!
//! None of them is ordered by how well a creative does (ratings, clients): ranking on
//! performance needs the lawyer first (LEGAL-GUIDANCE HUM-10). Each is a filter, then either the
//! day's shuffle or how new the page is.
//!
//! **Suggested for you** (signed in, with personal recommendations on: foryou::personalized) is
//! the one personal shelf: people offering the skills the viewer has hired for (their jobs as a
//! client, the most asked-for first), or, before they've hired anyone, the skills that fit what
//! they publish (games or apps). Those who can be hired now come first, then the viewer's own
//! shuffle for the day; never the viewer's own page. Its `why` says which of the two it is (DSA
//! Art. 27).
//!
//! Then a shelf per skill anyone offers, starting from the day's spotlight skill.

use axum::{Json, extract::State, http::HeaderMap};
use serde::Serialize;
use sqlx::{Postgres, QueryBuilder};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{Card, CardRow, OPEN_SQL, SKILLS, Skill, push_card_select, skill};
use crate::{AppState, auth::session::MaybeUser, error::ApiResult, policy::ViewerPolicy};

/// People on each shelf.
const SHELF: i64 = 12;
/// Skills that games are usually made with, for a maker who hasn't hired anyone yet.
const GAME_SKILLS: [&str; 5] = ["2d-art", "pixel-art", "music", "sound", "playtesting"];
/// The same for apps.
const APP_SKILLS: [&str; 4] = ["ui-ux", "writing", "localization", "playtesting"];
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
    /// Studios (teams), shuffled for the day.
    studios: Vec<Card>,
    /// A shelf per skill someone offers, each the day's shuffle of the people who offer it,
    /// starting from today's spotlight skill (one skill a day, in turn).
    by_skill: Vec<Spotlight>,
    /// People for the viewer; null signed out, switched off, or with nothing to go on.
    suggested: Option<Suggested>,
}

#[derive(Serialize)]
struct Suggested {
    /// What it's based on, in plain words.
    why: String,
    skills: Vec<Skill>,
    people: Vec<Card>,
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

/// "A", "A and B", "A, B and C".
fn and_list(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Suggested for you, or None (see the module's comment).
async fn suggested(
    state: &AppState,
    viewer: Uuid,
    policy: ViewerPolicy,
) -> ApiResult<Option<Suggested>> {
    if !crate::foryou::personalized(&state.db, viewer, policy).await? {
        return Ok(None);
    }
    let hired: Vec<String> = sqlx::query_scalar(
        "select skill from jobs where client_id = $1
         group by skill order by count(*) desc, max(created_at) desc",
    )
    .bind(viewer)
    .fetch_all(&state.db)
    .await?;
    let hired: Vec<&str> = hired
        .iter()
        .filter_map(|h| SKILLS.iter().find(|(s, _)| s == h).map(|(s, _)| *s))
        .take(3)
        .collect();
    let (slugs, why): (Vec<&str>, String) = if hired.is_empty() {
        let kinds: Vec<String> = sqlx::query_scalar(
            "select distinct kind::text from apps where maker_id = $1 and status <> 'delisted'",
        )
        .bind(viewer)
        .fetch_all(&state.db)
        .await?;
        if kinds.iter().any(|k| k == "game") {
            (
                GAME_SKILLS.to_vec(),
                "For the games you make: art, music, sound and playtesting".into(),
            )
        } else if kinds.iter().any(|k| k == "app") {
            (
                APP_SKILLS.to_vec(),
                "For the apps you make: design, writing, translation and testing".into(),
            )
        } else {
            return Ok(None);
        }
    } else {
        let names: Vec<&str> = hired.iter().map(|s| skill(s).name).collect();
        (
            hired.clone(),
            format!("Because you've hired for {}", and_list(&names)),
        )
    };
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("select * from (");
    push_card_select(&mut qb, "");
    qb.push(" where ")
        .push(OPEN_SQL)
        .push(" and c.user_id <> ")
        .push_bind(viewer)
        .push(" and c.skills && ")
        .push_bind(slugs.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        .push(") x order by can_hire desc, md5(handle || ")
        .push_bind(format!("{}{viewer}", OffsetDateTime::now_utc().date()))
        .push(") limit ")
        .push_bind(SHELF);
    let people = rows(state, qb).await?;
    if people.is_empty() {
        return Ok(None);
    }
    Ok(Some(Suggested {
        why,
        skills: slugs.iter().map(|s| skill(s)).collect(),
        people,
    }))
}

pub(super) async fn discover(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
) -> ApiResult<Json<Discover>> {
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
    // Today's spotlight first, then the rest in the directory's order, wrapping round.
    let first = match offered.len() {
        0 => 0,
        n => OffsetDateTime::now_utc()
            .date()
            .to_julian_day()
            .rem_euclid(n as i32) as usize,
    };
    let mut by_skill = Vec::with_capacity(offered.len());
    for i in 0..offered.len() {
        let slug = offered[(first + i) % offered.len()];
        by_skill.push(Spotlight {
            skill: skill(slug),
            people: shuffled(
                &state,
                |qb| {
                    qb.push_bind(slug.to_string()).push(" = any(c.skills)");
                },
                slug,
            )
            .await?,
        });
    }

    let studios = shuffled(
        &state,
        |qb| {
            qb.push("c.kind = 'studio'");
        },
        "studios",
    )
    .await?;
    let suggested = match &user {
        Some(u) => {
            let policy = ViewerPolicy::resolve(&state.db, &state.config, Some(u), &headers).await?;
            suggested(&state, u.id, policy).await?
        }
        None => None,
    };
    Ok(Json(Discover {
        taking_work,
        new,
        studios,
        by_skill,
        suggested,
    }))
}
