// SPDX-License-Identifier: AGPL-3.0-or-later
//! Checks anyone can run before they have an account (checks.rs): store addresses, a repository's
//! readiness, and a portfolio dry run. They answer plain JSON with a summary, never write
//! anything, ask GitHub as nobody, and are limited per address and for every address together.

use crate::*;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

// ---------------------------------------------------------------------------------------------
// Helpers

/// An API answer: status, extra headers, body.
type Answer = (u16, Vec<(String, String)>, String);

/// A GitHub for these tests: API answers each test sets by path (404 otherwise), raw files under
/// /raw/, and every request it got with its Authorization header.
#[derive(Clone, Default)]
struct Hub {
    api: Arc<Mutex<HashMap<String, Answer>>>,
    raw: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    seen: Arc<Mutex<Vec<(String, String)>>>,
}

impl Hub {
    fn api(&self, path: &str, status: u16, body: Value) {
        self.api_with(path, status, &[], body);
    }

    fn api_with(&self, path: &str, status: u16, headers: &[(&str, &str)], body: Value) {
        self.api.lock().unwrap().insert(
            path.to_string(),
            (
                status,
                headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                body.to_string(),
            ),
        );
    }

    /// A file in `repo` at `commit`.
    fn file(&self, repo: &str, commit: &str, path: &str, bytes: impl Into<Vec<u8>>) {
        self.raw
            .lock()
            .unwrap()
            .insert(format!("{repo}/{commit}/{path}"), bytes.into());
    }

    fn hits(&self, path: &str) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p == path)
            .count()
    }

    /// The newest commit on `repo`'s default branch.
    fn head(&self, repo: &str, commit: &str) {
        self.api(
            &format!("/repos/{repo}/commits/HEAD"),
            200,
            json!({ "sha": commit }),
        );
    }
}

async fn hub() -> (String, Hub) {
    let hub = Hub::default();
    let state = hub.clone();
    let app = Router::new().fallback(
        move |uri: axum::http::Uri, headers: axum::http::HeaderMap| {
            let hub = state.clone();
            async move {
                let auth = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                let path = uri.path().to_string();
                hub.seen.lock().unwrap().push((path.clone(), auth));
                if let Some(file) = path.strip_prefix("/raw/") {
                    return match hub.raw.lock().unwrap().get(file) {
                        Some(bytes) => (StatusCode::OK, bytes.clone()).into_response(),
                        None => StatusCode::NOT_FOUND.into_response(),
                    };
                }
                let answer = hub.api.lock().unwrap().get(&path).cloned();
                match answer {
                    Some((status, extra, body)) => {
                        let mut res = Response::new(Body::from(body));
                        *res.status_mut() = StatusCode::from_u16(status).unwrap();
                        res.headers_mut()
                            .insert("content-type", "application/json".parse().unwrap());
                        for (k, v) in extra {
                            res.headers_mut().insert(
                                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                                v.parse().unwrap(),
                            );
                        }
                        res
                    }
                    None => (
                        StatusCode::NOT_FOUND,
                        axum::Json(json!({ "message": "Not Found" })),
                    )
                        .into_response(),
                }
            }
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/"), hub)
}

/// A server whose GitHub is `base`, with SLAPorSLOP's own GitHub token set (the checks must
/// never send it).
async fn checks_app(db: PgPool, base: &str, adjust: impl FnOnce(&mut Config)) -> TestApp {
    let base = base.to_string();
    test_app_with(db, move |c| {
        c.releases.github_api_base = base.parse().unwrap();
        c.releases.github_token = Some("gh-server-token".into());
        c.media.github_raw_base = format!("{base}raw/").parse().unwrap();
        c.rate_limits.check_per_hour = 1_000;
        adjust(c);
    })
    .await
}

async fn get_json(app: &TestApp, uri: &str) -> (StatusCode, Value) {
    let res = app.send(req(Method::GET, uri)).await;
    (res.status(), body_json(res).await)
}

async fn rows(app: &TestApp) -> Vec<i64> {
    let mut out = Vec::new();
    for table in [
        "apps",
        "media_sets",
        "portfolio_sets",
        "creatives",
        "audit_log",
        "notifications",
    ] {
        out.push(
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("select count(*) from {table}")))
                .fetch_one(&app.db)
                .await
                .unwrap(),
        );
    }
    out
}

fn kinds(report: &Value) -> Vec<String> {
    report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["kind"].as_str().unwrap().to_string())
        .collect()
}

fn finding<'a>(report: &'a Value, kind: &str) -> &'a Value {
    report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["kind"] == kind)
        .unwrap_or_else(|| panic!("no {kind} in {report}"))
}

const WORKFLOW: &str = "name: Slop Build\non: push\njobs:\n  slop:\n    uses: DocFoxHound/slop-build/.github/workflows/build.yml@v1\n    with:\n      app: good-game   # your store address\n      slop-url: http://localhost:8080\n      project-type: rust\n";

/// octo/good at `commit`: MIT, an icon, a thumbnail, two screenshots and a video, Slop Build.
fn good_repository(hub: &Hub, commit: &str) {
    hub.head("octo/good", commit);
    hub.api(
        "/repos/octo/good/license",
        200,
        json!({ "license": { "spdx_id": "MIT", "name": "MIT License" } }),
    );
    let file = |path: &str, bytes: Vec<u8>| hub.file("octo/good", commit, path, bytes);
    file(".slop/icon.png", png(512, 512));
    file(".slop/thumb.png", png(920, 430));
    file(".slop/shots/a.png", png(640, 360));
    file(".slop/shots/b.png", png(1280, 720));
    file(
        ".slop/media.json",
        json!({
            "icon": "icon.png",
            "thumbnail": "thumb.png",
            "gallery": [
                { "image": "shots/a.png", "caption": "Level one" },
                { "youtube": "https://youtu.be/dQw4w9WgXcQ" },
                { "image": "shots/b.png" }
            ]
        })
        .to_string()
        .into_bytes(),
    );
    file(".github/workflows/slop.yml", WORKFLOW.as_bytes().to_vec());
}

// ---------------------------------------------------------------------------------------------
// Store addresses

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn store_addresses_are_checked_without_an_account(db: PgPool) {
    let app = test_app(db).await;
    let (maker, _) = app.user("Maker", "US").await;
    let before = rows(&app).await;

    // Free.
    let (status, body) = get_json(&app, "/v1/check/store-address?slug=angry-herds").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["valid"], true);
    assert_eq!(body["available"], true);
    assert_eq!(body["suggestion"], Value::Null);
    assert!(
        body["summary"]
            .as_str()
            .unwrap()
            .starts_with("angry-herds is a valid store address and it's free"),
        "{body}"
    );

    // Taken, by a draft or a listed app alike: the next free one.
    listed_with(&app, maker, "angry-herds", "released", "").await;
    listed_with(&app, maker, "angry-herds-2", "released", "status = 'draft'").await;
    let (_, body) = get_json(&app, "/v1/check/store-address?slug=angry-herds").await;
    assert_eq!(
        (&body["valid"], &body["available"], &body["suggestion"]),
        (&json!(true), &json!(false), &json!("angry-herds-3"))
    );
    assert_eq!(
        body["summary"],
        "angry-herds is taken by another app; angry-herds-3 is free. Use that one (the app's name can stay the same)."
    );

    // Not valid: what's wrong, and a valid one made from it that's free.
    let (status, body) = get_json(&app, "/v1/check/store-address?slug=Angry_Herds").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["valid"], false);
    assert_eq!(body["suggestion"], "angry-herds-3");
    assert!(
        body["summary"]
            .as_str()
            .unwrap()
            .contains("isn't a valid store address: use 3 to 60 lowercase letters"),
        "{body}"
    );
    let (_, body) = get_json(&app, "/v1/check/store-address?slug=My%20Game!").await;
    assert_eq!(body["suggestion"], "my-game");
    let (_, body) = get_json(&app, "/v1/check/store-address?slug=a").await;
    assert_eq!(
        (&body["valid"], &body["suggestion"]),
        (&json!(false), &Value::Null)
    );

    // Nothing to check: says how to ask.
    let (status, body) = get_json(&app, "/v1/check/store-address").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["summary"],
        "Give the store address to check as ?slug=my-game."
    );

    // Asking wrote nothing (the two apps are the test's own).
    let mut after = rows(&app).await;
    after[0] -= 2;
    assert_eq!(after, before);
}

// ---------------------------------------------------------------------------------------------
// Repositories

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_ready_repository_passes_and_nothing_is_saved(db: PgPool) {
    let (base, hub) = hub().await;
    let app = checks_app(db, &base, |_| {}).await;
    let (maker, _) = app.user("Maker", "US").await;
    // Another app has pictures already, so a stored set would show in the counts.
    listed_with(&app, maker, "other", "released", "").await;
    let commit = "c".repeat(40);
    good_repository(&hub, &commit);
    let before = rows(&app).await;

    let (status, report) = get_json(
        &app,
        "/v1/check/repository?url=https://github.com/octo/good",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["status"], "looks_good", "{report}");
    assert_eq!(report["repository"], "octo/good");
    assert_eq!(report["commit"], commit);
    assert_eq!(report["findings"], json!([]));
    assert_eq!(
        report["summary"],
        "octo/good at commit ccccccc passed what this checks (licence, pictures and builds). SLAPorSLOP's source check and a person's review still run on the first release."
    );
    assert_eq!(
        report["licence"],
        json!({ "spdx": "MIT", "name": "MIT License", "osi_approved": true })
    );
    assert_eq!(
        report["pictures"],
        json!({ "manifest": ".slop/media.json", "found": true, "icon": true, "thumbnail": true, "gallery_items": 3, "problems": 0 })
    );
    assert_eq!(report["builds"]["slop_build_workflow"], true);
    assert_eq!(report["builds"]["workflow_app"], "good-game");
    assert_eq!(report["builds"]["workflow_app_note"], "good-game is free");
    let passed: Vec<&str> = report["passed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect();
    assert_eq!(
        passed,
        [
            "Licence: MIT (MIT License), OSI-approved.",
            "Pictures: .slop/media.json read without problems (an icon, a thumbnail, 3 gallery items).",
            "Builds: .github/workflows/slop.yml is there (Slop Build builds, signs and releases on each push to the main branch or a v* tag); its app is good-game (good-game is free).",
        ]
    );
    assert!(!report["not_checked"].as_array().unwrap().is_empty());
    assert!(
        report["dry_run"]
            .as_str()
            .unwrap()
            .contains("nothing was saved")
    );
    // Slop Build makes the releases: they aren't looked up.
    assert_eq!(hub.hits("/repos/octo/good/releases/latest"), 0);

    // The same commit again: the kept report, without downloading anything; GitHub is only
    // asked for the newest commit.
    let (_, again) = get_json(&app, "/v1/check/repository?url=github.com/octo/good/").await;
    assert_eq!(again, report);
    assert_eq!(hub.hits("/repos/octo/good/commits/HEAD"), 2);
    assert_eq!(hub.hits("/repos/octo/good/license"), 1);
    assert_eq!(
        hub.hits(&format!("/raw/octo/good/{commit}/.slop/media.json")),
        1
    );

    // A new commit is checked again; the licence comes from the day's cache.
    let next = "d".repeat(40);
    good_repository(&hub, &next);
    let (_, report) = get_json(
        &app,
        "/v1/check/repository?url=https://github.com/octo/good",
    )
    .await;
    assert_eq!(report["commit"], next);
    assert_eq!(hub.hits("/repos/octo/good/license"), 1);
    assert_eq!(
        hub.hits(&format!("/raw/octo/good/{next}/.slop/media.json")),
        1
    );

    // GitHub was asked as nobody, never with SLAPorSLOP's token.
    let seen = hub.seen.lock().unwrap().clone();
    assert!(!seen.is_empty());
    assert!(seen.iter().all(|(_, auth)| auth.is_empty()), "{seen:?}");

    // Nothing was stored, queued or recorded.
    assert_eq!(rows(&app).await, before);
    let checked: Option<OffsetDateTime> =
        sqlx::query_scalar("select media_checked_at from apps where slug = 'other'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(checked, None);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn every_problem_in_a_repository_comes_with_its_fix(db: PgPool) {
    let (base, hub) = hub().await;
    let app = checks_app(db, &base, |_| {}).await;
    let before = rows(&app).await;

    // No licence; four picture problems; a release with a Linux archive but no package.
    let commit = "e".repeat(40);
    hub.head("octo/broken", &commit);
    let file = |path: &str, bytes: Vec<u8>| hub.file("octo/broken", &commit, path, bytes);
    file(".slop/icon.png", png(512, 300));
    file(".slop/thumb.png", png(400, 200));
    file(".slop/shots/a.png", png(640, 360));
    file(
        ".slop/media.json",
        json!({
            "icon": "icon.png",
            "thumbnail": "thumb.png",
            "gallery": [
                { "image": "shots/a.png" },
                { "youtube": "https://vimeo.com/123" },
                { "image": "shots/missing.png" }
            ]
        })
        .to_string()
        .into_bytes(),
    );
    hub.api(
        "/repos/octo/broken/releases/latest",
        200,
        json!({ "tag_name": "v0.1.0", "html_url": "https://github.com/octo/broken/releases/tag/v0.1.0",
                "assets": [{ "name": "broken-0.1.0-linux.tar.gz" }, { "name": "notes.txt" }] }),
    );
    let (status, report) = get_json(
        &app,
        "/v1/check/repository?url=https://github.com/octo/broken",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["status"], "must_fix", "{report}");
    assert_eq!(
        kinds(&report),
        [
            "licence:no_file",
            "icon:problem",
            "pictures:problem",
            "pictures:problem",
            "pictures:problem",
            "builds:linux_package",
        ]
    );
    assert_eq!(
        report["summary"],
        "octo/broken at commit eeeeeee: 1 thing must be fixed before SLAPorSLOP can publish it, and 5 things should be. Each finding says what's wrong and how to fix it."
    );
    let problems: Vec<&str> = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["problem"].as_str().unwrap())
        .collect();
    assert_eq!(
        problems[1..5],
        [
            "The icon: icon.png is 512×300; the icon has to be square, e.g. 512×512",
            "thumb.png is 400×200; it needs to be at least 460×215",
            "gallery item 2: \"https://vimeo.com/123\" isn't a YouTube video link",
            ".slop/shots/missing.png isn't in the repository at that commit",
        ]
    );
    let licence = finding(&report, "licence:no_file");
    assert_eq!(licence["severity"], "must");
    assert!(
        licence["instruction"]
            .as_str()
            .unwrap()
            .contains("don't choose one for them"),
        "{licence}"
    );
    assert_eq!(
        licence["guide_section"],
        "What SLAPorSLOP asks of your project"
    );
    let picture = finding(&report, "pictures:problem");
    assert_eq!(
        (&picture["severity"], &picture["guide_section"]),
        (&json!("should"), &json!("Pictures and videos"))
    );
    assert_eq!(
        report["builds"]["latest_release"]["platforms"],
        json!(["linux"])
    );
    assert_eq!(report["pictures"]["problems"], 4);

    // A licence GitHub can't name, and one that isn't OSI-approved.
    for (repo, spdx, kind, severity) in [
        ("octo/odd", "NOASSERTION", "licence:unidentified", "should"),
        (
            "octo/source-available",
            "BUSL-1.1",
            "licence:not_osi",
            "must",
        ),
    ] {
        hub.head(repo, &commit);
        hub.api(
            &format!("/repos/{repo}/license"),
            200,
            json!({ "license": { "spdx_id": spdx, "name": "Some licence" } }),
        );
        let (_, report) = get_json(
            &app,
            &format!("/v1/check/repository?url=https://github.com/{repo}"),
        )
        .await;
        let found = finding(&report, kind);
        assert_eq!(found["severity"], severity, "{report}");
        // No media.json and no release either: what to add, and that builds are missing.
        assert_eq!(finding(&report, "pictures:none")["severity"], "recommended");
        assert_eq!(finding(&report, "builds:none")["severity"], "should");
    }
    let (_, report) = get_json(
        &app,
        "/v1/check/repository?url=https://github.com/octo/source-available",
    )
    .await;
    assert!(
        finding(&report, "licence:not_osi")["problem"]
            .as_str()
            .unwrap()
            .contains("BUSL-1.1 isn't on SLAPorSLOP's list of OSI-approved licences"),
        "{report}"
    );

    // A media.json without an icon or thumbnail: both recommended, nothing blocks.
    let plain = "f".repeat(40);
    hub.head("octo/plain", &plain);
    hub.api(
        "/repos/octo/plain/license",
        200,
        json!({ "license": { "spdx_id": "GPL-3.0-or-later", "name": "GNU GPL v3" } }),
    );
    hub.file("octo/plain", &plain, ".slop/shot.png", png(640, 360));
    hub.file(
        "octo/plain",
        &plain,
        ".slop/media.json",
        br#"{ "gallery": [{ "image": "shot.png" }] }"#.to_vec(),
    );
    // Slop Build, but still sending releases to the guide's example site, with no app.
    hub.file(
        "octo/plain",
        &plain,
        ".github/workflows/slop.yml",
        WORKFLOW
            .replace("http://localhost:8080", "https://slop.example")
            .replace("app: good-game", "app: Bad_Name")
            .into_bytes(),
    );
    let (_, report) = get_json(
        &app,
        "/v1/check/repository?url=https://github.com/octo/plain",
    )
    .await;
    assert_eq!(report["status"], "should_fix", "{report}");
    assert_eq!(
        kinds(&report),
        [
            "builds:slop_url",
            "builds:app",
            "icon:missing",
            "pictures:no_thumbnail"
        ]
    );
    assert_eq!(
        finding(&report, "builds:slop_url")["instruction"],
        "Set `slop-url: http://localhost:8080` under `with:` in .github/workflows/slop.yml."
    );
    assert!(
        finding(&report, "builds:app")["instruction"]
            .as_str()
            .unwrap()
            .contains("(e.g. bad-name)")
    );

    assert_eq!(rows(&app).await, before);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn repositories_that_cant_be_checked_say_why(db: PgPool) {
    let (base, hub) = hub().await;
    let app = checks_app(db, &base, |_| {}).await;
    let summary = |body: &Value| body["summary"].as_str().unwrap().to_string();

    // Not GitHub: what SLAPorSLOP does with it instead.
    let (status, body) =
        get_json(&app, "/v1/check/repository?url=https://gitlab.com/ann/game").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "not_checked");
    assert!(
        summary(&body).starts_with(
            "This check reads GitHub repositories only so far, and gitlab.com isn't GitHub."
        ),
        "{body}"
    );
    let (_, body) = get_json(&app, "/v1/check/repository").await;
    assert_eq!(
        summary(&body),
        "Give the repository's address as ?url=https://github.com/OWNER/REPO."
    );
    let (_, body) = get_json(&app, "/v1/check/repository?url=https://github.com/octo").await;
    assert_eq!(body["status"], "not_checked");

    // Missing or private look the same: GitHub answers nobody 404 for both.
    let (status, body) = get_json(
        &app,
        "/v1/check/repository?url=https://github.com/octo/secret",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        summary(&body).starts_with(
            "GitHub can't find a public repository at https://github.com/octo/secret."
        ),
        "{body}"
    );
    hub.api(
        "/repos/octo/empty/commits/HEAD",
        409,
        json!({ "message": "Git Repository is empty." }),
    );
    let (_, body) = get_json(
        &app,
        "/v1/check/repository?url=https://github.com/octo/empty",
    )
    .await;
    assert!(summary(&body).contains("has no commits yet"), "{body}");

    // GitHub says SLAPorSLOP's allowance is used up: a 503 saying when, and GitHub isn't asked
    // again until then.
    let reset = OffsetDateTime::now_utc().unix_timestamp() + 1200;
    hub.api_with(
        "/repos/octo/busy/commits/HEAD",
        403,
        &[
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", &reset.to_string()),
        ],
        json!({ "message": "API rate limit exceeded" }),
    );
    let res = app
        .send(req(
            Method::GET,
            "/v1/check/repository?url=https://github.com/octo/busy",
        ))
        .await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    let wait: u64 = res.headers()["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1100..=1200).contains(&wait), "{wait}");
    let body = body_json(res).await;
    assert!(
        summary(&body).starts_with("SLAPorSLOP has asked GitHub all it can for checks without an account this hour: try again in 20 minutes."),
        "{body}"
    );
    good_repository(&hub, &"c".repeat(40));
    let res = app
        .send(req(
            Method::GET,
            "/v1/check/repository?url=https://github.com/octo/good",
        ))
        .await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(hub.hits("/repos/octo/good/commits/HEAD"), 0);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn checks_are_limited_per_address_and_for_everyone(db: PgPool) {
    let (base, hub) = hub().await;
    let app = checks_app(db, &base, |c| {
        c.rate_limits.check_per_hour = 2;
        c.rate_limits.check_github_per_hour = 3;
    })
    .await;
    good_repository(&hub, &"c".repeat(40));
    let check = |ip: &str| {
        req(
            Method::GET,
            "/v1/check/repository?url=https://github.com/octo/good",
        )
        .header("x-test-client-ip", ip)
    };

    // Two an hour from one address; the third says when to come back.
    for _ in 0..2 {
        assert_eq!(app.send(check("10.0.0.1")).await.status(), StatusCode::OK);
    }
    let res = app.send(check("10.0.0.1")).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    let wait: u64 = res.headers()["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1790..=1800).contains(&wait), "{wait}");
    let body = body_json(res).await;
    assert_eq!(body["error"]["code"], "rate_limited");
    assert!(
        body["summary"].as_str().unwrap().starts_with(
            "Too many repository and portfolio checks from this address: try again in 30 minutes."
        ),
        "{body}"
    );
    // Portfolio checks count with them; store addresses don't (they only ask the database).
    let res = app
        .send(
            req(
                Method::GET,
                "/v1/check/portfolio?url=https://github.com/octo/good",
            )
            .header("x-test-client-ip", "10.0.0.1"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    let res = app
        .send(
            req(Method::GET, "/v1/check/store-address?slug=good-game")
                .header("x-test-client-ip", "10.0.0.1"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);

    // Every address together: the first check asked GitHub twice (the newest commit, the
    // licence), the second only for the newest commit (the report was kept). The share of 3 is
    // used up, so another address waits too, without GitHub being asked.
    assert_eq!(hub.hits("/repos/octo/good/commits/HEAD"), 2);
    let res = app.send(check("10.0.0.2")).await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(res.headers().contains_key("retry-after"));
    assert_eq!(hub.hits("/repos/octo/good/commits/HEAD"), 2);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn checks_can_be_turned_off(db: PgPool) {
    let (base, hub) = hub().await;
    let app = checks_app(db, &base, |c| c.rate_limits.check_github_per_hour = 0).await;
    good_repository(&hub, &"c".repeat(40));
    let (status, body) = get_json(
        &app,
        "/v1/check/repository?url=https://github.com/octo/good",
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        body["summary"]
            .as_str()
            .unwrap()
            .starts_with("This SLAPorSLOP server doesn't run repository or portfolio checks"),
        "{body}"
    );
    assert!(hub.seen.lock().unwrap().is_empty());
}

// ---------------------------------------------------------------------------------------------
// Portfolios

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_portfolio_dry_run_lists_every_problem_and_saves_nothing(db: PgPool) {
    let (base, hub) = hub().await;
    let app = checks_app(db, &base, |_| {}).await;
    let before = rows(&app).await;
    let commit = "a".repeat(40);
    hub.head("vera/portfolio", &commit);
    let check = || {
        get_json(
            &app,
            "/v1/check/portfolio?url=https://github.com/vera/portfolio",
        )
    };

    // No portfolio.json yet: how to start one.
    let (status, report) = check().await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["status"], "must_fix", "{report}");
    assert!(
        report["problems"][0]
            .as_str()
            .unwrap()
            .starts_with("There's no .slop/portfolio.json yet"),
        "{report}"
    );

    // Every problem at once, each naming the work and field (a new commit: nothing kept).
    let commit = "b".repeat(40);
    hub.head("vera/portfolio", &commit);
    let file_b = |path: &str, bytes: Vec<u8>| hub.file("vera/portfolio", &commit, path, bytes);
    file_b(".slop/avatar.png", png(64, 64));
    file_b(".slop/knight.png", png(64, 64));
    file_b(".slop/tiny.png", png(16, 16));
    file_b(
        ".slop/portfolio.json",
        json!({
            "avatar": "avatar.png",
            "works": [
                { "image": "tiny.png", "title": "Tiny" },
                { "image": "knight.png", "skill": "juggling" },
                { "image": "missing.png" },
                { "image": "knight.png", "title": "Knight" }
            ]
        })
        .to_string()
        .into_bytes(),
    );
    let (_, report) = check().await;
    assert_eq!(report["status"], "must_fix");
    let problems: Vec<&str> = report["problems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect();
    assert_eq!(problems.len(), 4, "{report}");
    assert_eq!(
        problems[0],
        "\"avatar\": avatar.png is 64×64; it needs to be at least 128×128"
    );
    assert_eq!(
        problems[1],
        "work 1 \"image\": tiny.png is 16×16; it needs to be at least 32×32"
    );
    assert!(problems[2].starts_with("work 2 \"skill\": \"juggling\" isn't one of the skills"));
    assert_eq!(
        problems[3],
        "work 3 \"image\": .slop/missing.png isn't in the repository at that commit"
    );
    assert_eq!(
        report["summary"],
        "vera/portfolio at commit bbbbbbb: 4 things to fix before a People page can use this portfolio. Each problem names the work or file and what to change."
    );

    // Fixed, with a page design: it reads cleanly, and says what the design's checks dropped.
    let commit = "c".repeat(40);
    hub.head("vera/portfolio", &commit);
    let file_c = |path: &str, bytes: Vec<u8>| hub.file("vera/portfolio", &commit, path, bytes);
    file_c(".slop/avatar.png", png(256, 256));
    file_c(".slop/knight.png", png(64, 64));
    file_c(
        ".slop/portfolio.json",
        json!({ "avatar": "avatar.png", "works": [{ "image": "knight.png", "title": "Knight", "skill": "pixel-art" }] })
            .to_string()
            .into_bytes(),
    );
    file_c(
        ".slop/page/index.html",
        b"<h1>Vera</h1>\n<script>alert(1)</script>\n<slop-works></slop-works>\n".to_vec(),
    );
    let (_, report) = check().await;
    assert_eq!(report["status"], "looks_good", "{report}");
    assert_eq!(report["problems"], json!([]));
    assert_eq!(
        report["found"],
        json!({ "manifest": ".slop/portfolio.json", "found": true, "avatar": true, "banner": false, "works": 1, "page_design": true })
    );
    assert_eq!(
        report["design_warnings"].as_array().unwrap().len(),
        2,
        "{report}"
    );
    assert!(
        report["summary"]
            .as_str()
            .unwrap()
            .contains("the page design's checks dropped 2 things"),
        "{report}"
    );

    // Only GitHub.
    let (_, report) = get_json(
        &app,
        "/v1/check/portfolio?url=https://codeberg.org/vera/portfolio",
    )
    .await;
    assert_eq!(report["status"], "not_checked");
    assert!(
        report["summary"]
            .as_str()
            .unwrap()
            .starts_with("Portfolios come from public GitHub repositories only"),
        "{report}"
    );

    // Nothing was stored or queued for review, and nobody was asked for with credentials.
    assert_eq!(rows(&app).await, before);
    assert!(
        hub.seen
            .lock()
            .unwrap()
            .iter()
            .all(|(_, auth)| auth.is_empty())
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_report_says_what_it_couldnt_check(db: PgPool) {
    let (base, hub) = hub().await;
    // Enough of GitHub's share for the newest commit, not for the licence or the releases.
    let app = checks_app(db, &base, |c| c.rate_limits.check_github_per_hour = 1).await;
    let commit = "c".repeat(40);
    hub.head("octo/half", &commit);
    hub.file("octo/half", &commit, ".slop/icon.png", png(512, 512));
    hub.file("octo/half", &commit, ".slop/thumb.png", png(920, 430));
    hub.file(
        "octo/half",
        &commit,
        ".slop/media.json",
        br#"{ "icon": "icon.png", "thumbnail": "thumb.png" }"#.to_vec(),
    );
    let (status, report) = get_json(
        &app,
        "/v1/check/repository?url=https://github.com/octo/half",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["status"], "incomplete", "{report}");
    assert_eq!(report["findings"], json!([]));
    assert_eq!(
        report["summary"],
        "octo/half at commit ccccccc: nothing found needs fixing, but not everything could be checked this time (see not_checked). Check again later."
    );
    let not_checked = report["not_checked"].as_array().unwrap();
    assert!(not_checked[0].as_str().unwrap().starts_with("The licence:"));
    assert!(not_checked[1].as_str().unwrap().starts_with("Builds:"));
    assert_eq!(hub.hits("/repos/octo/half/license"), 0);
    assert_eq!(hub.hits("/repos/octo/half/releases/latest"), 0);
}
