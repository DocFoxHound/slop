// SPDX-License-Identifier: AGPL-3.0-or-later
//! A fix for every problem SLAPorSLOP's checks find (plan phase 10): `GET /v1/apps/{slug}/fixes`, for
//! the maker and their AI agent (maker_agents.rs).
//!
//! Each fix says what's wrong, where (file and line, from the checks), and exactly what to do, as
//! an instruction a coding agent can follow in the repository. They're built from fixed text and
//! the check results: SLAPorSLOP calls no AI model. The guide sections they point to are readable with
//! `GET /v1/maker/publishing-guide/{section}`. A browser game gets no fixes about what only the
//! SLAPorSLOP app's installs have (`SLOP_SAVE_DIR`, Slop Pulse), and its releases are play
//! addresses. The Content Policy's rule on code from outside SLAPorSLOP covers neither browser
//! games nor dedicated servers ("Browser games and dedicated servers"), so code loading is never a
//! `must` for them (`AppType`).

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{AppState, auth::session::CurrentUser, catalog, error::ApiResult};

pub fn routes() -> Router<AppState> {
    Router::new().route("/apps/{slug}/fixes", get(fixes))
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Fix {
    /// A short key, e.g. `licence:no_file`, `saves`, `secret`.
    pub kind: String,
    /// `must`: SLAPorSLOP won't approve the release until it's fixed. `should`: a reviewer may still
    /// approve, but players see it or it causes trouble. `recommended`: never holds anything up,
    /// but the store page is better with it (e.g. an icon).
    pub severity: &'static str,
    pub problem: String,
    /// What to do, written for a coding agent working in the repository.
    pub instruction: String,
    /// The publishing guide section with the details, if one helps.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guide_section: Option<&'static str>,
}

#[derive(Serialize)]
struct Fixes {
    /// The release the fixes are for (the newest), if there is one.
    release: Option<ReleaseRef>,
    fixes: Vec<Fix>,
    /// Worth doing, but never blocking a release or the store (e.g. "no icon yet"), whether or
    /// not there's a release.
    recommendations: Vec<Fix>,
    /// What to do once they're fixed.
    next: &'static str,
}

#[derive(Serialize)]
struct ReleaseRef {
    id: Uuid,
    version: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    status_reason: Option<String>,
}

/// Which of the Content Policy's rules an app's checks are read against ("Browser games and
/// dedicated servers").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppType {
    /// A download: every rule.
    Download,
    /// A download in the Servers category (`catalog::IS_DEDICATED_SERVER`): it may load mods,
    /// maps and plugins from other places, which the rule on code from outside SLAPorSLOP doesn't
    /// cover. It's still built from its source and checked like any other release.
    DedicatedServer,
    /// It runs on its maker's website: the rules on builds and on code from outside SLAPorSLOP
    /// don't cover it, and it never gets what only the SLAPorSLOP app's installs have
    /// (`SLOP_SAVE_DIR`, a Pulse token).
    BrowserGame,
}

impl AppType {
    pub async fn of(db: &PgPool, app_id: Uuid) -> sqlx::Result<Self> {
        let (browser, server): (bool, bool) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select 'browser' = any(a.platforms), {} from apps a where a.id = $1",
            catalog::IS_DEDICATED_SERVER
        )))
        .bind(app_id)
        .fetch_one(db)
        .await?;
        Ok(if browser {
            Self::BrowserGame
        } else if server {
            Self::DedicatedServer
        } else {
            Self::Download
        })
    }
}

type Row = (
    Uuid,
    String,
    String,
    Option<String>,
    Option<Value>,
    Option<Value>,
);

async fn fixes(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Fixes>> {
    let (app_id, _) = crate::updates::owned(&state, &slug, &user).await?;
    let app_type = AppType::of(&state.db, app_id).await?;
    let in_browser = app_type == AppType::BrowserGame;
    let mut recommendations = crate::media::recommendations(&state.db, app_id).await?;
    recommendations.extend(crate::catalog::tag_recommendations(&state.db, app_id).await?);
    let row: Option<Row> = sqlx::query_as(
        "select id, version, status::text, status_reason, repo_scan, source_checks
         from releases where app_id = $1 order by created_at desc limit 1",
    )
    .bind(app_id)
    .fetch_optional(&state.db)
    .await?;
    let Some((id, version, status, status_reason, repo_scan, source_checks)) = row else {
        return Ok(Json(Fixes {
            release: None,
            fixes: vec![],
            recommendations,
            next: if in_browser {
                "Give its play address first (submit_play_address, or the Releases page)."
            } else {
                "Submit a release first (submit_release, or add Slop Build with slop_build_setup)."
            },
        }));
    };
    let advisories: Vec<(String, String, String, Option<String>, bool)> = sqlx::query_as(
        "select package, version, vuln_id, fixed, malicious from release_advisories
         where release_id = $1 order by malicious desc, package limit 50",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let mut list = from_checks(
        repo_scan.as_ref().unwrap_or(&Value::Null),
        source_checks.as_ref().unwrap_or(&Value::Null),
        app_type,
    );
    list.extend(from_advisories(&advisories));
    if status == "failed" || status == "rejected" {
        list.insert(
            0,
            Fix {
                kind: format!("release:{status}"),
                severity: "must",
                problem: format!(
                    "Release {version} was {status}: {}",
                    status_reason.as_deref().unwrap_or("no reason given")
                ),
                instruction: if in_browser {
                    "Read the reason, fix what it names on the site or in the repository, then give the play address again with a new version (submit_play_address).".into()
                } else {
                    "Read the reason, fix what it names in the repository (or the release's files), then submit a new release with a new version.".into()
                },
                guide_section: None,
            },
        );
    }
    Ok(Json(Fixes {
        release: Some(ReleaseRef {
            id,
            version,
            status,
            status_reason,
        }),
        fixes: list,
        recommendations,
        next: if in_browser {
            "Fix the `must` items, then the `should` items; commit and push, and put the fixed game on your site; then give its play address again with a new version (submit_play_address). SLAPorSLOP's checks run again on it."
        } else {
            "Fix the `must` items, then the `should` items; commit and push; then submit a new release (Slop Build does it on push). SLAPorSLOP's checks run again on the new release."
        },
    }))
}

/// `path:line` examples of a check hit, for the instruction.
fn at(hit: &Value) -> String {
    let examples: Vec<&str> = hit["examples"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if examples.is_empty() {
        String::new()
    } else {
        format!(" (e.g. {})", examples.join(", "))
    }
}

fn hits(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten()
}

fn labels(v: &Value) -> String {
    hits(v)
        .map(|h| format!("{}{}", h["label"].as_str().unwrap_or("?"), at(h)))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The hits whose rule is (or, with `not`, isn't) `rule`, as a check's list.
fn with_rule(v: &Value, rule: &str, not: bool) -> Value {
    Value::Array(
        hits(v)
            .filter(|h| (h["rule"] == rule) != not)
            .cloned()
            .collect(),
    )
}

/// Fixes from SLAPorSLOP's source check (source_scan.rs) and Slop Build's checks, read against
/// the rules that cover `app_type`. A browser game runs on its maker's site: it never gets
/// `SLOP_SAVE_DIR` or a Pulse token, so saves and Pulse fixes don't apply to it, and neither does
/// the rule on code from outside SLAPorSLOP, which doesn't cover dedicated servers either.
pub fn from_checks(scan: &Value, build: &Value, app_type: AppType) -> Vec<Fix> {
    let in_browser = app_type == AppType::BrowserGame;
    let mut out = Vec::new();
    let mut fix = |kind: String, severity, problem: String, instruction: String, guide| {
        out.push(Fix {
            kind,
            severity,
            problem,
            instruction,
            guide_section: guide,
        })
    };
    // Secrets first: they're already public.
    for s in hits(&build["secrets"]) {
        let file = s["file"].as_str().unwrap_or("?");
        let line = s["line"]
            .as_u64()
            .map(|l| format!(":{l}"))
            .unwrap_or_default();
        fix(
            "secret".into(),
            "must",
            format!(
                "A possible secret ({}) is in {file}{line}.",
                s["rule"].as_str().unwrap_or("secret")
            ),
            format!(
                "If it's a real key, password or token: revoke it with whoever issued it first (it's in the repository's history, so removing the line isn't enough), then remove it from {file} and read it from an environment variable or a file outside the repository. If it isn't a secret, leave it and say so in your reply to the reviewer."
            ),
            None,
        );
    }
    for v in hits(&build["vulnerabilities"]) {
        fix(
            "vulnerable_package".into(),
            "should",
            format!(
                "{} {} ({}) has a known vulnerability: {} ({}).",
                v["package"].as_str().unwrap_or("?"),
                v["version"].as_str().unwrap_or("?"),
                v["ecosystem"].as_str().unwrap_or("?"),
                v["summary"].as_str().unwrap_or(""),
                v["id"].as_str().unwrap_or("")
            ),
            format!(
                "Update {} to a version without this vulnerability (see {} on osv.dev), update the lockfile, and check the app still builds and runs.",
                v["package"].as_str().unwrap_or("the package"),
                v["id"].as_str().unwrap_or("the advisory")
            ),
            None,
        );
    }
    if scan["status"] != "checked" {
        return out;
    }
    let licence = &scan["licence"];
    let declared = licence["declared"]
        .as_str()
        .unwrap_or("the store page's licence");
    let kinds: Vec<&str> = hits(&licence["kinds"]).filter_map(Value::as_str).collect();
    let problems: Vec<&str> = hits(&licence["problems"])
        .filter_map(Value::as_str)
        .collect();
    for (kind, problem) in kinds.iter().zip(&problems) {
        let instruction = match *kind {
            "no_file" => format!("Add the full, official text of {declared} as LICENSE at the top of the repository: copy it from https://spdx.org/licenses/{declared}.html, don't write or shorten it. Fill in the year and copyright holder where it asks."),
            "unidentified" => format!("Replace LICENSE with the unchanged official text of {declared} (https://spdx.org/licenses/{declared}.html), so it can be recognised."),
            "mismatch" => "Make the licence file and the store page agree: either change the store page's licence_spdx (update_store_page) to the licence in the file, or replace the file with the store page's licence, whichever the copyright holders chose.".into(),
            k if k.starts_with("header:") => format!("Some files say another licence in their SPDX-License-Identifier header. If that code came from another project, add its licence to the store page's licence (an SPDX expression like \"{declared} AND MIT\") and keep its notice; if the header is a mistake, change it to {declared}."),
            k if k.starts_with("notice:") => "Code from elsewhere carries its own licence. Either name that licence too in the store page's licence_spdx and keep its notice, or remove that code and use something under a compatible licence.".into(),
            k if k.starts_with("closed:") => "Remove the dependency on the closed-source library: an open-source app can't need it. For NVIDIA Cg, rewrite the shaders in GLSL or HLSL and load them with the engine's own shader support.".into(),
            _ => format!("Fix this licence problem: {problem}"),
        };
        fix(
            format!("licence:{kind}"),
            if *kind == "unidentified" {
                "should"
            } else {
                "must"
            },
            problem.to_string(),
            instruction,
            None,
        );
    }
    if hits(&scan["monetization"]).next().is_some() {
        fix(
            "purchases".into(),
            "must",
            format!("The code sells things or takes payments: {}.", labels(&scan["monetization"])),
            "Remove all purchases, payment links, loot boxes and premium currencies: apps on SLAPorSLOP are free and sell nothing. Players can support the maker with SLAPorSLOP's donate button instead, which gives nothing in return. Remove the payment libraries from the dependencies too.".into(),
            None,
        );
    }
    match app_type {
        // Loading code from its own site is how a web page works, and the rule doesn't cover it.
        AppType::BrowserGame => {}
        // Allowed for a server; players are told on its store page. An unsafe data format is
        // still worth replacing: it's what lets a message run code where the server runs.
        AppType::DedicatedServer => {
            let loading = with_rule(&scan["code_loading"], "unsafe_format", true);
            if hits(&loading).next().is_some() {
                fix(
                    "code_loading".into(),
                    "should",
                    format!("The server loads code from outside SLAPorSLOP: {}.", labels(&loading)),
                    "Nothing here is against the Content Policy: dedicated servers may load mods, maps and plugins from other places, and the store page tells players so. SLAPorSLOP checks only what's in the release, so say in the store page's description (update_store_page) what the server loads and from where.".into(),
                    Some("Dedicated servers"),
                );
            }
            let unsafe_formats = with_rule(&scan["code_loading"], "unsafe_format", false);
            if hits(&unsafe_formats).next().is_some() {
                fix(
                    "unsafe_format".into(),
                    "should",
                    format!("The server uses a data format that can rebuild any object: {}.", labels(&unsafe_formats)),
                    "For anything from players, other servers or files people send, use a format that can't rebuild objects (JSON, MessagePack, protobuf or your own plain format) instead of BinaryFormatter, pickle and the like, so a message can't run code on the computer the server runs on. Reading only the server's own bundled files that way is fine.".into(),
                    Some("Rules for the code"),
                );
            }
        }
        AppType::Download => {
            if hits(&scan["code_loading"]).next().is_some() {
                fix(
                    "code_loading".into(),
                    "must",
                    format!(
                        "The code loads or replaces code from outside SLAPorSLOP: {}.",
                        labels(&scan["code_loading"])
                    ),
                    "Remove run-time code loading from the network, self-updaters and unsafe data formats (e.g. pickle, BinaryFormatter). SLAPorSLOP delivers every update itself, after checking it. Mods are fine when they come from SLAPorSLOP: read them from SLOP_MODS or your mods folder.".into(),
                    Some("Rules for the code"),
                );
            }
        }
    }
    if scan["online_undeclared"] == true {
        if in_browser {
            // A site is on the web anyway: only other hosts are worth telling players about.
            fix(
                "online_undeclared".into(),
                "should",
                format!("The game's code connects online ({}), but the store page doesn't say so.", labels(&scan["network"])),
                "If the game connects to servers besides its own site (multiplayer, leaderboards, online services), say so on the store page (update_store_page: online, and online_hosts with each host), so players know. If it only loads its own files from its site, leave it.".into(),
                Some("Online play and mods"),
            );
        } else {
            fix(
                "online_undeclared".into(),
                "must",
                format!("The code goes online ({}), but the store page doesn't say so.", labels(&scan["network"])),
                "Either declare it on the store page (update_store_page: online, and online_hosts with every host it connects to), or remove the networking code if it isn't needed.".into(),
                Some("Online play and mods"),
            );
        }
    }
    if scan["save_folder"] == "elsewhere" && !in_browser {
        fix(
            "saves".into(),
            if scan["sandbox_mismatch"] == true { "must" } else { "should" },
            format!("The app saves outside SLAPorSLOP's save folder: {}.", labels(&scan["saves"])),
            "Save under the folder in the SLOP_SAVE_DIR environment variable when it's set, and keep the current location only as a fallback when it isn't. Write a new file and rename it over the old one, so a crash never leaves half a save. Then players' saves survive reinstalls and sandbox mode works.".into(),
            Some("Saves and sandbox mode"),
        );
    }
    if (scan["pulse_missing"] == true || scan["pulse"] == "start_only") && !in_browser {
        fix(
            "pulse".into(),
            "should",
            if scan["pulse"] == "start_only" {
                "The app starts Slop Pulse but never sends heartbeats, so it shows no players in game.".into()
            } else {
                "The original app uses Slop Pulse, but this build doesn't.".into()
            },
            "Send Slop Pulse's start, then a heartbeat every minute while the game runs, with the token from SLOP_PULSE_TOKEN, as in the guide's snippet for your engine.".into(),
            Some("Slop Pulse (player counts)"),
        );
    }
    if scan["mod_online"] == true {
        fix(
            "mod_online".into(),
            "should",
            "This mod's code connects online. It can still be approved, but players are told and offered sandbox mode.".into(),
            "Remove the networking code if the mod doesn't need it. If it does, declare the hosts on the mod's store page (online_hosts).".into(),
            Some("Publishing a mod"),
        );
    }
    out
}

fn from_advisories(rows: &[(String, String, String, Option<String>, bool)]) -> Vec<Fix> {
    rows.iter()
        .map(|(package, version, id, fixed, malicious)| Fix {
            kind: if *malicious { "malicious_package" } else { "vulnerable_package" }.into(),
            severity: if *malicious { "must" } else { "should" },
            problem: if *malicious {
                format!("{package} {version} is a known malicious package ({id}).")
            } else {
                format!("{package} {version} has a newly found vulnerability ({id}).")
            },
            instruction: match (malicious, fixed) {
                (true, _) => format!("Remove {package} from the dependencies at once and check nothing in the repository runs it; look for what it may have changed. Then release again."),
                (false, Some(f)) => format!("Update {package} to {f} or newer, update the lockfile, and release again."),
                (false, None) => format!("No fixed version of {package} exists yet: replace it, or remove the feature that needs it, if the vulnerability affects your app."),
            },
            guide_section: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn each_problem_gets_a_fix_with_its_location() {
        let scan = json!({
            "status": "checked",
            "licence": { "declared": "MIT", "kinds": ["no_file", "closed:nvidia_cg"],
                         "problems": ["There's no licence file", "2 files build against NVIDIA's Cg"] },
            "monetization": [{ "rule": "loot_box", "label": "Loot boxes or gacha", "count": 1, "examples": ["src/shop.rs:12"] }],
            "save_folder": "elsewhere",
            "saves": [{ "rule": "appdata", "label": "AppData", "count": 1, "examples": ["src/save.rs:4"] }],
            "sandbox_mismatch": true,
            "online_undeclared": true,
            "network": [{ "rule": "http", "label": "HTTP requests", "count": 1, "examples": ["src/net.rs:9"] }],
            "pulse": "start_only",
        });
        let build = json!({ "secrets": [{ "file": ".env", "line": 3, "rule": "aws-access-token" }], "vulnerabilities": [] });
        let fixes = from_checks(&scan, &build, AppType::Download);
        let kinds: Vec<&str> = fixes.iter().map(|f| f.kind.as_str()).collect();
        assert_eq!(
            kinds,
            [
                "secret",
                "licence:no_file",
                "licence:closed:nvidia_cg",
                "purchases",
                "online_undeclared",
                "saves",
                "pulse"
            ]
        );
        let online = fixes
            .iter()
            .find(|f| f.kind == "online_undeclared")
            .unwrap();
        assert_eq!(online.severity, "must");
        assert!(fixes[0].problem.contains(".env:3"));
        assert!(fixes[1].instruction.contains("spdx.org/licenses/MIT.html"));
        assert!(fixes[3].problem.contains("src/shop.rs:12"));
        let saves = fixes.iter().find(|f| f.kind == "saves").unwrap();
        assert_eq!(saves.severity, "must", "the store page says sandbox-ready");
        assert_eq!(saves.guide_section, Some("Saves and sandbox mode"));

        // A browser game never gets SLOP_SAVE_DIR or a Pulse token: no fixes asking for them,
        // and none saying SLAPorSLOP delivers its updates. The rule on code from outside
        // SLAPorSLOP doesn't cover it, so its code loading needs no fix; and it's on the web
        // anyway, so going online unsaid is only a `should`.
        let mut scan = scan;
        scan["code_loading"] = json!([
            { "rule": "load_script", "label": "Runs code from text", "count": 1, "examples": ["src/game.js:7"] },
            { "rule": "unsafe_format", "label": "A data format that can rebuild any object", "count": 1, "examples": ["src/save.js:2"] }
        ]);
        let fixes = from_checks(&scan, &build, AppType::BrowserGame);
        let kinds: Vec<&str> = fixes.iter().map(|f| f.kind.as_str()).collect();
        assert_eq!(
            kinds,
            [
                "secret",
                "licence:no_file",
                "licence:closed:nvidia_cg",
                "purchases",
                "online_undeclared"
            ]
        );
        let online = fixes
            .iter()
            .find(|f| f.kind == "online_undeclared")
            .unwrap();
        assert_eq!(online.severity, "should");
        assert!(online.instruction.contains("online_hosts"));
        assert!(online.problem.contains("HTTP requests"));
        for f in &fixes {
            for app_only in [
                "SLOP_SAVE_DIR",
                "SLOP_PULSE_TOKEN",
                "SLOP_MODS",
                "delivers every update",
            ] {
                assert!(
                    !f.instruction.contains(app_only),
                    "{}: {}",
                    f.kind,
                    f.instruction
                );
            }
        }
    }

    /// A dedicated server may load mods, maps and plugins from other places: its code loading is
    /// a `should` (say what it loads), never a `must`, and its unsafe formats a `should` of their
    /// own. Everything else is read as for any download.
    #[test]
    fn a_dedicated_servers_code_loading_is_never_a_must() {
        let scan = json!({
            "status": "checked",
            "licence": { "declared": "MIT", "kinds": [], "problems": [] },
            "monetization": [{ "rule": "loot_box", "label": "Loot boxes or gacha", "count": 1, "examples": ["src/shop.rs:12"] }],
            "code_loading": [
                { "rule": "load_native", "label": "Loads native libraries at run time", "count": 2, "examples": ["src/plugins.rs:40"] },
                { "rule": "unsafe_format", "label": "A data format that can rebuild any object", "count": 1, "examples": ["src/net.cs:9"] }
            ],
            "online_undeclared": true,
            "network": [{ "rule": "sockets", "label": "Network sockets (TCP, UDP)", "count": 1, "examples": ["src/net.rs:3"] }],
            "save_folder": "slop",
            "pulse": "heartbeat",
        });
        let build = json!({ "secrets": [], "vulnerabilities": [] });
        let fixes = from_checks(&scan, &build, AppType::DedicatedServer);
        let got: Vec<(&str, &str)> = fixes
            .iter()
            .map(|f| (f.kind.as_str(), f.severity))
            .collect();
        assert_eq!(
            got,
            [
                ("purchases", "must"),
                ("code_loading", "should"),
                ("unsafe_format", "should"),
                ("online_undeclared", "must")
            ]
        );
        let code = &fixes[1];
        assert!(
            code.problem.contains("src/plugins.rs:40"),
            "{}",
            code.problem
        );
        assert!(!code.problem.contains("src/net.cs"), "{}", code.problem);
        assert!(code.instruction.contains("mods, maps and plugins"));
        assert!(code.instruction.contains("only what's in the release"));
        assert_eq!(code.guide_section, Some("Dedicated servers"));
        assert!(fixes[2].problem.contains("src/net.cs:9"));
        // The same scan on a download: code loading is a must, as before.
        let download = from_checks(&scan, &build, AppType::Download);
        let code = download.iter().find(|f| f.kind == "code_loading").unwrap();
        assert_eq!(code.severity, "must");
        assert!(code.problem.contains("src/net.cs:9"));
        assert!(download.iter().all(|f| f.kind != "unsafe_format"));
        // Only code loading: no unsafe-format fix, and the other way round.
        let mut only = scan.clone();
        only["code_loading"] = json!([scan["code_loading"][1]]);
        let kinds: Vec<String> = from_checks(&only, &build, AppType::DedicatedServer)
            .into_iter()
            .map(|f| f.kind)
            .collect();
        assert!(kinds.contains(&"unsafe_format".to_string()));
        assert!(!kinds.contains(&"code_loading".to_string()));
    }

    /// The guide sections fixes point to are headings in the publishing guide, which agents read
    /// by name (`GET /v1/maker/publishing-guide/{section}`).
    #[test]
    fn the_guide_sections_fixes_name_are_there() {
        let guide = include_str!("../../../web/public/publishing.md");
        for section in [
            "Rules for the code",
            "Dedicated servers",
            "Online play and mods",
            "Saves and sandbox mode",
            "Slop Pulse (player counts)",
            "Publishing a mod",
        ] {
            assert!(
                guide
                    .lines()
                    .any(|l| l == format!("## {section}") || l == format!("### {section}")),
                "{section}"
            );
        }
    }

    #[test]
    fn a_clean_check_needs_nothing() {
        let scan = json!({ "status": "checked", "licence": { "declared": "MIT", "kinds": [], "problems": [] }, "save_folder": "slop", "pulse": "heartbeat" });
        assert!(
            from_checks(
                &scan,
                &json!({ "secrets": [], "vulnerabilities": [] }),
                AppType::Download
            )
            .is_empty()
        );
        for app_type in [
            AppType::Download,
            AppType::DedicatedServer,
            AppType::BrowserGame,
        ] {
            assert!(
                from_checks(&json!({ "status": "not_checked" }), &Value::Null, app_type).is_empty()
            );
        }
    }
}
