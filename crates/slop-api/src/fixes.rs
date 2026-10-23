// SPDX-License-Identifier: AGPL-3.0-or-later
//! A fix for every problem Slop's checks find (plan phase 10): `GET /v1/apps/{slug}/fixes`, for
//! the maker and their AI agent (maker_agents.rs).
//!
//! Each fix says what's wrong, where (file and line, from the checks), and exactly what to do, as
//! an instruction a coding agent can follow in the repository. They're built from fixed text and
//! the check results: Slop calls no AI model. The guide sections they point to are readable with
//! `GET /v1/maker/publishing-guide/{section}`.

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::{AppState, auth::session::CurrentUser, error::ApiResult};

pub fn routes() -> Router<AppState> {
    Router::new().route("/apps/{slug}/fixes", get(fixes))
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Fix {
    /// A short key, e.g. `licence:no_file`, `saves`, `secret`.
    pub kind: String,
    /// `must`: Slop won't approve the release until it's fixed. `should`: a reviewer may still
    /// approve, but players see it or it causes trouble.
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
            next: "Submit a release first (submit_release, or add Slop Build with slop_build_setup).",
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
                instruction: "Read the reason, fix what it names in the repository (or the release's files), then submit a new release with a new version.".into(),
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
        next: "Fix the `must` items, then the `should` items; commit and push; then submit a new release (Slop Build does it on push). Slop's checks run again on the new release.",
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

/// Fixes from Slop's source check (source_scan.rs) and Slop Build's checks.
pub fn from_checks(scan: &Value, build: &Value) -> Vec<Fix> {
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
            "Remove all purchases, payment links, loot boxes and premium currencies: apps on Slop are free and sell nothing. Players can support the maker with Slop's donate button instead, which gives nothing in return. Remove the payment libraries from the dependencies too.".into(),
            None,
        );
    }
    if hits(&scan["code_loading"]).next().is_some() {
        fix(
            "code_loading".into(),
            "must",
            format!("The code loads or replaces code from outside Slop: {}.", labels(&scan["code_loading"])),
            "Remove run-time code loading from the network, self-updaters and unsafe data formats (e.g. pickle, BinaryFormatter). Slop delivers every update itself, after checking it. Mods are fine when they come from Slop: read them from SLOP_MODS or your mods folder.".into(),
            Some("Rules for the code"),
        );
    }
    if scan["online_undeclared"] == true {
        fix(
            "online_undeclared".into(),
            "must",
            format!("The code goes online ({}), but the store page doesn't say so.", labels(&scan["network"])),
            "Either declare it on the store page (update_store_page: online, and online_hosts with every host it connects to), or remove the networking code if it isn't needed.".into(),
            Some("Online play and mods"),
        );
    }
    if scan["save_folder"] == "elsewhere" {
        fix(
            "saves".into(),
            if scan["sandbox_mismatch"] == true { "must" } else { "should" },
            format!("The app saves outside Slop's save folder: {}.", labels(&scan["saves"])),
            "Save under the folder in the SLOP_SAVE_DIR environment variable when it's set, and keep the current location only as a fallback when it isn't. Write a new file and rename it over the old one, so a crash never leaves half a save. Then players' saves survive reinstalls and sandbox mode works.".into(),
            Some("Saves and sandbox mode"),
        );
    }
    if scan["pulse_missing"] == true || scan["pulse"] == "start_only" {
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
        let fixes = from_checks(&scan, &build);
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
        assert!(fixes[0].problem.contains(".env:3"));
        assert!(fixes[1].instruction.contains("spdx.org/licenses/MIT.html"));
        assert!(fixes[3].problem.contains("src/shop.rs:12"));
        let saves = fixes.iter().find(|f| f.kind == "saves").unwrap();
        assert_eq!(saves.severity, "must", "the store page says sandbox-ready");
        assert_eq!(saves.guide_section, Some("Saves and sandbox mode"));
    }

    #[test]
    fn a_clean_check_needs_nothing() {
        let scan = json!({ "status": "checked", "licence": { "declared": "MIT", "kinds": [], "problems": [] }, "save_folder": "slop", "pulse": "heartbeat" });
        assert!(from_checks(&scan, &json!({ "secrets": [], "vulnerabilities": [] })).is_empty());
        assert!(from_checks(&json!({ "status": "not_checked" }), &Value::Null).is_empty());
    }
}
