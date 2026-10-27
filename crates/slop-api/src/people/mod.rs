// SPDX-License-Identifier: AGPL-3.0-or-later
//! People: creatives offering services to makers (DESIGN §15, PAYMENTS §7, LEGAL §4).
//!
//! - `mod.rs`: profiles (customisable pages, unlike app pages), the directory, verified reviews.
//! - `portfolio.rs`: portfolios from a GitHub repository, reviewed like store pictures.
//! - `design.rs`: the creative's own page design from the same repository, reviewed with it.
//! - `jobs.rs`: request → offer → accept & pay → deliver → complete, with the Service Agreement.
//! - `stripe.rs`: Stripe Connect. Every payment is a direct charge on the creative's own account;
//!   SLAPorSLOP only takes its 5% fee and never holds the money (LEGAL-GUIDANCE §1 rule 1).
//!
//! SLAPorSLOP never sets prices, assigns work, or ranks creatives by default on performance (HUM-9,
//! HUM-10): the directory's default order is a daily shuffle.

pub mod design;
mod discover;
pub mod jobs;
pub mod mediation;
pub mod portfolio;
pub mod stripe;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, QueryBuilder};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::{CurrentUser, MaybeUser},
    catalog::{self, AppSummary},
    error::{ApiError, ApiResult, is_unique_violation},
    media,
    policy::ViewerPolicy,
    votes,
};
use portfolio::Work;

/// Skills, as `(slug, name)`. The order is the directory's.
pub const SKILLS: &[(&str, &str)] = &[
    ("2d-art", "2D Art"),
    ("pixel-art", "Pixel Art"),
    ("3d", "3D Modelling"),
    ("animation", "Animation"),
    ("ui-ux", "UI / UX"),
    ("music", "Music"),
    ("sound", "Sound Design"),
    ("writing", "Writing"),
    ("voice", "Voice Acting"),
    ("localization", "Localization"),
    ("playtesting", "Playtesting"),
];

/// The word an app page credits a skill with: "Art by …", "Music by …".
pub fn credit_role(skill: &str) -> &'static str {
    match skill {
        "music" => "Music",
        "sound" => "Sound",
        "writing" => "Writing",
        "voice" => "Voice",
        "localization" => "Localization",
        "playtesting" => "Playtesting",
        _ => "Art",
    }
}

/// Accent colours a profile can pick; the UI maps each to colours that keep text readable.
pub const ACCENTS: &[&str] = &[
    "slime", "ocean", "grape", "rose", "ember", "sun", "forest", "slate",
];
/// Profile sections, which the creative can put in any order.
pub const SECTIONS: &[&str] = &["portfolio", "services", "reviews", "worked_on", "about"];
const MAX_SERVICES: usize = 8;
const MAX_LINKS: usize = 5;
/// A creative's rating shows from this many reviews.
pub const MIN_REVIEWS: i64 = 3;
/// Job prices, in minor units of any of the currencies (PAYMENTS §7).
pub const MIN_PRICE_MINOR: i64 = 1_000;
pub const MAX_PRICE_MINOR: i64 = 1_000_000;
pub const CURRENCIES: &[&str] = &["usd", "eur", "gbp"];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/people", get(directory))
        .route("/people/skills", get(skills))
        .route("/people/discover", get(discover::discover))
        .route("/people/{handle}", get(page))
        .route("/me/creative", get(my_profile).put(save_profile))
        .route("/me/creative/portfolio", get(portfolio::overview))
        .route("/me/creative/portfolio/check", post(portfolio::check))
        .route("/me/creative/portfolio/order", put(portfolio::set_order))
        .route("/me/creative/stripe", post(stripe::onboard))
        .route("/me/creative/stripe/refresh", post(stripe::refresh_account))
        .route("/people/reviews/{job_id}/reply", put(reply))
        .merge(jobs::routes())
        .merge(mediation::routes())
        .merge(stripe::webhook_routes())
}

/// The caller's creative profile, or 404.
pub async fn my_creative(state: &AppState, user: &CurrentUser) -> ApiResult<Uuid> {
    let exists: bool =
        sqlx::query_scalar("select exists (select 1 from creatives where user_id = $1)")
            .bind(user.id)
            .fetch_one(&state.db)
            .await?;
    if exists {
        Ok(user.id)
    } else {
        Err(ApiError::NotFound)
    }
}

/// Confirms the caller is 18+ (LEGAL §6), recording it the first time. People needs it to sell
/// or hire.
pub async fn require_adult(state: &AppState, user: &CurrentUser, confirm: bool) -> ApiResult<()> {
    let confirmed: Option<OffsetDateTime> =
        sqlx::query_scalar("select adult_confirmed_at from users where id = $1")
            .bind(user.id)
            .fetch_one(&state.db)
            .await?;
    if confirmed.is_some() {
        return Ok(());
    }
    if !confirm {
        return Err(ApiError::invalid(
            "confirm_adult",
            "People is for adults: confirm you're 18 or older",
        ));
    }
    sqlx::query("update users set adult_confirmed_at = now() where id = $1")
        .bind(user.id)
        .execute(&state.db)
        .await?;
    Ok(())
}

/// SQL for a creative `c` whose page others can see: its first portfolio approved, not hidden,
/// and the account not suspended. A closed account's page stays visible, frozen (account.rs).
pub(crate) const VISIBLE_SQL: &str = "c.portfolio_set_id is not null and c.hidden_reason is null
     and not exists (select 1 from users su where su.id = c.user_id and su.suspended_at is not null)";

/// SQL for a creative `c` in the directory and open to hiring: visible, and the account open.
pub(crate) const OPEN_SQL: &str = "c.portfolio_set_id is not null and c.hidden_reason is null
     and not exists (select 1 from users su where su.id = c.user_id
                     and (su.suspended_at is not null or su.closed_at is not null))";

async fn skills() -> Json<Vec<Skill>> {
    Json(SKILLS.iter().map(|(s, _)| skill(s)).collect())
}

#[derive(Clone, Serialize)]
pub struct Skill {
    pub slug: String,
    pub name: &'static str,
}

pub fn skill(slug: &str) -> Skill {
    Skill {
        slug: slug.to_string(),
        name: SKILLS
            .iter()
            .find(|(s, _)| *s == slug)
            .map_or("Other", |(_, n)| n),
    }
}

// ---------------------------------------------------------------------------------------------
// Services (packages) and links

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Service {
    /// Assigned by SLAPorSLOP when empty.
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// "From" price, in minor units.
    pub price_minor: i64,
    pub currency: String,
    /// Typical days to deliver.
    pub days: i32,
    pub skill: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub label: String,
    pub url: String,
}

fn one_line(field: &'static str, value: &str, max: usize) -> ApiResult<String> {
    let v = value.trim();
    if v.chars().count() > max || v.chars().any(char::is_control) {
        return Err(ApiError::invalid(
            field,
            format!("one line of up to {max} characters"),
        ));
    }
    Ok(v.to_string())
}

pub fn check_price(field: &'static str, minor: i64, currency: &str) -> ApiResult<()> {
    if !CURRENCIES.contains(&currency) {
        return Err(ApiError::invalid(field, "use usd, eur or gbp"));
    }
    if !(MIN_PRICE_MINOR..=MAX_PRICE_MINOR).contains(&minor) {
        return Err(ApiError::invalid(
            field,
            "prices are from 10 to 10,000 (PAYMENTS §7)",
        ));
    }
    Ok(())
}

fn clean_services(mut services: Vec<Service>, skills: &[String]) -> ApiResult<Vec<Service>> {
    if services.len() > MAX_SERVICES {
        return Err(ApiError::invalid(
            "services",
            format!("up to {MAX_SERVICES} services"),
        ));
    }
    for s in &mut services {
        s.name = one_line("services", &s.name, 60)?;
        if s.name.is_empty() {
            return Err(ApiError::invalid("services", "every service needs a name"));
        }
        s.description = s.description.trim().to_string();
        if s.description.chars().count() > 600 {
            return Err(ApiError::invalid(
                "services",
                "descriptions are up to 600 characters",
            ));
        }
        check_price("services", s.price_minor, &s.currency)?;
        if !(1..=365).contains(&s.days) {
            return Err(ApiError::invalid("services", "days are from 1 to 365"));
        }
        if !skills.contains(&s.skill) {
            return Err(ApiError::invalid(
                "services",
                format!("\"{}\" isn't one of your skills", s.skill),
            ));
        }
        let id_ok = s.id.len() == 8 && s.id.bytes().all(|b| b.is_ascii_hexdigit());
        if !id_ok {
            s.id = Uuid::new_v4().simple().to_string()[..8].to_string();
        }
    }
    Ok(services)
}

fn clean_links(links: Vec<Link>) -> ApiResult<Vec<Link>> {
    if links.len() > MAX_LINKS {
        return Err(ApiError::invalid(
            "links",
            format!("up to {MAX_LINKS} links"),
        ));
    }
    links
        .into_iter()
        .map(|l| {
            let label = one_line("links", &l.label, 30)?;
            let url: Url =
                l.url.trim().parse().map_err(|_| {
                    ApiError::invalid("links", "links must be full https:// addresses")
                })?;
            if url.scheme() != "https" || url.host_str().is_none() || url.as_str().len() > 300 {
                return Err(ApiError::invalid(
                    "links",
                    "links must be https:// addresses of up to 300 characters",
                ));
            }
            Ok(Link {
                label: if label.is_empty() {
                    url.host_str().unwrap_or_default().to_string()
                } else {
                    label
                },
                url: url.to_string(),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The creative's own profile

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileInput {
    handle: String,
    name: String,
    kind: String,
    team_size: Option<i16>,
    #[serde(default)]
    headline: String,
    #[serde(default)]
    about_md: String,
    #[serde(default)]
    location: String,
    skills: Vec<String>,
    ai_use: String,
    #[serde(default)]
    ai_note: String,
    trader: bool,
    taking_work: bool,
    #[serde(default)]
    turnaround: String,
    #[serde(default)]
    services: Vec<Service>,
    #[serde(default = "default_accent")]
    accent: String,
    #[serde(default)]
    sections: Vec<String>,
    #[serde(default)]
    links: Vec<Link>,
    #[serde(default)]
    confirm_adult: bool,
}

fn default_accent() -> String {
    "slime".into()
}

#[derive(Serialize, sqlx::FromRow)]
pub struct MyProfile {
    handle: String,
    name: String,
    kind: String,
    team_size: Option<i16>,
    headline: String,
    about_md: String,
    location: String,
    skills: Vec<String>,
    ai_use: String,
    ai_note: String,
    trader: bool,
    taking_work: bool,
    turnaround: String,
    #[sqlx(json)]
    services: Vec<Service>,
    accent: String,
    sections: Vec<String>,
    #[sqlx(json)]
    links: Vec<Link>,
    portfolio_repo: Option<String>,
    stripe_status: String,
    /// Live once a portfolio is approved, unless hidden for breaking the rules.
    live: bool,
    hidden_reason: Option<String>,
    /// Whether Stripe payments are switched on for this server (or simulated).
    #[sqlx(skip)]
    payments: &'static str,
    /// Where the page is: draft, waiting for review, turned down, live or hidden.
    #[sqlx(skip)]
    status: PageStatus,
}

async fn load_my_profile(state: &AppState, user_id: Uuid) -> ApiResult<MyProfile> {
    let mut p: MyProfile = sqlx::query_as(
        "select handle, name, kind, team_size, headline, about_md, location, skills, ai_use, ai_note,
                trader, taking_work, turnaround, services, accent, sections, links, portfolio_repo,
                stripe_status, (portfolio_set_id is not null and hidden_reason is null) as live,
                hidden_reason
         from creatives where user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    p.payments = stripe::mode(state).name();
    p.status = page_status(&state.db, user_id).await?;
    Ok(p)
}

/// Where a creative's page is, for the creative themselves (their editor, their page, /people,
/// their account). Only portfolios are reviewed (portfolio.rs): the profile's own text is saved
/// as it is, and shows once the page is live.
#[derive(Serialize, Default, Debug, PartialEq)]
pub struct PageStatus {
    /// - `draft`: no portfolio sent yet (or none could be read); only the creative sees the page.
    /// - `waiting`: the first portfolio waits for a reviewer; only the creative sees the page.
    /// - `turned_down`: the newest portfolio was turned down (see `reason`) and none is approved
    ///   yet; the creative fixes it and checks again.
    /// - `live`: an approved portfolio; everyone sees the page.
    /// - `hidden`: hidden by a moderation decision (`reason`), or the account is suspended.
    pub state: &'static str,
    /// turned_down: the reviewer's reason. live: why a newer portfolio was turned down (the
    /// approved one still shows). hidden: the decision's reason (none when suspended).
    pub reason: Option<String>,
    /// live: a newer portfolio waits for review; the approved one shows until it's decided.
    pub update_waiting: bool,
    /// When the portfolio waiting for review was read.
    #[serde(with = "time::serde::rfc3339::option")]
    pub waiting_since: Option<OffsetDateTime>,
    /// What went wrong the last time SLAPorSLOP read the portfolio, if anything.
    pub problem: Option<String>,
    /// hidden: because the account is suspended.
    pub suspended: bool,
}

pub async fn page_status(db: &sqlx::PgPool, creative_id: Uuid) -> sqlx::Result<PageStatus> {
    #[derive(sqlx::FromRow)]
    struct Row {
        has_live: bool,
        hidden_reason: Option<String>,
        problem: Option<String>,
        suspended: bool,
        pending_at: Option<OffsetDateTime>,
        rejected_reason: Option<String>,
    }
    // The newest turned-down portfolio counts only when it's newer than the approved one.
    let r: Row = sqlx::query_as(
        "select c.portfolio_set_id is not null as has_live, c.hidden_reason,
                c.portfolio_problem as problem, u.suspended_at is not null as suspended,
                (select max(ps.created_at) from portfolio_sets ps
                  where ps.creative_id = c.user_id and ps.status = 'pending') as pending_at,
                (select ps.status_reason from portfolio_sets ps
                  where ps.creative_id = c.user_id and ps.status = 'rejected'
                    and ps.created_at > coalesce((select l.created_at from portfolio_sets l
                                                  where l.id = c.portfolio_set_id), '-infinity')
                  order by ps.created_at desc limit 1) as rejected_reason
         from creatives c join users u on u.id = c.user_id where c.user_id = $1",
    )
    .bind(creative_id)
    .fetch_one(db)
    .await?;
    let mut s = PageStatus {
        problem: r.problem,
        waiting_since: r.pending_at,
        ..PageStatus::default()
    };
    if r.hidden_reason.is_some() || r.suspended {
        s.state = "hidden";
        s.reason = r.hidden_reason;
        s.suspended = r.suspended;
    } else if r.has_live {
        s.state = "live";
        s.update_waiting = r.pending_at.is_some();
        if r.pending_at.is_none() {
            s.reason = r.rejected_reason;
        }
    } else if r.pending_at.is_some() {
        s.state = "waiting";
    } else if r.rejected_reason.is_some() {
        s.state = "turned_down";
        s.reason = r.rejected_reason;
    } else {
        s.state = "draft";
    }
    Ok(s)
}

async fn my_profile(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<MyProfile>> {
    Ok(Json(load_my_profile(&state, user.id).await?))
}

/// Creates or updates the caller's profile.
async fn save_profile(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(p): Json<ProfileInput>,
) -> ApiResult<Json<MyProfile>> {
    require_adult(&state, &user, p.confirm_adult).await?;
    crate::legal::require(&state.db, user.id, "people").await?;
    let handle = p.handle.trim().to_ascii_lowercase();
    let handle_ok = (3..=40).contains(&handle.len())
        && handle.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        });
    if !handle_ok {
        return Err(ApiError::invalid(
            "handle",
            "3 to 40 lowercase letters, numbers and single dashes",
        ));
    }
    let name = one_line("name", &p.name, 60)?;
    if name.is_empty() {
        return Err(ApiError::invalid("name", "add your name or your studio's"));
    }
    let team_size = match p.kind.as_str() {
        "individual" => None,
        "studio" => match p.team_size {
            Some(n) if (2..=500).contains(&n) => Some(n),
            _ => return Err(ApiError::invalid("team_size", "how many people, 2 to 500")),
        },
        _ => return Err(ApiError::invalid("kind", "individual or studio")),
    };
    let mut skills: Vec<String> = Vec::new();
    for s in p.skills {
        if !SKILLS.iter().any(|(slug, _)| *slug == s) {
            return Err(ApiError::invalid(
                "skills",
                format!("\"{s}\" isn't a skill"),
            ));
        }
        if !skills.contains(&s) {
            skills.push(s);
        }
    }
    if skills.is_empty() {
        return Err(ApiError::invalid("skills", "pick at least one skill"));
    }
    if !["none", "assisted", "heavy"].contains(&p.ai_use.as_str()) {
        return Err(ApiError::invalid(
            "ai_use",
            "say how you use AI: none, assisted or heavy",
        ));
    }
    if !ACCENTS.contains(&p.accent.as_str()) {
        return Err(ApiError::invalid("accent", "pick one of the colours"));
    }
    // Every section once, in the creative's order; any left out go at the end.
    let mut sections: Vec<String> = Vec::new();
    for s in p.sections {
        if !SECTIONS.contains(&s.as_str()) {
            return Err(ApiError::invalid(
                "sections",
                format!("\"{s}\" isn't a section"),
            ));
        }
        if !sections.contains(&s) {
            sections.push(s);
        }
    }
    for s in SECTIONS {
        if !sections.iter().any(|x| x == s) {
            sections.push(s.to_string());
        }
    }
    let about = p.about_md.trim();
    if about.chars().count() > 6000 {
        return Err(ApiError::invalid("about_md", "up to 6,000 characters"));
    }
    let services = clean_services(p.services, &skills)?;
    let links = clean_links(p.links)?;
    let result = sqlx::query(
        "insert into creatives (user_id, handle, name, kind, team_size, headline, about_md, location, skills,
                                ai_use, ai_note, trader, taking_work, turnaround, services, accent, sections, links)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
         on conflict (user_id) do update set
             handle = $2, name = $3, kind = $4, team_size = $5, headline = $6, about_md = $7,
             location = $8, skills = $9, ai_use = $10, ai_note = $11, trader = $12,
             taking_work = $13, turnaround = $14, services = $15, accent = $16, sections = $17,
             links = $18, updated_at = now()",
    )
    .bind(user.id)
    .bind(&handle)
    .bind(&name)
    .bind(&p.kind)
    .bind(team_size)
    .bind(one_line("headline", &p.headline, 140)?)
    .bind(about)
    .bind(one_line("location", &p.location, 60)?)
    .bind(&skills)
    .bind(&p.ai_use)
    .bind(one_line("ai_note", &p.ai_note, 300)?)
    .bind(p.trader)
    .bind(p.taking_work)
    .bind(one_line("turnaround", &p.turnaround, 40)?)
    .bind(sqlx::types::Json(&services))
    .bind(&p.accent)
    .bind(&sections)
    .bind(sqlx::types::Json(&links))
    .execute(&state.db)
    .await;
    match result {
        Err(e) if is_unique_violation(&e) => {
            return Err(ApiError::Conflict(format!(
                "the handle \"{handle}\" is taken"
            )));
        }
        r => r?,
    };
    Ok(Json(load_my_profile(&state, user.id).await?))
}

// ---------------------------------------------------------------------------------------------
// Public pages

#[derive(Clone, Serialize)]
pub struct WorkView {
    pub key: String,
    /// image, video or audio.
    pub kind: &'static str,
    /// The picture, or the video's or track's poster.
    pub url: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub youtube_id: Option<String>,
    pub audio_url: Option<String>,
    pub audio_format: Option<String>,
    pub title: Option<String>,
    pub caption: Option<String>,
    pub skill: Option<Skill>,
}

pub fn work_view(w: Work) -> WorkView {
    let key = w.key();
    match w {
        Work::Image {
            picture,
            title,
            caption,
            skill: s,
        } => WorkView {
            key,
            kind: "image",
            url: Some(picture.url),
            width: Some(picture.width),
            height: Some(picture.height),
            youtube_id: None,
            audio_url: None,
            audio_format: None,
            title,
            caption,
            skill: s.as_deref().map(skill),
        },
        Work::Video {
            youtube_id,
            poster,
            title,
            caption,
            skill: s,
        } => WorkView {
            key,
            kind: "video",
            width: poster.as_ref().map(|p| p.width),
            height: poster.as_ref().map(|p| p.height),
            url: poster.map(|p| p.url),
            youtube_id: Some(youtube_id),
            audio_url: None,
            audio_format: None,
            title,
            caption,
            skill: s.as_deref().map(skill),
        },
        Work::Audio {
            audio,
            poster,
            title,
            caption,
            skill: s,
        } => WorkView {
            key,
            kind: "audio",
            width: poster.as_ref().map(|p| p.width),
            height: poster.as_ref().map(|p| p.height),
            url: poster.map(|p| p.url),
            youtube_id: None,
            audio_url: Some(audio.url),
            audio_format: Some(audio.format),
            title,
            caption,
            skill: s.as_deref().map(skill),
        },
    }
}

/// A creative's rating from verified clients: each client's newest review counts once.
#[derive(Clone, Serialize, Default)]
pub struct CreativeRating {
    pub not_slop: i64,
    pub slop: i64,
    /// Null below MIN_REVIEWS.
    pub percent: Option<i32>,
    pub label: Option<&'static str>,
    pub min_reviews: i64,
}

impl CreativeRating {
    pub fn new(not_slop: i64, slop: i64) -> Self {
        let total = not_slop + slop;
        let percent =
            (total >= MIN_REVIEWS).then(|| (100.0 * not_slop as f64 / total as f64).round() as i32);
        Self {
            not_slop,
            slop,
            percent,
            label: percent.map(votes::label),
            min_reviews: MIN_REVIEWS,
        }
    }
}

/// A creative's rating, as their page shows it.
pub async fn rating_of(db: &sqlx::PgPool, creative_id: Uuid) -> sqlx::Result<CreativeRating> {
    let (up, down): (i64, i64) = sqlx::query_as(
        "select count(*) filter (where vote = 1), count(*) filter (where vote = -1)
         from (select distinct on (client_id) vote from creative_reviews
               where creative_id = $1 and hidden_at is null order by client_id, created_at desc) r",
    )
    .bind(creative_id)
    .fetch_one(db)
    .await?;
    Ok(CreativeRating::new(up, down))
}

const RATING_SQL: &str = "(select coalesce(count(*) filter (where r.vote = 1), 0)
        from (select distinct on (client_id) vote from creative_reviews
              where creative_id = c.user_id and hidden_at is null order by client_id, created_at desc) r) as not_slop,
     (select coalesce(count(*) filter (where r.vote = -1), 0)
        from (select distinct on (client_id) vote from creative_reviews
              where creative_id = c.user_id and hidden_at is null order by client_id, created_at desc) r) as slop,
     (select count(distinct client_id) from jobs j
        where j.creative_id = c.user_id and j.status = 'completed') as clients";

#[derive(Serialize)]
pub struct Card {
    pub handle: String,
    pub name: String,
    pub kind: String,
    pub team_size: Option<i16>,
    pub headline: String,
    pub location: String,
    pub skills: Vec<Skill>,
    pub ai_use: String,
    pub taking_work: bool,
    pub can_hire: bool,
    pub accent: String,
    pub avatar_url: Option<String>,
    /// Up to 4 pictures from the portfolio.
    pub preview: Vec<String>,
    /// The lowest service price, for "from $X".
    pub from_price: Option<Price>,
    pub rating: CreativeRating,
    pub clients: i64,
}

#[derive(Clone, Serialize)]
pub struct Price {
    pub minor: i64,
    pub currency: String,
}

#[derive(sqlx::FromRow)]
struct CardRow {
    handle: String,
    name: String,
    kind: String,
    team_size: Option<i16>,
    headline: String,
    location: String,
    skills: Vec<String>,
    ai_use: String,
    taking_work: bool,
    can_hire: bool,
    accent: String,
    #[sqlx(json)]
    services: Vec<Service>,
    #[sqlx(json(nullable))]
    avatar: Option<media::Picture>,
    /// An individual's profile picture from their account (avatars.rs), when the portfolio has
    /// no avatar.
    account_avatar: Option<String>,
    #[sqlx(json(nullable))]
    works: Option<Vec<Work>>,
    portfolio_order: Vec<String>,
    not_slop: i64,
    slop: i64,
    clients: i64,
}

impl CardRow {
    fn into_card(self, config: &crate::config::Config) -> Card {
        let mut works = self.works.unwrap_or_default();
        portfolio::arrange(&mut works, &self.portfolio_order);
        let preview = works
            .into_iter()
            .filter_map(|w| work_view(w).url)
            .take(4)
            .collect();
        let from_price = self
            .services
            .iter()
            .min_by_key(|s| s.price_minor)
            .map(|s| Price {
                minor: s.price_minor,
                currency: s.currency.clone(),
            });
        Card {
            handle: self.handle,
            name: self.name,
            kind: self.kind,
            team_size: self.team_size,
            headline: self.headline,
            location: self.location,
            skills: self.skills.iter().map(|s| skill(s)).collect(),
            ai_use: self.ai_use,
            taking_work: self.taking_work,
            can_hire: self.can_hire,
            accent: self.accent,
            avatar_url: self
                .avatar
                .map(|a| a.url)
                .or_else(|| crate::avatars::absolute(config, self.account_avatar)),
            preview,
            from_price,
            rating: CreativeRating::new(self.not_slop, self.slop),
            clients: self.clients,
        }
    }
}

/// `select <card columns><extra> from creatives c left join portfolio_sets ps …`.
fn push_card_select(qb: &mut QueryBuilder<Postgres>, extra: &'static str) {
    qb.push(
        "select c.handle, c.name, c.kind, c.team_size, c.headline, c.location, c.skills, c.ai_use,
                c.taking_work, (c.taking_work and c.stripe_status = 'active') as can_hire, c.accent,
                c.services, ps.avatar, ps.works, c.portfolio_order, ",
    )
    .push(format!(
        "case when c.kind <> 'studio' then {} end as account_avatar, ",
        crate::avatars::path_sql("c.user_id")
    ))
    .push(RATING_SQL)
    .push(extra)
    .push(" from creatives c left join portfolio_sets ps on ps.id = c.portfolio_set_id");
}

#[derive(Deserialize)]
struct DirectoryQuery {
    skill: Option<String>,
    #[serde(default)]
    taking_work: bool,
    /// none, assisted or heavy: creatives who use AI exactly that much.
    ai: Option<String>,
    #[serde(default)]
    studios: bool,
    q: Option<String>,
    /// shuffle (default), rating, clients or new.
    sort: Option<String>,
}

#[derive(Serialize)]
struct Directory {
    people: Vec<Card>,
    skills: Vec<SkillCount>,
}

#[derive(Serialize)]
struct SkillCount {
    #[serde(flatten)]
    skill: Skill,
    count: i64,
}

async fn directory(
    State(state): State<AppState>,
    Query(q): Query<DirectoryQuery>,
) -> ApiResult<Json<Directory>> {
    let mut qb = QueryBuilder::new("select * from (");
    push_card_select(&mut qb, "");
    qb.push(" where ").push(OPEN_SQL);
    if let Some(s) = q.skill.as_deref().filter(|s| !s.is_empty()) {
        qb.push(" and ")
            .push_bind(s.to_string())
            .push(" = any(c.skills)");
    }
    if q.taking_work {
        qb.push(" and c.taking_work");
    }
    // Each level on its own: the filter finds a way of working, it doesn't rank them.
    if let Some(ai) =
        q.ai.as_deref()
            .filter(|a| ["none", "assisted", "heavy"].contains(a))
    {
        qb.push(" and c.ai_use = ").push_bind(ai.to_string());
    }
    if q.studios {
        qb.push(" and c.kind = 'studio'");
    }
    if let Some(text) = q.q.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        let like = format!("%{}%", text.replace(['%', '_', '\\'], ""));
        qb.push(" and (c.name ilike ")
            .push_bind(like.clone())
            .push(" or c.headline ilike ")
            .push_bind(like.clone())
            .push(" or c.handle ilike ")
            .push_bind(like)
            .push(")");
    }
    qb.push(") x order by ");
    match q.sort.as_deref() {
        Some("rating") => qb.push(
            "case when not_slop + slop >= 3 then not_slop::float8 / (not_slop + slop) end desc nulls last, clients desc, handle",
        ),
        Some("clients") => qb.push("clients desc, handle"),
        // A different order every day, the same all day (HUM-10: no default ranking on performance).
        _ => qb
            .push("md5(handle || ")
            .push_bind(OffsetDateTime::now_utc().date().to_string())
            .push(")"),
    };
    qb.push(" limit 200");
    let rows: Vec<CardRow> = qb.build_query_as().fetch_all(&state.db).await?;
    let counts: Vec<(String, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select s, count(*) from creatives c, unnest(c.skills) s where {OPEN_SQL} group by s"
    )))
    .fetch_all(&state.db)
    .await?;
    Ok(Json(Directory {
        people: rows
            .into_iter()
            .map(|r| r.into_card(&state.config))
            .collect(),
        skills: SKILLS
            .iter()
            .map(|(s, _)| SkillCount {
                skill: skill(s),
                count: counts.iter().find(|(c, _)| c == s).map_or(0, |(_, n)| *n),
            })
            .collect(),
    }))
}

#[derive(Serialize)]
pub struct ReviewView {
    pub job_id: Uuid,
    pub client_name: String,
    /// The project the work was for, when it's still in the store.
    pub project: Option<String>,
    pub project_slug: Option<String>,
    /// What was made (the offer's title).
    pub work: String,
    pub skill: Skill,
    /// not_slop or slop.
    pub vote: &'static str,
    pub body: String,
    pub reply: Option<String>,
    /// A mediator's factual note, after a review dispute (mediation.rs).
    pub mediator_note: Option<String>,
    /// The creative's dispute about it: pending, removed, noted or kept.
    pub dispute: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub replied_at: Option<OffsetDateTime>,
}

#[derive(Serialize)]
pub struct WorkedOn {
    #[serde(flatten)]
    pub app: AppSummary,
    pub role: &'static str,
}

#[derive(Serialize)]
struct Page {
    #[serde(flatten)]
    card: Card,
    about_md: String,
    ai_note: String,
    trader: bool,
    turnaround: String,
    sections: Vec<String>,
    links: Vec<LinkView>,
    banner_url: Option<String>,
    /// The creative's own page design (design.rs), from the approved portfolio; none shows the
    /// standard layout.
    design: Option<design::PageDesign>,
    works: Vec<WorkView>,
    services: Vec<ServiceView>,
    reviews: Vec<ReviewView>,
    worked_on: Vec<WorkedOn>,
    is_me: bool,
    /// Always live for others; for the creative themselves, `mine.state`.
    status: &'static str,
    /// Only for the creative themselves: where their page is (`PageStatus`).
    mine: Option<PageStatus>,
    /// Only for the creative themselves: their portfolio that's newer than the approved one
    /// (waiting for review, or turned down), which their page previews. The other fields are the
    /// live page, as everyone else sees it.
    preview_set: Option<PreviewSet>,
    hidden_reason: Option<String>,
    /// The account was closed: the page is kept, frozen, and takes no new work.
    closed: bool,
}

/// What a portfolio that isn't approved yet changes on the page: the live page's fields, from
/// the same stored, sanitized set (portfolio.rs, design.rs).
#[derive(Serialize)]
struct PreviewSet {
    /// pending (waiting for review) or rejected (turned down: `status_reason`).
    status: String,
    status_reason: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    reviewed_at: Option<OffsetDateTime>,
    avatar_url: Option<String>,
    banner_url: Option<String>,
    design: Option<design::PageDesign>,
    works: Vec<WorkView>,
}

#[derive(Serialize)]
struct LinkView {
    label: String,
    url: String,
    domain: String,
}

#[derive(Serialize)]
pub struct ServiceView {
    #[serde(flatten)]
    pub service: Service,
    pub skill_name: &'static str,
}

async fn page(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Path(handle): Path<String>,
) -> ApiResult<Json<Page>> {
    #[derive(sqlx::FromRow)]
    struct Row {
        #[sqlx(flatten)]
        card: CardRow,
        user_id: Uuid,
        about_md: String,
        ai_note: String,
        trader: bool,
        turnaround: String,
        sections: Vec<String>,
        #[sqlx(json)]
        links: Vec<Link>,
        #[sqlx(json(nullable))]
        banner: Option<media::Picture>,
        #[sqlx(json(nullable))]
        design: Option<design::Design>,
        portfolio_set_id: Option<Uuid>,
        hidden_reason: Option<String>,
        suspended: bool,
        closed: bool,
    }
    let mut qb = QueryBuilder::new("");
    push_card_select(
        &mut qb,
        ", c.user_id, c.about_md, c.ai_note, c.trader, c.turnaround, c.sections, c.links,
           ps.banner, ps.design, c.portfolio_set_id, c.hidden_reason,
           (select su.suspended_at is not null from users su where su.id = c.user_id) as suspended,
           (select su.closed_at is not null from users su where su.id = c.user_id) as closed",
    );
    qb.push(" where c.handle = ")
        .push_bind(handle.to_ascii_lowercase());
    let row: Option<Row> = qb.build_query_as().fetch_optional(&state.db).await?;
    let row = row.ok_or(ApiError::NotFound)?;
    let is_me = user.as_ref().is_some_and(|u| u.id == row.user_id);
    let live = row.portfolio_set_id.is_some() && row.hidden_reason.is_none() && !row.suspended;
    if !live && !is_me {
        return Err(ApiError::NotFound);
    }
    let mine = if is_me {
        Some(page_status(&state.db, row.user_id).await?)
    } else {
        None
    };
    let status = mine.as_ref().map_or("live", |m| m.state);
    let order = row.card.portfolio_order.clone();
    let mut works: Vec<Work> = row.card.works.clone().unwrap_or_default();
    portfolio::arrange(&mut works, &order);
    let preview_set = if is_me {
        portfolio::newer_set(&state.db, row.user_id)
            .await?
            .map(|mut set| {
                portfolio::arrange(&mut set.works, &order);
                PreviewSet {
                    status: set.status,
                    status_reason: set.status_reason,
                    created_at: set.created_at,
                    reviewed_at: set.reviewed_at,
                    // Like the card: the account's picture when the portfolio has no avatar.
                    avatar_url: set.avatar.map(|a| a.url).or_else(|| {
                        crate::avatars::absolute(&state.config, row.card.account_avatar.clone())
                    }),
                    banner_url: set.banner.map(|b| b.url),
                    design: set.design.map(design::PageDesign::from),
                    works: set.works.into_iter().map(work_view).collect(),
                }
            })
    } else {
        None
    };
    let services = row
        .card
        .services
        .iter()
        .cloned()
        .map(|s| ServiceView {
            skill_name: skill(&s.skill).name,
            service: s,
        })
        .collect();
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let reviews = reviews_of(&state, row.user_id, policy).await?;
    let worked_on = worked_on(&state, row.user_id, policy).await?;
    Ok(Json(Page {
        about_md: row.about_md,
        ai_note: row.ai_note,
        trader: row.trader,
        turnaround: row.turnaround,
        sections: row.sections,
        links: row
            .links
            .into_iter()
            .map(|l| LinkView {
                domain: Url::parse(&l.url)
                    .ok()
                    .and_then(|u| {
                        u.host_str()
                            .map(|h| h.trim_start_matches("www.").to_string())
                    })
                    .unwrap_or_default(),
                label: l.label,
                url: l.url,
            })
            .collect(),
        banner_url: row.banner.map(|b| b.url),
        design: row.design.map(design::PageDesign::from),
        works: works.into_iter().map(work_view).collect(),
        services,
        reviews,
        worked_on,
        is_me,
        status,
        mine,
        preview_set,
        hidden_reason: if is_me { row.hidden_reason } else { None },
        closed: row.closed,
        card: row.card.into_card(&state.config),
    }))
}

async fn reviews_of(
    state: &AppState,
    creative_id: Uuid,
    policy: ViewerPolicy,
) -> ApiResult<Vec<ReviewView>> {
    let mut qb =
        QueryBuilder::new("select r.job_id, u.display_name as client_name, a.title, a.slug, (");
    catalog::push_store_visible(&mut qb, state.config.catalog, policy);
    qb.push(
        ") as visible, j.offer->>'title' as work, j.skill, r.vote, r.body, r.reply, r.mediator_note,
           d.status as dispute, r.created_at, r.replied_at
         from creative_reviews r join jobs j on j.id = r.job_id join users u on u.id = r.client_id
         join apps a on a.id = j.app_id left join review_disputes d on d.job_id = r.job_id
         where r.hidden_at is null and r.creative_id = ",
    )
    .push_bind(creative_id)
    .push(" order by r.created_at desc limit 100");
    #[derive(sqlx::FromRow)]
    struct Row {
        job_id: Uuid,
        client_name: String,
        title: String,
        slug: String,
        visible: bool,
        work: Option<String>,
        skill: String,
        vote: i16,
        body: String,
        reply: Option<String>,
        mediator_note: Option<String>,
        dispute: Option<String>,
        created_at: OffsetDateTime,
        replied_at: Option<OffsetDateTime>,
    }
    let rows: Vec<Row> = qb.build_query_as().fetch_all(&state.db).await?;
    Ok(rows
        .into_iter()
        .map(|r| ReviewView {
            job_id: r.job_id,
            client_name: r.client_name,
            project: r.visible.then_some(r.title),
            project_slug: r.visible.then_some(r.slug),
            work: r.work.unwrap_or_else(|| "Commissioned work".into()),
            skill: skill(&r.skill),
            vote: if r.vote > 0 { "not_slop" } else { "slop" },
            body: r.body,
            reply: r.reply,
            mediator_note: r.mediator_note,
            dispute: r.dispute,
            created_at: r.created_at,
            replied_at: r.replied_at,
        })
        .collect())
}

/// Apps in the store that credit the creative.
async fn worked_on(
    state: &AppState,
    creative_id: Uuid,
    policy: ViewerPolicy,
) -> ApiResult<Vec<WorkedOn>> {
    let mut qb = QueryBuilder::new("");
    catalog::push_summary_columns_with(&mut qb, &state.config, ", cr.skill as credit_skill");
    qb.push(
        " join (select distinct on (app_id) app_id, skill from jobs
                where creative_id = ",
    )
    .push_bind(creative_id)
    .push(
        " and credited_at is not null and coalesce((offer->>'credit')::boolean, true)
                order by app_id, credited_at desc) cr on cr.app_id = a.id where ",
    );
    catalog::push_store_visible(&mut qb, state.config.catalog, policy);
    qb.push(" order by a.updated_at desc limit 24");
    #[derive(sqlx::FromRow)]
    struct Row {
        #[sqlx(flatten)]
        app: AppSummary,
        credit_skill: String,
    }
    let rows: Vec<Row> = qb.build_query_as().fetch_all(&state.db).await?;
    Ok(rows
        .into_iter()
        .map(|r| WorkedOn {
            role: credit_role(&r.credit_skill),
            app: r.app,
        })
        .collect())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyBody {
    reply: String,
}

/// The creative's one public reply to a review (LEGAL §4.5). Saying it again replaces it.
async fn reply(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(job_id): Path<Uuid>,
    Json(body): Json<ReplyBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let text = body.reply.trim();
    if text.chars().count() > 1000 {
        return Err(ApiError::invalid("reply", "up to 1,000 characters"));
    }
    let done = sqlx::query(
        "update creative_reviews set reply = nullif($3, ''), replied_at = case when $3 = '' then null else now() end
         where job_id = $1 and creative_id = $2",
    )
    .bind(job_id)
    .bind(user.id)
    .bind(text)
    .execute(&state.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------------------------------------------------------------------------------------------
// What an app gets (DESIGN §15.4)

#[derive(Serialize)]
pub struct Credit {
    pub handle: String,
    pub name: String,
    /// Art, Music, Sound…
    pub role: &'static str,
    pub skill: Skill,
    /// What was made.
    pub work: String,
}

/// The creatives an app credits (completed, fully paid jobs with credit on). Hidden profiles
/// aren't linked.
pub async fn credits(db: &sqlx::PgPool, app_id: Uuid) -> sqlx::Result<Vec<Credit>> {
    let rows: Vec<(String, String, String, Option<String>)> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select c.handle, c.name, j.skill, j.offer->>'title'
         from jobs j join creatives c on c.user_id = j.creative_id
         where j.app_id = $1 and j.credited_at is not null
           and coalesce((j.offer->>'credit')::boolean, true)
           and {VISIBLE_SQL}
         order by j.credited_at"
        )))
        .bind(app_id)
        .fetch_all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(handle, name, s, work)| Credit {
            handle,
            name,
            role: credit_role(&s),
            skill: skill(&s),
            work: work.unwrap_or_else(|| "Commissioned work".into()),
        })
        .collect())
}

/// "Since the new work": votes cast or changed since the first release after the newest
/// credited job that has one (DESIGN §15.4). Hiring never changes the overall rating (HUM-13).
#[derive(Serialize)]
pub struct SinceNewWork {
    /// Null while the newest work hasn't shipped in a release yet.
    #[serde(with = "time::serde::rfc3339::option")]
    pub since: Option<OffsetDateTime>,
    pub version: Option<String>,
    pub credits: Vec<String>,
    #[serde(flatten)]
    pub tally: Option<votes::Tally>,
}

pub async fn since_new_work(db: &sqlx::PgPool, app_id: Uuid) -> sqlx::Result<Option<SinceNewWork>> {
    // Named only when they chose credit and their page is visible, like `credits` (security
    // review, 2026-10-22); the timing counts every job.
    let jobs: Vec<(Option<String>, Option<OffsetDateTime>, Option<String>)> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select case when coalesce((j.offer->>'credit')::boolean, true) and {VISIBLE_SQL} then c.name end,
                (select min(r.reviewed_at) from releases r
                  where r.app_id = j.app_id and r.status = 'approved' and r.reviewed_at >= j.credited_at),
                (select r.version from releases r
                  where r.app_id = j.app_id and r.status = 'approved' and r.reviewed_at >= j.credited_at
                  order by r.reviewed_at limit 1)
         from jobs j join creatives c on c.user_id = j.creative_id
         where j.app_id = $1 and j.credited_at is not null
         order by j.credited_at desc"
        )))
        .bind(app_id)
        .fetch_all(db)
        .await?;
    if jobs.is_empty() {
        return Ok(None);
    }
    let mut credits: Vec<String> = Vec::new();
    for (name, _, _) in &jobs {
        if let Some(name) = name
            && !credits.contains(name)
        {
            credits.push(name.clone());
        }
    }
    let shipped = jobs.iter().find(|(_, since, _)| since.is_some());
    let Some((_, Some(since), version)) = shipped else {
        return Ok(Some(SinceNewWork {
            since: None,
            version: None,
            credits,
            tally: None,
        }));
    };
    let (up, down, upw, downw): (i64, i64, f64, f64) = sqlx::query_as(
        "select count(*) filter (where vote = 1), count(*) filter (where vote = -1),
                coalesce(sum(weight) filter (where vote = 1), 0)::float8,
                coalesce(sum(weight) filter (where vote = -1), 0)::float8
         from votes where app_id = $1 and updated_at >= $2",
    )
    .bind(app_id)
    .bind(since)
    .fetch_one(db)
    .await?;
    Ok(Some(SinceNewWork {
        since: Some(*since),
        version: version.clone(),
        credits,
        tally: Some(votes::Tally::new(up, down, upw, downw)),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratings_need_three_reviews() {
        assert_eq!(CreativeRating::new(2, 0).percent, None);
        let r = CreativeRating::new(2, 1);
        assert_eq!(r.percent, Some(67));
        assert_eq!(r.label, Some("Mixed"));
    }

    #[test]
    fn credits_use_plain_words() {
        assert_eq!(credit_role("pixel-art"), "Art");
        assert_eq!(credit_role("music"), "Music");
        assert_eq!(credit_role("voice"), "Voice");
    }

    #[test]
    fn links_must_be_https() {
        assert!(
            clean_links(vec![Link {
                label: "x".into(),
                url: "javascript:alert(1)".into()
            }])
            .is_err()
        );
        assert!(
            clean_links(vec![Link {
                label: "x".into(),
                url: "http://a.example".into()
            }])
            .is_err()
        );
        let ok = clean_links(vec![Link {
            label: "".into(),
            url: "https://www.artstation.com/me".into(),
        }])
        .unwrap();
        assert_eq!(ok[0].label, "www.artstation.com");
    }
}
