// SPDX-License-Identifier: AGPL-3.0-or-later
//! Profile pictures from a linked sign-in (built 2026-10-24).
//!
//! On the account page someone picks which linked sign-in's picture to show, or keeps the letter
//! (the default). Slop downloads that picture, decodes it and re-encodes it as a 128×128 PNG, and
//! serves its own copy at `/v1/users/{id}/avatar`: people looking at it never contact Google,
//! Discord or the others, the site's CSP keeps allowing only Slop's own images, and re-encoding
//! strips metadata and anything else hidden in the file.
//!
//! Keeping it current:
//! - **GitHub** pictures live at a fixed address per account (`avatars.githubusercontent.com/u/<id>`)
//!   and **GitLab** says where an account's picture is without a token (`/api/v4/users/<id>`), so
//!   both are simply read again daily.
//! - **Google, Discord and Bitbucket** say where the picture is only when the person signs in, so
//!   the address is updated at every sign-in with them (auth/mod.rs `callback`), and the account
//!   page's "Refresh picture" goes through that provider's sign-in again (the link flow). The
//!   stored address is still read daily, in case the picture changed at the same address.
//! - **Microsoft** keeps pictures in Microsoft Graph, which needs another permission: not offered.
//!
//! The daily check (`refresh_due`, from the hourly cleanup) takes the pictures last checked over
//! a day ago, a batch at a time and about three a second, with conditional requests (ETag and
//! Last-Modified), so an unchanged picture costs a "304 Not Modified". A picture that can't be
//! read keeps its last copy.
//!
//! Downloads go only to that provider's own picture hosts over https, through a client that
//! connects only to public addresses and follows redirects by hand (each hop's host checked
//! again); at most 1 MB, only PNG, JPEG, WebP or GIF by their first bytes, decoded with a size and
//! memory limit off the async workers.

use std::{io::Cursor, time::Duration as StdDuration};

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use image::{ImageEncoder, ImageFormat, ImageReader, imageops::FilterType};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use time::{Duration, OffsetDateTime};
use url::Url;
use uuid::Uuid;

use crate::{
    AppState,
    auth::{oauth::Provider, session::CurrentUser},
    error::{ApiError, ApiResult},
    limits::{self, Action},
};

/// Width and height of Slop's copy, in pixels.
pub const SIZE: u32 = 128;
/// The largest picture Slop downloads.
pub const MAX_BYTES: u64 = 1024 * 1024;
/// The largest picture Slop decodes, on each side.
const MAX_SIDE: u32 = 4096;
/// Redirects followed for one picture.
const MAX_REDIRECTS: usize = 3;
/// A chosen picture is read again once its last check is this old.
pub const REFRESH_AFTER: Duration = Duration::hours(24);
/// Pictures checked per run of the hourly cleanup, and the pause between them (about three a
/// second at most).
const BATCH: i64 = 300;
const PAUSE: StdDuration = StdDuration::from_millis(350);
/// How long a sign-in waits for its chosen picture to be brought up to date.
const SIGN_IN_WAIT: StdDuration = StdDuration::from_secs(5);

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/avatar", get(mine).put(choose))
        .route("/users/{id}/avatar", get(serve))
}

// ---------------------------------------------------------------------------------------------
// Where pictures are

/// A picture address a provider gave, if it's usable at all (hosts are checked when it's read).
pub fn clean_picture(picture: Option<&str>) -> Option<String> {
    let picture = picture?.trim();
    if picture.is_empty() || picture.len() > 2048 {
        return None;
    }
    let url = Url::parse(picture).ok()?;
    matches!(url.scheme(), "https" | "http").then(|| url.to_string())
}

/// A GitHub account's picture: always its current one.
pub fn github_source(state: &AppState, subject: &str) -> Option<Url> {
    if subject.is_empty() || subject.len() > 20 || !subject.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    state
        .config
        .media
        .github_avatar_base
        .join(&format!("u/{subject}?v=4&s={SIZE}"))
        .ok()
}

/// A Discord account's picture, or the default one Discord shows when it has none.
pub fn discord_source(
    id: &str,
    avatar: Option<&str>,
    discriminator: Option<&str>,
) -> Option<String> {
    let number: u64 = id.parse().ok()?;
    let hash = avatar.filter(|h| {
        !h.is_empty() && h.len() <= 64 && h.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    });
    Some(match hash {
        Some(hash) => format!("https://cdn.discordapp.com/avatars/{id}/{hash}.png?size={SIZE}"),
        None => {
            // Accounts with the old four-digit tags pick by tag; the rest by their ID.
            let index = match discriminator
                .and_then(|d| d.parse::<u64>().ok())
                .filter(|d| *d != 0)
            {
                Some(tag) => tag % 5,
                None => (number >> 22) % 6,
            };
            format!("https://cdn.discordapp.com/embed/avatars/{index}.png")
        }
    })
}

/// How a provider's picture is kept current.
fn updates(provider: Provider) -> &'static str {
    match provider {
        Provider::Github | Provider::Gitlab => "daily",
        Provider::Google | Provider::Discord | Provider::Bitbucket => "sign_in",
        Provider::Microsoft => "never",
    }
}

/// Whether `url` is one of `provider`'s own picture hosts, over https.
fn allowed(state: &AppState, provider: Provider, url: &Url) -> bool {
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    if state.config.media.avatar_test_hosts.contains(&host) {
        return matches!(url.scheme(), "https" | "http");
    }
    if url.scheme() != "https" || url.port().is_some() {
        return false;
    }
    let configured = |base: &Url| base.scheme() == "https" && base.host_str() == Some(&host);
    let (exact, suffixes): (&[&str], &[&str]) = match provider {
        Provider::Github => {
            if configured(&state.config.media.github_avatar_base) {
                return true;
            }
            (&["avatars.githubusercontent.com"], &[])
        }
        Provider::Gitlab => {
            // A GitLab of their own serves its pictures itself.
            if state
                .config
                .gitlab
                .as_ref()
                .is_some_and(|c| configured(&c.api_base))
            {
                return true;
            }
            (
                &["gitlab.com", "secure.gravatar.com", "www.gravatar.com"],
                &[],
            )
        }
        Provider::Google => (&[], &[".googleusercontent.com"]),
        Provider::Discord => (&["cdn.discordapp.com"], &[]),
        Provider::Bitbucket => (
            &["bitbucket.org", "secure.gravatar.com", "www.gravatar.com"],
            &[".atl-paas.net"],
        ),
        Provider::Microsoft => (&[], &[]),
    };
    exact.contains(&host.as_str()) || suffixes.iter().any(|s| host.ends_with(s))
}

/// Where a GitLab account's picture is now, asked without a token.
async fn gitlab_picture(state: &AppState, subject: &str) -> anyhow::Result<Option<String>> {
    #[derive(Deserialize)]
    struct User {
        avatar_url: Option<String>,
    }
    let Some(cfg) = state.config.gitlab.as_ref() else {
        return Ok(None);
    };
    anyhow::ensure!(
        !subject.is_empty() && subject.bytes().all(|b| b.is_ascii_digit()),
        "not a GitLab account ID"
    );
    let user: User = state
        .http
        .get(cfg.api_base.join(&format!("users/{subject}"))?)
        .header(header::ACCEPT, "application/json")
        .header(header::USER_AGENT, "slop-api")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(clean_picture(user.avatar_url.as_deref()))
}

/// Where to read the picture of one linked sign-in; None when it has none Slop can show.
async fn source(
    state: &AppState,
    provider: Provider,
    subject: &str,
    stored: Option<&str>,
) -> anyhow::Result<Option<Url>> {
    Ok(match provider {
        Provider::Github => github_source(state, subject),
        Provider::Gitlab => {
            let found = match stored {
                Some(s) => Some(s.to_string()),
                None => gitlab_picture(state, subject).await?,
            };
            found.and_then(|s| Url::parse(&s).ok())
        }
        Provider::Google | Provider::Discord | Provider::Bitbucket => {
            stored.and_then(|s| Url::parse(s).ok())
        }
        Provider::Microsoft => None,
    })
}

// ---------------------------------------------------------------------------------------------
// Reading a picture

pub(crate) struct Picture {
    png: Vec<u8>,
    version: String,
    etag: Option<String>,
    last_modified: Option<String>,
}

pub(crate) enum Fetched {
    NotModified,
    /// The host says there's no picture there (any more).
    Gone,
    Picture(Picture),
}

/// A header worth keeping for the next conditional request.
fn header_text(headers: &HeaderMap, name: header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= 200)
        .map(String::from)
}

/// Reads a picture from one of `provider`'s hosts and makes Slop's copy.
pub(crate) async fn fetch(
    state: &AppState,
    provider: Provider,
    mut url: Url,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> anyhow::Result<Fetched> {
    for _ in 0..=MAX_REDIRECTS {
        anyhow::ensure!(
            allowed(state, provider, &url),
            "{} isn't one of {}'s picture hosts",
            url.host_str().unwrap_or("that address"),
            provider.label()
        );
        let mut request = state
            .avatar_http
            .get(url.clone())
            .header(header::USER_AGENT, "slop-api")
            .header(header::ACCEPT, "image/png,image/jpeg,image/webp,image/gif");
        if let Some(e) = etag {
            request = request.header(header::IF_NONE_MATCH, e);
        }
        if let Some(m) = last_modified {
            request = request.header(header::IF_MODIFIED_SINCE, m);
        }
        let mut res = request.send().await?;
        let status = res.status();
        if status == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(Fetched::NotModified);
        }
        if status.is_redirection() {
            let to = res
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .context("a redirect without a Location")?;
            url = url.join(to)?;
            continue;
        }
        if matches!(
            status,
            reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::GONE
        ) {
            return Ok(Fetched::Gone);
        }
        anyhow::ensure!(status.is_success(), "the picture host answered {status}");
        if res.content_length().is_some_and(|n| n > MAX_BYTES) {
            anyhow::bail!("the picture is over 1 MB");
        }
        let etag = header_text(res.headers(), header::ETAG);
        let last_modified = header_text(res.headers(), header::LAST_MODIFIED);
        let mut body = Vec::new();
        while let Some(chunk) = res.chunk().await? {
            body.extend_from_slice(&chunk);
            anyhow::ensure!(body.len() as u64 <= MAX_BYTES, "the picture is over 1 MB");
        }
        let png = tokio::task::spawn_blocking(move || normalize(&body)).await??;
        let version = hex::encode(&Sha256::digest(&png)[..8]);
        return Ok(Fetched::Picture(Picture {
            png,
            version,
            etag,
            last_modified,
        }));
    }
    anyhow::bail!("too many redirects")
}

/// The picture's format from its first bytes: PNG, JPEG, WebP or GIF only.
pub fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(ImageFormat::Gif)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(ImageFormat::WebP)
    } else {
        None
    }
}

/// Decodes a picture (the first frame of an animation) and re-encodes it as a SIZE×SIZE PNG,
/// cropped to a centred square. Nothing of the original file but its pixels survives.
pub fn normalize(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let format = sniff(bytes).context("not a PNG, JPEG, WebP or GIF picture")?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let picture = reader.decode().context("the picture is damaged")?;
    let square = picture
        .resize_to_fill(SIZE, SIZE, FilterType::CatmullRom)
        .to_rgba8();
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(
        &square,
        SIZE,
        SIZE,
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Addresses Slop hands out

/// SQL for the site-relative address of the picture of the user `user` (a column or parameter),
/// or null: the letter. Suspended and closed accounts show the letter.
pub fn path_sql(user: &str) -> String {
    format!(
        "(select '/v1/users/' || a.user_id::text || '/avatar?v=' || a.version from user_avatars a
          join users au on au.id = a.user_id
          where a.user_id = {user} and au.closed_at is null and au.suspended_at is null)"
    )
}

/// Makes a path from `path_sql` a full address. The desktop app shows the website from its own
/// files, so a relative one wouldn't reach Slop there.
pub fn absolute(config: &crate::config::Config, path: Option<String>) -> Option<String> {
    path.map(|p| format!("{}{p}", config.public_url.as_str().trim_end_matches('/')))
}

/// The address of a user's picture, or None for the letter.
pub async fn url_of(state: &AppState, user: Uuid) -> sqlx::Result<Option<String>> {
    let path: Option<String> =
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!("select {}", path_sql("$1"))))
            .bind(user)
            .fetch_one(&state.db)
            .await?;
    Ok(absolute(&state.config, path))
}

// ---------------------------------------------------------------------------------------------
// The account page

#[derive(Serialize)]
struct Choice {
    provider: String,
    subject: String,
    login: Option<String>,
    /// Slop can show this sign-in's picture now.
    available: bool,
    /// "daily", "sign_in" (updated when they sign in with it) or "never" (no picture).
    updates: &'static str,
    /// "Refresh picture" can send them through this sign-in again.
    can_refresh: bool,
}

#[derive(Serialize)]
struct Mine {
    avatar_url: Option<String>,
    /// The sign-in it comes from; null: the letter.
    provider: Option<String>,
    subject: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    fetched_at: Option<OffsetDateTime>,
    choices: Vec<Choice>,
}

async fn load_mine(state: &AppState, user: Uuid) -> ApiResult<Mine> {
    let chosen: Option<(String, String, OffsetDateTime)> = sqlx::query_as(
        "select provider, provider_subject, fetched_at from user_avatars where user_id = $1",
    )
    .bind(user)
    .fetch_optional(&state.db)
    .await?;
    let identities: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "select provider, provider_subject, login, picture_url from identities
         where user_id = $1 order by created_at",
    )
    .bind(user)
    .fetch_all(&state.db)
    .await?;
    let choices = identities
        .into_iter()
        .filter_map(|(name, subject, login, picture)| {
            let provider = Provider::parse(&name)?;
            let available = match provider {
                Provider::Github => github_source(state, &subject).is_some(),
                Provider::Gitlab => picture.is_some() || state.config.gitlab.is_some(),
                Provider::Google | Provider::Discord | Provider::Bitbucket => picture.is_some(),
                Provider::Microsoft => false,
            };
            Some(Choice {
                provider: name,
                subject,
                login,
                available,
                updates: updates(provider),
                can_refresh: updates(provider) == "sign_in" && provider.enabled(state),
            })
        })
        .collect();
    let (provider, subject, fetched_at) = match chosen {
        Some((p, s, f)) => (Some(p), Some(s), Some(f)),
        None => (None, None, None),
    };
    Ok(Mine {
        avatar_url: url_of(state, user).await?,
        provider,
        subject,
        fetched_at,
        choices,
    })
}

/// The account's picture and the linked sign-ins it can come from.
async fn mine(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Mine>> {
    Ok(Json(load_mine(&state, user.id).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChooseBody {
    /// A linked sign-in's provider; null: the letter.
    provider: Option<String>,
    /// Which of their accounts at that provider, when they linked more than one.
    #[serde(default)]
    subject: Option<String>,
}

/// Picks the picture: a linked sign-in's, or the letter (`provider: null`).
async fn choose(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<ChooseBody>,
) -> ApiResult<Json<Mine>> {
    let Some(name) = body.provider else {
        sqlx::query("delete from user_avatars where user_id = $1")
            .bind(user.id)
            .execute(&state.db)
            .await?;
        return Ok(Json(load_mine(&state, user.id).await?));
    };
    let provider =
        Provider::parse(&name).ok_or_else(|| ApiError::invalid("provider", "unknown sign-in"))?;
    let linked: Option<(String, Option<String>)> = sqlx::query_as(
        "select provider_subject, picture_url from identities
         where user_id = $1 and provider = $2 and ($3::text is null or provider_subject = $3)
         order by last_sign_in_at desc limit 1",
    )
    .bind(user.id)
    .bind(provider.name())
    .bind(&body.subject)
    .fetch_optional(&state.db)
    .await?;
    let Some((subject, stored)) = linked else {
        return Err(ApiError::invalid(
            "provider",
            format!("{} isn't linked to your account", provider.label()),
        ));
    };
    limits::check(&state, user.id, Action::Picture)?;
    let no_picture = || {
        ApiError::invalid(
            "provider",
            match updates(provider) {
                "never" => format!("{} doesn't share a picture with Slop", provider.label()),
                "sign_in" => format!(
                    "Slop doesn't have your {} picture yet: press Refresh to sign in with it again",
                    provider.label()
                ),
                _ => format!("your {} account has no picture", provider.label()),
            },
        )
    };
    let failed = |err: anyhow::Error| {
        tracing::warn!(error = %format!("{err:#}"), provider = provider.name(), "profile picture");
        ApiError::invalid(
            "provider",
            format!(
                "couldn't get your picture from {}: try again later",
                provider.label()
            ),
        )
    };
    let url = source(&state, provider, &subject, stored.as_deref())
        .await
        .map_err(failed)?
        .ok_or_else(no_picture)?;
    let picture = match fetch(&state, provider, url.clone(), None, None)
        .await
        .map_err(failed)?
    {
        Fetched::Picture(p) => p,
        Fetched::Gone | Fetched::NotModified => return Err(no_picture()),
    };
    sqlx::query(
        "insert into user_avatars (user_id, provider, provider_subject, source_url, image, version, etag, last_modified)
         values ($1, $2, $3, $4, $5, $6, $7, $8)
         on conflict (user_id) do update set provider = excluded.provider, provider_subject = excluded.provider_subject,
             source_url = excluded.source_url, image = excluded.image, version = excluded.version,
             etag = excluded.etag, last_modified = excluded.last_modified,
             fetched_at = now(), checked_at = now(), failures = 0",
    )
    .bind(user.id)
    .bind(provider.name())
    .bind(&subject)
    .bind(url.as_str())
    .bind(&picture.png)
    .bind(&picture.version)
    .bind(&picture.etag)
    .bind(&picture.last_modified)
    .execute(&state.db)
    .await?;
    Ok(Json(load_mine(&state, user.id).await?))
}

/// Staff remove someone's picture (admin_tools.rs). Returns the sign-in it came from.
pub async fn remove(conn: &mut PgConnection, user: Uuid) -> sqlx::Result<Option<String>> {
    sqlx::query_scalar("delete from user_avatars where user_id = $1 returning provider")
        .bind(user)
        .fetch_optional(conn)
        .await
}

// ---------------------------------------------------------------------------------------------
// Serving Slop's copy

#[derive(Deserialize)]
struct ServeQuery {
    v: Option<String>,
}

/// Slop's copy of someone's picture. The address the API hands out carries the version, so it
/// can be cached for good; without it (or with an old one) for five minutes.
async fn serve(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<ServeQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let row: Option<(Vec<u8>, String)> = sqlx::query_as(
        "select a.image, a.version from user_avatars a join users u on u.id = a.user_id
         where a.user_id = $1 and u.closed_at is null and u.suspended_at is null",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let (image, version) = row.ok_or(ApiError::NotFound)?;
    let etag = format!("\"{version}\"");
    let cache = if q.v.as_deref() == Some(version.as_str()) {
        "public, max-age=31536000, immutable"
    } else {
        "public, max-age=300"
    };
    let unchanged = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag || t.trim() == "*"));
    let mut res = if unchanged {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let mut res = image.into_response();
        res.headers_mut()
            .insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
        res
    };
    let h = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&etag) {
        h.insert(header::ETAG, v);
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    // Opened on its own, it's a picture and nothing else.
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("inline; filename=\"picture.png\""),
    );
    Ok(res)
}

// ---------------------------------------------------------------------------------------------
// Keeping pictures current

#[derive(sqlx::FromRow)]
struct Chosen {
    user_id: Uuid,
    provider: String,
    provider_subject: String,
    source_url: String,
    etag: Option<String>,
    last_modified: Option<String>,
}

/// What a check found.
#[derive(Debug, PartialEq, Eq)]
pub enum Checked {
    Unchanged,
    Updated,
    Failed,
}

/// Reads one chosen picture again and keeps the new copy.
async fn check(state: &AppState, mut chosen: Chosen) -> anyhow::Result<Checked> {
    let provider = Provider::parse(&chosen.provider).context("unknown provider")?;
    if provider == Provider::Gitlab
        && let Ok(Some(now)) = gitlab_picture(state, &chosen.provider_subject).await
        && now != chosen.source_url
    {
        moved(state, &mut chosen, &now).await?;
    }
    let url = Url::parse(&chosen.source_url)?;
    let fetched = fetch(
        state,
        provider,
        url,
        chosen.etag.as_deref(),
        chosen.last_modified.as_deref(),
    )
    .await;
    let picture = match fetched {
        Ok(Fetched::Picture(p)) => p,
        Ok(Fetched::NotModified) => {
            sqlx::query(
                "update user_avatars set checked_at = now(), failures = 0
                 where user_id = $1 and provider = $2 and provider_subject = $3",
            )
            .bind(chosen.user_id)
            .bind(&chosen.provider)
            .bind(&chosen.provider_subject)
            .execute(&state.db)
            .await?;
            return Ok(Checked::Unchanged);
        }
        Ok(Fetched::Gone) | Err(_) => {
            if let Err(err) = &fetched {
                tracing::debug!(error = %format!("{err:#}"), user = %chosen.user_id, "profile picture check");
            }
            sqlx::query(
                "update user_avatars set checked_at = now(), failures = failures + 1
                 where user_id = $1 and provider = $2 and provider_subject = $3",
            )
            .bind(chosen.user_id)
            .bind(&chosen.provider)
            .bind(&chosen.provider_subject)
            .execute(&state.db)
            .await?;
            return Ok(Checked::Failed);
        }
    };
    // Only the row as it still is: they may have switched to the letter meanwhile.
    let updated: Option<bool> = sqlx::query_scalar(
        "update user_avatars a set image = $4, version = $5, etag = $6, last_modified = $7,
                fetched_at = now(), checked_at = now(), failures = 0
         from (select version as old from user_avatars where user_id = $1) o
         where a.user_id = $1 and a.provider = $2 and a.provider_subject = $3
         returning o.old <> $5",
    )
    .bind(chosen.user_id)
    .bind(&chosen.provider)
    .bind(&chosen.provider_subject)
    .bind(&picture.png)
    .bind(&picture.version)
    .bind(&picture.etag)
    .bind(&picture.last_modified)
    .fetch_optional(&state.db)
    .await?;
    Ok(if updated == Some(true) {
        Checked::Updated
    } else {
        Checked::Unchanged
    })
}

/// The picture is somewhere else now: read it from there, without the old conditions.
async fn moved(state: &AppState, chosen: &mut Chosen, to: &str) -> anyhow::Result<()> {
    sqlx::query(
        "update user_avatars set source_url = $4, etag = null, last_modified = null
         where user_id = $1 and provider = $2 and provider_subject = $3",
    )
    .bind(chosen.user_id)
    .bind(&chosen.provider)
    .bind(&chosen.provider_subject)
    .bind(to)
    .execute(&state.db)
    .await?;
    sqlx::query(
        "update identities set picture_url = $3 where provider = $1 and provider_subject = $2",
    )
    .bind(&chosen.provider)
    .bind(&chosen.provider_subject)
    .bind(to)
    .execute(&state.db)
    .await?;
    chosen.source_url = to.to_string();
    chosen.etag = None;
    chosen.last_modified = None;
    Ok(())
}

/// After a sign-in with a linked account (its `identities.picture_url` already updated): if
/// that's the chosen picture, bring Slop's copy up to date, waiting a few seconds at most.
pub async fn signed_in(state: &AppState, provider: Provider, subject: &str) {
    let work = async {
        let row: Option<(Chosen, Option<String>)> = sqlx::query_as::<_, (Uuid, String, String, String, Option<String>, Option<String>, Option<String>)>(
            "select a.user_id, a.provider, a.provider_subject, a.source_url, a.etag, a.last_modified, i.picture_url
             from user_avatars a join identities i on i.provider = a.provider and i.provider_subject = a.provider_subject
             where a.provider = $1 and a.provider_subject = $2",
        )
        .bind(provider.name())
        .bind(subject)
        .fetch_optional(&state.db)
        .await?
        .map(|(user_id, provider, provider_subject, source_url, etag, last_modified, now)| {
            (
                Chosen { user_id, provider, provider_subject, source_url, etag, last_modified },
                now,
            )
        });
        let Some((mut chosen, now)) = row else {
            return anyhow::Ok(());
        };
        if let Some(now) = now.filter(|n| *n != chosen.source_url) {
            moved(state, &mut chosen, &now).await?;
        }
        check(state, chosen).await?;
        Ok(())
    };
    match tokio::time::timeout(SIGN_IN_WAIT, work).await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => {
            tracing::warn!(error = %format!("{err:#}"), "updating a profile picture at sign-in")
        }
        Err(_) => tracing::info!("updating a profile picture at sign-in took too long"),
    }
}

/// Reads again the chosen pictures last checked over a day ago, a batch at a time (from the
/// hourly cleanup). Each is claimed first, so overlapping API instances don't read it twice.
pub async fn refresh_due(state: &AppState) -> anyhow::Result<usize> {
    let due: Vec<Chosen> = sqlx::query_as(
        "update user_avatars set checked_at = now() where user_id in (
             select user_id from user_avatars where checked_at < now() - $1
             order by checked_at limit $2 for update skip locked)
         returning user_id, provider, provider_subject, source_url, etag, last_modified",
    )
    .bind(REFRESH_AFTER)
    .bind(BATCH)
    .fetch_all(&state.db)
    .await?;
    let mut updated = 0;
    for (i, chosen) in due.into_iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(PAUSE).await;
        }
        match check(state, chosen).await {
            Ok(Checked::Updated) => updated += 1,
            Ok(_) => {}
            Err(err) => tracing::warn!(error = %format!("{err:#}"), "profile picture check"),
        }
    }
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbImage::from_fn(w, h, |x, y| image::Rgb([x as u8, y as u8, 90]))
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn pictures_are_re_encoded_as_small_squares() {
        let out = normalize(&png(300, 200)).unwrap();
        assert_eq!(sniff(&out), Some(ImageFormat::Png));
        let img = image::load_from_memory(&out).unwrap();
        assert_eq!((img.width(), img.height()), (SIZE, SIZE));
        // Whatever follows the picture in the file (a polyglot's payload) is gone.
        let mut polyglot = png(64, 64);
        polyglot.extend_from_slice(b"<script>alert(1)</script>");
        let out = normalize(&polyglot).unwrap();
        assert!(!out.windows(8).any(|w| w == b"<script>"));
    }

    #[test]
    fn only_known_formats_by_their_first_bytes() {
        assert_eq!(sniff(b"GIF89a...."), Some(ImageFormat::Gif));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(ImageFormat::WebP));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(ImageFormat::Jpeg));
        assert_eq!(sniff(b"<svg xmlns="), None);
        assert_eq!(sniff(b"BM\0\0"), None);
        assert!(normalize(b"<svg></svg>").is_err());
        assert!(normalize(b"\x89PNG\r\n\x1a\nnot really").is_err());
    }

    #[test]
    fn discord_pictures_and_defaults() {
        assert_eq!(
            discord_source(
                "80351110224678912",
                Some("8342729096ea3675442027381ff50dfe"),
                None
            )
            .as_deref(),
            Some(
                "https://cdn.discordapp.com/avatars/80351110224678912/8342729096ea3675442027381ff50dfe.png?size=128"
            )
        );
        // No picture: Discord's default, by the old tag or by the ID.
        assert_eq!(
            discord_source("80351110224678912", None, Some("1337")).as_deref(),
            Some("https://cdn.discordapp.com/embed/avatars/2.png")
        );
        let by_id = (80351110224678912u64 >> 22) % 6;
        assert_eq!(
            discord_source("80351110224678912", None, Some("0")),
            Some(format!(
                "https://cdn.discordapp.com/embed/avatars/{by_id}.png"
            ))
        );
        assert_eq!(
            discord_source("80351110224678912", Some("../x"), Some("0")),
            discord_source("80351110224678912", None, Some("0"))
        );
        assert_eq!(discord_source("not-a-number", None, None), None);
    }

    #[test]
    fn picture_addresses_must_be_web_addresses() {
        assert_eq!(
            clean_picture(Some(" https://lh3.googleusercontent.com/a/x=s96-c ")).as_deref(),
            Some("https://lh3.googleusercontent.com/a/x=s96-c")
        );
        assert_eq!(clean_picture(Some("javascript:alert(1)")), None);
        assert_eq!(clean_picture(Some("file:///etc/passwd")), None);
        assert_eq!(clean_picture(Some("")), None);
        assert_eq!(clean_picture(None), None);
        let long = format!("https://example.com/{}", "a".repeat(3000));
        assert_eq!(clean_picture(Some(&long)), None);
    }
}
