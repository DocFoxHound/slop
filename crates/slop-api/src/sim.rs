//! The simulation sandbox: a made-up store full of apps and players, for testing recommendations
//! ("For you"), ratings (% Not Slop over time) and player counts against a known truth.
//!
//! **Only ever on a sandbox database** (its name ends in `_sim`, or a test database): it writes
//! thousands of fake users, apps, votes and player counts. `slop-api simulate …` refuses
//! anywhere else, and the server only offers "view as a simulated user" when `SLOP_SIMULATION`
//! is set on a sandbox database.
//!
//! **The hidden truth.** Every simulated user has a taste (a weight per tag, from one or two
//! personas such as "roguelike fan" or "developer"; some are gamers who also use a few tools),
//! and every app has a quality (some change with an update partway through). Each simulated day,
//! users find apps (likelier when they match their taste and when others have them), play them
//! (more when they enjoy them), and vote: 👍 with a probability that rises with quality and taste
//! match. After an update, some change their vote. The truth is kept in `sim_users` and
//! `sim_apps`, which the real code never reads; only the report does.
//!
//! **The report** then asks the real code (foryou.rs, votes.rs, pulse.rs) for its answers and
//! scores them against the truth.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, ensure};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header::USER_AGENT},
    routing::{get, post},
};
use axum_extra::extract::CookieJar;
use serde::Serialize;
use sqlx::{PgPool, Postgres, QueryBuilder};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{
    AppState,
    auth::session,
    catalog::AppKind,
    config::Config,
    error::{ApiError, ApiResult},
    foryou::{self, PickKind},
    policy::ViewerPolicy,
    votes::{self, vote_weight},
};

// ---------------------------------------------------------------------------------------------
// Guard

/// Refuses unless this is a sandbox: a database named `…_sim`, or a test database.
pub async fn ensure_sandbox(db: &PgPool) -> anyhow::Result<String> {
    let name: String = sqlx::query_scalar("select current_database()::text")
        .fetch_one(db)
        .await?;
    ensure!(
        name.ends_with("_sim") || name.starts_with("_sqlx_test"),
        "the simulation only runs on a sandbox database (its name must end in _sim); this is {name}"
    );
    Ok(name)
}

// ---------------------------------------------------------------------------------------------
// Randomness (deterministic from the seed, so a run can be repeated)

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0x5DEE_CE66_D1CE_4E5B)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// 0..1
    pub fn f(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn chance(&mut self, p: f64) -> bool {
        self.f() < p
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
    /// Index chosen with probability proportional to its weight.
    pub fn weighted(&mut self, weights: &[f64]) -> Option<usize> {
        let total: f64 = weights.iter().filter(|w| **w > 0.0).sum();
        if total <= 0.0 {
            return None;
        }
        let mut x = self.f() * total;
        for (i, w) in weights.iter().enumerate() {
            if *w > 0.0 {
                if x < *w {
                    return Some(i);
                }
                x -= w;
            }
        }
        weights.iter().rposition(|w| *w > 0.0)
    }
}

// ---------------------------------------------------------------------------------------------
// Personas: kinds of taste, over the real tags (migration 0002)

pub struct Persona {
    pub name: &'static str,
    pub kind: AppKind,
    pub tags: &'static [&'static str],
}

pub const PERSONAS: &[Persona] = &[
    Persona {
        name: "roguelike fan",
        kind: AppKind::Game,
        tags: &[
            "roguelike",
            "pixel-art",
            "retro",
            "single-player",
            "dark",
            "short-sessions",
        ],
    },
    Persona {
        name: "cozy gamer",
        kind: AppKind::Game,
        tags: &[
            "cozy",
            "simulation",
            "relaxing",
            "idle",
            "tycoon",
            "visual-novel",
        ],
    },
    Persona {
        name: "strategist",
        kind: AppKind::Game,
        tags: &[
            "strategy",
            "card-game",
            "tycoon",
            "single-player",
            "minimalist",
        ],
    },
    Persona {
        name: "action fan",
        kind: AppKind::Game,
        tags: &[
            "platformer",
            "racing",
            "chaotic",
            "controller-support",
            "multiplayer",
            "co-op",
        ],
    },
    Persona {
        name: "puzzler",
        kind: AppKind::Game,
        tags: &[
            "puzzle",
            "minimalist",
            "short-sessions",
            "relaxing",
            "funny",
        ],
    },
    Persona {
        name: "developer",
        kind: AppKind::App,
        tags: &[
            "developer-tools",
            "automation",
            "file-management",
            "productivity",
            "system-monitor",
        ],
    },
    Persona {
        name: "creator",
        kind: AppKind::App,
        tags: &[
            "music",
            "pixel-art",
            "3d",
            "generative-art",
            "video",
            "writing",
        ],
    },
    Persona {
        name: "organiser",
        kind: AppKind::App,
        tags: &[
            "note-taking",
            "productivity",
            "finance",
            "backup",
            "privacy",
            "minimalist",
        ],
    },
    Persona {
        name: "learner",
        kind: AppKind::App,
        tags: &["language", "math", "typing", "kids", "offline"],
    },
];

const GAME_PERSONAS: std::ops::Range<usize> = 0..5;
const TOOL_PERSONAS: std::ops::Range<usize> = 5..9;

const ADJECTIVES: &[&str] = &[
    "Angry", "Tiny", "Neon", "Cozy", "Quantum", "Rusty", "Silent", "Pixel", "Lucky", "Hollow",
    "Golden", "Frozen", "Wild", "Clever", "Sleepy", "Brave", "Crimson", "Velvet", "Cosmic",
    "Feral", "Paper", "Iron", "Moss", "Static", "Hyper",
];
const GAME_NOUNS: &[&str] = &[
    "Herds", "Dungeon", "Garden", "Racers", "Tactics", "Kingdom", "Island", "Caverns", "Slimes",
    "Orbit", "Farm", "Arena", "Heist", "Tower", "Voyage", "Bakery", "Mansion", "Swarm", "Derby",
    "Crypt",
];
const TOOL_NOUNS: &[&str] = &[
    "Notes", "Ledger", "Sync", "Studio", "Tracker", "Launcher", "Backup", "Monitor", "Tutor",
    "Sketch", "Mixer", "Vault", "Scribe", "Runner", "Lens", "Deck", "Shelf", "Clock", "Forge",
    "Canvas",
];

// ---------------------------------------------------------------------------------------------
// Generation

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub apps: usize,
    pub users: usize,
    pub days: i64,
    pub seed: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            apps: 400,
            users: 1500,
            days: 120,
            seed: 42,
        }
    }
}

struct SimApp {
    id: Uuid,
    slug: String,
    kind: AppKind,
    theme: usize,
    tags: Vec<String>,
    stage: &'static str,
    min_age: i16,
    quality: f64,
    /// (day of the update, quality after it)
    update: Option<(i64, f64)>,
    created_day: i64,
    dead: bool,
    /// Reports playtime through Slop Pulse.
    pulse: bool,
    maker: Uuid,
}

impl SimApp {
    fn quality_on(&self, day: i64) -> f64 {
        match self.update {
            Some((d, q)) if day >= d => q,
            _ => self.quality,
        }
    }
    fn playable(&self) -> bool {
        self.stage != "coming_soon"
    }
}

struct SimUser {
    id: Uuid,
    name: String,
    country: &'static str,
    personas: Vec<(usize, f64)>,
    taste: HashMap<String, f64>,
    activity: f64,
}

/// How well an app's tags fit a taste: a cosine, 0..1.
pub fn fit(taste: &HashMap<String, f64>, tags: &[String]) -> f64 {
    let norm = taste.values().map(|v| v * v).sum::<f64>().sqrt();
    if norm == 0.0 || tags.is_empty() {
        return 0.0;
    }
    let dot: f64 = tags
        .iter()
        .map(|t| taste.get(t).copied().unwrap_or(0.0))
        .sum();
    (dot / ((tags.len() as f64).sqrt() * norm)).clamp(0.0, 1.0)
}

/// The truth: how likely a user is to vote 👍 for an app of this quality and fit.
pub fn like_chance(quality: f64, fit: f64) -> f64 {
    1.0 / (1.0 + (-7.0 * (0.55 * quality + 0.45 * fit - 0.45)).exp())
}

fn taste_of(personas: &[(usize, f64)]) -> HashMap<String, f64> {
    let mut t = HashMap::new();
    for (p, w) in personas {
        for tag in PERSONAS[*p].tags {
            *t.entry((*tag).to_string()).or_default() += w;
        }
    }
    t
}

struct Event {
    user: usize,
    app: usize,
    vote: i16,
    prev: Option<i16>,
    playtime: i64,
    days: i32,
    weight: f32,
    at: OffsetDateTime,
}

#[derive(Debug, Serialize)]
pub struct Generated {
    pub apps: usize,
    pub users: usize,
    pub makers: usize,
    pub library_entries: usize,
    pub launched: usize,
    pub vote_events: usize,
    pub current_votes: usize,
    pub player_buckets: usize,
    pub people: crate::sim_people::PeopleGenerated,
    pub moderation: crate::sim_moderation::ModerationGenerated,
    /// Interested marks on upcoming apps.
    pub interests: u64,
    pub donations: crate::sim_donations::DonationsGenerated,
    /// Written reviews with votes.
    pub reviews: u64,
    /// Community branches (DESIGN §9).
    pub branches: usize,
    /// Private Slop apps, each shared with a few players.
    pub private_apps: usize,
}

/// Wipes the sandbox's catalog and users, then makes a new simulated store.
pub async fn generate(db: &PgPool, settings: Settings) -> anyhow::Result<Generated> {
    ensure_sandbox(db).await?;
    let mut rng = Rng::new(settings.seed);
    let now = OffsetDateTime::now_utc();
    let start = now - Duration::days(settings.days);
    let at = |day: i64, rng: &mut Rng| {
        start + Duration::days(day) + Duration::seconds((rng.f() * 86_000.0) as i64)
    };

    // Makers: mostly in the US, some in the UK and Germany (People's consumer rules differ).
    let maker_rows: Vec<(Uuid, String, &'static str)> = (0..40)
        .map(|i| {
            let country = match i % 7 {
                5 => "GB",
                6 => "DE",
                _ => "US",
            };
            (Uuid::now_v7(), format!("Sim maker {}", i + 1), country)
        })
        .collect();
    let makers: Vec<Uuid> = maker_rows.iter().map(|m| m.0).collect();

    // Apps: each built around a persona's tags, plus a little noise.
    let all_tags: Vec<String> = sqlx::query_scalar("select slug from tags order by slug")
        .fetch_all(db)
        .await?;
    let mut apps: Vec<SimApp> = Vec::with_capacity(settings.apps);
    let mut slugs = HashSet::new();
    for i in 0..settings.apps {
        let theme = if rng.chance(0.68) {
            GAME_PERSONAS.start + rng.below(GAME_PERSONAS.len())
        } else {
            TOOL_PERSONAS.start + rng.below(TOOL_PERSONAS.len())
        };
        let persona = &PERSONAS[theme];
        let mut tags: Vec<String> = Vec::new();
        let want = 2 + rng.below(3);
        while tags.len() < want {
            let t = rng.pick(persona.tags).to_string();
            if !tags.contains(&t) {
                tags.push(t);
            }
        }
        for _ in 0..rng.below(3) {
            let t = rng.pick(&all_tags).clone();
            if !tags.contains(&t) {
                tags.push(t);
            }
        }
        let nouns = if persona.kind == AppKind::Game {
            GAME_NOUNS
        } else {
            TOOL_NOUNS
        };
        let mut slug = format!("{} {}", rng.pick(ADJECTIVES), rng.pick(nouns))
            .to_lowercase()
            .replace(' ', "-");
        if !slugs.insert(slug.clone()) {
            slug = format!("{slug}-{i}");
            slugs.insert(slug.clone());
        }
        let r = rng.f();
        let stage = if r < 0.6 {
            "released"
        } else if r < 0.85 {
            "beta"
        } else {
            "coming_soon"
        };
        let created_day = if rng.chance(0.7) {
            rng.below((settings.days / 3).max(1) as usize) as i64
        } else {
            rng.below(settings.days.max(1) as usize) as i64
        };
        // Quality: mostly middling, some great, some bad.
        let quality = ((rng.f() + rng.f() + rng.f()) / 3.0 * 1.3 - 0.15).clamp(0.02, 0.98);
        let update = (stage == "released" && rng.chance(0.15)).then(|| {
            let day = created_day + (settings.days - created_day) / 2 + rng.below(10) as i64;
            let shift = if rng.chance(0.65) { 0.35 } else { -0.35 };
            (
                day.min(settings.days - 5),
                (quality + shift).clamp(0.02, 0.98),
            )
        });
        let a = rng.f();
        let min_age = if a < 0.7 {
            0
        } else if a < 0.8 {
            7
        } else if a < 0.88 {
            12
        } else if a < 0.95 {
            16
        } else {
            18
        };
        apps.push(SimApp {
            id: Uuid::now_v7(),
            slug,
            kind: persona.kind,
            theme,
            tags,
            stage,
            min_age,
            quality,
            update,
            created_day,
            dead: stage != "released" && rng.chance(0.25),
            pulse: persona.kind == AppKind::Game && rng.chance(0.7),
            maker: *rng.pick(&makers),
        });
    }

    // People: creatives and jobs. Work shipped in an update changes the app's true quality.
    let app_info: Vec<crate::sim_people::AppInfo> = apps
        .iter()
        .map(|a| crate::sim_people::AppInfo {
            kind: a.kind,
            stage: a.stage,
            created_day: a.created_day,
            quality: a.quality,
            update: a.update,
            maker: makers.iter().position(|m| *m == a.maker).unwrap_or(0),
        })
        .collect();
    let (people_plan, hired_updates) = crate::sim_people::plan(&mut rng, &app_info, settings.days);
    for (a, day, q) in &hired_updates {
        apps[*a].update = Some((*day, *q));
    }

    // Users: one or two personas; about a third are gamers who also use a few tools.
    let mut users: Vec<SimUser> = Vec::with_capacity(settings.users);
    for i in 0..settings.users {
        let r = rng.f();
        let personas = if r < 0.33 {
            let game = GAME_PERSONAS.start + rng.below(GAME_PERSONAS.len());
            let tool = TOOL_PERSONAS.start + rng.below(TOOL_PERSONAS.len());
            vec![(game, 1.0), (tool, 0.35)]
        } else if r < 0.7 {
            vec![(rng.below(PERSONAS.len()), 1.0)]
        } else {
            let a = rng.below(PERSONAS.len());
            let mut b = rng.below(PERSONAS.len());
            if b == a {
                b = (b + 1) % PERSONAS.len();
            }
            vec![(a, 1.0), (b, 0.6)]
        };
        let label: Vec<&str> = personas.iter().map(|(p, _)| PERSONAS[*p].name).collect();
        users.push(SimUser {
            id: Uuid::now_v7(),
            name: format!("Sim {} {}", i + 1, label.join(" + "))
                .chars()
                .take(40)
                .collect(),
            country: if rng.chance(0.1) { "GB" } else { "US" },
            taste: taste_of(&personas),
            personas,
            activity: 0.15 + 0.5 * rng.f(),
        });
    }

    // The days.
    let fits: Vec<Vec<f64>> = users
        .iter()
        .map(|u| apps.iter().map(|a| fit(&u.taste, &a.tags)).collect())
        .collect();
    let mut library: HashMap<usize, Vec<(usize, i64)>> = HashMap::new();
    let mut in_library: HashSet<(usize, usize)> = HashSet::new();
    let mut usage: HashMap<(usize, usize), (i64, i32, i64, i64)> = HashMap::new(); // playtime, days, first day, last day
    let mut current: HashMap<(usize, usize), (i16, i64)> = HashMap::new(); // vote, day
    let mut events: Vec<Event> = Vec::new();
    let mut library_count = vec![0usize; apps.len()];
    let mut players_by_day: HashMap<(usize, i64), i32> = HashMap::new();

    for day in 0..settings.days {
        let available: Vec<usize> = (0..apps.len())
            .filter(|&a| apps[a].created_day <= day)
            .collect();
        if available.is_empty() {
            continue;
        }
        for (u, user) in users.iter().enumerate() {
            if !rng.chance(user.activity) {
                continue;
            }
            // Find something: a handful of what's out there, likelier if it fits and is popular.
            let mine = library.get(&u).map_or(0, Vec::len);
            if mine == 0 || rng.chance(0.18) {
                let sample: Vec<usize> = (0..30).map(|_| *rng.pick(&available)).collect();
                let weights: Vec<f64> = sample
                    .iter()
                    .map(|&a| {
                        if in_library.contains(&(u, a)) {
                            0.0
                        } else {
                            (0.05 + fits[u][a]).powi(2)
                                * (1.0 + (1.0 + library_count[a] as f64).ln())
                        }
                    })
                    .collect();
                if let Some(k) = rng.weighted(&weights) {
                    let a = sample[k];
                    library.entry(u).or_default().push((a, day));
                    in_library.insert((u, a));
                    library_count[a] += 1;
                }
            }
            // Play one or two, more often the ones they enjoy.
            let playable: Vec<usize> = library
                .get(&u)
                .into_iter()
                .flatten()
                .map(|(a, _)| *a)
                .filter(|&a| apps[a].playable())
                .collect();
            if playable.is_empty() {
                continue;
            }
            let enjoy: Vec<f64> = playable
                .iter()
                .map(|&a| 0.1 + apps[a].quality_on(day) * 0.6 + fits[u][a])
                .collect();
            let mut played = HashSet::new();
            for _ in 0..1 + rng.below(2) {
                let Some(k) = rng.weighted(&enjoy) else { break };
                let a = playable[k];
                if !played.insert(a) {
                    continue;
                }
                let minutes = 3.0 + 80.0 * enjoy[k] * rng.f();
                let entry = usage.entry((u, a)).or_insert((0, 0, day, day));
                if apps[a].pulse {
                    entry.0 += (minutes * 60.0) as i64;
                }
                entry.1 += 1;
                entry.3 = day;
                *players_by_day.entry((a, day)).or_default() += 1;

                // Vote (released apps only), or think again after an update.
                if apps[a].stage != "released" {
                    continue;
                }
                let q = apps[a].quality_on(day);
                let truth = like_chance(q, fits[u][a]);
                let prev = current.get(&(u, a)).copied();
                let updated_since = match (prev, apps[a].update) {
                    (Some((_, voted)), Some((d, _))) => voted < d && day >= d,
                    _ => false,
                };
                let decide = match prev {
                    None => entry.1 >= 2 && rng.chance(0.2),
                    Some(_) => updated_since && rng.chance(0.3),
                };
                if !decide {
                    continue;
                }
                let vote: i16 = if rng.chance(truth) { 1 } else { -1 };
                let vote = if prev.is_some() && rng.chance(0.03) {
                    0
                } else {
                    vote
                };
                if prev.map(|p| p.0) == Some(vote) {
                    current.insert((u, a), (vote, day));
                    continue;
                }
                let (playtime, days) = (entry.0, entry.1);
                events.push(Event {
                    user: u,
                    app: a,
                    vote,
                    prev: prev.map(|p| p.0),
                    playtime,
                    days,
                    weight: vote_weight(playtime, days),
                    at: at(day, &mut rng),
                });
                if vote == 0 {
                    current.remove(&(u, a));
                } else {
                    current.insert((u, a), (vote, day));
                }
            }
        }
    }

    // Write it all.
    let mut tx = db.begin().await?;
    sqlx::query("update creatives set portfolio_set_id = null")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "delete from legal_acceptances where user_id in (select id from users where display_name like 'Sim %')",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("update apps set donations_on = false, runs_servers = false")
        .execute(&mut *tx)
        .await?;
    for table in [
        "notifications",
        "app_shares",
        "interests",
        "app_reviews",
        "reports",
        "moderation_decisions",
        "creative_reviews",
        "job_events",
        "payments",
        "donations",
        "server_donations",
        "donation_accounts",
        "job_payments",
        "jobs",
        "stripe_events",
        "portfolio_sets",
        "creatives",
        "vote_events",
        "votes",
        "pulse_sessions",
        "pulse_counts",
        "user_app_usage",
        "library_entries",
        "media_sets",
        "app_changes",
        "release_files",
        "releases",
        "app_tags",
        "review_notes",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!("delete from {table}")))
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("update apps set media_set_id = null, pinned_release_id = null")
        .execute(&mut *tx)
        .await?;
    sqlx::query("delete from apps").execute(&mut *tx).await?;
    sqlx::query("drop table if exists sim_users, sim_apps, sim_creatives")
        .execute(&mut *tx)
        .await?;
    sqlx::query("delete from sessions where user_id in (select id from users where display_name like 'Sim %' or display_name like 'Sim maker %')")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "delete from users where display_name like 'Sim %' or display_name like 'Sim maker %'",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "create table sim_users (user_id uuid primary key, personas text not null, taste jsonb not null, activity real not null)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "create table sim_apps (app_id uuid primary key, theme text not null, quality real not null,
                                update_day date, quality_after real)",
    )
    .execute(&mut *tx)
    .await?;

    let rows: Vec<(Uuid, String, &str)> = maker_rows
        .iter()
        .cloned()
        .chain(users.iter().map(|u| (u.id, u.name.clone(), u.country)))
        .collect();
    bulk(
        &mut tx,
        "insert into users (id, display_name, country_code, age_16_confirmed_at, created_at) ",
        &rows,
        |b, (id, name, country)| {
            b.push_bind(*id)
                .push_bind(name.clone())
                .push_bind(*country)
                .push_bind(start)
                .push_bind(start);
        },
    )
    .await?;

    let rules = crate::config::CatalogRules::default();
    bulk(
        &mut tx,
        "insert into apps (id, slug, title, summary, description_md, kind, stage, stage_entered_at, status, maker_id,
                           repo_url, license_spdx, min_age, last_commit_at, created_at, updated_at, platforms, watch_releases) ",
        &apps,
        |b, a| {
            let created = start + Duration::days(a.created_day);
            // Upcoming apps entered their stage recently enough not to have missed its deadline.
            let limit = if a.stage == "beta" { rules.beta_days } else { rules.coming_soon_days };
            let entered = if a.stage == "released" {
                created
            } else {
                created.max(now - Duration::days(i64::from(limit) - 5))
            };
            let last_commit = if a.dead { now - Duration::days(200) } else { now - Duration::days(a.created_day % 9) };
            let title = a.slug.split('-').map(capitalize).collect::<Vec<_>>().join(" ");
            let summary = format!("A {} {} made with AI.", PERSONAS[a.theme].name, if a.kind == AppKind::Game { "game" } else { "tool" });
            b.push_bind(a.id)
                .push_bind(a.slug.clone())
                .push_bind(title)
                .push_bind(summary)
                .push_bind(format!("Simulated app. Tags: {}.", a.tags.join(", ")))
                .push_bind(a.kind)
                .push("cast(")
                .push_bind_unseparated(a.stage)
                .push_unseparated(" as app_stage)")
                .push_bind(entered)
                .push("'listed'")
                .push_bind(a.maker)
                .push_bind(format!("https://github.com/sim/{}", a.slug))
                .push_bind("MIT")
                .push_bind(a.min_age)
                .push_bind(last_commit)
                .push_bind(created)
                .push_bind(created)
                .push_bind(vec!["windows".to_string()])
                .push_bind(false);
        },
    )
    .await?;
    let app_tags: Vec<(Uuid, String)> = apps
        .iter()
        .flat_map(|a| a.tags.iter().map(move |t| (a.id, t.clone())))
        .collect();
    for chunk in app_tags.chunks(5000) {
        let (ids, tags): (Vec<Uuid>, Vec<String>) = chunk.iter().cloned().unzip();
        sqlx::query(
            "insert into app_tags (app_id, tag_id)
             select x.app_id, t.id from unnest($1::uuid[], $2::text[]) as x(app_id, slug) join tags t on t.slug = x.slug
             on conflict do nothing",
        )
        .bind(ids)
        .bind(tags)
        .execute(&mut *tx)
        .await?;
    }

    let lib_rows: Vec<(Uuid, Uuid, OffsetDateTime)> = library
        .iter()
        .flat_map(|(u, entries)| {
            entries
                .iter()
                .map(|(a, d)| (users[*u].id, apps[*a].id, start + Duration::days(*d)))
                .collect::<Vec<_>>()
        })
        .collect();
    bulk(
        &mut tx,
        "insert into library_entries (user_id, app_id, added_at) ",
        &lib_rows,
        |b, (u, a, t)| {
            b.push_bind(*u).push_bind(*a).push_bind(*t);
        },
    )
    .await?;
    let usage_rows: Vec<_> = usage
        .iter()
        .map(|((u, a), (p, d, first, last))| {
            (
                users[*u].id,
                apps[*a].id,
                *p,
                *d,
                (start + Duration::days(*last)).date(),
                start + Duration::days(*first),
                start + Duration::days(*last),
            )
        })
        .collect();
    bulk(
        &mut tx,
        "insert into user_app_usage (user_id, app_id, playtime_seconds, days_launched, last_launch_day, first_launched_at, last_played_at) ",
        &usage_rows,
        |b, r| {
            b.push_bind(r.0).push_bind(r.1).push_bind(r.2).push_bind(r.3).push_bind(r.4).push_bind(r.5).push_bind(r.6);
        },
    )
    .await?;
    let event_rows: Vec<_> = events
        .iter()
        .map(|e| (Uuid::now_v7(), users[e.user].id, apps[e.app].id, e))
        .collect();
    bulk(
        &mut tx,
        "insert into vote_events (id, user_id, app_id, vote, prev_vote, release_version, playtime_seconds, days_launched, weight, created_at) ",
        &event_rows,
        |b, (id, u, a, e)| {
            b.push_bind(*id)
                .push_bind(*u)
                .push_bind(*a)
                .push_bind(e.vote)
                .push_bind(e.prev.filter(|p| *p != 0))
                .push_bind("sim")
                .push_bind(e.playtime)
                .push_bind(e.days)
                .push_bind(e.weight)
                .push_bind(e.at);
        },
    )
    .await?;
    // The current vote is each account's last event (withdrawals removed).
    sqlx::query(
        "insert into votes (user_id, app_id, vote, weight, updated_at)
         select user_id, app_id, vote, weight, created_at from (
             select distinct on (user_id, app_id) user_id, app_id, vote, weight, created_at
             from vote_events order by user_id, app_id, created_at desc, id desc) last
         where vote <> 0",
    )
    .execute(&mut *tx)
    .await?;

    // Player counts: each day's players spread over the day, busiest in the evening (UTC).
    let mut buckets: Vec<(Uuid, OffsetDateTime, i32)> = Vec::new();
    for ((a, day), n) in &players_by_day {
        if !apps[*a].pulse {
            continue;
        }
        for hour in (0..24).step_by(3) {
            let curve = 0.25
                + 0.75 * (((hour as f64 - 19.0) / 24.0 * std::f64::consts::TAU).cos() * 0.5 + 0.5);
            let players = (f64::from(*n) * 0.4 * curve).round() as i32;
            if players > 0 {
                let bucket = start + Duration::days(*day) + Duration::hours(hour);
                buckets.push((apps[*a].id, bucket, players));
            }
        }
    }
    bulk(
        &mut tx,
        "insert into pulse_counts (app_id, bucket, players) ",
        &buckets,
        |b, (a, t, n)| {
            b.push_bind(*a).push_bind(*t).push_bind(*n);
        },
    )
    .await?;

    let truth_users: Vec<_> = users
        .iter()
        .map(|u| {
            let names: Vec<String> = u
                .personas
                .iter()
                .map(|(p, w)| format!("{}:{w}", PERSONAS[*p].name))
                .collect();
            (
                u.id,
                names.join(","),
                sqlx::types::Json(u.taste.clone()),
                u.activity as f32,
            )
        })
        .collect();
    bulk(
        &mut tx,
        "insert into sim_users (user_id, personas, taste, activity) ",
        &truth_users,
        |b, r| {
            b.push_bind(r.0)
                .push_bind(r.1.clone())
                .push_bind(r.2.clone())
                .push_bind(r.3);
        },
    )
    .await?;
    let truth_apps: Vec<_> = apps
        .iter()
        .map(|a| {
            (
                a.id,
                PERSONAS[a.theme].name,
                a.quality as f32,
                a.update.map(|(d, _)| (start + Duration::days(d)).date()),
                a.update.map(|(_, q)| q as f32),
            )
        })
        .collect();
    bulk(
        &mut tx,
        "insert into sim_apps (app_id, theme, quality, update_day, quality_after) ",
        &truth_apps,
        |b, r| {
            b.push_bind(r.0)
                .push_bind(r.1)
                .push_bind(r.2)
                .push_bind(r.3)
                .push_bind(r.4);
        },
    )
    .await?;
    tx.commit().await?;

    let updates: Vec<(usize, i64)> = apps
        .iter()
        .enumerate()
        .filter_map(|(i, a)| a.update.map(|(d, _)| (i, d)))
        .collect();
    let people = crate::sim_people::write(
        db,
        &mut rng,
        &people_plan,
        &apps.iter().map(|a| a.id).collect::<Vec<_>>(),
        &apps
            .iter()
            .map(|a| {
                a.slug
                    .split('-')
                    .map(capitalize)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>(),
        &apps.iter().map(|a| a.slug.clone()).collect::<Vec<_>>(),
        &maker_rows,
        &app_info.iter().map(|a| a.maker).collect::<Vec<_>>(),
        &updates,
        start,
        now,
    )
    .await?;
    // Interested: players mark upcoming apps that share at least two tags with their library,
    // about one in six of them (the same ones for the same seed of users and apps).
    let interests = sqlx::query(
        "insert into interests (user_id, app_id, created_at)
         select u.user_id, a.id, a.stage_entered_at + (abs(hashtext(u.user_id::text || a.id::text)) % 20) * interval '1 day' / 2
         from sim_users u cross join apps a
         where a.stage <> 'released' and a.status = 'listed'
           and abs(hashtext(a.id::text || u.user_id::text)) % 6 = 0
           and (select count(*) from app_tags at where at.app_id = a.id and at.tag_id in
                  (select at2.tag_id from library_entries l join app_tags at2 on at2.app_id = l.app_id
                   where l.user_id = u.user_id)) >= 2
         on conflict do nothing",
    )
    .execute(db)
    .await?
    .rows_affected();
    // Written reviews: about one vote in seven, in a few voices for each side.
    let reviews = sqlx::query(
        "insert into app_reviews (id, user_id, app_id, body, created_at, updated_at)
         select gen_random_uuid(), v.user_id, v.app_id,
                case when v.vote = 1 then (array[
                    'Exactly what I wanted. Runs well and I keep coming back to it.',
                    'Small, but every part of it feels made with care.',
                    'Surprised how polished this is. The last update fixed my one complaint.',
                    'Great for short sessions. Wish there was more of it!'
                  ])[1 + abs(hashtext(v.user_id::text)) % 4]
                else (array[
                    'Feels unfinished: menus are confusing and it crashed twice.',
                    'Looks nice in the screenshots, but there is not much to do.',
                    'Same three ideas over and over. Not for me.',
                    'Performance is rough on my laptop, and the controls fight you.'
                  ])[1 + abs(hashtext(v.user_id::text)) % 4] end,
                v.updated_at, v.updated_at
         from votes v join sim_users s on s.user_id = v.user_id
         where abs(hashtext(v.user_id::text || v.app_id::text)) % 7 = 0
         on conflict do nothing",
    )
    .execute(db)
    .await?
    .rows_affected();
    let branches = sim_branches(db, now).await?;
    // Private Slop: a few makers keep a tool to themselves and share it with a handful of players.
    let private_apps: Vec<Uuid> = sqlx::query_scalar(
        "update apps set visibility = 'private', status = 'draft'
         where id in (select id from apps where status = 'listed' and stage = 'released' and parent_app_id is null
                      and kind = 'app' order by md5(id::text) desc limit 6)
         returning id",
    )
    .fetch_all(db)
    .await?;
    for app in &private_apps {
        sqlx::query(
            "insert into app_shares (app_id, user_id)
             select $1, user_id from sim_users order by md5(user_id::text || $1::text) limit 4
             on conflict do nothing",
        )
        .bind(app)
        .execute(db)
        .await?;
    }
    let moderation = crate::sim_moderation::write(db, &mut rng, now).await?;
    let donations = crate::sim_donations::write(db, &mut rng, now).await?;

    Ok(Generated {
        apps: apps.len(),
        users: users.len(),
        makers: makers.len(),
        library_entries: lib_rows.len(),
        launched: usage_rows.len(),
        vote_events: events.len(),
        current_votes: current.len(),
        player_buckets: buckets.len(),
        people,
        moderation,
        interests,
        donations,
        reviews,
        branches,
        private_apps: private_apps.len(),
    })
}

/// Community branches: ports of Windows-only games to Linux, and a few mods and translations,
/// each by another simulated maker, with an approved release and some votes.
async fn sim_branches(db: &PgPool, now: OffsetDateTime) -> anyhow::Result<usize> {
    let parents: Vec<(Uuid, String, String, Uuid)> = sqlx::query_as(
        "select a.id, a.slug, a.title, a.maker_id from apps a
         where a.status = 'listed' and a.stage = 'released' and a.parent_app_id is null
         order by md5(a.id::text) limit 14",
    )
    .fetch_all(db)
    .await?;
    let makers: Vec<Uuid> = sqlx::query_scalar(
        "select id from users where display_name like 'Sim maker %' order by id",
    )
    .fetch_all(db)
    .await?;
    let mut made = 0;
    for (i, (parent, slug, title, owner)) in parents.iter().enumerate() {
        let author = makers[(i * 5 + 3) % makers.len().max(1)];
        if author == *owner {
            continue;
        }
        let (kind, suffix, name, platforms, note): (&str, &str, String, Vec<&str>, &str) =
            match i % 4 {
                0 | 1 => (
                    "port",
                    "linux",
                    format!("{title} (Linux port)"),
                    vec!["linux", "linux-deb"],
                    "A native Linux build with a .deb package.",
                ),
                2 => (
                    "mod",
                    "plus",
                    format!("{title}+"),
                    vec!["windows"],
                    "Adds a hard mode and a level editor.",
                ),
                _ => (
                    "translation",
                    "de",
                    format!("{title} (Deutsch)"),
                    vec!["windows"],
                    "German text and menus.",
                ),
            };
        let id = Uuid::now_v7();
        sqlx::query(
            "insert into apps (id, slug, title, summary, description_md, kind, stage, status, maker_id, repo_url, license_spdx,
                               min_age, platforms, parent_app_id, branch_kind, branch_note, created_at, updated_at, last_commit_at)
             select $1, $2 || '-' || $3, $4, $5, $5, p.kind, 'released', 'listed', $6, 'https://github.com/sim-fork/' || $2,
                    p.license_spdx, p.min_age, $7, p.id, $8, $5, $9, $9, $9
             from apps p where p.id = $10",
        )
        .bind(id)
        .bind(slug)
        .bind(suffix)
        .bind(&name)
        .bind(note)
        .bind(author)
        .bind(&platforms)
        .bind(kind)
        .bind(now - time::Duration::days(20 + i as i64))
        .bind(parent)
        .execute(db)
        .await?;
        sqlx::query("insert into app_tags (app_id, tag_id) select $1, tag_id from app_tags where app_id = $2")
            .bind(id)
            .bind(parent)
            .execute(db)
            .await?;
        sqlx::query(
            "insert into releases (id, app_id, version, commit_sha, status, submitted_by, created_at, reviewed_at, commit_verified)
             values ($1, $2, '1.0.0', repeat('f', 40), 'approved', $3, $4, $4, true)",
        )
        .bind(Uuid::now_v7())
        .bind(id)
        .bind(author)
        .bind(now - time::Duration::days(15 + i as i64))
        .execute(db)
        .await?;
        // Some of the original's players try it and vote.
        sqlx::query(
            "insert into votes (user_id, app_id, vote, weight, updated_at)
             select v.user_id, $1, case when abs(hashtext(v.user_id::text || $3)) % 5 = 0 then -1 else 1 end, 0.6, $2
             from votes v where v.app_id = $4 and abs(hashtext(v.user_id::text)) % 3 = 0
             on conflict do nothing",
        )
        .bind(id)
        .bind(now - time::Duration::days(3))
        .bind(suffix)
        .bind(parent)
        .execute(db)
        .await?;
        made += 1;
    }
    Ok(made)
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// Inserts rows in chunks that stay under Postgres's limit on bind parameters.
async fn bulk<T>(
    tx: &mut sqlx::PgConnection,
    head: &'static str,
    rows: &[T],
    mut push: impl FnMut(&mut sqlx::query_builder::Separated<'_, Postgres, &'static str>, &T),
) -> anyhow::Result<()> {
    for chunk in rows.chunks(2000) {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(head);
        qb.push_values(chunk, |mut b, row| push(&mut b, row));
        qb.build()
            .execute(&mut *tx)
            .await
            .with_context(|| head.to_string())?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Live players

/// Makes "playing now" look like a real evening: replaces the sandbox's live Pulse sessions with
/// a fresh set, sized from each game's recent daily players and the time of day. Returns how
/// many sessions each app got, so the report can check what the real code counts.
pub async fn pulse_tick(db: &PgPool, rng: &mut Rng) -> anyhow::Result<HashMap<Uuid, i64>> {
    ensure_sandbox(db).await?;
    let hour = OffsetDateTime::now_utc().hour() as f64;
    let curve = 0.25 + 0.75 * (((hour - 19.0) / 24.0 * std::f64::consts::TAU).cos() * 0.5 + 0.5);
    let recent: Vec<(Uuid, f64)> = sqlx::query_as(
        "select app_id, avg(peak)::float8 from (
             select app_id, date_trunc('day', bucket) d, max(players) peak from pulse_counts
             where bucket > now() - interval '7 days' group by 1, 2) x group by app_id",
    )
    .fetch_all(db)
    .await?;
    let mut tx = db.begin().await?;
    sqlx::query("delete from pulse_sessions")
        .execute(&mut *tx)
        .await?;
    let mut live = HashMap::new();
    for (app, avg) in recent {
        let want = (avg * curve * (0.8 + 0.4 * rng.f())).round() as i64;
        if want == 0 {
            continue;
        }
        let players: Vec<Uuid> = sqlx::query_scalar(
            "select user_id from user_app_usage where app_id = $1 order by md5(user_id::text || $2) limit $3",
        )
        .bind(app)
        .bind(rng.next_u64().to_string())
        .bind(want)
        .fetch_all(&mut *tx)
        .await?;
        for user in &players {
            sqlx::query(
                "insert into pulse_sessions (id, token_hash, app_id, user_id, issued_at, started_at, last_beat_at, beats)
                 values ($1, $2, $3, $4, now() - interval '20 minutes', now() - interval '19 minutes', now(), 19)",
            )
            .bind(Uuid::now_v7())
            .bind(rng.next_u64().to_le_bytes().repeat(4))
            .bind(app)
            .bind(user)
            .execute(&mut *tx)
            .await?;
        }
        live.insert(app, players.len() as i64);
    }
    tx.commit().await?;
    crate::pulse::sample(db).await?;
    Ok(live)
}

// ---------------------------------------------------------------------------------------------
// The report

#[derive(Debug, Default, Serialize)]
pub struct SlateStats {
    pub users: usize,
    /// Average truth chance the user would vote 👍 on what's shown.
    pub like_chance: f64,
    /// Share of cards the user would probably like (truth chance ≥ 50%).
    pub hit_rate: f64,
    /// Average taste fit (truth, 0..1).
    pub fit: f64,
    /// Average 1 − tag overlap between cards in one slate (higher: more varied).
    pub variety: f64,
    /// Average popularity percentile of what's shown (lower: more long tail).
    pub popularity_percentile: f64,
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub database: String,
    pub apps: i64,
    pub users: i64,
    /// For you, for users with personal recommendations.
    pub for_you: SlateStats,
    /// The non-personal feed (popular, rated, new) for the same users: the baseline.
    pub popular_baseline: SlateStats,
    /// Random eligible apps for the same users: the floor.
    pub random_baseline: SlateStats,
    /// Share of For you cards that are discoveries.
    pub discovery_share: f64,
    /// Truth fit of matches vs discoveries: discoveries should fit less, but still be liked often.
    pub match_fit: f64,
    pub discovery_fit: f64,
    pub discovery_like_chance: f64,
    /// Share of discoveries whose tags are mostly new to the user.
    pub discovery_novel_tags: f64,
    /// Distinct apps shown to anyone / eligible apps.
    pub catalog_coverage: f64,
    /// Gamers who also use a few tools: share of their cards that are tools (truth: their taste
    /// is about 1/4 tools).
    pub gamer_with_tools_tool_share: f64,
    pub gamer_with_tools_users: usize,
    /// Roguelike fans' share of roguelike cards, before and after 👎 on three roguelikes.
    pub dislike_roguelike_share_before: f64,
    pub dislike_roguelike_share_after: f64,
    /// Gamers' share of tool cards before and after 👍 on two tools.
    pub like_tool_share_before: f64,
    pub like_tool_share_after: f64,
    /// Users in the under-18 view: their feed isn't personal by default (PRIV-6).
    pub under_18_users: usize,
    pub under_18_personal: usize,
    pub under_18_shown_18_plus: usize,
    /// Must all be 0.
    pub shown_dead: usize,
    pub shown_in_library_or_played: usize,
    pub shown_own_apps: usize,
    /// Ratings: apps whose current tally matches a recount from the raw events.
    pub ratings_checked: usize,
    pub ratings_matching: usize,
    /// Apps whose latest monthly rating equals the current rating.
    pub monthly_matching_now: usize,
    /// Updated apps: the rating moved the way the update moved quality (before vs after).
    pub updates_checked: usize,
    pub updates_moved_right_way: usize,
    /// How well % Not Slop tracks the truth: correlation across rated apps.
    pub rating_truth_correlation: f64,
    /// Players: apps whose "playing now" equals the live sessions made.
    pub players_checked: usize,
    pub players_matching: usize,
    pub players_now_total: i64,
    pub apps_with_30_day_history: usize,
    /// People: creatives, jobs, payments, credits and "since the new work" (sim_people.rs).
    pub people: crate::sim_people::PeopleReport,
    /// Reports, decisions and appeals (sim_moderation.rs).
    pub moderation: crate::sim_moderation::ModerationReport,
    /// Donations through the real amounts (sim_donations.rs).
    pub donations: crate::sim_donations::DonationsReport,
}

struct Truth {
    taste: HashMap<String, f64>,
}

/// One card as the truth sees it: (like chance, fit, tags, popularity percentile).
type TruthCard = (f64, f64, Vec<String>, f64);

fn stats(slates: &[Vec<TruthCard>]) -> SlateStats {
    // (like chance, fit, tags, popularity percentile)
    let cards: Vec<&TruthCard> = slates.iter().flatten().collect();
    let n = cards.len().max(1) as f64;
    let mut variety = 0.0;
    let mut pairs = 0.0;
    for s in slates {
        for i in 0..s.len() {
            for j in i + 1..s.len() {
                let (a, b) = (&s[i].2, &s[j].2);
                let inter = a.iter().filter(|t| b.contains(t)).count() as f64;
                let union = (a.len() + b.len()) as f64 - inter;
                variety += if union == 0.0 {
                    1.0
                } else {
                    1.0 - inter / union
                };
                pairs += 1.0;
            }
        }
    }
    SlateStats {
        users: slates.len(),
        like_chance: cards.iter().map(|c| c.0).sum::<f64>() / n,
        hit_rate: cards.iter().filter(|c| c.0 >= 0.5).count() as f64 / n,
        fit: cards.iter().map(|c| c.1).sum::<f64>() / n,
        variety: if pairs == 0.0 { 0.0 } else { variety / pairs },
        popularity_percentile: cards.iter().map(|c| c.3).sum::<f64>() / n,
    }
}

/// A sampled user: id, personas, taste, country.
type SampledUser = (
    Uuid,
    String,
    sqlx::types::Json<HashMap<String, f64>>,
    String,
);

/// Scores the real code against the simulated truth. `sample`: how many users to look at.
pub async fn report(
    db: &PgPool,
    config: &Config,
    sample: usize,
    seed: u64,
) -> anyhow::Result<Report> {
    let database = ensure_sandbox(db).await?;
    let mut rng = Rng::new(seed ^ 0xA11CE);
    let mut r = Report {
        database,
        ..Default::default()
    };
    r.apps = sqlx::query_scalar("select count(*) from sim_apps")
        .fetch_one(db)
        .await?;
    r.users = sqlx::query_scalar("select count(*) from sim_users")
        .fetch_one(db)
        .await?;

    let quality: HashMap<Uuid, f64> = sqlx::query_as::<_, (Uuid, f32, Option<f32>)>(
        "select app_id, quality, quality_after from sim_apps",
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|(a, q, after)| (a, f64::from(after.unwrap_or(q))))
    .collect();
    let truths: Vec<SampledUser> = sqlx::query_as(
        "select s.user_id, s.personas, s.taste, u.country_code::text from sim_users s join users u on u.id = s.user_id
         order by md5(s.user_id::text || $1) limit $2",
    )
    .bind(seed.to_string())
    .bind(sample as i64)
    .fetch_all(db)
    .await?;
    let names = foryou::tag_names(db).await?;
    let full = foryou::candidates(db, config, ViewerPolicy::FULL_VIEW).await?;
    let under18 = foryou::candidates(db, config, ViewerPolicy::UNDER_18_VIEW).await?;
    let mut pop_rank: Vec<(Uuid, i32)> = full.iter().map(|c| (c.id, c.players_week)).collect();
    pop_rank.sort_by_key(|p| p.1);
    let percentile: HashMap<Uuid, f64> = pop_rank
        .iter()
        .enumerate()
        .map(|(i, (id, _))| (*id, i as f64 / pop_rank.len().max(1) as f64))
        .collect();
    let eligible_ids: HashSet<Uuid> = full
        .iter()
        .filter(|c| !c.app.is_dead)
        .map(|c| c.id)
        .collect();
    let mut shown: HashSet<Uuid> = HashSet::new();

    let mut fy = Vec::new();
    let mut popular = Vec::new();
    let mut random = Vec::new();
    let (mut disc_cards, mut all_cards, mut match_fit, mut disc_fit, mut disc_like, mut disc_novel) =
        (0usize, 0usize, 0.0, 0.0, 0.0, 0usize);
    let mut match_cards = 0usize;
    let (mut gwt_tools, mut gwt_cards) = (0usize, 0usize);

    for (user_id, personas, taste, country) in &truths {
        let truth = Truth {
            taste: taste.0.clone(),
        };
        let signals = foryou::signals(db, *user_id).await?;
        let under = country == "GB";
        let policy = if under {
            ViewerPolicy::UNDER_18_VIEW
        } else {
            ViewerPolicy::FULL_VIEW
        };
        let cands = if under { &under18 } else { &full };
        let personal = foryou::personalized(db, *user_id, policy).await?;
        let picks = foryou::recommend(
            &signals,
            cands,
            &names,
            personal,
            rng.next_u64(),
            foryou::SLATE,
        );

        let card = |id: Uuid, tags: &[String]| {
            let f = fit(&truth.taste, tags);
            (
                like_chance(quality.get(&id).copied().unwrap_or(0.5), f),
                f,
                tags.to_vec(),
                percentile.get(&id).copied().unwrap_or(0.0),
            )
        };
        for p in &picks {
            let c = &cands[p.index];
            if c.app.is_dead {
                r.shown_dead += 1;
            }
            if signals.library.contains(&c.id)
                || signals.launched.contains_key(&c.id)
                || signals.votes.contains_key(&c.id)
            {
                r.shown_in_library_or_played += 1;
            }
            if Some(c.maker_id) == signals.user_id {
                r.shown_own_apps += 1;
            }
        }
        if under {
            r.under_18_users += 1;
            if picks.iter().any(|p| p.kind != PickKind::Popular) {
                r.under_18_personal += 1;
            }
            r.under_18_shown_18_plus += picks
                .iter()
                .filter(|p| cands[p.index].app.min_age >= 18)
                .count();
            continue;
        }
        if !picks.iter().any(|p| p.kind != PickKind::Popular) {
            continue; // no activity yet
        }

        let slate: Vec<_> = picks
            .iter()
            .map(|p| card(cands[p.index].id, &cands[p.index].app.tags))
            .collect();
        let mine: HashSet<String> = signals
            .library
            .iter()
            .chain(signals.launched.keys())
            .filter_map(|id| cands.iter().find(|c| c.id == *id))
            .flat_map(|c| c.app.tags.clone())
            .collect();
        for (p, c) in picks.iter().zip(&slate) {
            shown.insert(cands[p.index].id);
            all_cards += 1;
            match p.kind {
                PickKind::Discover => {
                    disc_cards += 1;
                    disc_fit += c.1;
                    disc_like += c.0;
                    let tags = &cands[p.index].app.tags;
                    if tags.iter().filter(|t| !mine.contains(*t)).count() * 2 >= tags.len() {
                        disc_novel += 1;
                    }
                }
                _ => {
                    match_cards += 1;
                    match_fit += c.1;
                }
            }
        }
        let is_gwt = personas.split(',').count() == 2 && personas.contains(":0.35");
        if is_gwt {
            gwt_cards += picks.len();
            gwt_tools += picks
                .iter()
                .filter(|p| cands[p.index].app.kind == AppKind::App)
                .count();
        }
        fy.push(slate);

        let base = foryou::recommend(
            &signals,
            cands,
            &names,
            false,
            rng.next_u64(),
            foryou::SLATE,
        );
        popular.push(
            base.iter()
                .map(|p| card(cands[p.index].id, &cands[p.index].app.tags))
                .collect(),
        );
        let pool: Vec<&foryou::Candidate> = cands
            .iter()
            .filter(|c| {
                !c.app.is_dead
                    && !signals.library.contains(&c.id)
                    && !signals.launched.contains_key(&c.id)
            })
            .collect();
        random.push(
            (0..foryou::SLATE.min(pool.len()))
                .map(|_| {
                    let c = rng.pick(&pool);
                    card(c.id, &c.app.tags)
                })
                .collect(),
        );
    }
    r.for_you = stats(&fy);
    r.popular_baseline = stats(&popular);
    r.random_baseline = stats(&random);
    r.discovery_share = disc_cards as f64 / all_cards.max(1) as f64;
    r.match_fit = match_fit / match_cards.max(1) as f64;
    r.discovery_fit = disc_fit / disc_cards.max(1) as f64;
    r.discovery_like_chance = disc_like / disc_cards.max(1) as f64;
    r.discovery_novel_tags = disc_novel as f64 / disc_cards.max(1) as f64;
    r.catalog_coverage = shown.len() as f64 / eligible_ids.len().max(1) as f64;
    r.gamer_with_tools_users = fy.len().min(gwt_cards / foryou::SLATE.max(1));
    r.gamer_with_tools_tool_share = gwt_tools as f64 / gwt_cards.max(1) as f64;

    // Experiments on the real recommender: what a 👎 or 👍 does.
    let fans: Vec<Uuid> = sqlx::query_scalar(
        "select s.user_id from sim_users s join users u on u.id = s.user_id
         where s.personas like 'roguelike fan:1%' and u.country_code = 'US' limit 40",
    )
    .fetch_all(db)
    .await?;
    let share = |picks: &[foryou::Pick], test: &dyn Fn(&foryou::Candidate) -> bool| {
        picks.iter().filter(|p| test(&full[p.index])).count() as f64 / picks.len().max(1) as f64
    };
    let is_rogue = |c: &foryou::Candidate| c.app.tags.iter().any(|t| t == "roguelike");
    let (mut before, mut after, mut n) = (0.0f64, 0.0f64, 0.0f64);
    for user in &fans {
        let mut s = foryou::signals(db, *user).await?;
        // The experiment is about votes: Interested apps (left out of the feed) would hide the
        // upcoming roguelikes it measures.
        s.interested.clear();
        let seed = rng.next_u64();
        let picks = foryou::recommend(&s, &full, &names, true, seed, foryou::SLATE);
        let disliked: Vec<Uuid> = full
            .iter()
            .filter(|c| is_rogue(c) && !s.votes.contains_key(&c.id))
            .filter(|c| s.library.contains(&c.id) || s.launched.contains_key(&c.id))
            .take(3)
            .map(|c| c.id)
            .collect();
        if disliked.len() < 3 {
            continue;
        }
        for id in disliked {
            s.votes.insert(id, -1);
        }
        let picks_after = foryou::recommend(&s, &full, &names, true, seed, foryou::SLATE);
        before += share(&picks, &is_rogue);
        after += share(&picks_after, &is_rogue);
        n += 1.0;
    }
    r.dislike_roguelike_share_before = before / n.max(1.0);
    r.dislike_roguelike_share_after = after / n.max(1.0);

    let gamers: Vec<Uuid> = sqlx::query_scalar(
        "select s.user_id from sim_users s join users u on u.id = s.user_id
         where s.personas not like '%,%' and s.personas similar to '(roguelike fan|cozy gamer|strategist|action fan|puzzler)%'
           and u.country_code = 'US' limit 40",
    )
    .fetch_all(db)
    .await?;
    let tools: Vec<&foryou::Candidate> = full
        .iter()
        .filter(|c| {
            c.app.kind == AppKind::App
                && c.app
                    .tags
                    .iter()
                    .any(|t| t == "developer-tools" || t == "automation")
        })
        .collect();
    let is_tool = |c: &foryou::Candidate| c.app.kind == AppKind::App;
    let (mut before, mut after, mut n) = (0.0f64, 0.0f64, 0.0f64);
    for user in &gamers {
        let mut s = foryou::signals(db, *user).await?;
        let seed = rng.next_u64();
        let picks = foryou::recommend(&s, &full, &names, true, seed, foryou::SLATE);
        let liked: Vec<Uuid> = tools
            .iter()
            .filter(|c| !s.library.contains(&c.id))
            .take(2)
            .map(|c| c.id)
            .collect();
        for id in liked {
            s.votes.insert(id, 1);
            s.launched.insert(id, (0, 3));
        }
        let picks_after = foryou::recommend(&s, &full, &names, true, seed, foryou::SLATE);
        before += share(&picks, &is_tool);
        after += share(&picks_after, &is_tool);
        n += 1.0;
    }
    r.like_tool_share_before = before / n.max(1.0);
    r.like_tool_share_after = after / n.max(1.0);

    // Ratings: the real numbers against a recount from the raw events.
    let rated: Vec<Uuid> = sqlx::query_scalar("select distinct app_id from vote_events")
        .fetch_all(db)
        .await?;
    let mut pairs = Vec::new();
    for app in &rated {
        let t = votes::tally(db, *app).await?;
        let (up, down, upw, downw): (i64, i64, f64, f64) = sqlx::query_as(
            "select count(*) filter (where vote = 1), count(*) filter (where vote = -1),
                    coalesce(sum(weight) filter (where vote = 1), 0)::float8, coalesce(sum(weight) filter (where vote = -1), 0)::float8
             from (select distinct on (user_id) vote, weight from vote_events where app_id = $1
                   order by user_id, created_at desc, id desc) last",
        )
        .bind(app)
        .fetch_one(db)
        .await?;
        r.ratings_checked += 1;
        if t.not_slop == up
            && t.slop == down
            && (t.not_slop_weight - upw).abs() < 1e-3
            && (t.slop_weight - downw).abs() < 1e-3
        {
            r.ratings_matching += 1;
        }
        let months = votes::monthly(db, *app).await?;
        if months
            .last()
            .is_some_and(|m| m.tally.not_slop == t.not_slop && m.tally.slop == t.slop)
        {
            r.monthly_matching_now += 1;
        }
        if let (Some(p), Some(q)) = (t.percent_weighted, quality.get(app)) {
            pairs.push((f64::from(p), *q));
        }
    }
    r.rating_truth_correlation = correlation(&pairs);

    let updated: Vec<(Uuid, time::Date, f32, f32)> = sqlx::query_as(
        "select app_id, update_day, quality, quality_after from sim_apps where update_day is not null",
    )
    .fetch_all(db)
    .await?;
    for (app, day, q0, q1) in updated {
        let rates: (Option<f64>, Option<f64>) = sqlx::query_as(
            "with last as (
                 select distinct on (user_id, created_at >= $2) user_id, created_at >= $2 as after, vote, weight
                 from vote_events where app_id = $1 and vote <> 0
                 order by user_id, created_at >= $2, created_at desc)
             select (sum(weight) filter (where vote = 1 and not after) / nullif(sum(weight) filter (where not after), 0))::float8,
                    (sum(weight) filter (where vote = 1 and after) / nullif(sum(weight) filter (where after), 0))::float8
             from last",
        )
        .bind(app)
        .bind(day.midnight().assume_utc())
        .fetch_one(db)
        .await?;
        if let (Some(b), Some(a)) = rates {
            r.updates_checked += 1;
            if (a - b).signum() == f64::from(q1 - q0).signum() {
                r.updates_moved_right_way += 1;
            }
        }
    }

    // Players: make a live evening, then ask the real code.
    let live = pulse_tick(db, &mut rng).await?;
    for (app, made) in &live {
        let p = crate::pulse::player_stats(db, *app).await?;
        r.players_checked += 1;
        r.players_now_total += p.playing_now;
        if p.playing_now == *made && p.peak_this_month >= p.playing_now {
            r.players_matching += 1;
        }
    }
    r.apps_with_30_day_history = sqlx::query_scalar::<_, i64>(
        "select count(*) from (select app_id from pulse_counts where bucket > now() - interval '30 days'
                               group by app_id having count(distinct date_trunc('day', bucket)) >= 20) x",
    )
    .fetch_one(db)
    .await? as usize;
    r.people = crate::sim_people::report(db).await?;
    r.moderation = crate::sim_moderation::report(db).await?;
    r.donations = crate::sim_donations::report(db).await?;
    Ok(r)
}

pub fn correlation(pairs: &[(f64, f64)]) -> f64 {
    let n = pairs.len() as f64;
    if n < 3.0 {
        return 0.0;
    }
    let (mx, my) = (
        pairs.iter().map(|p| p.0).sum::<f64>() / n,
        pairs.iter().map(|p| p.1).sum::<f64>() / n,
    );
    let cov: f64 = pairs.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let vx: f64 = pairs.iter().map(|p| (p.0 - mx).powi(2)).sum();
    let vy: f64 = pairs.iter().map(|p| (p.1 - my).powi(2)).sum();
    if vx == 0.0 || vy == 0.0 {
        0.0
    } else {
        cov / (vx.sqrt() * vy.sqrt())
    }
}

// ---------------------------------------------------------------------------------------------
// "View as a simulated user", on the sandbox site only (SLOP_SIMULATION)

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/sim/users", get(sim_users))
        .route("/sim/login/{id}", post(sim_login))
}

#[derive(Serialize, sqlx::FromRow)]
struct SimUserRow {
    id: Uuid,
    display_name: String,
    personas: String,
    country_code: String,
    library: i64,
    votes: i64,
}

async fn sim_users(State(state): State<AppState>) -> ApiResult<Json<Vec<SimUserRow>>> {
    if !state.config.simulation {
        return Err(ApiError::NotFound);
    }
    Ok(Json(
        sqlx::query_as(
            "select id, display_name, personas, country_code, library, votes from (
                 select u.id, u.display_name, s.personas, u.country_code::text,
                        (select count(*) from library_entries l where l.user_id = u.id) as library,
                        (select count(*) from votes v where v.user_id = u.id) as votes,
                        row_number() over (partition by s.personas order by u.country_code = 'GB', u.display_name) as n
                 from sim_users s join users u on u.id = s.user_id) x
             where n <= 3
             union all
             -- Makers and creatives, to try People jobs from either side.
             select u.id, u.display_name || coalesce(' (' || c.name || ')', ''),
                    case when c.user_id is not null then 'People: creative' else 'People: maker' end,
                    u.country_code::text,
                    (select count(*) from jobs j where j.creative_id = u.id or j.client_id = u.id),
                    0
             from users u left join creatives c on c.user_id = u.id
             where u.display_name like 'Sim maker %' or (c.user_id is not null and u.display_name like 'Sim creative %')
             order by personas, display_name",
        )
        .fetch_all(&state.db)
        .await?,
    ))
}

async fn sim_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
    Path(id): Path<Uuid>,
) -> ApiResult<(StatusCode, CookieJar)> {
    if !state.config.simulation {
        return Err(ApiError::NotFound);
    }
    let exists: bool =
        sqlx::query_scalar(
            "select exists (select 1 from sim_users where user_id = $1)
                 or exists (select 1 from users where id = $1 and (display_name like 'Sim maker %' or display_name like 'Sim creative %'))",
        )
            .bind(id)
            .fetch_one(&state.db)
            .await?;
    if !exists {
        return Err(ApiError::NotFound);
    }
    let ua = headers.get(USER_AGENT).and_then(|v| v.to_str().ok());
    let cookie = session::create(&state.db, &state.config, id, ua).await?;
    Ok((StatusCode::NO_CONTENT, jar.add(cookie)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truth_likes_quality_and_fit() {
        assert!(like_chance(0.9, 0.9) > 0.9);
        assert!(like_chance(0.1, 0.1) < 0.1);
        assert!(like_chance(0.5, 0.8) > like_chance(0.5, 0.1));
        let mut rng = Rng::new(1);
        let mut again = Rng::new(1);
        assert_eq!(rng.next_u64(), again.next_u64(), "repeatable");
        assert_eq!(rng.weighted(&[0.0, 0.0, 5.0]), Some(2));
    }
}
