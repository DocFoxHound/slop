// SPDX-License-Identifier: AGPL-3.0-or-later
//! Updates for the Slop desktop app itself (SECURITY §7: self-updates are signed and verified).
//!
//! Production's Windows app is built on GitHub by the pipeline (.github/workflows/pipeline.yml),
//! after the owner approves the run's `production` environment, and uploaded here with the run's
//! GitHub OIDC token (no secrets in CI). Linux packages, and the test server's "Slop Dev" app for
//! every platform, are built on the server itself and published with `slop-api desktop publish`;
//! the test server takes no uploads from GitHub (deploy/README.md, "The desktop app"). The server
//! signs each build with the updater key, which never leaves the server, and answers the app's
//! update checks in the format Tauri's updater expects. The signature records the version, and
//! the app requires that it matches what it was offered, so an old build can't be passed off as
//! a new one.

use std::{io::Write, path::PathBuf};

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    AppState,
    error::{ApiError, ApiResult},
    slop_build::verify_github_token,
};

/// The largest installer accepted.
pub const MAX_BUILD_BYTES: usize = 300 * 1024 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/desktop/update/{target}/{arch}/{current}",
            get(check_update),
        )
        .route("/desktop/latest/{platform}", get(latest))
        .route("/desktop/files/{file_name}", get(download))
}

/// The upload route, kept apart so it can take a large body and a long time.
pub fn upload_routes() -> Router<AppState> {
    Router::new().route("/desktop/builds", post(upload))
}

/// The updater signing key: a base64-encoded, unencrypted minisign secret key box, as printed by
/// `slop-api keys generate-updater`.
pub fn updater_key_from_base64(value: &str) -> anyhow::Result<minisign::SecretKey> {
    let text = String::from_utf8(STANDARD.decode(value.trim())?)?;
    Ok(minisign::SecretKeyBox::from_string(&text)?.into_unencrypted_secret_key()?)
}

/// A new updater key pair: (SLOP_UPDATER_KEY, the public key for tauri.conf.json).
pub fn generate_updater_key() -> anyhow::Result<(String, String)> {
    let pair = minisign::KeyPair::generate_unencrypted_keypair()?;
    let secret = pair.sk.to_box(Some("Slop desktop updater"))?.to_string();
    let public = pair.pk.to_box()?.to_string();
    Ok((STANDARD.encode(secret), STANDARD.encode(public)))
}

/// Signs an installer the way Tauri's CLI does: base64 of a minisign signature box whose trusted
/// comment names the file and the version.
pub fn sign_build(
    key: &minisign::SecretKey,
    data: &[u8],
    file_name: &str,
    version: &str,
) -> anyhow::Result<String> {
    let trusted = format!(
        "timestamp:{}\tfile:{file_name}\tversion:{version}",
        OffsetDateTime::now_utc().unix_timestamp()
    );
    let sig = minisign::sign(None, key, data, Some(&trusted), Some("signature from Slop"))?;
    Ok(STANDARD.encode(sig.to_string()))
}

/// The published platform for an update check: Tauri's target, plus on Linux the package format
/// the app was installed from (`{{bundle_type}}`). AppImages aren't published.
fn platform_for_target(target: &str, bundle: Option<&str>) -> Option<&'static str> {
    match (target, bundle) {
        ("windows", _) => Some("windows"),
        ("darwin" | "macos", _) => Some("mac"),
        ("linux", Some("deb")) => Some("linux-deb"),
        ("linux", Some("rpm")) => Some("linux-rpm"),
        _ => None,
    }
}

fn file_name_for(platform: &str, version: &str) -> String {
    match platform {
        "windows" => format!("Slop_{version}_x64-setup.exe"),
        "mac" => format!("Slop_{version}.app.tar.gz"),
        "linux-deb" => format!("Slop_{version}_amd64.deb"),
        _ => format!("Slop-{version}-1.x86_64.rpm"),
    }
}

fn dir(state: &AppState) -> ApiResult<&PathBuf> {
    state
        .config
        .desktop
        .dir
        .as_ref()
        .ok_or(ApiError::Unavailable(
            "desktop updates aren't set up on this server",
        ))
}

// ---------------------------------------------------------------------------------------------
// Upload from CI

#[derive(Deserialize)]
struct UploadQuery {
    platform: String,
    version: String,
    commit: String,
}

#[derive(Serialize, Debug)]
pub struct Uploaded {
    pub version: String,
    pub file_name: String,
    pub sha256: String,
}

async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<UploadQuery>,
    body: Body,
) -> ApiResult<(StatusCode, Json<Uploaded>)> {
    let cfg = &state.config.desktop;
    let (Some(repo), Some(key)) = (cfg.repo.as_deref(), cfg.updater_key.as_ref()) else {
        return Err(ApiError::Unavailable(
            "desktop updates aren't set up on this server",
        ));
    };
    let Some(environment) = cfg.environment.as_deref() else {
        return Err(ApiError::Unavailable(
            "this server builds its own desktop apps and takes no uploads",
        ));
    };
    let dir = dir(&state)?.clone();
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(ApiError::Unauthorized)?;
    let claims = verify_github_token(&state, token.trim()).await?;
    if !uploader_allowed(&claims, repo, &cfg.workflow, &cfg.git_ref, environment) {
        return Err(ApiError::Forbidden);
    }
    if !claims.sha.eq_ignore_ascii_case(&q.commit) {
        return Err(ApiError::invalid(
            "commit",
            "doesn't match the commit this run built",
        ));
    }
    if !PLATFORMS.contains(&q.platform.as_str()) {
        return Err(ApiError::invalid("platform", "unknown platform"));
    }
    let version = semver::Version::parse(&q.version)
        .map_err(|_| ApiError::invalid("version", "use a semantic version like 0.1.42"))?;
    ensure_newer(&state.db, &q.platform, &version).await?;

    // Stream to a temporary file in the builds folder, then publish it.
    std::fs::create_dir_all(&dir).map_err(|e| ApiError::Internal(e.into()))?;
    let tmp = dir.join(format!(".upload-{}.part", uuid::Uuid::now_v7()));
    let mut out = std::fs::File::create(&tmp).map_err(|e| ApiError::Internal(e.into()))?;
    let mut size = 0usize;
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(ApiError::invalid(
                    "body",
                    format!("upload interrupted: {e}"),
                ));
            }
        };
        size += chunk.len();
        if size > MAX_BUILD_BYTES {
            let _ = std::fs::remove_file(&tmp);
            return Err(ApiError::invalid("body", "installer is too large"));
        }
        out.write_all(&chunk)
            .map_err(|e| ApiError::Internal(e.into()))?;
    }
    out.sync_all().map_err(|e| ApiError::Internal(e.into()))?;
    let published = publish(&state.db, &dir, key, &q.platform, &version, &q.commit, &tmp).await;
    let _ = std::fs::remove_file(&tmp);
    Ok((StatusCode::CREATED, Json(published?)))
}

/// Only Slop's own pipeline on main (SLOP_DESKTOP_WORKFLOW, SLOP_DESKTOP_REF) in Slop's own
/// repository, and only a job in the approved environment (SLOP_DESKTOP_ENVIRONMENT): a run
/// waiting for, or refused, the owner's approval never gets that far, and no other workflow,
/// branch or fork can update the players.
fn uploader_allowed(
    claims: &crate::slop_build::GithubClaims,
    repo: &str,
    workflow: &str,
    git_ref: &str,
    environment: &str,
) -> bool {
    let expected = format!("{repo}/.github/workflows/{workflow}@{git_ref}");
    claims.repository.eq_ignore_ascii_case(repo)
        && claims.job_workflow_ref.eq_ignore_ascii_case(&expected)
        // GitHub's environment names ignore case.
        && claims
            .environment
            .as_deref()
            .is_some_and(|e| e.eq_ignore_ascii_case(environment))
}

/// Platforms the desktop app is published for. Linux builds are per package format, because
/// the updater installs the same kind of package the app was installed from.
pub const PLATFORMS: [&str; 4] = ["windows", "mac", "linux-deb", "linux-rpm"];

/// Updates only ever go forward.
async fn ensure_newer(
    db: &sqlx::PgPool,
    platform: &str,
    version: &semver::Version,
) -> ApiResult<()> {
    let existing: Vec<String> =
        sqlx::query_scalar("select version from desktop_builds where platform = $1")
            .bind(platform)
            .fetch_all(db)
            .await?;
    if existing
        .iter()
        .filter_map(|v| semver::Version::parse(v).ok())
        .any(|v| &v >= version)
    {
        return Err(ApiError::Conflict(format!(
            "version {version} isn't newer than what's published"
        )));
    }
    Ok(())
}

/// Signs and publishes a desktop build: copies `file` into `dir` under its published name and
/// records it. Shared by CI uploads and `slop-api desktop publish` (builds made on this server).
pub async fn publish(
    db: &sqlx::PgPool,
    dir: &std::path::Path,
    key: &minisign::SecretKey,
    platform: &str,
    version: &semver::Version,
    commit: &str,
    file: &std::path::Path,
) -> ApiResult<Uploaded> {
    if !PLATFORMS.contains(&platform) {
        return Err(ApiError::invalid("platform", "unknown platform"));
    }
    if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError::invalid("commit", "use the full commit SHA"));
    }
    ensure_newer(db, platform, version).await?;
    let data = std::fs::read(file).map_err(|e| ApiError::Internal(e.into()))?;
    if data.is_empty() {
        return Err(ApiError::invalid("body", "empty upload"));
    }
    let version = version.to_string();
    let file_name = file_name_for(platform, &version);
    let sha256 = hex::encode(Sha256::digest(&data));
    let signature = sign_build(key, &data, &file_name, &version).map_err(ApiError::Internal)?;
    std::fs::create_dir_all(dir).map_err(|e| ApiError::Internal(e.into()))?;
    let staged = dir.join(format!(".{file_name}.part"));
    std::fs::write(&staged, &data).map_err(|e| ApiError::Internal(e.into()))?;
    std::fs::rename(&staged, dir.join(&file_name)).map_err(|e| ApiError::Internal(e.into()))?;
    sqlx::query(
        "insert into desktop_builds (platform, version, file_name, sha256, size_bytes, signature, commit_sha)
         values ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(platform)
    .bind(&version)
    .bind(&file_name)
    .bind(&sha256)
    .bind(data.len() as i64)
    .bind(&signature)
    .bind(commit.to_ascii_lowercase())
    .execute(db)
    .await?;
    tracing::info!(%platform, %version, "desktop build published");
    Ok(Uploaded {
        version,
        file_name,
        sha256,
    })
}

// ---------------------------------------------------------------------------------------------
// Update checks and downloads

#[derive(sqlx::FromRow)]
struct Build {
    version: String,
    file_name: String,
    signature: String,
    commit_sha: String,
    created_at: OffsetDateTime,
}

async fn newest(state: &AppState, platform: &str) -> ApiResult<Option<Build>> {
    let builds: Vec<Build> = sqlx::query_as(
        "select version, file_name, signature, commit_sha, created_at from desktop_builds where platform = $1",
    )
    .bind(platform)
    .fetch_all(&state.db)
    .await?;
    Ok(builds
        .into_iter()
        .filter_map(|b| semver::Version::parse(&b.version).ok().map(|v| (v, b)))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, b)| b))
}

fn file_url(state: &AppState, file_name: &str) -> String {
    format!(
        "{}/v1/desktop/files/{file_name}",
        state.config.public_url.as_str().trim_end_matches('/')
    )
}

/// Tauri's dynamic update check: 204 when up to date, otherwise the newer build.
#[derive(Deserialize)]
struct UpdateQuery {
    bundle: Option<String>,
}

async fn check_update(
    State(state): State<AppState>,
    Path((target, arch, current)): Path<(String, String, String)>,
    Query(q): Query<UpdateQuery>,
) -> ApiResult<Response> {
    let Some(platform) = platform_for_target(&target, q.bundle.as_deref()) else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    if arch != "x86_64" {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let current = semver::Version::parse(current.trim_start_matches('v'))
        .map_err(|_| ApiError::invalid("version", "not a version"))?;
    let Some(build) = newest(&state, platform).await? else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    let newest =
        semver::Version::parse(&build.version).map_err(|e| ApiError::Internal(e.into()))?;
    if newest <= current {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    Ok(Json(serde_json::json!({
        "version": build.version,
        "notes": format!("Built from {}", &build.commit_sha[..build.commit_sha.len().min(12)]),
        "pub_date": build.created_at.format(&Rfc3339).map_err(|e| ApiError::Internal(e.into()))?,
        "url": file_url(&state, &build.file_name),
        "signature": build.signature,
    }))
    .into_response())
}

#[derive(Serialize)]
struct Latest {
    version: String,
    url: String,
}

/// The newest build, for the website's download button.
async fn latest(
    State(state): State<AppState>,
    Path(platform): Path<String>,
) -> ApiResult<Json<Latest>> {
    let build = newest(&state, &platform).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(Latest {
        url: file_url(&state, &build.file_name),
        version: build.version,
    }))
}

async fn download(
    State(state): State<AppState>,
    Path(file_name): Path<String>,
) -> ApiResult<Response> {
    let known: Option<String> =
        sqlx::query_scalar("select file_name from desktop_builds where file_name = $1")
            .bind(&file_name)
            .fetch_optional(&state.db)
            .await?;
    let file_name = known.ok_or(ApiError::NotFound)?;
    let path = dir(&state)?.join(&file_name);
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| ApiError::NotFound)?;
    let len = file
        .metadata()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?
        .len();
    let body = Body::from_stream(tokio_util::io::ReaderStream::new(file));
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (header::CONTENT_LENGTH, len.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{file_name}\""),
            ),
            (
                header::CACHE_CONTROL,
                "public, max-age=31536000, immutable".to_string(),
            ),
        ],
        body,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_approved_pipeline_on_main_may_upload() {
        let claims = |repo: &str, workflow_ref: &str, environment: Option<&str>| {
            crate::slop_build::GithubClaims {
                repository: repo.into(),
                sha: "1".repeat(40),
                job_workflow_ref: workflow_ref.into(),
                repository_id: None,
                repository_owner_id: None,
                environment: environment.map(Into::into),
            }
        };
        let allowed = |c: &crate::slop_build::GithubClaims| {
            uploader_allowed(
                c,
                "octo/slop",
                "pipeline.yml",
                "refs/heads/main",
                "production",
            )
        };
        let ok = "octo/slop/.github/workflows/pipeline.yml@refs/heads/main";
        assert!(allowed(&claims("octo/slop", ok, Some("production"))));
        assert!(allowed(&claims("Octo/Slop", ok, Some("Production"))));
        for (repo, workflow_ref, environment) in [
            // Not approved: no environment, or another one.
            ("octo/slop", ok, None),
            ("octo/slop", ok, Some("dev")),
            ("octo/slop", ok, Some("production-old")),
            // Another repository, workflow, branch or tag.
            (
                "mallory/slop",
                "mallory/slop/.github/workflows/pipeline.yml@refs/heads/main",
                Some("production"),
            ),
            (
                "octo/slop",
                "mallory/slop/.github/workflows/pipeline.yml@refs/heads/main",
                Some("production"),
            ),
            (
                "octo/slop",
                "octo/slop/.github/workflows/desktop.yml@refs/heads/main",
                Some("production"),
            ),
            (
                "octo/slop",
                "octo/slop/.github/workflows/pipeline.yml@refs/heads/dev",
                Some("production"),
            ),
            (
                "octo/slop",
                "octo/slop/.github/workflows/pipeline.yml@refs/tags/production",
                Some("production"),
            ),
            (
                "octo/slop",
                "octo/slop/.github/workflows/pipeline.yml@refs/pull/7/merge",
                Some("production"),
            ),
        ] {
            assert!(
                !allowed(&claims(repo, workflow_ref, environment)),
                "{repo} {workflow_ref} {environment:?}"
            );
        }
    }

    /// The signature must verify exactly the way Tauri's updater checks it (tauri-plugin-updater
    /// `verify_signature`): base64-decode, parse with minisign-verify, verify, then read the
    /// version from the trusted comment.
    #[test]
    fn signatures_verify_like_tauri() {
        let (secret, public) = generate_updater_key().unwrap();
        let key = updater_key_from_base64(&secret).unwrap();
        let data = b"pretend installer";
        let sig = sign_build(&key, data, "Slop_0.1.7_x64-setup.exe", "0.1.7").unwrap();

        let pk_text = String::from_utf8(STANDARD.decode(&public).unwrap()).unwrap();
        let pk = minisign_verify::PublicKey::decode(&pk_text).unwrap();
        let sig_text = String::from_utf8(STANDARD.decode(&sig).unwrap()).unwrap();
        let signature = minisign_verify::Signature::decode(&sig_text).unwrap();
        pk.verify(data, &signature, true).unwrap();
        let version = signature
            .trusted_comment()
            .split('\t')
            .find_map(|f| f.strip_prefix("version:"))
            .unwrap();
        assert_eq!(version, "0.1.7");
        assert!(pk.verify(b"something else", &signature, true).is_err());
    }
}
