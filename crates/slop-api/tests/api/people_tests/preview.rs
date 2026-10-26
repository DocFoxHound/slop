// SPDX-License-Identifier: AGPL-3.0-or-later
//! A creative's own page previews their portfolio while it waits for review (people/mod.rs
//! `PreviewSet`): only they get it; everyone else gets the approved one, and only that.

use super::part1::{Files, call, fetch, link_github, portfolio_app, post, put_file, staff};
use crate::*;

fn manifest(files: &Files, works: &[&str]) {
    let works: Vec<Value> = works
        .iter()
        .map(|w| json!({ "image": format!("{}.png", w.to_lowercase()), "title": w }))
        .collect();
    put_file(
        files,
        "portfolio.json",
        json!({ "avatar": "avatar.png", "works": works })
            .to_string()
            .into_bytes(),
    );
}

fn design(files: &Files, heading: &str) {
    put_file(
        files,
        "page/index.html",
        format!("<h2>{heading}</h2><slop-works></slop-works>").into_bytes(),
    );
}

/// The page as someone sees it: the creative (`Some(their cookie)`), someone else, or nobody
/// signed in (`None`).
async fn page(app: &TestApp, who: Option<&str>) -> (StatusCode, Value) {
    let r = match who {
        Some(cookie) => fetch("/v1/people/vera", cookie),
        None => req(Method::GET, "/v1/people/vera"),
    };
    call(app, r).await
}

fn titles(works: &Value) -> Vec<&str> {
    works
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["title"].as_str().unwrap())
        .collect()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn only_the_creative_previews_a_portfolio_waiting_for_review(db: PgPool) {
    let (app, files, vera, cookie) = portfolio_app(db).await;
    link_github(&app, vera, "42").await;
    let (_, stranger) = app.user("Stranger", "US").await;
    let (_, reviewer) = staff(&app, "Rae Reviewer", "reviewer").await;
    put_file(&files, "dragon.png", png(64, 64));
    let check = || {
        call(
            &app,
            post(
                "/v1/me/creative/portfolio/check",
                &cookie,
                json!({ "repo_url": "https://github.com/octo/game" }),
            ),
        )
    };
    let decide = |set: String, verdict: &'static str| {
        let reviewer = reviewer.clone();
        let app = &app;
        async move {
            let body = match verdict {
                "approve" => json!({}),
                _ => json!({ "reason": "The dragon is someone else's art." }),
            };
            let res = app
                .send(post(
                    format!("/v1/admin/portfolio/{set}/{verdict}"),
                    &reviewer,
                    body,
                ))
                .await;
            assert_eq!(res.status(), StatusCode::NO_CONTENT);
        }
    };

    // The first portfolio: the creative previews it (works, avatar, design); the page isn't
    // live, so nobody else sees anything.
    manifest(&files, &["Knight"]);
    design(&files, "First look");
    let (_, body) = check().await;
    assert_eq!(body["result"], "pending", "{body}");
    let first = body["pending"]["id"].as_str().unwrap().to_string();
    let (status, mine) = page(&app, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mine["mine"]["state"], "waiting");
    assert_eq!(mine["works"], json!([]));
    assert_eq!(mine["design"], Value::Null);
    let preview = &mine["preview_set"];
    assert_eq!(preview["status"], "pending");
    assert_eq!(preview["status_reason"], Value::Null);
    assert_eq!(titles(&preview["works"]), ["Knight"]);
    assert!(
        preview["avatar_url"]
            .as_str()
            .unwrap()
            .ends_with("/.slop/avatar.png"),
        "{preview}"
    );
    assert_eq!(preview["design"]["tree"][0]["tag"], "h2");
    assert_eq!(
        preview["design"]["tree"][1]["tag"], "slop-works",
        "the same sanitized tree the live page draws: {preview}"
    );
    assert_eq!(preview["created_at"], body["pending"]["created_at"]);
    for who in [Some(stranger.as_str()), None] {
        assert_eq!(page(&app, who).await.0, StatusCode::NOT_FOUND);
    }

    // Approved: it's the live page, so there's nothing newer to preview.
    decide(first, "approve").await;
    let (_, mine) = page(&app, Some(&cookie)).await;
    assert_eq!(mine["preview_set"], Value::Null);
    assert_eq!(titles(&mine["works"]), ["Knight"]);

    // A newer one waits: the creative gets it beside the live page, which is unchanged; nobody
    // else gets it, or any of it.
    manifest(&files, &["Knight", "Dragon"]);
    design(&files, "Second look");
    let (_, body) = check().await;
    assert_eq!(body["result"], "pending", "{body}");
    let second = body["pending"]["id"].as_str().unwrap().to_string();
    let (_, mine) = page(&app, Some(&cookie)).await;
    assert_eq!(mine["mine"]["state"], "live");
    assert_eq!(mine["mine"]["update_waiting"], true);
    assert_eq!(titles(&mine["works"]), ["Knight"]);
    assert_eq!(mine["design"]["tree"][0]["children"][0], "First look");
    let preview = &mine["preview_set"];
    assert_eq!(preview["status"], "pending");
    assert_eq!(titles(&preview["works"]), ["Knight", "Dragon"]);
    assert_eq!(preview["design"]["tree"][0]["children"][0], "Second look");
    for who in [Some(stranger.as_str()), None] {
        let (status, theirs) = page(&app, who).await;
        assert_eq!(status, StatusCode::OK);
        assert!(theirs.get("preview_set").unwrap().is_null(), "{theirs}");
        assert_eq!(theirs["mine"], Value::Null);
        assert_eq!(titles(&theirs["works"]), ["Knight"]);
        assert_eq!(theirs["design"]["tree"][0]["children"][0], "First look");
        let text = theirs.to_string();
        assert!(
            !text.contains("Second look") && !text.contains("Dragon"),
            "{text}"
        );
    }

    // Its order follows the creative's, like the live works.
    let res = app
        .send(
            req(Method::PUT, "/v1/me/creative/portfolio/order")
                .cookie(&cookie)
                .json(json!({ "keys": ["image:dragon.png", "image:knight.png"] })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let (_, mine) = page(&app, Some(&cookie)).await;
    assert_eq!(titles(&mine["preview_set"]["works"]), ["Dragon", "Knight"]);

    // Turned down: still previewed, with the reviewer's reason, until the next check; still
    // nobody else's.
    decide(second, "reject").await;
    let (_, mine) = page(&app, Some(&cookie)).await;
    let preview = &mine["preview_set"];
    assert_eq!(preview["status"], "rejected");
    assert_eq!(
        preview["status_reason"],
        "The dragon is someone else's art."
    );
    assert!(preview["reviewed_at"].is_string(), "{preview}");
    assert_eq!(preview["design"]["tree"][0]["children"][0], "Second look");
    assert_eq!(mine["design"]["tree"][0]["children"][0], "First look");
    let (_, theirs) = page(&app, Some(&stranger)).await;
    assert!(theirs["preview_set"].is_null());
    assert!(!theirs.to_string().contains("Second look"));

    // Back to the approved one: a turned-down portfolio older than it isn't previewed.
    manifest(&files, &["Knight"]);
    design(&files, "First look");
    let (_, body) = check().await;
    assert_eq!(body["result"], "unchanged", "{body}");
    let (_, mine) = page(&app, Some(&cookie)).await;
    assert_eq!(mine["preview_set"]["status"], "rejected");
    manifest(&files, &["Knight"]);
    design(&files, "Third look");
    let (_, body) = check().await;
    let third = body["pending"]["id"].as_str().unwrap().to_string();
    decide(third, "approve").await;
    let (_, mine) = page(&app, Some(&cookie)).await;
    assert_eq!(mine["preview_set"], Value::Null, "{mine}");
    assert_eq!(mine["design"]["tree"][0]["children"][0], "Third look");
}
