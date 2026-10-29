// SPDX-License-Identifier: AGPL-3.0-or-later
//! Discover (the store bar's tab, `GET /v1/discover`, DESIGN §5): one page of reels, each a
//! sideways row of up to 12 apps with a title and a plain-words `why` (the main parameters of
//! the recommendation, DSA Art. 27).
//!
//! **With personal recommendations on** (foryou::personalized: the account's setting; otherwise
//! on, except after Global Privacy Control or in the UK under-18 view):
//!
//! 1. **Recommended for you**, always first: the For you feed (foryou.rs), unchanged.
//! 2. **Your genres**: up to four tags, one from each of these signals in turn, never the same
//!    tag twice, and never a tag their overall taste is against (foryou::taste, e.g. 👎 votes):
//!    - *play*: playtime and launch days, by each app's vote weight (votes::vote_weight), at
//!      least half an hour's worth: "Because you play a lot of Roguelike";
//!    - *like*: 👍 Slaps votes (minus 👎) and the Interested list: "Because you like Cozy";
//!    - *library*: apps in their library (added on the website, or installed with the desktop
//!      app while signed in; SLAPorSLOP keeps no per-account download count): "More Puzzle for
//!      your library";
//!    - *donated*: apps they paid a donation to: "You've supported Cozy". Only the donor's own
//!      page uses it; a donation never changes what anyone else sees.
//!
//!    Each reel is apps with that tag they don't have yet, by rating, players and fit, with a
//!    small daily shuffle.
//! 3. **Coming soon, for you**: upcoming apps that match their taste (interested.rs).
//!
//! **Mixed in, for everyone:** All-time favourites (catalog's `rating` order, at least 70%
//! Slaps), Up-and-coming (released in the last 60 days, most added and played this week; with
//! personal recommendations, only ones in genres they like, when there are enough), Popular this
//! week (catalog's `trending`: most players this week, then library adds this week) and Popular
//! this month (catalog's `popular`). Their order is shuffled once a day, and they take turns
//! with the personal reels.
//!
//! **Never shown:** dead projects, the viewer's own apps, anything their age view hides, and,
//! with personal recommendations, what they already have, marked, played, voted on or donated
//! to. An app is in at most two reels on the page, and at most four of a reel's apps are repeats
//! (earlier reels keep theirs). Signed out or switched off, nobody's activity is read: the page
//! is the general reels, the same for everyone that day, and `off` says why.

use std::collections::{HashMap, HashSet};

use axum::{Json, Router, extract::State, http::HeaderMap, routing::get};
use serde::Serialize;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::MaybeUser,
    catalog::{AppStage, AppSummary},
    config::Config,
    error::ApiResult,
    foryou::{self, Candidate, Signals},
    interested,
    policy::ViewerPolicy,
    votes::vote_weight,
};

/// Apps in a reel, at most.
pub const REEL: usize = 12;
/// Genre reels on a page, at most.
pub const MAX_GENRES: usize = 4;
/// A genre needs at least this many apps to show to get a reel.
pub const MIN_GENRE_APPS: usize = 2;
/// An app shows in at most this many reels on the page.
pub const MAX_SHOWN: u8 = 2;
/// At most this many of a reel's apps may already be in an earlier reel.
pub const MAX_REPEATS: usize = 4;
/// "Play" needs about half an hour in a tag (the sum of vote weights).
const PLAYED_ENOUGH: f64 = 0.5;
/// All-time favourites: at least this weighted % Slaps (and votes::MIN_VOTES votes).
pub const FAVOURITE_PERCENT: i32 = 70;
/// Up-and-coming is personal when at least this many match the person's genres.
const MIN_RISING_MATCHES: usize = 3;

pub fn routes() -> Router<AppState> {
    Router::new().route("/discover", get(discover))
}

#[derive(Clone, Serialize)]
pub struct Tile {
    #[serde(flatten)]
    pub app: AppSummary,
    /// Why this app is here, when it has its own reason (For you cards).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Serialize)]
pub struct Reel {
    /// Stable per kind: `for_you`, `genre:play:<tag>`, `upcoming_for_you`, `all_time`…
    pub key: String,
    pub title: String,
    /// What it's based on, in plain words.
    pub why: String,
    pub apps: Vec<Tile>,
    /// A store page with more like it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub see_all: Option<String>,
}

/// Why the page has no personal reels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Off {
    SignedOut,
    /// Their setting, Global Privacy Control, or the under-18 view's default.
    SwitchedOff,
}

#[derive(Serialize)]
pub struct Discover {
    /// Personal recommendations are on for this viewer.
    pub personalized: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub off: Option<Off>,
    pub reels: Vec<Reel>,
}

/// What a person did that Discover reads (only with personal recommendations on).
#[derive(Default, Clone)]
pub struct Person {
    pub signals: Signals,
    /// Apps they paid a donation to.
    pub donated: HashSet<Uuid>,
}

/// The signal a genre reel comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Because {
    Play,
    Like,
    Library,
    Donated,
}

impl Because {
    /// The order the signals take turns in.
    pub const ALL: [Because; 4] = [
        Because::Play,
        Because::Like,
        Because::Library,
        Because::Donated,
    ];

    fn key(self) -> &'static str {
        match self {
            Because::Play => "play",
            Because::Like => "like",
            Because::Library => "library",
            Because::Donated => "donated",
        }
    }
}

/// Play, like and so on per tag, from what the person did. `tags_of` gives each app's tags.
fn signal_weights(
    person: &Person,
    because: Because,
    tags_of: &HashMap<Uuid, Vec<String>>,
) -> HashMap<String, f64> {
    let s = &person.signals;
    let mut w: HashMap<String, f64> = HashMap::new();
    let mut add = |app: &Uuid, amount: f64| {
        for t in tags_of.get(app).into_iter().flatten() {
            *w.entry(t.clone()).or_default() += amount;
        }
    };
    match because {
        Because::Play => {
            for (app, (playtime, days)) in &s.launched {
                add(app, f64::from(vote_weight(*playtime, *days)));
            }
        }
        Because::Like => {
            for (app, vote) in &s.votes {
                add(app, f64::from(*vote));
            }
            for app in &s.interested {
                add(app, 1.0);
            }
        }
        Because::Library => {
            for app in &s.library {
                add(app, 1.0);
            }
        }
        Because::Donated => {
            for app in &person.donated {
                add(app, 1.0);
            }
        }
    }
    w
}

/// The person's genres: each signal in turn gives its strongest tag not taken yet, up to
/// MAX_GENRES. A tag needs enough of that signal, an overall taste not against it
/// (foryou::taste), and `enough` apps to show.
pub fn pick_genres(
    person: &Person,
    tags_of: &HashMap<Uuid, Vec<String>>,
    enough: &dyn Fn(&str) -> bool,
) -> Vec<(Because, String)> {
    let taste = foryou::taste(&person.signals, tags_of);
    let ranked: Vec<(Because, Vec<String>)> = Because::ALL
        .iter()
        .map(|&b| {
            let least = if b == Because::Play {
                PLAYED_ENOUGH
            } else {
                1.0
            };
            let mut tags: Vec<(String, f64)> = signal_weights(person, b, tags_of)
                .into_iter()
                .filter(|(t, v)| *v >= least && taste.get(t).copied().unwrap_or(0.0) >= 0.0)
                .collect();
            tags.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            (
                b,
                tags.into_iter()
                    .map(|(t, _)| t)
                    .filter(|t| enough(t))
                    .collect(),
            )
        })
        .collect();
    let mut chosen: Vec<(Because, String)> = Vec::new();
    let mut used: HashSet<String> = HashSet::new();
    loop {
        let before = chosen.len();
        for (b, tags) in &ranked {
            if chosen.len() >= MAX_GENRES {
                break;
            }
            if let Some(t) = tags.iter().find(|t| !used.contains(*t)) {
                used.insert(t.clone());
                chosen.push((*b, t.clone()));
            }
        }
        if chosen.len() == before || chosen.len() >= MAX_GENRES {
            return chosen;
        }
    }
}

/// A stand-in ID for a reel's key, for the daily shuffle.
fn key_id(key: &str) -> Uuid {
    // FNV-1a
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    Uuid::from_u64_pair(h, h.rotate_left(29))
}

/// The page's order after For you: the personal reels in their order and the general ones,
/// shuffled by `seed`, taking turns (personal first); whichever runs out first, the rest follow.
pub fn mix(personal: &[String], general: &[String], seed: u64) -> Vec<String> {
    let mut general: Vec<&String> = general.iter().collect();
    general.sort_by(|a, b| {
        foryou::shuffle(seed, key_id(a))
            .total_cmp(&foryou::shuffle(seed, key_id(b)))
            .then_with(|| a.cmp(b))
    });
    let (mut p, mut g) = (personal.iter(), general.into_iter());
    let mut out = Vec::with_capacity(personal.len() + g.len());
    loop {
        match (p.next(), g.next()) {
            (None, None) => return out,
            (a, b) => out.extend(a.into_iter().chain(b).cloned()),
        }
    }
}

/// Per-app numbers the general reels rank by.
#[derive(Clone, Copy, Default)]
pub struct Stats {
    /// Released in the last 60 days.
    pub new_release: bool,
    /// Library adds in the last 7 days.
    pub adds_week: i64,
    /// Library adds this calendar month.
    pub adds_month: i64,
    /// Most players at once this calendar month.
    pub players_month: i32,
}

async fn stats(db: &PgPool, ids: &[Uuid]) -> sqlx::Result<HashMap<Uuid, Stats>> {
    let rows: Vec<(Uuid, bool, i64, i64, i32)> = sqlx::query_as(
        "select a.id,
                a.stage = 'released' and a.stage_entered_at > now() - interval '60 days',
                (select count(*) from library_entries l
                  where l.app_id = a.id and l.added_at > now() - interval '7 days'),
                (select count(*) from library_entries l
                  where l.app_id = a.id and l.added_at >= date_trunc('month', now())),
                (select coalesce(max(p.players), 0) from pulse_counts p
                  where p.app_id = a.id and p.bucket >= date_trunc('month', now()))
         from apps a where a.id = any($1)",
    )
    .bind(ids)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, new_release, adds_week, adds_month, players_month)| {
            (
                id,
                Stats {
                    new_release,
                    adds_week,
                    adds_month,
                    players_month,
                },
            )
        })
        .collect())
}

async fn person(db: &PgPool, user_id: Uuid) -> sqlx::Result<Person> {
    let donated: Vec<Uuid> = sqlx::query_scalar(
        "select distinct app_id from donations
         where donor_id = $1 and status in ('paid', 'partially_refunded')",
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;
    Ok(Person {
        signals: foryou::signals(db, user_id).await?,
        donated: donated.into_iter().collect(),
    })
}

/// Tags of these apps, wherever they are now (a library keeps apps that left the store).
async fn tags_of(db: &PgPool, ids: &[Uuid]) -> sqlx::Result<HashMap<Uuid, Vec<String>>> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "select at.app_id, t.slug from app_tags at join tags t on t.id = at.tag_id
         where at.app_id = any($1) order by t.slug",
    )
    .bind(ids)
    .fetch_all(db)
    .await?;
    let mut out: HashMap<Uuid, Vec<String>> = HashMap::new();
    for (app, tag) in rows {
        out.entry(app).or_default().push(tag);
    }
    Ok(out)
}

/// The open apps with this tag that are out (released or in beta).
fn in_genre<'a>(
    open: &'a [usize],
    cands: &'a [Candidate],
    tag: &'a str,
) -> impl Iterator<Item = usize> + 'a {
    open.iter().copied().filter(move |&k| {
        cands[k].app.stage != AppStage::ComingSoon && cands[k].app.tags.iter().any(|t| t == tag)
    })
}

/// A reel before the page is put together: its apps best first, more than it shows.
struct Draft {
    key: String,
    title: String,
    why: String,
    see_all: Option<String>,
    ranked: Vec<(usize, Option<String>)>,
    /// Fewer apps than this after dedupe and it's left out.
    min: usize,
    /// For you: shown as it is.
    fixed: bool,
}

/// Everything the page is made from.
pub struct Inputs<'a> {
    pub cands: &'a [Candidate],
    pub stats: &'a HashMap<Uuid, Stats>,
    pub names: &'a HashMap<String, String>,
    /// Only with personal recommendations on.
    pub person: Option<&'a Person>,
    /// Tags of the person's apps (and the candidates').
    pub tags_of: &'a HashMap<Uuid, Vec<String>>,
    /// Signed in: never their own apps.
    pub viewer: Option<Uuid>,
    pub seed: u64,
}

/// Puts the page together (see the module's header).
pub fn build(i: &Inputs) -> Vec<Reel> {
    let cands = i.cands;
    let name = |t: &str| i.names.get(t).cloned().unwrap_or_else(|| t.to_string());
    let stat = |c: &Candidate| i.stats.get(&c.id).copied().unwrap_or_default();
    let open: Vec<usize> = (0..cands.len())
        .filter(|&k| {
            let c = &cands[k];
            !c.app.is_dead
                && i.viewer != Some(c.maker_id)
                && i.person.is_none_or(|p| {
                    let s = &p.signals;
                    !(s.library.contains(&c.id)
                        || s.interested.contains(&c.id)
                        || s.launched.contains_key(&c.id)
                        || s.votes.contains_key(&c.id)
                        || p.donated.contains(&c.id))
                })
        })
        .collect();
    let quality = |c: &Candidate| {
        c.app
            .not_slop_percent
            .map_or(0.0, |p| f64::from(p - 50) / 50.0)
    };
    let activity = |c: &Candidate| f64::from(c.players_week) + stat(c).adds_month as f64;
    let max_activity = open
        .iter()
        .map(|&k| activity(&cands[k]))
        .fold(0.0, f64::max);
    let pop = |c: &Candidate| {
        if max_activity == 0.0 {
            0.0
        } else {
            (1.0 + activity(c)).ln() / (1.0 + max_activity).ln()
        }
    };
    let taste = i
        .person
        .map(|p| foryou::taste(&p.signals, i.tags_of))
        .unwrap_or_default();
    let norm = taste.values().map(|v| v * v).sum::<f64>().sqrt();
    let affinity = |c: &Candidate| {
        if c.app.tags.is_empty() || norm == 0.0 {
            return 0.0;
        }
        let dot: f64 = c
            .app
            .tags
            .iter()
            .map(|t| taste.get(t).copied().unwrap_or(0.0))
            .sum();
        dot / ((c.app.tags.len() as f64).sqrt() * norm)
    };
    let plain = |v: Vec<usize>| v.into_iter().map(|k| (k, None)).collect::<Vec<_>>();

    // For you, and the personal reels.
    let mut first: Option<Draft> = None;
    let mut personal: Vec<Draft> = Vec::new();
    if let Some(p) = i.person {
        let feed = foryou::feed(&p.signals, cands, i.names, true, i.seed);
        let at: HashMap<&str, usize> = cands
            .iter()
            .enumerate()
            .map(|(k, c)| (c.app.slug.as_str(), k))
            .collect();
        first = Some(Draft {
            key: "for_you".into(),
            title: "Recommended for you".into(),
            why: if feed.personalized {
                format!(
                    "From your library, Interested list, playtime and votes, leaning on {}. Every fourth is something different.",
                    feed.based_on.join(", ")
                )
            } else {
                "Popular, well rated and new. Add apps to your library and play them for picks of your own."
                    .into()
            },
            see_all: None,
            ranked: feed
                .items
                .into_iter()
                .filter_map(|card| {
                    at.get(card.app.slug.as_str())
                        .map(|&k| (k, Some(card.reason)))
                })
                .collect(),
            min: 0,
            fixed: true,
        });

        let enough =
            |tag: &str| in_genre(&open, cands, tag).take(MIN_GENRE_APPS).count() >= MIN_GENRE_APPS;
        for (because, tag) in pick_genres(p, i.tags_of, &enough) {
            let t = name(&tag);
            let (title, why) = match because {
                Because::Play => (
                    format!("Because you play a lot of {t}"),
                    format!(
                        "{t} is what you've spent the most time in. More of it you haven't played, best rated and most played first."
                    ),
                ),
                Because::Like => (
                    format!("Because you like {t}"),
                    format!(
                        "From your 👍 Slaps votes and Interested list. More {t} you don't have yet, best rated and most played first."
                    ),
                ),
                Because::Library => (
                    format!("More {t} for your library"),
                    format!(
                        "{t} is the tag your library has most (apps you added, or installed while signed in). Ones you don't have yet, best rated first."
                    ),
                ),
                Because::Donated => (
                    format!("You've supported {t}"),
                    format!(
                        "You've donated to {t} projects. More of them, best rated first. Only you see this: donations never change what anyone else is shown."
                    ),
                ),
            };
            let mut ranked: Vec<(usize, f64)> = in_genre(&open, cands, &tag)
                .map(|k| {
                    let c = &cands[k];
                    (
                        k,
                        0.45 * quality(c)
                            + 0.35 * pop(c)
                            + 0.15 * affinity(c)
                            + 0.05 * foryou::shuffle(i.seed, c.id),
                    )
                })
                .collect();
            ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
            personal.push(Draft {
                key: format!("genre:{}:{tag}", because.key()),
                title,
                why,
                see_all: Some(format!("/tag/{tag}?sort=rating")),
                ranked: plain(ranked.into_iter().map(|(k, _)| k).collect()),
                min: MIN_GENRE_APPS,
                fixed: false,
            });
        }

        // Coming soon, for you: upcoming apps matching their taste.
        let upcoming: Vec<Candidate> = open
            .iter()
            .map(|&k| &cands[k])
            .filter(|c| c.app.stage != AppStage::Released)
            .cloned()
            .collect();
        let ranked: Vec<(usize, Option<String>)> =
            interested::upcoming_matches(&p.signals, &taste, i.names, upcoming)
                .into_iter()
                .filter(|(_, _, matched)| *matched)
                .filter_map(|(c, reason, _)| {
                    at.get(c.app.slug.as_str()).map(|&k| (k, Some(reason)))
                })
                .collect();
        let upcoming = Draft {
            key: "upcoming_for_you".into(),
            title: "Coming soon, for you".into(),
            why: "Upcoming projects like the ones in your library and Interested list, most anticipated first.".into(),
            see_all: Some("/upcoming".into()),
            ranked,
            min: 1,
            fixed: false,
        };
        // After the second genre reel.
        personal.insert(personal.len().min(2), upcoming);
    }

    // Mixed in, for everyone.
    let mut general: Vec<Draft> = Vec::new();
    let mut favourites: Vec<usize> = open
        .iter()
        .copied()
        .filter(|&k| {
            cands[k]
                .app
                .not_slop_percent
                .is_some_and(|p| p >= FAVOURITE_PERCENT)
        })
        .collect();
    favourites.sort_by(|&a, &b| {
        let (x, y) = (&cands[a].app, &cands[b].app);
        y.not_slop_percent
            .cmp(&x.not_slop_percent)
            .then(y.vote_count.cmp(&x.vote_count))
            .then_with(|| x.slug.cmp(&y.slug))
    });
    general.push(Draft {
        key: "all_time".into(),
        title: "All-time favourites".into(),
        why: format!(
            "The best rated ever: the highest % Slaps, at least {FAVOURITE_PERCENT}%, among apps with {} or more votes.",
            crate::votes::MIN_VOTES
        ),
        see_all: Some("/search?sort=rating".into()),
        ranked: plain(favourites),
        min: 1,
        fixed: false,
    });

    let mut rising: Vec<(usize, f64)> = open
        .iter()
        .copied()
        .filter(|&k| {
            let c = &cands[k];
            let s = stat(c);
            s.new_release && (s.adds_week > 0 || c.players_week > 0)
        })
        .map(|k| {
            let c = &cands[k];
            let s = stat(c);
            (
                k,
                (1.0 + s.adds_week as f64).ln()
                    + (1.0 + f64::from(c.players_week)).ln()
                    + 0.3 * quality(c)
                    + 0.05 * foryou::shuffle(i.seed, c.id),
            )
        })
        .collect();
    rising.sort_by(|a, b| b.1.total_cmp(&a.1));
    let liked = |k: &usize| {
        cands[*k]
            .app
            .tags
            .iter()
            .any(|t| taste.get(t).is_some_and(|w| *w > 0.0))
    };
    let matching: Vec<usize> = rising.iter().map(|(k, _)| *k).filter(liked).collect();
    if matching.len() >= MIN_RISING_MATCHES {
        // The genres they like most among these, for the why.
        let mut tags: Vec<&String> = matching
            .iter()
            .flat_map(|&k| cands[k].app.tags.iter())
            .filter(|t| taste.get(*t).is_some_and(|w| *w > 0.0))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        tags.sort_by(|a, b| taste[*b].total_cmp(&taste[*a]).then_with(|| a.cmp(b)));
        let tags: Vec<String> = tags.into_iter().take(2).map(|t| name(t)).collect();
        general.push(Draft {
            key: "rising".into(),
            title: "Up-and-coming for you".into(),
            why: format!(
                "Released in the last 60 days and most added and played this week, in genres you like ({}).",
                tags.join(", ")
            ),
            see_all: None,
            ranked: plain(matching),
            min: 1,
            fixed: false,
        });
    } else {
        general.push(Draft {
            key: "rising".into(),
            title: "Up-and-coming".into(),
            why: "Released in the last 60 days and most added and played this week.".into(),
            see_all: None,
            ranked: plain(rising.into_iter().map(|(k, _)| k).collect()),
            min: 1,
            fixed: false,
        });
    }

    let mut week: Vec<usize> = open
        .iter()
        .copied()
        .filter(|&k| {
            let c = &cands[k];
            c.app.stage != AppStage::ComingSoon && (c.players_week > 0 || stat(c).adds_week > 0)
        })
        .collect();
    week.sort_by(|&a, &b| {
        let (x, y) = (&cands[a], &cands[b]);
        y.players_week
            .cmp(&x.players_week)
            .then(stat(y).adds_week.cmp(&stat(x).adds_week))
            .then_with(|| x.app.slug.cmp(&y.app.slug))
    });
    general.push(Draft {
        key: "week".into(),
        title: "Popular this week".into(),
        why: "Most players at once this week, then most added to libraries this week.".into(),
        see_all: Some("/search?sort=trending".into()),
        ranked: plain(week),
        min: 1,
        fixed: false,
    });

    let mut month: Vec<usize> = open
        .iter()
        .copied()
        .filter(|&k| {
            let c = &cands[k];
            let s = stat(c);
            c.app.stage != AppStage::ComingSoon && (s.adds_month > 0 || s.players_month > 0)
        })
        .collect();
    month.sort_by(|&a, &b| {
        let (x, y) = (stat(&cands[a]), stat(&cands[b]));
        y.adds_month
            .cmp(&x.adds_month)
            .then(y.players_month.cmp(&x.players_month))
            .then_with(|| cands[a].app.slug.cmp(&cands[b].app.slug))
    });
    general.push(Draft {
        key: "month".into(),
        title: "Popular this month".into(),
        why: "Most added to libraries this month, then most players.".into(),
        see_all: Some("/search?sort=popular".into()),
        ranked: plain(month),
        min: 1,
        fixed: false,
    });

    // Order, then fill each reel in that order, leaving out apps shown enough already.
    let keys = |d: &[Draft]| d.iter().map(|d| d.key.clone()).collect::<Vec<_>>();
    let order = mix(&keys(&personal), &keys(&general), i.seed);
    let mut drafts: HashMap<String, Draft> = personal
        .into_iter()
        .chain(general)
        .map(|d| (d.key.clone(), d))
        .collect();
    let mut shown: HashMap<usize, u8> = HashMap::new();
    let mut reels = Vec::new();
    for d in first
        .into_iter()
        .chain(order.iter().filter_map(|k| drafts.remove(k)))
    {
        let mut apps: Vec<(usize, Option<String>)> = Vec::new();
        let mut repeats = 0;
        for (k, reason) in d.ranked {
            if apps.len() >= REEL {
                break;
            }
            let times = shown.get(&k).copied().unwrap_or(0);
            if !d.fixed {
                if times >= MAX_SHOWN {
                    continue;
                }
                if times > 0 {
                    if repeats >= MAX_REPEATS {
                        continue;
                    }
                    repeats += 1;
                }
            }
            apps.push((k, reason));
        }
        if !d.fixed && apps.len() < d.min.max(1) {
            continue;
        }
        for (k, _) in &apps {
            *shown.entry(*k).or_default() += 1;
        }
        reels.push(Reel {
            key: d.key,
            title: d.title,
            why: d.why,
            see_all: d.see_all,
            apps: apps
                .into_iter()
                .map(|(k, reason)| Tile {
                    app: cands[k].app.clone(),
                    reason,
                })
                .collect(),
        });
    }
    reels
}

/// The page for a viewer on a day: the same all day, a new shuffle the next.
pub async fn page(
    db: &PgPool,
    config: &Config,
    viewer: Option<Uuid>,
    policy: ViewerPolicy,
    day: time::Date,
) -> sqlx::Result<Discover> {
    let personal = match viewer {
        Some(id) => foryou::personalized(db, id, policy).await?,
        None => false,
    };
    let cands = foryou::candidates(db, config, policy).await?;
    let ids: Vec<Uuid> = cands.iter().map(|c| c.id).collect();
    let stats = stats(db, &ids).await?;
    let names = foryou::tag_names(db).await?;
    let mut tags: HashMap<Uuid, Vec<String>> =
        cands.iter().map(|c| (c.id, c.app.tags.clone())).collect();
    let (person, seed) = match viewer.filter(|_| personal) {
        Some(id) => {
            let p = person(db, id).await?;
            let s = &p.signals;
            let theirs: Vec<Uuid> = s
                .library
                .iter()
                .chain(&s.interested)
                .chain(s.launched.keys())
                .chain(s.votes.keys())
                .chain(&p.donated)
                .filter(|a| !tags.contains_key(*a))
                .copied()
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            tags.extend(tags_of(db, &theirs).await?);
            (Some(p), foryou::daily_seed(id, day))
        }
        // Not personal: the same for everyone that day.
        None => (None, foryou::daily_seed(Uuid::nil(), day)),
    };
    let reels = build(&Inputs {
        cands: &cands,
        stats: &stats,
        names: &names,
        person: person.as_ref(),
        tags_of: &tags,
        viewer,
        seed,
    });
    Ok(Discover {
        personalized: personal,
        off: match (viewer, personal) {
            (None, _) => Some(Off::SignedOut),
            (Some(_), false) => Some(Off::SwitchedOff),
            (Some(_), true) => None,
        },
        reels,
    })
}

async fn discover(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
) -> ApiResult<Json<Discover>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let viewer = user.as_ref().map(|u| u.id);
    Ok(Json(
        page(
            &state.db,
            &state.config,
            viewer,
            policy,
            OffsetDateTime::now_utc().date(),
        )
        .await?,
    ))
}
