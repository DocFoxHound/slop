// SPDX-License-Identifier: AGPL-3.0-or-later
//! SLAPorSLOP's own look at an app's source, at the commit a release was built from (DESIGN §10's
//! "Licence" and "Monetization rules" rows, LEGAL-GUIDANCE IP-2). SLAPorSLOP downloads the
//! repository's archive at that commit once, reads it in memory, and deletes it. What it looks
//! for (licence, monetization, code from outside SLAPorSLOP, online play, saves, packages) is in
//! the `slop-source-scan` crate, which `slop-source-check` also runs on makers' GitHub runners:
//! a runner's result (a `RawScan`) becomes a report here with `finish_raw`. This module adds
//! what depends on the store page and stores the result:
//!
//! - **Online play:** an app whose store page doesn't say it goes online gets a note, since
//!   players are told about servers SLAPorSLOP can't check.
//! - **Packages:** the dependencies the lockfiles name, kept for the daily advisory watch
//!   (dep_watch.rs).
//!
//! Everything here is a note for the reviewer and the maker: patterns in text have false
//! positives, so nothing is held or rejected by itself.
//!
//! SLAPorSLOP reads repositories on GitHub, GitLab and Codeberg (and other GitLab or Gitea servers
//! listed in `SLOP_SOURCE_HOSTS`, or whose maker saved a read-only token), through each host's
//! archive API. Private ones are read with the maker's token (repo_tokens.rs). Anything else is
//! recorded as not checked, for a reviewer to look at by hand.

use std::collections::BTreeSet;

use anyhow::{Context, bail};
use serde_json::{Value, json};
use uuid::Uuid;

use url::Url;

use crate::{AppState, repo_tokens::Kind};
pub use slop_source_scan::{
    Hit, Licence, LicenceFile, Package, RawScan, Report, expression_ids, finalize,
    identify_licence, not_osi_approved, parse_lockfile, scan_archive, scan_archive_raw,
};

/// Largest repository archive SLAPorSLOP downloads (hosts' archives leave out Git LFS files).
const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
const EVERY: std::time::Duration = std::time::Duration::from_secs(120);
const PER_ROUND: i64 = 3;

/// The report a successful check stores (status `checked`) for a raw scan, with the store page's
/// licence and visibility applied, and the package list. For a `RawScan` from a maker's runner
/// (`slop-source-check`): call `RawScan::validate` first and refuse it if that fails. The result
/// is what `scan_release` gives for the same source; `flag_for_app` then adds what depends on
/// the rest of the store page.
/// Keeps the source check a maker's GitHub runner sent for `commit` (slop_build.rs), after
/// checking it's well-formed.
pub async fn record_from_runner(
    db: &sqlx::PgPool,
    app_id: Uuid,
    commit: &str,
    check: &slop_source_scan::SourceCheck,
) -> Result<(), &'static str> {
    check.raw.validate()?;
    if check.tool.chars().count() > 100 {
        return Err("the tool's name is too long");
    }
    sqlx::query(
        "insert into ci_source_scans (app_id, commit_sha, raw, tool) values ($1, $2, $3, $4)
         on conflict (app_id, commit_sha) do update set raw = excluded.raw, tool = excluded.tool,
             received_at = now()",
    )
    .bind(app_id)
    .bind(commit.to_ascii_lowercase())
    .bind(sqlx::types::Json(&check.raw))
    .bind(&check.tool)
    .execute(db)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "keeping a source check from GitHub");
        "couldn't be kept"
    })?;
    Ok(())
}

/// The source check of the app's repository at `commit` from its GitHub runner, if one came.
async fn from_runner(db: &sqlx::PgPool, app_id: Uuid, commit: &str) -> Option<RawScan> {
    let raw: Option<Value> =
        sqlx::query_scalar("select raw from ci_source_scans where app_id = $1 and commit_sha = $2")
            .bind(app_id)
            .bind(commit.to_ascii_lowercase())
            .fetch_optional(db)
            .await
            .ok()?;
    let raw: RawScan = serde_json::from_value(raw?).ok()?;
    raw.validate().is_ok().then_some(raw)
}

pub fn finish_raw(raw: RawScan, declared: &str, public: bool) -> (Value, Vec<Package>) {
    let (report, packages) = finalize(raw, declared, public);
    (serde_json::to_value(report).unwrap_or_default(), packages)
}

/// The repository is bigger than SLAPorSLOP reads: recorded as not checked, not as a failure.
#[derive(Debug, thiserror::Error)]
#[error("the repository is bigger than the {} MiB SLAPorSLOP reads, so its licence isn't checked automatically", MAX_ARCHIVE / 1024 / 1024)]
struct TooBig;

/// Where a repository's archive at a commit comes from.
#[derive(Debug, PartialEq, Eq)]
pub struct Source {
    /// The host's name for messages ("GitHub", "GitLab", "Codeberg").
    pub host: String,
    pub url: Url,
    /// Whether it's GitHub's API, asked with SLAPorSLOP's GitHub App or token.
    pub github: bool,
}

/// The archive's address on a GitLab or Gitea host: `https://host/group/sub/repo` at `commit`.
pub fn archive_url(kind: Kind, repo_url: &Url, commit: &str) -> Option<Url> {
    let path = repo_url.path().trim_matches('/').trim_end_matches(".git");
    if !path.contains('/') {
        return None;
    }
    let origin = repo_url.origin().ascii_serialization();
    let url = match kind {
        Kind::Gitlab => format!(
            "{origin}/api/v4/projects/{}/repository/archive.tar.gz?sha={commit}",
            percent_encoding::utf8_percent_encode(path, percent_encoding::NON_ALPHANUMERIC)
        ),
        Kind::Gitea => format!("{origin}/api/v1/repos/{path}/archive/{commit}.tar.gz"),
        // Bitbucket Cloud's download link for a commit.
        Kind::Bitbucket => format!("{origin}/{path}/get/{commit}.tar.gz"),
    };
    Url::parse(&url).ok()
}

/// What software a self-hosted server runs, by asking it: Gitea and Forgejo answer
/// `/api/v1/version`, GitLab its project API for a public project. Never on a private network,
/// and in tests only on this machine (tests never reach the internet).
async fn detect(state: &AppState, repo: &Url) -> Option<Kind> {
    let host = repo.host_str()?;
    if state.config.releases.allow_http_downloads && !matches!(host, "127.0.0.1" | "localhost") {
        return None;
    }
    let origin = repo.origin().ascii_serialization();
    let path = repo.path().trim_matches('/').trim_end_matches(".git");
    let ask = |url: String, key: &'static str| async move {
        let url = Url::parse(&url).ok()?;
        crate::repo_owner::public_host(state, &url).await.ok()?;
        let res = state
            .http
            .get(url)
            .header("user-agent", "slop-api")
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .ok()?;
        if !res.status().is_success() {
            return None;
        }
        let body: Value = res.json().await.ok()?;
        body[key].is_string().then_some(())
    };
    if ask(format!("{origin}/api/v1/version"), "version")
        .await
        .is_some()
    {
        return Some(Kind::Gitea);
    }
    let project = percent_encoding::utf8_percent_encode(path, percent_encoding::NON_ALPHANUMERIC);
    if ask(
        format!("{origin}/api/v4/projects/{project}"),
        "path_with_namespace",
    )
    .await
    .is_some()
    {
        return Some(Kind::Gitlab);
    }
    None
}

/// Where to read a release's source, or None for hosts SLAPorSLOP can't read.
async fn source(
    state: &AppState,
    app_id: Uuid,
    repo_url: &str,
    commit: &str,
) -> anyhow::Result<Option<Source>> {
    // A commit is 40 (or 64) hex digits; anything else never reaches a URL.
    anyhow::ensure!(
        !commit.is_empty() && commit.chars().all(|c| c.is_ascii_hexdigit()),
        "the release's commit isn't a commit ID"
    );
    if let Some(repo) = slop_provenance::github_repo_path(repo_url) {
        return github_source(state, &repo, commit).map(Some);
    }
    let Ok(url) = Url::parse(repo_url) else {
        return Ok(None);
    };
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    // A token the maker saved says what the host is (self-hosted servers); then the list; then
    // asking the server itself.
    let saved = crate::repo_tokens::saved_kind(&state.db, app_id).await?;
    // Development and tests: a stand-in GitHub serves repositories under its own address.
    let base = state.config.releases.github_api_base.as_str();
    let stand_in = state
        .config
        .releases
        .allow_http_downloads
        .then(|| repo_url.strip_prefix(base))
        .flatten()
        .map(|p| p.trim_matches('/').to_string())
        .filter(|p| p.split('/').count() == 2);
    if let (None, Some(repo)) = (saved, stand_in) {
        return github_source(state, &repo, commit).map(Some);
    }
    let kind = match saved {
        Some(kind) => Some(kind),
        None => match state
            .config
            .releases
            .source_hosts
            .iter()
            .find(|(h, _)| *h == host)
            .map(|(_, k)| *k)
        {
            Some(k) => Some(k),
            None => detect(state, &url).await,
        },
    };
    if let Some(archive) = kind.and_then(|k| archive_url(k, &url, commit)) {
        let name = match (host.as_str(), kind) {
            ("codeberg.org", _) => "Codeberg".to_string(),
            ("bitbucket.org", _) => "Bitbucket".to_string(),
            (_, Some(Kind::Gitlab)) => "GitLab".to_string(),
            _ => host.clone(),
        };
        return Ok(Some(Source {
            host: name,
            url: archive,
            github: false,
        }));
    }
    Ok(None)
}

fn github_source(state: &AppState, repo: &str, commit: &str) -> anyhow::Result<Source> {
    Ok(Source {
        host: "GitHub".into(),
        url: state
            .config
            .releases
            .github_api_base
            .join(&format!("repos/{repo}/tarball/{commit}"))?,
        github: true,
    })
}

/// Downloads the repository archive to a temporary file.
async fn download_archive(
    state: &AppState,
    app_id: Uuid,
    source: &Source,
    path: &std::path::Path,
) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    let host = &source.host;
    let request = if source.github {
        crate::releases::github_get(state, source.url.clone()).await
    } else {
        // Self-hosted servers: never one on a private network.
        crate::repo_owner::public_host(state, &source.url).await?;
        let mut req = state
            .fetch_http
            .get(source.url.clone())
            .header("user-agent", "slop-api");
        // Private repositories: the maker's read-only token, only ever sent to its own host.
        if let Some(auth) = crate::repo_tokens::auth_for(state, app_id, source.url.as_str()).await?
        {
            req = req.header("authorization", auth);
        }
        req
    };
    let res = request.send().await.context("downloading the source")?;
    match res.status().as_u16() {
        200 => {}
        401 | 403 | 404 => bail!(
            "{host} can't find that commit (or the repository isn't visible to SLAPorSLOP; for a private repository, save a read-only token)"
        ),
        code => bail!("{host} answered {code} for the source archive"),
    }
    let mut out = tokio::fs::File::create(path).await?;
    let mut size = 0u64;
    let mut res = res;
    while let Some(chunk) = res.chunk().await.context("source download interrupted")? {
        size += chunk.len() as u64;
        if size > MAX_ARCHIVE {
            return Err(TooBig.into());
        }
        out.write_all(&chunk).await?;
    }
    out.flush().await?;
    Ok(())
}

/// Looks at one release's source. Returns the report to store and the package list.
pub async fn scan_release(
    state: &AppState,
    app_id: Uuid,
    repo_url: &str,
    commit: &str,
    declared: &str,
    public: bool,
) -> (Value, Vec<Package>) {
    let path = std::env::temp_dir().join(format!("slop-source-{}.tar.gz", Uuid::now_v7()));
    let result = async {
        let Some(source) = source(state, app_id, repo_url, commit).await? else {
            return Ok(None);
        };
        download_archive(state, app_id, &source, &path).await?;
        // On the scanning server, where there is one.
        if let Some(read) = state.config.releases.scanner.scan_source(&path).await {
            return read.map(Some);
        }
        let file = std::fs::File::open(&path)?;
        tokio::task::spawn_blocking(move || scan_archive_raw(std::io::BufReader::new(file)))
            .await?
            .map(Some)
    }
    .await;
    let _ = tokio::fs::remove_file(&path).await;
    match result {
        Ok(Some(raw)) => finish_raw(raw, declared, public),
        Ok(None) => (
            json!({ "status": "not_checked", "detail": "SLAPorSLOP can only read source on GitHub, GitLab, Bitbucket, Codeberg and other Gitea or Forgejo servers so far, so the licence isn't checked automatically." }),
            vec![],
        ),
        Err(err) if err.downcast_ref::<TooBig>().is_some() => (
            json!({ "status": "not_checked", "detail": err.to_string() }),
            vec![],
        ),
        Err(err) => (
            json!({ "status": "failed", "detail": format!("{err:#}") }),
            vec![],
        ),
    }
}

/// One round: releases waiting for their source check, and failed ones once an hour. A release
/// its maker can't publish yet without a result (on hold or ready, publish.rs) is tried for as
/// long as it takes: after a week of failures it's recorded as not checked (`check_release`),
/// which counts as a result. Published ones are tried for a week after they were sent.
pub async fn run_once(state: &AppState) -> anyhow::Result<usize> {
    let due: Vec<(Uuid, String, String, String, bool)> = sqlx::query_as(
        "select r.id, a.repo_url, r.commit_sha, a.license_spdx, a.visibility = 'public'
         from releases r join apps a on a.id = r.app_id
         where r.status in ('awaiting_review', 'ready', 'approved')
           and (r.repo_scanned_at is null
                or (r.repo_scan ->> 'status' = 'failed' and r.repo_scanned_at < now() - interval '1 hour'
                    and (r.status in ('awaiting_review', 'ready')
                         or r.created_at > now() - interval '7 days')))
         order by r.repo_scanned_at nulls first, r.created_at desc
         limit $1",
    )
    .bind(PER_ROUND)
    .fetch_all(&state.db)
    .await?;
    let n = due.len();
    for (id, ..) in due {
        check_release(state, id).await?;
    }
    Ok(n)
}

/// What the source check reads from an app's store page.
pub struct StorePage {
    pub repo_url: String,
    /// The licence it names.
    pub declared: String,
    pub public: bool,
    /// It says the app goes online.
    online: bool,
    sandbox_ready: bool,
    is_mod: bool,
    dedicated_server: bool,
}

impl StorePage {
    pub async fn of<'e>(db: impl sqlx::PgExecutor<'e>, app_id: Uuid) -> sqlx::Result<Self> {
        type Row = (String, String, bool, bool, bool, bool, bool);
        let (repo_url, declared, public, online, sandbox_ready, is_mod, dedicated_server): Row =
            sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "select a.repo_url, a.license_spdx, a.visibility = 'public',
                        cardinality(a.online) > 0, a.sandbox_ready, a.branch_kind is not distinct from 'addon',
                        {}
                 from apps a where a.id = $1",
                crate::catalog::IS_DEDICATED_SERVER
            )))
            .bind(app_id)
            .fetch_one(db)
            .await?;
        Ok(Self {
            repo_url,
            declared,
            public,
            online,
            sandbox_ready,
            is_mod,
            dedicated_server,
        })
    }
}

/// What `flag_for_app` adds to a report: all of it depends on the store page (and, for Pulse, on
/// the original game), never on the source.
const PAGE_FLAGS: [&str; 6] = [
    "online_undeclared",
    "sandbox_mismatch",
    "mod_online",
    "mod_code",
    "pulse_missing",
    "dedicated_server",
];

/// A stored report with what depends on the store page judged again against the page as it is
/// now (`flag_for_app` only ever sets its flags, so they're cleared first): fixing the store page
/// is then enough for those (fixes.rs `of_release`, which Publish uses). The stored report isn't
/// changed.
pub async fn flagged_now(
    conn: &mut sqlx::PgConnection,
    app_id: Uuid,
    stored: &Value,
) -> sqlx::Result<Value> {
    let mut report = stored.clone();
    let Some(fields) = report.as_object_mut() else {
        return Ok(report);
    };
    for flag in PAGE_FLAGS {
        fields.remove(flag);
    }
    let page = StorePage::of(&mut *conn, app_id).await?;
    flag_for_app(conn, app_id, &page, &mut report).await?;
    Ok(report)
}

/// Adds to a scan's report what depends on the app's store page as it is now (`PAGE_FLAGS`).
pub async fn flag_for_app(
    conn: &mut sqlx::PgConnection,
    app_id: Uuid,
    page: &StorePage,
    report: &mut Value,
) -> sqlx::Result<()> {
    // Players are told about apps that go online; one that doesn't say so needs a look.
    if !page.online && report["network"].as_array().is_some_and(|n| !n.is_empty()) {
        report["online_undeclared"] = json!(true);
    }
    // A mod may carry code (DESIGN §9): flagged for a full read, and one that goes online,
    // declared or not, is flagged and suggests sandbox mode for its game.
    if page.is_mod && report["status"] == "checked" {
        if report["code_files"]
            .as_array()
            .is_some_and(|c| !c.is_empty())
        {
            report["mod_code"] = json!(true);
        }
        if report["network"].as_array().is_some_and(|n| !n.is_empty()) {
            report["mod_online"] = json!(true);
        }
    }
    // A community build of a game that counts players must count them too (DESIGN §9).
    if report["status"] == "checked"
        && report["pulse"] == "none"
        && crate::branches::needs_pulse(conn, app_id).await?
    {
        report["pulse_missing"] = json!(true);
    }
    // "Works in sandbox mode" means local saves only in SLOP_SAVE_DIR.
    if page.sandbox_ready && report["save_folder"] == "elsewhere" {
        report["sandbox_mismatch"] = json!(true);
    }
    // Whether it was a dedicated server when it was checked: the rule on code from outside
    // SLAPorSLOP doesn't cover servers, and the maker chooses the tag, so an app that becomes
    // one is pointed out to its maker and logged (`new_concerns`).
    report["dedicated_server"] = json!(page.dedicated_server);
    Ok(())
}

/// Failed tries to read a release's source, from the first, after which it's recorded as not
/// checked: Publish waits for a result (publish.rs), and the maker can't fix the host.
const GIVE_UP_AFTER: time::Duration = time::Duration::days(7);
/// The reason then, shown to the maker as information.
pub const UNREADABLE_FOR_A_WEEK: &str = "SLAPorSLOP couldn't read this release's source for a week";

/// Checks one release's source now and stores the result. Called when a release's files are
/// fetched, before its checks end (publish.rs), and by the background round for the rest. A
/// failure keeps when the failures began (`failing_since`); a week of them is recorded as not
/// checked (`UNREADABLE_FOR_A_WEEK`).
pub async fn check_release(state: &AppState, id: Uuid) -> anyhow::Result<()> {
    let (app_id, commit, before): (Uuid, String, Option<Value>) =
        sqlx::query_as("select app_id, commit_sha, repo_scan from releases where id = $1")
            .bind(id)
            .fetch_one(&state.db)
            .await?;
    let page = StorePage::of(&state.db, app_id).await?;
    {
        // Read on the maker's GitHub runner if it sent it; else the repository is downloaded.
        let (mut report, packages) = match from_runner(&state.db, app_id, &commit).await {
            Some(raw) => {
                let (mut report, packages) = finish_raw(raw, &page.declared, page.public);
                report["checked_on"] = json!("github");
                (report, packages)
            }
            None => {
                scan_release(
                    state,
                    app_id,
                    &page.repo_url,
                    &commit,
                    &page.declared,
                    page.public,
                )
                .await
            }
        };
        flag_for_app(&mut *state.db.acquire().await?, app_id, &page, &mut report).await?;
        if report["status"] == "failed" {
            let now = time::OffsetDateTime::now_utc();
            let since = before
                .as_ref()
                .filter(|b| b["status"] == "failed")
                .and_then(|b| b["failing_since"].as_str())
                .and_then(|t| {
                    time::OffsetDateTime::parse(t, &time::format_description::well_known::Rfc3339)
                        .ok()
                })
                .unwrap_or(now);
            if now - since >= GIVE_UP_AFTER {
                report = json!({
                    "status": "not_checked",
                    "detail": format!("{UNREADABLE_FOR_A_WEEK} ({}).", report["detail"].as_str().unwrap_or("no details").trim_end_matches('.')),
                });
            } else {
                report["failing_since"] = json!(
                    since
                        .format(&time::format_description::well_known::Rfc3339)
                        .unwrap_or_default()
                );
            }
        }
        let mut tx = state.db.begin().await?;
        sqlx::query("update releases set repo_scan = $2, repo_scanned_at = now() where id = $1")
            .bind(id)
            .bind(&report)
            .execute(&mut *tx)
            .await?;
        if !packages.is_empty() {
            sqlx::query("delete from release_packages where release_id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "insert into release_packages (release_id, ecosystem, name, version)
                 select $1, * from unnest($2::text[], $3::text[], $4::text[]) on conflict do nothing",
            )
            .bind(id)
            .bind(packages.iter().map(|p| p.ecosystem.clone()).collect::<Vec<_>>())
            .bind(packages.iter().map(|p| p.name.clone()).collect::<Vec<_>>())
            .bind(packages.iter().map(|p| p.version.clone()).collect::<Vec<_>>())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
    }
    Ok(())
}

/// The concerns in a check (monetization rules, licence problem kinds), or None when the source
/// couldn't be read.
fn concerns(scan: &Value) -> Option<BTreeSet<String>> {
    match scan["status"].as_str() {
        Some("checked") => {}
        Some("not_checked") => return Some(BTreeSet::new()),
        _ => return None,
    }
    let strings = |v: &Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str().map(String::from))
            .collect()
    };
    let mut out: BTreeSet<String> = strings(&scan["licence"]["kinds"])
        .into_iter()
        .map(|k| format!("licence:{k}"))
        .collect();
    if scan["online_undeclared"] == json!(true) {
        out.insert("online:undeclared".into());
    }
    if scan["sandbox_mismatch"] == json!(true) {
        out.insert("saves:sandbox_mismatch".into());
    }
    for flag in ["mod_code", "mod_online"] {
        if scan[flag] == json!(true) {
            out.insert(format!("mod:{flag}"));
        }
    }
    for (section, key) in [("monetization", "monetization"), ("code_loading", "code")] {
        for hit in scan[section].as_array().into_iter().flatten() {
            if let Some(rule) = hit["rule"].as_str() {
                out.insert(format!("{key}:{rule}"));
            }
        }
    }
    Some(out)
}

/// The kinds of concern in `new` (keys from `concerns`), in words. Every prefix `concerns`
/// makes is named here; one it doesn't know still reads as something new, never a panic (this
/// runs inside the fetcher).
fn new_kinds(new: &[String]) -> String {
    let kinds: Vec<&str> = [
        ("monetization:", "possible purchases or loot boxes"),
        ("code:", "code loading from outside SLAPorSLOP"),
        ("online:", "online play the store page doesn't mention"),
        ("licence:", "licence problems"),
        (
            "saves:",
            "saves outside the save folder while it's marked as working in sandbox mode",
        ),
        ("mod:", "code or networking in a mod"),
    ]
    .into_iter()
    .filter(|(prefix, _)| new.iter().any(|k| k.starts_with(prefix)))
    .map(|(_, what)| what)
    .collect();
    match kinds.as_slice() {
        [] => "something new".into(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// What a release's source check found that the app's last published release didn't have (a new
/// monetization pattern or licence problem), that its source couldn't be read, or that it's a
/// dedicated server now and wasn't when that release was checked. Shown to its maker before they
/// publish it and logged (publish.rs): information, which on its own doesn't stop Publish (a
/// `must` fix does, fixes.rs `of_release`). None when there's nothing new.
pub async fn new_concerns(
    conn: &mut sqlx::PgConnection,
    release_id: Uuid,
) -> anyhow::Result<Option<String>> {
    type Row = (Option<Value>, Option<Value>, Option<String>);
    let (this, before, last_version): Row = sqlx::query_as(
        "select r.repo_scan, p.repo_scan, p.version
         from releases r
         left join lateral (select p.repo_scan, p.version from releases p
                            where p.app_id = r.app_id and p.id <> r.id and p.status = 'approved'
                            order by p.published_at desc nulls last limit 1) p on true
         where r.id = $1",
    )
    .bind(release_id)
    .fetch_one(conn)
    .await?;
    let Some(now) = concerns(this.as_ref().unwrap_or(&Value::Null)) else {
        return Ok(Some(
            "SLAPorSLOP couldn't read this release's source".into(),
        ));
    };
    // The first release: everything it has is new, and it can't have become anything.
    let Some(last_version) = last_version else {
        return Ok((!now.is_empty()).then(|| {
            format!(
                "the source check found {}",
                new_kinds(&now.into_iter().collect::<Vec<_>>())
            )
        }));
    };
    let is_server = |scan: Option<&Value>| scan.is_some_and(|s| s["dedicated_server"] == true);
    let became_server = is_server(this.as_ref()) && !is_server(before.as_ref());
    let server = "it's now in the Servers category";
    let before = before.as_ref().and_then(concerns).unwrap_or_default();
    let new: Vec<String> = now.difference(&before).cloned().collect();
    if new.is_empty() {
        return Ok(became_server.then(|| server.to_string()));
    }
    let found = format!(
        "the source check found {} that {last_version} didn't have",
        new_kinds(&new)
    );
    Ok(Some(if became_server {
        format!("{server}, and {found}")
    } else {
        found
    }))
}

pub async fn run(state: AppState) {
    let mut tick = tokio::time::interval(EVERY);
    loop {
        tick.tick().await;
        if let Err(err) = run_once(&state).await {
            tracing::error!(error = %err, "source checks");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_new_concern_is_put_in_words() {
        let words =
            |keys: &[&str]| new_kinds(&keys.iter().map(|k| k.to_string()).collect::<Vec<_>>());
        // Every key `concerns` can make is named, so none falls through to the fallback.
        let scan = json!({
            "status": "checked",
            "licence": { "kinds": ["missing"] },
            "online_undeclared": true,
            "sandbox_mismatch": true,
            "mod_code": true,
            "mod_online": true,
            "monetization": [{ "rule": "iap" }],
            "code_loading": [{ "rule": "eval" }],
        });
        for key in concerns(&scan).unwrap() {
            assert_ne!(words(&[&key]), "something new", "{key} has no words");
        }
        // These used to panic inside the fetcher.
        assert_eq!(
            words(&["saves:sandbox_mismatch"]),
            "saves outside the save folder while it's marked as working in sandbox mode"
        );
        assert_eq!(
            words(&["mod:mod_code", "mod:mod_online"]),
            "code or networking in a mod"
        );
        assert_eq!(
            words(&["licence:missing", "code:eval", "monetization:iap"]),
            "possible purchases or loot boxes, code loading from outside SLAPorSLOP and licence problems"
        );
        assert_eq!(words(&["future:thing"]), "something new");
    }

    #[test]
    fn archives_come_from_each_hosts_own_address() {
        let url = |s: &str| Url::parse(s).unwrap();
        let c = "a".repeat(40);
        assert_eq!(
            archive_url(
                Kind::Bitbucket,
                &url("https://bitbucket.org/ws/game.git"),
                &c
            )
            .unwrap()
            .as_str(),
            format!("https://bitbucket.org/ws/game/get/{c}.tar.gz")
        );
        assert_eq!(
            archive_url(Kind::Gitea, &url("https://git.example/me/game"), &c)
                .unwrap()
                .as_str(),
            format!("https://git.example/api/v1/repos/me/game/archive/{c}.tar.gz")
        );
        assert!(archive_url(Kind::Gitea, &url("https://git.example/onlyone"), &c).is_none());
    }

    #[test]
    fn saves_outside_slops_save_folder_become_notes() {
        let mit = (
            "LICENSE",
            "MIT License\n\nPermission is hereby granted, free of charge",
        );
        let data = archive(&[
            mit,
            (
                "Scripts/Save.cs",
                "var dir = Environment.GetFolderPath(Environment.SpecialFolder.MyDocuments);\nvar p = Application.persistentDataPath;",
            ),
            (
                "save.gd",
                "var f = FileAccess.open(\"user://save.json\", FileAccess.WRITE)",
            ),
            (
                "src/save.rs",
                "let home = std::env::var(\"HOME\")?;\nlet d = dirs::data_dir();",
            ),
            ("main.js", "const p = app.getPath('userData');"),
            ("game.py", "p = Path.home() / '.mygame'"),
        ]);
        let (report, _) = scan_archive(&data[..], "MIT", true).unwrap();
        let rules: Vec<_> = report.saves.iter().map(|h| h.rule.as_str()).collect();
        assert_eq!(
            rules,
            [
                "dotnet_folders",
                "electron_paths",
                "home_env",
                "home_python",
                "rust_dirs",
                "unity_data_path"
            ],
            "Godot's user:// isn't a note"
        );
        assert_eq!(report.save_folder, "elsewhere");
        let langs: Vec<_> = report
            .code_files
            .iter()
            .map(|h| (h.label.as_str(), h.count))
            .collect();
        assert_eq!(
            langs,
            [
                ("C#", 1),
                ("GDScript", 1),
                ("JavaScript", 1),
                ("Python", 1),
                ("Rust", 1)
            ]
        );
        let value = serde_json::to_value(&report).unwrap();
        // (The short licence text here isn't recognised; that's a licence note.)
        assert!(
            concerns(&value)
                .unwrap()
                .iter()
                .all(|k| k.starts_with("licence:")),
            "save notes don't hold updates"
        );

        // Code that reads SLOP_SAVE_DIR knows about SLAPorSLOP's save folder.
        let data = archive(&[
            mit,
            (
                "Scripts/Save.cs",
                "var dir = Environment.GetEnvironmentVariable(\"SLOP_SAVE_DIR\") ?? Application.persistentDataPath;",
            ),
        ]);
        let (report, _) = scan_archive(&data[..], "MIT", true).unwrap();
        assert!(report.saves.is_empty());
        assert_eq!(report.save_folder, "slop");
        let value = serde_json::to_value(&report).unwrap();
        assert!(value.get("saves").is_none());
        assert_eq!(value["save_folder"], "slop", "every checked release says");

        // Godot's user:// lands in the save folder; code that saves nothing says so.
        let data = archive(&[
            mit,
            (
                "save.gd",
                "var f = FileAccess.open(\"user://save.json\", FileAccess.WRITE)",
            ),
        ]);
        assert_eq!(
            scan_archive(&data[..], "MIT", true).unwrap().0.save_folder,
            "engine"
        );
        let data = archive(&[mit, ("main.gd", "func _ready():\n\tpass")]);
        assert_eq!(
            scan_archive(&data[..], "MIT", true).unwrap().0.save_folder,
            "none"
        );
    }

    #[test]
    fn code_from_outside_slop_becomes_notes() {
        let data = archive(&[
            (
                "LICENSE",
                "MIT License\n\nPermission is hereby granted, free of charge",
            ),
            (
                "net/Client.cs",
                "var asm = Assembly.Load(bytes);\nvar f = new BinaryFormatter();",
            ),
            (
                "net/client.gd",
                "var mod = load_resource_pack(path)\nvar v = bytes_to_var_with_objects(packet)\nvar peer = ENetMultiplayerPeer.new()",
            ),
            ("mods/run.lua", "local f = loadstring(body)"),
            (
                "tools/train.py",
                "model.eval()\nresult = re.compile('x').match(s)",
            ),
            (
                "web/app.js",
                "const m = /a/.exec(s);\nconst ws = new WebSocket(url);",
            ),
            (
                "pulse.py",
                "import urllib.request\nurl = os.environ['SLOP_PULSE_URL']",
            ),
            (
                "web/package-lock.json",
                r#"{"packages":{"node_modules/electron-updater":{"version":"6.0.0"}}}"#,
            ),
        ]);
        let (report, _) = scan_archive(&data[..], "MIT", true).unwrap();
        let rules: Vec<_> = report
            .code_loading
            .iter()
            .map(|h| h.rule.as_str())
            .collect();
        assert_eq!(
            rules,
            [
                "load_assembly",
                "load_resource_pack",
                "load_script",
                "self_update",
                "unsafe_format"
            ],
            "model.eval() and regex .exec() aren't eval or exec"
        );
        let net: Vec<_> = report.network.iter().map(|h| h.rule.as_str()).collect();
        assert_eq!(
            net,
            ["http", "multiplayer"],
            "the Pulse snippet doesn't count"
        );
        assert_eq!(report.network[0].examples, ["web/app.js:2"]);
        let unsafe_format = report
            .code_loading
            .iter()
            .find(|h| h.rule == "unsafe_format")
            .unwrap();
        assert_eq!(unsafe_format.count, 2);
        assert_eq!(
            concerns(&serde_json::to_value(&report).unwrap())
                .unwrap()
                .into_iter()
                .filter(|k| k.starts_with("code:"))
                .count(),
            5
        );
    }

    #[test]
    fn archives_on_gitlab_and_gitea_hosts() {
        let commit = "a".repeat(40);
        let gitlab = archive_url(
            Kind::Gitlab,
            &"https://gitlab.com/group/sub/game.git".parse().unwrap(),
            &commit,
        )
        .unwrap();
        assert_eq!(
            gitlab.as_str(),
            format!(
                "https://gitlab.com/api/v4/projects/group%2Fsub%2Fgame/repository/archive.tar.gz?sha={commit}"
            )
        );
        let gitea = archive_url(
            Kind::Gitea,
            &"https://codeberg.org/ann/game/".parse().unwrap(),
            &commit,
        )
        .unwrap();
        assert_eq!(
            gitea.as_str(),
            format!("https://codeberg.org/api/v1/repos/ann/game/archive/{commit}.tar.gz")
        );
        assert!(
            archive_url(
                Kind::Gitlab,
                &"https://gitlab.com/lonely".parse().unwrap(),
                &commit
            )
            .is_none()
        );
        assert_eq!(
            archive_url(
                Kind::Bitbucket,
                &"https://bitbucket.org/a/b".parse().unwrap(),
                &commit
            )
            .unwrap()
            .as_str(),
            format!("https://bitbucket.org/a/b/get/{commit}.tar.gz"),
            "Bitbucket archives are read since 2026-10-21"
        );
    }

    fn archive(files: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let gz = flate2::write::GzEncoder::new(&mut out, flate2::Compression::fast());
            let mut tar = tar::Builder::new(gz);
            for (path, text) in files {
                let mut h = tar::Header::new_gnu();
                h.set_size(text.len() as u64);
                h.set_mode(0o644);
                h.set_cksum();
                tar.append_data(&mut h, format!("octo-game-abc/{path}"), text.as_bytes())
                    .unwrap();
            }
            tar.into_inner().unwrap().finish().unwrap();
        }
        out
    }

    #[test]
    fn a_runners_raw_scan_gives_the_servers_report() {
        let data = archive(&[
            (
                "LICENSE",
                "GNU GENERAL PUBLIC LICENSE\nVersion 3, 29 June 2007",
            ),
            (
                "src/shop.cs",
                "// SPDX-License-Identifier: MIT\nusing UnityEngine.Purchasing;\nvar s = new WebSocket(\"wss://x\");",
            ),
            (
                "web/package-lock.json",
                r#"{"packages":{"node_modules/stripe":{"version":"12.0.0"}}}"#,
            ),
        ]);
        for (declared, public) in [("MIT", true), ("GPL-3.0-only", false)] {
            let (report, packages) = scan_archive(&data[..], declared, public).unwrap();
            let raw = scan_archive_raw(&data[..]).unwrap();
            // As it arrives from a runner.
            let raw: RawScan = serde_json::from_str(&serde_json::to_string(&raw).unwrap()).unwrap();
            assert_eq!(raw.validate(), Ok(()));
            let (value, from_raw) = finish_raw(raw, declared, public);
            assert_eq!(value, serde_json::to_value(&report).unwrap());
            assert_eq!(value["status"], "checked");
            assert_eq!(from_raw, packages);
        }
    }
}
