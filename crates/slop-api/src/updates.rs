//! Updates without the paperwork (decided 2026-10-19).
//!
//! - **Picking a release:** makers choose one of their repository's GitHub releases instead of
//!   typing a version, commit and file links. Slop resolves the tag to its commit.
//! - **Watching:** Slop checks each app's repository for a new published GitHub release and
//!   submits it, using the previous release's files as the pattern (same file names with the
//!   version swapped, same program to start). Apps built by Slop Build are left to it.
//! - **Automatic approval:** once a person has approved one release of an app, later releases
//!   that pass every check are approved and signed without waiting for a person. A withdrawal or
//!   rejection since that approval sends the next release back to a person.

use std::time::Duration as StdDuration;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{
    AppState, admin,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
    releases::{self, NewFile, ReleaseInfo},
    slop_build::path_segment,
};

/// The reviewer name on automatic approvals, in releases.reviewed_by and the audit log.
pub const AUTO_REVIEWER: &str = "slop";
/// How often the watcher looks at every repository.
const WATCH_INTERVAL: StdDuration = StdDuration::from_secs(10 * 60);
/// A new GitHub release whose files don't match yet is retried for this long, since files are
/// often uploaded after the release is published.
const WAIT_FOR_FILES: Duration = Duration::hours(2);

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/apps/{slug}/github-releases", get(github_releases))
        .route(
            "/apps/{slug}/releases/from-github",
            post(submit_from_github),
        )
        .route(
            "/apps/{slug}/update-settings",
            get(get_settings).put(put_settings),
        )
}

// ---------------------------------------------------------------------------------------------
// Automatic approval

/// Whether the app's next release may be approved without a person, and if not, why.
pub async fn auto_approval(db: &sqlx::PgPool, app_id: Uuid) -> anyhow::Result<Result<(), String>> {
    let (enabled, approved_at, setback): (bool, Option<OffsetDateTime>, bool) = sqlx::query_as(
        "select a.auto_update,
                h.at,
                exists (select 1 from releases r where r.app_id = a.id and h.at is not null and (
                    (r.status = 'revoked' and r.revoked_at > h.at) or
                    (r.status = 'rejected' and r.reviewed_at > h.at)))
         from apps a
         left join lateral (
             select max(reviewed_at) as at from releases r
             where r.app_id = a.id and r.status in ('approved', 'revoked')
               and coalesce(r.reviewed_by, '') <> $2
         ) h on true
         where a.id = $1",
    )
    .bind(app_id)
    .bind(AUTO_REVIEWER)
    .fetch_one(db)
    .await?;
    Ok(if !enabled {
        Err("automatic updates are off for this app".into())
    } else if approved_at.is_none() {
        Err("an app's first release is always reviewed by a person".into())
    } else if setback {
        Err("a release was withdrawn or rejected since the last one a person approved".into())
    } else {
        Ok(())
    })
}

/// Called once a release's files are checked. Approves it if the app qualifies and every file
/// meets the server's rules; otherwise it stays waiting for a person, with the reason noted.
pub async fn try_auto_approve(state: &AppState, release_id: Uuid) -> anyhow::Result<()> {
    let app_id: Uuid = sqlx::query_scalar("select app_id from releases where id = $1")
        .bind(release_id)
        .fetch_one(&state.db)
        .await?;
    let why_not = match auto_approval(&state.db, app_id).await? {
        Ok(()) => {
            let cfg = &state.config;
            match admin::approve(
                &state.db,
                cfg.releases.signing_key.as_ref(),
                admin::ReviewRules::from_config(cfg),
                release_id,
                AUTO_REVIEWER,
            )
            .await
            {
                Ok(()) => {
                    tracing::info!(release = %release_id, "release approved automatically");
                    return Ok(());
                }
                Err(err) => format!("Waiting for a person: {err:#}"),
            }
        }
        Err(why) => format!("Waiting for a person: {why}"),
    };
    sqlx::query(
        "update releases set status_reason = $2 where id = $1 and status = 'awaiting_review'",
    )
    .bind(release_id)
    .bind(why_not)
    .execute(&state.db)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// GitHub releases

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    name: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    published_at: Option<String>,
    html_url: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    size: u64,
}

/// `v1.2.0` → `1.2.0`; other tags are used as they are.
pub fn version_from_tag(tag: &str) -> &str {
    match tag.strip_prefix('v').or_else(|| tag.strip_prefix('V')) {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => rest,
        _ => tag,
    }
}

/// A first guess at what a release file is, from its name: (platform, kind).
pub fn guess_file(name: &str) -> Option<(&'static str, &'static str)> {
    let n = name.to_ascii_lowercase();
    if n.ends_with(".deb") {
        Some(("linux-deb", "installer"))
    } else if n.ends_with(".rpm") {
        Some(("linux-rpm", "installer"))
    } else if n.ends_with(".tar.gz") || n.ends_with(".tgz") {
        Some(("linux", "archive"))
    } else if n.ends_with(".zip") {
        Some(("windows", "archive"))
    } else {
        None
    }
}

/// A previous release's file, used as the pattern for the next release.
#[derive(sqlx::FromRow)]
pub struct TemplateFile {
    pub platform: String,
    pub kind: String,
    pub file_name: String,
    pub exec: Option<String>,
    pub args: Vec<String>,
}

/// Builds the next release's files from the previous release's, by file name with the version
/// swapped (`game-1.2.0-windows.zip` → `game-1.3.0-windows.zip`). Errors say which is missing.
pub(crate) fn files_from_template(
    template: &[TemplateFile],
    old_version: &str,
    new_version: &str,
    asset_names: &[String],
    url_for: impl Fn(&str) -> String,
) -> Result<Vec<NewFile>, String> {
    let swap = |s: &str| {
        if old_version.is_empty() {
            s.to_string()
        } else {
            s.replace(old_version, new_version)
        }
    };
    template
        .iter()
        .map(|t| {
            let wanted = swap(&t.file_name);
            let name = asset_names
                .iter()
                .find(|a| **a == wanted)
                .or_else(|| asset_names.iter().find(|a| **a == t.file_name))
                .ok_or_else(|| {
                    format!(
                        "no file named {wanted} for {} (the last release had {})",
                        t.platform, t.file_name
                    )
                })?;
            Ok(NewFile {
                platform: t.platform.clone(),
                kind: t.kind.clone(),
                url: url_for(name),
                exec: t.exec.as_deref().map(swap),
                args: t.args.clone(),
            })
        })
        .collect()
}

fn github_path(repo_url: &str) -> ApiResult<String> {
    slop_provenance::github_repo_path(repo_url).ok_or_else(|| {
        ApiError::Conflict("picking releases only works for GitHub repositories so far".into())
    })
}

async fn gh_json<T: serde::de::DeserializeOwned>(
    state: &AppState,
    path: &str,
) -> anyhow::Result<Option<T>> {
    let url = state.config.releases.github_api_base.join(path)?;
    let res = releases::github_get(state, url).send().await?;
    match res.status().as_u16() {
        200 => Ok(Some(res.json().await?)),
        404 => Ok(None),
        code => anyhow::bail!("GitHub answered {code}"),
    }
}

/// The commit a tag points at.
async fn tag_commit(state: &AppState, repo_path: &str, tag: &str) -> anyhow::Result<String> {
    #[derive(Deserialize)]
    struct Commit {
        sha: String,
    }
    let commit: Commit = gh_json(
        state,
        &format!("repos/{repo_path}/commits/{}", path_segment(tag)),
    )
    .await?
    .ok_or_else(|| anyhow::anyhow!("the tag {tag} isn't in the repository"))?;
    Ok(commit.sha.to_ascii_lowercase())
}

fn download_url(repo_url: &str, tag: &str, name: &str) -> String {
    format!(
        "{}/releases/download/{}/{}",
        repo_url.trim_end_matches('/'),
        path_segment(tag),
        path_segment(name)
    )
}

async fn owned(state: &AppState, slug: &str, user: &CurrentUser) -> ApiResult<(Uuid, String)> {
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

#[derive(Serialize)]
struct PickableAsset {
    name: String,
    url: String,
    size: u64,
    /// Slop's guess, for the form to start from.
    platform: Option<&'static str>,
    kind: Option<&'static str>,
}

#[derive(Serialize)]
struct PickableRelease {
    tag: String,
    name: Option<String>,
    version: String,
    published_at: Option<String>,
    prerelease: bool,
    html_url: String,
    /// Already sent to Slop (any status).
    submitted: bool,
    assets: Vec<PickableAsset>,
}

/// The repository's GitHub releases, newest first, for the "Pick a release" form.
async fn github_releases(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<PickableRelease>>> {
    let (app_id, repo_url) = owned(&state, &slug, &user).await?;
    let repo_path = github_path(&repo_url)?;
    let found: Vec<GhRelease> = gh_json(&state, &format!("repos/{repo_path}/releases?per_page=20"))
        .await
        .map_err(|e| {
            ApiError::Conflict(format!(
                "couldn't read the releases of {repo_url} ({e}); is the repository public?"
            ))
        })?
        .unwrap_or_default();
    let versions: Vec<String> =
        sqlx::query_scalar("select version from releases where app_id = $1")
            .bind(app_id)
            .fetch_all(&state.db)
            .await?;
    Ok(Json(
        found
            .into_iter()
            .filter(|r| !r.draft)
            .map(|r| {
                let version = version_from_tag(&r.tag_name).to_string();
                PickableRelease {
                    submitted: versions.contains(&version),
                    assets: r
                        .assets
                        .into_iter()
                        .map(|a| {
                            let guess = guess_file(&a.name);
                            PickableAsset {
                                url: download_url(&repo_url, &r.tag_name, &a.name),
                                platform: guess.map(|g| g.0),
                                kind: guess.map(|g| g.1),
                                name: a.name,
                                size: a.size,
                            }
                        })
                        .collect(),
                    version,
                    tag: r.tag_name,
                    name: r.name.filter(|n| !n.trim().is_empty()),
                    published_at: r.published_at,
                    prerelease: r.prerelease,
                    html_url: r.html_url,
                }
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FromGithub {
    tag: String,
    files: Vec<NewFile>,
}

/// Submits a GitHub release: the version comes from the tag and the commit from GitHub.
async fn submit_from_github(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(req): Json<FromGithub>,
) -> ApiResult<(StatusCode, Json<ReleaseInfo>)> {
    let (app_id, repo_url) = owned(&state, &slug, &user).await?;
    let repo_path = github_path(&repo_url)?;
    if req.tag.is_empty() || req.tag.len() > 100 || req.tag.chars().any(char::is_control) {
        return Err(ApiError::invalid("tag", "pick a release"));
    }
    let commit = tag_commit(&state, &repo_path, &req.tag)
        .await
        .map_err(|e| ApiError::invalid("tag", format!("{e:#}")))?;
    let release = releases::create_release(
        &state,
        app_id,
        &repo_url,
        user.id,
        version_from_tag(&req.tag),
        &commit,
        req.files,
        "maker",
    )
    .await?;
    Ok((StatusCode::CREATED, Json(release)))
}

// ---------------------------------------------------------------------------------------------
// Settings

#[derive(Serialize)]
struct Settings {
    watch_releases: bool,
    auto_update: bool,
    /// Whether the next release would be approved without a person, and if not, why.
    auto_approval_ready: bool,
    auto_approval_note: Option<String>,
    /// Whether the watcher can work for this app at all, and if not, why.
    watch_note: Option<String>,
    watch_last_tag: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    watch_checked_at: Option<OffsetDateTime>,
    watch_problem: Option<String>,
}

async fn load_settings(state: &AppState, app_id: Uuid, repo_url: &str) -> ApiResult<Settings> {
    let (watch_releases, auto_update, watch_last_tag, watch_checked_at, watch_problem): (
        bool,
        bool,
        Option<String>,
        Option<OffsetDateTime>,
        Option<String>,
    ) = sqlx::query_as(
        "select watch_releases, auto_update, watch_last_tag, watch_checked_at, watch_problem
         from apps where id = $1",
    )
    .bind(app_id)
    .fetch_one(&state.db)
    .await?;
    let approval = auto_approval(&state.db, app_id).await?;
    let newest_source: Option<String> = sqlx::query_scalar(
        "select source from releases where app_id = $1 order by created_at desc limit 1",
    )
    .bind(app_id)
    .fetch_optional(&state.db)
    .await?;
    let watch_note = if slop_provenance::github_repo_path(repo_url).is_none() {
        Some("Only GitHub repositories can be watched so far.".into())
    } else {
        match newest_source.as_deref() {
            None => Some(
                "Submit your first release below; later ones are picked up using it as the pattern."
                    .into(),
            ),
            Some("slop_build") => Some("Slop Build sends your releases, so there's nothing to watch.".into()),
            _ => None,
        }
    };
    Ok(Settings {
        watch_releases,
        auto_update,
        auto_approval_ready: approval.is_ok(),
        auto_approval_note: approval.err(),
        watch_note,
        watch_last_tag,
        watch_checked_at,
        watch_problem,
    })
}

async fn get_settings(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Settings>> {
    let (app_id, repo_url) = owned(&state, &slug, &user).await?;
    Ok(Json(load_settings(&state, app_id, &repo_url).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsChange {
    watch_releases: Option<bool>,
    auto_update: Option<bool>,
}

async fn put_settings(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(change): Json<SettingsChange>,
) -> ApiResult<Json<Settings>> {
    let (app_id, repo_url) = owned(&state, &slug, &user).await?;
    sqlx::query(
        "update apps set watch_releases = coalesce($2, watch_releases),
                         auto_update = coalesce($3, auto_update)
         where id = $1",
    )
    .bind(app_id)
    .bind(change.watch_releases)
    .bind(change.auto_update)
    .execute(&state.db)
    .await?;
    state.watch_wakeup.notify_one();
    Ok(Json(load_settings(&state, app_id, &repo_url).await?))
}

// ---------------------------------------------------------------------------------------------
// Watching

/// Runs forever: looks for new GitHub releases every few minutes.
pub async fn run_watcher(state: AppState) {
    loop {
        if let Err(err) = watch_once(&state).await {
            tracing::error!(error = ?err, "release watcher");
        }
        tokio::select! {
            _ = state.watch_wakeup.notified() => {}
            _ = tokio::time::sleep(WATCH_INTERVAL) => {}
        }
    }
}

/// Checks every watched app once. Returns how many releases it submitted.
pub async fn watch_once(state: &AppState) -> anyhow::Result<usize> {
    // Apps with a release to use as the pattern, whose newest release wasn't sent by Slop Build.
    let apps: Vec<(Uuid, Uuid, String, Option<String>)> = sqlx::query_as(
        "select a.id, a.maker_id, a.repo_url, a.watch_last_tag from apps a
         where a.watch_releases and a.status <> 'delisted'
           and (select r.source from releases r where r.app_id = a.id order by r.created_at desc limit 1)
               in ('maker', 'github_release')",
    )
    .fetch_all(&state.db)
    .await?;
    let mut submitted = 0;
    for (app_id, maker_id, repo_url, last_tag) in apps {
        let Some(repo_path) = slop_provenance::github_repo_path(&repo_url) else {
            continue;
        };
        match watch_app(
            state,
            app_id,
            maker_id,
            &repo_url,
            &repo_path,
            last_tag.as_deref(),
        )
        .await
        {
            Ok(did) => submitted += usize::from(did),
            Err(err) => tracing::warn!(app = %app_id, error = %err, "watching releases"),
        }
    }
    Ok(submitted)
}

async fn note(
    state: &AppState,
    app_id: Uuid,
    seen_tag: Option<&str>,
    problem: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "update apps set watch_checked_at = now(), watch_problem = $3,
                         watch_last_tag = coalesce($2, watch_last_tag)
         where id = $1",
    )
    .bind(app_id)
    .bind(seen_tag)
    .bind(problem)
    .execute(&state.db)
    .await?;
    Ok(())
}

async fn watch_app(
    state: &AppState,
    app_id: Uuid,
    maker_id: Uuid,
    repo_url: &str,
    repo_path: &str,
    last_tag: Option<&str>,
) -> anyhow::Result<bool> {
    // GitHub's "latest" skips drafts and pre-releases.
    let Some(latest): Option<GhRelease> =
        gh_json(state, &format!("repos/{repo_path}/releases/latest")).await?
    else {
        note(state, app_id, None, None).await?;
        return Ok(false);
    };
    if last_tag == Some(latest.tag_name.as_str()) {
        note(state, app_id, None, None).await?;
        return Ok(false);
    }
    let tag = latest.tag_name.clone();
    let version = version_from_tag(&tag).to_string();
    let exists: bool = sqlx::query_scalar(
        "select exists (select 1 from releases where app_id = $1 and version = $2)",
    )
    .bind(app_id)
    .bind(&version)
    .fetch_one(&state.db)
    .await?;
    if exists {
        note(state, app_id, Some(&tag), None).await?;
        return Ok(false);
    }

    // The pattern: the newest approved release, or else the newest one with files.
    let template_id: Option<(Uuid, String)> = sqlx::query_as(
        "select id, version from releases where app_id = $1 and status <> 'failed'
         order by (status = 'approved') desc, created_at desc limit 1",
    )
    .bind(app_id)
    .fetch_optional(&state.db)
    .await?;
    let Some((template_id, old_version)) = template_id else {
        return Ok(false);
    };
    let template: Vec<TemplateFile> = sqlx::query_as(
        "select platform, kind, file_name, exec, args from release_files where release_id = $1",
    )
    .bind(template_id)
    .fetch_all(&state.db)
    .await?;
    let names: Vec<String> = latest.assets.iter().map(|a| a.name.clone()).collect();
    let files = match files_from_template(&template, &old_version, &version, &names, |n| {
        download_url(repo_url, &tag, n)
    }) {
        Ok(files) => files,
        Err(problem) => {
            let young = latest
                .published_at
                .as_deref()
                .and_then(|p| {
                    OffsetDateTime::parse(p, &time::format_description::well_known::Rfc3339).ok()
                })
                .is_some_and(|p| OffsetDateTime::now_utc() - p < WAIT_FOR_FILES);
            let message = if young {
                format!("{tag}: waiting for its files ({problem})")
            } else {
                format!("{tag} couldn't be sent automatically: {problem}. Pick it below instead.")
            };
            // Give up on it once it's old enough that the files aren't still uploading.
            note(
                state,
                app_id,
                (!young).then_some(tag.as_str()),
                Some(&message),
            )
            .await?;
            return Ok(false);
        }
    };
    let commit = tag_commit(state, repo_path, &tag).await?;
    match releases::create_release(
        state,
        app_id,
        repo_url,
        maker_id,
        &version,
        &commit,
        files,
        "github_release",
    )
    .await
    {
        Ok(_) => {
            note(state, app_id, Some(&tag), None).await?;
            tracing::info!(app = %app_id, %tag, "picked up a new GitHub release");
            Ok(true)
        }
        Err(err) => {
            let message = format!("{tag} couldn't be sent automatically: {err}");
            note(state, app_id, Some(&tag), Some(&message)).await?;
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_from_tags() {
        assert_eq!(version_from_tag("v1.2.0"), "1.2.0");
        assert_eq!(version_from_tag("1.2.0"), "1.2.0");
        assert_eq!(version_from_tag("vacation"), "vacation");
    }

    #[test]
    fn guesses() {
        assert_eq!(guess_file("game-windows.zip"), Some(("windows", "archive")));
        assert_eq!(
            guess_file("game-linux-x86_64.tar.gz"),
            Some(("linux", "archive"))
        );
        assert_eq!(
            guess_file("game_1.0_amd64.deb"),
            Some(("linux-deb", "installer"))
        );
        assert_eq!(
            guess_file("game-1.0.x86_64.rpm"),
            Some(("linux-rpm", "installer"))
        );
        assert_eq!(guess_file("Game-Setup.exe"), None);
        assert_eq!(guess_file("checksums.txt"), None);
    }

    #[test]
    fn next_release_from_the_last() {
        let template = vec![
            TemplateFile {
                platform: "windows".into(),
                kind: "archive".into(),
                file_name: "herds-1.2.0-windows.zip".into(),
                exec: Some("herds-1.2.0/herds.exe".into()),
                args: vec!["--fast".into()],
            },
            TemplateFile {
                platform: "linux".into(),
                kind: "archive".into(),
                file_name: "herds-linux.zip".into(),
                exec: Some("herds".into()),
                args: vec![],
            },
        ];
        let assets = vec![
            "herds-1.3.0-windows.zip".to_string(),
            "herds-linux.zip".to_string(),
        ];
        let files = files_from_template(&template, "1.2.0", "1.3.0", &assets, |n| format!("u/{n}"))
            .unwrap();
        assert_eq!(files[0].url, "u/herds-1.3.0-windows.zip");
        assert_eq!(files[0].exec.as_deref(), Some("herds-1.3.0/herds.exe"));
        assert_eq!(files[0].args, vec!["--fast".to_string()]);
        assert_eq!(files[1].url, "u/herds-linux.zip");

        let missing = files_from_template(&template, "1.2.0", "1.3.0", &assets[..1], |n| n.into());
        assert!(missing.unwrap_err().contains("herds-linux.zip"));
    }
}
