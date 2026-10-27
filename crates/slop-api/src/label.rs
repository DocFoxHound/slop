// SPDX-License-Identifier: AGPL-3.0-or-later
//! What an app does, in plain words (plan phase 11, DESIGN §5).
//!
//! - **The label** (`GET /v1/apps/{slug}/label`): online, saves, sandbox mode, outside code,
//!   purchases, mods, its own installer, shared runtimes, licence and scans, each marked as
//!   found by SLAPorSLOP's checks or said by the maker. Built from the store page and the newest
//!   approved release's checks; SLAPorSLOP calls no AI. Says "checked" or "scanned", never "safe".
//! - **Accessibility:** the features the maker says it has, and players who launched it
//!   confirming or disputing each (`PUT`/`DELETE /v1/apps/{slug}/accessibility/{feature}`).
//! - **Glow-ups** (`GET /v1/shelves/glow-ups`): apps whose % Slaps rose clearly since 60 days
//!   ago, with the before and after, for the store front.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, put},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::QueryBuilder;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::{CurrentUser, MaybeUser},
    catalog::{self, AppSummary},
    error::{ApiError, ApiResult},
    policy::ViewerPolicy,
};

/// Accessibility features a maker can declare: (key, what players see).
pub const FEATURES: &[(&str, &str)] = &[
    ("remap_controls", "Remappable controls"),
    ("colorblind", "Colourblind modes"),
    ("subtitles", "Subtitles or captions"),
    ("text_size", "Adjustable text size"),
    ("one_handed", "Playable one-handed"),
    ("difficulty", "Difficulty or assist options"),
    ("no_flashing", "No flashing lights"),
    ("screen_reader", "Works with a screen reader"),
    ("controller", "Full controller support"),
];

pub(crate) fn validate_accessibility(features: &[String]) -> ApiResult<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for f in features {
        if !FEATURES.iter().any(|(k, _)| k == f) {
            return Err(ApiError::invalid(
                "accessibility",
                format!(
                    "use {}",
                    FEATURES
                        .iter()
                        .map(|(k, _)| *k)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
        if !out.contains(f) {
            out.push(f.clone());
        }
    }
    Ok(out)
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/apps/{slug}/label", get(label))
        .route(
            "/apps/{slug}/accessibility/{feature}",
            put(report).delete(withdraw),
        )
        .route("/shelves/glow-ups", get(glow_ups))
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Item {
    pub key: &'static str,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// From SLAPorSLOP's own checks (true) or what the maker says (false).
    pub checked: bool,
    /// `good`, `info` or `note` (something to know before installing).
    pub tone: &'static str,
}

#[derive(Serialize)]
struct Feature {
    key: &'static str,
    name: &'static str,
    confirmed: i64,
    disputed: i64,
    /// The viewer's own report: true works, false doesn't.
    mine: Option<bool>,
}

#[derive(Serialize)]
struct Label {
    items: Vec<Item>,
    accessibility: Vec<Feature>,
    /// The viewer launched it, so they can confirm or dispute accessibility features.
    can_report: bool,
    /// The release the checks are from.
    version: Option<String>,
}

/// What the label needs from the store page.
pub struct AppFacts {
    pub kind_is_mod: bool,
    pub online: Vec<String>,
    pub online_hosts: Vec<String>,
    pub sandbox_ready: bool,
    pub takes_mods: bool,
    pub license: String,
}

/// What the label needs from the newest approved release.
pub struct ReleaseFacts {
    pub repo_scan: Value,
    pub runtimes: Vec<String>,
    pub has_installer: bool,
    /// Every file's scan result.
    pub scans: Vec<Option<String>>,
}

fn labels(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|h| h["label"].as_str().map(String::from))
        .collect()
}

/// The label's items, in the order players read them.
pub fn items(app: &AppFacts, release: Option<&ReleaseFacts>, scanning: bool) -> Vec<Item> {
    let mut out = Vec::new();
    let mut push = |key, title: String, detail: Option<String>, checked, tone| {
        out.push(Item {
            key,
            title,
            detail,
            checked,
            tone,
        })
    };
    let scan = release
        .map(|r| &r.repo_scan)
        .filter(|s| s["status"] == "checked");

    // Online.
    if !app.online.is_empty() {
        push(
            "online",
            "Goes online".into(),
            Some(if app.online_hosts.is_empty() {
                "To servers SLAPorSLOP can't check. Sandbox mode is suggested.".into()
            } else {
                format!(
                    "To {}. SLAPorSLOP can't check those servers.",
                    app.online_hosts.join(", ")
                )
            }),
            false,
            "note",
        );
    } else if let Some(s) = scan {
        if s["online_undeclared"] == true || s["mod_online"] == true {
            push(
                "online",
                "Has code that goes online".into(),
                Some(format!("Found: {}.", labels(&s["network"]).join(", "))),
                true,
                "note",
            );
        } else {
            push(
                "online",
                "Works offline".into(),
                Some("No networking code found.".into()),
                true,
                "good",
            );
        }
    }

    // Saves and sandbox mode.
    if let Some(s) = scan {
        let (title, detail, tone) = match s["save_folder"].as_str() {
            Some("slop" | "engine") => (
                "Saves in SLAPorSLOP's save folder",
                "They stay when you uninstall, and work in sandbox mode.",
                "good",
            ),
            Some("elsewhere") => (
                "Saves outside SLAPorSLOP's save folder",
                "SLAPorSLOP can't keep them when you uninstall, and sandbox mode may lose them.",
                "note",
            ),
            _ => (
                "No saves found",
                "The code doesn't seem to save anything on your computer.",
                "info",
            ),
        };
        push("saves", title.into(), Some(detail.into()), true, tone);
    }
    let mismatch = scan.is_some_and(|s| s["save_folder"] == "elsewhere");
    if app.sandbox_ready && !mismatch {
        push(
            "sandbox",
            "Works in sandbox mode".into(),
            Some("You can run it with no access to the rest of your computer.".into()),
            false,
            "good",
        );
    }

    // Outside code, purchases.
    if let Some(s) = scan {
        let code = labels(&s["code_loading"]);
        if code.is_empty() {
            push(
                "outside_code",
                "Loads no code from outside SLAPorSLOP".into(),
                None,
                true,
                "good",
            );
        } else {
            push(
                "outside_code",
                "Loads code from outside SLAPorSLOP".into(),
                Some(format!(
                    "{}. A person reviewed it before approving.",
                    code.join(", ")
                )),
                true,
                "note",
            );
        }
        if labels(&s["monetization"]).is_empty() {
            push(
                "purchases",
                "Sells nothing".into(),
                Some("No purchases or payment code found.".into()),
                true,
                "good",
            );
        }
        let design = labels(&s["design"]);
        if !design.is_empty() {
            push(
                "design",
                "A person checked its design".into(),
                Some(format!("SLAPorSLOP noticed: {}.", design.join(", "))),
                true,
                "info",
            );
        }
        if app.kind_is_mod && s["mod_code"] == true {
            push(
                "mod_code",
                "Contains code".into(),
                Some("This mod runs code inside the game.".into()),
                true,
                "note",
            );
        }
    }
    if app.takes_mods {
        push(
            "mods",
            "Uses mods from SLAPorSLOP".into(),
            Some("Only mods approved on SLAPorSLOP are kept in its mods folder.".into()),
            false,
            "info",
        );
    }

    // Installing.
    if let Some(r) = release {
        if r.has_installer {
            push(
                "installer",
                "Runs its own installer".into(),
                Some("The SLAPorSLOP app asks you first.".into()),
                false,
                "note",
            );
        }
        let names: Vec<&str> = r
            .runtimes
            .iter()
            .filter_map(|id| slop_manifest::runtimes::runtime(id))
            .map(|rt| rt.name)
            .collect();
        if !names.is_empty() {
            push(
                "runtimes",
                "Needs shared runtimes".into(),
                Some(format!(
                    "{} from Microsoft, installed if missing after you agree.",
                    names.join(", ")
                )),
                false,
                "info",
            );
        }
        let scanned = r
            .scans
            .iter()
            .all(|s| matches!(s.as_deref(), Some("passed" | "note")));
        if scanning && scanned && !r.scans.is_empty() {
            push(
                "scanned",
                "Files scanned".into(),
                Some("SLAPorSLOP scanned every file before approving it.".into()),
                true,
                "good",
            );
        } else if !scanning {
            push(
                "scanned",
                "Files not scanned".into(),
                Some("This SLAPorSLOP doesn't scan files.".into()),
                true,
                "note",
            );
        }
    }
    push(
        "licence",
        "Open source".into(),
        Some(format!(
            "{}: anyone can read, check and improve the code.",
            app.license
        )),
        false,
        "good",
    );
    out
}

async fn label(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<Json<Label>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let (app_id, _) = catalog::visible_app(&state, &slug, user.as_ref(), policy).await?;
    type AppRow = (
        bool,
        Vec<String>,
        Vec<String>,
        bool,
        bool,
        String,
        Vec<String>,
    );
    let (kind_is_mod, online, online_hosts, sandbox_ready, takes_mods, license, declared): AppRow = sqlx::query_as(
        "select branch_kind is not distinct from 'addon', online, online_hosts, sandbox_ready, mods_folder is not null,
                license_spdx, accessibility
         from apps where id = $1",
    )
    .bind(app_id)
    .fetch_one(&state.db)
    .await?;
    let release: Option<(Uuid, String, Option<Value>, Vec<String>)> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select r.id, r.version, r.repo_scan, r.runtimes {} limit 1",
            crate::releases::SERVING
        )))
        .bind(app_id)
        .fetch_optional(&state.db)
        .await?;
    let (version, release) = match release {
        Some((id, version, scan, runtimes)) => {
            let files: Vec<(String, Option<String>)> =
                sqlx::query_as("select kind, scan_result from release_files where release_id = $1")
                    .bind(id)
                    .fetch_all(&state.db)
                    .await?;
            (
                Some(version),
                Some(ReleaseFacts {
                    repo_scan: scan.unwrap_or(Value::Null),
                    runtimes,
                    has_installer: files.iter().any(|(k, _)| k == "installer"),
                    scans: files.into_iter().map(|(_, s)| s).collect(),
                }),
            )
        }
        None => (None, None),
    };
    let facts = AppFacts {
        kind_is_mod,
        online,
        online_hosts,
        sandbox_ready,
        takes_mods,
        license,
    };
    let viewer = user.as_ref().map(|u| u.id);
    let counts: Vec<(String, i64, i64, Option<bool>)> = sqlx::query_as(
        "select feature, count(*) filter (where works), count(*) filter (where not works),
                bool_or(works) filter (where user_id = $2)
         from accessibility_reports where app_id = $1 group by feature",
    )
    .bind(app_id)
    .bind(viewer)
    .fetch_all(&state.db)
    .await?;
    let accessibility = FEATURES
        .iter()
        .filter(|(k, _)| declared.iter().any(|d| d == k))
        .map(|(key, name)| {
            let c = counts.iter().find(|(f, ..)| f == key);
            Feature {
                key,
                name,
                confirmed: c.map_or(0, |c| c.1),
                disputed: c.map_or(0, |c| c.2),
                mine: c.and_then(|c| c.3),
            }
        })
        .collect();
    let can_report =
        match viewer {
            Some(id) => sqlx::query_scalar(
                "select exists (select 1 from user_app_usage where user_id = $1 and app_id = $2)",
            )
            .bind(id)
            .bind(app_id)
            .fetch_one(&state.db)
            .await?,
            None => false,
        };
    Ok(Json(Label {
        items: items(
            &facts,
            release.as_ref(),
            state.config.releases.scanner.is_on(),
        ),
        accessibility,
        can_report,
        version,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Report {
    works: bool,
}

/// A player who launched the app confirms (or disputes) one of its accessibility features.
async fn report(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path((slug, feature)): Path<(String, String)>,
    Json(b): Json<Report>,
) -> ApiResult<StatusCode> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, Some(&user), &headers).await?;
    let (app_id, is_mine) = catalog::visible_app(&state, &slug, Some(&user), policy).await?;
    if is_mine {
        return Err(ApiError::Conflict(
            "players confirm these, not the maker".into(),
        ));
    }
    let declared: bool =
        sqlx::query_scalar("select $2 = any(accessibility) from apps where id = $1")
            .bind(app_id)
            .bind(&feature)
            .fetch_one(&state.db)
            .await?;
    if !declared {
        return Err(ApiError::NotFound);
    }
    let played: bool = sqlx::query_scalar(
        "select exists (select 1 from user_app_usage where user_id = $1 and app_id = $2)",
    )
    .bind(user.id)
    .bind(app_id)
    .fetch_one(&state.db)
    .await?;
    if !played {
        return Err(ApiError::Conflict(
            "play it through SLAPorSLOP first".into(),
        ));
    }
    sqlx::query(
        "insert into accessibility_reports (user_id, app_id, feature, works) values ($1, $2, $3, $4)
         on conflict (user_id, app_id, feature) do update set works = $4, updated_at = now()",
    )
    .bind(user.id)
    .bind(app_id)
    .bind(&feature)
    .bind(b.works)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn withdraw(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((slug, feature)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    sqlx::query(
        "delete from accessibility_reports r using apps a
         where a.id = r.app_id and a.slug = $2 and r.user_id = $1 and r.feature = $3",
    )
    .bind(user.id)
    .bind(&slug)
    .bind(&feature)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A clear rise: at least this many points of % Slaps.
const GLOW_UP_POINTS: f64 = 15.0;

#[derive(Serialize)]
struct GlowUp {
    #[serde(flatten)]
    app: AppSummary,
    /// % Slaps 60 days ago, and now (unweighted).
    before: i32,
    after: i32,
}

/// Apps whose % Slaps rose clearly since 60 days ago: each player's vote as it stood then
/// (vote_events) against the votes now, both with enough votes.
async fn glow_ups(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<GlowUp>>> {
    let policy = ViewerPolicy::resolve(&state.db, &state.config, user.as_ref(), &headers).await?;
    let mut qb = QueryBuilder::new(
        "with then_votes as (
             select distinct on (e.app_id, e.user_id) e.app_id, e.vote
             from vote_events e where e.created_at < now() - interval '60 days'
             order by e.app_id, e.user_id, e.created_at desc, e.id desc),
         before as (
             select app_id, count(*) as n, 100.0 * count(*) filter (where vote = 1) / count(*) as pct
             from then_votes where vote <> 0 group by app_id),
         after as (
             select app_id, count(*) as n, 100.0 * count(*) filter (where vote = 1) / count(*) as pct
             from votes group by app_id)
         select a.id, round(b.pct)::int4, round(f.pct)::int4
         from apps a join before b on b.app_id = a.id join after f on f.app_id = a.id
         where b.n >= ",
    );
    qb.push_bind(crate::votes::MIN_VOTES)
        .push(" and f.n >= ")
        .push_bind(crate::votes::MIN_VOTES)
        .push(" and f.pct - b.pct >= ")
        .push_bind(GLOW_UP_POINTS)
        .push(" and a.parent_app_id is null and ");
    catalog::push_store_visible(&mut qb, state.config.catalog, policy);
    qb.push(" order by f.pct - b.pct desc limit 8");
    let rows: Vec<(Uuid, i32, i32)> = qb.build_query_as().fetch_all(&state.db).await?;
    let mut out = Vec::new();
    for (id, before, after) in rows {
        let mut qb = QueryBuilder::new("");
        catalog::push_summary_columns(&mut qb, &state.config);
        qb.push(" where a.id = ").push_bind(id);
        let app: AppSummary = qb.build_query_as().fetch_one(&state.db).await?;
        out.push(GlowUp { app, before, after });
    }
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app() -> AppFacts {
        AppFacts {
            kind_is_mod: false,
            online: vec![],
            online_hosts: vec![],
            sandbox_ready: true,
            takes_mods: false,
            license: "MIT".into(),
        }
    }

    #[test]
    fn a_clean_offline_game() {
        let release = ReleaseFacts {
            repo_scan: json!({ "status": "checked", "save_folder": "slop", "code_loading": [], "monetization": [], "network": [] }),
            runtimes: vec!["vcredist-x64".into()],
            has_installer: false,
            scans: vec![Some("passed".into())],
        };
        let keys: Vec<&str> = items(&app(), Some(&release), true)
            .iter()
            .map(|i| i.key)
            .collect();
        assert_eq!(
            keys,
            [
                "online",
                "saves",
                "sandbox",
                "outside_code",
                "purchases",
                "runtimes",
                "scanned",
                "licence"
            ]
        );
        let all = items(&app(), Some(&release), true);
        assert_eq!(all[0].title, "Works offline");
        assert!(all.iter().all(|i| !i.title.to_lowercase().contains("safe")));
    }

    #[test]
    fn notes_for_what_players_should_know() {
        let mut a = app();
        a.online = vec!["own_servers".into()];
        a.online_hosts = vec!["play.example.com".into()];
        let release = ReleaseFacts {
            repo_scan: json!({ "status": "checked", "save_folder": "elsewhere",
                "code_loading": [{ "label": "Downloads code", "examples": [] }], "monetization": [] }),
            runtimes: vec![],
            has_installer: true,
            scans: vec![Some("passed".into())],
        };
        let all = items(&a, Some(&release), false);
        let get = |k: &str| all.iter().find(|i| i.key == k);
        assert_eq!(get("online").unwrap().tone, "note");
        assert!(
            get("online")
                .unwrap()
                .detail
                .as_deref()
                .unwrap()
                .contains("play.example.com")
        );
        assert_eq!(get("saves").unwrap().tone, "note");
        assert!(get("sandbox").is_none(), "saves elsewhere: not claimed");
        assert_eq!(get("outside_code").unwrap().tone, "note");
        assert!(get("installer").is_some());
        assert_eq!(get("scanned").unwrap().title, "Files not scanned");
    }

    #[test]
    fn accessibility_features_are_checked() {
        assert_eq!(
            validate_accessibility(&["subtitles".into(), "subtitles".into()]).unwrap(),
            ["subtitles"]
        );
        assert!(validate_accessibility(&["telepathy".into()]).is_err());
    }
}
