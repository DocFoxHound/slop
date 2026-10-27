// SPDX-License-Identifier: AGPL-3.0-or-later
//! "For you": recommendations from a user's own activity (DESIGN §5, METRICS §5), mixed with
//! discovery so the feed isn't only more of the same.
//!
//! **Taste.** Each tag gets a weight from what the user did:
//!
//! | Signal | Weight per tag |
//! |---|---|
//! | in their library | +1 |
//! | marked Interested | +2 |
//! | launched it | + its vote weight (0.1–1: more for more playtime or launch days) |
//! | voted 👍 Slaps | +2 |
//! | voted 👎 Slop | −2 |
//!
//! **Matches** are scored by how well an app's tags line up with that taste (a cosine, so an app
//! with 3 of 3 matching tags beats one with 3 of 12, and disliked tags count against it), plus a
//! little for the kind they use most (games or apps), for being played this week, and for its
//! rating. Picks are spread out (an app very like one already picked loses some score), so ten
//! roguelikes don't crowd out everything else.
//!
//! **Discovery.** A quarter of the slots (every fourth card) go to apps outside the user's taste
//! but not against it: good ratings and players count, plus a shuffle that changes daily, and
//! no two from the same first tag.
//!
//! Never recommended: what's in their library or Interested list, launched or voted on, their own apps, dead
//! projects, and anything their age view hides. Each card says why it's there. With
//! recommendations switched off (or no activity yet), everyone gets the same popular, well-rated
//! and new apps.

use std::collections::{HashMap, HashSet};

use axum::{Json, Router, extract::State, http::HeaderMap, routing::get};
use serde::Serialize;
use sqlx::{FromRow, PgPool, QueryBuilder};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::CurrentUser,
    catalog::{self, AppKind, AppSummary},
    config::Config,
    error::ApiResult,
    policy::ViewerPolicy,
    votes::vote_weight,
};

pub const SLATE: usize = 12;
/// Every DISCOVER_EVERY-th card is a discovery (a quarter of the feed).
pub const DISCOVER_EVERY: usize = 4;

pub fn routes() -> Router<AppState> {
    Router::new().route("/me/for-you", get(for_you))
}

/// An app that could be recommended, with what scoring needs.
#[derive(Clone, FromRow)]
pub struct Candidate {
    #[sqlx(flatten)]
    pub app: AppSummary,
    pub id: Uuid,
    pub maker_id: Uuid,
    pub players_week: i32,
    pub created_at: OffsetDateTime,
}

/// What a user did that shapes their recommendations.
#[derive(Default, Clone)]
pub struct Signals {
    pub library: HashSet<Uuid>,
    /// Upcoming apps they marked Interested.
    pub interested: HashSet<Uuid>,
    /// app → (playtime seconds, days launched)
    pub launched: HashMap<Uuid, (i64, i32)>,
    /// app → 1 or -1
    pub votes: HashMap<Uuid, i16>,
    pub user_id: Option<Uuid>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PickKind {
    /// Lines up with their taste.
    Match,
    /// Outside their taste, to discover.
    Discover,
    /// Not personal: popular, well rated or new.
    Popular,
}

#[derive(Clone, Debug)]
pub struct Pick {
    /// Index into the candidates.
    pub index: usize,
    pub kind: PickKind,
    pub reason: String,
    pub score: f64,
}

/// Tag weights from a user's activity. `tags_of` gives each app's tags.
pub fn taste(signals: &Signals, tags_of: &HashMap<Uuid, Vec<String>>) -> HashMap<String, f64> {
    let mut w: HashMap<String, f64> = HashMap::new();
    let mut add = |app: &Uuid, amount: f64| {
        for t in tags_of.get(app).into_iter().flatten() {
            *w.entry(t.clone()).or_default() += amount;
        }
    };
    for app in &signals.library {
        add(app, 1.0);
    }
    for app in &signals.interested {
        add(app, 2.0);
    }
    for (app, (playtime, days)) in &signals.launched {
        add(app, f64::from(vote_weight(*playtime, *days)));
    }
    for (app, vote) in &signals.votes {
        add(app, 2.0 * f64::from(*vote));
    }
    w
}

/// Deterministic 0..1 from a seed and an app, for shuffles that hold for a day.
fn shuffle(seed: u64, id: Uuid) -> f64 {
    let mut x = seed ^ (id.as_u128() as u64) ^ ((id.as_u128() >> 64) as u64);
    // splitmix64
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

fn jaccard(a: &[String], b: &[String]) -> f64 {
    let inter = a.iter().filter(|t| b.contains(t)).count();
    let union = a.len() + b.len() - inter;
    if union == 0 {
        0.0
    } else {
        inter as f64 / union as f64
    }
}

fn names(tags: &[&String], tag_names: &HashMap<String, String>) -> String {
    let n: Vec<&str> = tags
        .iter()
        .map(|t| tag_names.get(*t).map_or(t.as_str(), String::as_str))
        .collect();
    match n.as_slice() {
        [] => String::new(),
        [a] => (*a).to_string(),
        [a, b, ..] => format!("{a} & {b}"),
    }
}

/// Builds a slate of up to `n` picks. `seed` varies the discovery shuffle (per user, per day).
pub fn recommend(
    signals: &Signals,
    candidates: &[Candidate],
    tag_names: &HashMap<String, String>,
    personalized: bool,
    seed: u64,
    n: usize,
) -> Vec<Pick> {
    let tags_of: HashMap<Uuid, Vec<String>> = candidates
        .iter()
        .map(|c| (c.id, c.app.tags.clone()))
        .collect();
    let max_players = candidates.iter().map(|c| c.players_week).max().unwrap_or(0);
    let pop = |c: &Candidate| {
        if max_players == 0 {
            0.0
        } else {
            (1.0 + f64::from(c.players_week)).ln() / (1.0 + f64::from(max_players)).ln()
        }
    };
    let quality = |c: &Candidate| {
        c.app
            .not_slop_percent
            .map_or(0.0, |p| f64::from(p - 50) / 50.0)
    };
    let newest = candidates.iter().map(|c| c.created_at).max();
    let fresh = |c: &Candidate| match newest {
        Some(n) => (1.0 - (n - c.created_at).whole_days() as f64 / 60.0).clamp(0.0, 1.0),
        None => 0.0,
    };

    let eligible: Vec<usize> = (0..candidates.len())
        .filter(|&i| {
            let c = &candidates[i];
            !c.app.is_dead
                && !signals.library.contains(&c.id)
                && !signals.interested.contains(&c.id)
                && !signals.launched.contains_key(&c.id)
                && !signals.votes.contains_key(&c.id)
                && signals.user_id != Some(c.maker_id)
        })
        .collect();

    let w = taste(signals, &tags_of);
    let norm = w.values().map(|v| v * v).sum::<f64>().sqrt();
    let has_taste = personalized && w.values().any(|v| *v > 0.0);

    if !has_taste {
        // Not personal: popular, then well rated, then new, with the shuffle breaking ties.
        let mut ranked: Vec<(usize, f64, &'static str)> = eligible
            .iter()
            .map(|&i| {
                let c = &candidates[i];
                let (p, q, f) = (pop(c), quality(c), fresh(c));
                let reason = if p >= q.max(f) && p > 0.0 {
                    "Popular right now"
                } else if q >= f && q > 0.0 {
                    "Players say it slaps"
                } else {
                    "New on SLAPorSLOP"
                };
                (
                    i,
                    0.5 * p + 0.3 * q + 0.2 * f + 0.05 * shuffle(seed, c.id),
                    reason,
                )
            })
            .collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        return ranked
            .into_iter()
            .take(n)
            .map(|(index, score, reason)| Pick {
                index,
                kind: PickKind::Popular,
                reason: reason.to_string(),
                score,
            })
            .collect();
    }

    // What the user uses most: games or apps.
    // Weighted like the taste: library 1, launched by its use, 👍 2.
    let mut kind_count: HashMap<AppKind, f64> = HashMap::new();
    for c in candidates {
        let mut amount = if signals.library.contains(&c.id) {
            1.0
        } else {
            0.0
        };
        if let Some((p, d)) = signals.launched.get(&c.id) {
            amount += f64::from(vote_weight(*p, *d));
        }
        if signals.votes.get(&c.id) == Some(&1) {
            amount += 2.0;
        }
        if amount > 0.0 {
            *kind_count.entry(c.app.kind).or_default() += amount;
        }
    }
    let kind_total: f64 = kind_count.values().sum();
    let affinity = |c: &Candidate| {
        if c.app.tags.is_empty() || norm == 0.0 {
            return 0.0;
        }
        let dot: f64 = c
            .app
            .tags
            .iter()
            .map(|t| w.get(t).copied().unwrap_or(0.0))
            .sum();
        dot / ((c.app.tags.len() as f64).sqrt() * norm)
    };
    let kind_share = |c: &Candidate| {
        if kind_total == 0.0 {
            0.0
        } else {
            kind_count.get(&c.app.kind).copied().unwrap_or(0.0) / kind_total
        }
    };

    let scored: Vec<(usize, f64, f64)> = eligible
        .iter()
        .map(|&i| {
            let c = &candidates[i];
            let a = affinity(c);
            (
                i,
                a,
                a + 0.15 * kind_share(c) + 0.10 * pop(c) + 0.15 * quality(c),
            )
        })
        .collect();

    // Matches, greedily, each losing a little for being like ones already picked. Games and
    // apps get the share of match slots the user gives them (calibration): someone who mostly
    // plays games but uses a few tools sees mostly games and a few tools, not games only.
    let discover_slots = n / DISCOVER_EVERY;
    let match_slots = n - discover_slots;
    let mut quota: HashMap<AppKind, usize> = HashMap::new();
    if kind_total > 0.0 {
        let apps_share = kind_count.get(&AppKind::App).copied().unwrap_or(0.0) / kind_total;
        let mut for_apps = (match_slots as f64 * apps_share).round() as usize;
        if apps_share >= 0.05 && for_apps == 0 {
            for_apps = 1;
        }
        if apps_share <= 0.95 && for_apps == match_slots {
            for_apps -= 1;
        }
        quota.insert(AppKind::App, for_apps);
        quota.insert(AppKind::Game, match_slots - for_apps);
    }
    let mut pool: Vec<(usize, f64, f64)> = scored.iter().filter(|s| s.1 > 0.0).copied().collect();
    let mut matches: Vec<Pick> = Vec::new();
    let mut taken: HashMap<AppKind, usize> = HashMap::new();
    while matches.len() < match_slots && !pool.is_empty() {
        // A kind with its quota filled waits, unless nothing of the other kind is left.
        let open = |k: AppKind| {
            quota
                .get(&k)
                .is_none_or(|q| taken.get(&k).copied().unwrap_or(0) < *q)
        };
        let any_open = pool.iter().any(|&(i, _, _)| open(candidates[i].app.kind));
        let (best, _) = pool
            .iter()
            .enumerate()
            .filter(|(_, (i, _, _))| !any_open || open(candidates[*i].app.kind))
            .map(|(k, &(i, _, s))| {
                let similar = matches
                    .iter()
                    .map(|m| jaccard(&candidates[i].app.tags, &candidates[m.index].app.tags))
                    .fold(0.0, f64::max);
                (k, s - 0.3 * similar)
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .expect("pool isn't empty");
        let (i, _, s) = pool.swap_remove(best);
        *taken.entry(candidates[i].app.kind).or_default() += 1;
        let mut liked: Vec<&String> = candidates[i]
            .app
            .tags
            .iter()
            .filter(|t| w.get(*t).copied().unwrap_or(0.0) > 0.0)
            .collect();
        liked.sort_by(|a, b| w[*b].total_cmp(&w[*a]));
        matches.push(Pick {
            index: i,
            kind: PickKind::Match,
            reason: format!("Because you like {}", names(&liked, tag_names)),
            score: s,
        });
    }

    // Discovery: outside their taste (low affinity) but not against it (no disliked majority).
    let picked: HashSet<usize> = matches.iter().map(|m| m.index).collect();
    let mut explore: Vec<(usize, f64)> = scored
        .iter()
        .filter(|(i, a, _)| !picked.contains(i) && *a < 0.2 && *a > -0.1)
        .map(|&(i, _, _)| {
            let c = &candidates[i];
            (
                i,
                0.4 * quality(c).max(0.0)
                    + 0.3 * pop(c)
                    + 0.1 * fresh(c)
                    + 0.4 * shuffle(seed, c.id),
            )
        })
        .collect();
    explore.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut discoveries: Vec<Pick> = Vec::new();
    let mut first_tags: HashSet<&String> = HashSet::new();
    for (i, score) in explore {
        if discoveries.len() >= discover_slots + (match_slots - matches.len()) {
            break;
        }
        let c = &candidates[i];
        let lead = c
            .app
            .tags
            .iter()
            .find(|t| w.get(*t).copied().unwrap_or(0.0) <= 0.0);
        if let Some(t) = lead
            && !first_tags.insert(t)
        {
            continue;
        }
        let reason = match (lead, c.app.not_slop_percent) {
            (Some(t), Some(p)) if p >= 80 => format!(
                "Something different: {}, {p}% Slaps",
                names(&[t], tag_names)
            ),
            (Some(t), _) => format!("Something different: {}", names(&[t], tag_names)),
            (None, _) => "Something different".to_string(),
        };
        discoveries.push(Pick {
            index: i,
            kind: PickKind::Discover,
            reason,
            score,
        });
    }

    // Every DISCOVER_EVERY-th card is a discovery; leftovers fill the end.
    let mut slate = Vec::with_capacity(n);
    let (mut m, mut d) = (matches.into_iter(), discoveries.into_iter());
    while slate.len() < n {
        let want_discovery = (slate.len() + 1) % DISCOVER_EVERY == 0;
        let next = if want_discovery {
            d.next().or_else(|| m.next())
        } else {
            m.next().or_else(|| d.next())
        };
        match next {
            Some(p) => slate.push(p),
            None => break,
        }
    }
    slate
}

/// Everything in the store for this viewer, with what scoring needs.
pub async fn candidates(
    db: &PgPool,
    config: &Config,
    policy: ViewerPolicy,
) -> sqlx::Result<Vec<Candidate>> {
    let mut qb = QueryBuilder::new("select s.*, ");
    qb.push(catalog::PLAYERS_THIS_WEEK.replace("a.id", "s.id"))
        .push(" as players_week from (");
    catalog::push_summary_columns_with(&mut qb, config, ", a.id, a.maker_id, a.created_at");
    qb.push(" where a.parent_app_id is null and ");
    catalog::push_store_visible(&mut qb, config.catalog, policy);
    qb.push(") s order by s.slug");
    qb.build_query_as().fetch_all(db).await
}

pub async fn signals(db: &PgPool, user_id: Uuid) -> sqlx::Result<Signals> {
    let library: Vec<Uuid> =
        sqlx::query_scalar("select app_id from library_entries where user_id = $1")
            .bind(user_id)
            .fetch_all(db)
            .await?;
    let interested: Vec<Uuid> =
        sqlx::query_scalar("select app_id from interests where user_id = $1")
            .bind(user_id)
            .fetch_all(db)
            .await?;
    let launched: Vec<(Uuid, i64, i32)> = sqlx::query_as(
        "select app_id, playtime_seconds, days_launched from user_app_usage where user_id = $1",
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;
    let votes: Vec<(Uuid, i16)> =
        sqlx::query_as("select app_id, vote from votes where user_id = $1")
            .bind(user_id)
            .fetch_all(db)
            .await?;
    Ok(Signals {
        library: library.into_iter().collect(),
        interested: interested.into_iter().collect(),
        launched: launched.into_iter().map(|(a, p, d)| (a, (p, d))).collect(),
        votes: votes.into_iter().collect(),
        user_id: Some(user_id),
    })
}

pub async fn tag_names(db: &PgPool) -> sqlx::Result<HashMap<String, String>> {
    Ok(
        sqlx::query_as::<_, (String, String)>("select slug, name from tags")
            .fetch_all(db)
            .await?
            .into_iter()
            .collect(),
    )
}

/// Whether a user's recommendations are personal: their setting, else on except in the
/// under-18 view (PRIV-6).
pub async fn personalized(db: &PgPool, user_id: Uuid, policy: ViewerPolicy) -> sqlx::Result<bool> {
    let (setting, gpc): (Option<bool>, bool) =
        sqlx::query_as("select personalize, gpc_at is not null from users where id = $1")
            .bind(user_id)
            .fetch_one(db)
            .await?;
    // Their own choice wins; otherwise off in the under-18 view or after a Global Privacy
    // Control signal (migration 0055).
    Ok(setting.unwrap_or(!gpc && policy != ViewerPolicy::UNDER_18_VIEW))
}

/// A seed that changes daily per user, so discovery rotates.
pub fn daily_seed(user_id: Uuid, day: time::Date) -> u64 {
    (user_id.as_u128() as u64) ^ (day.to_julian_day() as u64).wrapping_mul(0x2545_F491_4F6C_DD1D)
}

#[derive(Serialize)]
pub struct Card {
    #[serde(flatten)]
    pub app: AppSummary,
    pub pick: PickKind,
    pub reason: String,
}

#[derive(Serialize)]
pub struct ForYou {
    pub personalized: bool,
    /// The tags the user's taste leans on most, for "Based on …".
    pub based_on: Vec<String>,
    pub items: Vec<Card>,
}

pub async fn for_user(
    db: &PgPool,
    config: &Config,
    user_id: Uuid,
    policy: ViewerPolicy,
    seed: u64,
) -> sqlx::Result<ForYou> {
    let personal = personalized(db, user_id, policy).await?;
    let cands = candidates(db, config, policy).await?;
    let signals = signals(db, user_id).await?;
    let names = tag_names(db).await?;
    let picks = recommend(&signals, &cands, &names, personal, seed, SLATE);
    let based_on = if personal {
        let tags_of = cands.iter().map(|c| (c.id, c.app.tags.clone())).collect();
        let mut w: Vec<(String, f64)> = taste(&signals, &tags_of)
            .into_iter()
            .filter(|(_, v)| *v > 0.0)
            .collect();
        w.sort_by(|a, b| b.1.total_cmp(&a.1));
        w.into_iter()
            .take(3)
            .map(|(t, _)| names.get(&t).cloned().unwrap_or(t))
            .collect()
    } else {
        vec![]
    };
    Ok(ForYou {
        personalized: personal && picks.iter().any(|p| p.kind != PickKind::Popular),
        based_on,
        items: picks
            .into_iter()
            .map(|p| Card {
                app: cands[p.index].app.clone(),
                pick: p.kind,
                reason: p.reason,
            })
            .collect(),
    })
}

async fn for_you(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
) -> ApiResult<Json<ForYou>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, Some(&user), &headers).await?;
    let seed = daily_seed(user.id, OffsetDateTime::now_utc().date());
    Ok(Json(
        for_user(&state.db, &state.config, user.id, policy, seed).await?,
    ))
}
