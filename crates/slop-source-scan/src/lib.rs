// SPDX-License-Identifier: AGPL-3.0-or-later
//! SLAPorSLOP's own look at an app's source (DESIGN §10's "Licence" and "Monetization rules"
//! rows, LEGAL-GUIDANCE IP-2): the part that reads files. The server
//! (crates/slop-api/src/source_scan.rs) runs it on a repository archive it downloads, and the
//! `slop-source-check` command runs it on a checked-out tree on a maker's GitHub runner. Both
//! read the same files with the same limits into a [`RawScan`], which doesn't depend on the
//! store page; [`finalize`] then adds what does (the licence the store page names, and whether
//! the app is public). It looks for:
//!
//! - **Licence:** which licence the repository's own licence files are, against the store page's.
//!   Missing or contradicting licence files, source files whose SPDX headers name a different
//!   licence, and (for public apps) licences that aren't OSI-approved become notes.
//! - **Monetization:** apps on SLAPorSLOP can't sell anything (LEGAL §8). Payment libraries, in-app
//!   purchase APIs, checkout links, loot boxes and premium currencies in the code become notes.
//! - **Code from outside SLAPorSLOP:** apps may only run code from releases and mods SLAPorSLOP checked
//!   (Content Policy, "Code from outside SLAPorSLOP"). Code that loads programs, libraries or scripts at
//!   run time, self-updaters, and data formats that can rebuild arbitrary objects (the usual way a
//!   hostile game server takes over a player's computer) become notes.
//! - **Online play:** networking code (engines' multiplayer, sockets, HTTP and WebSocket
//!   clients, online-service SDKs). Files with the Slop Pulse snippet (which only talks to
//!   SLAPorSLOP) don't count.
//! - **Packages:** the dependencies named in lockfiles (Cargo.lock, package-lock.json, yarn.lock,
//!   go.mod, poetry.lock, Pipfile.lock, requirements.txt, NuGet's packages.lock.json), kept for
//!   the daily advisory watch (crates/slop-api/src/dep_watch.rs).
//!
//! Everything here is a note for the reviewer and the maker: patterns in text have false
//! positives, so nothing is held or rejected by itself. Nothing here runs, builds or follows
//! anything it reads.

mod licence;
mod lockfile;
mod rules;
mod tree;

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use licence::{SPDX_HEADER, family, is_licence_file};
pub use licence::{expression_ids, identify_licence, not_osi_approved};
use lockfile::is_lockfile;
pub use lockfile::parse_lockfile;
use rules::*;
pub use tree::scan_dir;

/// Text files bigger than this aren't read.
pub const MAX_TEXT_FILE: u64 = 2 * 1024 * 1024;
/// Stop reading after this much text, so one huge repository can't take the server's time.
pub const MAX_TEXT_TOTAL: u64 = 256 * 1024 * 1024;
/// Packages kept from the lockfiles.
pub const MAX_PACKAGES: usize = 5000;
/// Places kept per pattern found.
pub const EXAMPLES: usize = 3;

/// A dependency, in OSV's ecosystem names.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Package {
    pub ecosystem: String,
    pub name: String,
    pub version: String,
}

/// A checked release's report, as the server stores it (`releases.repo_scan`).
#[derive(Debug, Default, PartialEq, Serialize)]
pub struct Report {
    pub status: &'static str,
    /// Files in the archive, and whether reading stopped early.
    pub files: u64,
    pub truncated: bool,
    pub licence: Licence,
    /// Lockfiles the package list came from.
    pub lockfiles: Vec<String>,
    pub packages: usize,
    pub monetization: Vec<Hit>,
    /// Loading code at run time, self-updaters, and unsafe data formats.
    pub code_loading: Vec<Hit>,
    /// Networking code: the app goes online.
    pub network: Vec<Hit>,
    /// Saves, settings or logs written to fixed places in the user's folders instead of
    /// SLOP_SAVE_DIR, so they won't follow the player into sandbox mode. A note only; left out
    /// when the code reads SLOP_SAVE_DIR.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub saves: Vec<Hit>,
    /// Where local saves go, as far as the source shows: `slop` (it reads SLOP_SAVE_DIR),
    /// `engine` (an engine folder the SLAPorSLOP app points into the save folder, like Godot's
    /// `user://`), `elsewhere` (fixed per-user folders: see `saves`) or `none` (no local saves
    /// found). Every checked release says which, so makers and reviewers see it's there.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub save_folder: String,
    /// The store page says it works in sandbox mode, but the code saves elsewhere (set when
    /// the release is checked). Players aren't told it works in sandbox mode while it's so.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub sandbox_mismatch: bool,
    /// Code in the repository, by language, plus built code (`.dll`, `.so`, `.pck`…). Shown for
    /// mods: a mod with code gets a full read (DESIGN §9).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub code_files: Vec<Hit>,
    /// A mod with code (set when the release is checked).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub mod_code: bool,
    /// A mod with networking code: flagged, and sandbox mode is suggested for its game.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub mod_online: bool,
    /// Pay-to-win designs a person should look at (premium currencies, energy skips, VIP tiers,
    /// battle passes, bought boosts). Notes only: nothing on SLAPorSLOP can be sold anyway.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub design: Vec<Hit>,
    /// Slop Pulse in the code: `none`, `start_only` (never sends heartbeats) or `heartbeat`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub pulse: String,
    /// A community build of a game that counts players, without Pulse (set when checked).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub pulse_missing: bool,
    /// It goes online, but its store page doesn't say so (set when the release is checked).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub online_undeclared: bool,
}

#[derive(Debug, Default, PartialEq, Serialize)]
pub struct Licence {
    pub declared: String,
    pub files: Vec<LicenceFile>,
    /// SPDX-License-Identifier headers naming something other than the declared licence.
    pub headers: Vec<Hit>,
    /// Licence texts and vendor notices inside files (GPL, Apache, MPL, Unity Asset Store, Epic
    /// Games): code that came from elsewhere, under its own terms.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notices: Vec<Hit>,
    /// Closed-source libraries the code builds against (NVIDIA's Cg runtime): an open-source app
    /// can't depend on them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub closed: Vec<Hit>,
    pub problems: Vec<String>,
    /// A short key per problem (`no_file`, `mismatch`, `header:GPL-3.0`…), so an update's check
    /// can be compared with the last approved release's.
    pub kinds: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicenceFile {
    pub path: String,
    /// The licence it is, when SLAPorSLOP recognises the text.
    pub spdx: Option<String>,
}

/// A pattern found: what it is, how often, and where (`path:line`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hit {
    pub rule: String,
    pub label: String,
    pub count: u32,
    pub examples: Vec<String>,
}

/// A licence text or vendor notice found in files, with the declared licence families that cover
/// it (`ok`; empty: none does).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub rule: String,
    pub label: String,
    pub ok: Vec<String>,
    pub count: u32,
    pub examples: Vec<String>,
}

/// What reading a repository found, before the store page's licence and visibility are known:
/// every SPDX header and every licence notice is kept, and [`finalize`] picks the ones that are
/// notes. It travels as JSON from a maker's runner (`slop-source-check`) to the server, which
/// checks it with [`RawScan::validate`] first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawScan {
    /// Files in the repository, and whether reading stopped early.
    pub files: u64,
    pub truncated: bool,
    pub licence_files: Vec<LicenceFile>,
    /// Every SPDX-License-Identifier header, by expression (`rule` and `label` are both it).
    pub headers: Vec<Hit>,
    /// Every licence text and vendor notice.
    pub notices: Vec<Notice>,
    /// Closed-source libraries the code builds against.
    pub closed: Vec<Hit>,
    pub lockfiles: Vec<String>,
    /// Sorted, without repeats, at most [`MAX_PACKAGES`].
    pub packages: Vec<Package>,
    pub monetization: Vec<Hit>,
    pub code_loading: Vec<Hit>,
    pub network: Vec<Hit>,
    /// Empty when the code reads SLOP_SAVE_DIR.
    pub saves: Vec<Hit>,
    /// `slop`, `engine`, `elsewhere` or `none` (see [`Report::save_folder`]).
    pub save_folder: String,
    pub code_files: Vec<Hit>,
    pub design: Vec<Hit>,
    /// `none`, `start_only` or `heartbeat`.
    pub pulse: String,
}

/// What `slop-source-check` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceCheck {
    pub raw: RawScan,
    /// `slop-source-check <version>`.
    pub tool: String,
}

/// The name and version `slop-source-check` reports itself as.
pub fn tool() -> String {
    format!("slop-source-check {}", env!("CARGO_PKG_VERSION"))
}

/// Most hits a raw scan may carry, across all its lists.
const MAX_RAW_HITS: usize = 2000;
/// Most packages a raw scan may carry.
const MAX_RAW_PACKAGES: usize = 20_000;
/// Most licence files and lockfiles a raw scan may name.
const MAX_RAW_FILES: usize = 10_000;
/// Longest string in a raw scan, in characters.
const MAX_RAW_STRING: usize = 1000;

impl RawScan {
    /// Refuses a raw scan that's malformed or bigger than any real repository's: too many hits,
    /// packages or files, strings over 1000 characters, or values SLAPorSLOP never writes.
    pub fn validate(&self) -> Result<(), &'static str> {
        fn short(s: &str) -> bool {
            s.len() <= MAX_RAW_STRING || s.chars().count() <= MAX_RAW_STRING
        }
        fn hit_ok(rule: &str, label: &str, count: u32, examples: &[String]) -> bool {
            !rule.is_empty()
                && short(rule)
                && short(label)
                && examples.len() <= EXAMPLES
                && count as usize >= examples.len().max(1)
                && examples.iter().all(|e| short(e))
        }
        if !matches!(self.pulse.as_str(), "none" | "start_only" | "heartbeat") {
            return Err("unknown Pulse kind");
        }
        if !matches!(
            self.save_folder.as_str(),
            "slop" | "engine" | "elsewhere" | "none"
        ) {
            return Err("unknown save folder");
        }
        let lists = [
            &self.headers,
            &self.closed,
            &self.monetization,
            &self.code_loading,
            &self.network,
            &self.saves,
            &self.code_files,
            &self.design,
        ];
        if lists.iter().map(|l| l.len()).sum::<usize>() + self.notices.len() > MAX_RAW_HITS {
            return Err("too many hits");
        }
        if !lists
            .iter()
            .flat_map(|l| l.iter())
            .all(|h| hit_ok(&h.rule, &h.label, h.count, &h.examples))
        {
            return Err("a hit is malformed or too long");
        }
        if !self.notices.iter().all(|n| {
            hit_ok(&n.rule, &n.label, n.count, &n.examples)
                && n.ok.len() <= 16
                && n.ok.iter().all(|o| short(o))
        }) {
            return Err("a licence notice is malformed or too long");
        }
        if self.licence_files.len() > MAX_RAW_FILES || self.lockfiles.len() > MAX_RAW_FILES {
            return Err("too many licence files or lockfiles");
        }
        if !self
            .licence_files
            .iter()
            .all(|f| short(&f.path) && f.spdx.as_deref().is_none_or(short))
            || !self.lockfiles.iter().all(|p| short(p))
        {
            return Err("a path is too long");
        }
        if self.packages.len() > MAX_RAW_PACKAGES {
            return Err("too many packages");
        }
        if !self
            .packages
            .iter()
            .all(|p| short(&p.ecosystem) && short(&p.name) && short(&p.version))
        {
            return Err("a package's name is too long");
        }
        Ok(())
    }
}

/// One file in a repository: its path inside it (`/`-separated), its size, and its bytes (read
/// only when the check needs them, and never more than `size`).
pub struct Entry<R> {
    pub path: String,
    pub size: u64,
    pub reader: R,
}

#[derive(Default)]
struct Tally {
    hits: BTreeMap<String, (String, u32, Vec<String>)>,
}

impl Tally {
    fn add(&mut self, rule: &str, label: &str, at: String) {
        let e = self
            .hits
            .entry(rule.to_string())
            .or_insert_with(|| (label.to_string(), 0, Vec::new()));
        e.1 += 1;
        if e.2.len() < EXAMPLES {
            e.2.push(at);
        }
    }

    fn into_hits(self) -> Vec<Hit> {
        self.hits
            .into_iter()
            .map(|(rule, (label, count, examples))| Hit {
                rule,
                label,
                count,
                examples,
            })
            .collect()
    }
}

/// Joins hits with the same label (`js` and `mjs` are both JavaScript).
fn merge_by_label(hits: Vec<Hit>) -> Vec<Hit> {
    let mut out: Vec<Hit> = Vec::new();
    for h in hits {
        match out.iter_mut().find(|o| o.label == h.label) {
            Some(o) => {
                o.count += h.count;
                for e in h.examples {
                    if o.examples.len() < EXAMPLES {
                        o.examples.push(e);
                    }
                }
            }
            None => out.push(h),
        }
    }
    out
}

/// Reads a repository archive (`.tar.gz`, one top folder, as GitHub, GitLab and Gitea make them).
pub fn scan_archive_raw(archive: impl Read) -> anyhow::Result<RawScan> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let entries = tar.entries().context("reading the repository archive")?;
    scan_entries(entries.filter_map(|entry| {
        let entry = match entry.context("reading the repository archive") {
            Ok(entry) => entry,
            Err(err) => return Some(Err(err)),
        };
        if !entry.header().entry_type().is_file() {
            return None;
        }
        let full = match entry.path() {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(err) => return Some(Err(err.into())),
        };
        // Archives put everything under one folder, such as GitHub's `<owner>-<repo>-<sha>/`.
        let (_, path) = full.split_once('/')?;
        Some(Ok(Entry {
            path: path.to_string(),
            size: entry.header().size().unwrap_or(0),
            reader: entry,
        }))
    }))
}

/// Reads a repository archive and applies the store page's licence and visibility: the report
/// the server stores, and the package list.
pub fn scan_archive(
    archive: impl Read,
    declared: &str,
    public: bool,
) -> anyhow::Result<(Report, Vec<Package>)> {
    Ok(finalize(scan_archive_raw(archive)?, declared, public))
}

/// Reads a repository's files, in order. Stops at the first error, and after
/// [`MAX_TEXT_TOTAL`] of text (`truncated`).
pub fn scan_entries<R: Read>(
    entries: impl IntoIterator<Item = anyhow::Result<Entry<R>>>,
) -> anyhow::Result<RawScan> {
    let mut raw = RawScan::default();
    let mut packages = BTreeSet::new();
    let mut monetization = Tally::default();
    let mut headers = Tally::default();
    // Notices by rule, with the declared licences that cover them.
    let mut notices = Tally::default();
    let mut notice_ok: BTreeMap<&'static str, &'static [&'static str]> = BTreeMap::new();
    let mut closed = Tally::default();
    let mut design = Tally::default();
    let (mut pulse_uses, mut pulse_beats) = (false, false);
    let mut code_loading = Tally::default();
    let mut network = Tally::default();
    let mut saves = Tally::default();
    let mut code_files = Tally::default();
    // Code that reads SLOP_SAVE_DIR knows about SLAPorSLOP's save folder: no save notes then.
    let mut uses_save_dir = false;
    // Godot's user:// lands in the save folder by itself.
    let mut engine_saves = false;
    let mut text_total: u64 = 0;
    for entry in entries {
        let Entry {
            path,
            size,
            mut reader,
        } = entry?;
        raw.files += 1;
        if path.split('/').any(|part| SKIP_DIRS.contains(&part)) {
            continue;
        }
        let name = path.rsplit('/').next().unwrap_or(&path).to_string();
        let ext = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        if let Some(lang) = language(&ext) {
            code_files.add(&ext, lang, path.clone());
        }
        let licence_file = is_licence_file(&path);
        let lockfile = is_lockfile(&name)
            || (name == "plugin.cfg" && (path.starts_with("addons/") || path.contains("/addons/")));
        let unity_manifest = path.ends_with("Packages/manifest.json");
        let source = SOURCE_EXTS.contains(&ext.as_str()) || ext == "cfg" && name == "project.godot";
        // Build scripts, for libraries the program links.
        let build_file = matches!(
            name.as_str(),
            "CMakeLists.txt"
                | "Makefile"
                | "makefile"
                | "GNUmakefile"
                | "meson.build"
                | "SConstruct"
        ) || matches!(ext.as_str(), "cmake" | "mk" | "pro" | "vcxproj");
        // Third-party notices shipped inside the project (Asset Store packages, vendored code).
        let lower = name.to_ascii_lowercase();
        let notice_file = !licence_file
            && matches!(ext.as_str(), "txt" | "md" | "")
            && (lower.contains("license") || lower.contains("licence") || lower.contains("eula"));
        if !(licence_file || lockfile || unity_manifest || source || notice_file || build_file) {
            continue;
        }
        if size > MAX_TEXT_FILE {
            continue;
        }
        if text_total + size > MAX_TEXT_TOTAL {
            raw.truncated = true;
            break;
        }
        text_total += size;
        let mut bytes = Vec::with_capacity(size as usize);
        reader.by_ref().take(size).read_to_end(&mut bytes)?;
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        if licence_file {
            raw.licence_files.push(LicenceFile {
                path: path.clone(),
                spdx: identify_licence(&text).map(String::from),
            });
        }
        if lockfile {
            let found = parse_lockfile(&name, &text);
            if !found.is_empty() {
                raw.lockfiles.push(path.clone());
                packages.extend(found);
            }
        }
        if source || build_file {
            // Once per file, at its first mention.
            for rule in CLOSED_RULES.iter() {
                if let Some(i) = text
                    .lines()
                    .position(|l| l.len() <= 2000 && rule.re.is_match(l))
                {
                    closed.add(rule.rule, rule.label, format!("{path}:{}", i + 1));
                }
            }
        }
        if notice_file || source {
            for (rule, ok) in NOTICE_RULES.iter() {
                if let Some(i) = text.lines().take(60).position(|l| rule.re.is_match(l)) {
                    notices.add(rule.rule, rule.label, format!("{path}:{}", i + 1));
                    notice_ok.insert(rule.rule, ok);
                }
            }
        }
        if unity_manifest || source {
            // The Slop Pulse snippet only talks to SLAPorSLOP.
            let pulse = text.contains("SLOP_PULSE");
            if text.contains("SLOP_PULSE_TOKEN") {
                pulse_uses = true;
                pulse_beats |= text.contains("heartbeat");
            }
            uses_save_dir |= text.contains("SLOP_SAVE_DIR");
            engine_saves |=
                matches!(ext.as_str(), "gd" | "cs" | "godot" | "cfg") && text.contains("user://");
            for (i, line) in text.lines().enumerate() {
                if line.len() > 2000 {
                    continue;
                }
                for rule in RULES.iter() {
                    if rule.re.is_match(line) {
                        monetization.add(rule.rule, rule.label, format!("{path}:{}", i + 1));
                    }
                }
                for (rule, exts) in CODE_RULES.iter() {
                    if exts.contains(&ext.as_str()) && rule.re.is_match(line) {
                        code_loading.add(rule.rule, rule.label, format!("{path}:{}", i + 1));
                    }
                }
                for (rule, exts) in NET_RULES.iter() {
                    if !pulse && exts.contains(&ext.as_str()) && rule.re.is_match(line) {
                        network.add(rule.rule, rule.label, format!("{path}:{}", i + 1));
                    }
                }
                for (rule, exts) in SAVE_RULES.iter() {
                    if exts.contains(&ext.as_str()) && rule.re.is_match(line) {
                        saves.add(rule.rule, rule.label, format!("{path}:{}", i + 1));
                    }
                }
                if source {
                    for rule in DESIGN_RULES.iter() {
                        if rule.re.is_match(line) {
                            design.add(rule.rule, rule.label, format!("{path}:{}", i + 1));
                        }
                    }
                }
                // Every header: which name a different licence depends on the store page.
                if let Some(m) = (i < 30).then(|| SPDX_HEADER.captures(line)).flatten() {
                    let expr = m[1].trim();
                    if !expression_ids(expr).is_empty() {
                        headers.add(expr, expr, format!("{path}:{}", i + 1));
                    }
                }
            }
        }
    }
    raw.packages = packages.into_iter().take(MAX_PACKAGES).collect();
    for p in raw.packages.iter().filter(|p| is_payment_package(p)) {
        monetization.add(
            "payment_library",
            "A payment library",
            format!("{} {} ({})", p.name, p.version, p.ecosystem),
        );
    }
    for p in raw.packages.iter().filter(|p| in_list(UPDATER_PACKAGES, p)) {
        code_loading.add(
            "self_update",
            "Updates itself outside SLAPorSLOP",
            format!("{} {} ({})", p.name, p.version, p.ecosystem),
        );
    }
    raw.monetization = monetization.into_hits();
    raw.code_loading = code_loading.into_hits();
    raw.network = network.into_hits();
    if !uses_save_dir {
        raw.saves = saves.into_hits();
    }
    raw.code_files = merge_by_label(code_files.into_hits());
    raw.design = design.into_hits();
    raw.pulse = match (pulse_uses, pulse_beats) {
        (false, _) => "none",
        (true, false) => "start_only",
        (true, true) => "heartbeat",
    }
    .into();
    raw.notices = notices
        .into_hits()
        .into_iter()
        .map(|h| Notice {
            ok: notice_ok
                .get(h.rule.as_str())
                .into_iter()
                .flat_map(|ok| ok.iter())
                .map(|o| o.to_string())
                .collect(),
            rule: h.rule,
            label: h.label,
            count: h.count,
            examples: h.examples,
        })
        .collect();
    raw.closed = closed.into_hits();
    raw.save_folder = if uses_save_dir {
        "slop"
    } else if !raw.saves.is_empty() {
        "elsewhere"
    } else if engine_saves {
        "engine"
    } else {
        "none"
    }
    .into();
    raw.headers = headers.into_hits();
    Ok(raw)
}

/// Applies the store page to a raw scan: the licence it names (an SPDX expression) picks which
/// SPDX headers and licence notices are notes and whether the licence files match, and a public
/// app's licence must be OSI-approved. Returns the report and the package list.
pub fn finalize(raw: RawScan, declared: &str, public: bool) -> (Report, Vec<Package>) {
    let declared_families: BTreeSet<String> = expression_ids(declared)
        .iter()
        .map(|id| family(id))
        .collect();
    let packages: Vec<Package> = raw
        .packages
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(MAX_PACKAGES)
        .collect();
    // SPDX headers naming none of the store page's licences.
    let headers = raw
        .headers
        .into_iter()
        .filter(|h| {
            let ids = expression_ids(&h.rule);
            !ids.is_empty() && !ids.iter().any(|id| declared_families.contains(&family(id)))
        })
        .collect();
    // Licence notices the store page's licence doesn't cover.
    let notices = raw
        .notices
        .into_iter()
        .filter(|n| {
            let covered = !n.ok.is_empty()
                && declared_families
                    .iter()
                    .any(|f| n.ok.iter().any(|o| f.starts_with(&o.to_ascii_lowercase())));
            !covered
        })
        .map(|n| Hit {
            rule: n.rule,
            label: n.label,
            count: n.count,
            examples: n.examples,
        })
        .collect();
    let mut report = Report {
        status: "checked",
        files: raw.files,
        truncated: raw.truncated,
        licence: Licence {
            declared: declared.to_string(),
            files: raw.licence_files,
            headers,
            notices,
            closed: raw.closed,
            ..Default::default()
        },
        lockfiles: raw.lockfiles,
        packages: packages.len(),
        monetization: raw.monetization,
        code_loading: raw.code_loading,
        network: raw.network,
        saves: raw.saves,
        save_folder: raw.save_folder,
        code_files: raw.code_files,
        design: raw.design,
        pulse: raw.pulse,
        ..Default::default()
    };
    (report.licence.kinds, report.licence.problems) =
        licence_problems(&report.licence, &declared_families, public)
            .into_iter()
            .unzip();
    (report, packages)
}

fn licence_problems(
    l: &Licence,
    declared: &BTreeSet<String>,
    public: bool,
) -> Vec<(String, String)> {
    let mut problems = Vec::new();
    if l.files.is_empty() {
        problems.push((
            "no_file".into(),
            "There's no licence file (LICENSE or COPYING) at the top of the repository, so nobody has permission to use the code. Add the licence's full text as LICENSE.".into(),
        ));
    } else {
        let known: Vec<&str> = l.files.iter().filter_map(|f| f.spdx.as_deref()).collect();
        if known.is_empty() {
            problems.push((
                "unidentified".into(),
                format!(
                    "SLAPorSLOP couldn't tell which licence {} is. Make sure it's {}.",
                    l.files[0].path, l.declared
                ),
            ));
        } else if !known.iter().any(|id| declared.contains(&family(id))) {
            problems.push((
                "mismatch".into(),
                format!(
                    "The licence file says {}, but the store page says {}. Make them match.",
                    known.join(" and "),
                    l.declared
                ),
            ));
        }
    }
    for h in &l.headers {
        problems.push((
            format!("header:{}", h.rule),
            format!(
                "{} source file{} say{} {} in their headers (e.g. {}), not the store page's {}.",
                h.count,
                if h.count == 1 { "" } else { "s" },
                if h.count == 1 { "s" } else { "" },
                h.label,
                h.examples.join(", "),
                l.declared
            ),
        ));
    }
    for c in &l.closed {
        problems.push((
            format!("closed:{}", c.rule),
            format!(
                "{} file{} build{} against {} (e.g. {}). An open-source app can't depend on a closed-source library: use GLSL or HLSL shaders instead.",
                c.count,
                if c.count == 1 { "" } else { "s" },
                if c.count == 1 { "s" } else { "" },
                c.label,
                c.examples.join(", "),
            ),
        ));
    }
    for n in &l.notices {
        problems.push((
            format!("notice:{}", n.rule),
            format!(
                "{} file{} carr{} {} (e.g. {}), which the store page's {} doesn't cover. Code from elsewhere keeps its own terms: name its licence on the store page too, or take it out.",
                n.count,
                if n.count == 1 { "" } else { "s" },
                if n.count == 1 { "ies" } else { "y" },
                n.label,
                n.examples.join(", "),
                l.declared
            ),
        ));
    }
    if public {
        let not_osi = not_osi_approved(&l.declared);
        if !not_osi.is_empty() {
            problems.push((
                "not_osi".into(),
                format!(
                    "{} isn't on SLAPorSLOP's list of OSI-approved licences, which apps in the store need (opensource.org/licenses).",
                    not_osi.join(", ")
                ),
            ));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_from_elsewhere_designs_and_pulse_become_notes() {
        let mit = (
            "LICENSE",
            "MIT License\n\nPermission is hereby granted, free of charge",
        );
        let gpl_header = "// This program is free software: you can redistribute it and/or modify\n// it under the terms of the GNU General Public License as published by\n// the Free Software Foundation.\nfn x() {}";
        let data = archive(&[
            mit,
            ("src/vendor/parser.rs", gpl_header),
            (
                "Source/Game.cpp",
                "// Copyright Epic Games, Inc. All Rights Reserved.\nint main() {}",
            ),
            (
                "Assets/Plugins/Fancy/License.txt",
                "This asset is governed by the Asset Store Terms of Service and EULA.",
            ),
            (
                "Assets/Shop.cs",
                "int premiumCurrency = 0; // hard_currency\nvoid SkipTimer() {}\nvar vip_level = 2;",
            ),
            (
                "Assets/Pulse.cs",
                "var token = Environment.GetEnvironmentVariable(\"SLOP_PULSE_TOKEN\"); Post(\"start\");",
            ),
        ]);
        let (report, _) = scan_archive(&data[..], "MIT", true).unwrap();
        let notices: Vec<_> = report
            .licence
            .notices
            .iter()
            .map(|h| h.rule.as_str())
            .collect();
        assert_eq!(notices, ["epic_games", "gpl", "unity_asset_store"]);
        assert!(report.licence.kinds.contains(&"notice:gpl".to_string()));
        let design: Vec<_> = report.design.iter().map(|h| h.rule.as_str()).collect();
        assert_eq!(design, ["energy_skip", "premium_currency", "vip_tiers"]);
        assert_eq!(report.pulse, "start_only");

        // The same GPL code in a GPL project is fine.
        let gpl = (
            "LICENSE",
            "GNU GENERAL PUBLIC LICENSE\nVersion 3, 29 June 2007",
        );
        let data = archive(&[gpl, ("src/parser.rs", gpl_header)]);
        let (report, _) = scan_archive(&data[..], "GPL-3.0-or-later", true).unwrap();
        assert!(report.licence.notices.is_empty());
        assert_eq!(report.pulse, "none");
    }

    #[test]
    fn closed_source_cg_is_a_licence_problem_but_unity_shaders_are_not() {
        let licence = (
            "LICENSE",
            "MIT License\n\nPermission is hereby granted, free of charge",
        );
        let data = archive(&[
            licence,
            (
                "src/render.cpp",
                "#include <GL/gl.h>\n#include <Cg/cgGL.h>\nint main() {}",
            ),
            (
                "CMakeLists.txt",
                "find_package(Cg REQUIRED)\ntarget_link_libraries(game -lCgGL)",
            ),
            (
                "Assets/Water.shader",
                "CGPROGRAM\n#pragma vertex vert\nENDCG",
            ),
        ]);
        let (report, _) = scan_archive(&data[..], "MIT", true).unwrap();
        let closed: Vec<_> = report
            .licence
            .closed
            .iter()
            .map(|h| (h.rule.as_str(), h.count))
            .collect();
        assert_eq!(closed, [("nvidia_cg", 2)]);
        assert!(
            report
                .licence
                .kinds
                .contains(&"closed:nvidia_cg".to_string())
        );

        let data = archive(&[licence, ("Assets/Water.shader", "CGPROGRAM\nENDCG")]);
        let (report, _) = scan_archive(&data[..], "MIT", true).unwrap();
        assert!(report.licence.closed.is_empty());
    }

    #[test]
    fn licences_are_recognised() {
        assert_eq!(
            identify_licence(
                "MIT License\n\nPermission is hereby granted, free of charge, to any person obtaining a copy\nof this software... The above copyright notice and this permission notice shall be included in all"
            ),
            Some("MIT")
        );
        assert_eq!(
            identify_licence(
                "                                 Apache License\n                           Version 2.0, January 2004"
            ),
            Some("Apache-2.0")
        );
        assert_eq!(
            identify_licence("GNU GENERAL PUBLIC LICENSE\n Version 3, 29 June 2007"),
            Some("GPL-3.0")
        );
        assert_eq!(
            identify_licence("GNU GENERAL PUBLIC LICENSE\n Version 2, June 1991"),
            Some("GPL-2.0")
        );
        assert_eq!(identify_licence("All rights reserved."), None);
        assert_eq!(
            expression_ids("(MIT OR Apache-2.0) AND GPL-2.0-only WITH Classpath-exception-2.0"),
            ["MIT", "Apache-2.0", "GPL-2.0-only"]
        );
        assert_eq!(family("GPL-3.0-or-later"), family("GPL-3.0"));
    }

    #[test]
    fn more_package_lists_give_packages() {
        let names = |file: &str, text: &str| -> Vec<String> {
            parse_lockfile(file, text)
                .into_iter()
                .map(|p| format!("{} {}@{}", p.ecosystem, p.name, p.version))
                .collect()
        };
        let pnpm = "lockfileVersion: '9.0'\nimporters:\n  .:\n    dependencies:\n      lodash:\n        version: 4.17.21\npackages:\n  lodash@4.17.21:\n    resolution: {integrity: x}\n  '@stripe/stripe-js@4.1.0':\n    resolution: {integrity: y}\n  react-dom@19.0.0(react@19.0.0):\n    resolution: {integrity: z}\nsnapshots:\n  other@1.0.0: {}\n";
        assert_eq!(
            names("pnpm-lock.yaml", pnpm),
            [
                "npm lodash@4.17.21",
                "npm @stripe/stripe-js@4.1.0",
                "npm react-dom@19.0.0"
            ]
        );
        assert_eq!(
            names(
                "pnpm-lock.yaml",
                "lockfileVersion: 5.4\npackages:\n  /left-pad/1.3.0:\n    dev: false\n"
            ),
            ["npm left-pad@1.3.0"]
        );
        let gradle = "# comment\ncom.android.billingclient:billing:7.0.0=releaseRuntimeClasspath\njunit:junit:4.13.2=testCompileClasspath,testRuntimeClasspath\nempty=\n";
        assert_eq!(
            names("gradle.lockfile", gradle),
            ["Maven com.android.billingclient:billing@7.0.0"]
        );
        let unity = r#"{"dependencies": {"com.unity.purchasing": {"version": "4.12.2", "source": "registry"}, "com.unity.modules.ui": {"version": "1.0.0", "source": "builtin"}}}"#;
        assert_eq!(
            names("packages-lock.json", unity),
            ["Unity com.unity.purchasing@4.12.2"]
        );
        let godot = "[plugin]\n\nname=\"GodotGooglePlayBilling\"\ndescription=\"x\"\nversion=\"2.1.0\"\nscript=\"plugin.gd\"\n";
        assert_eq!(
            names("plugin.cfg", godot),
            ["Godot GodotGooglePlayBilling@2.1.0"]
        );
        // The payment check sees them.
        assert!(
            parse_lockfile("packages-lock.json", unity)
                .iter()
                .any(is_payment_package)
        );
        assert!(
            parse_lockfile("plugin.cfg", godot)
                .iter()
                .any(is_payment_package)
        );
        assert!(
            parse_lockfile("gradle.lockfile", gradle)
                .iter()
                .any(is_payment_package)
        );
    }

    #[test]
    fn lockfiles_give_packages() {
        let cargo = "[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n\n[[package]]\nname = \"mygame\"\nversion = \"0.1.0\"\n";
        assert_eq!(
            parse_lockfile("Cargo.lock", cargo),
            [Package {
                ecosystem: "crates.io".into(),
                name: "serde".into(),
                version: "1.0.0".into()
            }]
        );
        let npm = r#"{"lockfileVersion":3,"packages":{"":{"name":"g"},"node_modules/lodash":{"version":"4.17.20"},"node_modules/a/node_modules/@scope/b":{"version":"1.0.0"},"node_modules/vitest":{"version":"1.0.0","dev":true}}}"#;
        let mut names: Vec<_> = parse_lockfile("package-lock.json", npm)
            .into_iter()
            .map(|p| p.name)
            .collect();
        names.sort();
        assert_eq!(names, ["@scope/b", "lodash"]);
        let yarn = "# yarn\n\n\"@babel/core@^7.0.0\", \"@babel/core@^7.1.0\":\n  version \"7.2.0\"\n  resolved \"x\"\n\nleft-pad@1.0.0:\n  version \"1.0.0\"\n";
        let got: Vec<_> = parse_lockfile("yarn.lock", yarn)
            .into_iter()
            .map(|p| format!("{}@{}", p.name, p.version))
            .collect();
        assert_eq!(got, ["@babel/core@7.2.0", "left-pad@1.0.0"]);
        let go = "module x\n\nrequire github.com/a/b v1.2.3\nrequire (\n\tgolang.org/x/net v0.1.0 // indirect\n)\n";
        assert_eq!(parse_lockfile("go.mod", go).len(), 2);
        assert_eq!(
            parse_lockfile(
                "requirements.txt",
                "requests[socks]==2.0.1 ; python_version>'3'\nflask>=2\n"
            )[0]
            .name,
            "requests"
        );
        let nuget = r#"{"dependencies":{"net8.0":{"Newtonsoft.Json":{"type":"Direct","resolved":"13.0.1"},"Game.Core":{"type":"Project"}}}}"#;
        assert_eq!(parse_lockfile("packages.lock.json", nuget).len(), 1);
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
    fn archives_give_licence_monetization_and_package_notes() {
        let data = archive(&[
            (
                "LICENSE",
                "GNU GENERAL PUBLIC LICENSE\nVersion 3, 29 June 2007",
            ),
            (
                "src/shop.cs",
                "// SPDX-License-Identifier: GPL-3.0-or-later\nusing UnityEngine.Purchasing;\nclass Shop { void OpenLootBox() {} string gemPack = \"gem pack\"; }",
            ),
            (
                "src/main.rs",
                "// SPDX-License-Identifier: MIT\nfn main() {}",
            ),
            ("node_modules/stripe/index.js", "api.stripe.com"),
            (
                "web/package-lock.json",
                r#"{"packages":{"node_modules/stripe":{"version":"12.0.0"}}}"#,
            ),
        ]);
        let (report, packages) = scan_archive(&data[..], "MIT", true).unwrap();
        assert_eq!(report.licence.files[0].spdx.as_deref(), Some("GPL-3.0"));
        assert!(
            report
                .licence
                .problems
                .iter()
                .any(|p| p.contains("says GPL-3.0, but the store page says MIT")),
            "{:?}",
            report.licence.problems
        );
        assert!(
            report
                .licence
                .problems
                .iter()
                .any(|p| p.contains("1 source file says GPL-3.0-or-later")),
            "{:?}",
            report.licence.problems
        );
        let rules: Vec<_> = report
            .monetization
            .iter()
            .map(|h| h.rule.as_str())
            .collect();
        assert_eq!(
            rules,
            ["payment_library", "premium_currency", "unity_iap"],
            "node_modules isn't read; LootBox isn't a word boundary"
        );
        assert_eq!(packages.len(), 1);
        assert_eq!(report.lockfiles, ["web/package-lock.json"]);

        let (clean, _) = scan_archive(&archive(&[("LICENSE", "Permission is hereby granted, free of charge, to any person obtaining a copy. The above copyright notice and this permission notice shall be included")])[..], "MIT", true).unwrap();
        assert!(clean.licence.problems.is_empty() && clean.monetization.is_empty());
        let (missing, _) = scan_archive(
            &archive(&[("README.md", "hi")])[..],
            "Proprietary-1.0",
            true,
        )
        .unwrap();
        assert_eq!(
            missing.licence.problems.len(),
            2,
            "no file, and not OSI-approved"
        );
    }

    /// A repository with something for every part of the check.
    fn sample() -> Vec<(&'static str, String)> {
        let mit = "MIT License\n\nPermission is hereby granted, free of charge, to any person obtaining a copy. The above copyright notice and this permission notice shall be included";
        let gpl_header = "// SPDX-License-Identifier: GPL-3.0-or-later\n// This program is free software: you can redistribute it and/or modify\n// it under the terms of the GNU General Public License as published by\n// the Free Software Foundation.\nfn x() {}";
        let files: Vec<(&str, &str)> = vec![
            ("LICENSE", mit),
            (
                "LICENSES/Apache-2.0.txt",
                "Apache License\nVersion 2.0, January 2004",
            ),
            (
                "src/main.rs",
                "// SPDX-License-Identifier: MIT\nfn main() {}",
            ),
            ("src/vendor/parser.rs", gpl_header),
            ("src/vendor/lexer.rs", gpl_header),
            (
                "src/vendor/apache.rs",
                "// SPDX-License-Identifier: Apache-2.0\n// Licensed under the Apache License, Version 2.0\nfn y() {}",
            ),
            (
                "src/net.rs",
                "let s = std::net::TcpStream::connect(a)?;\nlet r = reqwest::get(u);\nlet d = dirs::data_dir();",
            ),
            // Git's order: net-old.rs, net.rs, net/udp.rs (the examples show it).
            ("src/net/udp.rs", "let u = std::net::UdpSocket::bind(a)?;"),
            ("src/net-old.rs", "let u = std::net::UdpSocket::bind(a)?;"),
            (
                "Source/Game.cpp",
                "// Copyright Epic Games, Inc. All Rights Reserved.\n#include <Cg/cgGL.h>\nint main() {}",
            ),
            ("CMakeLists.txt", "find_package(Cg REQUIRED)"),
            (
                "Assets/Plugins/Fancy/License.txt",
                "This asset is governed by the Asset Store Terms of Service and EULA.",
            ),
            (
                "Assets/Shop.cs",
                "using UnityEngine.Purchasing;\nint premiumCurrency = 0; // hard_currency\nvoid SkipTimer() {}\nvar vip_level = 2;\nvar a = Assembly.Load(bytes);",
            ),
            (
                "Assets/Pulse.cs",
                "var token = Environment.GetEnvironmentVariable(\"SLOP_PULSE_TOKEN\"); Post(\"heartbeat\");",
            ),
            (
                "game/save.gd",
                "var f = FileAccess.open(\"user://save.json\", FileAccess.WRITE)",
            ),
            ("game/mods.lua", "local f = loadstring(body)"),
            (
                "web/package-lock.json",
                r#"{"packages":{"node_modules/stripe":{"version":"12.0.0"},"node_modules/electron-updater":{"version":"6.0.0"}}}"#,
            ),
            (
                "Cargo.lock",
                "[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
            ),
            ("node_modules/stripe/index.js", "api.stripe.com"),
            ("README.md", "A game."),
            ("assets/logo.png", "\u{89}PNG not text"),
        ];
        let mut out: Vec<(&'static str, String)> =
            files.into_iter().map(|(p, t)| (p, t.to_string())).collect();
        // Too big to read, in both.
        out.push((
            "src/huge.rs",
            "// eval(\n".repeat(MAX_TEXT_FILE as usize / 9 + 1),
        ));
        out
    }

    /// The sample as a host's archive: Git's order, by path.
    fn sample_archive() -> Vec<u8> {
        let mut files = sample();
        files.sort_by(|a, b| a.0.cmp(b.0));
        let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
        archive(&files)
    }

    /// The sample as a checkout: plus `.git` and symbolic links, which aren't read.
    fn sample_tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in sample() {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        std::fs::create_dir_all(dir.path().join(".git/objects")).unwrap();
        std::fs::write(dir.path().join(".git/config"), "Assembly.Load(x)").unwrap();
        std::fs::write(dir.path().join("src/.git"), "gitdir: ../.git/modules/src").unwrap();
        #[cfg(unix)]
        {
            let outside = dir.path().parent().unwrap().join("outside.rs");
            std::os::unix::fs::symlink(&outside, dir.path().join("src/outside.rs")).unwrap();
            std::os::unix::fs::symlink("src", dir.path().join("linked")).unwrap();
            std::os::unix::fs::symlink("/etc/passwd", dir.path().join("LICENSE-2")).unwrap();
        }
        dir
    }

    #[test]
    fn a_checkout_reads_like_its_archive() {
        let tree = sample_tree();
        let from_dir = scan_dir(tree.path()).unwrap();
        let from_archive = scan_archive_raw(&sample_archive()[..]).unwrap();
        assert_eq!(from_dir, from_archive);
        assert_eq!(from_dir.files, sample().len() as u64);
        assert!(!from_dir.truncated);
        for declared in [
            "MIT",
            "GPL-3.0-or-later",
            "Apache-2.0 OR MIT",
            "LicenseRef-Proprietary",
        ] {
            for public in [true, false] {
                let (report, packages) =
                    scan_archive(&sample_archive()[..], declared, public).unwrap();
                assert_eq!(
                    (report, packages),
                    finalize(from_dir.clone(), declared, public),
                    "{declared}, public: {public}"
                );
            }
        }
        // The sample has a bit of everything.
        let (report, packages) = finalize(from_dir, "MIT", true);
        assert_eq!(packages.len(), 3);
        assert_eq!(report.pulse, "heartbeat");
        assert_eq!(report.save_folder, "elsewhere");
        let sockets = report.network.iter().find(|h| h.rule == "sockets").unwrap();
        assert_eq!(
            sockets.examples,
            ["src/net-old.rs:1", "src/net.rs:1", "src/net/udp.rs:1"]
        );
        let headers: Vec<_> = report
            .licence
            .headers
            .iter()
            .map(|h| (h.rule.as_str(), h.count))
            .collect();
        assert_eq!(headers, [("Apache-2.0", 1), ("GPL-3.0-or-later", 2)]);
        let notices: Vec<_> = report
            .licence
            .notices
            .iter()
            .map(|h| h.rule.as_str())
            .collect();
        assert_eq!(
            notices,
            ["apache", "epic_games", "gpl", "unity_asset_store"]
        );
        assert!(
            report
                .code_loading
                .iter()
                .all(|h| h.examples.iter().all(|e| !e.starts_with("src/huge.rs")))
        );
        // GPL and Apache code is fine in a GPL-or-Apache project; vendor notices never are.
        let (report, _) = scan_archive(
            &sample_archive()[..],
            "GPL-3.0-or-later OR Apache-2.0",
            true,
        )
        .unwrap();
        let notices: Vec<_> = report
            .licence
            .notices
            .iter()
            .map(|h| h.rule.as_str())
            .collect();
        assert_eq!(notices, ["epic_games", "unity_asset_store"]);
        let headers: Vec<_> = report
            .licence
            .headers
            .iter()
            .map(|h| h.rule.as_str())
            .collect();
        assert_eq!(headers, ["MIT"], "only the header naming neither");
    }

    #[test]
    fn the_report_keeps_its_shape() {
        let (report, _) = scan_archive(&sample_archive()[..], "MIT", true).unwrap();
        let value = serde_json::to_value(&report).unwrap();
        let keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            [
                "code_files",
                "code_loading",
                "design",
                "files",
                "licence",
                "lockfiles",
                "monetization",
                "network",
                "packages",
                "pulse",
                "save_folder",
                "saves",
                "status",
                "truncated"
            ]
        );
        let licence: Vec<_> = value["licence"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(
            licence,
            [
                "closed", "declared", "files", "headers", "kinds", "notices", "problems"
            ]
        );
        assert_eq!(
            value["licence"]["files"][0],
            serde_json::json!({ "path": "LICENSE", "spdx": "MIT" })
        );
        assert_eq!(value["licence"]["files"][1]["spdx"], "Apache-2.0");
        let (empty, _) = finalize(RawScan::default(), "MIT", false);
        let value = serde_json::to_value(&empty).unwrap();
        for absent in [
            "saves",
            "save_folder",
            "pulse",
            "code_files",
            "design",
            "mod_code",
            "sandbox_mismatch",
        ] {
            assert!(value.get(absent).is_none(), "{absent}");
        }
    }

    #[test]
    fn raw_scans_travel_as_json() {
        let raw = scan_archive_raw(&sample_archive()[..]).unwrap();
        let check = SourceCheck {
            raw: raw.clone(),
            tool: tool(),
        };
        let json = serde_json::to_string(&check).unwrap();
        assert!(json.starts_with(r#"{"raw":{"files":"#));
        assert!(json.ends_with(&format!(
            r#""tool":"slop-source-check {}"}}"#,
            env!("CARGO_PKG_VERSION")
        )));
        let back: SourceCheck = serde_json::from_str(&json).unwrap();
        assert_eq!(back, check);
        assert_eq!(
            finalize(back.raw, "MIT", true),
            scan_archive(&sample_archive()[..], "MIT", true).unwrap()
        );
        // Something that isn't a raw scan isn't read as one.
        assert!(serde_json::from_str::<RawScan>(r#"{"files": 3}"#).is_err());
    }

    #[test]
    fn malformed_or_huge_raw_scans_are_refused() {
        let raw = scan_archive_raw(&sample_archive()[..]).unwrap();
        assert_eq!(raw.validate(), Ok(()));
        let hit = |examples: usize, long: usize| Hit {
            rule: "loot_box".into(),
            label: "x".repeat(long),
            count: examples.max(1) as u32,
            examples: vec!["a.cs:1".into(); examples],
        };
        let refused = |change: &dyn Fn(&mut RawScan)| {
            let mut bad = raw.clone();
            change(&mut bad);
            bad.validate().is_err()
        };
        assert!(refused(&|r| r.pulse = "sometimes".into()));
        assert!(refused(&|r| r.save_folder = String::new()));
        assert!(refused(&|r| r.monetization.push(hit(4, 1))));
        assert!(refused(&|r| r.monetization.push(hit(1, 1001))));
        assert!(!refused(&|r| r.monetization.push(hit(3, 1000))));
        assert!(refused(&|r| r.monetization[0].count = 0));
        assert!(refused(&|r| r.network = vec![hit(1, 1); 2001]));
        assert!(refused(&|r| r.notices[0].ok = vec!["GPL".into(); 17]));
        assert!(refused(&|r| r.lockfiles.push("a/".repeat(501))));
        assert!(refused(
            &|r| r.licence_files[0].spdx = Some("M".repeat(1001))
        ));
        assert!(refused(&|r| {
            r.packages = vec![
                Package {
                    ecosystem: "npm".into(),
                    name: "a".into(),
                    version: "1".into()
                };
                20_001
            ]
        }));
        assert!(refused(&|r| r.packages[0].name = "n".repeat(1001)));
        // Characters, not bytes.
        assert!(!refused(&|r| r.lockfiles.push("é".repeat(1000))));
    }
}
