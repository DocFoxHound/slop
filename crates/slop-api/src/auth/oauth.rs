// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sign-in providers: GitHub, Google, GitLab, Bitbucket, Microsoft and Discord (SECURITY §4.1).
//!
//! The same authorization-code flow for each (with PKCE where the provider supports it). Slop
//! reads the account's stable ID, its name at the provider and, where the provider has verified
//! it, the email; the provider's token is used once and never stored.

use anyhow::{Context, anyhow};
use serde::Deserialize;
use url::Url;

use crate::{AppState, config::OAuthConfig};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Github,
    Google,
    Gitlab,
    Bitbucket,
    Microsoft,
    Discord,
}

pub const ALL: [Provider; 6] = [
    Provider::Github,
    Provider::Google,
    Provider::Gitlab,
    Provider::Bitbucket,
    Provider::Microsoft,
    Provider::Discord,
];

impl Provider {
    pub fn parse(name: &str) -> Option<Self> {
        ALL.into_iter().find(|p| p.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Google => "google",
            Self::Gitlab => "gitlab",
            Self::Bitbucket => "bitbucket",
            Self::Microsoft => "microsoft",
            Self::Discord => "discord",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Github => "GitHub",
            Self::Google => "Google",
            Self::Gitlab => "GitLab",
            Self::Bitbucket => "Bitbucket",
            Self::Microsoft => "Microsoft",
            Self::Discord => "Discord",
        }
    }

    pub fn enabled(self, state: &AppState) -> bool {
        match self {
            Self::Github => state.config.github.is_some(),
            _ => self.config(state).is_some(),
        }
    }

    fn config(self, state: &AppState) -> Option<&OAuthConfig> {
        match self {
            Self::Github => None,
            Self::Google => state.config.google.as_ref(),
            Self::Gitlab => state.config.gitlab.as_ref(),
            Self::Bitbucket => state.config.bitbucket.as_ref(),
            Self::Microsoft => state.config.microsoft.as_ref(),
            Self::Discord => state.config.discord.as_ref(),
        }
    }

    /// Bitbucket Cloud and Discord don't take PKCE; the state cookie still ties the flow to the
    /// browser.
    fn pkce(self) -> bool {
        !matches!(self, Self::Bitbucket | Self::Discord)
    }

    /// Microsoft and Discord take the app's secret in the form rather than as Basic auth.
    fn secret_in_form(self) -> bool {
        matches!(self, Self::Microsoft | Self::Discord)
    }
}

/// What Slop keeps from a provider account.
#[derive(Debug)]
pub struct Profile {
    /// The provider's stable account ID.
    pub subject: String,
    /// The account's name at the provider (a login or username), if it has one.
    pub login: Option<String>,
    /// A display name to suggest at sign-up.
    pub name: String,
    /// Only if the provider has verified it.
    pub email: Option<String>,
}

pub fn callback_url(state: &AppState, provider: Provider) -> String {
    state
        .config
        .public_url
        .join(&format!("/v1/auth/{}/callback", provider.name()))
        .expect("valid public URL")
        .into()
}

pub fn authorize_url(
    state: &AppState,
    provider: Provider,
    oauth_state: &str,
    challenge: &str,
) -> Option<Url> {
    let redirect = callback_url(state, provider);
    if provider == Provider::Github {
        let cfg = state.config.github.as_ref()?;
        return Some(super::github::authorize_url(
            cfg,
            &redirect,
            oauth_state,
            challenge,
        ));
    }
    let cfg = provider.config(state)?;
    let mut url = cfg.authorize_url.clone();
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("client_id", &cfg.client_id)
            .append_pair("response_type", "code")
            .append_pair("state", oauth_state);
        match provider {
            Provider::Google => {
                q.append_pair("redirect_uri", &redirect)
                    .append_pair("scope", "openid email profile")
                    .append_pair("prompt", "select_account");
            }
            Provider::Gitlab => {
                q.append_pair("redirect_uri", &redirect)
                    .append_pair("scope", "read_user");
            }
            Provider::Microsoft => {
                q.append_pair("redirect_uri", &redirect)
                    .append_pair("scope", "openid profile email")
                    .append_pair("prompt", "select_account");
            }
            Provider::Discord => {
                q.append_pair("redirect_uri", &redirect)
                    .append_pair("scope", "identify email")
                    .append_pair("prompt", "consent");
            }
            // Bitbucket uses the callback registered with the app.
            Provider::Bitbucket | Provider::Github => {}
        }
        if provider.pkce() {
            q.append_pair("code_challenge", challenge)
                .append_pair("code_challenge_method", "S256");
        }
    }
    Some(url)
}

async fn token(
    state: &AppState,
    provider: Provider,
    cfg: &OAuthConfig,
    code: &str,
    verifier: &str,
) -> anyhow::Result<String> {
    #[derive(Deserialize)]
    struct Token {
        access_token: Option<String>,
        error: Option<String>,
    }
    let redirect = callback_url(state, provider);
    let mut form = vec![("grant_type", "authorization_code"), ("code", code)];
    if provider != Provider::Bitbucket {
        form.push(("redirect_uri", redirect.as_str()));
    }
    if provider.pkce() {
        form.push(("code_verifier", verifier));
    }
    let mut request = state.http.post(cfg.token_url.clone());
    if provider.secret_in_form() {
        form.push(("client_id", cfg.client_id.as_str()));
        form.push(("client_secret", cfg.client_secret.as_str()));
    } else {
        request = request.basic_auth(&cfg.client_id, Some(&cfg.client_secret));
    }
    let res: Token = request
        .header("accept", "application/json")
        .form(&form)
        .send()
        .await
        .with_context(|| format!("{} token request", provider.label()))?
        .error_for_status()
        .with_context(|| format!("{} token request", provider.label()))?
        .json()
        .await
        .with_context(|| format!("{} token response", provider.label()))?;
    res.access_token.ok_or_else(|| {
        anyhow!(
            "{} refused the code: {}",
            provider.label(),
            res.error.unwrap_or_default()
        )
    })
}

async fn get<T: serde::de::DeserializeOwned>(
    state: &AppState,
    base: &Url,
    path: &str,
    token: &str,
) -> anyhow::Result<T> {
    state
        .http
        .get(base.join(path)?)
        .bearer_auth(token)
        .header("accept", "application/json")
        .header("user-agent", "slop-api")
        .send()
        .await
        .with_context(|| format!("GET {path}"))?
        .error_for_status()
        .with_context(|| format!("GET {path}"))?
        .json()
        .await
        .with_context(|| format!("GET {path} response"))
}

/// Swaps the code for a token and reads the account.
pub async fn profile(
    state: &AppState,
    provider: Provider,
    code: &str,
    verifier: &str,
) -> anyhow::Result<Profile> {
    if provider == Provider::Github {
        let cfg = state
            .config
            .github
            .as_ref()
            .context("GitHub isn't set up")?;
        let redirect = callback_url(state, provider);
        let token =
            super::github::exchange_code(&state.http, cfg, code, verifier, &redirect).await?;
        let user = super::github::fetch_user(&state.http, cfg, &token).await?;
        let email = super::github::fetch_primary_email(&state.http, cfg, &token).await?;
        return Ok(Profile {
            subject: user.id.to_string(),
            name: user
                .name
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| user.login.clone()),
            login: Some(user.login),
            email,
        });
    }
    let cfg = provider
        .config(state)
        .with_context(|| format!("{} isn't set up", provider.label()))?;
    let token = token(state, provider, cfg, code, verifier).await?;
    match provider {
        Provider::Google => {
            #[derive(Deserialize)]
            struct User {
                sub: String,
                name: Option<String>,
                email: Option<String>,
                #[serde(default)]
                email_verified: bool,
            }
            let u: User = get(state, &cfg.api_base, "userinfo", &token).await?;
            Ok(Profile {
                name: u
                    .name
                    .clone()
                    .or_else(|| {
                        u.email
                            .as_ref()
                            .and_then(|e| e.split('@').next().map(String::from))
                    })
                    .unwrap_or_else(|| "Player".into()),
                login: u.email.clone().filter(|_| u.email_verified),
                email: u.email.filter(|_| u.email_verified),
                subject: u.sub,
            })
        }
        Provider::Gitlab => {
            #[derive(Deserialize)]
            struct User {
                id: i64,
                username: String,
                name: Option<String>,
                email: Option<String>,
                confirmed_at: Option<String>,
            }
            let u: User = get(state, &cfg.api_base, "user", &token).await?;
            Ok(Profile {
                subject: u.id.to_string(),
                name: u
                    .name
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| u.username.clone()),
                login: Some(u.username),
                email: u.email.filter(|_| u.confirmed_at.is_some()),
            })
        }
        Provider::Bitbucket => {
            #[derive(Deserialize)]
            struct User {
                uuid: String,
                username: Option<String>,
                display_name: Option<String>,
            }
            #[derive(Deserialize)]
            struct Emails {
                values: Vec<Email>,
            }
            #[derive(Deserialize)]
            struct Email {
                email: String,
                is_primary: bool,
                is_confirmed: bool,
            }
            let u: User = get(state, &cfg.api_base, "user", &token).await?;
            let emails: Option<Emails> =
                get(state, &cfg.api_base, "user/emails", &token).await.ok();
            Ok(Profile {
                subject: u.uuid,
                name: u
                    .display_name
                    .clone()
                    .or_else(|| u.username.clone())
                    .unwrap_or_else(|| "Player".into()),
                login: u.username,
                email: emails.and_then(|e| {
                    e.values
                        .into_iter()
                        .find(|e| e.is_primary && e.is_confirmed)
                        .map(|e| e.email)
                }),
            })
        }
        Provider::Microsoft => {
            // Microsoft's OpenID userinfo. The `sub` is stable for this app. Microsoft doesn't
            // promise the email is verified (anyone can put any address on a personal account),
            // so it's never used.
            #[derive(Deserialize)]
            struct User {
                sub: String,
                name: Option<String>,
            }
            let u: User = get(state, &cfg.api_base, "oidc/userinfo", &token).await?;
            Ok(Profile {
                subject: u.sub,
                name: u
                    .name
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| "Player".into()),
                login: None,
                email: None,
            })
        }
        Provider::Discord => {
            #[derive(Deserialize)]
            struct User {
                id: String,
                username: String,
                global_name: Option<String>,
                email: Option<String>,
                #[serde(default)]
                verified: bool,
            }
            let u: User = get(state, &cfg.api_base, "users/@me", &token).await?;
            Ok(Profile {
                subject: u.id,
                name: u
                    .global_name
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| u.username.clone()),
                login: Some(u.username),
                email: u.email.filter(|_| u.verified),
            })
        }
        Provider::Github => unreachable!(),
    }
}
