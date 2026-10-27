// SPDX-License-Identifier: AGPL-3.0-or-later
//! The dependency watch (SECURITY §5.6): once a day, the packages of every release players get
//! (and of releases waiting for review) are checked against OSV, the free open-source
//! vulnerability database, so a security advisory published after a release was built still
//! reaches its maker.
//!
//! - **New advisories** on a live release: the maker is told, with each package's fixed version
//!   where OSV has one. Advisories Slop Build's own check already showed at build time aren't
//!   repeated.
//! - **Malicious packages** (OSV's `MAL-` entries, such as a hijacked npm package): an abuse
//!   signal for the admins (abuse.rs), since the build may carry it.
//!
//! The package lists come from the source check (source_scan.rs). Only names and versions are
//! sent to OSV; nothing about the app or its maker.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Context;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{AppState, source_scan::Package};

const EVERY: std::time::Duration = std::time::Duration::from_secs(3600);
const PER_ROUND: i64 = 20;
/// OSV takes up to 1,000 queries per batch.
const BATCH: usize = 1000;
/// Advisories listed in one notice; the rest are on the Releases page.
const IN_NOTICE: usize = 5;

#[derive(sqlx::FromRow)]
struct Due {
    release_id: Uuid,
    app_id: Uuid,
    maker_id: Uuid,
    slug: String,
    title: String,
    version: String,
    live: bool,
    source_checks: Option<Value>,
}

/// An advisory found for one package.
#[derive(Debug, Clone)]
pub struct Found {
    pub vuln_id: String,
    pub package: Package,
    pub summary: String,
    pub fixed: Option<String>,
}

impl Found {
    pub fn malicious(&self) -> bool {
        self.vuln_id.starts_with("MAL-")
    }
}

/// Ecosystems OSV has advisories for, as SLAPorSLOP names them.
const OSV_ECOSYSTEMS: &[&str] = &["crates.io", "npm", "PyPI", "Go", "NuGet", "Maven"];

/// Asks OSV which advisories affect these packages. Returns (package index, advisory ID) pairs.
async fn query(state: &AppState, packages: &[Package]) -> anyhow::Result<Vec<(usize, String)>> {
    #[derive(Deserialize)]
    struct Batch {
        results: Vec<Res>,
    }
    #[derive(Deserialize)]
    struct Res {
        #[serde(default)]
        vulns: Vec<Id>,
    }
    #[derive(Deserialize)]
    struct Id {
        id: String,
    }
    let url = state.config.releases.osv_api_base.join("v1/querybatch")?;
    let mut out = Vec::new();
    // OSV knows these ecosystems; Unity and Godot packages are read for the payment check only.
    let known: Vec<(usize, &Package)> = packages
        .iter()
        .enumerate()
        .filter(|(_, p)| OSV_ECOSYSTEMS.contains(&p.ecosystem.as_str()))
        .collect();
    for chunk in known.chunks(BATCH) {
        let queries: Vec<Value> = chunk
            .iter()
            .map(|(_, p)| json!({ "package": { "ecosystem": p.ecosystem, "name": p.name }, "version": p.version }))
            .collect();
        let res = state
            .fetch_http
            .post(url.clone())
            .json(&json!({ "queries": queries }))
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await
            .context("asking OSV")?;
        if !res.status().is_success() {
            anyhow::bail!("OSV answered {}", res.status().as_u16());
        }
        let batch: Batch = res.json().await.context("OSV's answer")?;
        for ((index, _), r) in chunk.iter().zip(batch.results) {
            for v in r.vulns {
                out.push((*index, v.id));
            }
        }
    }
    Ok(out)
}

/// An advisory's summary and the first version that fixes it for this package.
async fn details(state: &AppState, id: &str, package: &Package) -> (String, Option<String>) {
    let Ok(url) = state
        .config
        .releases
        .osv_api_base
        .join(&format!("v1/vulns/{id}"))
    else {
        return (String::new(), None);
    };
    let Ok(res) = state
        .fetch_http
        .get(url)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
    else {
        return (String::new(), None);
    };
    let v: Value = res.json().await.unwrap_or_default();
    let summary = v["summary"]
        .as_str()
        .or_else(|| v["details"].as_str())
        .unwrap_or("")
        .chars()
        .take(300)
        .collect();
    let fixed = v["affected"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["package"]["name"].as_str() == Some(&package.name))
        .flat_map(|a| a["ranges"].as_array().into_iter().flatten())
        .flat_map(|r| r["events"].as_array().into_iter().flatten())
        .find_map(|e| e["fixed"].as_str().map(String::from));
    (summary, fixed)
}

/// One release: asks OSV, records new advisories, and returns them with whether each was
/// already known when it was built.
async fn check_release(state: &AppState, r: &Due) -> anyhow::Result<Vec<(Found, bool)>> {
    let packages: Vec<Package> = sqlx::query_as::<_, (String, String, String)>(
        "select ecosystem, name, version from release_packages where release_id = $1 order by 1, 2, 3",
    )
    .bind(r.release_id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|(ecosystem, name, version)| Package { ecosystem, name, version })
    .collect();
    let hits = query(state, &packages).await?;
    let known: BTreeSet<(String, String, String)> = sqlx::query_as(
        "select vuln_id, ecosystem, package from release_advisories where release_id = $1",
    )
    .bind(r.release_id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .collect();
    // What Slop Build's check showed the maker and reviewer at build time.
    let at_build: BTreeSet<String> = r
        .source_checks
        .as_ref()
        .and_then(|c| c["vulnerabilities"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|v| v["id"].as_str().map(String::from))
        .collect();
    let mut new = Vec::new();
    for (i, vuln_id) in hits {
        let p = &packages[i];
        if known.contains(&(vuln_id.clone(), p.ecosystem.clone(), p.name.clone())) {
            continue;
        }
        let (summary, fixed) = details(state, &vuln_id, p).await;
        let f = Found {
            vuln_id,
            package: p.clone(),
            summary,
            fixed,
        };
        let was_known = at_build.contains(&f.vuln_id);
        sqlx::query(
            "insert into release_advisories (release_id, vuln_id, ecosystem, package, version, summary, fixed, malicious, known_at_build)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9) on conflict do nothing",
        )
        .bind(r.release_id)
        .bind(&f.vuln_id)
        .bind(&p.ecosystem)
        .bind(&p.name)
        .bind(&p.version)
        .bind(&f.summary)
        .bind(&f.fixed)
        .bind(f.malicious())
        .bind(was_known)
        .execute(&state.db)
        .await?;
        new.push((f, was_known));
    }
    Ok(new)
}

/// The notice's body: up to a few advisories, one per line.
pub fn describe(found: &[&Found]) -> String {
    let mut lines: Vec<String> = found
        .iter()
        .take(IN_NOTICE)
        .map(|f| {
            let fix = f
                .fixed
                .as_deref()
                .map(|v| format!(", fixed in {v}"))
                .unwrap_or_default();
            format!(
                "{} {} ({}{fix})",
                f.package.name, f.package.version, f.vuln_id
            )
        })
        .collect();
    if found.len() > IN_NOTICE {
        lines.push(format!("and {} more", found.len() - IN_NOTICE));
    }
    lines.join("; ")
}

/// One round. Returns how many releases were checked.
pub async fn run_once(state: &AppState) -> anyhow::Result<usize> {
    let due: Vec<Due> = sqlx::query_as(
        "with serving as (
             select distinct on (r.app_id) r.id
             from releases r join apps a on a.id = r.app_id
             where r.status = 'approved' and r.signed_manifest is not null
             order by r.app_id, coalesce(r.id = a.pinned_release_id, false) desc, r.reviewed_at desc
         )
         select r.id as release_id, a.id as app_id, a.maker_id, a.slug, a.title, r.version,
                r.id in (select id from serving) as live, r.source_checks
         from releases r join apps a on a.id = r.app_id
         where (r.id in (select id from serving) or r.status = 'awaiting_review')
           and exists (select 1 from release_packages p where p.release_id = r.id)
           and coalesce(r.deps_checked_at, '-infinity') < now() - interval '24 hours'
         order by r.deps_checked_at nulls first
         limit $1",
    )
    .bind(PER_ROUND)
    .fetch_all(&state.db)
    .await?;
    let n = due.len();
    for r in due {
        let new = match check_release(state, &r).await {
            Ok(new) => new,
            Err(err) => {
                // OSV unreachable: try again next round.
                tracing::warn!(release = %r.release_id, error = %err, "dependency watch");
                continue;
            }
        };
        sqlx::query("update releases set deps_checked_at = now() where id = $1")
            .bind(r.release_id)
            .execute(&state.db)
            .await?;
        let mut tx = state.db.begin().await?;
        let fresh: Vec<&Found> = new
            .iter()
            .filter(|(_, at_build)| !at_build)
            .map(|(f, _)| f)
            .collect();
        if r.live && !fresh.is_empty() {
            crate::notify::send(
                &mut tx,
                crate::notify::Notice {
                    user_id: r.maker_id,
                    category: "apps",
                    title: format!("New security advisories for {} {}", r.title, r.version),
                    body: format!(
                        "Advisories published since this release was built affect {} of its packages: {}. Update them and release again.",
                        fresh.len(),
                        describe(&fresh)
                    ),
                    link: format!("/publish/{}/releases", r.slug),
                    collapse_key: Some(format!("advisories:{}", r.release_id)),
                },
            )
            .await?;
        }
        let mut malicious: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for f in fresh.iter().filter(|f| f.malicious()) {
            malicious
                .entry(&f.vuln_id)
                .or_default()
                .push(format!("{} {}", f.package.name, f.package.version));
        }
        for (vuln, pkgs) in malicious {
            crate::abuse::record(
                &mut *tx,
                &crate::abuse::Found {
                    kind: "malicious_dependency",
                    key: format!("malicious_dependency:{}:{vuln}", r.release_id),
                    subject_kind: "app",
                    subject_id: r.app_id,
                    label: r.title.clone(),
                    detail: json!({ "advisory": vuln, "packages": pkgs, "release": r.version, "live": r.live }),
                },
            )
            .await?;
        }
        tx.commit().await?;
    }
    Ok(n)
}

pub async fn run(state: AppState) {
    let mut tick = tokio::time::interval(EVERY);
    loop {
        tick.tick().await;
        if let Err(err) = run_once(&state).await {
            tracing::error!(error = %err, "dependency watch");
        }
    }
}
