// SPDX-License-Identifier: AGPL-3.0-or-later
//! Releases: makers submit them, SLAPorSLOP fetches and hashes their files, a reviewer approves them
//! and SLAPorSLOP signs the manifest (docs/ARCHITECTURE.md §6.3, docs/SECURITY.md §5).
//!
//! Builds stay on the maker's release page. SLAPorSLOP downloads each file once to hash it, keeps
//! nothing but the hash and size, and the client later downloads from the same URL and checks
//! the hash against SLAPorSLOP's signed manifest.
//!
//! A browser game's release is a play address instead of files (DESIGN §8): SLAPorSLOP checks it
//! (https, a public site, the bad-link lists, and that the page loads), a person approves it, and
//! nothing is signed. Play then opens it (`catalog::PLAY_ADDRESS`). SLAPorSLOP checks the
//! address, not what the site serves.

use std::{sync::atomic::Ordering, time::Duration as StdDuration};

use anyhow::{Context, anyhow, bail};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, patch, post},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use slop_manifest::{
    FileKind, FileMatch, FileVerification, MatchStatus, Platform, Purpose, REVOCATIONS_SCHEMA,
    RevocationList, Revoked, Signed, VERIFICATION_SCHEMA, is_safe_relative_path,
};
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};
use url::Url;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::{CurrentUser, MaybeUser},
    catalog,
    error::{ApiError, ApiResult, is_unique_violation},
    policy::ViewerPolicy,
};

/// How long a signed revocation list is valid. Clients refresh at least this often.
pub const REVOCATION_LIST_LIFETIME: Duration = Duration::hours(6);
/// A fetch that hasn't moved on for this long is presumed dead and retried. Each step of each
/// file renews it (`file_stage`), so it only has to outlast one file's longest scan (scan.rs).
const FETCH_LEASE: Duration = Duration::minutes(40);
/// The same for a play address, whose check takes seconds (at most `AppState.page_check_deadline`
/// for the page, plus the commit): one that died with the server is checked again soon.
const ADDRESS_FETCH_LEASE: Duration = Duration::minutes(3);

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/apps/{slug}/releases",
            get(list_releases).post(submit_release),
        )
        .route("/apps/{slug}/releases/{release_id}", patch(edit_release))
        .route(
            "/apps/{slug}/releases/{release_id}/withdraw",
            post(withdraw_release),
        )
        .route("/apps/{slug}/manifest", get(latest_manifest))
        .route("/runtimes", get(runtimes_list))
        .route(
            "/apps/{slug}/releases/{release_id}/download/{platform}",
            get(file_download_link),
        )
        .route("/revocations", get(revocations))
        .route("/keys", get(keys))
        .route("/verify/{sha256}", get(verify_file))
        .route("/server", get(server_info))
}

// ---------------------------------------------------------------------------------------------
// Submitting

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewFile {
    pub platform: String,
    pub kind: String,
    pub url: String,
    pub exec: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    /// A file over GitHub's 2 GiB limit, as parts in order (`<file>.001`, `<file>.002`…). `url`
    /// then names the joined file, which isn't uploaded itself.
    #[serde(default)]
    pub parts: Vec<String>,
}

/// Most parts one file may be split into (each up to the per-file limit).
pub const MAX_PARTS: usize = 64;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewRelease {
    version: String,
    /// The commit it's built from. A browser game on GitHub may leave it out: its site is built
    /// from the repository, and SLAPorSLOP takes the default branch's newest commit.
    #[serde(default)]
    commit_sha: Option<String>,
    /// One build per platform on the store page; none for a browser game.
    #[serde(default)]
    files: Vec<NewFile>,
    /// A browser game's release: the https address it plays at, instead of files.
    #[serde(default)]
    play_address: Option<String>,
    /// This release's rating (AGE-7); the store page's when left out.
    #[serde(default)]
    rating: Option<Rating>,
    /// Shared runtimes it needs (slop-manifest runtimes.rs); the last approved release's when
    /// left out.
    #[serde(default)]
    runtimes: Option<Vec<String>>,
    /// What changed, for players (Markdown).
    #[serde(default)]
    notes: Option<String>,
}

/// A release's age rating and content labels (AGE-7).
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rating {
    pub min_age: i16,
    pub content_labels: Vec<String>,
}

/// Whether `release`'s rating is below the last approved release's: a lower age or a label
/// dropped. Such an update waits for a person (updates.rs).
pub(crate) async fn rating_lowered(db: &sqlx::PgPool, release: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "select coalesce((
             select r.min_age < p.min_age or not (r.content_labels @> p.content_labels)
             from releases r, releases p
             where r.id = $1 and p.app_id = r.app_id and p.id <> r.id and p.status = 'approved'
               and p.min_age is not null and r.min_age is not null
             order by p.reviewed_at desc limit 1), false)",
    )
    .bind(release)
    .fetch_one(db)
    .await
}

struct CheckedFile {
    platform: Platform,
    kind: FileKind,
    url: String,
    file_name: String,
    exec: Option<String>,
    args: Vec<String>,
    /// (url, file name) of each part, in order.
    parts: Vec<(String, String)>,
}

fn validate_version(v: &str) -> ApiResult<()> {
    let ok = (1..=40).contains(&v.len())
        && v.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c));
    if !ok {
        return Err(ApiError::invalid(
            "version",
            "use up to 40 letters, numbers, dots and dashes, like 1.2.0",
        ));
    }
    Ok(())
}

pub(crate) fn validate_commit(sha: &str) -> ApiResult<()> {
    if sha.len() != 40
        || !sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(ApiError::invalid(
            "commit_sha",
            "use the full 40-character commit SHA, in lowercase",
        ));
    }
    Ok(())
}

fn is_github(url: &Url) -> bool {
    url.host_str() == Some("github.com")
}

/// Release files must live on the maker's own release page (ARCHITECTURE §6.4): for GitHub,
/// under `<repo>/releases/download/`; elsewhere, on the repo's own host.
pub(crate) fn validate_file_url(
    repo_url: &str,
    value: &str,
    allow_http: bool,
) -> ApiResult<(String, String)> {
    let invalid = |msg: &str| ApiError::invalid("files", msg.to_string());
    let url = Url::parse(value.trim()).map_err(|_| invalid("a file URL isn't valid"))?;
    let repo = Url::parse(repo_url).map_err(|_| invalid("the app's repository URL isn't valid"))?;
    let scheme_ok = url.scheme() == "https" || (allow_http && url.scheme() == "http");
    if !scheme_ok
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid("file URLs must be plain https links"));
    }
    if is_github(&repo) {
        let prefix = format!("{}/releases/download/", repo_url.trim_end_matches('/'));
        if !url.as_str().starts_with(&prefix) || url.query().is_some() {
            return Err(invalid(
                "files must be on this repository's GitHub releases page",
            ));
        }
    } else if url.host_str() != repo.host_str()
        || url.port_or_known_default() != repo.port_or_known_default()
    {
        return Err(invalid("files must be on the same site as the repository"));
    }
    let file_name = url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| invalid("a file URL has no file name"))?;
    let file_name = percent_encoding::percent_decode_str(file_name)
        .decode_utf8()
        .map_err(|_| invalid("a file name isn't valid text"))?;
    if file_name.contains('/')
        || file_name.contains('\\')
        || file_name.chars().any(char::is_control)
    {
        return Err(invalid("a file name isn't valid"));
    }
    let file_name = file_name.into_owned();
    Ok((url.into(), file_name))
}

/// The longest play address (the database checks the same).
const MAX_PLAY_ADDRESS: usize = 2000;

/// SLAPorSLOP's own sites, as (host, port): its address and former addresses, the trusted
/// origins and the demos' origin. A play address is never one of them or under one: Play would
/// open it with SLAPorSLOP's cookies.
pub(crate) fn own_sites(config: &crate::config::Config) -> Vec<(String, u16)> {
    let mut urls = vec![config.public_url.clone()];
    urls.extend(config.former_urls.iter().cloned());
    urls.extend(
        config
            .trusted_origins
            .iter()
            .filter_map(|o| Url::parse(o).ok()),
    );
    urls.extend(config.demos.play_url.iter().cloned());
    urls.iter()
        .filter_map(|u| {
            Some((
                u.host_str()?.trim_end_matches('.').to_ascii_lowercase(),
                u.port_or_known_default()?,
            ))
        })
        .collect()
}

/// Whether `url` is on one of `own` sites, or under one. On test setups (`allow_http`) every
/// site runs on this machine, so the port tells them apart; otherwise the name alone does, as
/// cookies don't care about ports.
fn is_own_site(url: &Url, own: &[(String, u16)], allow_http: bool) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let port = url.port_or_known_default();
    own.iter().any(|(h, p)| {
        (host == *h || host.ends_with(&format!(".{h}"))) && (!allow_http || port == Some(*p))
    })
}

/// A browser game's play address (DESIGN §8): the plain https address of a public website that
/// isn't SLAPorSLOP's own, without a user name or password. Checked without the network; the
/// site's name is then checked to lead to public addresses (`repo_owner::public_host`) and
/// against the bad-link lists, and the fetcher loads the page (`check_play_address`). Plain
/// http, IP addresses and this machine pass only on test setups (`allow_http`). Returns the
/// address as it's kept.
pub(crate) fn validate_play_address(
    value: &str,
    allow_http: bool,
    own: &[(String, u16)],
) -> ApiResult<String> {
    let invalid = |msg: &str| ApiError::invalid("play_address", msg.to_string());
    let value = value.trim();
    if value.is_empty() {
        return Err(invalid("give the https:// address your game plays at"));
    }
    let too_long = || invalid("use an address of up to 2,000 characters");
    if value.chars().count() > MAX_PLAY_ADDRESS {
        return Err(too_long());
    }
    let mut url =
        Url::parse(value).map_err(|_| invalid("use the https:// address your game plays at"))?;
    drop_trailing_dot(&mut url)
        .map_err(|_| invalid("use the https:// address your game plays at"))?;
    if !(url.scheme() == "https" || (allow_http && url.scheme() == "http")) {
        return Err(invalid(
            "use an https:// address: it's what players' browsers open",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid(
            "leave the user name and password out of the address",
        ));
    }
    let Some(host) = url.host() else {
        return Err(invalid("use the https:// address your game plays at"));
    };
    if !allow_http {
        let ip: Option<std::net::IpAddr> = match host {
            url::Host::Ipv4(ip) => Some(ip.into()),
            url::Host::Ipv6(ip) => Some(ip.into()),
            url::Host::Domain(_) => None,
        };
        if let Some(ip) = ip {
            return Err(invalid(if crate::repo_owner::private_ip(ip) {
                "that address is on a private network"
            } else {
                "use the site's name, not its IP address"
            }));
        }
        let name = url
            .host_str()
            .unwrap_or("")
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if name == "localhost" || name.ends_with(".localhost") {
            return Err(invalid("that address is on a private network"));
        }
        if !name.contains('.') {
            return Err(invalid("use the site's full name, like mygame.example.com"));
        }
    }
    if is_own_site(&url, own, allow_http) {
        return Err(invalid(
            "that's SLAPorSLOP's own site: give the address of the site your game plays on",
        ));
    }
    let address = url.to_string();
    if address.chars().count() > MAX_PLAY_ADDRESS {
        return Err(too_long());
    }
    Ok(address)
}

/// `validate_play_address` with this server's rules.
pub(crate) fn play_address_in(state: &AppState, value: &str) -> ApiResult<String> {
    validate_play_address(
        value,
        state.config.releases.allow_http_downloads,
        &own_sites(&state.config),
    )
}

/// Drops the dot a site's name may end with (`games.example.` is the same site), so an address
/// is kept, compared and matched against the bad-link lists in one form. Fails for a name that's
/// only dots.
fn drop_trailing_dot(url: &mut Url) -> Result<(), url::ParseError> {
    if let Some(url::Host::Domain(name)) = url.host()
        && name.ends_with('.')
    {
        let name = name.trim_end_matches('.').to_string();
        url.set_host(Some(&name))?;
    }
    Ok(())
}

/// Whether two play addresses are the same, exactly as they're kept (`validate_play_address`):
/// the scheme, the site (`www.` included), the port, the path, the query and the fragment all
/// count, so automatic approval (updates.rs) never takes a changed address for the one a person
/// approved. The bad-link lists compare more loosely (bad_links.rs `normalize`).
pub(crate) fn same_address(a: &str, b: &str) -> bool {
    let kept = |s: &str| {
        let mut url = Url::parse(s.trim()).ok()?;
        drop_trailing_dot(&mut url).ok()?;
        Some(url.to_string())
    };
    matches!((kept(a), kept(b)), (Some(x), Some(y)) if x == y)
}

/// What each build must be, by platform (migrations/0012_platforms.sql).
fn expected_file(platform: Platform) -> (FileKind, &'static [&'static str], &'static str) {
    match platform {
        Platform::Windows => (FileKind::Archive, &[".zip"], "a .zip of the app's files"),
        Platform::Linux => (
            FileKind::Archive,
            &[".tar.gz", ".tgz"],
            "a .tar.gz of the app's files",
        ),
        Platform::LinuxDeb => (FileKind::Installer, &[".deb"], "a .deb package"),
        Platform::LinuxRpm => (FileKind::Installer, &[".rpm"], "an .rpm package"),
        Platform::Mac => (FileKind::Archive, &[".zip"], "a .zip of the app"),
        Platform::Android => (FileKind::Apk, &[".apk"], "an .apk"),
        Platform::Any => (FileKind::Archive, &[".zip"], "a .zip of the mod's files"),
    }
}

fn platform_label(platform: Platform) -> &'static str {
    match platform {
        Platform::Windows => "Windows",
        Platform::Linux => "Linux",
        Platform::LinuxDeb => "Linux .deb",
        Platform::LinuxRpm => "Linux .rpm",
        Platform::Mac => "macOS",
        Platform::Android => "Android",
        Platform::Any => "mod",
    }
}

/// `required` is the app's platforms from its store page: a release brings exactly those builds.
fn validate_files(
    repo_url: &str,
    files: Vec<NewFile>,
    allow_http: bool,
    required: &[String],
) -> ApiResult<Vec<CheckedFile>> {
    if required.is_empty() {
        return Err(ApiError::invalid(
            "files",
            "choose your app's platforms on its store page first (Edit page)",
        ));
    }
    if files.is_empty() || files.len() > Platform::ALL.len() {
        return Err(ApiError::invalid("files", "add one build per platform"));
    }
    let mut checked: Vec<CheckedFile> = Vec::new();
    for f in files {
        let platform = Platform::parse(&f.platform)
            .ok_or_else(|| ApiError::invalid("files", "unknown platform"))?;
        if checked.iter().any(|c| c.platform == platform) {
            return Err(ApiError::invalid("files", "one build per platform"));
        }
        if !required.iter().any(|r| r == platform.as_str()) {
            return Err(ApiError::invalid(
                "files",
                format!(
                    "your store page doesn't list {}; add it there first, or leave this file out",
                    platform_label(platform)
                ),
            ));
        }
        let kind = FileKind::parse(&f.kind)
            .ok_or_else(|| ApiError::invalid("files", "unknown build type"))?;
        match (platform, kind) {
            (Platform::Android, FileKind::Apk) => {}
            (Platform::Android, _) | (_, FileKind::Apk) => {
                return Err(ApiError::invalid(
                    "files",
                    "Android builds are APKs, and APKs are only for Android",
                ));
            }
            _ => {}
        }
        let exec = match (
            kind,
            f.exec.as_deref().map(str::trim).filter(|e| !e.is_empty()),
        ) {
            // A mod has nothing to start: the game loads it.
            (FileKind::Archive, None) if platform == Platform::Any => None,
            (_, Some(_)) if platform == Platform::Any => {
                return Err(ApiError::invalid(
                    "files",
                    "a mod has no program to start; leave the program path out",
                ));
            }
            (FileKind::Archive, Some(exec)) if is_safe_relative_path(exec) => {
                Some(exec.to_string())
            }
            (FileKind::Archive, _) => {
                return Err(ApiError::invalid(
                    "files",
                    "archives need the path of the program to start, inside the archive, like bin/game",
                ));
            }
            (_, None) => None,
            (_, Some(_)) => {
                return Err(ApiError::invalid(
                    "files",
                    "only archives have a program path",
                ));
            }
        };
        if f.args.len() > 16
            || f.args
                .iter()
                .any(|a| a.len() > 200 || a.chars().any(char::is_control))
        {
            return Err(ApiError::invalid("files", "launch arguments are too long"));
        }
        let (url, file_name) = validate_file_url(repo_url, &f.url, allow_http)?;
        let (want_kind, extensions, description) = expected_file(platform);
        let lower = file_name.to_ascii_lowercase();
        if kind != want_kind || !extensions.iter().any(|e| lower.ends_with(e)) {
            return Err(ApiError::invalid(
                "files",
                format!(
                    "{file_name}: the {} build must be {description}",
                    platform_label(platform)
                ),
            ));
        }
        let mut parts = Vec::new();
        if !f.parts.is_empty() {
            if kind != FileKind::Archive || !(2..=MAX_PARTS).contains(&f.parts.len()) {
                return Err(ApiError::invalid(
                    "files",
                    format!("only archives come in parts, from 2 to {MAX_PARTS} of them"),
                ));
            }
            for (i, part) in f.parts.iter().enumerate() {
                let (part_url, part_name) = validate_file_url(repo_url, part, allow_http)?;
                if part_name != format!("{file_name}.{:03}", i + 1) {
                    return Err(ApiError::invalid(
                        "files",
                        format!(
                            "part {} of {file_name} must be named {file_name}.{:03}",
                            i + 1,
                            i + 1
                        ),
                    ));
                }
                parts.push((part_url, part_name));
            }
        }
        checked.push(CheckedFile {
            platform,
            kind,
            url,
            file_name,
            exec,
            args: f.args,
            parts,
        });
    }
    let missing: Vec<&str> = required
        .iter()
        .filter_map(|r| Platform::parse(r))
        .filter(|p| !checked.iter().any(|c| c.platform == *p))
        .map(platform_label)
        .collect();
    if !missing.is_empty() {
        return Err(ApiError::invalid(
            "files",
            format!(
                "your store page lists {} too; add {}",
                missing.join(" and "),
                if missing.len() == 1 {
                    "that build"
                } else {
                    "those builds"
                }
            ),
        ));
    }
    Ok(checked)
}

#[derive(Serialize, sqlx::FromRow)]
struct ReleaseFileInfo {
    platform: String,
    kind: String,
    url: String,
    file_name: String,
    exec: Option<String>,
    args: Vec<String>,
    size_bytes: Option<i64>,
    sha256: Option<String>,
    /// Who built it, from the maker's verified attestation.
    #[sqlx(json(nullable))]
    provenance: Option<slop_provenance::Provenance>,
    /// Why there's no verified maker signature.
    provenance_problem: Option<String>,
    /// Signed by hand with the maker's registered key: its fingerprint and name.
    maker_key: Option<serde_json::Value>,
    /// passed, note, on_hold, rejected, blocked, or skipped (scanning off).
    scan_result: Option<String>,
    scan_detail: Option<String>,
    /// The scanner's engines, findings and notable capabilities.
    scan_report: Option<serde_json::Value>,
    /// Microsoft Defender's check (Windows builds).
    defender: Option<serde_json::Value>,
    /// A reviewer cleared a finding as a false positive, and why.
    cleared_reason: Option<String>,
    /// What SLAPorSLOP is doing with this file right now: downloading, signature or scanning
    /// (release_progress.rs). Null when it isn't being checked.
    check_stage: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    check_started_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    checked_at: Option<OffsetDateTime>,
    /// waiting, checking, checked, or not_checked (the release stopped before this file).
    #[sqlx(skip)]
    check: &'static str,
}

#[derive(Serialize)]
pub(crate) struct ReleaseInfo {
    pub(crate) id: Uuid,
    version: String,
    commit_sha: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    status_reason: Option<String>,
    commit_verified: bool,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    /// maker, slop_build or github_release.
    source: String,
    /// True when SLAPorSLOP approved it automatically as an update (updates.rs).
    auto_approved: bool,
    /// Its rating (AGE-7).
    rating: Option<Rating>,
    /// Shared runtimes it needs.
    runtimes: Vec<String>,
    /// What changed, for players (Markdown).
    notes: Option<String>,
    files: Vec<ReleaseFileInfo>,
    /// A browser game's release: the address it plays at, instead of files. Everyone but the
    /// maker sees only the address Play opens (`list_releases`).
    play_address: Option<String>,
    /// What SLAPorSLOP's automatic checks of the address found (`AddressChecks`); null until
    /// they've run.
    address_checks: Option<serde_json::Value>,
    /// A browser game's release, for its maker: whether it's the one whose address Play opens
    /// (`catalog::PLAY_RELEASE`), so their progress never says Play opens an address it doesn't.
    /// Left out for downloads, and for everyone else (`list_releases`).
    #[serde(skip_serializing_if = "Option::is_none")]
    plays_now: Option<bool>,
    /// Slop Build's source checks: possible secrets and vulnerable dependencies.
    source_checks: Option<serde_json::Value>,
    /// SLAPorSLOP's own source check (source_scan.rs): licence, monetization notes, package count.
    repo_scan: Option<serde_json::Value>,
    /// Advisories for its packages (dep_watch.rs), newest first.
    advisories: Vec<Advisory>,
    /// Where it is in SLAPorSLOP's checks, file by file, with a summary for the maker.
    progress: crate::release_progress::ReleaseProgress,
}

#[derive(Serialize, sqlx::FromRow)]
pub(crate) struct Advisory {
    vuln_id: String,
    ecosystem: String,
    package: String,
    version: String,
    summary: String,
    fixed: Option<String>,
    malicious: bool,
    /// Slop Build's check already showed it at build time.
    known_at_build: bool,
    #[serde(with = "time::serde::rfc3339")]
    found_at: OffsetDateTime,
}

async fn owned_app(state: &AppState, slug: &str, user: &CurrentUser) -> ApiResult<(Uuid, String)> {
    let row: Option<(Uuid, Uuid, String)> =
        sqlx::query_as("select id, maker_id, repo_url from apps where slug = $1")
            .bind(slug)
            .fetch_optional(&state.db)
            .await?;
    match row {
        Some((id, maker, repo)) if maker == user.id => Ok((id, repo)),
        _ => Err(ApiError::NotFound),
    }
}

async fn submit_release(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(req): Json<NewRelease>,
) -> ApiResult<(StatusCode, Json<ReleaseInfo>)> {
    crate::limits::check(&state, user.id, crate::limits::Action::Release)?;
    let (app_id, repo_url) = owned_app(&state, &slug, &user).await?;
    // Checked first, so a refusal doesn't leave the release made without them.
    if let Some(notes) = &req.notes {
        check_notes(notes)?;
    }
    let commit = match req.commit_sha {
        Some(commit) => commit,
        None => site_commit(&state, app_id, &repo_url).await?,
    };
    let release = create_release(
        &state,
        app_id,
        &repo_url,
        user.id,
        &req.version,
        &commit,
        req.files,
        "maker",
        req.rating,
        req.runtimes,
        req.play_address,
    )
    .await?;
    let release = match req.notes {
        Some(notes) => {
            set_notes(&state.db, release.id, &notes).await?;
            load_release(&state, release.id).await?
        }
        None => release,
    };
    Ok((StatusCode::CREATED, Json(release)))
}

/// The commit for a release that didn't name one. Only a browser game may leave it out: nothing
/// is downloaded that says which commit its site is built from, so SLAPorSLOP takes the newest
/// on the default branch, from GitHub.
async fn site_commit(state: &AppState, app_id: Uuid, repo_url: &str) -> ApiResult<String> {
    if !catalog::plays_in_browser(&state.db, app_id).await? {
        return Err(ApiError::invalid(
            "commit_sha",
            "use the full 40-character commit SHA, in lowercase",
        ));
    }
    let Some(repo) = slop_provenance::github_repo_path(repo_url) else {
        return Err(ApiError::invalid(
            "commit_sha",
            "give the commit your site is built from: SLAPorSLOP can only look up the newest one on GitHub",
        ));
    };
    crate::media::head_commit(state, &repo).await.map_err(|p| {
        ApiError::invalid(
            "commit_sha",
            format!(
                "give the commit your site is built from: SLAPorSLOP couldn't look up the newest one ({})",
                p.0
            ),
        )
    })
}

/// Up to this many characters of release notes.
const MAX_NOTES: usize = 5000;

/// Checks release notes, before a release is made with them.
pub(crate) fn check_notes(notes: &str) -> ApiResult<()> {
    if notes.trim().chars().count() > MAX_NOTES {
        return Err(ApiError::invalid("notes", "up to 5,000 characters"));
    }
    Ok(())
}

/// Sets a release's notes: what changed, for players. Empty clears them.
pub(crate) async fn set_notes(db: &sqlx::PgPool, release_id: Uuid, notes: &str) -> ApiResult<()> {
    check_notes(notes)?;
    let notes = notes.trim();
    sqlx::query("update releases set notes = $2 where id = $1")
        .bind(release_id)
        .bind((!notes.is_empty()).then_some(notes))
        .execute(db)
        .await?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditRelease {
    notes: String,
}

/// The maker changes a release's notes (any time: they're not part of the signed manifest).
async fn edit_release(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, release_id)): Path<(String, Uuid)>,
    Json(req): Json<EditRelease>,
) -> ApiResult<Json<ReleaseInfo>> {
    let (app_id, _) = owned_app(&state, &slug, &user).await?;
    let found: bool =
        sqlx::query_scalar("select exists (select 1 from releases where id = $1 and app_id = $2)")
            .bind(release_id)
            .bind(app_id)
            .fetch_one(&state.db)
            .await?;
    if !found {
        return Err(ApiError::NotFound);
    }
    set_notes(&state.db, release_id, &req.notes).await?;
    Ok(Json(load_release(&state, release_id).await?))
}

/// The maker withdraws a release that hasn't been reviewed yet: one they didn't publish (the
/// "was submitted" notice says how), or one sent by mistake. It never reaches players, and the
/// app's next release waits for a person (updates.rs), in case it wasn't the maker who sent it.
async fn withdraw_release(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, release_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<ReleaseInfo>> {
    let (app_id, _) = owned_app(&state, &slug, &user).await?;
    let mut tx = state.db.begin().await?;
    let status: Option<String> = sqlx::query_scalar(
        "select status::text from releases where id = $1 and app_id = $2 for update",
    )
    .bind(release_id)
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;
    match status.as_deref() {
        None => return Err(ApiError::NotFound),
        Some("fetching" | "awaiting_review") => {}
        Some(_) => {
            return Err(ApiError::Conflict(
                "only a release that hasn't been reviewed yet can be withdrawn".into(),
            ));
        }
    }
    let version: String = sqlx::query_scalar(
        "update releases set status = 'withdrawn', reviewed_at = now(),
                status_reason = 'Withdrawn by the maker before review.'
         where id = $1 returning version",
    )
    .bind(release_id)
    .fetch_one(&mut *tx)
    .await?;
    crate::admin::audit(
        &mut tx,
        &user.id.to_string(),
        "release.withdraw",
        &release_id.to_string(),
        serde_json::json!({ "app": slug, "version": version, "by_agent": user.agent.is_some() }),
    )
    .await
    .map_err(ApiError::Internal)?;
    tell_maker(&mut tx, release_id, ReleaseEvent::WithdrawnByMaker).await?;
    tx.commit().await?;
    Ok(Json(load_release(&state, release_id).await?))
}

/// Checks and records a new release for fetching and review. Shared by makers' own submissions,
/// Slop Build's (slop_build.rs) and the GitHub release watcher (updates.rs), so all go through
/// the same checks. `source` is maker, slop_build or github_release. A browser game's release
/// brings `play_address` and no files, and only from its maker.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn create_release(
    state: &AppState,
    app_id: Uuid,
    repo_url: &str,
    submitted_by: Uuid,
    version: &str,
    commit_sha: &str,
    files: Vec<NewFile>,
    source: &str,
    rating: Option<Rating>,
    runtimes: Option<Vec<String>>,
    play_address: Option<String>,
) -> ApiResult<ReleaseInfo> {
    validate_version(version)?;
    validate_commit(commit_sha)?;
    let required: Vec<String> = sqlx::query_scalar("select platforms from apps where id = $1")
        .bind(app_id)
        .fetch_one(&state.db)
        .await?;
    // A browser game's release is its play address; everything else brings files.
    let in_browser = required.iter().any(|p| p == "browser");
    let play_address = if in_browser {
        if !files.is_empty() {
            return Err(ApiError::invalid(
                "files",
                "a browser game has no files: give its play address",
            ));
        }
        if source != "maker" {
            return Err(ApiError::Conflict(
                "a browser game's releases are play addresses, given on its Releases page".into(),
            ));
        }
        if runtimes.as_ref().is_some_and(|r| !r.is_empty()) {
            return Err(ApiError::invalid(
                "runtimes",
                "a browser game needs no runtimes: it runs in the player's browser",
            ));
        }
        let given = play_address.ok_or_else(|| {
            ApiError::invalid(
                "play_address",
                "give the https:// address your game plays at",
            )
        })?;
        Some(play_address_in(state, &given)?)
    } else {
        if play_address.is_some() {
            return Err(ApiError::invalid(
                "play_address",
                "only games that play in the browser have a play address",
            ));
        }
        None
    };
    // The store page's rating unless the release says otherwise; never below a branch's
    // original (branches.rs).
    let rating = match rating {
        Some(r) => Rating {
            min_age: crate::catalog::validate_min_age(r.min_age)?,
            content_labels: crate::catalog::validate_content_labels(&r.content_labels)?,
        },
        None => {
            let (min_age, content_labels): (i16, Vec<String>) =
                sqlx::query_as("select min_age, content_labels from apps where id = $1")
                    .bind(app_id)
                    .fetch_one(&state.db)
                    .await?;
            Rating {
                min_age,
                content_labels,
            }
        }
    };
    let runtimes = match runtimes {
        _ if in_browser => Vec::new(),
        Some(list) => {
            let mut out: Vec<String> = Vec::new();
            for id in list {
                if slop_manifest::runtimes::runtime(&id).is_none() {
                    let known: Vec<&str> = slop_manifest::runtimes::RUNTIMES
                        .iter()
                        .map(|r| r.id)
                        .collect();
                    return Err(ApiError::invalid(
                        "runtimes",
                        format!("{id:?} isn't on the list: {}", known.join(", ")),
                    ));
                }
                if !out.contains(&id) {
                    out.push(id);
                }
            }
            out
        }
        None => sqlx::query_scalar(
            "select runtimes from releases where app_id = $1 and status = 'approved'
             order by reviewed_at desc limit 1",
        )
        .bind(app_id)
        .fetch_optional(&state.db)
        .await?
        .unwrap_or_default(),
    };
    if let Some(floor) =
        crate::branches::rating_floor(&mut *state.db.acquire().await?, app_id).await?
    {
        floor.check_age(rating.min_age)?;
        floor.check_labels(&rating.content_labels)?;
    }
    if let Some(reason) = crate::frozen::frozen_reason(&state.db, app_id).await? {
        return Err(ApiError::Conflict(
            if crate::frozen::by_staff(&state.db, app_id).await? {
                format!(
                    "SLAPorSLOP staff froze this app, so it can't take new releases: {reason}. Fix what \
                     it says and SLAPorSLOP lifts the freeze once it's checked"
                )
            } else {
                format!(
                    "this app is frozen, so it can't take new releases: {reason}. Restore SLAPorSLOP's \
                     access to the repository, then press Check now on the app's Manage page"
                )
            },
        ));
    }
    let files = if in_browser {
        Vec::new()
    } else {
        validate_files(
            repo_url,
            files,
            state.config.releases.allow_http_downloads,
            &required,
        )?
    };

    let pending: i64 = sqlx::query_scalar(
        "select count(*) from releases where app_id = $1 and status in ('fetching', 'awaiting_review')",
    )
    .bind(app_id)
    .fetch_one(&state.db)
    .await?;
    if pending >= 3 {
        return Err(ApiError::Conflict(
            "three releases are already waiting for review".into(),
        ));
    }

    // The address leads to a public site (not on test setups), and no bad-link list names it.
    // Refusing a new address that's listed takes nothing away from anyone (SECURITY §11).
    if let Some(address) = &play_address {
        let url = Url::parse(address).map_err(|e| ApiError::Internal(e.into()))?;
        crate::repo_owner::public_host(state, &url)
            .await
            .map_err(|e| {
                ApiError::invalid(
                    "play_address",
                    match e.downcast_ref::<std::io::Error>() {
                        Some(lookup) => format!("SLAPorSLOP can't find that site ({lookup})"),
                        None => format!("{e:#}"),
                    },
                )
            })?;
        if let Some((_, list)) = crate::bad_links::find(&state.db, std::slice::from_ref(address))
            .await?
            .into_iter()
            .next()
        {
            return Err(ApiError::invalid(
                "play_address",
                format!("this address is on {list}, a list of known-bad links"),
            ));
        }
    }

    let id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    // The store page is locked until the release is in, and its platforms checked again: they
    // were read without a lock, and a store page may switch between a download and a browser
    // game only while it has no release (catalog.rs `update_app`, which locks it too).
    let now: Vec<String> =
        sqlx::query_scalar("select platforms from apps where id = $1 for update")
            .bind(app_id)
            .fetch_one(&mut *tx)
            .await?;
    if now != required {
        return Err(ApiError::Conflict(
            "the store page's platforms changed while this release was sent: send it again".into(),
        ));
    }
    sqlx::query("insert into releases (id, app_id, version, commit_sha, submitted_by, source, min_age, content_labels, runtimes, play_address) values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)")
        .bind(id)
        .bind(app_id)
        .bind(version)
        .bind(commit_sha)
        .bind(submitted_by)
        .bind(source)
        .bind(rating.min_age)
        .bind(&rating.content_labels)
        .bind(&runtimes)
        .bind(&play_address)
        .execute(&mut *tx)
        .await
        .map_err(|err| {
            if is_unique_violation(&err) {
                ApiError::invalid("version", "this version already exists")
            } else {
                err.into()
            }
        })?;
    for f in &files {
        sqlx::query(
            "insert into release_files (release_id, platform, kind, url, file_name, exec, args, parts)
             values ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(id)
        .bind(f.platform.as_str())
        .bind(f.kind.as_str())
        .bind(&f.url)
        .bind(&f.file_name)
        .bind(&f.exec)
        .bind(&f.args)
        .bind((!f.parts.is_empty()).then(|| {
            serde_json::Value::Array(
                f.parts
                    .iter()
                    .map(|(url, name)| serde_json::json!({ "url": url, "file_name": name }))
                    .collect(),
            )
        }))
        .execute(&mut *tx)
        .await?;
    }
    tell_maker(&mut tx, id, ReleaseEvent::Received { source }).await?;
    tx.commit().await?;
    state.fetch_wakeup.notify_one();
    load_release(state, id).await
}

/// What happened to a release, for its maker. `Withdrawn` is SLAPorSLOP's kill switch;
/// `WithdrawnByMaker` is the maker taking one back before review (`withdraw_release`).
pub enum ReleaseEvent<'a> {
    Received { source: &'a str },
    Approved { automatically: bool },
    Rejected { reason: &'a str },
    Failed { reason: &'a str },
    Withdrawn { reason: &'a str },
    WithdrawnByMaker,
}

/// Tells the app's maker about every release (SECURITY §5.3, §5.4): if someone else publishes in
/// their name (a stolen account, a compromised repository), they hear about it straight away,
/// under the 🔔 and by email.
pub async fn tell_maker(
    conn: &mut sqlx::PgConnection,
    release_id: Uuid,
    event: ReleaseEvent<'_>,
) -> sqlx::Result<()> {
    let row: Option<(Uuid, String, String, String, bool, bool)> = sqlx::query_as(
        "select a.maker_id, a.slug, a.title, r.version, r.play_address is not null,
                a.stage = 'coming_soon'
         from releases r join apps a on a.id = r.app_id where r.id = $1",
    )
    .bind(release_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((maker, slug, title, version, address, coming_soon)) = row else {
        return Ok(());
    };
    // Coming soon, Play opens nothing for players: an approved address waits for Beta.
    let play_opens = if coming_soon {
        "Play opens it once the game is in Beta."
    } else {
        "Play opens it now."
    };
    // Where to act on it: an unexpected release is withdrawn from the app's Manage page.
    let page = match event {
        ReleaseEvent::Received { .. } => "manage",
        _ => "releases",
    };
    let (headline, body) = match event {
        ReleaseEvent::Received { source } => (
            format!("{title} {version} was submitted"),
            format!(
                "{}. SLAPorSLOP is checking {} now. If you didn't publish this, withdraw it on the app's Manage page and secure your accounts and repository.",
                match source {
                    "slop_build" => "Slop Build sent it from your repository",
                    "github_release" => "SLAPorSLOP picked it up from a GitHub release",
                    _ => "It was submitted from your account",
                },
                if address { "its play address" } else { "it" }
            ),
        ),
        ReleaseEvent::Approved { automatically } => (
            format!("{title} {version} is approved"),
            match (address, automatically) {
                (true, true) => format!(
                    "A person approved its play address before, and it passed every check, so it \
                     was approved automatically. {play_opens}"
                ),
                (true, false) => format!("A reviewer approved its play address. {play_opens}"),
                (false, true) => "It passed every check and was approved automatically. Players get it as an update."
                    .into(),
                (false, false) => "A reviewer approved it. Players get it as an update.".into(),
            },
        ),
        ReleaseEvent::Rejected { reason } => (
            format!("{title} {version} wasn't approved"),
            reason.to_string(),
        ),
        ReleaseEvent::Failed { reason } => (
            format!("{title} {version} couldn't be checked"),
            reason.to_string(),
        ),
        ReleaseEvent::Withdrawn { reason } => (
            format!("{title} {version} was withdrawn"),
            if address {
                format!("Play no longer opens this address. Why: {reason}")
            } else {
                format!("It no longer launches for anyone. Why: {reason}")
            },
        ),
        ReleaseEvent::WithdrawnByMaker => (
            format!("{title} {version} was withdrawn"),
            "It was withdrawn from your account before review, so it never reached players. If you didn't do this, secure your accounts."
                .into(),
        ),
    };
    crate::notify::send(
        conn,
        crate::notify::Notice {
            user_id: maker,
            category: "apps",
            title: headline,
            body,
            link: format!("/publish/{slug}/{page}"),
            collapse_key: None,
        },
    )
    .await
}

pub(crate) async fn load_release(state: &AppState, id: Uuid) -> ApiResult<ReleaseInfo> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: Uuid,
        version: String,
        commit_sha: String,
        status: String,
        status_reason: Option<String>,
        commit_verified: bool,
        created_at: OffsetDateTime,
        source: String,
        auto_approved: bool,
        min_age: Option<i16>,
        content_labels: Option<Vec<String>>,
        runtimes: Vec<String>,
        notes: Option<String>,
        play_address: Option<String>,
        address_checks: Option<serde_json::Value>,
        /// The address Play opens now (`catalog::PLAY_RELEASE`).
        plays_now: bool,
        coming_soon: bool,
    }
    let row: Row = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select r.id, r.version, r.commit_sha, r.status::text as status, r.status_reason, r.commit_verified,
                r.created_at, r.source, coalesce(r.reviewed_by = 'slop', false) as auto_approved, r.min_age,
                r.content_labels, r.runtimes, r.notes, r.play_address, r.address_checks,
                r.play_address is not null and r.id is not distinct from {} as plays_now,
                a.stage = 'coming_soon' as coming_soon
         from releases r join apps a on a.id = r.app_id where r.id = $1",
        catalog::PLAY_RELEASE
    )))
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    let mut files: Vec<ReleaseFileInfo> = sqlx::query_as(
        "select platform, kind, url, file_name, exec, args, size_bytes, sha256, provenance, provenance_problem,
                scan_result, scan_detail, scan_report, defender, cleared_reason,
                maker_signature - 'signature' - 'public_key' as maker_key,
                check_stage, check_started_at, checked_at
         from release_files where release_id = $1 order by platform",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let progress = release_progress(
        state,
        (&row.status, row.status_reason.as_deref(), row.auto_approved),
        &mut files,
        row.play_address
            .as_ref()
            .map(|_| crate::release_progress::Address {
                plays_now: row.plays_now,
                coming_soon: row.coming_soon,
            }),
    )
    .await?;
    let (source_checks, repo_scan): (Option<serde_json::Value>, Option<serde_json::Value>) =
        sqlx::query_as("select source_checks, repo_scan from releases where id = $1")
            .bind(id)
            .fetch_one(&state.db)
            .await?;
    let advisories = sqlx::query_as(
        "select vuln_id, ecosystem, package, version, summary, fixed, malicious, known_at_build, found_at
         from release_advisories where release_id = $1 order by malicious desc, found_at desc, package limit 200",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    Ok(ReleaseInfo {
        notes: row.notes,
        source_checks,
        repo_scan,
        advisories,
        id: row.id,
        version: row.version,
        commit_sha: row.commit_sha,
        status: row.status,
        status_reason: row.status_reason,
        commit_verified: row.commit_verified,
        created_at: row.created_at,
        source: row.source,
        auto_approved: row.auto_approved,
        rating: row.min_age.map(|min_age| Rating {
            min_age,
            content_labels: row.content_labels.unwrap_or_default(),
        }),
        runtimes: row.runtimes,
        files,
        plays_now: row.play_address.as_ref().map(|_| row.plays_now),
        play_address: row.play_address,
        address_checks: row.address_checks,
        progress,
    })
}

/// Each file's check state, and the release's progress (release_progress.rs). The estimate is
/// looked up only while its files are being checked (a play address has none).
async fn release_progress(
    state: &AppState,
    (status, status_reason, auto_approved): (&str, Option<&str>, bool),
    files: &mut [ReleaseFileInfo],
    address: Option<crate::release_progress::Address>,
) -> ApiResult<crate::release_progress::ReleaseProgress> {
    use crate::release_progress as rp;
    let seconds_per_file = if status == "fetching" && address.is_none() {
        rp::seconds_per_file(&state.db, state.config.releases.scanner.is_on()).await?
    } else {
        None
    };
    let states: Vec<rp::FileState> = files
        .iter()
        .map(|f| rp::FileState {
            platform: &f.platform,
            file_name: &f.file_name,
            stage: f.check_stage.as_deref(),
            started_at: f.check_started_at,
            checked_at: f.checked_at,
            hashed: f.sha256.is_some(),
            scan_result: f.scan_result.as_deref(),
            scan_detail: f.scan_detail.as_deref(),
        })
        .collect();
    let checks: Vec<&'static str> = states.iter().map(|f| f.check(status)).collect();
    let progress = rp::build(&rp::Inputs {
        status,
        status_reason,
        auto_approved,
        files: &states,
        now: OffsetDateTime::now_utc(),
        seconds_per_file,
        address,
    });
    for (f, check) in files.iter_mut().zip(checks) {
        f.check = check;
    }
    Ok(progress)
}

#[derive(Deserialize)]
struct ListReleasesQuery {
    /// How many, newest first: 1 to 50 (50 when left out). Each release comes with its checks,
    /// so a maker's AI following its newest release asks for 1.
    limit: Option<i64>,
}

/// Makers see every release with its status; everyone else sees approved releases.
async fn list_releases(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Query(q): Query<ListReleasesQuery>,
) -> ApiResult<Json<Vec<ReleaseInfo>>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let (app_id, is_mine) = catalog::visible_app(&state, &slug, user.as_ref(), policy).await?;
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "select id from releases where app_id = $1 and ($2 or status = 'approved') order by created_at desc limit $3",
    )
    .bind(app_id)
    .bind(is_mine)
    .bind(q.limit.unwrap_or(50).clamp(1, 50))
    .fetch_all(&state.db)
    .await?;
    // A browser game's address shows only where Play opens it: the one release Play opens now,
    // once the game is playable (Coming soon isn't). An older address may be a lapsed or sold
    // site, and only its maker sees addresses that aren't approved.
    let plays_now: Option<Uuid> = if is_mine {
        None
    } else {
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "select {} from apps a where a.id = $1 and a.stage <> 'coming_soon'",
            catalog::PLAY_RELEASE
        )))
        .bind(app_id)
        .fetch_optional(&state.db)
        .await?
        .flatten()
    };
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let mut release = load_release(&state, id).await?;
        // Players see what was scanned and Defender's result; the findings, the reviewer's
        // reasons and the source checks (which point at possible secrets) stay with the maker.
        if !is_mine {
            release.source_checks = None;
            release.repo_scan = None;
            release.advisories.clear();
            for f in &mut release.files {
                f.cleared_reason = None;
                if let Some(report) = f.scan_report.as_mut().and_then(|r| r.as_object_mut()) {
                    report.retain(|k, _| k == "engines");
                }
            }
            if plays_now != Some(release.id) {
                release.play_address = None;
                release.address_checks = None;
            }
            release.plays_now = None;
        }
        out.push(release);
    }
    Ok(Json(out))
}

/// The approved releases players can get, the one they do get first: the release the maker
/// holds the app at, otherwise the newest approved. A browser game's approved addresses count
/// (they have no manifest); where Play goes is `catalog::PLAY_ADDRESS`, and what needs files to
/// install uses `SERVING_SIGNED`. Bind the app's ID as $1.
pub const SERVING: &str = "from releases r join apps a on a.id = r.app_id
     where r.app_id = $1 and r.status = 'approved'
       and (r.signed_manifest is not null or r.play_address is not null)
     order by coalesce(r.id = a.pinned_release_id, false) desc, r.reviewed_at desc";

/// `SERVING`, for what's installed: releases with a signed manifest only, so a browser game
/// never has one.
pub const SERVING_SIGNED: &str = "from releases r join apps a on a.id = r.app_id
     where r.app_id = $1 and r.status = 'approved' and r.signed_manifest is not null
     order by coalesce(r.id = a.pinned_release_id, false) desc, r.reviewed_at desc";

/// The signed manifest of the release players get, for installing. A browser game has none:
/// there's nothing to install. Desktop apps from before browser games offer to install every
/// game, so they're told (409) where it plays instead of a bare "not found".
async fn latest_manifest(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<Json<Signed>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let (app_id, _) = catalog::visible_app(&state, &slug, user.as_ref(), policy).await?;
    if catalog::plays_in_browser(&state.db, app_id).await? {
        return Err(ApiError::Conflict(
            "this game plays in your browser: open its store page on the website and press Play, \
             or update the SLAPorSLOP app"
                .into(),
        ));
    }
    let manifest: Option<sqlx::types::Json<Signed>> = sqlx::query_scalar(sqlx::AssertSqlSafe(
        format!("select r.signed_manifest {SERVING_SIGNED} limit 1"),
    ))
    .bind(app_id)
    .fetch_optional(&state.db)
    .await?;
    manifest.map(|m| Json(m.0)).ok_or(ApiError::NotFound)
}

// ---------------------------------------------------------------------------------------------
// Downloads from private repositories (ARCHITECTURE §6.5)

/// `(owner/repo, tag, file name)` of a GitHub release download URL.
fn github_release_file(url: &str) -> Option<(String, String, String)> {
    let rest = url.strip_prefix("https://github.com/")?;
    let mut parts = rest.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    if parts.next()? != "releases" || parts.next()? != "download" {
        return None;
    }
    let tag = percent_encoding::percent_decode_str(parts.next()?)
        .decode_utf8()
        .ok()?;
    let name = percent_encoding::percent_decode_str(parts.next()?)
        .decode_utf8()
        .ok()?;
    parts.next().is_none().then(|| {
        (
            format!("{owner}/{repo}"),
            tag.into_owned(),
            name.into_owned(),
        )
    })
}

/// Where to download a release file from. For a GitHub release with a `GITHUB_TOKEN` configured,
/// a short-lived signed link from GitHub's API, which also works for private repositories;
/// otherwise the file's own URL. SLAPorSLOP never relays the file itself.
pub async fn download_link(state: &AppState, url: &str) -> anyhow::Result<String> {
    let cfg = &state.config.releases;
    let (true, Some((repo, tag, name))) = (
        cfg.github_token.is_some() || cfg.github_app.is_some(),
        github_release_file(url),
    ) else {
        return Ok(url.to_string());
    };
    // A public release's own link works for everyone and costs no API request; only a private
    // repository's needs the API. (state.http doesn't follow the redirect to the file.)
    if let Ok(res) = state
        .http
        .head(url)
        .header("user-agent", "slop-api")
        .send()
        .await
        && (res.status().is_success() || res.status().is_redirection())
    {
        return Ok(url.to_string());
    }
    #[derive(Deserialize)]
    struct Asset {
        name: String,
        url: String,
    }
    #[derive(Deserialize)]
    struct Release {
        assets: Vec<Asset>,
    }
    let api = &state.config.releases.github_api_base;
    let tag_path = percent_encoding::utf8_percent_encode(&tag, percent_encoding::NON_ALPHANUMERIC);
    let release: Release = github_get(
        state,
        api.join(&format!("repos/{repo}/releases/tags/{tag_path}"))?,
    )
    .await
    .send()
    .await?
    .error_for_status()
    .context("finding the release on GitHub")?
    .json()
    .await?;
    let asset = release
        .assets
        .into_iter()
        .find(|a| a.name == name)
        .ok_or_else(|| anyhow!("the release has no file named {name}"))?;
    // The asset API answers with a redirect to a signed link that expires within minutes. Ask
    // without following it (state.http doesn't follow redirects).
    let res = github_request(
        state,
        &state.http,
        Url::parse(&asset.url)?,
        "application/octet-stream",
    )
    .await
    .send()
    .await?;
    if !res.status().is_redirection() {
        bail!(
            "GitHub didn't give a download link (HTTP {})",
            res.status().as_u16()
        );
    }
    let location = res
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| anyhow!("GitHub's redirect had no location"))?;
    Ok(location.to_string())
}

#[derive(Serialize)]
struct DownloadLink {
    url: String,
}

/// A download link for one file of an approved release, for the desktop app when the file's own
/// URL isn't public. The app still checks the file against the signed manifest.
#[derive(Deserialize)]
struct LinkQuery {
    /// For a file in parts: which one, from 1.
    part: Option<usize>,
}

async fn file_download_link(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Path((slug, release_id, platform)): Path<(String, Uuid, String)>,
    Query(q): Query<LinkQuery>,
) -> ApiResult<Json<DownloadLink>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let (app_id, _) = catalog::visible_app(&state, &slug, user.as_ref(), policy).await?;
    let url: Option<Option<String>> = sqlx::query_scalar(
        "select case when $4::int is null then f.url else f.parts -> ($4::int - 1) ->> 'url' end
         from release_files f join releases r on r.id = f.release_id
         where r.id = $1 and r.app_id = $2 and r.status = 'approved' and f.platform = $3",
    )
    .bind(release_id)
    .bind(app_id)
    .bind(&platform)
    .bind(q.part.map(|p| p.clamp(1, MAX_PARTS) as i32))
    .fetch_optional(&state.db)
    .await?;
    let url = url.flatten().ok_or(ApiError::NotFound)?;
    // A private repository outside GitHub: the host's short-lived link, asked for with the
    // maker's token (repo_tokens.rs).
    if let Some(auth) = crate::repo_tokens::auth_for(&state, app_id, &url)
        .await
        .map_err(ApiError::Internal)?
    {
        let url = crate::repo_tokens::short_lived_link(&state, &url, &auth)
            .await
            .map_err(|e| ApiError::Conflict(format!("{e:#}")))?;
        return Ok(Json(DownloadLink { url }));
    }
    let url = download_link(&state, &url)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(DownloadLink { url }))
}

/// The current revocation list, freshly signed (SECURITY §5.5). Only releases SLAPorSLOP signed
/// are on it: a withdrawn play address was never installed, so there's nothing for a client to
/// stop.
async fn revocations(State(state): State<AppState>) -> ApiResult<Json<Signed>> {
    let key = state
        .config
        .releases
        .signing_key
        .as_ref()
        .ok_or(ApiError::Unavailable("release signing isn't set up"))?;
    let rows: Vec<(Uuid, String, Option<String>, OffsetDateTime)> = sqlx::query_as(
        "select r.id, a.slug, r.status_reason, r.revoked_at from releases r join apps a on a.id = r.app_id
         where r.status = 'revoked' and r.signed_manifest is not null order by r.revoked_at",
    )
    .fetch_all(&state.db)
    .await?;
    let now = OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .expect("valid");
    let list = RevocationList {
        schema: REVOCATIONS_SCHEMA,
        issued_at: now.format(&Rfc3339).expect("formats"),
        expires_at: (now + REVOCATION_LIST_LIFETIME)
            .format(&Rfc3339)
            .expect("formats"),
        revoked: rows
            .into_iter()
            .map(|(id, slug, reason, at)| Revoked {
                release_id: id.to_string(),
                app_slug: slug,
                reason: reason.unwrap_or_default(),
                revoked_at: at.format(&Rfc3339).expect("formats"),
            })
            .collect(),
    };
    Ok(Json(slop_manifest::sign(
        &list,
        Purpose::RevocationList,
        key,
    )))
}

/// How long a file verification is valid.
pub const VERIFICATION_LIFETIME: Duration = Duration::hours(24);

/// "Does SLAPorSLOP know this file?" (docs/standard/file-verification.md), signed, for anyone: other
/// stores showing "verified by SLAPorSLOP", or players checking a download. Only public apps are
/// named, as a viewer with no account would see them; revoked releases of public apps are named
/// even once the app has left the store, since that's the answer that matters most. Private
/// apps are never disclosed, so an unknown file and a private one look the same.
async fn verify_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(sha256): Path<String>,
) -> ApiResult<Json<Signed>> {
    let key = state
        .config
        .releases
        .signing_key
        .as_ref()
        .ok_or(ApiError::Unavailable("release signing isn't set up"))?;
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError::invalid(
            "sha256",
            "a SHA-256 is 64 hex characters",
        ));
    }
    let sha256 = sha256.to_ascii_lowercase();
    let policy = ViewerPolicy::resolve(&state.db, &state.config, None, &headers).await?;
    let mut qb = sqlx::QueryBuilder::new(
        "select a.slug, a.title, (r.signed_manifest->>'payload')::jsonb->>'maker', r.id, r.version,
                f.platform, r.status::text, (r.signed_manifest->>'payload')::jsonb->>'approved_at',
                r.revoked_at,
                -- A re-scan of the same file counts; changed and unreachable are about the
                -- maker's page, not this file.
                case when f.rescan_result in ('passed', 'note', 'on_hold') then f.rescan_result
                     else f.scan_result end,
                case when f.defender is null then null
                     when jsonb_array_length(coalesce(f.defender->'detections', '[]')) = 0 then 'clean'
                     else 'detections' end
         from release_files f join releases r on r.id = f.release_id join apps a on a.id = r.app_id
         where f.sha256 = ",
    );
    qb.push_bind(&sha256).push(
        " and r.signed_manifest is not null
          and (r.status = 'revoked' and a.visibility = 'public' or r.status = 'approved' and ",
    );
    catalog::push_store_visible(&mut qb, state.config.catalog, policy);
    qb.push(") order by r.reviewed_at desc limit 20");
    type Row = (
        String,
        String,
        Option<String>,
        Uuid,
        String,
        String,
        String,
        Option<String>,
        Option<OffsetDateTime>,
        Option<String>,
        Option<String>,
    );
    let rows: Vec<Row> = qb.build_query_as().fetch_all(&state.db).await?;
    let site = state.config.public_url.as_str().trim_end_matches('/');
    let matches = rows
        .into_iter()
        .filter_map(
            |(
                slug,
                title,
                maker,
                id,
                version,
                platform,
                status,
                approved,
                revoked,
                scan,
                defender,
            )| {
                Some(FileMatch {
                    page_url: format!("{site}/app/{slug}"),
                    app_slug: slug,
                    app_title: title,
                    maker: maker.unwrap_or_default(),
                    release_id: id.to_string(),
                    version,
                    platform: Platform::parse(&platform)?,
                    status: if status == "revoked" {
                        MatchStatus::Revoked
                    } else {
                        MatchStatus::Approved
                    },
                    approved_at: approved.unwrap_or_default(),
                    revoked_at: revoked.and_then(|t| t.format(&Rfc3339).ok()),
                    scan: match scan.as_deref() {
                        None | Some("skipped") => "not_scanned".into(),
                        Some(s) => s.into(),
                    },
                    defender,
                })
            },
        )
        .collect();
    let now = OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .expect("valid");
    let answer = FileVerification {
        schema: VERIFICATION_SCHEMA,
        sha256,
        issued_at: now.format(&Rfc3339).expect("formats"),
        expires_at: (now + VERIFICATION_LIFETIME)
            .format(&Rfc3339)
            .expect("formats"),
        matches,
    };
    Ok(Json(slop_manifest::sign(
        &answer,
        Purpose::FileVerification,
        key,
    )))
}

#[derive(Serialize)]
struct ServerInfo {
    /// Whether uploads are scanned. Off on test servers; the UI must not claim otherwise.
    scanning: bool,
    maker_signatures_required: bool,
    /// The simulation sandbox: everything here is made up (sim.rs).
    simulation: bool,
}

/// What this server checks, so the UI only claims what's true (LEGAL-GUIDANCE §5.5).
async fn server_info(State(state): State<AppState>) -> Json<ServerInfo> {
    Json(ServerInfo {
        scanning: state.config.releases.scanner.is_on(),
        maker_signatures_required: state.config.releases.require_maker_signature,
        simulation: state.config.simulation,
    })
}

#[derive(Serialize)]
struct PublicKey {
    key_id: String,
    public_key: String,
}

/// SLAPorSLOP's current public signing key. Clients pin keys at build time; this is for transparency.
async fn keys(State(state): State<AppState>) -> Json<Vec<PublicKey>> {
    Json(
        state
            .config
            .releases
            .signing_key
            .iter()
            .map(|k| PublicKey {
                key_id: slop_manifest::key_id(&k.verifying_key()),
                public_key: slop_manifest::verifying_key_to_base64(&k.verifying_key()),
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------------------------
// Fetching (the semi-automatic first step of scanning)

/// Runs forever: picks up releases waiting to be fetched and hashes their files.
pub async fn run_fetcher(state: AppState) {
    loop {
        match fetch_next(&state).await {
            Ok(true) => continue,
            Ok(false) => {}
            Err(err) => tracing::error!(error = ?err, "release fetcher"),
        }
        tokio::select! {
            _ = state.fetch_wakeup.notified() => {}
            _ = tokio::time::sleep(StdDuration::from_secs(30)) => {}
        }
    }
}

/// Fetches one release, if one is waiting. Returns whether it did any work. Play addresses go
/// first: their checks take seconds, and shouldn't wait behind files that take minutes each. But
/// never two in a row while a file release waits, so a run of addresses can't hold files up.
pub async fn fetch_next(state: &AppState) -> anyhow::Result<bool> {
    let files_first = state.fetched_address_last.load(Ordering::Relaxed);
    let claimed: Option<(Uuid, Uuid, String, String, Option<String>)> = sqlx::query_as(
        "update releases r set fetch_started_at = now()
         from apps a
         where a.id = r.app_id and r.id = (
             select id from releases
             where status = 'fetching'
               and (fetch_started_at is null
                    or fetch_started_at < now() - case when play_address is null then $1 else $2 end)
             order by case when $3 then play_address is not null else play_address is null end,
                      created_at
             limit 1 for update skip locked)
         returning r.id, r.app_id, r.commit_sha, a.repo_url, r.play_address",
    )
    .bind(FETCH_LEASE)
    .bind(ADDRESS_FETCH_LEASE)
    .bind(files_first)
    .fetch_optional(&state.db)
    .await?;
    let Some((id, app_id, commit, repo_url, play_address)) = claimed else {
        return Ok(false);
    };
    state
        .fetched_address_last
        .store(play_address.is_some(), Ordering::Relaxed);
    // A fetch that died part-way starts over: every file is checked again.
    sqlx::query(
        "update release_files set check_stage = null, check_started_at = null, checked_at = null
         where release_id = $1",
    )
    .bind(id)
    .execute(&state.db)
    .await?;

    let checked = match &play_address {
        Some(address) => check_address_release(state, id, &commit, &repo_url, address).await,
        None => fetch_release(state, id, &commit, &repo_url).await,
    };
    match checked {
        Ok(commit_verified) => {
            let updated = sqlx::query(
                "update releases set status = 'awaiting_review', commit_verified = $2
                 where id = $1 and status = 'fetching'",
            )
            .bind(id)
            .bind(commit_verified)
            .execute(&state.db)
            .await?
            .rows_affected();
            // Withdrawn by its maker while it was fetched: nothing more to check.
            if updated == 0 {
                return Ok(true);
            }
            tracing::info!(release = %id, "release fetched; awaiting review");
            // Pictures that changed with this release wait for their own review.
            crate::media::refresh_for_release(state, app_id, &repo_url, &commit).await;
            // Licence, monetization and packages, before an update can be approved on its own.
            crate::source_scan::check_release(state, id).await?;
            crate::updates::try_auto_approve(state, id).await?;
        }
        Err(err) => {
            tracing::warn!(release = %id, error = %err, "release fetch failed");
            let reason = format!("{err:#}");
            let mut tx = state.db.begin().await?;
            let updated = sqlx::query("update releases set status = 'failed', status_reason = $2 where id = $1 and status = 'fetching'")
                .bind(id)
                .bind(&reason)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            if updated == 1 {
                tell_maker(&mut tx, id, ReleaseEvent::Failed { reason: &reason }).await?;
            }
            tx.commit().await?;
        }
    }
    Ok(true)
}

async fn fetch_release(
    state: &AppState,
    id: Uuid,
    commit: &str,
    repo_url: &str,
) -> anyhow::Result<bool> {
    let commit_verified = check_commit(state, repo_url, commit).await?;
    let app_id: Uuid = sqlx::query_scalar("select app_id from releases where id = $1")
        .bind(id)
        .fetch_one(&state.db)
        .await?;
    let files: Vec<(String, String, String, Option<serde_json::Value>)> = sqlx::query_as(
        "select platform, url, file_name, parts from release_files where release_id = $1 order by platform",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let scanner = state.config.releases.scanner;
    for (platform, url, file_name, parts) in files {
        file_stage(state, id, &platform, "downloading").await?;
        let part_urls: Vec<String> = parts
            .as_ref()
            .and_then(|p| p.as_array())
            .into_iter()
            .flatten()
            .filter_map(|p| p["url"].as_str().map(String::from))
            .collect();
        // A private repository outside GitHub: read with the maker's token (repo_tokens.rs).
        let auth = crate::repo_tokens::auth_for(state, app_id, &url).await?;
        // With a scanner, the file is kept on disk until it's scanned, then deleted.
        let local = scanner
            .is_on()
            .then(|| std::env::temp_dir().join(format!("slop-release-{}", Uuid::now_v7())));
        let mut part_digests = Vec::new();
        let downloaded = if part_urls.is_empty() {
            let source = if auth.is_some() {
                url.clone()
            } else {
                download_link(state, &url)
                    .await
                    .with_context(|| format!("{platform} build"))?
            };
            download_full(
                &state.fetch_http,
                &source,
                auth.as_deref(),
                state.config.releases.max_file_bytes,
                local.as_deref(),
            )
            .await
        } else {
            download_parts(state, &part_urls, auth.as_deref(), local.as_deref())
                .await
                .map(|(joined, each)| {
                    part_digests = each;
                    joined
                })
        }
        .with_context(|| format!("{platform} build"));
        let file = match downloaded {
            Ok(d) => d,
            Err(e) => {
                if let Some(p) = &local {
                    let _ = tokio::fs::remove_file(p).await;
                }
                return Err(e);
            }
        };
        let (size, sha256) = (file.size, file.sha256.clone());
        file_stage(state, id, &platform, "signature").await?;
        // In parts: each part carries its own attestation (Slop Build attests every file it
        // uploads); the joined file has none of its own.
        let mut parts_out = None;
        let (attestation, provenance, mut problem) = if part_urls.is_empty() {
            match find_attestation(state, repo_url, commit, &sha256).await {
                Ok((bundle, provenance)) => {
                    (Some(bundle), Some(sqlx::types::Json(provenance)), None)
                }
                Err(problem) => (None, None, Some(problem)),
            }
        } else {
            let mut out = Vec::new();
            let mut first = None;
            let mut missing = None;
            for (i, ((part_url, (part_size, part_sha)), name)) in part_urls
                .iter()
                .zip(&part_digests)
                .zip((1..).map(|n| format!("{file_name}.{n:03}")))
                .enumerate()
            {
                let att = match find_attestation(state, repo_url, commit, part_sha).await {
                    Ok((bundle, provenance)) => {
                        first.get_or_insert(provenance);
                        Some(bundle)
                    }
                    Err(p) => {
                        missing.get_or_insert(format!("part {}: {p}", i + 1));
                        None
                    }
                };
                out.push(serde_json::json!({
                    "url": part_url, "file_name": name, "size": part_size, "sha256": part_sha, "attestation": att,
                }));
            }
            parts_out = Some(serde_json::Value::Array(out));
            match missing {
                None => (None, first.map(sqlx::types::Json), None),
                Some(p) => (None, None, Some(p)),
            }
        };
        let signed_in_parts = !part_urls.is_empty() && problem.is_none();
        // No attestation: maybe the maker signed it by hand with their registered key.
        let mut maker_sig = None;
        if attestation.is_none() && !signed_in_parts {
            match maker_signature(state, app_id, &url, auth.as_deref(), &file).await? {
                Ok(sig) => {
                    maker_sig = Some(sig);
                    problem = None;
                }
                Err(why) => {
                    // Keep the attestation problem for Git hosts that have attestations.
                    problem = Some(match problem {
                        Some(p) => format!("{p}; {why}"),
                        None => why,
                    });
                }
            }
        }
        if scanner.is_on() {
            file_stage(state, id, &platform, "scanning").await?;
        }
        let verdict = match &local {
            Some(path) => {
                let v = scanner
                    .scan(&crate::scan::Target {
                        path,
                        file_name: &file_name,
                        sha256: &sha256,
                    })
                    .await;
                let _ = tokio::fs::remove_file(path).await;
                v
            }
            None => {
                scanner
                    .scan(&crate::scan::Target {
                        path: std::path::Path::new(""),
                        file_name: &file_name,
                        sha256: &sha256,
                    })
                    .await
            }
        };
        sqlx::query(
            "update release_files
             set size_bytes = $3, sha256 = $4, attestation = $5, provenance = $6, provenance_problem = $7,
                 scan_result = $8, scan_detail = $9, scan_report = $10, maker_signature = $11,
                 parts = coalesce($12, parts), check_stage = null, checked_at = now()
             where release_id = $1 and platform = $2",
        )
        .bind(id)
        .bind(&platform)
        .bind(size as i64)
        .bind(&sha256)
        .bind(attestation)
        .bind(provenance)
        .bind(problem)
        .bind(verdict.result.as_str())
        .bind(verdict.detail)
        .bind(verdict.report.map(sqlx::types::Json))
        .bind(maker_sig)
        .bind(parts_out)
        .execute(&state.db)
        .await?;
        // A Defender check of exactly this file, from Slop Build or Slop Check.
        sqlx::query(
            "update release_files f set defender = c.report
             from releases r, defender_checks c
             where f.release_id = $1 and f.platform = $2 and f.platform = 'windows'
               and r.id = f.release_id and c.app_id = r.app_id and c.sha256 = f.sha256",
        )
        .bind(id)
        .bind(&platform)
        .execute(&state.db)
        .await?;
    }
    Ok(commit_verified)
}

/// A browser game's release: its commit, then its play address (`check_play_address`). What the
/// checks found is kept for the maker and the reviewer, passed or not.
async fn check_address_release(
    state: &AppState,
    id: Uuid,
    commit: &str,
    repo_url: &str,
    address: &str,
) -> anyhow::Result<bool> {
    let commit_verified = check_commit(state, repo_url, commit).await?;
    let checks = check_play_address(state, address).await?;
    sqlx::query("update releases set address_checks = $2 where id = $1")
        .bind(id)
        .bind(sqlx::types::Json(&checks))
        .execute(&state.db)
        .await?;
    if !checks.ok {
        bail!(
            "{}",
            checks
                .problem
                .as_deref()
                .unwrap_or("the play address didn't pass its checks")
        );
    }
    Ok(commit_verified)
}

/// Redirects a play page may take before it loads.
const MAX_PAGE_REDIRECTS: u32 = 5;
/// How much of a play page is read to know it loads. None of it is kept.
const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;

/// What SLAPorSLOP's automatic checks of a play address found (`releases.address_checks`), for
/// the maker, the reviewer, and approval (admin.rs needs `ok`). It says what was checked, never
/// that the site is safe: what the site serves isn't checked.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct AddressChecks {
    /// The address, and every address it redirected to, is https.
    pub https: bool,
    /// Its site, and every one it redirected to, has only public addresses. Null where that isn't
    /// known, and `public_host_skipped` says why.
    pub public_host: Option<bool>,
    /// Why `public_host` is null: `test_setup` (test setups serve their pages from this machine,
    /// so names aren't looked up) or `lookup_failed` (a site's name couldn't be looked up; the
    /// check then fails). Null otherwise.
    #[serde(default)]
    pub public_host_skipped: Option<String>,
    pub bad_links: BadLinkCheck,
    /// The HTTP status of the page it ended at; null when nothing answered.
    pub status: Option<u16>,
    /// Where it ended up, after redirects; null when nothing answered.
    pub final_address: Option<String>,
    /// How many redirects it took.
    pub redirects: u32,
    /// Every address on the way: the address itself, then each one it redirected to (the last is
    /// `final_address`). Approval and the hourly checks match them all against the bad-link
    /// lists again.
    #[serde(default)]
    pub hops: Vec<String>,
    /// The kind of page it ended at (text/html), as the site said.
    pub content_type: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub checked_at: OffsetDateTime,
    /// Every check passed.
    pub ok: bool,
    /// What didn't pass, in words for the maker; null when `ok`.
    pub problem: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BadLinkCheck {
    /// The list that names the address, or one it redirected to.
    pub listed_by: Option<String>,
    /// Whether this server has any bad-link list loaded (bad_links.rs). Without one, nothing can
    /// be said about the lists: "no bad-link lists are loaded on this server", never "not on any
    /// list".
    pub lists_loaded: bool,
}

/// Checks a play address the way Play will open it: the page must load (HTTP 2xx) as a web page
/// (text/html or application/xhtml+xml), not a download, over https, on public sites only, not
/// SLAPorSLOP's own, and neither it nor anywhere it redirects to may be on a bad-link list. All
/// of it within `AppState.page_check_deadline`, so one slow site can't hold the fetcher up. The
/// page itself isn't kept. Problems with the page are in the result; errors are SLAPorSLOP's.
pub(crate) async fn check_play_address(
    state: &AppState,
    address: &str,
) -> anyhow::Result<AddressChecks> {
    let allow_http = state.config.releases.allow_http_downloads;
    let mut checks = AddressChecks {
        https: true,
        public_host: (!allow_http).then_some(true),
        public_host_skipped: allow_http.then(|| "test_setup".to_string()),
        bad_links: BadLinkCheck {
            listed_by: None,
            lists_loaded: false,
        },
        status: None,
        final_address: None,
        redirects: 0,
        hops: Vec::new(),
        content_type: None,
        checked_at: OffsetDateTime::now_utc().replace_nanosecond(0)?,
        ok: false,
        problem: None,
    };
    let mut visited = vec![address.to_string()];
    let page = tokio::time::timeout(
        state.page_check_deadline,
        load_page(state, address, &mut checks, &mut visited),
    )
    .await
    .unwrap_or_else(|_| Err("the page didn't load in time".into()));
    let lists_loaded: bool = sqlx::query_scalar("select exists (select 1 from bad_links)")
        .fetch_one(&state.db)
        .await?;
    let listed = crate::bad_links::find(&state.db, &visited).await?;
    let listed_problem = listed.first().map(|(on, list)| {
        format!(
            "{} is on {list}, a list of known-bad links",
            if on == address {
                "this address".to_string()
            } else {
                format!("the address it leads to ({on})")
            }
        )
    });
    checks.bad_links = BadLinkCheck {
        listed_by: listed.into_iter().next().map(|(_, list)| list),
        lists_loaded,
    };
    checks.hops = visited;
    checks.problem = listed_problem.or(page.err());
    checks.ok = checks.problem.is_none();
    Ok(checks)
}

/// Loads the page at `address`, following redirects itself (so each one is checked and counted
/// as the address was when it was given: https, a site's name rather than an IP address, only
/// public addresses, not SLAPorSLOP's own site), and reads a little of it. Records what it found
/// in `checks` and every address on the way in `visited`; says what's wrong.
async fn load_page(
    state: &AppState,
    address: &str,
    checks: &mut AddressChecks,
    visited: &mut Vec<String>,
) -> Result<(), String> {
    use reqwest::header;
    let allow_http = state.config.releases.allow_http_downloads;
    let own = own_sites(&state.config);
    let mut url = Url::parse(address).map_err(|_| "the address isn't valid".to_string())?;
    loop {
        // Where it's got to, and (once it answers) what that page said.
        checks.final_address = Some(url.to_string());
        checks.status = None;
        if url.scheme() != "https" {
            checks.https = false;
            if !allow_http {
                return Err("it leads to a page that isn't https".into());
            }
        }
        if is_own_site(&url, &own, allow_http) {
            return Err("it leads to SLAPorSLOP's own site".into());
        }
        if !allow_http {
            let ip: Option<std::net::IpAddr> = match url.host() {
                Some(url::Host::Ipv4(ip)) => Some(ip.into()),
                Some(url::Host::Ipv6(ip)) => Some(ip.into()),
                _ => None,
            };
            if let Some(ip) = ip {
                if crate::repo_owner::private_ip(ip) {
                    checks.public_host = Some(false);
                    return Err("it leads to a private network".into());
                }
                return Err("it leads to an IP address instead of a site's name".into());
            }
            if let Err(e) = crate::repo_owner::public_host(state, &url).await {
                return Err(match e.downcast_ref::<std::io::Error>() {
                    Some(lookup) => {
                        checks.public_host = None;
                        checks.public_host_skipped = Some("lookup_failed".into());
                        format!(
                            "SLAPorSLOP can't find the site {} ({lookup})",
                            url.host_str().unwrap_or("")
                        )
                    }
                    None => {
                        checks.public_host = Some(false);
                        format!("it leads to a private network ({e:#})")
                    }
                });
            }
        }
        // The page is the maker's: it's read as text, nothing of it is run or kept.
        let sent = state
            .page_http
            .get(url.clone())
            .header(header::USER_AGENT, "slop-api")
            .header(header::ACCEPT, "text/html,application/xhtml+xml")
            .send()
            .await;
        let mut res = sent.map_err(|e| page_error(&e))?;
        let status = res.status();
        checks.status = Some(status.as_u16());
        if status.is_redirection() {
            if checks.redirects >= MAX_PAGE_REDIRECTS {
                return Err(format!("it redirects more than {MAX_PAGE_REDIRECTS} times"));
            }
            let to = res
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or("the site redirects without saying where")?;
            url = url
                .join(to)
                .map_err(|_| "the site redirects to an address that isn't valid".to_string())?;
            checks.redirects += 1;
            visited.push(url.to_string());
            continue;
        }
        if !status.is_success() {
            return Err(format!(
                "the page didn't load: the site answered HTTP {}",
                status.as_u16()
            ));
        }
        let text = |name: header::HeaderName| {
            res.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.trim().to_ascii_lowercase())
        };
        let media = text(header::CONTENT_TYPE).map(|t| {
            t.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .chars()
                .take(100)
                .collect::<String>()
        });
        let attachment =
            text(header::CONTENT_DISPOSITION).is_some_and(|d| d.starts_with("attachment"));
        checks.content_type = media.clone();
        if attachment {
            return Err("it downloads a file instead of opening a page".into());
        }
        match media.as_deref() {
            Some("text/html" | "application/xhtml+xml") => {}
            Some(other) => {
                return Err(format!(
                    "it doesn't open a web page: the site sends {other}"
                ));
            }
            None => {
                return Err(
                    "it doesn't open a web page: the site doesn't say what it sends".into(),
                );
            }
        }
        // A little of the page, to know it loads.
        let mut read = 0;
        while let Some(chunk) = res.chunk().await.map_err(|e| page_error(&e))? {
            read += chunk.len();
            if read >= MAX_PAGE_BYTES {
                break;
            }
        }
        return Ok(());
    }
}

/// Why a play page didn't load, in words for the maker.
fn page_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        return "the page didn't load in time".into();
    }
    let mut why = String::new();
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        why = s.to_string();
        source = s.source();
    }
    if why.is_empty() {
        "the page didn't load".into()
    } else {
        format!("the page didn't load: {why}")
    }
}

/// Whether a browser game's release brings another address than the one Play opens now (or
/// Play opens none yet). A new address always waits for a person (updates.rs).
pub(crate) async fn new_play_address(db: &sqlx::PgPool, release_id: Uuid) -> sqlx::Result<bool> {
    let row: Option<(Option<String>, Option<String>)> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select r.play_address, {} from releases r join apps a on a.id = r.app_id where r.id = $1",
            catalog::PLAY_ADDRESS
        )))
        .bind(release_id)
        .fetch_optional(db)
        .await?;
    Ok(match row {
        Some((Some(this), now)) => !now.is_some_and(|now| same_address(&this, &now)),
        _ => false,
    })
}

/// Where Play goes, for a person reviewing a browser game's release (review.rs, admin_web.rs), so
/// they never take a resubmitted address for a first one, or miss that approving an older one
/// moves Play back to it. All null for a download.
#[derive(Serialize)]
pub(crate) struct AddressContext {
    /// The address Play opens now (`catalog::PLAY_ADDRESS`), or null.
    pub current_play_address: Option<String>,
    /// The address Play opened until SLAPorSLOP withdrew it, when that's why Play opens none now.
    /// With `current_play_address` null too, the game never had an approved address.
    pub withdrawn_play_address: Option<String>,
    /// An approved address release sent after this one: approving this one makes Play open this
    /// older address instead (Play opens the newest approved).
    pub newer_approved: Option<NewerApproved>,
}

#[derive(Serialize, sqlx::FromRow)]
pub(crate) struct NewerApproved {
    pub version: String,
    pub play_address: String,
    /// The same, in a sentence for the reviewer.
    #[sqlx(skip)]
    pub note: String,
}

/// Release `release_id`'s `AddressContext`.
pub(crate) async fn address_context(
    db: &sqlx::PgPool,
    release_id: Uuid,
) -> sqlx::Result<AddressContext> {
    let (current_play_address, withdrawn_play_address): (Option<String>, Option<String>) =
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select {}, {} from releases r join apps a on a.id = r.app_id where r.id = $1",
            catalog::PLAY_ADDRESS,
            catalog::WITHDRAWN_PLAY_ADDRESS
        )))
        .bind(release_id)
        .fetch_one(db)
        .await?;
    let newer_approved: Option<NewerApproved> = sqlx::query_as(
        "select n.version, n.play_address from releases r join releases n on n.app_id = r.app_id
         where r.id = $1 and r.play_address is not null and n.play_address is not null
           and n.status = 'approved' and n.created_at > r.created_at
         order by n.created_at desc limit 1",
    )
    .bind(release_id)
    .fetch_optional(db)
    .await?;
    Ok(AddressContext {
        current_play_address,
        withdrawn_play_address,
        newer_approved: newer_approved.map(|n| NewerApproved {
            note: format!(
                "Version {} ({}) is approved and newer: approving this makes Play open this older address",
                n.version, n.play_address
            ),
            ..n
        }),
    })
}

/// Records what SLAPorSLOP is doing with one file now, for its maker's progress view
/// (release_progress.rs). Starting a file also renews the release's lease, so a release with
/// several slow scans isn't taken for a dead fetch and started again.
async fn file_stage(state: &AppState, id: Uuid, platform: &str, stage: &str) -> sqlx::Result<()> {
    debug_assert!(crate::release_progress::STAGES.contains(&stage));
    sqlx::query(
        "update release_files
         set check_stage = $3,
             check_started_at = case when $3 = 'downloading' then now() else check_started_at end,
             checked_at = null
         where release_id = $1 and platform = $2",
    )
    .bind(id)
    .bind(platform)
    .bind(stage)
    .execute(&state.db)
    .await?;
    sqlx::query(
        "update releases set fetch_started_at = now() where id = $1 and status = 'fetching'",
    )
    .bind(id)
    .execute(&state.db)
    .await?;
    Ok(())
}

pub(crate) async fn github_get(state: &AppState, url: Url) -> reqwest::RequestBuilder {
    github_request(state, &state.fetch_http, url, "application/vnd.github+json").await
}

/// A GitHub API GET on a chosen client, with the repository's GitHub App token or SLAPorSLOP's
/// shared token (github_app::auth_for).
async fn github_request(
    state: &AppState,
    client: &reqwest::Client,
    url: Url,
    accept: &str,
) -> reqwest::RequestBuilder {
    let auth = crate::github_app::auth_for(state, &url).await;
    let req = client
        .get(url)
        .header("accept", accept)
        .header("user-agent", "slop-api");
    match auth {
        Some((token, _)) => req.bearer_auth(token),
        None => req,
    }
}

/// Finds and verifies the maker's GitHub artifact attestation for a file (SECURITY §5.3).
/// Returns the attestation bundle and what it proves, or a sentence saying what's missing.
pub async fn find_attestation(
    state: &AppState,
    repo_url: &str,
    commit: &str,
    sha256: &str,
) -> Result<(String, slop_provenance::Provenance), String> {
    let Some(repo_path) = slop_provenance::github_repo_path(repo_url) else {
        return Err(
            "maker signatures are only supported for GitHub repositories so far".to_string(),
        );
    };
    let url = state
        .config
        .releases
        .github_api_base
        .join(&format!("repos/{repo_path}/attestations/sha256:{sha256}"))
        .map_err(|e| e.to_string())?;
    let res = github_get(state, url)
        .await
        .send()
        .await
        .map_err(|e| format!("couldn't reach GitHub to find the attestation: {e}"))?;
    if res.status().as_u16() == 404 {
        return Err(NO_ATTESTATION.to_string());
    }
    if !res.status().is_success() {
        return Err(format!(
            "GitHub answered {} when looking for the attestation",
            res.status().as_u16()
        ));
    }
    #[derive(Deserialize)]
    struct Attestations {
        #[serde(default)]
        attestations: Vec<Attestation>,
    }
    #[derive(Deserialize)]
    struct Attestation {
        bundle: Option<serde_json::Value>,
    }
    let found: Attestations = res
        .json()
        .await
        .map_err(|e| format!("unexpected attestation response from GitHub: {e}"))?;

    // Attestations name the signing workflow as a URL: https://github.com/<recipe>@<ref>.
    let recipe_prefix = state
        .config
        .releases
        .slop_build_workflow
        .as_ref()
        .map(|w| format!("https://github.com/{w}@"));
    let repo_id = crate::repo_identity::recorded_for_url(state, repo_url)
        .await
        .map_err(|e| format!("couldn't read the app's repository ID: {e}"))?;
    let expected = slop_provenance::Expected {
        repo_url,
        commit,
        slop_build_workflow: recipe_prefix.as_deref(),
        repo_id,
    };
    let mut last_problem = NO_ATTESTATION.to_string();
    for bundle in found.attestations.into_iter().filter_map(|a| a.bundle) {
        let json = bundle.to_string();
        match state.provenance.verify_sha256(&json, sha256, expected) {
            Ok(provenance) => return Ok((json, provenance)),
            Err(err) => last_problem = err.to_string(),
        }
    }
    Err(last_problem)
}

const NO_ATTESTATION: &str = "no build attestation found for this file. Build it in GitHub Actions and \
    sign it with actions/attest-build-provenance (Slop Build does this for you)";

/// For GitHub repos, checks the commit exists (SECURITY §5.1). Other hosts are checked by the
/// reviewer for now; returns false for them.
async fn check_commit(state: &AppState, repo_url: &str, commit: &str) -> anyhow::Result<bool> {
    let repo = Url::parse(repo_url)?;
    if !is_github(&repo) {
        return Ok(false);
    }
    let path = repo.path().trim_matches('/').trim_end_matches(".git");
    let url = state
        .config
        .releases
        .github_api_base
        .join(&format!("repos/{path}/commits/{commit}"))?;
    let res = github_get(state, url)
        .await
        .send()
        .await
        .context("checking the commit on GitHub")?;
    match res.status().as_u16() {
        200 => Ok(true),
        404 | 422 => bail!("commit {commit} isn't in {repo_url}"),
        code => bail!("GitHub answered {code} when checking the commit"),
    }
}

/// Streams a file, hashing as it goes. Nothing is written to disk.
pub async fn hash_download(
    http: &reqwest::Client,
    url: &str,
    max: u64,
) -> anyhow::Result<(u64, String)> {
    download(http, url, max, None).await
}

/// Downloads and hashes a file, writing it to `keep` too when given (for scanning).
pub async fn download(
    http: &reqwest::Client,
    url: &str,
    max: u64,
    keep: Option<&std::path::Path>,
) -> anyhow::Result<(u64, String)> {
    download_with(http, url, None, max, keep).await
}

/// `download`, with an `Authorization` header for a private repository. The HTTP client drops
/// it if a redirect leaves the host, so it never reaches file storage elsewhere.
pub async fn download_with(
    http: &reqwest::Client,
    url: &str,
    auth: Option<&str>,
    max: u64,
    keep: Option<&std::path::Path>,
) -> anyhow::Result<(u64, String)> {
    download_full(http, url, auth, max, keep)
        .await
        .map(|d| (d.size, d.sha256))
}

/// A downloaded file's size and digests (SHA-512 too, for makers' SSH signatures).
pub struct Downloaded {
    pub size: u64,
    pub sha256: String,
    pub sha512: [u8; 64],
}

pub async fn download_full(
    http: &reqwest::Client,
    url: &str,
    auth: Option<&str>,
    max: u64,
    keep: Option<&std::path::Path>,
) -> anyhow::Result<Downloaded> {
    use sha2::Sha512;
    use tokio::io::AsyncWriteExt;
    let mut out = match keep {
        Some(p) => Some(tokio::fs::File::create(p).await.context("temporary file")?),
        None => None,
    };
    let mut request = http.get(url);
    if let Some(auth) = auth {
        request = request.header(reqwest::header::AUTHORIZATION, auth);
    }
    let res = request.send().await.context("download failed")?;
    if !res.status().is_success() {
        bail!("download failed: HTTP {}", res.status().as_u16());
    }
    if res.content_length().is_some_and(|len| len > max) {
        bail!("file is larger than the {max}-byte limit");
    }
    let mut hasher = Sha256::new();
    let mut hasher512 = Sha512::new();
    let mut size: u64 = 0;
    let mut res = res;
    while let Some(chunk) = res.chunk().await.context("download interrupted")? {
        size += chunk.len() as u64;
        if size > max {
            return Err(anyhow!("file is larger than the {max}-byte limit"));
        }
        hasher.update(&chunk);
        hasher512.update(&chunk);
        if let Some(f) = out.as_mut() {
            f.write_all(&chunk).await.context("temporary file")?;
        }
    }
    if let Some(f) = out.as_mut() {
        f.flush().await?;
    }
    if size == 0 {
        bail!("file is empty");
    }
    Ok(Downloaded {
        size,
        sha256: hex::encode(hasher.finalize()),
        sha512: hasher512.finalize().into(),
    })
}

/// A file released in parts: each downloaded in order, hashed on its own and as the joined file
/// (and appended to `keep` for scanning). Each part is held to the per-file limit, the whole to
/// `MAX_PARTS` of them. Returns the joined file's digests and each part's (size, SHA-256).
async fn download_parts(
    state: &AppState,
    urls: &[String],
    auth: Option<&str>,
    keep: Option<&std::path::Path>,
) -> anyhow::Result<(Downloaded, Vec<(u64, String)>)> {
    use sha2::Sha512;
    use tokio::io::AsyncWriteExt;
    let max = state.config.releases.max_file_bytes;
    let mut out = match keep {
        Some(p) => Some(tokio::fs::File::create(p).await.context("temporary file")?),
        None => None,
    };
    let (mut whole256, mut whole512) = (Sha256::new(), Sha512::new());
    let mut total = 0u64;
    let mut each = Vec::new();
    for (i, url) in urls.iter().enumerate() {
        let source = if auth.is_some() {
            url.clone()
        } else {
            download_link(state, url).await?
        };
        let mut request = state.fetch_http.get(&source);
        if let Some(auth) = auth {
            request = request.header(reqwest::header::AUTHORIZATION, auth);
        }
        let mut res = request
            .send()
            .await
            .with_context(|| format!("part {}", i + 1))?;
        if !res.status().is_success() {
            bail!(
                "part {}: download failed: HTTP {}",
                i + 1,
                res.status().as_u16()
            );
        }
        let mut part256 = Sha256::new();
        let mut size = 0u64;
        while let Some(chunk) = res.chunk().await.context("download interrupted")? {
            size += chunk.len() as u64;
            if size > max {
                bail!("part {} is larger than the {max}-byte limit", i + 1);
            }
            part256.update(&chunk);
            whole256.update(&chunk);
            whole512.update(&chunk);
            if let Some(f) = out.as_mut() {
                f.write_all(&chunk).await.context("temporary file")?;
            }
        }
        if size == 0 {
            bail!("part {} is empty", i + 1);
        }
        total += size;
        each.push((size, hex::encode(part256.finalize())));
    }
    if let Some(f) = out.as_mut() {
        f.flush().await?;
    }
    Ok((
        Downloaded {
            size: total,
            sha256: hex::encode(whole256.finalize()),
            sha512: whole512.finalize().into(),
        },
        each,
    ))
}

/// The maker's own signature on a file without an attestation (SECURITY §5.3): `<file>.sig` on
/// the release page, made with one of the maker's registered keys that's past its 72-hour wait.
/// `Ok(None)` when the maker has no usable key or there's no .sig.
async fn maker_signature(
    state: &AppState,
    app_id: Uuid,
    url: &str,
    auth: Option<&str>,
    file: &Downloaded,
) -> anyhow::Result<Result<serde_json::Value, String>> {
    let keys: Vec<(String, String, String)> = sqlx::query_as(
        "select k.public_key, k.fingerprint, k.name from maker_keys k join apps a on a.maker_id = k.user_id
         where a.id = $1 and k.revoked_at is null and k.active_from <= now()",
    )
    .bind(app_id)
    .fetch_all(&state.db)
    .await?;
    if keys.is_empty() {
        return Ok(Err("no maker signature: no build attestation, and the maker has no signing key that's past its 72-hour wait".into()));
    }
    let mut request = state.fetch_http.get(format!("{url}.sig"));
    if let Some(auth) = auth {
        request = request.header(reqwest::header::AUTHORIZATION, auth);
    }
    let res = match request.send().await {
        Ok(r) if r.status().is_success() => r,
        _ => {
            return Ok(Err(format!(
                "no maker signature: no build attestation, and no {} file next to it on the release page",
                url.rsplit('/')
                    .next()
                    .map(|n| format!("{n}.sig"))
                    .unwrap_or_else(|| ".sig".into())
            )));
        }
    };
    let bytes = res.bytes().await?;
    if bytes.len() > 16 * 1024 {
        return Ok(Err("the .sig file is too big to be an SSH signature".into()));
    }
    let armored = String::from_utf8_lossy(&bytes).to_string();
    let sha256: [u8; 32] = hex::decode(&file.sha256)?
        .try_into()
        .map_err(|_| anyhow!("bad digest"))?;
    let mut last = String::new();
    for (public_key, fingerprint, name) in keys {
        let (key, _) = slop_manifest::sshsig::parse_public_key(&public_key)?;
        match slop_manifest::sshsig::verify(&key, &armored, &file.sha512, &sha256) {
            Ok(()) => {
                return Ok(Ok(serde_json::json!({
                    "public_key": public_key,
                    "signature": armored,
                    "fingerprint": fingerprint,
                    "key_name": name,
                })));
            }
            Err(e) => last = e.to_string(),
        }
    }
    Ok(Err(format!("the .sig file doesn't check out: {last}")))
}

/// The approved shared runtimes a release can name (slop-manifest runtimes.rs).
async fn runtimes_list() -> Json<serde_json::Value> {
    Json(serde_json::Value::Array(
        slop_manifest::runtimes::RUNTIMES
            .iter()
            .map(|r| serde_json::json!({ "id": r.id, "name": r.name, "vendor": r.vendor }))
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = "https://github.com/octo/game";

    #[test]
    fn file_urls_must_be_on_the_makers_release_page() {
        let (url, name) = validate_file_url(
            REPO,
            "https://github.com/octo/game/releases/download/v1/Game%20Linux.zip",
            false,
        )
        .unwrap();
        assert_eq!(
            url,
            "https://github.com/octo/game/releases/download/v1/Game%20Linux.zip"
        );
        assert_eq!(name, "Game Linux.zip");
        for bad in [
            "https://github.com/evil/game/releases/download/v1/x.zip",
            "https://github.com/octo/game-evil/releases/download/v1/x.zip",
            "https://evil.example/octo/game/releases/download/v1/x.zip",
            "http://github.com/octo/game/releases/download/v1/x.zip",
            "https://github.com/octo/game/releases/download/v1/x.zip?redirect=evil",
        ] {
            assert!(validate_file_url(REPO, bad, false).is_err(), "{bad}");
        }
        // Self-hosted: same host as the repo.
        assert!(
            validate_file_url(
                "https://git.example/me/app",
                "https://git.example/me/app/-/releases/x.zip",
                false
            )
            .is_ok()
        );
        assert!(
            validate_file_url(
                "https://git.example/me/app",
                "https://cdn.example/x.zip",
                false
            )
            .is_err()
        );
    }

    #[test]
    fn github_release_file_urls() {
        assert_eq!(
            github_release_file(
                "https://github.com/octo/game/releases/download/v1.0/My%20Game.zip"
            ),
            Some(("octo/game".into(), "v1.0".into(), "My Game.zip".into()))
        );
        assert_eq!(
            github_release_file("https://github.com/octo/game/archive/main.zip"),
            None
        );
        assert_eq!(
            github_release_file("https://git.example/octo/game/releases/download/v1/x.zip"),
            None
        );
        assert_eq!(
            github_release_file("https://github.com/octo/game/releases/download/v1/x.zip/extra"),
            None
        );
    }

    #[test]
    fn play_addresses_are_plain_https_addresses_of_public_sites() {
        let own = vec![
            ("slop.example".to_string(), 443),
            ("play.slop.example".to_string(), 443),
            ("localhost".to_string(), 8080),
        ];
        let ok = |a: &str| validate_play_address(a, false, &own).unwrap();
        assert_eq!(
            ok(" https://Games.Example.com "),
            "https://games.example.com/"
        );
        assert_eq!(
            ok("https://octo.github.io/game/?level=2#start"),
            "https://octo.github.io/game/?level=2#start"
        );
        assert_eq!(
            ok("https://bücher.example/spiel"),
            "https://xn--bcher-kva.example/spiel"
        );
        // A dot after the name is the same site: kept without it, so the bad-link lists match it.
        assert_eq!(ok("https://evil.example./"), "https://evil.example/");
        assert_eq!(
            ok("https://WWW.evil.example.:443/x"),
            "https://www.evil.example/x"
        );
        let refused = |a: &str| match validate_play_address(a, false, &own) {
            Err(ApiError::Invalid { field, message }) => {
                assert_eq!(field, "play_address", "{a}");
                message
            }
            other => panic!("{a} isn't refused: {other:?}"),
        };
        for (address, why) in [
            ("", "give the https"),
            ("games.example.com", "https://"),
            ("http://games.example.com/", "https://"),
            ("ftp://games.example.com/", "https://"),
            ("javascript:alert(1)", "https://"),
            ("data:text/html,hi", "https://"),
            ("https://", "https://"),
            ("https://./", "https://"),
            (
                "https://me:secret@games.example.com/",
                "user name and password",
            ),
            ("https://me@games.example.com/", "user name and password"),
            ("https://127.0.0.1/", "private network"),
            ("https://10.1.2.3/game", "private network"),
            ("https://169.254.169.254/", "private network"),
            ("https://[::1]/", "private network"),
            ("https://[::ffff:192.168.0.1]/", "private network"),
            ("https://0x7f.1/", "private network"),
            ("https://8.8.8.8/", "not its IP address"),
            ("https://localhost/", "private network"),
            ("https://game.localhost/", "private network"),
            ("https://intranet/", "full name"),
            ("https://slop.example/play", "SLAPorSLOP's own site"),
            ("https://SLOP.example./", "SLAPorSLOP's own site"),
            ("https://demo.play.slop.example/x/", "SLAPorSLOP's own site"),
            ("https://slop.example:8443/", "SLAPorSLOP's own site"),
        ] {
            let message = refused(address);
            assert!(message.contains(why), "{address}: {message}");
        }
        let long = format!("https://games.example.com/{}", "a".repeat(2000));
        assert!(refused(&long).contains("2,000"));
        assert!(validate_play_address("https://slop.example.games.com/", false, &own).is_ok());

        // Test setups serve pages over http from this machine, told apart by port.
        let test = |a: &str| validate_play_address(a, true, &own);
        assert_eq!(
            test("http://127.0.0.1:4000/octo/game/play/").unwrap(),
            "http://127.0.0.1:4000/octo/game/play/"
        );
        assert!(test("http://localhost:4000/").is_ok());
        assert!(test("http://localhost:8080/").is_err(), "the site itself");
        assert!(test("ftp://127.0.0.1/").is_err());
        assert!(test("http://u:p@127.0.0.1:4000/").is_err());
    }

    #[test]
    fn the_same_play_address_is_exactly_the_same() {
        // Written differently, kept the same.
        for (a, b) in [
            (
                "https://games.example.com/play/",
                "HTTPS://Games.Example.COM:443/play/",
            ),
            (
                "https://games.example.com/play/",
                "https://games.example.com./play/",
            ),
            ("https://games.example.com", "https://games.example.com/"),
        ] {
            assert!(same_address(a, b), "{a} {b}");
        }
        // Anything else is another address, which a person looks at.
        for (a, b) in [
            (
                "https://games.example.com/play/",
                "https://www.games.example.com/play/",
            ),
            (
                "https://portal.example/#/game/1",
                "https://portal.example/#/game/2",
            ),
            (
                "https://games.example.com/play/",
                "https://games.example.com/play/#top",
            ),
            (
                "https://games.example.com/play/",
                "https://games.example.com/play",
            ),
            (
                "https://games.example.com/?level=1",
                "https://games.example.com/?level=2",
            ),
            (
                "https://games.example.com/play/",
                "https://games.example.com:8443/play/",
            ),
            (
                "https://games.example.com/play/",
                "https://games.example.com/play/2",
            ),
            ("https://games.example.com/", "http://games.example.com/"),
            ("https://games.example.com/", "https://evil.example/"),
        ] {
            assert!(!same_address(a, b), "{a} {b}");
        }
        assert!(!same_address("not an address", "not an address"));
    }

    #[test]
    fn versions_and_commits() {
        assert!(validate_version("1.2.0-beta.1").is_ok());
        assert!(validate_version("-1").is_err());
        assert!(validate_version("1 2").is_err());
        assert!(validate_commit(&"a".repeat(40)).is_ok());
        assert!(validate_commit(&"A".repeat(40)).is_err());
        assert!(validate_commit("abc").is_err());
    }
}
