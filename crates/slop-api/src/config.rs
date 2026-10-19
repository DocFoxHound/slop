//! Runtime configuration, read from environment variables (see `.env.example`).

use std::{env, fmt, net::SocketAddr};

use anyhow::{Context, bail};
use url::Url;

#[derive(Clone, Debug)]
pub struct Config {
    pub bind: SocketAddr,
    pub database_url: String,
    /// Public origin of the site (web UI and API share it), e.g. `https://slop.example`.
    pub public_url: Url,
    /// GitHub sign-in. `None` turns sign-in off.
    pub github: Option<GithubConfig>,
    /// Extra origins allowed to make state-changing requests (e.g. a dev server on another port).
    pub trusted_origins: Vec<String>,
    /// Header set by the CDN or load balancer with the viewer's ISO country code.
    pub country_header: Option<String>,
    pub catalog: CatalogRules,
    pub releases: ReleaseConfig,
    pub rate_limits: crate::ratelimit::Limits,
    pub desktop: DesktopConfig,
    pub media: MediaConfig,
    /// The built website (`web/dist`) to serve from the same origin. Unset: API only.
    pub web_dir: Option<std::path::PathBuf>,
    /// Header holding the client's IP, set by the load balancer (e.g. `x-forwarded-for`,
    /// `fly-client-ip`). Unset: the connection's peer address. Only set it when every request
    /// passes through a proxy that overwrites the header, or clients can pick their own IP.
    pub client_ip_header: Option<String>,
    /// The simulation sandbox (sim.rs): offers "view as a simulated user". `serve` refuses it
    /// unless the database's name ends in `_sim`.
    pub simulation: bool,
}

/// Updates for the Slop desktop app itself (desktop_updates.rs).
#[derive(Clone, Default)]
pub struct DesktopConfig {
    /// Where uploaded builds are kept.
    pub dir: Option<std::path::PathBuf>,
    /// Signs builds for the app's updater (`slop-api keys generate-updater`).
    pub updater_key: Option<minisign::SecretKey>,
    /// Slop's own repository (`owner/repo`); only its desktop workflow on main may upload.
    pub repo: Option<String>,
}

impl fmt::Debug for DesktopConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DesktopConfig")
            .field("dir", &self.dir)
            .field(
                "updater_key",
                &self.updater_key.as_ref().map(|_| "<redacted>"),
            )
            .field("repo", &self.repo)
            .finish()
    }
}

/// Store-page pictures and videos (media.rs).
#[derive(Clone, Debug)]
pub struct MediaConfig {
    /// Where GitHub serves repository files: `<base><owner>/<repo>/<commit>/<path>`. Not the
    /// API, so it doesn't use Slop's request allowance. Changed only by tests.
    pub github_raw_base: Url,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            github_raw_base: "https://raw.githubusercontent.com/"
                .parse()
                .expect("valid URL"),
        }
    }
}

/// Release fetching and signing.
#[derive(Clone)]
pub struct ReleaseConfig {
    /// Slop's manifest signing key (base64 Ed25519 secret). Without it, nothing can be approved.
    /// Production keeps this in a KMS (SECURITY §8.3); see `slop-api keys generate`.
    pub signing_key: Option<slop_manifest::ed25519_dalek::SigningKey>,
    /// Largest release file Slop will fetch and hash. GitHub's own limit is 2 GiB per file.
    pub max_file_bytes: u64,
    /// GitHub REST API, for checking that a release's commit exists in the linked repo.
    pub github_api_base: Url,
    /// Lets release files use plain http. Only for local development and tests.
    pub allow_http_downloads: bool,
    /// Refuse to approve builds without a verified maker signature (SECURITY §5.3). Only turn this
    /// off for local development.
    pub require_maker_signature: bool,
    /// The scan step. Only `Off` exists so far (see scan.rs).
    pub scanner: crate::scan::Scanner,
    /// Slop Build's recipe, as `owner/repo/.github/workflows/build.yml`. Unset: Slop Build is off.
    pub slop_build_workflow: Option<String>,
    /// The recipe tag makers pin, e.g. `v1`.
    pub slop_build_version: String,
    /// Issuer of GitHub Actions OIDC tokens (its keys are at `<issuer>/.well-known/jwks`).
    pub github_oidc_issuer: String,
    /// Optional GitHub token, for higher API rate limits when checking commits and attestations.
    pub github_token: Option<String>,
    /// Slop's GitHub App, when configured (github_app.rs).
    pub github_app: Option<GithubAppConfig>,
}

/// Slop's GitHub App: makers install it on a repository so GitHub sends release webhooks and
/// requests about that repository use the installation's own API allowance.
#[derive(Clone)]
pub struct GithubAppConfig {
    pub app_id: String,
    /// The App's public name, for its install link: github.com/apps/<slug>.
    pub slug: String,
    /// Signs the short-lived JWTs that ask GitHub for installation tokens.
    pub private_key: jsonwebtoken::EncodingKey,
    /// Proves webhooks come from GitHub (X-Hub-Signature-256).
    pub webhook_secret: String,
}

impl GithubAppConfig {
    /// From GITHUB_APP_ID, GITHUB_APP_SLUG, GITHUB_APP_PRIVATE_KEY_FILE and
    /// GITHUB_APP_WEBHOOK_SECRET; None unless all four are set.
    fn from_env() -> anyhow::Result<Option<Self>> {
        let get = |k: &str| env::var(k).ok().filter(|s| !s.trim().is_empty());
        let (Some(app_id), Some(slug), Some(key_file), Some(webhook_secret)) = (
            get("GITHUB_APP_ID"),
            get("GITHUB_APP_SLUG"),
            get("GITHUB_APP_PRIVATE_KEY_FILE"),
            get("GITHUB_APP_WEBHOOK_SECRET"),
        ) else {
            return Ok(None);
        };
        let pem = std::fs::read(&key_file).with_context(|| format!("reading {key_file}"))?;
        let private_key = jsonwebtoken::EncodingKey::from_rsa_pem(&pem)
            .context("GITHUB_APP_PRIVATE_KEY_FILE must be the App's .pem private key")?;
        Ok(Some(Self {
            app_id: app_id.trim().to_string(),
            slug: slug.trim().to_string(),
            private_key,
            webhook_secret,
        }))
    }
}

impl fmt::Debug for ReleaseConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReleaseConfig")
            .field(
                "signing_key",
                &self.signing_key.as_ref().map(|_| "<redacted>"),
            )
            .field("max_file_bytes", &self.max_file_bytes)
            .field("github_api_base", &self.github_api_base.as_str())
            .field("allow_http_downloads", &self.allow_http_downloads)
            .field("require_maker_signature", &self.require_maker_signature)
            .field("scanner", &self.scanner)
            .field("slop_build_workflow", &self.slop_build_workflow)
            .field("slop_build_version", &self.slop_build_version)
            .field("github_oidc_issuer", &self.github_oidc_issuer)
            .field(
                "github_token",
                &self.github_token.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "github_app",
                &self.github_app.as_ref().map(|a| (&a.app_id, &a.slug)),
            )
            .finish()
    }
}

#[derive(Clone)]
pub struct GithubConfig {
    pub client_id: String,
    pub client_secret: String,
    pub oauth_base: Url,
    pub api_base: Url,
}

impl fmt::Debug for GithubConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GithubConfig")
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .field("oauth_base", &self.oauth_base.as_str())
            .field("api_base", &self.api_base.as_str())
            .finish()
    }
}

/// Stage time limits and the dead-project rule (DESIGN §8).
#[derive(Clone, Copy, Debug)]
pub struct CatalogRules {
    pub dead_after_days: i32,
    pub coming_soon_days: i32,
    pub beta_days: i32,
}

impl Default for CatalogRules {
    fn default() -> Self {
        Self {
            dead_after_days: 90,
            coming_soon_days: 30,
            beta_days: 365,
        }
    }
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind = var_or("SLOP_BIND", "127.0.0.1:8080")
            .parse()
            .context("SLOP_BIND")?;
        let database_url = env::var("DATABASE_URL").context("DATABASE_URL is required")?;
        let public_url: Url = var_or("SLOP_PUBLIC_URL", "http://localhost:8080")
            .parse()
            .context("SLOP_PUBLIC_URL")?;
        if public_url.path() != "/" {
            bail!("SLOP_PUBLIC_URL must be an origin with no path");
        }

        let github = match (
            env::var("GITHUB_CLIENT_ID"),
            env::var("GITHUB_CLIENT_SECRET"),
        ) {
            (Ok(client_id), Ok(client_secret)) if !client_id.is_empty() => Some(GithubConfig {
                client_id,
                client_secret,
                oauth_base: var_or("GITHUB_OAUTH_BASE", "https://github.com/")
                    .parse()
                    .context("GITHUB_OAUTH_BASE")?,
                api_base: var_or("GITHUB_API_BASE", "https://api.github.com/")
                    .parse()
                    .context("GITHUB_API_BASE")?,
            }),
            _ => None,
        };

        let trusted_origins = list_var("SLOP_TRUSTED_ORIGINS");

        let country_header = env::var("SLOP_COUNTRY_HEADER")
            .ok()
            .filter(|s| !s.is_empty());

        let defaults = CatalogRules::default();
        let catalog = CatalogRules {
            dead_after_days: int_var("SLOP_DEAD_AFTER_DAYS", defaults.dead_after_days)?,
            coming_soon_days: int_var("SLOP_COMING_SOON_DAYS", defaults.coming_soon_days)?,
            beta_days: int_var("SLOP_BETA_DAYS", defaults.beta_days)?,
        };

        let signing_key = match env::var("SLOP_SIGNING_KEY") {
            Ok(v) if !v.is_empty() => Some(
                slop_manifest::signing_key_from_base64(&v)
                    .context("SLOP_SIGNING_KEY must be a base64 32-byte Ed25519 key")?,
            ),
            _ => None,
        };
        let releases = ReleaseConfig {
            signing_key,
            max_file_bytes: var_or("SLOP_MAX_FILE_BYTES", "2147483648")
                .parse()
                .context("SLOP_MAX_FILE_BYTES")?,
            github_api_base: var_or("GITHUB_API_BASE", "https://api.github.com/")
                .parse()
                .context("GITHUB_API_BASE")?,
            allow_http_downloads: var_or("SLOP_ALLOW_HTTP_DOWNLOADS", "false") == "true",
            require_maker_signature: var_or("SLOP_REQUIRE_MAKER_SIGNATURE", "true") != "false",
            scanner: crate::scan::Scanner::from_setting(&var_or("SLOP_SCANNER", "off"))?,
            slop_build_workflow: env::var("SLOP_BUILD_WORKFLOW")
                .ok()
                .map(|s| {
                    s.trim()
                        .trim_start_matches("https://github.com/")
                        .trim_end_matches('@')
                        .to_string()
                })
                .filter(|s| !s.is_empty()),
            slop_build_version: var_or("SLOP_BUILD_VERSION", "v1"),
            github_oidc_issuer: var_or(
                "GITHUB_OIDC_ISSUER",
                "https://token.actions.githubusercontent.com",
            ),
            github_token: env::var("GITHUB_TOKEN").ok().filter(|s| !s.is_empty()),
            github_app: GithubAppConfig::from_env()?,
        };

        Ok(Self {
            bind,
            database_url,
            public_url,
            github,
            trusted_origins,
            country_header,
            catalog,
            releases,
            rate_limits: crate::ratelimit::Limits {
                auth_per_minute: int_var("SLOP_RATE_AUTH_PER_MIN", 20)?.max(1) as u32,
                write_per_minute: int_var("SLOP_RATE_WRITE_PER_MIN", 60)?.max(1) as u32,
                read_per_minute: int_var("SLOP_RATE_READ_PER_MIN", 600)?.max(1) as u32,
            },
            desktop: DesktopConfig {
                dir: env::var("SLOP_DESKTOP_DIR")
                    .ok()
                    .filter(|s| !s.is_empty())
                    .map(std::path::PathBuf::from),
                updater_key: match env::var("SLOP_UPDATER_KEY") {
                    Ok(v) if !v.is_empty() => Some(
                        crate::desktop_updates::updater_key_from_base64(&v)
                            .context("SLOP_UPDATER_KEY isn't a valid updater key")?,
                    ),
                    _ => None,
                },
                repo: env::var("SLOP_DESKTOP_REPO").ok().filter(|s| !s.is_empty()),
            },
            media: MediaConfig {
                github_raw_base: var_or("GITHUB_RAW_BASE", "https://raw.githubusercontent.com/")
                    .parse()
                    .context("GITHUB_RAW_BASE")?,
            },
            web_dir: env::var("SLOP_WEB_DIR")
                .ok()
                .filter(|s| !s.is_empty())
                .map(std::path::PathBuf::from),
            client_ip_header: env::var("SLOP_CLIENT_IP_HEADER")
                .ok()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_ascii_lowercase()),
            simulation: var_or("SLOP_SIMULATION", "false") == "true",
        })
    }

    /// Cookies are `Secure` (and use the `__Host-` prefix) whenever the site is served over HTTPS.
    pub fn secure_cookies(&self) -> bool {
        self.public_url.scheme() == "https"
    }
}

fn var_or(name: &str, default: &str) -> String {
    env::var(name)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn list_var(name: &str) -> Vec<String> {
    env::var(name)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

fn int_var(name: &str, default: i32) -> anyhow::Result<i32> {
    match env::var(name) {
        Ok(v) if !v.is_empty() => v
            .parse()
            .with_context(|| format!("{name} must be a number")),
        _ => Ok(default),
    }
}
