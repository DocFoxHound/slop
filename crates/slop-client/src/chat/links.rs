//! Links in messages. The sender's app fetches what a link points to, so recipients' apps never
//! contact the site (and it never learns who read the message):
//!
//! - a link straight to a picture, GIF or short video (a meme) is downloaded and sent as an
//!   encrypted attachment, shown inline;
//! - a page gets a preview card from its Open Graph tags (title, description, picture).
//!
//! Only public http(s) addresses are fetched, with size and time limits.

use std::{net::IpAddr, time::Duration};

use url::Url;

use crate::{Error, Result};

/// The most a linked picture or video may be.
pub const MAX_MEDIA: usize = 10 * 1024 * 1024;
/// The most of a page read for its preview.
const MAX_PAGE: usize = 1024 * 1024;
/// A preview's picture.
const MAX_PREVIEW_IMAGE: usize = 2 * 1024 * 1024;

/// Kinds of media shown inline in chat.
pub fn is_inline_media(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "video/mp4" | "video/webm"
    )
}

/// The first http(s) link in a text.
pub fn first_link(text: &str) -> Option<Url> {
    text.split(|c: char| c.is_whitespace() || c == '<' || c == '>')
        .map(|w| w.trim_start_matches(|c: char| "([{'\"".contains(c)))
        .filter(|w| w.starts_with("https://") || w.starts_with("http://"))
        .map(|w| w.trim_end_matches(|c: char| ".,;:!?)]}'\"".contains(c)))
        .find_map(|w| Url::parse(w).ok())
}

/// What a link turned out to be.
pub enum Fetched {
    Media {
        mime: String,
        bytes: Vec<u8>,
        name: String,
    },
    Page(PagePreview),
}

#[derive(Debug, Default, PartialEq)]
pub struct PagePreview {
    pub title: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<Url>,
}

fn private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1])
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Only public web addresses: no local network, no other schemes.
pub async fn check_public(url: &Url) -> Result<()> {
    if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() {
        return Err(Error::Install("only web links get previews".into()));
    }
    let host = url
        .host_str()
        .ok_or_else(|| Error::Install("no host".into()))?;
    let port = url.port_or_known_default().unwrap_or(443);
    let addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| Error::Network(e.to_string()))?;
    for addr in addrs {
        if private_ip(addr.ip()) {
            return Err(Error::Install("that link is on a private network".into()));
        }
    }
    Ok(())
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(3))
        .user_agent("Mozilla/5.0 (compatible; SlopLinkPreview/1.0)")
        .build()
        .expect("http client")
}

async fn read_limited(mut res: reqwest::Response, max: usize) -> Result<Vec<u8>> {
    if res.content_length().is_some_and(|l| l as usize > max) {
        return Err(Error::Install("too large".into()));
    }
    let mut out = Vec::new();
    while let Some(chunk) = res.chunk().await? {
        if out.len() + chunk.len() > max {
            return Err(Error::Install("too large".into()));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// Fetches a link: the media itself, or a page's preview.
pub async fn fetch(url: &Url) -> Result<Fetched> {
    check_public(url).await?;
    let res = client().get(url.clone()).send().await?;
    // Redirects could lead to a private address.
    check_public(res.url()).await?;
    if !res.status().is_success() {
        return Err(Error::Install(format!(
            "the site answered {}",
            res.status()
        )));
    }
    let mime = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
        .unwrap_or_default();
    if is_inline_media(&mime) {
        let name = url
            .path_segments()
            .and_then(|mut s| s.next_back())
            .filter(|n| !n.is_empty())
            .unwrap_or("linked")
            .to_string();
        let bytes = read_limited(res, MAX_MEDIA).await?;
        return Ok(Fetched::Media { mime, bytes, name });
    }
    if mime != "text/html" && mime != "application/xhtml+xml" {
        return Ok(Fetched::Page(PagePreview::default()));
    }
    let base = res.url().clone();
    let bytes = read_limited(res, MAX_PAGE).await.unwrap_or_default();
    let html = String::from_utf8_lossy(&bytes);
    Ok(Fetched::Page(parse_preview(&html, &base)))
}

/// A preview's picture, if it's a picture and small enough.
pub async fn fetch_image(url: &Url) -> Result<(String, Vec<u8>)> {
    check_public(url).await?;
    let res = client().get(url.clone()).send().await?;
    check_public(res.url()).await?;
    let mime = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
        .unwrap_or_default();
    if !mime.starts_with("image/") || !is_inline_media(&mime) {
        return Err(Error::Install("not a picture".into()));
    }
    Ok((mime, read_limited(res, MAX_PREVIEW_IMAGE).await?))
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
}

/// An attribute's value in one tag (`name="value"` or `name='value'`).
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find(name) {
        let at = from + pos;
        from = at + name.len();
        let before = lower[..at].chars().last();
        if !matches!(before, Some(c) if c.is_whitespace()) {
            continue;
        }
        let rest = tag[at + name.len()..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let quote = rest.chars().next()?;
        if quote == '"' || quote == '\'' {
            let end = rest[1..].find(quote)?;
            return Some(unescape(&rest[1..1 + end]));
        }
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(rest.len());
        return Some(unescape(&rest[..end]));
    }
    None
}

fn clip(s: String, max: usize) -> Option<String> {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.is_empty() {
        return None;
    }
    Some(if s.chars().count() > max {
        format!("{}…", s.chars().take(max).collect::<String>())
    } else {
        s
    })
}

/// The Open Graph (or Twitter card, or plain) title, description and picture of a page.
pub fn parse_preview(html: &str, base: &Url) -> PagePreview {
    let head = &html[..html.len().min(256 * 1024)];
    let mut og = std::collections::HashMap::new();
    let lower = head.to_ascii_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find("<meta") {
        let start = from + pos;
        let end = lower[start..]
            .find('>')
            .map(|e| start + e)
            .unwrap_or(lower.len());
        let tag = &head[start..end];
        from = end;
        let key = attr(tag, "property").or_else(|| attr(tag, "name"));
        if let (Some(key), Some(content)) = (key, attr(tag, "content")) {
            og.entry(key.to_ascii_lowercase()).or_insert(content);
        }
    }
    let title_tag = lower.find("<title").and_then(|s| {
        let open = s + lower[s..].find('>')? + 1;
        let close = open + lower[open..].find("</title")?;
        Some(unescape(&head[open..close]))
    });
    let pick = |keys: &[&str]| keys.iter().find_map(|k| og.get(*k).cloned());
    PagePreview {
        title: pick(&["og:title", "twitter:title"])
            .or(title_tag)
            .and_then(|t| clip(t, 200)),
        description: pick(&["og:description", "twitter:description", "description"])
            .and_then(|d| clip(d, 300)),
        image_url: pick(&["og:image", "og:image:url", "twitter:image"])
            .and_then(|i| base.join(&i).ok())
            .filter(|u| matches!(u.scheme(), "http" | "https")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_first_link() {
        assert_eq!(
            first_link("look (https://example.com/a.gif).")
                .unwrap()
                .as_str(),
            "https://example.com/a.gif"
        );
        assert!(first_link("no links, javascript:alert(1)").is_none());
        assert!(first_link("ftp://example.com").is_none());
    }

    #[test]
    fn reads_open_graph_tags() {
        let html = r#"<html><head><title>Plain &amp; simple</title>
            <meta property="og:title" content="The &quot;real&quot; title">
            <meta name='description' content='Desc   here'>
            <meta property="og:image" content="/pic.png"></head></html>"#;
        let p = parse_preview(html, &Url::parse("https://site.example/page").unwrap());
        assert_eq!(p.title.as_deref(), Some("The \"real\" title"));
        assert_eq!(p.description.as_deref(), Some("Desc here"));
        assert_eq!(
            p.image_url.unwrap().as_str(),
            "https://site.example/pic.png"
        );
        let p = parse_preview(
            "<title>Only a title</title>",
            &Url::parse("https://a.example").unwrap(),
        );
        assert_eq!(p.title.as_deref(), Some("Only a title"));
    }

    #[test]
    fn private_addresses_are_refused() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "::1",
            "fd00::1",
        ] {
            assert!(private_ip(ip.parse().unwrap()), "{ip}");
        }
        assert!(!private_ip("93.184.216.34".parse().unwrap()));
    }
}
