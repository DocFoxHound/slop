// SPDX-License-Identifier: AGPL-3.0-or-later
//! Known-bad links (SECURITY §11): once a day SLAPorSLOP downloads public lists of malware and
//! phishing addresses (phishunt.io and PhishDestroy by default, `SLOP_BAD_LINK_FEEDS`), and the hourly abuse checks
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

/// An address as the lists and SLAPorSLOP compare them: lower-case scheme and site, no fragment, no
/// trailing slash, `www.` and a dot after the site's name (`evil.example.`, the same site)
/// dropped. Returns (address, site).
pub fn normalize(raw: &str) -> Option<(String, String)> {
    let mut url = Url::parse(raw.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.set_fragment(None);
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host.trim_end_matches('.');
    let host = host.strip_prefix("www.").unwrap_or(host).to_string();
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
pub async fn find<'c>(
    db: impl sqlx::PgExecutor<'c>,
    urls: &[String],
) -> sqlx::Result<Vec<(String, String)>> {
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

/// The names the lists are shown by ("… is on PhishDestroy, a list of known-bad links"): the
/// lists SLAPorSLOP knows by their names, any other by its site, or its site and path when two
/// lists share a site (each list's copy is kept under its name).
pub fn list_names(feeds: &[Url]) -> Vec<String> {
    let host = |feed: &Url| feed.host_str().unwrap_or("a list").to_string();
    feeds
        .iter()
        .map(|feed| match feed.host_str() {
            Some("huggingface.co") if feed.path().starts_with("/datasets/phishdestroy/") => {
                "PhishDestroy".into()
            }
            Some("urlhaus.abuse.ch") => "URLhaus".into(),
            _ if feeds.iter().filter(|f| host(f) == host(feed)).count() > 1 => {
                format!("{}{}", host(feed), feed.path())
            }
            _ => host(feed),
        })
        .collect()
}

/// Whether this run of the API has tried to download the lists yet (for the admins' alert, so a
/// server that has just started isn't reported before it has had a go).
static TRIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Downloads each list again if its copy is over a day old, or missing. A list that can't be
/// downloaded keeps its last copy, is tried again on the next run, and doesn't hold up the others.
/// Returns whether any list was refreshed.
pub async fn refresh_if_due(state: &AppState) -> anyhow::Result<bool> {
    let feeds = &state.config.bad_link_feeds;
    if feeds.is_empty() {
        return Ok(false);
    }
    let names = list_names(feeds);
    // Lists that are no longer used (URLhaus, until 2026-11-19) go.
    sqlx::query("delete from bad_links where not (source = any($1))")
        .bind(&names)
        .execute(&state.db)
        .await?;
    let fresh: Vec<String> = sqlx::query_scalar(
        "select source from bad_links group by source having max(listed_at) > now() - $1::interval",
    )
    .bind(format!("{} hours", REFRESH_AFTER.whole_hours()))
    .fetch_all(&state.db)
    .await?;
    let mut refreshed = false;
    for (feed, name) in feeds.iter().zip(&names) {
        if fresh.contains(name) {
            continue;
        }
        TRIED.store(true, std::sync::atomic::Ordering::Relaxed);
        match fetch(state, feed).await {
            Ok((urls, hosts)) => {
                let mut tx = state.db.begin().await?;
                sqlx::query("delete from bad_links where source = $1")
                    .bind(name)
                    .execute(&mut *tx)
                    .await?;
                // The same address on two lists is kept once, under the first.
                sqlx::query(
                    "insert into bad_links (url, host, source)
                     select u, h, $3 from unnest($1::text[], $2::text[]) as t(u, h)
                     on conflict (url) do nothing",
                )
                .bind(&urls)
                .bind(&hosts)
                .bind(name)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                tracing::info!(list = %name, addresses = urls.len(), "bad-link list refreshed");
                refreshed = true;
            }
            Err(err) => {
                tracing::warn!(list = %name, error = %format!("{err:#}"), "couldn't refresh a bad-link list")
            }
        }
    }
    Ok(refreshed)
}

/// One list's addresses and their sites.
async fn fetch(state: &AppState, feed: &Url) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    let body = crate::media::download(state, feed.clone(), MAX_FEED_BYTES)
        .await?
        .ok_or_else(|| anyhow::anyhow!("{feed} isn't there"))?;
    let mut urls = Vec::new();
    let mut hosts = Vec::new();
    let mut seen = HashSet::new();
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
        }
    }
    // A list that came back empty is more likely broken than clean: keep the old copy.
    anyhow::ensure!(!urls.is_empty(), "{feed} came back empty");
    Ok((urls, hosts))
}

/// The lists that haven't been downloaded for two days, or ever, once this run has tried (ops.rs
/// tells the admins): links aren't being checked against them.
pub async fn stale(state: &AppState) -> sqlx::Result<Vec<String>> {
    let feeds = &state.config.bad_link_feeds;
    if feeds.is_empty() || !TRIED.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(vec![]);
    }
    let names = list_names(feeds);
    sqlx::query_scalar(
        "select n from unnest($1::text[]) as n
         where not exists (select 1 from bad_links where source = n and listed_at > now() - interval '2 days')",
    )
    .bind(&names)
    .fetch_all(&state.db)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_are_shown_by_their_names() {
        let feeds: Vec<Url> = [
            "https://phishunt.io/feed.txt",
            "https://huggingface.co/datasets/phishdestroy/destroylist/resolve/main/urls.txt",
            "https://lists.example/a.txt",
            "https://lists.example/b.txt",
            "https://other.example/list",
        ]
        .iter()
        .map(|u| u.parse().unwrap())
        .collect();
        assert_eq!(
            list_names(&feeds),
            [
                "phishunt.io",
                "PhishDestroy",
                "lists.example/a.txt",
                "lists.example/b.txt",
                "other.example"
            ]
        );
    }

    #[test]
    fn addresses_compare_as_people_would_expect() {
        assert_eq!(
            normalize("HTTPS://WWW.Evil.Example/Payload.exe/#x"),
            Some(("evil.example/Payload.exe".into(), "evil.example".into()))
        );
        assert_eq!(normalize("ftp://evil.example/x"), None);
        // A dot after the name is the same site, however it's written.
        assert_eq!(
            normalize("https://evil.example./"),
            Some(("evil.example".into(), "evil.example".into()))
        );
        assert_eq!(
            normalize("https://WWW.evil.example.:443/x"),
            Some(("evil.example/x".into(), "evil.example".into()))
        );
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
