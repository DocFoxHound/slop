// SPDX-License-Identifier: AGPL-3.0-or-later
//! Checks anyone can run before they have an account (decided 2026-10-27), so a person's AI
//! assistant can get a project ready first. Plain GETs any AI with a web-fetch tool can make,
//! answering JSON that starts with a one-sentence `summary`:
//!
//! - `GET /v1/check/store-address?slug=…`: is the address valid and free? If not, a free one.
//!   Only asks the database.
//! - `GET /v1/check/repository?url=…`: a readiness report for a public GitHub repository: its
//!   licence (GitHub's licence API, sharing `detect_license`'s day-long cache) and whether it's
//!   OSI-approved, a dry run of the pictures check (media.rs), and its builds: the Slop Build
//!   workflow, or else the newest GitHub release's files. Each finding says what's wrong and how
//!   to fix it, like fixes.rs.
//! - `GET /v1/check/portfolio?url=…`: a dry run of the People portfolio check (portfolio.rs,
//!   with the page design, design.rs), without a People page.
//!
//! Dry runs read and check, and nothing else: they never write to the database, never create a
//! set of pictures or a portfolio, and never queue anything for review.
//!
//! They make SLAPorSLOP fetch from GitHub for anyone, so (docs/SECURITY.md §8.1):
//! - files only come from the configured GitHub bases, for an `owner/name` that passed
//!   `github_repo_path`, at a commit GitHub named (40 hex digits), under media.rs's size limits;
//! - GitHub's API is asked as nobody, never with SLAPorSLOP's token or App, so a check can't
//!   reveal a private repository SLAPorSLOP can read, and doesn't use the allowance the release
//!   watcher needs (security review, 2026-10-22). Those requests are capped per hour for every
//!   address together (`check_github_per_hour`), and when GitHub says its allowance is used up,
//!   none are sent until it resets;
//! - each address gets `check_per_hour` repository and portfolio checks (ratelimit.rs);
//! - a report is kept for a few minutes per repository and commit, so asking again is free.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderValue, StatusCode, header::RETRY_AFTER},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::{
    AppState, catalog,
    fixes::Fix,
    media::{self, Problem},
    people::portfolio,
    source_scan,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/check/store-address", get(store_address))
        .route("/check/repository", get(repository))
        .route("/check/portfolio", get(portfolio_check))
}

/// How long a report is kept for its repository and commit.
const CACHE_FOR: Duration = Duration::from_secs(5 * 60);
/// Reports kept at once; past this the cache starts again.
const MAX_CACHED: usize = 2_000;
/// Where Slop Build's workflow lives (publishing.md, Option B), and the most of it that's read.
const WORKFLOW_PATH: &str = ".github/workflows/slop.yml";
const MAX_WORKFLOW_BYTES: u64 = 64 * 1024;
const DRY_RUN: &str = "This was a dry run: nothing was saved, and nothing was sent for review.";

/// What the checks share between requests (AppState::checks).
pub struct Shared {
    /// GitHub API requests left this hour, for every address together.
    github: Mutex<Bucket>,
    per_hour: u32,
    /// GitHub said its allowance for SLAPorSLOP's address is used up: none are sent until then.
    github_resets: Mutex<Option<Instant>>,
    /// Reports by kind, repository and commit.
    cache: Mutex<HashMap<String, (Instant, Value)>>,
}

struct Bucket {
    tokens: f64,
    updated: Instant,
}

impl Shared {
    pub fn new(github_per_hour: u32) -> Self {
        Self {
            github: Mutex::new(Bucket {
                tokens: f64::from(github_per_hour),
                updated: Instant::now(),
            }),
            per_hour: github_per_hour,
            github_resets: Mutex::new(None),
            cache: Mutex::default(),
        }
    }

    /// Takes one GitHub API request. `Err` is how many seconds until there's one.
    fn take_github(&self, now: Instant) -> Result<(), u64> {
        if self.per_hour == 0 {
            return Err(3600);
        }
        if let Some(reset) = *self.github_resets.lock().expect("checks lock")
            && reset > now
        {
            return Err(reset.duration_since(now).as_secs().max(1));
        }
        let capacity = f64::from(self.per_hour);
        let per_second = capacity / 3600.0;
        let mut b = self.github.lock().expect("checks lock");
        b.tokens = (b.tokens + now.saturating_duration_since(b.updated).as_secs_f64() * per_second)
            .min(capacity);
        b.updated = now;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            Ok(())
        } else {
            Err(((1.0 - b.tokens) / per_second).ceil().max(1.0) as u64)
        }
    }

    fn cached(&self, key: &str) -> Option<Value> {
        let cache = self.cache.lock().ok()?;
        let (at, report) = cache.get(key)?;
        (at.elapsed() < CACHE_FOR).then(|| report.clone())
    }

    fn keep(&self, key: String, report: &Value) {
        if let Ok(mut cache) = self.cache.lock() {
            if cache.len() >= MAX_CACHED {
                cache.retain(|_, (at, _)| at.elapsed() < CACHE_FOR);
                if cache.len() >= MAX_CACHED {
                    cache.clear();
                }
            }
            cache.insert(key, (Instant::now(), report.clone()));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Answers that aren't reports

/// One address asked for too many checks (ratelimit.rs): 429, saying when to come back.
pub(crate) fn too_many(retry_after: u64) -> Response {
    let minutes = retry_after.div_ceil(60).max(1);
    let message = format!(
        "Too many repository and portfolio checks from this address: try again in {minutes} minute{}. A report for the same commit is kept for {} minutes, so check again after pushing changes.",
        plural(minutes),
        CACHE_FOR.as_secs() / 60
    );
    refusal(
        StatusCode::TOO_MANY_REQUESTS,
        "rate_limited",
        message,
        retry_after,
    )
}

/// SLAPorSLOP can't ask GitHub right now (its hourly share is used, or GitHub said stop): 503.
fn github_busy(retry_after: u64, off: bool) -> Response {
    let message = if off {
        "This SLAPorSLOP server doesn't run repository or portfolio checks without an account. Sign in and use the maker tools instead.".to_string()
    } else {
        let minutes = retry_after.div_ceil(60).max(1);
        format!(
            "SLAPorSLOP has asked GitHub all it can for checks without an account this hour: try again in {minutes} minute{}. Signed-in makers can use Check my repository now meanwhile.",
            plural(minutes)
        )
    };
    refusal(
        StatusCode::SERVICE_UNAVAILABLE,
        "unavailable",
        message,
        retry_after,
    )
}

fn refusal(status: StatusCode, code: &str, message: String, retry_after: u64) -> Response {
    let body = json!({
        "summary": message,
        "error": { "code": code, "message": message },
        "retry_after_seconds": retry_after,
    });
    let mut res = (status, Json(body)).into_response();
    if let Ok(v) = HeaderValue::from_str(&retry_after.to_string()) {
        res.headers_mut().insert(RETRY_AFTER, v);
    }
    res
}

fn plural(n: u64) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn things(n: usize) -> String {
    if n == 1 {
        "1 thing".into()
    } else {
        format!("{n} things")
    }
}

// ---------------------------------------------------------------------------------------------
// Store addresses

#[derive(Deserialize)]
struct SlugQuery {
    slug: Option<String>,
}

#[derive(Serialize)]
struct AddressReport {
    summary: String,
    slug: String,
    valid: bool,
    available: bool,
    /// A valid, free address like this one, when this one isn't both.
    suggestion: Option<String>,
    rules: &'static str,
}

const SLUG_RULES: &str = "3 to 60 lowercase letters, numbers and single dashes, e.g. my-game. Each app's address is unique; its name needn't be. Addresses aren't held: someone else can take a free one before the store page is created.";

/// An address made from what was given: lower case, other characters as dashes. None when that
/// still isn't one (too short).
fn slug_like(input: &str) -> Option<String> {
    let mut out = String::new();
    for c in input.trim().chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.truncate(60);
    let out = out.trim_end_matches('-').to_string();
    catalog::validate_slug(&out).is_ok().then_some(out)
}

async fn store_address(State(state): State<AppState>, Query(q): Query<SlugQuery>) -> Response {
    let slug = q.slug.unwrap_or_default().trim().to_string();
    match address_report(&state, &slug).await {
        Ok(report) => Json(report).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn address_report(state: &AppState, slug: &str) -> crate::error::ApiResult<AddressReport> {
    if slug.is_empty() {
        return Ok(AddressReport {
            summary: "Give the store address to check as ?slug=my-game.".into(),
            slug: String::new(),
            valid: false,
            available: false,
            suggestion: None,
            rules: SLUG_RULES,
        });
    }
    let shown: String = slug.chars().take(80).collect();
    if catalog::validate_slug(slug).is_err() {
        let suggestion = match slug_like(slug) {
            Some(s) => Some(catalog::free_address(&state.db, &s).await?.unwrap_or(s)),
            None => None,
        };
        let summary = match &suggestion {
            Some(s) => format!(
                "\"{shown}\" isn't a valid store address: use 3 to 60 lowercase letters, numbers and single dashes. {s} is valid and free."
            ),
            None => format!(
                "\"{shown}\" isn't a valid store address: use 3 to 60 lowercase letters, numbers and single dashes, e.g. my-game."
            ),
        };
        return Ok(AddressReport {
            summary,
            slug: shown,
            valid: false,
            available: false,
            suggestion,
            rules: SLUG_RULES,
        });
    }
    let suggestion = catalog::free_address(&state.db, slug).await?;
    let summary = match &suggestion {
        None => format!(
            "{slug} is a valid store address and it's free: use it when the store page is created (and as `app` in {WORKFLOW_PATH} for Slop Build)."
        ),
        Some(free) => format!(
            "{slug} is taken by another app; {free} is free. Use that one (the app's name can stay the same)."
        ),
    };
    Ok(AddressReport {
        summary,
        slug: slug.to_string(),
        valid: true,
        available: suggestion.is_none(),
        suggestion,
        rules: SLUG_RULES,
    })
}

// ---------------------------------------------------------------------------------------------
// Asking GitHub, as nobody

#[derive(Deserialize)]
struct UrlQuery {
    url: Option<String>,
}

/// Why a repository address can't be checked.
enum NotGithub {
    /// Nothing given, or not an address.
    NoAddress,
    /// Another host: GitLab, Bitbucket, a self-hosted Git…
    Host(String),
}

/// `owner/name` of a GitHub repository, however people write its address: with or without
/// https:// and www., with .git, a trailing slash, or a page inside it (…/tree/main). Only
/// `github_repo_path`'s characters get through, and only into the configured GitHub bases.
fn github_repo(input: Option<&str>) -> Result<String, NotGithub> {
    let input = input
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 300);
    let Some(input) = input else {
        return Err(NotGithub::NoAddress);
    };
    let with_scheme = if input.contains("://") {
        input.to_string()
    } else {
        format!("https://{input}")
    };
    let url = url::Url::parse(&with_scheme).map_err(|_| NotGithub::NoAddress)?;
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    if !matches!(url.scheme(), "https" | "http") || host.is_empty() {
        return Err(NotGithub::NoAddress);
    }
    if host != "github.com" && host != "www.github.com" {
        return Err(NotGithub::Host(host));
    }
    let mut parts = url
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty());
    let (Some(owner), Some(name)) = (parts.next(), parts.next()) else {
        return Err(NotGithub::NoAddress);
    };
    slop_provenance::github_repo_path(&format!("https://github.com/{owner}/{name}"))
        .ok_or(NotGithub::NoAddress)
}

/// What GitHub's API answered.
enum Gh<T> {
    Found(T),
    /// Not there, or not public: GitHub answers the same to nobody.
    Missing,
    /// Renamed or transferred.
    Moved,
    /// No commits yet.
    Empty,
    /// SLAPorSLOP can't ask now, for this many seconds.
    Busy(u64),
    Failed(String),
}

/// A GET on GitHub's API without credentials, counted against the hourly share.
async fn ask_github<T: DeserializeOwned>(state: &AppState, path: &str) -> Gh<T> {
    if let Err(wait) = state.checks.take_github(Instant::now()) {
        return Gh::Busy(wait);
    }
    let Ok(url) = state.config.releases.github_api_base.join(path) else {
        return Gh::Failed("not a GitHub address".into());
    };
    // `http` follows no redirects and gives up after 10 seconds.
    let res = match state
        .http
        .get(url)
        .header("accept", "application/vnd.github+json")
        .header("user-agent", "slop-api")
        .send()
        .await
    {
        Ok(res) => res,
        Err(e) => return Gh::Failed(format!("couldn't reach GitHub: {e}")),
    };
    let status = res.status().as_u16();
    let header = |name: &str| {
        res.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
    };
    let used_up = status == 429 || (status == 403 && header("x-ratelimit-remaining") == Some(0));
    if used_up {
        let now_unix = OffsetDateTime::now_utc().unix_timestamp().max(0) as u64;
        let wait = header("retry-after")
            .or_else(|| header("x-ratelimit-reset").map(|at| at.saturating_sub(now_unix)))
            .unwrap_or(600)
            .clamp(1, 3600);
        if let Ok(mut resets) = state.checks.github_resets.lock() {
            *resets = Some(Instant::now() + Duration::from_secs(wait));
        }
        return Gh::Busy(wait);
    }
    match status {
        200 => match res.json().await {
            Ok(v) => Gh::Found(v),
            Err(e) => Gh::Failed(format!("GitHub's answer wasn't readable: {e}")),
        },
        404 | 451 => Gh::Missing,
        301 | 302 | 307 | 308 => Gh::Moved,
        409 => Gh::Empty,
        code => Gh::Failed(format!("GitHub answered {code}")),
    }
}

/// The newest commit on the default branch, or the answer to give instead of a report.
async fn head_commit(
    state: &AppState,
    repo: &str,
    for_what: &str,
) -> Result<String, Box<Response>> {
    #[derive(Deserialize)]
    struct Commit {
        sha: String,
    }
    let not_checked = |summary: String| Err(Box::new(not_checked(repo, summary)));
    match ask_github::<Commit>(state, &format!("repos/{repo}/commits/HEAD")).await {
        Gh::Found(c) => {
            let sha = c.sha.to_ascii_lowercase();
            // It goes into the files' addresses: only a commit ID.
            if sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                Ok(sha)
            } else {
                not_checked(
                    "GitHub's answer about the newest commit wasn't a commit ID; try again later."
                        .into(),
                )
            }
        }
        Gh::Missing => not_checked(format!(
            "GitHub can't find a public repository at https://github.com/{repo}. Check the address. {for_what}"
        )),
        Gh::Moved => not_checked(format!(
            "GitHub says https://github.com/{repo} moved (it was renamed or transferred): check it again at its current address."
        )),
        Gh::Empty => not_checked(format!(
            "https://github.com/{repo} has no commits yet: push the project first, then check again."
        )),
        Gh::Busy(wait) => Err(Box::new(github_busy(wait, state.checks.per_hour == 0))),
        Gh::Failed(why) => not_checked(format!(
            "SLAPorSLOP couldn't read https://github.com/{repo} from GitHub ({why}); try again in a few minutes."
        )),
    }
}

/// A report that couldn't be made, saying why (200: an AI reading it raw sees the reason).
fn not_checked(repo: &str, summary: String) -> Response {
    Json(json!({
        "summary": summary,
        "status": "not_checked",
        "repository": (!repo.is_empty()).then_some(repo),
    }))
    .into_response()
}

// ---------------------------------------------------------------------------------------------
// Repositories

#[derive(Serialize)]
struct RepositoryReport {
    summary: String,
    /// `must_fix` (SLAPorSLOP won't publish it as it is), `should_fix`, `looks_good`,
    /// `incomplete` (nothing found, but GitHub couldn't be asked about everything), or
    /// `not_checked`.
    status: &'static str,
    repository: String,
    commit: String,
    #[serde(with = "time::serde::rfc3339")]
    checked_at: OffsetDateTime,
    /// Most important first: `must`, then `should`, then `recommended`.
    findings: Vec<Fix>,
    /// What was checked and found fine.
    passed: Vec<String>,
    /// What this doesn't (or couldn't) check.
    not_checked: Vec<String>,
    licence: Value,
    pictures: Value,
    builds: Value,
    next: String,
    dry_run: &'static str,
}

/// The parts of SLAPorSLOP's requirements this report never covers.
const NEVER_CHECKED: &[&str] = &[
    "The source check (closed-source libraries, purchases, code loading, where saves go), the build's maker signature and the malware scan run on each release, after the store page is created.",
    "The rules (no sexual content, gambling or pay-to-win, an honest age rating) are for the owner to confirm; a person reviews the first release.",
];

async fn repository(State(state): State<AppState>, Query(q): Query<UrlQuery>) -> Response {
    let repo = match github_repo(q.url.as_deref()) {
        Ok(repo) => repo,
        Err(NotGithub::NoAddress) => {
            return not_checked(
                "",
                "Give the repository's address as ?url=https://github.com/OWNER/REPO.".into(),
            );
        }
        Err(NotGithub::Host(host)) => {
            return not_checked(
                "",
                format!(
                    "This check reads GitHub repositories only so far, and {host} isn't GitHub. SLAPorSLOP also publishes from GitLab, Bitbucket and self-hosted Git: the owner signs in, creates the store page, and SLAPorSLOP's checks run on the first release (store-page pictures come from GitHub repositories only)."
                ),
            );
        }
    };
    let commit = match head_commit(
        &state,
        &repo,
        "Apps in the store need a public repository; for a private app, the owner signs in and gives SLAPorSLOP access.",
    )
    .await
    {
        Ok(c) => c,
        Err(answer) => return *answer,
    };
    let key = format!("repository:{}:{commit}", repo.to_ascii_lowercase());
    if let Some(report) = state.checks.cached(&key) {
        return Json(report).into_response();
    }
    let report = repository_report(&state, &repo, &commit).await;
    let value = serde_json::to_value(&report).unwrap_or(Value::Null);
    state.checks.keep(key, &value);
    Json(value).into_response()
}

async fn repository_report(state: &AppState, repo: &str, commit: &str) -> RepositoryReport {
    let mut findings = Vec::new();
    let mut passed = Vec::new();
    let mut not_checked: Vec<String> = Vec::new();

    let licence = check_licence(state, repo, &mut findings, &mut passed, &mut not_checked).await;
    let pictures = check_pictures(state, repo, commit, &mut findings, &mut passed).await;
    let builds = check_builds(
        state,
        repo,
        commit,
        &mut findings,
        &mut passed,
        &mut not_checked,
    )
    .await;
    // Parts GitHub couldn't be asked about this time.
    let skipped = not_checked.len();
    not_checked.extend(NEVER_CHECKED.iter().map(|s| s.to_string()));

    let rank = |f: &Fix| match f.severity {
        "must" => 0,
        "should" => 1,
        _ => 2,
    };
    findings.sort_by_key(rank);
    let count = |s: &str| findings.iter().filter(|f| f.severity == s).count();
    let (must, should, recommended) = (count("must"), count("should"), count("recommended"));
    let at = format!("{repo} at commit {}", &commit[..7]);
    let mut recommended_too = match recommended {
        0 => String::new(),
        1 => " 1 more thing is recommended.".into(),
        n => format!(" {n} more things are recommended."),
    };
    if skipped > 0 && (must > 0 || should > 0) {
        recommended_too.push_str(&format!(
            " {} couldn't be checked this time (see not_checked).",
            if skipped == 1 {
                "1 part".to_string()
            } else {
                format!("{skipped} parts")
            }
        ));
    }
    let (status, summary) = if must > 0 {
        (
            "must_fix",
            format!(
                "{at}: {} must be fixed before SLAPorSLOP can publish it{}.{recommended_too} Each finding says what's wrong and how to fix it.",
                things(must),
                match should {
                    0 => String::new(),
                    n => format!(", and {} should be", things(n)),
                }
            ),
        )
    } else if should > 0 {
        (
            "should_fix",
            format!(
                "{at}: nothing found blocks publishing, but {} should be fixed.{recommended_too} Each finding says what's wrong and how to fix it.",
                things(should)
            ),
        )
    } else if skipped > 0 {
        (
            "incomplete",
            format!(
                "{at}: nothing found needs fixing, but not everything could be checked this time (see not_checked).{recommended_too} Check again later."
            ),
        )
    } else {
        (
            "looks_good",
            format!(
                "{at} passed what this checks (licence, pictures and builds).{recommended_too} SLAPorSLOP's source check and a person's review still run on the first release."
            ),
        )
    };
    let guide = state
        .config
        .public_url
        .join("publishing.md")
        .map(|u| u.to_string())
        .unwrap_or_default();
    RepositoryReport {
        summary,
        status,
        repository: repo.to_string(),
        commit: commit.to_string(),
        checked_at: OffsetDateTime::now_utc(),
        findings,
        passed,
        not_checked,
        licence,
        pictures,
        builds,
        next: format!(
            "Fix the findings in the repository (must first, then should), commit and push, then check again: a report for one commit is kept for {} minutes. Then the owner signs in to SLAPorSLOP, checks the store address with /v1/check/store-address, and creates the store page. The publishing guide: {guide}",
            CACHE_FOR.as_secs() / 60
        ),
        dry_run: DRY_RUN,
    }
}

/// The publishing guide's section on what SLAPorSLOP asks of a project.
const REQUIREMENTS: &str = "What SLAPorSLOP asks of your project";

async fn check_licence(
    state: &AppState,
    repo: &str,
    findings: &mut Vec<Fix>,
    passed: &mut Vec<String>,
    not_checked: &mut Vec<String>,
) -> Value {
    let (found, status) = match catalog::cached_license(state, repo) {
        Some(found) => (found, 200),
        None => match state.checks.take_github(Instant::now()) {
            Err(_) => {
                not_checked.push(
                    "The licence: SLAPorSLOP has asked GitHub all it can for this hour. Check again later."
                        .into(),
                );
                return Value::Null;
            }
            Ok(()) => match catalog::ask_license(state, repo, false).await {
                Ok(answer) => answer,
                Err(_) => {
                    not_checked
                        .push("The licence: GitHub couldn't be asked. Check again later.".into());
                    return Value::Null;
                }
            },
        },
    };
    match (status, &found.spdx) {
        (200, Some(spdx)) => {
            let not_osi = source_scan::not_osi_approved(spdx);
            let name = found.name.clone().unwrap_or_else(|| spdx.clone());
            if not_osi.is_empty() {
                passed.push(format!("Licence: {spdx} ({name}), OSI-approved."));
            } else {
                findings.push(Fix {
                    kind: "licence:not_osi".into(),
                    severity: "must",
                    problem: format!(
                        "The repository's licence is {spdx}, and {} isn't on SLAPorSLOP's list of OSI-approved licences, which apps in the store need (opensource.org/licenses).",
                        not_osi.join(", ")
                    ),
                    instruction: "Ask the owner whether they want to release the project under an OSI-approved licence such as MIT, Apache-2.0, MPL-2.0 or GPL-3.0-or-later: only the copyright holders can change it, so don't choose for them. If they do, replace LICENSE with that licence's full, official text from https://spdx.org/licenses/ (fill in the year and copyright holder where it asks). SLAPorSLOP asks GitHub about a repository's licence once a day, so the change shows here by the next day.".into(),
                    guide_section: Some(REQUIREMENTS),
                });
            }
            json!({ "spdx": spdx, "name": found.name, "osi_approved": not_osi.is_empty() })
        }
        (200, None) => {
            findings.push(Fix {
                kind: "licence:unidentified".into(),
                severity: "should",
                problem: format!(
                    "There's a licence file, but GitHub can't tell which licence it is{}, so SLAPorSLOP can't confirm it's OSI-approved.",
                    found
                        .name
                        .as_deref()
                        .map(|n| format!(" (it says \"{n}\")"))
                        .unwrap_or_default()
                ),
                instruction: "Ask the owner which licence they chose. Replace LICENSE with that licence's unchanged, official text from https://spdx.org/licenses/ (only the year and copyright holder filled in), so it can be recognised. A licence that isn't OSI-approved, or written for the project, can't go in the store.".into(),
                guide_section: Some(REQUIREMENTS),
            });
            json!({ "spdx": null, "name": found.name, "osi_approved": null })
        }
        (404, _) => {
            findings.push(Fix {
                kind: "licence:no_file".into(),
                severity: "must",
                problem: "GitHub finds no licence file in the repository. SLAPorSLOP publishes open-source projects only, with an OSI-approved licence in a LICENSE file.".into(),
                instruction: "Ask the owner which OSI-approved licence they want (MIT, Apache-2.0, MPL-2.0 and GPL-3.0-or-later are common); don't choose one for them. Then add its full, official text as LICENSE at the top of the repository: copy it from https://spdx.org/licenses/ (e.g. https://spdx.org/licenses/MIT.html), don't write or shorten it, and fill in the year and copyright holder where it asks.".into(),
                guide_section: Some(REQUIREMENTS),
            });
            json!({ "spdx": null, "name": null, "osi_approved": null })
        }
        (code, _) => {
            not_checked.push(format!(
                "The licence: GitHub answered {code}. Check again later."
            ));
            Value::Null
        }
    }
}

const PICTURES_FIX: &str = "Fix it in .slop/ (the picture, or .slop/media.json), commit and push, then check again. Until every picture reads without problems, the store page can't use any of them.";

async fn check_pictures(
    state: &AppState,
    repo: &str,
    commit: &str,
    findings: &mut Vec<Fix>,
    passed: &mut Vec<String>,
) -> Value {
    match media::read_and_check(state, repo, commit, true).await {
        Ok(None) => {
            findings.push(Fix {
                kind: "pictures:none".into(),
                severity: "recommended",
                problem: format!(
                    "There's no {}, so the store page would have no icon, thumbnail or screenshots.",
                    media::MANIFEST_PATH
                ),
                instruction: format!(
                    "Create {} listing an icon (a square PNG, {best}×{best} is best), a thumbnail (920×430 is best, at least 460×215) and a few screenshots (at least 320×180), with the pictures in .slop/, e.g. {{ \"icon\": \"icon.png\", \"thumbnail\": \"thumbnail.png\", \"gallery\": [{{ \"image\": \"screenshots/1.png\" }}] }}. Use the app's own art and real screenshots, never pictures that suggest another game, brand or maker. Commit and push, then check again; a person reviews the pictures before they show.",
                    media::MANIFEST_PATH,
                    best = media::ICON_BEST
                ),
                guide_section: Some(media::GUIDE_SECTION),
            });
            json!({ "manifest": media::MANIFEST_PATH, "found": false })
        }
        Ok(Some(checked)) => {
            if checked.icon.is_none() {
                findings.push(media::missing_icon());
            }
            if checked.thumbnail.is_none() {
                findings.push(Fix {
                    kind: "pictures:no_thumbnail".into(),
                    severity: "recommended",
                    problem: format!(
                        "{} names no thumbnail, so the app's tile in the store has no picture of its own.",
                        media::MANIFEST_PATH
                    ),
                    instruction: "Add a thumbnail: 920×430 is best (at least 460×215), made from the app's own art or a real screenshot. Save it as .slop/thumbnail.png and add \"thumbnail\": \"thumbnail.png\" to .slop/media.json.".into(),
                    guide_section: Some(media::GUIDE_SECTION),
                });
            }
            passed.push(format!(
                "Pictures: {} read without problems ({}{}{} gallery item{}).",
                media::MANIFEST_PATH,
                if checked.icon.is_some() {
                    "an icon, "
                } else {
                    ""
                },
                if checked.thumbnail.is_some() {
                    "a thumbnail, "
                } else {
                    ""
                },
                checked.gallery.len(),
                plural(checked.gallery.len() as u64)
            ));
            json!({
                "manifest": media::MANIFEST_PATH,
                "found": true,
                "icon": checked.icon.is_some(),
                "thumbnail": checked.thumbnail.is_some(),
                "gallery_items": checked.gallery.len(),
                "problems": 0,
            })
        }
        Err(problems) => {
            let n = problems.len();
            for Problem(p) in problems {
                let (kind, problem) = match p.strip_prefix("icon: ") {
                    Some(rest) => ("icon:problem", format!("The icon: {rest}")),
                    None => ("pictures:problem", p),
                };
                findings.push(Fix {
                    kind: kind.into(),
                    severity: "should",
                    problem,
                    instruction: PICTURES_FIX.into(),
                    guide_section: Some(media::GUIDE_SECTION),
                });
            }
            json!({ "manifest": media::MANIFEST_PATH, "found": true, "problems": n })
        }
    }
}

/// A release file's platform, by its type (publishing.md's table).
fn platform_of(name: &str) -> Option<&'static str> {
    let name = name.to_ascii_lowercase();
    if name.ends_with(".zip") {
        Some("windows")
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        Some("linux")
    } else if name.ends_with(".deb") {
        Some("linux-deb")
    } else if name.ends_with(".rpm") {
        Some("linux-rpm")
    } else {
        None
    }
}

/// `key: value` in the workflow's YAML, by plain reading (the first one, without quotes).
fn workflow_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let rest = line.trim_start().strip_prefix(key)?.strip_prefix(':')?;
        let value = rest.split(" #").next()?.trim().trim_matches(['"', '\'']);
        (!value.is_empty()).then(|| value.to_string())
    })
}

const RELEASES: &str = "3. Submit a release";

async fn check_builds(
    state: &AppState,
    repo: &str,
    commit: &str,
    findings: &mut Vec<Fix>,
    passed: &mut Vec<String>,
    not_checked: &mut Vec<String>,
) -> Value {
    let workflow = match state
        .config
        .media
        .github_raw_base
        .join(&format!("{repo}/{commit}/{WORKFLOW_PATH}"))
    {
        Ok(url) => media::download(state, url, MAX_WORKFLOW_BYTES).await,
        Err(e) => Err(e.into()),
    };
    let workflow = match workflow {
        Ok(found) => found.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
        Err(e) => {
            not_checked.push(format!("Builds: couldn't read {WORKFLOW_PATH} ({e:#})."));
            return Value::Null;
        }
    };
    if let Some(text) = workflow {
        let app = workflow_value(&text, "app");
        let slop_url = workflow_value(&text, "slop-url");
        if !text.contains("slop-build/.github/workflows/build.yml") {
            findings.push(Fix {
                kind: "builds:workflow".into(),
                severity: "should",
                problem: format!("{WORKFLOW_PATH} doesn't use Slop Build's shared workflow."),
                instruction: format!(
                    "Make {WORKFLOW_PATH} call it, as in the publishing guide's Option B: a job with `uses: DocFoxHound/slop-build/.github/workflows/build.yml@v1` and `with:` app, slop-url and project-type. If it's another workflow that only shares the name, rename it."
                ),
                guide_section: Some(RELEASES),
            });
        }
        let site = &state.config.public_url;
        let ours = |u: &str| {
            url::Url::parse(u).is_ok_and(|u| {
                u.host_str() == site.host_str()
                    || state
                        .config
                        .former_urls
                        .iter()
                        .any(|f| f.host_str() == u.host_str())
            })
        };
        match slop_url.as_deref() {
            Some(u) if ours(u) => {}
            other => findings.push(Fix {
                kind: "builds:slop_url".into(),
                severity: "should",
                problem: match other {
                    Some(u) => format!(
                        "{WORKFLOW_PATH} sends releases to {u}, which isn't this SLAPorSLOP site."
                    ),
                    None => format!("{WORKFLOW_PATH} has no slop-url."),
                },
                instruction: format!(
                    "Set `slop-url: {}` under `with:` in {WORKFLOW_PATH}.",
                    site.as_str().trim_end_matches('/')
                ),
                guide_section: Some(RELEASES),
            }),
        }
        let mut app_note = None;
        match app.as_deref() {
            None => findings.push(Fix {
                kind: "builds:app".into(),
                severity: "should",
                problem: format!("{WORKFLOW_PATH} has no app (the store address)."),
                instruction: format!(
                    "Set `app:` under `with:` in {WORKFLOW_PATH} to the store address the owner will create the store page with; check it's free with /v1/check/store-address?slug=…"
                ),
                guide_section: Some(RELEASES),
            }),
            Some(slug) if catalog::validate_slug(slug).is_err() => findings.push(Fix {
                kind: "builds:app".into(),
                severity: "should",
                problem: format!(
                    "{WORKFLOW_PATH} says app: {slug}, which isn't a valid store address."
                ),
                instruction: format!(
                    "Use 3 to 60 lowercase letters, numbers and single dashes{}, the same address the store page will have.",
                    slug_like(slug)
                        .map(|s| format!(" (e.g. {s})"))
                        .unwrap_or_default()
                ),
                guide_section: Some(RELEASES),
            }),
            Some(slug) => {
                app_note = match catalog::free_address(&state.db, slug).await {
                    Ok(None) => Some(format!("{slug} is free")),
                    Ok(Some(free)) => Some(format!(
                        "{slug} is taken: if it isn't the owner's own app, use a free one such as {free}"
                    )),
                    Err(_) => None,
                };
            }
        }
        passed.push(format!(
            "Builds: {WORKFLOW_PATH} is there (Slop Build builds, signs and releases on each push to the main branch or a v* tag){}.",
            match (&app, &app_note) {
                (Some(a), Some(note)) => format!("; its app is {a} ({note})"),
                _ => String::new(),
            }
        ));
        return json!({
            "slop_build_workflow": true,
            "workflow_app": app,
            "workflow_app_note": app_note,
            "latest_release": null,
        });
    }

    // No Slop Build: the newest GitHub release's files (publishing.md, Option A).
    #[derive(Deserialize)]
    struct Asset {
        name: String,
    }
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
        html_url: Option<String>,
        #[serde(default)]
        assets: Vec<Asset>,
    }
    let no_builds = |findings: &mut Vec<Fix>, problem: String| {
        findings.push(Fix {
            kind: "builds:none".into(),
            severity: "should",
            problem,
            instruction: format!(
                "Either add {WORKFLOW_PATH} (Slop Build: it builds a .zip for Windows and a .tar.gz, .deb and .rpm for Linux, signs them and makes the release on each push), or publish a GitHub release with the files the store page will ask for: a .zip of the app for Windows, and/or a .tar.gz plus a .deb or .rpm for Linux. Without builds there's nothing to publish."
            ),
            guide_section: Some(RELEASES),
        })
    };
    match ask_github::<Release>(state, &format!("repos/{repo}/releases/latest")).await {
        Gh::Found(release) => {
            let files: Vec<Value> = release
                .assets
                .iter()
                .map(|a| json!({ "name": a.name, "platform": platform_of(&a.name) }))
                .collect();
            let mut platforms: Vec<&str> = release
                .assets
                .iter()
                .filter_map(|a| platform_of(&a.name))
                .collect();
            platforms.sort();
            platforms.dedup();
            let has = |p: &str| platforms.contains(&p);
            let tag = &release.tag_name;
            if platforms.is_empty() {
                no_builds(
                    findings,
                    format!(
                        "The newest GitHub release, {tag}, has no .zip, .tar.gz, .deb or .rpm files, and there's no {WORKFLOW_PATH}."
                    ),
                );
            } else {
                if has("linux") && !has("linux-deb") && !has("linux-rpm") {
                    findings.push(Fix {
                        kind: "builds:linux_package".into(),
                        severity: "should",
                        problem: format!("The newest GitHub release, {tag}, has a .tar.gz for Linux but no .deb or .rpm package."),
                        instruction: "Linux on SLAPorSLOP needs the .tar.gz plus at least one package: attach a .deb, an .rpm or both to each release (Slop Build makes them), or don't offer Linux.".into(),
                        guide_section: Some(REQUIREMENTS),
                    });
                }
                if (has("linux-deb") || has("linux-rpm")) && !has("linux") {
                    findings.push(Fix {
                        kind: "builds:linux_archive".into(),
                        severity: "should",
                        problem: format!("The newest GitHub release, {tag}, has a Linux package but no .tar.gz of the app."),
                        instruction: "Attach a .tar.gz of the app's files, with the program to start inside it, to each release: the SLAPorSLOP app installs that one, and offers the packages beside it.".into(),
                        guide_section: Some(REQUIREMENTS),
                    });
                }
                passed.push(format!(
                    "Builds: the newest GitHub release, {tag}, has files for {}.",
                    platforms.join(", ")
                ));
            }
            json!({
                "slop_build_workflow": false,
                "latest_release": {
                    "tag": tag,
                    "url": release.html_url,
                    "files": files,
                    "platforms": platforms,
                },
            })
        }
        Gh::Missing => {
            no_builds(
                findings,
                format!(
                    "There's no GitHub release yet (drafts and pre-releases don't count), and no {WORKFLOW_PATH}."
                ),
            );
            json!({ "slop_build_workflow": false, "latest_release": null })
        }
        Gh::Busy(_) => {
            not_checked.push(format!(
                "Builds: there's no {WORKFLOW_PATH}, and SLAPorSLOP has asked GitHub all it can this hour, so the releases weren't looked at. Check again later."
            ));
            json!({ "slop_build_workflow": false })
        }
        Gh::Moved | Gh::Empty | Gh::Failed(_) => {
            not_checked.push(format!(
                "Builds: there's no {WORKFLOW_PATH}, and GitHub's list of releases couldn't be read. Check again later."
            ));
            json!({ "slop_build_workflow": false })
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Portfolios

#[derive(Serialize)]
struct PortfolioReport {
    summary: String,
    /// `must_fix` (a People page can't use it as it is), `looks_good`, or `not_checked`.
    status: &'static str,
    repository: String,
    commit: String,
    #[serde(with = "time::serde::rfc3339")]
    checked_at: OffsetDateTime,
    /// Every problem, each naming the work or file and what to change.
    problems: Vec<String>,
    /// What was read, when it read without problems.
    found: Value,
    /// What the page design's checks dropped (it still shows without it).
    design_warnings: Vec<String>,
    next: String,
    dry_run: &'static str,
}

async fn portfolio_check(State(state): State<AppState>, Query(q): Query<UrlQuery>) -> Response {
    let repo = match github_repo(q.url.as_deref()) {
        Ok(repo) => repo,
        Err(NotGithub::NoAddress) => {
            return not_checked(
                "",
                "Give the portfolio repository's address as ?url=https://github.com/OWNER/REPO."
                    .into(),
            );
        }
        Err(NotGithub::Host(host)) => {
            return not_checked(
                "",
                format!(
                    "Portfolios come from public GitHub repositories only, and {host} isn't GitHub: put the portfolio in one, e.g. https://github.com/you/portfolio."
                ),
            );
        }
    };
    let commit = match head_commit(
        &state,
        &repo,
        "A portfolio's repository has to be public: people see the files straight from GitHub.",
    )
    .await
    {
        Ok(c) => c,
        Err(answer) => return *answer,
    };
    let key = format!("portfolio:{}:{commit}", repo.to_ascii_lowercase());
    if let Some(report) = state.checks.cached(&key) {
        return Json(report).into_response();
    }
    let report = portfolio_report(&state, &repo, &commit).await;
    let value = serde_json::to_value(&report).unwrap_or(Value::Null);
    state.checks.keep(key, &value);
    Json(value).into_response()
}

async fn portfolio_report(state: &AppState, repo: &str, commit: &str) -> PortfolioReport {
    let at = format!("{repo} at commit {}", &commit[..7]);
    let (problems, found, design_warnings) = match portfolio::read_and_check(
        state, repo, commit, true,
    )
    .await
    {
        Ok(None) => (
            vec![format!(
                "There's no {} yet: create it, listing the works, e.g. {{ \"avatar\": \"avatar.png\", \"works\": [{{ \"image\": \"works/first.png\", \"title\": \"First piece\" }}] }}, with the files in .slop/",
                portfolio::MANIFEST_PATH
            )],
            json!({ "manifest": portfolio::MANIFEST_PATH, "found": false }),
            vec![],
        ),
        Ok(Some(checked)) => {
            let warnings = checked
                .design
                .as_ref()
                .map(|(d, _)| d.warnings.clone())
                .unwrap_or_default();
            (
                vec![],
                json!({
                    "manifest": portfolio::MANIFEST_PATH,
                    "found": true,
                    "avatar": checked.avatar.is_some(),
                    "banner": checked.banner.is_some(),
                    "works": checked.works.len(),
                    "page_design": checked.design.is_some(),
                }),
                warnings,
            )
        }
        Err(problems) => (
            problems,
            json!({ "manifest": portfolio::MANIFEST_PATH, "found": true }),
            vec![],
        ),
    };
    let (status, summary) = if problems.is_empty() {
        (
            "looks_good",
            format!(
                "{at}: the portfolio reads without problems{}. A person still reviews it before it shows on a People page.",
                match design_warnings.len() {
                    0 => String::new(),
                    n => format!(
                        "; the page design's checks dropped {} (see design_warnings)",
                        things(n)
                    ),
                }
            ),
        )
    } else {
        (
            "must_fix",
            format!(
                "{at}: {} to fix before a People page can use this portfolio. Each problem names the work or file and what to change.",
                things(problems.len())
            ),
        )
    };
    let guide = state
        .config
        .public_url
        .join("portfolio.md")
        .map(|u| u.to_string())
        .unwrap_or_default();
    PortfolioReport {
        summary,
        status,
        repository: repo.to_string(),
        commit: commit.to_string(),
        checked_at: OffsetDateTime::now_utc(),
        problems,
        found,
        design_warnings,
        next: format!(
            "Fix each problem in the repository, commit and push, then check again: a report for one commit is kept for {} minutes. When it's clean, the person signs in to SLAPorSLOP, sets up their People page with this repository and presses Check my portfolio now. The portfolio guide: {guide}",
            CACHE_FOR.as_secs() / 60
        ),
        dry_run: DRY_RUN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_addresses_as_people_write_them() {
        for given in [
            "https://github.com/octo/game",
            "github.com/octo/game",
            "http://www.github.com/octo/game.git",
            "https://github.com/octo/game/tree/main/src",
            "  https://github.com/octo/game/  ",
        ] {
            assert_eq!(
                github_repo(Some(given)).ok().as_deref(),
                Some("octo/game"),
                "{given}"
            );
        }
        assert!(matches!(
            github_repo(Some("https://gitlab.com/octo/game")),
            Err(NotGithub::Host(h)) if h == "gitlab.com"
        ));
        for bad in [
            None,
            Some(""),
            Some("https://github.com/octo"),
            Some("https://github.com/octo/ga%20me"),
            Some("ftp://github.com/octo/game"),
            Some("not an address at all"),
        ] {
            assert!(
                matches!(github_repo(bad), Err(NotGithub::NoAddress)),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn addresses_are_made_from_what_was_given() {
        assert_eq!(slug_like("My_Game").as_deref(), Some("my-game"));
        assert_eq!(
            slug_like("  Angry  Herds!! ").as_deref(),
            Some("angry-herds")
        );
        assert_eq!(slug_like("--x--"), None);
        assert_eq!(slug_like(&"a".repeat(80)).map(|s| s.len()), Some(60));
    }

    #[test]
    fn release_files_and_workflow_values() {
        assert_eq!(platform_of("Game-1.0-Windows.ZIP"), Some("windows"));
        assert_eq!(platform_of("game.tgz"), Some("linux"));
        assert_eq!(platform_of("game_1.0_amd64.deb"), Some("linux-deb"));
        assert_eq!(platform_of("checksums.txt"), None);
        let yml = "jobs:\n  slop:\n    with:\n      app: my-game   # your store address\n      slop-url: \"https://slop.example\"\n";
        assert_eq!(workflow_value(yml, "app").as_deref(), Some("my-game"));
        assert_eq!(
            workflow_value(yml, "slop-url").as_deref(),
            Some("https://slop.example")
        );
        assert_eq!(workflow_value(yml, "project-type"), None);
    }

    /// Findings point at the publishing guide's sections by their headings
    /// (`GET /v1/maker/publishing-guide/{section}` finds `##` and `###` ones).
    #[test]
    fn the_guide_sections_named_are_there() {
        let guide = include_str!("../../../web/public/publishing.md");
        for heading in [
            format!("## {REQUIREMENTS}"),
            format!("### {RELEASES}"),
            format!("### {}", media::GUIDE_SECTION),
        ] {
            assert!(guide.lines().any(|l| l == heading), "{heading}");
        }
    }

    #[test]
    fn githubs_share_runs_out_and_comes_back() {
        let shared = Shared::new(2);
        let t0 = Instant::now();
        assert!(shared.take_github(t0).is_ok());
        assert!(shared.take_github(t0).is_ok());
        let wait = shared.take_github(t0).unwrap_err();
        assert_eq!(wait, 1800);
        assert!(shared.take_github(t0 + Duration::from_secs(1800)).is_ok());
        // 0 turns the checks' GitHub requests off.
        assert!(Shared::new(0).take_github(t0).is_err());
    }
}
