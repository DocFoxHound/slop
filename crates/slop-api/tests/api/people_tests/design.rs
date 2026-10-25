// SPDX-License-Identifier: AGPL-3.0-or-later
//! A creative's own page design (people/design.rs): read with the portfolio, reviewed with it,
//! shown once approved; and the example templates pass the sanitizer as they are.

use super::part1::{
    Files, call, count, fetch, link_github, notices, portfolio_app, post, put_file, staff,
};
use crate::*;
use slop_api::{
    media::Picture,
    people::{design, portfolio::Work},
};

fn manifest(files: &Files) {
    put_file(
        files,
        "portfolio.json",
        json!({ "avatar": "avatar.png", "works": [{ "image": "knight.png", "title": "Knight" }] })
            .to_string()
            .into_bytes(),
    );
}

fn page(files: &Files, html: &str, css: Option<&str>) {
    put_file(files, "page/index.html", html.as_bytes().to_vec());
    match css {
        Some(css) => put_file(files, "page/style.css", css.as_bytes().to_vec()),
        None => {
            files
                .lock()
                .unwrap()
                .retain(|k, _| !k.ends_with("/.slop/page/style.css"));
        }
    }
}

/// Takes the page's files out (its pictures stay).
fn unpage(files: &Files) {
    files.lock().unwrap().retain(|k, _| {
        !k.ends_with("/.slop/page/index.html") && !k.ends_with("/.slop/page/style.css")
    });
}

const HTML: &str = "<h1>Vera</h1>\n<script>alert(document.cookie)</script>\n<section class=\"hero\"><img src=\"images/hero.png\" alt=\"Vera at work\" onerror=\"alert(1)\"><slop-work ref=\"1\"></slop-work></section> <slop-works></slop-works>";
const CSS: &str = ".hero { background: url(images/tile.png); padding: 2rem }\n.hero h2 { position: fixed; font-family: Fraunces, serif }";

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_page_design_is_reviewed_with_the_portfolio_and_shown_once_approved(db: PgPool) {
    let (app, files, vera, cookie) = portfolio_app(db).await;
    link_github(&app, vera, "42").await;
    let (_, stranger) = app.user("Stranger", "US").await;
    let (_, reviewer) = staff(&app, "Rae Reviewer", "reviewer").await;
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
    let problem = |body: &Value| body["problem"].as_str().unwrap_or_default().to_string();
    manifest(&files);

    // What can't work is a problem, saying which file and line; nothing is queued.
    page(&files, HTML, Some(CSS));
    let (status, body) = check().await;
    assert_eq!(
        (status, body["result"].as_str()),
        (StatusCode::OK, Some("problem")),
        "{body}"
    );
    assert_eq!(
        problem(&body),
        ".slop/page/index.html line 3: .slop/page/images/hero.png isn't in the repository at that commit"
    );
    put_file(&files, "page/images/hero.png", png(640, 360));
    let (_, body) = check().await;
    assert_eq!(
        problem(&body),
        ".slop/page/style.css line 1: .slop/page/images/tile.png isn't in the repository at that commit"
    );
    put_file(&files, "page/images/tile.png", png(8, 8));
    page(&files, &"<p>x</p>".repeat(9000), None);
    let (_, body) = check().await;
    assert!(
        problem(&body).contains(".slop/page/index.html is over 64 KB"),
        "{body}"
    );
    unpage(&files);
    put_file(&files, "page/style.css", b".a{color:red}".to_vec());
    let (_, body) = check().await;
    assert!(
        problem(&body).contains("needs an .slop/page/index.html beside it"),
        "{body}"
    );
    page(&files, "<slop-work ref=\"2\"></slop-work>", None);
    let (_, body) = check().await;
    assert!(problem(&body).contains("from 1 to 1"), "{body}");
    assert_eq!(count(&app, "select count(*) from portfolio_sets").await, 0);

    // A usable one waits for review with the works, cleaned, with what was removed and why.
    page(&files, HTML, Some(CSS));
    let (_, body) = check().await;
    assert_eq!(body["result"], "pending", "{body}");
    let design = &body["pending"]["design"];
    let raw = |path: &str| {
        format!(
            "{}octo/game/{}/.slop/{path}",
            app.config.media.github_raw_base,
            "b".repeat(40)
        )
    };
    assert_eq!(
        design["html"].as_str().unwrap(),
        format!(
            "<h2>Vera</h2> <section class=\"hero\"><img alt=\"Vera at work\" height=\"360\" src=\"{}\" width=\"640\"><slop-work work=\"image:knight.png\"></slop-work></section> <slop-works></slop-works>",
            raw("page/images/hero.png")
        )
    );
    assert_eq!(
        design["css"].as_str().unwrap(),
        format!(
            ".hero{{background:url(\"{}\");padding:2rem;}}.hero h2{{font-family:\"Fraunces\",serif;}}",
            raw("page/images/tile.png")
        )
    );
    let warnings: Vec<&str> = design["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap())
        .collect();
    assert_eq!(
        warnings,
        [
            ".slop/page/index.html line 1: <h1> became <h2>: the page's one <h1> is your name, in Slop's header",
            ".slop/page/index.html line 2: <script> was removed with what's inside: pages can't run scripts",
            ".slop/page/index.html line 3: <img onerror> was removed: pages can't run scripts",
            ".slop/page/style.css line 2: \"position: fixed\" was removed: position can be static, relative or absolute: fixed and sticky could cover Slop's page",
        ]
    );
    assert_eq!(design["has_services"], false);
    assert_eq!(
        design["pictures"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["page/images/hero.png", "page/images/tile.png"]
    );
    let (_, again) = check().await;
    assert_eq!(again["result"], "unchanged");

    // Reviewers see it (the website's queue and the AI reviewer's).
    let (_, queue) = call(&app, fetch("/v1/admin/queue", &reviewer)).await;
    assert_eq!(queue["portfolios"][0]["set"]["design"], *design);
    let (_, review) = call(&app, fetch("/v1/admin/review?kind=portfolio", &reviewer)).await;
    assert_eq!(
        review["items"][0]["detail"]["portfolio"]["design"], *design,
        "{review}"
    );
    // Not shown before it's approved, not even to the creative.
    assert_eq!(
        call(&app, fetch("/v1/people/vera", &stranger)).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, fetch("/v1/people/vera", &cookie)).await.1["design"],
        Value::Null
    );

    // Approved: the page shows the design (the tree and the CSS, not the review details).
    let set = body["pending"]["id"].as_str().unwrap().to_string();
    let res = app
        .send(req(Method::POST, format!("/v1/admin/portfolio/{set}/approve")).cookie(&reviewer))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let (_, shown) = call(&app, fetch("/v1/people/vera", &stranger)).await;
    assert_eq!(
        shown["design"],
        json!({ "tree": design["tree"], "css": design["css"], "has_services": false })
    );
    assert_eq!(
        shown["design"]["tree"][2]["children"][1],
        json!({ "tag": "slop-work", "attrs": { "work": "image:knight.png" } })
    );

    // A changed design is a new set: reviewed again while the approved one keeps showing.
    page(&files, HTML, Some(".hero { padding: 3rem }"));
    let (_, body) = check().await;
    assert_eq!(body["result"], "pending", "{body}");
    let newer = body["pending"]["id"].as_str().unwrap().to_string();
    assert_eq!(
        call(&app, fetch("/v1/people/vera", &stranger)).await.1["design"]["css"],
        design["css"]
    );
    let res = app
        .send(post(
            format!("/v1/admin/portfolio/{newer}/reject"),
            &reviewer,
            json!({ "reason": "The heading's text is too faint to read on the tile." }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        notices(&app, vera).await[0],
        "Your portfolio wasn't approved"
    );
    assert_eq!(
        call(&app, fetch("/v1/people/vera", &stranger)).await.1["design"]["css"],
        design["css"]
    );
    let (_, body) = check().await;
    assert_eq!(
        problem(&body),
        "this is the portfolio that was turned down: The heading's text is too faint to read on the tile."
    );

    // Taking the design out is a change too; once approved, the standard layout is back.
    unpage(&files);
    let (_, body) = check().await;
    assert_eq!(body["result"], "pending", "{body}");
    assert_eq!(body["pending"]["design"], Value::Null);
    let plain = body["pending"]["id"].as_str().unwrap().to_string();
    app.send(req(Method::POST, format!("/v1/admin/portfolio/{plain}/approve")).cookie(&reviewer))
        .await;
    assert_eq!(
        call(&app, fetch("/v1/people/vera", &stranger)).await.1["design"],
        Value::Null
    );
}

/// The example templates (examples/portfolio-templates/) as a creative would copy them: their
/// page files pass the sanitizer without a single warning, and every file they name is there.
#[test]
fn the_example_templates_pass_without_warnings() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/portfolio-templates");
    let mut seen = 0;
    for entry in std::fs::read_dir(&root).expect("examples/portfolio-templates") {
        let dir = entry.unwrap().path().join(".slop");
        if !dir.join("page/index.html").exists() {
            continue;
        }
        seen += 1;
        let picture = |path: &str| Picture {
            path: path.into(),
            url: format!("https://raw.githubusercontent.com/you/portfolio/0/.slop/{path}"),
            sha256: String::new(),
            width: 64,
            height: 64,
            format: "png".into(),
            dhash: None,
        };
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(dir.join("portfolio.json")).unwrap()).unwrap();
        let works: Vec<Work> = manifest["works"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| {
                let s = |k: &str| w[k].as_str().map(str::to_string);
                if let Some(image) = s("image") {
                    Work::Image {
                        picture: picture(&image),
                        title: s("title"),
                        caption: s("caption"),
                        skill: s("skill"),
                    }
                } else if let Some(link) = s("youtube") {
                    Work::Video {
                        youtube_id: slop_api::media::youtube_id(&link).expect("a YouTube link"),
                        poster: s("poster").map(|p| picture(&p)),
                        title: s("title"),
                        caption: s("caption"),
                        skill: s("skill"),
                    }
                } else {
                    let audio = s("audio").expect("image, youtube or audio");
                    Work::Audio {
                        audio: slop_api::people::portfolio::AudioFile {
                            url: String::new(),
                            sha256: String::new(),
                            bytes: 0,
                            format: "ogg".into(),
                            path: audio,
                        },
                        poster: s("poster").map(|p| picture(&p)),
                        title: s("title"),
                        caption: s("caption"),
                        skill: s("skill"),
                    }
                }
            })
            .collect();
        let html = std::fs::read_to_string(dir.join("page/index.html")).unwrap();
        let css = std::fs::read_to_string(dir.join("page/style.css")).ok();
        assert!(html.len() as u64 <= design::MAX_HTML_BYTES);
        assert!(
            css.as_ref()
                .is_none_or(|c| c.len() as u64 <= design::MAX_CSS_BYTES)
        );
        let draft = design::sanitize(&html, css.as_deref(), &works)
            .unwrap_or_else(|p| panic!("{}: {}", dir.display(), p.0));
        assert_eq!(draft.warnings, Vec::<String>::new(), "{}", dir.display());
        assert!(
            draft.has_works || html.contains("<slop-work "),
            "{}: shows no works",
            dir.display()
        );
        for (path, at) in draft.pictures.iter().chain(&draft.tracks) {
            assert!(
                dir.join(path).exists(),
                "{}: {at} names {path}, which isn't there",
                dir.display()
            );
        }
    }
    assert!(seen >= 4, "the four templates have page designs");
}
