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

        let trusted_origins = env::var("SLOP_TRUSTED_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();

        let country_header = env::var("SLOP_COUNTRY_HEADER")
            .ok()
            .filter(|s| !s.is_empty());

        let defaults = CatalogRules::default();
        let catalog = CatalogRules {
            dead_after_days: int_var("SLOP_DEAD_AFTER_DAYS", defaults.dead_after_days)?,
            coming_soon_days: int_var("SLOP_COMING_SOON_DAYS", defaults.coming_soon_days)?,
            beta_days: int_var("SLOP_BETA_DAYS", defaults.beta_days)?,
        };

        Ok(Self {
            bind,
            database_url,
            public_url,
            github,
            trusted_origins,
            country_header,
            catalog,
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

fn int_var(name: &str, default: i32) -> anyhow::Result<i32> {
    match env::var(name) {
        Ok(v) if !v.is_empty() => v
            .parse()
            .with_context(|| format!("{name} must be a number")),
        _ => Ok(default),
    }
}
