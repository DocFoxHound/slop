//! Known-bad links (SECURITY §11): once a day Slop downloads public lists of malware and
//! phishing addresses (URLhaus by default, `SLOP_BAD_LINK_FEEDS`), and the hourly abuse checks
//! (abuse.rs) look for them in store pages and creative profiles. A match is an abuse signal for
//! a person to look at; nothing is removed automatically.
//!
//! An address matches when it's on a list exactly, or when its site is, unless the site is one
//! many people share (GitHub, Discord, Google Drive…): there, only the exact address counts.

use std::collections::HashSet;

use url::Url;

use crate::AppState;

/// Lists are refreshed after this long.
const REFRESH_AFTER: time::Duration = time::Duration::hours(24);
/// A list bigger than this is refused.
const MAX_FEED_BYTES: u64 = 64 * 1024 * 1024;

/// Sites many people share, where a list naming one file mustn't flag every link to the site.
const SHARED_HOSTS: &[&str] = &[
    "github.com",
    "raw.githubusercontent.com",
    "gist.github.com",
    "gist.githubusercontent.com",
    "objects.githubusercontent.com",
    "gitlab.com",
    "bitbucket.org",
    "codeberg.org",
    "discord.com",
    "discord.gg",
    "cdn.discordapp.com",
    "drive.google.com",
    "docs.google.com",
    "storage.googleapis.com",
    "dropbox.com",
    "dl.dropboxusercontent.com",
    "onedrive.live.com",
    "1drv.ms",
    "mediafire.com",
    "youtube.com",
    "youtu.be",
    "itch.io",
    "pastebin.com",
    "t.me",
    "bit.ly",
    "archive.org",
];

/// An address as the lists and Slop compare them: lower-case scheme and site, no fragment, no
/// trailing slash, `www.` dropped. Returns (address, site).
pub fn normalize(raw: &str) -> Option<(String, String)> {
    let mut url = Url::parse(raw.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.set_fragment(None);
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    let mut path = url.path().trim_end_matches('/').to_string();
    if let Some(q) = url.query() {
        path = format!("{path}?{q}");
    }
    let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
    Some((format!("{host}{port}{path}"), host))
}

fn shared(host: &str) -> bool {
    SHARED_HOSTS
        .iter()
        .any(|s| host == *s || host.ends_with(&format!(".{s}")))
}

/// The http(s) addresses in some text (Markdown links included).
pub fn urls_in(text: &str) -> Vec<String> {
    static RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r#"https?://[^\s<>"'\)\]]+"#).unwrap());
    let mut seen = HashSet::new();
    RE.find_iter(text)
        .map(|m| {
            m.as_str()
                .trim_end_matches(['.', ',', ';', ':', '!', '?'])
                .to_string()
        })
        .filter(|u| seen.insert(u.clone()))
        .take(200)
        .collect()
}

/// Which of `urls` are known bad: (the address as written, the list that names it).
pub async fn find(db: &sqlx::PgPool, urls: &[String]) -> sqlx::Result<Vec<(String, String)>> {
    let normal: Vec<(String, (String, String))> = urls
        .iter()
        .filter_map(|u| normalize(u).map(|n| (u.clone(), n)))
        .collect();
    if normal.is_empty() {
        return Ok(vec![]);
    }
    let addresses: Vec<String> = normal.iter().map(|(_, (a, _))| a.clone()).collect();
    let hosts: Vec<String> = normal
        .iter()
        .map(|(_, (_, h))| h.clone())
        .filter(|h| !shared(h))
        .collect();
    let listed: Vec<(String, String, String)> = sqlx::query_as(
        "select url, host, source from bad_links where url = any($1) or host = any($2)",
    )
    .bind(&addresses)
    .bind(&hosts)
    .fetch_all(db)
    .await?;
    Ok(normal
        .into_iter()
        .filter_map(|(written, (address, host))| {
            listed
                .iter()
                .find(|(u, h, _)| *u == address || (*h == host && !shared(&host)))
                .map(|(_, _, source)| (written, source.clone()))
        })
        .collect())
}

/// Downloads the lists again if they're over a day old. Returns whether it did.
pub async fn refresh_if_due(state: &AppState) -> anyhow::Result<bool> {
    let feeds = &state.config.bad_link_feeds;
    if feeds.is_empty() {
        return Ok(false);
    }
    let newest: Option<time::OffsetDateTime> =
        sqlx::query_scalar("select max(listed_at) from bad_links")
            .fetch_one(&state.db)
            .await?;
    if newest.is_some_and(|t| time::OffsetDateTime::now_utc() - t < REFRESH_AFTER) {
        return Ok(false);
    }
    let mut urls: Vec<String> = Vec::new();
    let mut hosts: Vec<String> = Vec::new();
    let mut sources: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for feed in feeds {
        let body = crate::media::download(state, feed.clone(), MAX_FEED_BYTES)
            .await?
            .ok_or_else(|| anyhow::anyhow!("{feed} isn't there"))?;
        let source = feed.host_str().unwrap_or("list").to_string();
        for line in String::from_utf8_lossy(&body).lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((url, host)) = normalize(line)
                && seen.insert(url.clone())
            {
                urls.push(url);
                hosts.push(host);
                sources.push(source.clone());
            }
        }
    }
    // A list that came back empty is more likely broken than clean: keep the old one.
    anyhow::ensure!(!urls.is_empty(), "the bad-link lists came back empty");
    let mut tx = state.db.begin().await?;
    sqlx::query("delete from bad_links")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "insert into bad_links (url, host, source) select * from unnest($1::text[], $2::text[], $3::text[])",
    )
    .bind(&urls)
    .bind(&hosts)
    .bind(&sources)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    tracing::info!(addresses = urls.len(), "bad-link lists refreshed");
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_compare_as_people_would_expect() {
        assert_eq!(
            normalize("HTTPS://WWW.Evil.Example/Payload.exe/#x"),
            Some(("evil.example/Payload.exe".into(), "evil.example".into()))
        );
        assert_eq!(normalize("ftp://evil.example/x"), None);
        assert_eq!(
            urls_in(
                "See [the site](https://a.example/x). Or https://b.example/y, and https://a.example/x again"
            ),
            ["https://a.example/x", "https://b.example/y"]
        );
        assert!(
            shared("raw.githubusercontent.com") && shared("x.itch.io") && !shared("evil.example")
        );
    }
}
