// SPDX-License-Identifier: AGPL-3.0-or-later
//! People in the simulation sandbox (DESIGN §15): made-up artists, composers, writers and
//! studios, with a hidden truth (how good and how reliable each one is), and jobs from the
//! simulated makers in every state — requested, offered, paid, delivered, completed, declined,
//! cancelled, withdrawn.
//!
//! Hiring has a real effect in the simulated world: when a released app ships an update with
//! work from a creative, its true quality moves with the creative's quality (great art makes it
//! better, generator output makes it worse), and players vote on what they see. The report then
//! asks the real code (people/, votes.rs) whether the credits, the badge, the "since the new
//! work" rating and creatives' ratings tell the truth. Payments go through the real
//! `jobs::payment_succeeded_at`, marked simulated; no money moves.

use std::collections::BTreeMap;

use base64::Engine;
use serde::Serialize;
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{
    catalog::AppKind,
    media::Picture,
    people::{
        self, Link, Service,
        jobs::{self, Offer, Paid, Parties},
        portfolio::Work,
    },
    sim::Rng,
};

/// A creative as written: (name, handle, kind, team, skills, AI use, location, country, headline).
type Seed = (
    &'static str,
    &'static str,
    &'static str,
    i16,
    &'static [&'static str],
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

pub const CREATIVES: &[Seed] = &[
    (
        "Inkwell & Ivy",
        "inkwell-and-ivy",
        "studio",
        3,
        &["2d-art", "animation", "ui-ux"],
        "none",
        "Lisbon, PT",
        "PT",
        "Hand-painted characters, portraits and UI kits for cosy games.",
    ),
    (
        "Vera Okafor",
        "voxelvera",
        "individual",
        0,
        &["3d", "pixel-art"],
        "assisted",
        "Lagos, NG",
        "US",
        "Low-poly and voxel models, game-ready, rigged on request.",
    ),
    (
        "Loopsmith Audio",
        "loopsmith",
        "studio",
        2,
        &["music", "sound"],
        "none",
        "Montréal, CA",
        "US",
        "Adaptive soundtracks and punchy SFX. Loops that don't get old.",
    ),
    (
        "pixelpriest",
        "pixelpriest",
        "individual",
        0,
        &["pixel-art", "animation"],
        "none",
        "Online",
        "US",
        "Sprite sheets, tilesets and tiny animated things.",
    ),
    (
        "Sam Quill",
        "sam-quill",
        "individual",
        0,
        &["writing", "localization"],
        "assisted",
        "Edinburgh, UK",
        "GB",
        "Dialogue and item text that doesn't read like a chatbot.",
    ),
    (
        "Vox Populi Voices",
        "vox-populi",
        "studio",
        9,
        &["voice", "localization"],
        "none",
        "Berlin, DE",
        "DE",
        "Nine voice actors in six languages. Real people, real performances.",
    ),
    (
        "BugBusters QA",
        "bugbusters",
        "studio",
        14,
        &["playtesting"],
        "none",
        "Remote",
        "US",
        "Human playtesters who find the bugs your AI didn't know it wrote.",
    ),
    (
        "PromptMonger",
        "promptmonger",
        "individual",
        0,
        &["2d-art", "3d", "music", "writing"],
        "heavy",
        "Online",
        "US",
        "Any asset, any style, delivered in an hour!",
    ),
    (
        "Mireille Tan",
        "mireille-tan",
        "individual",
        0,
        &["music"],
        "none",
        "Singapore",
        "US",
        "Chiptune and lo-fi loops for small games.",
    ),
    (
        "Kofi Mensah",
        "kofi-sfx",
        "individual",
        0,
        &["sound"],
        "none",
        "Accra, GH",
        "US",
        "Foley-first sound design: footsteps, doors, magic.",
    ),
    (
        "Hana Sato",
        "hana-sato",
        "individual",
        0,
        &["2d-art", "ui-ux"],
        "none",
        "Osaka, JP",
        "US",
        "Clean UI and icons that still read at 16 pixels.",
    ),
    (
        "Dmitri Volkov",
        "dmitri-3d",
        "individual",
        0,
        &["3d", "animation"],
        "assisted",
        "Tbilisi, GE",
        "US",
        "Rigged characters and props, mocap clean-up.",
    ),
    (
        "Ana Lucía Ríos",
        "ana-rios",
        "individual",
        0,
        &["writing"],
        "none",
        "Bogotá, CO",
        "US",
        "Quests, barks and item text with a voice of their own.",
    ),
    (
        "Theo Brandt",
        "theo-brandt",
        "individual",
        0,
        &["music"],
        "assisted",
        "Hamburg, DE",
        "DE",
        "Orchestral-ish scores on a game-jam budget.",
    ),
    (
        "Pixel Forge Collective",
        "pixel-forge",
        "studio",
        5,
        &["pixel-art", "2d-art"],
        "none",
        "Warsaw, PL",
        "PL",
        "Five pixel artists, one consistent style.",
    ),
    (
        "Ruth Abernathy",
        "ruth-voice",
        "individual",
        0,
        &["voice"],
        "none",
        "Austin, US",
        "US",
        "Narration and character voices from a treated home studio.",
    ),
    (
        "Juniper Fox",
        "juniper-fox",
        "individual",
        0,
        &["animation", "2d-art"],
        "none",
        "Portland, US",
        "US",
        "Frame-by-frame animation with bounce.",
    ),
    (
        "Echo Chamber SFX",
        "echo-chamber",
        "studio",
        2,
        &["sound"],
        "assisted",
        "Manchester, UK",
        "GB",
        "Weapon, UI and ambience packs, mixed for headphones.",
    ),
    (
        "Priya Raman",
        "priya-ux",
        "individual",
        0,
        &["ui-ux", "2d-art"],
        "assisted",
        "Bengaluru, IN",
        "US",
        "Menus that players don't have to think about.",
    ),
    (
        "Mateus Costa",
        "mateus-costa",
        "individual",
        0,
        &["music", "sound"],
        "none",
        "Porto Alegre, BR",
        "US",
        "Bossa-flavoured soundtracks and the sounds to match.",
    ),
    (
        "QuickArt Bot Studio",
        "quickart",
        "studio",
        2,
        &["2d-art", "pixel-art"],
        "heavy",
        "Online",
        "US",
        "Hundreds of assets a day, any style.",
    ),
    (
        "Lena Schröder",
        "lena-l10n",
        "individual",
        0,
        &["localization", "writing"],
        "none",
        "Vienna, AT",
        "AT",
        "German and English localisation that keeps the jokes.",
    ),
    (
        "Oskar Lind",
        "oskar-lind",
        "individual",
        0,
        &["3d"],
        "none",
        "Malmö, SE",
        "SE",
        "Stylised low-poly worlds, ready for Godot and Bevy.",
    ),
    (
        "Bea Moreno",
        "bea-moreno",
        "individual",
        0,
        &["pixel-art"],
        "none",
        "Valencia, ES",
        "ES",
        "Tilesets and characters at 16×16 and 32×32.",
    ),
];

/// Two more, signed up but with their first portfolio still waiting for review.
pub const WAITING: &[Seed] = &[
    (
        "Nadia Karim",
        "nadia-karim",
        "individual",
        0,
        &["2d-art"],
        "none",
        "Leeds, UK",
        "GB",
        "Painterly backgrounds and key art.",
    ),
    (
        "Tomás Ferreira",
        "tomas-ferreira",
        "individual",
        0,
        &["music"],
        "assisted",
        "Lisbon, PT",
        "PT",
        "Synthwave for racing and arcade games.",
    ),
];

/// (name, description, lowest and highest price in cents, days) per skill.
fn service_templates(skill: &str) -> &'static [(&'static str, &'static str, i64, i64, i32)] {
    match skill {
        "pixel-art" => &[
            (
                "Character sprite sheet",
                "One character, 4 directions, walk and idle",
                2_500,
                6_000,
                5,
            ),
            (
                "16×16 tileset (64 tiles)",
                "Terrain, props and transitions",
                8_000,
                15_000,
                10,
            ),
        ],
        "2d-art" => &[
            (
                "Character portrait",
                "Bust portrait, transparent background",
                4_000,
                9_000,
                5,
            ),
            (
                "Store capsule art",
                "Thumbnail and header art for your store page",
                15_000,
                30_000,
                10,
            ),
        ],
        "3d" => &[
            (
                "Low-poly prop",
                "Game-ready, textured, under 2k tris",
                3_000,
                6_000,
                3,
            ),
            (
                "Rigged character",
                "Modelled, textured and rigged",
                18_000,
                30_000,
                14,
            ),
        ],
        "animation" => &[(
            "Character animation set",
            "Idle, walk, run, jump, hit",
            15_000,
            35_000,
            14,
        )],
        "ui-ux" => &[(
            "UI kit",
            "Buttons, panels, icons and fonts, with a style guide",
            30_000,
            60_000,
            21,
        )],
        "music" => &[
            (
                "Loopable track (2–3 min)",
                "Mixed and mastered, seamless loop",
                12_000,
                25_000,
                10,
            ),
            (
                "Adaptive score (5 layers)",
                "Stems that build with the action",
                60_000,
                120_000,
                30,
            ),
        ],
        "sound" => &[(
            "SFX pack (40 sounds)",
            "Recorded and designed, in WAV and OGG",
            20_000,
            40_000,
            14,
        )],
        "writing" => &[
            (
                "Dialogue pass (per 1,000 words)",
                "Rewrite for voice and clarity",
                6_000,
                12_000,
                5,
            ),
            (
                "Quest and item text",
                "Up to 40 quests or 200 items",
                20_000,
                40_000,
                14,
            ),
        ],
        "voice" => &[(
            "Character voice (100 lines)",
            "Recorded, edited, named files",
            10_000,
            20_000,
            14,
        )],
        "localization" => &[(
            "Translation (per 1,000 words)",
            "Native speaker, in context",
            8_000,
            14_000,
            5,
        )],
        _ => &[
            (
                "Smoke test and report",
                "Two testers, one afternoon, a written report",
                15_000,
                15_000,
                3,
            ),
            (
                "Week-long playtest (5 testers)",
                "Daily notes, bug list, video clips",
                100_000,
                100_000,
                7,
            ),
        ],
    }
}

fn work_titles(skill: &str) -> &'static [&'static str] {
    match skill {
        "pixel-art" => &[
            "Knight sprite sheet",
            "Forest tileset",
            "Slime idle animation",
            "Dungeon props",
            "Pixel portrait set",
        ],
        "2d-art" => &[
            "Tavern keeper portrait",
            "Key art: the harbour",
            "Spellbook icons",
            "Capsule art",
            "Character line-up",
        ],
        "3d" => &[
            "Low-poly lighthouse",
            "Rigged fox",
            "Market stall kit",
            "Sci-fi crates",
            "Voxel castle",
        ],
        "animation" => &[
            "Run cycle",
            "Spell cast",
            "Idle breathing loop",
            "Boss intro",
        ],
        "ui-ux" => &["Inventory screen", "Settings menu", "HUD kit", "Shop flow"],
        "music" => &[
            "Main theme",
            "Tavern loop",
            "Boss battle",
            "Rainy menu",
            "Credits roll",
        ],
        "sound" => &[
            "Footsteps on stone",
            "Magic whooshes",
            "UI clicks",
            "Forest ambience",
        ],
        "writing" => &[
            "Opening scene",
            "Item descriptions",
            "Merchant barks",
            "Quest log",
        ],
        "voice" => &["Narrator reel", "Villain lines", "Character barks"],
        "localization" => &["German menu pass", "Spanish dialogue sample", "Glossary"],
        _ => &["Bug report sample", "Playtest summary", "Heatmap notes"],
    }
}

fn glyph(skill: &str) -> &'static str {
    match skill {
        "music" => "♪",
        "sound" => "◉",
        "writing" | "localization" => "¶",
        "voice" => "◖",
        "playtesting" => "✓",
        "3d" => "◆",
        "ui-ux" => "▤",
        _ => "▦",
    }
}

/// A picture for the sandbox's portfolios, made on the spot (no files to host).
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn svg_picture(path: &str, w: u32, h: u32, color: &str, mark: &str, title: &str) -> Picture {
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{w}' height='{h}' viewBox='0 0 {w} {h}'>\
         <defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'><stop offset='0' stop-color='{color}'/>\
         <stop offset='1' stop-color='#151a21'/></linearGradient></defs>\
         <rect width='100%' height='100%' fill='url(#g)'/>\
         <text x='50%' y='52%' font-size='{}' text-anchor='middle' dominant-baseline='middle' fill='#ffffffcc' font-family='sans-serif'>{}</text>\
         <text x='24' y='{}' font-size='{}' fill='#ffffffdd' font-family='sans-serif'>{}</text></svg>",
        h / 3,
        escape(mark),
        h - 24,
        (h / 14).max(12),
        escape(title),
    );
    let url = format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(svg)
    );
    Picture {
        path: path.into(),
        sha256: format!("{:064x}", url.len()),
        url,
        width: w,
        height: h,
        format: "svg".into(),
        dhash: None,
    }
}

const ACCENT_COLORS: &[(&str, &str)] = &[
    ("slime", "#a3e635"),
    ("ocean", "#38bdf8"),
    ("grape", "#c084fc"),
    ("rose", "#fb7185"),
    ("ember", "#fb923c"),
    ("sun", "#facc15"),
    ("forest", "#4ade80"),
    ("slate", "#94a3b8"),
];

pub struct SimCreative {
    pub seed: &'static Seed,
    pub id: Uuid,
    /// The truth: how good the work is (0..1), and how likely they deliver well.
    pub quality: f64,
    pub reliability: f64,
    pub trader: bool,
    pub accent: usize,
    pub live: bool,
}

/// What a planned job ends as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    Requested,
    Offered,
    AwaitingPayment,
    InProgress,
    DeliveredRecently,
    /// Delivered more than 7 days ago with no answer: the real auto-complete finishes it.
    DeliveredStale,
    Completed,
    /// Completed (auto) with the final payment still owed: no credit.
    CompletedOwing,
    Declined,
    Cancelled,
    Withdrawn,
}

pub struct PlannedJob {
    pub app: usize,
    pub creative: usize,
    pub skill: &'static str,
    pub end: End,
    /// Simulated day it was requested.
    pub day: i64,
    /// For work shipped in an update: the update's day.
    pub ships_on: Option<i64>,
}

pub struct Plan {
    pub creatives: Vec<SimCreative>,
    pub jobs: Vec<PlannedJob>,
}

/// What the main simulation tells People about one app.
pub struct AppInfo {
    pub kind: AppKind,
    pub stage: &'static str,
    pub created_day: i64,
    pub quality: f64,
    /// The app's planned update (day, quality after), which hired work may replace.
    pub update: Option<(i64, f64)>,
    pub maker: usize,
}

/// How a creative's work moves an app's quality: great work lifts it, generator output sinks it.
pub fn quality_after(app_quality: f64, creative_quality: f64) -> f64 {
    (app_quality + 0.7 * (creative_quality - 0.45)).clamp(0.02, 0.98)
}

/// Chooses creatives and jobs. Returns the plan and the app updates hired work causes:
/// `(app, day, quality after)`.
pub fn plan(rng: &mut Rng, apps: &[AppInfo], days: i64) -> (Plan, Vec<(usize, i64, f64)>) {
    let mut creatives = Vec::new();
    for (i, seed) in CREATIVES.iter().chain(WAITING.iter()).enumerate() {
        let ai = seed.5;
        let quality = match ai {
            "none" => 0.55 + 0.4 * rng.f(),
            "assisted" => 0.45 + 0.4 * rng.f(),
            _ => 0.05 + 0.25 * rng.f(),
        };
        creatives.push(SimCreative {
            seed,
            id: Uuid::now_v7(),
            quality,
            reliability: 0.6 + 0.38 * rng.f(),
            trader: seed.2 == "studio" || rng.chance(0.5),
            accent: i % ACCENT_COLORS.len(),
            live: i < CREATIVES.len(),
        });
    }
    let live: Vec<usize> = (0..CREATIVES.len()).collect();
    let mut jobs = Vec::new();
    let mut updates = Vec::new();

    // Released apps that get an update: about half of those updates bring hired work.
    for (a, app) in apps.iter().enumerate() {
        let Some((day, _)) = app.update else { continue };
        if !rng.chance(0.5) || day < 25 {
            continue;
        }
        let skill = if app.kind == AppKind::Game {
            *rng.pick(&["pixel-art", "2d-art", "music", "sound", "3d", "animation"])
        } else {
            *rng.pick(&["ui-ux", "2d-art", "writing", "localization"])
        };
        let able: Vec<usize> = live
            .iter()
            .copied()
            .filter(|&c| creatives[c].seed.4.contains(&skill))
            .collect();
        if able.is_empty() {
            continue;
        }
        // Better-rated creatives get hired a little more, as they would.
        let weights: Vec<f64> = able.iter().map(|&c| 0.3 + creatives[c].quality).collect();
        let c = able[rng.weighted(&weights).unwrap_or(0)];
        updates.push((a, day, quality_after(app.quality, creatives[c].quality)));
        jobs.push(PlannedJob {
            app: a,
            creative: c,
            skill,
            end: End::Completed,
            day: (day - 20 - rng.below(10) as i64).max(app.created_day),
            ships_on: Some(day),
        });
    }

    // Everyone else: jobs in every state, for any of the maker's apps.
    let ends = [
        (End::Completed, 40.0),
        (End::InProgress, 9.0),
        (End::DeliveredRecently, 5.0),
        (End::DeliveredStale, 5.0),
        (End::CompletedOwing, 3.0),
        (End::Requested, 7.0),
        (End::Offered, 7.0),
        (End::AwaitingPayment, 4.0),
        (End::Declined, 9.0),
        (End::Cancelled, 5.0),
        (End::Withdrawn, 4.0),
    ];
    let weights: Vec<f64> = ends.iter().map(|e| e.1).collect();
    let popularity: Vec<f64> = live.iter().map(|&c| 0.3 + creatives[c].quality).collect();
    for _ in 0..(apps.len() * 3 / 4).max(10) {
        let a = rng.below(apps.len());
        let c = live[rng.weighted(&popularity).unwrap_or(0)];
        let skill = *rng.pick(creatives[c].seed.4);
        let end = ends[rng.weighted(&weights).unwrap_or(0)].0;
        let latest = match end {
            End::Requested | End::Offered | End::AwaitingPayment | End::DeliveredRecently => {
                days - 1
            }
            End::InProgress => days - 3,
            _ => days - 20,
        };
        let day =
            apps[a].created_day + rng.below((latest - apps[a].created_day).max(1) as usize) as i64;
        jobs.push(PlannedJob {
            app: a,
            creative: c,
            skill,
            end,
            day: day.min(latest),
            ships_on: None,
        });
    }
    (Plan { creatives, jobs }, updates)
}

#[derive(Debug, Default, Serialize)]
pub struct PeopleGenerated {
    pub creatives: usize,
    pub jobs: usize,
    pub payments: usize,
    pub reviews: usize,
    pub releases: usize,
    pub auto_completed: u64,
    pub messages: usize,
    pub sign_offs: usize,
    pub changes: usize,
    pub final_files: usize,
    pub problems: usize,
    pub review_disputes: usize,
    pub conversation_texts_deleted: u64,
}

/// The truth a review follows: 👍 likelier the better and more reliable the creative.
pub fn review_chance(quality: f64, reliability: f64) -> f64 {
    1.0 / (1.0 + (-9.0 * (0.75 * quality + 0.25 * reliability - 0.5)).exp())
}

const GOOD: &[&str] = &[
    "Exactly what we asked for, and early.",
    "Players noticed the new art straight away.",
    "Great communication, one painless revision.",
    "Hand-made and it shows. Hiring again.",
    "Turned our placeholder mess into something with a style.",
];
const BAD: &[&str] = &[
    "Looked like straight generator output.",
    "Missed the date twice and the files were the wrong size.",
    "Not what was in the offer.",
    "Fine, but we had to redo half of it ourselves.",
];

/// Writes the plan: creatives and their pages, portfolios, jobs through the real job code,
/// reviews, and the releases that ship updates. Run after the main simulation is written.
#[allow(clippy::too_many_arguments)]
pub async fn write(
    db: &PgPool,
    rng: &mut Rng,
    plan: &Plan,
    app_ids: &[Uuid],
    app_titles: &[String],
    app_slugs: &[String],
    makers: &[(Uuid, String, &'static str)],
    app_makers: &[usize],
    updates: &[(usize, i64)],
    start: OffsetDateTime,
    now: OffsetDateTime,
) -> anyhow::Result<PeopleGenerated> {
    let mut out = PeopleGenerated::default();
    sqlx::query("create table sim_creatives (user_id uuid primary key, quality real not null, reliability real not null)")
        .execute(db)
        .await?;

    // Releases for every update, so "since the new work" has a version to start from.
    for (a, day) in updates {
        sqlx::query(
            "insert into releases (id, app_id, version, commit_sha, status, submitted_by, created_at, reviewed_at, commit_verified)
             values ($1, $2, '1.1.0', repeat('e', 40), 'approved', $3, $4, $4, true)",
        )
        .bind(Uuid::now_v7())
        .bind(app_ids[*a])
        .bind(makers[app_makers[*a]].0)
        .bind(start + Duration::days(*day))
        .execute(db)
        .await?;
        out.releases += 1;
    }

    // Creatives.
    for (i, c) in plan.creatives.iter().enumerate() {
        let (name, handle, kind, team, skills, ai, location, country, headline) = *c.seed;
        sqlx::query(
            "insert into users (id, display_name, country_code, age_16_confirmed_at, adult_confirmed_at, created_at)
             values ($1, $2, $3, $4, $4, $4)",
        )
        .bind(c.id)
        .bind(format!("Sim creative {}", i + 1))
        .bind(country)
        .bind(start)
        .execute(db)
        .await?;
        let skills: Vec<String> = skills.iter().map(|s| s.to_string()).collect();
        let services: Vec<Service> = skills
            .iter()
            .flat_map(|s| service_templates(s).iter().map(move |t| (s, t)))
            .take(if ai == "heavy" { 1 } else { 4 })
            .map(|(s, (n, d, lo, hi, days))| Service {
                id: format!("{:08x}", rng.next_u64() as u32),
                name: if ai == "heavy" {
                    "Any asset".into()
                } else {
                    (*n).into()
                },
                description: if ai == "heavy" {
                    "Any style, delivered in an hour".into()
                } else {
                    (*d).into()
                },
                price_minor: if ai == "heavy" {
                    1_000
                } else {
                    ((lo + rng.below((hi - lo + 1) as usize) as i64) / 500) * 500
                },
                currency: "usd".into(),
                days: if ai == "heavy" { 1 } else { *days },
                skill: s.clone(),
            })
            .collect();
        let links = vec![Link {
            label: "Portfolio site".into(),
            url: format!("https://{handle}.example"),
        }];
        let (accent, color) = ACCENT_COLORS[c.accent];
        let mut sections: Vec<&str> = people::SECTIONS.to_vec();
        if skills.iter().any(|s| s == "music" || s == "sound") {
            sections.swap(0, 1);
        }
        let taking = rng.chance(0.85);
        let stripe = if i % 9 == 8 { "none" } else { "active" };
        let about = format!(
            "**{name}** — {headline}\n\nI work with small teams and solo developers. Tell me about your game, send references, and I'll reply with an offer.\n\n- Usual turnaround: {}\n- Formats: whatever your engine needs",
            if kind == "studio" {
                "1–3 weeks"
            } else {
                "1–2 weeks"
            }
        );
        sqlx::query(
            "insert into creatives (user_id, handle, name, kind, team_size, headline, about_md, location, skills, ai_use,
                                    ai_note, trader, taking_work, turnaround, services, accent, sections, links,
                                    stripe_account_id, stripe_status, created_at, updated_at)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $21)",
        )
        .bind(c.id)
        .bind(handle)
        .bind(name)
        .bind(kind)
        .bind((team > 1).then_some(team))
        .bind(headline)
        .bind(about)
        .bind(location)
        .bind(&skills)
        .bind(ai)
        .bind(if ai == "heavy" { "Generated with image and music models, lightly edited." } else { "" })
        .bind(c.trader)
        .bind(taking)
        .bind(if kind == "studio" { "1–3 weeks" } else { "1–2 weeks" })
        .bind(sqlx::types::Json(&services))
        .bind(accent)
        .bind(sections.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        .bind(sqlx::types::Json(&links))
        .bind((stripe == "active").then(|| format!("acct_sim_{handle}")))
        .bind(stripe)
        .bind(start)
        .execute(db)
        .await?;
        sqlx::query(
            "insert into sim_creatives (user_id, quality, reliability) values ($1, $2, $3)",
        )
        .bind(c.id)
        .bind(c.quality as f32)
        .bind(c.reliability as f32)
        .execute(db)
        .await?;

        // A portfolio: pictures made on the spot, one per work.
        let mut works = Vec::new();
        for s in &skills {
            for t in work_titles(s)
                .iter()
                .take(if skills.len() > 2 { 2 } else { 3 })
            {
                let path = format!("works/{}.svg", t.to_lowercase().replace(' ', "-"));
                let picture = svg_picture(&path, 640, 480, color, glyph(s), t);
                works.push(if s == "music" || s == "sound" || s == "voice" {
                    // No audio files in the sandbox: a cover with a caption stands in.
                    Work::Image {
                        picture,
                        title: Some((*t).into()),
                        caption: Some("Cover (the sandbox has no audio files)".into()),
                        skill: Some(s.clone()),
                    }
                } else {
                    Work::Image {
                        picture,
                        title: Some((*t).into()),
                        caption: None,
                        skill: Some(s.clone()),
                    }
                });
            }
        }
        let set = Uuid::now_v7();
        let initials: String = name
            .split_whitespace()
            .filter_map(|w| w.chars().next())
            .take(2)
            .collect();
        sqlx::query(
            "insert into portfolio_sets (id, creative_id, repo_url, commit_sha, avatar, banner, works, fingerprint, status, source, created_at, reviewed_at, reviewed_by)
             values ($1, $2, $3, repeat('a', 40), $4, $5, $6, $7, $8, 'sim', $9, $9, $10)",
        )
        .bind(set)
        .bind(c.id)
        .bind(format!("https://github.com/sim/{handle}-portfolio"))
        .bind(sqlx::types::Json(svg_picture("avatar.svg", 256, 256, color, &initials, "")))
        .bind(sqlx::types::Json(svg_picture("banner.svg", 1600, 400, color, glyph(&skills[0]), name)))
        .bind(sqlx::types::Json(&works))
        .bind(format!("sim-{handle}"))
        .bind(if c.live { "approved" } else { "pending" })
        .bind(start)
        .bind(c.live.then_some("simulation"))
        .execute(db)
        .await?;
        if c.live {
            sqlx::query("update creatives set portfolio_set_id = $2 where user_id = $1")
                .bind(c.id)
                .bind(set)
                .execute(db)
                .await?;
        }
        out.creatives += 1;
    }

    // Jobs, through the real job code where it decides something (payments, credit).
    let at = |day: i64, rng: &mut Rng| {
        start + Duration::days(day) + Duration::minutes(rng.below(1_200) as i64)
    };
    for job in &plan.jobs {
        let c = &plan.creatives[job.creative];
        let (maker_id, maker_name, maker_country) = &makers[app_makers[job.app]];
        let id = Uuid::now_v7();
        let requested = at(job.day, rng);
        let template = *rng.pick(service_templates(job.skill));
        let price = if c.seed.5 == "heavy" {
            1_000
        } else {
            ((template.2 + rng.below((template.3 - template.2 + 1) as usize) as i64) / 500) * 500
        };
        let business = rng.chance(0.4);
        let inquiry = rng.chance(0.2);
        sqlx::query(
            "insert into jobs (id, creative_id, client_id, app_id, skill, status, request_md, client_is_business, created_at, updated_at,
                               inquiry, request_ack_at, conversation_requested_at, conversation_request_note)
             values ($1, $2, $3, $4, $5, 'requested', $6, $7, $8, $8, $9, $8, case when $9 then $8 end,
                     case when $9 then 'Please review my request: your opinion, how long it would take, and anything unclear.' end)",
        )
        .bind(id)
        .bind(c.id)
        .bind(maker_id)
        .bind(app_ids[job.app])
        .bind(job.skill)
        .bind(format!(
            "Hi! We need {} for {}. Happy to share references and the current build.",
            template.0.to_lowercase(),
            app_titles[job.app]
        ))
        .bind(business)
        .bind(requested)
        .bind(inquiry)
        .execute(db)
        .await?;
        event(
            db,
            id,
            Some(*maker_id),
            if inquiry { "inquiry" } else { "requested" },
            None,
            requested,
        )
        .await?;
        // Inquiries: the creative usually opens a conversation to talk it through first.
        if inquiry && (job.end != End::Requested || rng.chance(0.5)) {
            out.messages += converse(
                db,
                rng,
                id,
                *maker_id,
                c.id,
                requested + Duration::hours(3),
                job.end == End::Requested,
            )
            .await?;
        }
        if job.end == End::Requested {
            out.jobs += 1;
            continue;
        }

        let offered_at = requested + Duration::hours(6 + rng.below(40) as i64);
        let payment = *rng.pick(&["split", "split", "split", "upfront", "on_delivery"]);
        let offer = Offer {
            title: template.0.to_string(),
            scope_md: template.1.to_string(),
            price_minor: price,
            currency: "usd".into(),
            payment: payment.into(),
            due_date: (offered_at + Duration::days(i64::from(template.4) + 7)).date().to_string(),
            sign_off: c.seed.5 != "heavy" && rng.chance(0.35),
            revisions: 0,
            rights: if rng.chance(0.7) { "transfer".into() } else { "licence".into() },
            open_source: rng.chance(0.3),
            credit: rng.chance(0.92),
            ai_use: c.seed.5.into(),
            refund_policy: "Full refund before work starts. After that, refunds cover the parts not yet delivered.".into(),
        };
        sqlx::query("update jobs set status = 'offered', offer = $2, offer_count = 1, offered_at = $3, updated_at = $3 where id = $1")
            .bind(id)
            .bind(sqlx::types::Json(&offer))
            .bind(offered_at)
            .execute(db)
            .await?;
        event(db, id, Some(c.id), "offered", None, offered_at).await?;
        if matches!(job.end, End::Offered) {
            out.jobs += 1;
            continue;
        }
        if job.end == End::Declined {
            let by = if rng.chance(0.5) { *maker_id } else { c.id };
            let when = offered_at + Duration::hours(20);
            sqlx::query("update jobs set status = 'declined', ended_at = $2, ended_by = $3, updated_at = $2 where id = $1")
                .bind(id)
                .bind(when)
                .bind(by)
                .execute(db)
                .await?;
            event(
                db,
                id,
                Some(by),
                "declined",
                Some("Not the right fit this time."),
                when,
            )
            .await?;
            out.jobs += 1;
            continue;
        }

        // Accepted: the real agreement text and payment plan.
        let accepted = offered_at + Duration::hours(4 + rng.below(30) as i64);
        let withdrawal_days: Option<i16> = if c.trader && !business && *maker_country != "US" {
            Some(14)
        } else {
            None
        };
        let text = jobs::agreement(
            &offer,
            &Parties {
                job_id: id,
                creative_name: c.seed.0,
                creative_handle: c.seed.1,
                trader: c.trader,
                client_name: maker_name,
                project_title: &app_titles[job.app],
                project_slug: &app_slugs[job.app],
                skill: job.skill,
                withdrawal_days,
            },
        );
        let sha = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(text.as_bytes()));
        let plan_pay = jobs::schedule(&offer);
        let due_now = plan_pay.iter().any(|(_, on, _)| *on == "accept");
        sqlx::query(
            "update jobs set status = $2, agreement_text = $3, agreement_sha256 = $4, agreement_template = $5,
                    accepted_at = $6, withdrawal_until = $7, start_now_ack_at = $8, started_at = $9, updated_at = $6,
                    exact_terms_ack_at = $6, due_date = $10::date
             where id = $1",
        )
        .bind(id)
        .bind(if due_now { "awaiting_payment" } else { "in_progress" })
        .bind(&text)
        .bind(&sha)
        .bind(jobs::TEMPLATE_VERSION)
        .bind(accepted)
        .bind(withdrawal_days.map(|d| accepted + Duration::days(d.into())))
        .bind(withdrawal_days.map(|_| accepted))
        .bind((!due_now).then_some(accepted))
        .bind(&offer.due_date)
        .execute(db)
        .await?;
        let mut payment_ids = Vec::new();
        for (part, on, amount) in &plan_pay {
            let pid = Uuid::now_v7();
            sqlx::query(
                "insert into job_payments (id, job_id, part, due_on, amount_minor, currency, status) values ($1, $2, $3, $4, $5, 'usd', $6)",
            )
            .bind(pid)
            .bind(id)
            .bind(part)
            .bind(on)
            .bind(*amount as i32)
            .bind(if *on == "accept" { "due" } else { "waiting" })
            .execute(db)
            .await?;
            payment_ids.push((pid, *on, *amount));
        }
        event(db, id, Some(*maker_id), "accepted", None, accepted).await?;
        let pay = |pid: Uuid, amount: i64, n: usize| Paid {
            job_payment_id: pid,
            stripe_account_id: Some(format!("acct_sim_{}", c.seed.1)),
            object_id: format!("sim_{id}_{n}"),
            payer: Some(*maker_id),
            amount_minor: amount,
            fee_minor: people::stripe::fee(amount),
            simulated: true,
        };
        if job.end == End::AwaitingPayment {
            out.jobs += 1;
            continue;
        }
        if job.end == End::Withdrawn && withdrawal_days.is_none() {
            // Only consumers hiring a business can withdraw; anyone else cancels instead.
            cancel(db, id, *maker_id, accepted + Duration::days(1)).await?;
            out.jobs += 1;
            continue;
        }
        let first_paid = accepted + Duration::hours(2);
        for (n, (pid, on, amount)) in payment_ids.iter().enumerate() {
            if *on == "accept" {
                jobs::payment_succeeded_at(db, pay(*pid, *amount, n), Some(first_paid)).await?;
                out.payments += 1;
            }
        }
        // Sign-off on the direction, when the offer asks for one.
        let mut work_day = first_paid + Duration::days(2);
        if offer.sign_off && (job.end != End::InProgress || rng.chance(0.5)) {
            event(
                db,
                id,
                Some(c.id),
                "sent_for_sign_off",
                Some("Rough sketch attached."),
                work_day,
            )
            .await?;
            sqlx::query("update jobs set signed_off_at = $2 where id = $1")
                .bind(id)
                .bind(work_day + Duration::hours(8))
                .execute(db)
                .await?;
            event(
                db,
                id,
                Some(*maker_id),
                "signed_off",
                None,
                work_day + Duration::hours(8),
            )
            .await?;
            out.sign_offs += 1;
            work_day += Duration::days(1);
        }
        // Changes to Agreement: new work with its own price and date.
        let finishing = matches!(
            job.end,
            End::Completed | End::CompletedOwing | End::DeliveredRecently | End::DeliveredStale
        );
        if (finishing && rng.chance(0.2)) || (job.end == End::InProgress && rng.chance(0.3)) {
            let accepted_change = finishing;
            out.changes += 1;
            let extra = if rng.chance(0.75) {
                (price / 4 / 500).max(2) * 500
            } else {
                0
            };
            let terms = jobs::ChangeTerms {
                number: 1,
                title: "Add a second variant",
                scope_md: "One more colour variant of everything delivered, same sizes.",
                extra_price_minor: extra,
                currency: "usd",
                new_due_date: &(work_day + Duration::days(10)).date().to_string(),
                requested_note: Some("Could we get a second colour variant?"),
            };
            let parties = Parties {
                job_id: id,
                creative_name: c.seed.0,
                creative_handle: c.seed.1,
                trader: c.trader,
                client_name: maker_name,
                project_title: &app_titles[job.app],
                project_slug: &app_slugs[job.app],
                skill: job.skill,
                withdrawal_days,
            };
            let doc = jobs::change_document(&terms, &parties, &sha);
            let change_id = Uuid::now_v7();
            sqlx::query(
                "insert into job_changes (id, job_id, number, requested_note, status, title, scope_md, extra_price_minor,
                                          new_due_date, text, sha256, created_at, offered_at, decided_at)
                 values ($1, $2, 1, $3, $4, $5, $6, $7, $8::date, $9, $10, $11, $12, $13)",
            )
            .bind(change_id)
            .bind(id)
            .bind(terms.requested_note)
            .bind(if accepted_change { "accepted" } else if rng.chance(0.5) { "offered" } else { "requested" })
            .bind(terms.title)
            .bind(terms.scope_md)
            .bind(extra as i32)
            .bind(terms.new_due_date)
            .bind(accepted_change.then_some(&doc))
            .bind(accepted_change.then(|| hex::encode(<sha2::Sha256 as sha2::Digest>::digest(doc.as_bytes()))))
            .bind(work_day)
            .bind(work_day + Duration::hours(6))
            .bind(accepted_change.then_some(work_day + Duration::hours(20)))
            .execute(db)
            .await?;
            event(
                db,
                id,
                Some(*maker_id),
                "change_requested",
                terms.requested_note,
                work_day,
            )
            .await?;
            if accepted_change {
                event(
                    db,
                    id,
                    Some(*maker_id),
                    "change_accepted",
                    Some("Change to Agreement 1"),
                    work_day + Duration::hours(20),
                )
                .await?;
                sqlx::query("update jobs set due_date = $2::date where id = $1")
                    .bind(id)
                    .bind(terms.new_due_date)
                    .execute(db)
                    .await?;
                if extra > 0 {
                    let pid = Uuid::now_v7();
                    sqlx::query(
                        "insert into job_payments (id, job_id, part, due_on, amount_minor, currency, status, change_id)
                         values ($1, $2, 'change', 'accept', $3, 'usd', 'due', $4)",
                    )
                    .bind(pid)
                    .bind(id)
                    .bind(extra as i32)
                    .bind(change_id)
                    .execute(db)
                    .await?;
                    jobs::payment_succeeded_at(
                        db,
                        pay(pid, extra, 5),
                        Some(work_day + Duration::hours(22)),
                    )
                    .await?;
                    out.payments += 1;
                }
            }
        }
        match job.end {
            End::InProgress => {
                out.jobs += 1;
                continue;
            }
            End::Withdrawn => {
                let when = accepted + Duration::days(3);
                sqlx::query("update jobs set status = 'withdrawn', withdrawn_at = $2, ended_at = $2, ended_by = $3, updated_at = $2 where id = $1")
                    .bind(id)
                    .bind(when)
                    .bind(maker_id)
                    .execute(db)
                    .await?;
                event(db, id, Some(*maker_id), "withdrawn", None, when).await?;
                out.jobs += 1;
                continue;
            }
            End::Cancelled => {
                cancel(db, id, c.id, accepted + Duration::days(5)).await?;
                out.jobs += 1;
                continue;
            }
            _ => {}
        }

        // Delivered.
        let delivered = match (job.end, job.ships_on) {
            (End::DeliveredRecently, _) => now - Duration::days(2),
            (End::DeliveredStale, _) => now - Duration::days(9),
            (_, Some(ships)) => start + Duration::days(ships - 4),
            _ => accepted + Duration::days(i64::from(template.4)),
        };
        let delivered = delivered
            .max(accepted + Duration::hours(6))
            .min(now - Duration::hours(1));
        sqlx::query("update jobs set status = 'delivered', delivered_at = $2, updated_at = $2 where id = $1")
            .bind(id)
            .bind(delivered)
            .execute(db)
            .await?;
        let preview = sqlx::query("update job_payments set status = 'due' where job_id = $1 and due_on = 'delivery' and status = 'waiting'")
            .bind(id)
            .execute(db)
            .await?
            .rows_affected()
            > 0;
        event(
            db,
            id,
            Some(c.id),
            if preview {
                "delivered_preview"
            } else {
                "delivered"
            },
            Some(if preview {
                "Watermarked preview in the shared folder."
            } else {
                "All files are in the shared folder."
            }),
            delivered,
        )
        .await?;
        if matches!(job.end, End::DeliveredRecently | End::DeliveredStale) {
            out.jobs += 1;
            continue;
        }

        // Completed: paying the rest accepts the work; otherwise "Looks good" or the 7 days.
        let done = delivered + Duration::days(1);
        let final_due: Vec<_> = payment_ids
            .iter()
            .filter(|(_, on, _)| *on == "delivery")
            .collect();
        if job.end == End::CompletedOwing || final_due.is_empty() {
            let automatic = job.end == End::CompletedOwing;
            let when = if automatic {
                delivered + jobs::COMPLETION_WINDOW + Duration::hours(1)
            } else {
                done
            };
            sqlx::query("update jobs set status = 'completed', completed_at = $2, completed_automatically = $3, updated_at = $2 where id = $1")
                .bind(id)
                .bind(when.min(now))
                .bind(automatic)
                .execute(db)
                .await?;
            event(
                db,
                id,
                (!automatic).then_some(*maker_id),
                if automatic {
                    "completed_automatically"
                } else {
                    "completed"
                },
                None,
                when.min(now),
            )
            .await?;
            let mut conn = db.acquire().await?;
            jobs::maybe_credit(&mut conn, id, when.min(now)).await?;
        } else {
            for (n, (pid, _, amount)) in final_due.iter().enumerate() {
                jobs::payment_succeeded_at(db, pay(*pid, *amount, 10 + n), Some(done)).await?;
                out.payments += 1;
            }
            // Paid for the preview: the creative sends the final files (most already have).
            if rng.chance(0.93) {
                let mut conn = db.acquire().await?;
                jobs::send_final_files(
                    &mut conn,
                    id,
                    c.id,
                    "Final files, full resolution.",
                    &[format!("https://files.example/{}/final.zip", c.seed.1)],
                    done + Duration::hours(10),
                )
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
                out.final_files += 1;
            }
        }

        // Some jobs hit a problem: both sides state their case and propose outcomes.
        if rng.chance(0.08) {
            out.problems += 1;
            problem(
                db,
                rng,
                id,
                *maker_id,
                c.id,
                price,
                done + Duration::days(1),
            )
            .await?;
        }

        // A review from the client, most of the time.
        if rng.chance(0.8) {
            let up = rng.chance(review_chance(c.quality, c.reliability));
            let body = if up { *rng.pick(GOOD) } else { *rng.pick(BAD) };
            sqlx::query(
                "insert into creative_reviews (job_id, creative_id, client_id, vote, body, reply, replied_at, created_at, updated_at)
                 values ($1, $2, $3, $4, $5, $6, $7, $8, $8)",
            )
            .bind(id)
            .bind(c.id)
            .bind(maker_id)
            .bind(if up { 1i16 } else { -1 })
            .bind(body)
            .bind((!up && rng.chance(0.5)).then_some("Sorry this didn't land. I've offered a partial refund."))
            .bind((!up).then_some(done + Duration::days(2)))
            .bind(done + Duration::days(1))
            .execute(db)
            .await?;
            out.reviews += 1;
            // Creatives dispute some bad reviews; a mediator decides on the rules, never on the
            // vote (HUM-18).
            if !up && rng.chance(0.35) {
                out.review_disputes += 1;
                let (reason, statement) = *rng.pick(&[
                    ("false_fact", "The review says the files were the wrong size; the agreement and delivery show the agreed sizes."),
                    ("extortion", "They said they'd leave a bad review unless I added more work for free."),
                    ("not_about_job", "This is about a different project than the one on this job."),
                ]);
                let r = rng.f();
                let (status, hidden, mediator_note, why) = if r < 0.4 {
                    ("pending", false, None, None)
                } else if r < 0.75 {
                    (
                        "kept",
                        false,
                        None,
                        Some("The review is the client's opinion of the work; it breaks no rule."),
                    )
                } else if r < 0.92 {
                    (
                        "noted",
                        false,
                        Some("The job record shows delivery at the sizes in the agreement."),
                        Some(
                            "A factual claim in the review differs from the job record; a note says so.",
                        ),
                    )
                } else {
                    (
                        "removed",
                        true,
                        None,
                        Some(
                            "The recorded conversation shows the review was used as a threat to get free work.",
                        ),
                    )
                };
                sqlx::query(
                    "insert into review_disputes (job_id, reason, statement, client_statement, status, decision_reason, decided_by, created_at, decided_at)
                     values ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                )
                .bind(id)
                .bind(reason)
                .bind(statement)
                .bind(rng.chance(0.6).then_some("I stand by my review."))
                .bind(status)
                .bind(why)
                .bind((status != "pending").then_some("Sim mediator"))
                .bind(done + Duration::days(3))
                .bind((status != "pending").then_some(done + Duration::days(5)))
                .execute(db)
                .await?;
                sqlx::query("update creative_reviews set hidden_at = case when $2 then $4::timestamptz end, hidden_reason = case when $2 then $5 end, mediator_note = $3 where job_id = $1")
                    .bind(id)
                    .bind(hidden)
                    .bind(mediator_note)
                    .bind(done + Duration::days(5))
                    .bind(why)
                    .execute(db)
                    .await?;
            }
        }
        out.jobs += 1;
    }

    // Jobs delivered over a week ago with no answer: the real hourly job completes them.
    out.auto_completed = jobs::auto_complete(db, now).await?;
    // Conversations closed over 30 days ago: the real cleanup deletes the text, keeping fingerprints.
    out.conversation_texts_deleted = jobs::forget_conversations(db, now).await?;
    Ok(out)
}

const LINES_CLIENT: &[&str] = &[
    "Hi! Before we sign anything: is the brief clear enough?",
    "We're thinking 32×32. Would that work for the style?",
    "Roughly how long would this take you?",
    "Great, that makes sense. Please send an offer with that wording.",
];
const LINES_CREATIVE: &[&str] = &[
    "Thanks for asking first. Two questions: which engine, and do you need transparent backgrounds?",
    "About two weeks. I'd suggest a sketch sign-off before the final pass.",
    "I'll write the offer so the sizes and file formats are exact.",
    "Sending the offer now. Please read the scope carefully: it's exactly what you'll get.",
];

/// A recorded conversation the creative opens, with fingerprints from the real code. Closed
/// afterwards unless `keep_open`.
async fn converse(
    db: &PgPool,
    rng: &mut Rng,
    job: Uuid,
    client: Uuid,
    creative: Uuid,
    from: OffsetDateTime,
    keep_open: bool,
) -> anyhow::Result<usize> {
    sqlx::query(
        "update jobs set conversation_open = true, conversation_requested_at = null, conversation_request_note = null,
                client_recording_ack_at = $2, creative_recording_ack_at = $2 where id = $1",
    )
    .bind(job)
    .bind(from)
    .execute(db)
    .await?;
    event(db, job, Some(creative), "conversation_opened", None, from).await?;
    let mut prev: Option<String> = None;
    let n = 2 + rng.below(3);
    for k in 0..n {
        let (author, body) = if k % 2 == 0 {
            (creative, LINES_CREATIVE[k / 2 % LINES_CREATIVE.len()])
        } else {
            (client, LINES_CLIENT[k / 2 % LINES_CLIENT.len()])
        };
        let id = Uuid::now_v7();
        let sha = jobs::message_fingerprint(prev.as_deref(), id, author, body);
        sqlx::query("insert into job_messages (id, job_id, author_id, body, sha256, created_at) values ($1, $2, $3, $4, $5, $6)")
            .bind(id)
            .bind(job)
            .bind(author)
            .bind(body)
            .bind(&sha)
            .bind(from + Duration::minutes(20 * k as i64 + 5))
            .execute(db)
            .await?;
        prev = Some(sha);
    }
    if !keep_open {
        let closed = from + Duration::hours(3);
        sqlx::query(
            "update jobs set conversation_open = false, conversation_closed_at = $2 where id = $1",
        )
        .bind(job)
        .bind(closed)
        .execute(db)
        .await?;
        event(db, job, Some(creative), "conversation_closed", None, closed).await?;
    }
    Ok(n)
}

/// A problem: a statement, an answer, proposals, and an outcome (agreed, a mediator's
/// suggestion still open, or closed unresolved).
async fn problem(
    db: &PgPool,
    rng: &mut Rng,
    job: Uuid,
    client: Uuid,
    creative: Uuid,
    price: i64,
    at: OffsetDateTime,
) -> anyhow::Result<()> {
    let by_client = rng.chance(0.7);
    let (opener, other) = if by_client {
        (client, creative)
    } else {
        (creative, client)
    };
    let statement = if by_client {
        "The delivery doesn't match the scope: the agreement says 4 directions and we got 3."
    } else {
        "The client has used the preview in their release but hasn't paid the second half."
    };
    let id = Uuid::now_v7();
    let r = rng.f();
    let status = if r < 0.55 {
        "resolved"
    } else if r < 0.8 {
        "open"
    } else {
        "unresolved"
    };
    sqlx::query(
        "insert into job_problems (id, job_id, opened_by, statement, reply, replied_at, status, mediator_suggestion, suggested_by,
                                   suggested_at, closing_note, created_at, closed_at)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
    )
    .bind(id)
    .bind(job)
    .bind(opener)
    .bind(statement)
    .bind("Fair point; let's sort it out here.")
    .bind(at + Duration::hours(5))
    .bind(status)
    .bind((status != "resolved").then_some("From the agreement and the delivery, the fair outcome looks like a partial refund for the missing part."))
    .bind((status != "resolved").then_some("Sim mediator"))
    .bind((status != "resolved").then_some(at + Duration::days(1)))
    .bind(match status {
        "resolved" => Some("Both agreed: refund"),
        "unresolved" => Some("Closed unresolved: both have the record for a card dispute or a court."),
        _ => None,
    })
    .bind(at)
    .bind((status != "open").then_some(at + Duration::days(2)))
    .execute(db)
    .await?;
    event(db, job, Some(opener), "problem_opened", Some(statement), at).await?;
    let refund = (price / 4 / 100).max(1) * 100;
    sqlx::query(
        "insert into job_problem_proposals (id, problem_id, proposed_by, kind, amount_minor, note, status, created_at, decided_at)
         values ($1, $2, $3, 'refund', $4, 'A partial refund for the missing part.', $5, $6, $7)",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(other)
    .bind(refund as i32)
    .bind(match status { "resolved" => "accepted", "unresolved" => "rejected", _ => "pending" })
    .bind(at + Duration::hours(6))
    .bind((status != "open").then_some(at + Duration::days(2)))
    .execute(db)
    .await?;
    if status == "resolved" {
        let mut conn = db.acquire().await?;
        jobs::simulated_refund(&mut conn, job, refund).await?;
    }
    Ok(())
}

async fn event(
    db: &PgPool,
    job: Uuid,
    actor: Option<Uuid>,
    kind: &str,
    note: Option<&str>,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query("insert into job_events (id, job_id, actor_id, kind, note, created_at) values ($1, $2, $3, $4, $5, $6)")
        .bind(Uuid::now_v7())
        .bind(job)
        .bind(actor)
        .bind(kind)
        .bind(note)
        .bind(at)
        .execute(db)
        .await?;
    sqlx::query("update jobs set updated_at = greatest(updated_at, $2) where id = $1")
        .bind(job)
        .bind(at)
        .execute(db)
        .await?;
    Ok(())
}

async fn cancel(db: &PgPool, job: Uuid, by: Uuid, at: OffsetDateTime) -> sqlx::Result<()> {
    sqlx::query("update jobs set status = 'cancelled', ended_at = $2, ended_by = $3, updated_at = $2 where id = $1")
        .bind(job)
        .bind(at)
        .bind(by)
        .execute(db)
        .await?;
    sqlx::query("update job_payments set status = 'waiting' where job_id = $1 and status = 'due'")
        .bind(job)
        .execute(db)
        .await?;
    event(
        db,
        job,
        Some(by),
        "cancelled",
        Some("Plans changed on our side."),
        at,
    )
    .await
}

// ---------------------------------------------------------------------------------------------
// The report

#[derive(Debug, Default, Serialize)]
pub struct PeopleReport {
    pub creatives: i64,
    pub live_profiles: i64,
    pub portfolios_waiting_for_review: i64,
    pub jobs_by_status: BTreeMap<String, i64>,
    /// Simulated payments recorded, their total and Slop's fees.
    pub payments: i64,
    pub paid_total_usd: f64,
    pub slop_fees_usd: f64,
    /// Payments whose fee isn't exactly 5% (rounded): must be 0.
    pub fees_not_5_percent: i64,
    /// Jobs credited; and jobs where credit disagrees with the rule (completed, fully paid,
    /// at least $10): must be 0.
    pub credited_jobs: i64,
    pub credit_rule_violations: i64,
    /// Completed jobs with a final payment still owed (no credit until it's paid).
    pub completed_owing: i64,
    /// Completed by the real auto-complete, 7 days after delivery.
    pub completed_automatically: i64,
    /// Apps with the "Made with people" badge (a credited job), and of those, how many list
    /// credits on their page (real `credits()`; a creative can ask not to be credited).
    pub badge_apps: usize,
    pub badge_apps_listing_credits: usize,
    pub reviews: i64,
    /// Must be 0: reviews on jobs that aren't completed, or not by the job's client.
    pub reviews_not_completed: i64,
    pub reviews_not_by_client: i64,
    /// Creatives' % Not Slop (real code) vs their true quality.
    pub rated_creatives: usize,
    pub creative_rating_truth_correlation: f64,
    pub no_ai_average_percent: f64,
    pub heavy_ai_average_percent: f64,
    /// Apps that shipped hired work in an update: does "since the new work" (real code) move
    /// the way the true quality moved, compared with the votes from before?
    pub hired_updates_checked: usize,
    pub since_new_work_moved_right_way: usize,
    /// Average "since the new work" minus the before-rating, for work that helped / hurt.
    pub uplift_when_work_was_good: f64,
    pub change_when_work_was_generated: f64,
    /// The overall rating is untouched by hiring (HUM-13): apps where tally == recount.
    pub overall_rating_untouched: usize,
    /// Conversations: messages, how many still have text, and copies that verify against the
    /// fingerprints (every conversation must: `conversations_failing_verification` = 0).
    pub messages: i64,
    pub messages_with_text: i64,
    pub conversations_verified: usize,
    pub conversations_failing_verification: usize,
    pub inquiries: i64,
    /// Changes to Agreement by status, and the extra paid for accepted ones.
    pub changes_by_status: BTreeMap<String, i64>,
    pub changes_extra_paid_usd: f64,
    pub signed_off: i64,
    /// Must be 0: final files sent before the delivery payment was paid.
    pub final_files_before_payment: i64,
    pub waiting_for_final_files: i64,
    pub problems_by_status: BTreeMap<String, i64>,
    pub agreed_refunds_usd: f64,
    pub review_disputes_by_status: BTreeMap<String, i64>,
    /// Must be 0: hidden reviews counted in a creative's rating.
    pub hidden_reviews_counted: i64,
}

pub async fn report(db: &PgPool) -> anyhow::Result<PeopleReport> {
    let mut r = PeopleReport::default();
    let exists: bool = sqlx::query_scalar("select to_regclass('sim_creatives') is not null")
        .fetch_one(db)
        .await?;
    if !exists {
        return Ok(r);
    }
    r.creatives = sqlx::query_scalar("select count(*) from creatives")
        .fetch_one(db)
        .await?;
    r.live_profiles = sqlx::query_scalar(
        "select count(*) from creatives where portfolio_set_id is not null and hidden_reason is null",
    )
    .fetch_one(db)
    .await?;
    r.portfolios_waiting_for_review =
        sqlx::query_scalar("select count(*) from portfolio_sets where status = 'pending'")
            .fetch_one(db)
            .await?;
    let statuses: Vec<(String, i64)> =
        sqlx::query_as("select status, count(*) from jobs group by status order by status")
            .fetch_all(db)
            .await?;
    r.jobs_by_status = statuses.into_iter().collect();
    let (n, total, fees, wrong): (i64, i64, i64, i64) = sqlx::query_as(
        "select count(*), coalesce(sum(amount_minor), 0)::int8, coalesce(sum(slop_fee_minor), 0)::int8,
                count(*) filter (where slop_fee_minor <> (amount_minor * 500 + 5000) / 10000)
         from payments where kind = 'people_job'",
    )
    .fetch_one(db)
    .await?;
    r.payments = n;
    r.paid_total_usd = total as f64 / 100.0;
    r.slop_fees_usd = fees as f64 / 100.0;
    r.fees_not_5_percent = wrong;
    r.credited_jobs = sqlx::query_scalar("select count(*) from jobs where credited_at is not null")
        .fetch_one(db)
        .await?;
    r.credit_rule_violations = sqlx::query_scalar(
        "select count(*) from jobs j where (j.credited_at is not null) <> (
             j.status = 'completed'
             and not exists (select 1 from job_payments p where p.job_id = j.id and p.status <> 'paid')
             and exists (select 1 from job_payments p where p.job_id = j.id)
             and (select sum(amount_minor) from job_payments p where p.job_id = j.id) >= 1000)",
    )
    .fetch_one(db)
    .await?;
    r.completed_owing = sqlx::query_scalar(
        "select count(*) from jobs j where status = 'completed'
           and exists (select 1 from job_payments p where p.job_id = j.id and p.status <> 'paid')",
    )
    .fetch_one(db)
    .await?;
    r.completed_automatically =
        sqlx::query_scalar("select count(*) from jobs where completed_automatically")
            .fetch_one(db)
            .await?;
    let credited_apps: Vec<Uuid> =
        sqlx::query_scalar("select distinct app_id from jobs where credited_at is not null")
            .fetch_all(db)
            .await?;
    r.badge_apps = credited_apps.len();
    for app in &credited_apps {
        if !people::credits(db, *app).await?.is_empty() {
            r.badge_apps_listing_credits += 1;
        }
    }
    r.reviews = sqlx::query_scalar("select count(*) from creative_reviews")
        .fetch_one(db)
        .await?;
    r.reviews_not_completed = sqlx::query_scalar(
        "select count(*) from creative_reviews r join jobs j on j.id = r.job_id where j.status not in ('completed', 'final_files')",
    )
    .fetch_one(db)
    .await?;
    r.reviews_not_by_client = sqlx::query_scalar(
        "select count(*) from creative_reviews r join jobs j on j.id = r.job_id where r.client_id <> j.client_id",
    )
    .fetch_one(db)
    .await?;

    // Creatives' ratings, from the real code, against the truth.
    let truth: Vec<(Uuid, f32, String)> = sqlx::query_as(
        "select s.user_id, s.quality, c.ai_use from sim_creatives s join creatives c on c.user_id = s.user_id",
    )
    .fetch_all(db)
    .await?;
    let mut pairs = Vec::new();
    let (mut none, mut heavy) = (Vec::new(), Vec::new());
    for (id, q, ai) in truth {
        let rating = people::rating_of(db, id).await?;
        if let Some(p) = rating.percent {
            pairs.push((f64::from(p), f64::from(q)));
            match ai.as_str() {
                "none" => none.push(f64::from(p)),
                "heavy" => heavy.push(f64::from(p)),
                _ => {}
            }
        }
    }
    r.rated_creatives = pairs.len();
    r.creative_rating_truth_correlation = crate::sim::correlation(&pairs);
    let avg = |v: &[f64]| {
        if v.is_empty() {
            0.0
        } else {
            v.iter().sum::<f64>() / v.len() as f64
        }
    };
    r.no_ai_average_percent = avg(&none);
    r.heavy_ai_average_percent = avg(&heavy);

    // Hired work shipped in an update: "since the new work" against the votes from before.
    let hired: Vec<(Uuid, f32, f32)> = sqlx::query_as(
        "select distinct s.app_id, s.quality, s.quality_after from sim_apps s
         join jobs j on j.app_id = s.app_id and j.credited_at is not null
         join releases r on r.app_id = s.app_id and r.status = 'approved' and r.reviewed_at >= j.credited_at
         where s.quality_after is not null",
    )
    .fetch_all(db)
    .await?;
    let (mut ups, mut downs) = (Vec::new(), Vec::new());
    for (app, q0, q1) in hired {
        let Some(since) = people::since_new_work(db, app).await? else {
            continue;
        };
        let (Some(from), Some(tally)) = (since.since, since.tally) else {
            continue;
        };
        let before: Option<f64> = sqlx::query_scalar(
            "select (sum(weight) filter (where vote = 1) / nullif(sum(weight), 0))::float8 from (
                 select distinct on (user_id) vote, weight from vote_events
                 where app_id = $1 and created_at < $2 and vote <> 0 order by user_id, created_at desc) x",
        )
        .bind(app)
        .bind(from)
        .fetch_one(db)
        .await?;
        let after = if tally.not_slop_weight + tally.slop_weight > 0.0 {
            Some(tally.not_slop_weight / (tally.not_slop_weight + tally.slop_weight))
        } else {
            None
        };
        if let (Some(b), Some(a)) = (before, after)
            && tally.not_slop + tally.slop >= 3
        {
            r.hired_updates_checked += 1;
            if (a - b).signum() == f64::from(q1 - q0).signum() {
                r.since_new_work_moved_right_way += 1;
            }
            if q1 > q0 {
                ups.push(100.0 * (a - b))
            } else {
                downs.push(100.0 * (a - b))
            }
        }
    }
    r.uplift_when_work_was_good = avg(&ups);
    r.change_when_work_was_generated = avg(&downs);

    // Conversations: rebuild each side's copy and check it against the fingerprints.
    r.messages = sqlx::query_scalar("select count(*) from job_messages")
        .fetch_one(db)
        .await?;
    r.messages_with_text =
        sqlx::query_scalar("select count(*) from job_messages where body is not null")
            .fetch_one(db)
            .await?;
    r.inquiries = sqlx::query_scalar("select count(*) from jobs where inquiry")
        .fetch_one(db)
        .await?;
    let talked: Vec<Uuid> = sqlx::query_scalar(
        "select distinct job_id from job_messages m where not exists (select 1 from job_messages x where x.job_id = m.job_id and x.body is null)",
    )
    .fetch_all(db)
    .await?;
    for job in talked {
        let rows: Vec<(Uuid, Uuid, String, String)> = sqlx::query_as(
            "select id, author_id, body, sha256 from job_messages where job_id = $1 order by created_at, id",
        )
        .bind(job)
        .fetch_all(db)
        .await?;
        let copy = jobs::ConversationCopy {
            job_id: job,
            messages: rows
                .into_iter()
                .map(|(id, author_id, body, sha256)| jobs::CopiedMessage {
                    id,
                    author_id,
                    author: String::new(),
                    created_at: OffsetDateTime::UNIX_EPOCH,
                    body,
                    sha256,
                })
                .collect(),
        };
        let (_, bad) = jobs::verify_copy(db, job, &copy).await?;
        if bad == 0 {
            r.conversations_verified += 1
        } else {
            r.conversations_failing_verification += 1
        }
    }
    let rows: Vec<(String, i64)> =
        sqlx::query_as("select status, count(*) from job_changes group by status")
            .fetch_all(db)
            .await?;
    r.changes_by_status = rows.into_iter().collect();
    let extra: i64 = sqlx::query_scalar(
        "select coalesce(sum(amount_minor), 0)::int8 from job_payments where part = 'change' and status = 'paid'",
    )
    .fetch_one(db)
    .await?;
    r.changes_extra_paid_usd = extra as f64 / 100.0;
    r.signed_off = sqlx::query_scalar("select count(*) from jobs where signed_off_at is not null")
        .fetch_one(db)
        .await?;
    r.final_files_before_payment = sqlx::query_scalar(
        "select count(*) from jobs j where final_files_at is not null
           and exists (select 1 from job_payments p where p.job_id = j.id and p.due_on = 'delivery' and (p.paid_at is null or p.paid_at > j.final_files_at))",
    )
    .fetch_one(db)
    .await?;
    r.waiting_for_final_files =
        sqlx::query_scalar("select count(*) from jobs where status = 'final_files'")
            .fetch_one(db)
            .await?;
    let rows: Vec<(String, i64)> =
        sqlx::query_as("select status, count(*) from job_problems group by status")
            .fetch_all(db)
            .await?;
    r.problems_by_status = rows.into_iter().collect();
    let refunds: i64 = sqlx::query_scalar(
        "select coalesce(-sum(amount_minor), 0)::int8 from payments where kind = 'refund'",
    )
    .fetch_one(db)
    .await?;
    r.agreed_refunds_usd = refunds as f64 / 100.0;
    let rows: Vec<(String, i64)> =
        sqlx::query_as("select status, count(*) from review_disputes group by status")
            .fetch_all(db)
            .await?;
    r.review_disputes_by_status = rows.into_iter().collect();
    // A hidden review must not count: compare each creative's real rating with a recount of
    // the visible reviews only.
    let creatives: Vec<Uuid> = sqlx::query_scalar("select user_id from creatives")
        .fetch_all(db)
        .await?;
    for c in creatives {
        let rating = people::rating_of(db, c).await?;
        let (up, down): (i64, i64) = sqlx::query_as(
            "select count(*) filter (where vote = 1), count(*) filter (where vote = -1) from (
                 select distinct on (client_id) vote from creative_reviews where creative_id = $1 and hidden_at is null
                 order by client_id, created_at desc) x",
        )
        .bind(c)
        .fetch_one(db)
        .await?;
        if rating.not_slop != up || rating.slop != down {
            r.hidden_reviews_counted += 1;
        }
    }

    // HUM-13: the overall tally is every current vote, nothing discounted.
    for app in &credited_apps {
        let t = crate::votes::tally(db, *app).await?;
        let (up, down): (i64, i64) = sqlx::query_as(
            "select count(*) filter (where vote = 1), count(*) filter (where vote = -1) from votes where app_id = $1",
        )
        .bind(app)
        .fetch_one(db)
        .await?;
        if t.not_slop == up && t.slop == down {
            r.overall_rating_untouched += 1;
        }
    }
    Ok(r)
}
