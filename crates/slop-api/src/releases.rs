// SPDX-License-Identifier: AGPL-3.0-or-later
//! Releases: makers submit them, Slop fetches and hashes their files, a reviewer approves them
//! and Slop signs the manifest (docs/ARCHITECTURE.md §6.3, docs/SECURITY.md §5).
//!
//! Builds stay on the maker's release page. Slop downloads each file once to hash it, keeps
//! nothing but the hash and size, and the client later downloads from the same URL and checks
//! the hash against Slop's signed manifest.

use std::time::Duration as StdDuration;

use anyhow::{Context, anyhow, bail};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::get,
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
/// A fetch that started this long ago is presumed dead and retried.
const FETCH_LEASE: Duration = Duration::minutes(30);

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/apps/{slug}/releases",
            get(list_releases).post(submit_release),
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
    commit_sha: String,
    files: Vec<NewFile>,
    /// This release's rating (AGE-7); the store page's when left out.
    #[serde(default)]
    rating: Option<Rating>,
    /// Shared runtimes it needs (slop-manifest runtimes.rs); the last approved release's when
    /// left out.
    #[serde(default)]
    runtimes: Option<Vec<String>>,
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

fn validate_commit(sha: &str) -> ApiResult<()> {
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
fn validate_file_url(repo_url: &str, value: &str, allow_http: bool) -> ApiResult<(String, String)> {
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
    /// True when Slop approved it automatically as an update (updates.rs).
    auto_approved: bool,
    /// Its rating (AGE-7).
    rating: Option<Rating>,
    /// Shared runtimes it needs.
    runtimes: Vec<String>,
    files: Vec<ReleaseFileInfo>,
    /// Slop Build's source checks: possible secrets and vulnerable dependencies.
    source_checks: Option<serde_json::Value>,
    /// Slop's own source check (source_scan.rs): licence, monetization notes, package count.
    repo_scan: Option<serde_json::Value>,
    /// Advisories for its packages (dep_watch.rs), newest first.
    advisories: Vec<Advisory>,
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
    crate::passkeys::require(&state, &user).await?;
    let release = create_release(
        &state,
        app_id,
        &repo_url,
        user.id,
        &req.version,
        &req.commit_sha,
        req.files,
        "maker",
        req.rating,
        req.runtimes,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(release)))
}

/// Checks and records a new release for fetching and review. Shared by makers' own submissions,
/// Slop Build's (slop_build.rs) and the GitHub release watcher (updates.rs), so all go through
/// the same checks. `source` is maker, slop_build or github_release.
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
) -> ApiResult<ReleaseInfo> {
    validate_version(version)?;
    validate_commit(commit_sha)?;
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
        return Err(ApiError::Conflict(format!(
            "this app is frozen, so it can't take new releases: {reason}. Restore Slop's access \
             to the repository, then press Check now on the app's Manage page"
        )));
    }
    let required: Vec<String> = sqlx::query_scalar("select platforms from apps where id = $1")
        .bind(app_id)
        .fetch_one(&state.db)
        .await?;
    let files = validate_files(
        repo_url,
        files,
        state.config.releases.allow_http_downloads,
        &required,
    )?;

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

    let id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    sqlx::query("insert into releases (id, app_id, version, commit_sha, submitted_by, source, min_age, content_labels, runtimes) values ($1, $2, $3, $4, $5, $6, $7, $8, $9)")
        .bind(id)
        .bind(app_id)
        .bind(version)
        .bind(commit_sha)
        .bind(submitted_by)
        .bind(source)
        .bind(rating.min_age)
        .bind(&rating.content_labels)
        .bind(&runtimes)
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

/// What happened to a release, for its maker.
pub enum ReleaseEvent<'a> {
    Received { source: &'a str },
    Approved { automatically: bool },
    Rejected { reason: &'a str },
    Failed { reason: &'a str },
    Withdrawn { reason: &'a str },
}

/// Tells the app's maker about every release (SECURITY §5.3, §5.4): if someone else publishes in
/// their name (a stolen account, a compromised repository), they hear about it straight away,
/// under the 🔔 and by email.
pub async fn tell_maker(
    conn: &mut sqlx::PgConnection,
    release_id: Uuid,
    event: ReleaseEvent<'_>,
) -> sqlx::Result<()> {
    let row: Option<(Uuid, String, String, String)> = sqlx::query_as(
        "select a.maker_id, a.slug, a.title, r.version from releases r join apps a on a.id = r.app_id where r.id = $1",
    )
    .bind(release_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((maker, slug, title, version)) = row else {
        return Ok(());
    };
    let (headline, body) = match event {
        ReleaseEvent::Received { source } => (
            format!("{title} {version} was submitted"),
            format!(
                "{}. Slop is checking it now. If you didn't publish this, withdraw it from Manage and secure your accounts and repository.",
                match source {
                    "slop_build" => "Slop Build sent it from your repository",
                    "github_release" => "Slop picked it up from a GitHub release",
                    _ => "It was submitted from your account",
                }
            ),
        ),
        ReleaseEvent::Approved { automatically } => (
            format!("{title} {version} is approved"),
            if automatically {
                "It passed every check and was approved automatically. Players get it as an update."
                    .into()
            } else {
                "A reviewer approved it. Players get it as an update.".into()
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
            format!("It no longer launches for anyone. Why: {reason}"),
        ),
    };
    crate::notify::send(
        conn,
        crate::notify::Notice {
            user_id: maker,
            category: "apps",
            title: headline,
            body,
            link: format!("/publish/{slug}/releases"),
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
    }
    let row: Row = sqlx::query_as(
        "select id, version, commit_sha, status::text as status, status_reason, commit_verified, created_at, source,
                coalesce(reviewed_by = 'slop', false) as auto_approved, min_age, content_labels, runtimes
         from releases where id = $1",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    let files = sqlx::query_as(
        "select platform, kind, url, file_name, exec, args, size_bytes, sha256, provenance, provenance_problem,
                scan_result, scan_detail, scan_report, defender, cleared_reason,
                maker_signature - 'signature' - 'public_key' as maker_key
         from release_files where release_id = $1 order by platform",
    )
    .bind(id)
    .fetch_all(&state.db)
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
    })
}

/// Makers see every release with its status; everyone else sees approved releases.
async fn list_releases(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<ReleaseInfo>>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let (app_id, is_mine) = catalog::visible_app(&state, &slug, user.as_ref(), policy).await?;
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "select id from releases where app_id = $1 and ($2 or status = 'approved') order by created_at desc limit 50",
    )
    .bind(app_id)
    .bind(is_mine)
    .fetch_all(&state.db)
    .await?;
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
        }
        out.push(release);
    }
    Ok(Json(out))
}

/// The newest approved release's signed manifest, for installing.
/// The approved releases players can get, the one they do get first: the release the maker
/// holds the app at, otherwise the newest approved. Bind the app's ID as $1.
pub const SERVING: &str = "from releases r join apps a on a.id = r.app_id
     where r.app_id = $1 and r.status = 'approved' and r.signed_manifest is not null
     order by coalesce(r.id = a.pinned_release_id, false) desc, r.reviewed_at desc";

async fn latest_manifest(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<Json<Signed>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let (app_id, _) = catalog::visible_app(&state, &slug, user.as_ref(), policy).await?;
    let manifest: Option<sqlx::types::Json<Signed>> = sqlx::query_scalar(sqlx::AssertSqlSafe(
        format!("select r.signed_manifest {SERVING} limit 1"),
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
/// otherwise the file's own URL. Slop never relays the file itself.
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

/// The current revocation list, freshly signed (SECURITY §5.5).
async fn revocations(State(state): State<AppState>) -> ApiResult<Json<Signed>> {
    let key = state
        .config
        .releases
        .signing_key
        .as_ref()
        .ok_or(ApiError::Unavailable("release signing isn't set up"))?;
    let rows: Vec<(Uuid, String, Option<String>, OffsetDateTime)> = sqlx::query_as(
        "select r.id, a.slug, r.status_reason, r.revoked_at from releases r join apps a on a.id = r.app_id
         where r.status = 'revoked' order by r.revoked_at",
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

/// "Does Slop know this file?" (docs/standard/file-verification.md), signed, for anyone: other
/// stores showing "verified by Slop", or players checking a download. Only public apps are
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

/// Slop's current public signing key. Clients pin keys at build time; this is for transparency.
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

/// Fetches one release, if one is waiting. Returns whether it did any work.
pub async fn fetch_next(state: &AppState) -> anyhow::Result<bool> {
    let claimed: Option<(Uuid, Uuid, String, String)> = sqlx::query_as(
        "update releases r set fetch_started_at = now()
         from apps a
         where a.id = r.app_id and r.id = (
             select id from releases
             where status = 'fetching' and (fetch_started_at is null or fetch_started_at < now() - $1)
             order by created_at limit 1 for update skip locked)
         returning r.id, r.app_id, r.commit_sha, a.repo_url",
    )
    .bind(FETCH_LEASE)
    .fetch_optional(&state.db)
    .await?;
    let Some((id, app_id, commit, repo_url)) = claimed else {
        return Ok(false);
    };

    match fetch_release(state, id, &commit, &repo_url).await {
        Ok(commit_verified) => {
            sqlx::query(
                "update releases set status = 'awaiting_review', commit_verified = $2
                 where id = $1 and status = 'fetching'",
            )
            .bind(id)
            .bind(commit_verified)
            .execute(&state.db)
            .await?;
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
            sqlx::query("update releases set status = 'failed', status_reason = $2 where id = $1 and status = 'fetching'")
                .bind(id)
                .bind(&reason)
                .execute(&mut *tx)
                .await?;
            tell_maker(&mut tx, id, ReleaseEvent::Failed { reason: &reason }).await?;
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
                 parts = coalesce($12, parts)
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

pub(crate) async fn github_get(state: &AppState, url: Url) -> reqwest::RequestBuilder {
    github_request(state, &state.fetch_http, url, "application/vnd.github+json").await
}

/// A GitHub API GET on a chosen client, with the repository's GitHub App token or Slop's
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
    let expected = slop_provenance::Expected {
        repo_url,
        commit,
        slop_build_workflow: recipe_prefix.as_deref(),
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
    fn versions_and_commits() {
        assert!(validate_version("1.2.0-beta.1").is_ok());
        assert!(validate_version("-1").is_err());
        assert!(validate_version("1 2").is_err());
        assert!(validate_commit(&"a".repeat(40)).is_ok());
        assert!(validate_commit(&"A".repeat(40)).is_err());
        assert!(validate_commit("abc").is_err());
    }
}
