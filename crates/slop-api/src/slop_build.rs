// SPDX-License-Identifier: AGPL-3.0-or-later
//! Slop Build (docs/ARCHITECTURE.md §6.2): the shared recipe in `slop-build/` builds a maker's app
//! on GitHub Actions, signs it, attaches it to their release, and calls
//! `POST /v1/slop-build/releases` here.
//!
//! The call carries the run's GitHub OIDC token instead of any secret. The token is signed by
//! GitHub and says which repository, commit and workflow the run is; Slop accepts it only from
//! the Slop Build recipe, for the app's own repository, at the commit being released. The release
//! then goes through the same fetch, signature and review steps as one a maker submits by hand.

use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    routing::{get, post},
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, jwk::JwkSet};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
    releases::{self, NewFile, ReleaseInfo},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/slop-build/releases", post(submit_from_ci))
        .route("/slop-build/checks", post(submit_checks))
        .route("/apps/{slug}/slop-build", get(setup))
}

/// GitHub's OIDC signing keys, cached.
#[derive(Default)]
pub struct OidcKeys {
    keys: Option<(Instant, JwkSet)>,
}

/// Re-fetch the key set at most this often when a token names an unknown key.
const KEY_REFRESH: Duration = Duration::from_secs(60);

fn recipe(state: &AppState) -> ApiResult<&str> {
    state
        .config
        .releases
        .slop_build_workflow
        .as_deref()
        .ok_or(ApiError::Unavailable(
            "Slop Build isn't set up on this server",
        ))
}

// ---------------------------------------------------------------------------------------------
// Releases submitted by the recipe

/// The GitHub Actions OIDC claims Slop relies on.
#[derive(Debug, Deserialize)]
pub(crate) struct GithubClaims {
    /// `owner/repo`.
    pub repository: String,
    /// The commit the run is building.
    pub sha: String,
    /// The reusable workflow running, e.g. `org/slop-build/.github/workflows/build.yml@refs/tags/v1`.
    pub job_workflow_ref: String,
}

async fn signing_keys(state: &AppState, kid: &str) -> ApiResult<DecodingKey> {
    let issuer = &state.config.releases.github_oidc_issuer;
    {
        let cache = state.oidc_keys.read().await;
        if let Some((fetched, set)) = &cache.keys {
            if let Some(jwk) = set.find(kid) {
                return DecodingKey::from_jwk(jwk).map_err(|e| ApiError::Internal(e.into()));
            }
            if fetched.elapsed() < KEY_REFRESH {
                return Err(ApiError::Unauthorized);
            }
        }
    }
    let url = format!("{}/.well-known/jwks", issuer.trim_end_matches('/'));
    let set: JwkSet = state
        .http
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("fetching GitHub's OIDC keys: {e}")))?
        .json()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("GitHub's OIDC keys: {e}")))?;
    let key = set.find(kid).map(DecodingKey::from_jwk);
    state.oidc_keys.write().await.keys = Some((Instant::now(), set));
    match key {
        Some(Ok(key)) => Ok(key),
        Some(Err(e)) => Err(ApiError::Internal(e.into())),
        None => Err(ApiError::Unauthorized),
    }
}

/// Checks a GitHub Actions OIDC token: signature, issuer, audience (this Slop site) and expiry.
pub(crate) async fn verify_github_token(state: &AppState, token: &str) -> ApiResult<GithubClaims> {
    let header = jsonwebtoken::decode_header(token).map_err(|_| ApiError::Unauthorized)?;
    if header.alg != Algorithm::RS256 {
        return Err(ApiError::Unauthorized);
    }
    let kid = header.kid.ok_or(ApiError::Unauthorized)?;
    let key = signing_keys(state, &kid).await?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[state
        .config
        .releases
        .github_oidc_issuer
        .trim_end_matches('/')]);
    validation.set_audience(&[state.config.public_url.as_str().trim_end_matches('/')]);
    validation.validate_nbf = true;
    validation.leeway = 30;
    jsonwebtoken::decode::<GithubClaims>(token, &key, &validation)
        .map(|data| data.claims)
        .map_err(|e| {
            tracing::warn!(error = %e, "rejected a Slop Build token");
            ApiError::Unauthorized
        })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CiFile {
    platform: String,
    file_name: String,
    /// The program to start, for zips and .tar.gz; packages (.deb, .rpm) have none.
    #[serde(default)]
    exec: Option<String>,
    /// Arguments to start it with (from the maker's slop.toml).
    #[serde(default)]
    args: Vec<String>,
    /// Over GitHub's 2 GiB limit: the parts' file names, in order (`<file_name>.001`…).
    #[serde(default)]
    parts: Vec<String>,
    /// Windows builds: Microsoft Defender's check of the file, on GitHub's Windows runner.
    #[serde(default)]
    defender: Option<crate::scan::DefenderCheck>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CiRelease {
    app: String,
    version: String,
    tag: String,
    commit: String,
    files: Vec<CiFile>,
    /// The source checks (gitleaks, OSV-Scanner): possible secrets by file and line (never their
    /// values), and dependencies with known vulnerabilities.
    #[serde(default)]
    source_checks: Option<serde_json::Value>,
    /// Shared runtimes from slop.toml's `[runtimes]` (slop-manifest runtimes.rs).
    #[serde(default)]
    runtimes: Option<Vec<String>>,
}

/// Everything but URL-unreserved characters (RFC 3986) is escaped in a path segment.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

pub(crate) fn path_segment(s: &str) -> String {
    utf8_percent_encode(s, SEGMENT).to_string()
}

async fn submit_from_ci(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CiRelease>,
) -> ApiResult<(StatusCode, Json<ReleaseInfo>)> {
    let recipe = recipe(&state)?.to_string();
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(ApiError::Unauthorized)?;
    let claims = verify_github_token(&state, token.trim()).await?;

    // Only the Slop Build recipe can submit this way.
    if !claims.job_workflow_ref.starts_with(&format!("{recipe}@")) {
        return Err(ApiError::Forbidden);
    }
    // The release must be the commit this run built.
    if !claims.sha.eq_ignore_ascii_case(&req.commit) {
        return Err(ApiError::invalid(
            "commit",
            "doesn't match the commit this run built",
        ));
    }

    let app: Option<(Uuid, Uuid, String, Vec<String>)> =
        sqlx::query_as("select id, maker_id, repo_url, platforms from apps where slug = $1")
            .bind(&req.app)
            .fetch_optional(&state.db)
            .await?;
    let (app_id, maker_id, repo_url, platforms) = app.ok_or(ApiError::NotFound)?;
    // The run must be in the app's own repository.
    let repo_path = slop_provenance::github_repo_path(&repo_url);
    if !repo_path.is_some_and(|p| p.eq_ignore_ascii_case(&claims.repository)) {
        return Err(ApiError::Forbidden);
    }

    let tag_ok = (1..=100).contains(&req.tag.len())
        && req
            .tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-+".contains(c));
    if !tag_ok {
        return Err(ApiError::invalid("tag", "unexpected release tag"));
    }
    // Download URLs are built here from the app's repository, never taken from the request.
    let base = repo_url.trim_end_matches('/');
    let mut checks = Vec::new();
    for f in &req.files {
        if let Some(d) = &f.defender {
            d.validate().map_err(|m| ApiError::invalid("defender", m))?;
            checks.push(d.clone());
        }
    }
    let files = req
        .files
        .into_iter()
        // Slop Build makes every Linux package by default; keep the ones the store page lists.
        .filter(|f| !f.platform.starts_with("linux-") || platforms.contains(&f.platform))
        .map(|f| NewFile {
            url: format!(
                "{base}/releases/download/{}/{}",
                path_segment(&req.tag),
                path_segment(&f.file_name)
            ),
            kind: if f.platform.starts_with("linux-") {
                "installer".into()
            } else {
                "archive".into()
            },
            parts: f
                .parts
                .iter()
                .map(|p| {
                    format!(
                        "{base}/releases/download/{}/{}",
                        path_segment(&req.tag),
                        path_segment(p)
                    )
                })
                .collect(),
            platform: f.platform,
            exec: f.exec,
            args: f.args,
        })
        .collect();
    let release = releases::create_release(
        &state,
        app_id,
        &repo_url,
        maker_id,
        &req.version,
        &req.commit.to_ascii_lowercase(),
        files,
        "slop_build",
        None,
        req.runtimes.clone(),
    )
    .await?;
    if let Some(checks) = &req.source_checks {
        if checks.to_string().len() > 256 * 1024 {
            return Err(ApiError::invalid("source_checks", "too large"));
        }
        sqlx::query("update releases set source_checks = $2 where id = $1")
            .bind(release.id)
            .bind(sqlx::types::Json(checks))
            .execute(&state.db)
            .await?;
    }
    // Defender's checks, by file hash: matched to the files once Slop has fetched them.
    for check in &checks {
        record_defender(&state, app_id, check).await?;
    }
    tracing::info!(app = %req.app, version = %req.version, run_repo = %claims.repository, "Slop Build submitted a release");
    Ok((StatusCode::CREATED, Json(release)))
}

/// Keeps a Defender check and attaches it to any Windows file of the app with that hash.
async fn record_defender(
    state: &AppState,
    app_id: Uuid,
    check: &crate::scan::DefenderCheck,
) -> ApiResult<u64> {
    let report = sqlx::types::Json(check);
    let sha = check.sha256.to_ascii_lowercase();
    sqlx::query(
        "insert into defender_checks (app_id, sha256, report) values ($1, $2, $3)
         on conflict (app_id, sha256) do update set report = excluded.report, created_at = now()",
    )
    .bind(app_id)
    .bind(&sha)
    .bind(report)
    .execute(&state.db)
    .await?;
    Ok(sqlx::query(
        "update release_files f set defender = $3 from releases r
         where r.id = f.release_id and r.app_id = $1 and f.sha256 = $2 and f.platform = 'windows'",
    )
    .bind(app_id)
    .bind(&sha)
    .bind(report)
    .execute(&state.db)
    .await?
    .rows_affected())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CiChecks {
    app: String,
    files: Vec<crate::scan::DefenderCheck>,
}

#[derive(Serialize)]
struct ChecksRecorded {
    /// Release files the checks were attached to now (others attach when Slop fetches them).
    matched: u64,
}

/// Slop Check (`check.yml` beside the recipe): Microsoft Defender's checks of Windows builds
/// made outside Slop Build, from the maker's own repository's GitHub Actions.
async fn submit_checks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CiChecks>,
) -> ApiResult<Json<ChecksRecorded>> {
    let recipe = recipe(&state)?;
    let check_workflow = match recipe.rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/check.yml"),
        None => {
            return Err(ApiError::Unavailable(
                "Slop Check isn't set up on this server",
            ));
        }
    };
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(ApiError::Unauthorized)?;
    let claims = verify_github_token(&state, token.trim()).await?;
    if !claims
        .job_workflow_ref
        .starts_with(&format!("{check_workflow}@"))
        && !claims.job_workflow_ref.starts_with(&format!("{recipe}@"))
    {
        return Err(ApiError::Forbidden);
    }
    let app: Option<(Uuid, String)> =
        sqlx::query_as("select id, repo_url from apps where slug = $1")
            .bind(&req.app)
            .fetch_optional(&state.db)
            .await?;
    let (app_id, repo_url) = app.ok_or(ApiError::NotFound)?;
    let repo_path = slop_provenance::github_repo_path(&repo_url);
    if !repo_path.is_some_and(|p| p.eq_ignore_ascii_case(&claims.repository)) {
        return Err(ApiError::Forbidden);
    }
    if req.files.is_empty() || req.files.len() > 20 {
        return Err(ApiError::invalid("files", "1 to 20 files"));
    }
    let mut matched = 0;
    for check in &req.files {
        check
            .validate()
            .map_err(|m| ApiError::invalid("files", m))?;
        matched += record_defender(&state, app_id, check).await?;
    }
    Ok(Json(ChecksRecorded { matched }))
}

// ---------------------------------------------------------------------------------------------
// Setting it up: "Add Slop Build"

#[derive(Serialize)]
struct Setup {
    /// Whether Slop Build can build this repository today.
    supported: bool,
    /// rust, go or custom.
    project_type: String,
    /// What the repository looks like, when it's something Slop Build can't build on its own yet.
    detected: Option<String>,
    note: Option<String>,
    file_path: &'static str,
    /// The workflow file to commit.
    workflow: String,
    /// GitHub's "create file" page with the workflow filled in. The maker only clicks Commit.
    create_url: Option<String>,
    /// Slop Check, for makers who build elsewhere: Microsoft Defender checks their Windows files.
    check_file_path: &'static str,
    check_workflow: String,
    check_create_url: Option<String>,
    /// "Build now": the workflow's page on GitHub, whose Run workflow button starts a build.
    build_now_url: Option<String>,
}

const WORKFLOW_PATH: &str = ".github/workflows/slop.yml";
const CHECK_PATH: &str = ".github/workflows/slop-check.yml";

/// Slop Check's workflow file: it sits next to the build recipe in the same repository.
fn check_yaml(recipe: &str, version: &str, slug: &str, site: &str) -> String {
    let dir = recipe.rsplit_once('/').map_or(recipe, |(d, _)| d);
    format!(
        "# Slop Check: when you publish a release, Microsoft Defender checks its Windows files\n\
         # and tells Slop. Docs: {site}/publish/guide#slop-check\n\
         name: Slop Check\n\
         on:\n  \
           release:\n    \
             types: [published]\n  \
           workflow_dispatch:\n    \
             inputs:\n      \
               tag:\n        \
                 description: The release tag to check\n\
         jobs:\n  \
           check:\n    \
             uses: {dir}/check.yml@{version}\n    \
             permissions:\n      \
               contents: read\n      \
               id-token: write\n    \
             with:\n      \
               tag: ${{{{ inputs.tag }}}}\n      \
               app: {slug}\n      \
               slop-url: {site}\n"
    )
}

/// GitHub's "create file" page with `value` filled in.
fn create_file_url(repo_path: &str, branch: &str, file: &str, value: &str) -> ApiResult<String> {
    let mut url = Url::parse(&format!(
        "https://github.com/{repo_path}/new/{}",
        path_segment(branch)
    ))
    .map_err(|e| ApiError::Internal(e.into()))?;
    url.query_pairs_mut()
        .append_pair("filename", file)
        .append_pair("value", value);
    Ok(url.into())
}

fn workflow_yaml(
    recipe: &str,
    version: &str,
    slug: &str,
    site: &str,
    project_type: &str,
    branch: &str,
) -> String {
    let custom = match project_type {
        "custom" => {
            "\n      # Fill these in: how to build, where the build ends up, and the program to start\n      \
             # (\"{ext}\" becomes .exe on Windows).\n      \
             build-command: \"make release\"\n      \
             output-dir: \"dist\"\n      \
             exec: \"my-app{ext}\"\n"
        }
        "godot" => {
            "\n      # Your Godot version (standard, not .NET). Exports use the presets named\n      \
             # \"Windows Desktop\" and \"Linux\" in export_presets.cfg (commit that file).\n      \
             godot-version: \"4.3\"\n"
        }
        "web" => {
            "\n      # Your site goes in a small window of its own (a Tauri shell). How it's built, and\n      \
             # the folder with its index.html. For a plain site, remove build-command and use \".\".\n      \
             build-command: \"npm run build\"\n      \
             output-dir: \"dist\"\n"
        }
        "godot-dotnet" => {
            "\n      # Your Godot version; a C# project uses Godot's .NET build. Exports use the presets\n      \
             # named \"Windows Desktop\" and \"Linux\" in export_presets.cfg (commit that file).\n      \
             godot-version: \"4.3\"\n      \
             godot-dotnet: true\n"
        }
        "dotnet" => {
            "\n      # Published self-contained, so players don't need .NET. If the repository has more\n      \
             # than one program project, name the one to ship:\n      \
             # dotnet-project: \"src/MyGame/MyGame.csproj\"\n"
        }
        "unity" => {
            "\n      # Built with GameCI. Add your Unity licence to this repository's secrets (Settings →\n      \
             # Secrets and variables → Actions): UNITY_LICENSE (your .ulf file's contents),\n      \
             # UNITY_EMAIL and UNITY_PASSWORD. See {site}/publish/guide#unity\n"
        }
        "python" => {
            "\n      # The script that starts your app (built with PyInstaller). Uncomment windowed for\n      \
             # apps with their own window, such as pygame games.\n      \
             python-entry: \"main.py\"\n      \
             # python-windowed: true\n"
        }
        _ => "\n",
    };
    let custom = custom.replace("{site}", site);
    // godot-dotnet is godot with C#: the recipe's type is still godot.
    let recipe_type = if project_type == "godot-dotnet" {
        "godot"
    } else {
        project_type
    };
    // Unity needs the maker's licence secrets passed on (SignPath's stay optional).
    let secrets = if project_type == "unity" {
        "    secrets:\n      \
           UNITY_LICENSE: ${{ secrets.UNITY_LICENSE }}\n      \
           UNITY_EMAIL: ${{ secrets.UNITY_EMAIL }}\n      \
           UNITY_PASSWORD: ${{ secrets.UNITY_PASSWORD }}\n      \
           # SIGNPATH_API_TOKEN: ${{ secrets.SIGNPATH_API_TOKEN }}\n      \
           # BUTLER_API_KEY: ${{ secrets.BUTLER_API_KEY }}\n"
    } else {
        "    # secrets:\n    \
         #   SIGNPATH_API_TOKEN: ${{ secrets.SIGNPATH_API_TOKEN }}\n    \
         #   BUTLER_API_KEY: ${{ secrets.BUTLER_API_KEY }}\n"
    };
    format!(
        "# Slop Build: builds, signs and attaches your app to a release, then sends it to Slop.\n\
         # Docs: {site}/publish\n\
         name: Slop Build\n\
         on:\n  \
           push:\n    \
             branches: [{branch}]\n    \
             tags: [\"v*\"]\n  \
           workflow_dispatch:\n\
         \n\
         permissions:\n  \
           contents: write\n  \
           id-token: write\n  \
           attestations: write\n\
         \n\
         jobs:\n  \
           slop:\n    \
             uses: {recipe}@{version}\n    \
             with:\n      \
               app: {slug}\n      \
               slop-url: {site}\n      \
               project-type: {recipe_type}{custom}      \
               # Recommended for Windows: sign with a free SignPath Foundation certificate for\n      \
               # open-source projects (apply at signpath.org; see {site}/publish/guide#signing),\n      \
               # then fill in your organization ID and add the SIGNPATH_API_TOKEN secret.\n      \
               # signpath-organization-id: \"your-organization-id\"\n      \
               # Also publish to your itch.io page (user/game): add the BUTLER_API_KEY secret.\n      \
               # itch-target: \"you/your-game\"\n\
         {secrets}"
    )
}

/// What a repository looks like from its top-level file names (and, when it has one, whether
/// its package.json depends on Electron). Returns the project type and, for a guess the maker
/// should check, what it looks like.
fn detect(names: &[String], electron: bool) -> (&'static str, Option<&'static str>) {
    let has = |n: &str| names.iter().any(|x| x == n);
    let dotnet = names
        .iter()
        .any(|n| n.ends_with(".csproj") || n.ends_with(".sln"));
    if has("Assets") && has("ProjectSettings") {
        ("unity", None)
    } else if has("project.godot") && dotnet {
        ("godot-dotnet", None)
    } else if has("Cargo.toml") && !has("src-tauri") {
        ("rust", None)
    } else if has("go.mod") {
        ("go", None)
    } else if has("project.godot") {
        ("godot", None)
    } else if has("src-tauri") {
        ("tauri", None)
    } else if has("package.json") && electron {
        ("electron", None)
    } else if has("package.json") {
        ("web", Some("web"))
    } else if dotnet {
        ("dotnet", None)
    } else if has("pyproject.toml") || has("requirements.txt") || has("main.py") {
        ("python", Some("python"))
    } else if has("index.html") {
        ("web", Some("web"))
    } else {
        ("custom", None)
    }
}

async fn setup(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Setup>> {
    let recipe = recipe(&state)?.to_string();
    let row: Option<(Uuid, String)> =
        sqlx::query_as("select maker_id, repo_url from apps where slug = $1")
            .bind(&slug)
            .fetch_optional(&state.db)
            .await?;
    let repo_url = match row {
        Some((maker, repo)) if maker == user.id => repo,
        _ => return Err(ApiError::NotFound),
    };
    let site = state
        .config
        .public_url
        .as_str()
        .trim_end_matches('/')
        .to_string();
    let version = &state.config.releases.slop_build_version;

    let Some(repo_path) = slop_provenance::github_repo_path(&repo_url) else {
        return Ok(Json(Setup {
            supported: false,
            project_type: "custom".into(),
            detected: None,
            note: Some(
                "Slop Build runs on GitHub Actions, so it needs a GitHub repository.".into(),
            ),
            file_path: WORKFLOW_PATH,
            workflow: workflow_yaml(&recipe, version, &slug, &site, "custom", "main"),
            create_url: None,
            check_file_path: CHECK_PATH,
            check_workflow: check_yaml(&recipe, version, &slug, &site),
            check_create_url: None,
            build_now_url: None,
        }));
    };

    #[derive(Deserialize)]
    struct Repo {
        default_branch: String,
    }
    #[derive(Deserialize)]
    struct Entry {
        name: String,
    }
    let repo: Repo = github_json(&state, &format!("repos/{repo_path}"), &repo_url).await?;
    let entries: Vec<Entry> =
        github_json(&state, &format!("repos/{repo_path}/contents/"), &repo_url).await?;
    let names: Vec<String> = entries.into_iter().map(|e| e.name).collect();
    // Electron or a website: package.json says.
    let electron = if names.iter().any(|n| n == "package.json") {
        #[derive(Deserialize)]
        struct File {
            content: String,
        }
        let file: Option<File> = github_json(
            &state,
            &format!("repos/{repo_path}/contents/package.json"),
            &repo_url,
        )
        .await
        .ok();
        file.and_then(|f| {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(f.content.replace('\n', ""))
                .ok()
        })
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .is_some_and(|pkg| {
            ["dependencies", "devDependencies"]
                .iter()
                .any(|k| pkg[k].get("electron").is_some())
        })
    } else {
        false
    };
    let (project_type, detected) = detect(&names, electron);
    let note = detected.map(|kind| match kind {
        "web" => "This looks like a website: check build-command and output-dir in the workflow \
                  match how your site is built and where its index.html ends up."
            .to_string(),
        "python" => "This looks like a Python app: check python-entry names the script that \
                     starts it."
            .to_string(),
        other => format!("This looks like a {other} project: check the workflow's settings."),
    });

    let workflow = workflow_yaml(
        &recipe,
        version,
        &slug,
        &site,
        project_type,
        &repo.default_branch,
    );
    let create = create_file_url(&repo_path, &repo.default_branch, WORKFLOW_PATH, &workflow)?;
    let check_workflow = check_yaml(&recipe, version, &slug, &site);
    let check_create = create_file_url(
        &repo_path,
        &repo.default_branch,
        CHECK_PATH,
        &check_workflow,
    )?;
    Ok(Json(Setup {
        supported: true,
        project_type: project_type.into(),
        detected: detected.map(String::from),
        note,
        file_path: WORKFLOW_PATH,
        workflow,
        create_url: Some(create),
        check_file_path: CHECK_PATH,
        check_workflow,
        check_create_url: Some(check_create),
        build_now_url: Some(format!(
            "https://github.com/{repo_path}/actions/workflows/{}",
            WORKFLOW_PATH.rsplit('/').next().unwrap_or("slop.yml")
        )),
    }))
}

/// GETs JSON from the GitHub API.
async fn github_json<T: serde::de::DeserializeOwned>(
    state: &AppState,
    path: &str,
    repo_url: &str,
) -> ApiResult<T> {
    let url = state
        .config
        .releases
        .github_api_base
        .join(path)
        .map_err(|e| ApiError::Internal(e.into()))?;
    let res = releases::github_get(state, url)
        .await
        .send()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    if !res.status().is_success() {
        return Err(ApiError::Conflict(format!(
            "GitHub answered {} for {repo_url}; is the repository public?",
            res.status().as_u16()
        )));
    }
    res.json().await.map_err(|e| ApiError::Internal(e.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    /// Prints sample workflows, for linting with actionlint:
    /// `cargo test -p slop-api print_sample_workflows -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print_sample_workflows() {
        for kind in [
            "rust",
            "custom",
            "godot",
            "godot-dotnet",
            "dotnet",
            "unity",
            "web",
            "python",
            "electron",
            "tauri",
        ] {
            println!("=== {kind}");
            println!(
                "{}",
                workflow_yaml(
                    "org/slop-build/.github/workflows/build.yml",
                    "v1",
                    "my-game",
                    "https://slop.example",
                    kind,
                    "main"
                )
            );
        }
    }

    #[test]
    fn detection() {
        let d = |n: &[&str]| detect(&names(n), false);
        assert_eq!(d(&["Cargo.toml", "src"]), ("rust", None));
        assert_eq!(d(&["go.mod", "main.go"]), ("go", None));
        assert_eq!(d(&["project.godot"]), ("godot", None));
        assert_eq!(
            d(&["package.json", "src-tauri", "Cargo.toml"]),
            ("tauri", None)
        );
        assert_eq!(
            detect(&names(&["package.json", "main.js"]), true),
            ("electron", None)
        );
        assert_eq!(d(&["package.json", "vite.config.ts"]), ("web", Some("web")));
        assert_eq!(d(&["index.html"]), ("web", Some("web")));
        assert_eq!(
            d(&["main.py", "requirements.txt"]),
            ("python", Some("python"))
        );
        assert_eq!(
            d(&["Assets", "ProjectSettings", "Packages"]),
            ("unity", None)
        );
        assert_eq!(d(&["project.godot", "Game.csproj"]), ("godot-dotnet", None));
        assert_eq!(d(&["Game.sln", "src"]), ("dotnet", None));
        assert_eq!(d(&["README.md"]), ("custom", None));
    }

    #[test]
    fn workflows_for_each_type_carry_their_settings() {
        let y = |t: &str| {
            workflow_yaml(
                "org/x/.github/workflows/build.yml",
                "v1",
                "a",
                "https://s",
                t,
                "main",
            )
        };
        assert!(y("godot").contains("godot-version: \"4.3\""));
        // The optional extras are there, commented out, for every type.
        for t in ["rust", "unity"] {
            assert!(y(t).contains("# itch-target: \"you/your-game\""), "{t}");
            assert!(
                y(t).contains("BUTLER_API_KEY: ${{ secrets.BUTLER_API_KEY }}"),
                "{t}"
            );
        }
        assert!(y("web").contains("output-dir: \"dist\""));
        assert!(y("python").contains("python-entry: \"main.py\""));
        assert!(y("tauri").contains("project-type: tauri\n"));
        let g = y("godot-dotnet");
        assert!(g.contains("project-type: godot\n") && g.contains("godot-dotnet: true"));
        let u = y("unity");
        assert!(
            u.contains("UNITY_LICENSE: ${{ secrets.UNITY_LICENSE }}")
                && u.contains("https://s/publish/guide#unity")
        );
        assert!(y("dotnet").contains("project-type: dotnet\n"));
    }

    #[test]
    fn the_workflow_is_valid_yaml_with_the_right_fields() {
        let y = workflow_yaml(
            "org/slop-build/.github/workflows/build.yml",
            "v1",
            "my-game",
            "https://slop.example",
            "rust",
            "main",
        );
        assert!(y.contains("uses: org/slop-build/.github/workflows/build.yml@v1"));
        assert!(y.contains("app: my-game"));
        assert!(y.contains("project-type: rust"));
        assert!(y.contains("id-token: write"));
        let custom = workflow_yaml(
            "org/x/.github/workflows/build.yml",
            "v1",
            "a-b",
            "https://s",
            "custom",
            "trunk",
        );
        assert!(custom.contains("build-command:") && custom.contains("branches: [trunk]"));
    }

    #[test]
    fn the_check_workflow_uses_the_recipe_folder() {
        let y = check_yaml(
            "org/slop-build/.github/workflows/build.yml",
            "v1",
            "my-game",
            "https://slop.example",
        );
        assert!(y.contains("uses: org/slop-build/.github/workflows/check.yml@v1"));
        assert!(y.contains("tag: ${{ inputs.tag }}"));
        assert!(y.contains("id-token: write") && y.contains("app: my-game"));
    }
}
