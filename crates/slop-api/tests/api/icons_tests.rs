// SPDX-License-Identifier: AGPL-3.0-or-later
//! App icons (media.rs, `"icon"` in .slop/media.json): checked like the other pictures, stored in
//! the set and reviewed with it, `icon_url` only once approved, look-alikes of other makers'
//! icons pointed out, and "no icon yet" as a recommendation that never blocks.
//!
//! Each test reads its own commit (the fake GitHub's raw files are shared by every test).

use crate::*;

/// A square icon: a disc on a gradient, `tint` shifting its colours a little.
fn icon(size: u32, tint: u8) -> Vec<u8> {
    let img = image::RgbImage::from_fn(size, size, |x, y| {
        let (fx, fy) = (x as f32 / size as f32, y as f32 / size as f32);
        if (fx - 0.35).powi(2) + (fy - 0.5).powi(2) < 0.08 {
            image::Rgb([220u8.saturating_add(tint), 40, 40])
        } else {
            let g = (fx * 200.0) as u8;
            image::Rgb([g.saturating_add(tint), g, 255 - g])
        }
    });
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn encoded(w: u32, h: u32, format: image::ImageFormat) -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbImage::from_pixel(w, h, image::Rgb([200, 80, 20]))
        .write_to(&mut std::io::Cursor::new(&mut out), format)
        .unwrap();
    out
}

async fn read_at(app: &TestApp, app_id: Uuid, commit: &str) -> Result<Uuid, String> {
    match slop_api::media::refresh(
        &app.state,
        app_id,
        "https://github.com/octo/game",
        Some(commit),
        "maker",
    )
    .await
    {
        Ok(slop_api::media::Outcome::Pending(set)) => Ok(set),
        Ok(other) => panic!("expected new pictures, got {other:?}"),
        Err(problem) => Err(problem.0),
    }
}

fn manifest(commit: &str, manifest: Value) {
    put_raw(commit, "media.json", manifest.to_string().into_bytes());
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn icons_are_reviewed_with_the_set_and_shown_once_approved(db: PgPool) {
    let app = test_app(db).await;
    let (maker, cookie) = app.user("Maker", "US").await;
    let (_, admin_cookie) = make_admin(&app, "Admin").await;
    let commit = "1".repeat(40);
    let app_id = app_with_pictures(&app, maker, "iconic", &commit).await;
    put_raw(&commit, "icon.png", icon(512, 0));

    // No icon yet: a recommendation for the maker's page, get_fixes and check_repository, and
    // never a fix that blocks.
    let overview = body_json(
        app.send(req(Method::GET, "/v1/apps/iconic/media").cookie(&cookie))
            .await,
    )
    .await;
    let todo = &overview["recommendations"][0];
    assert_eq!(todo["kind"], "icon:missing", "{overview}");
    assert_eq!(todo["severity"], "recommended");
    let instruction = todo["instruction"].as_str().unwrap();
    assert!(instruction.contains(".slop/icon.png"), "{instruction}");
    assert!(
        instruction.contains("\"icon\": \"icon.png\""),
        "{instruction}"
    );
    assert!(instruction.contains("512×512"), "{instruction}");
    assert_eq!(todo["guide_section"], "Pictures and videos");
    assert_eq!(overview["icon_problem"], Value::Null);
    let fixes = body_json(
        app.send(req(Method::GET, "/v1/apps/iconic/fixes").cookie(&cookie))
            .await,
    )
    .await;
    assert_eq!(fixes["fixes"], json!([]));
    assert_eq!(fixes["recommendations"][0]["kind"], "icon:missing");
    let res = app
        .send(req(Method::POST, "/v1/apps/iconic/repo-check").cookie(&cookie))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let checked = body_json(res).await;
    assert_eq!(checked["recommendations"][0]["kind"], "icon:missing");

    // media.json names it: read, checked and stored in a set waiting for review.
    manifest(
        &commit,
        json!({ "icon": "icon.png", "thumbnail": "thumb.png", "gallery": [{ "image": "shots/a.png" }] }),
    );
    let set = read_at(&app, app_id, &commit).await.unwrap();
    let overview = body_json(
        app.send(req(Method::GET, "/v1/apps/iconic/media").cookie(&cookie))
            .await,
    )
    .await;
    let url = format!("{}raw/octo/game/{commit}/.slop/icon.png", app.fake_base);
    let stored = &overview["pending"]["icon"];
    assert_eq!(stored["url"], url.as_str(), "{overview}");
    assert_eq!(stored["width"], 512);
    assert_eq!(stored["height"], 512);
    assert_eq!(stored["format"], "png");
    assert_eq!(stored["dhash"].as_str().unwrap().len(), 16);
    // One waiting counts: nothing more to recommend.
    assert_eq!(overview["recommendations"], json!([]));
    let fixes = body_json(
        app.send(req(Method::GET, "/v1/apps/iconic/fixes").cookie(&cookie))
            .await,
    )
    .await;
    assert_eq!(fixes["recommendations"], json!([]));

    // Not shown until a reviewer approves the set, who sees it with the rest.
    let page = body_json(
        app.send(req(Method::GET, "/v1/apps/iconic").cookie(&cookie))
            .await,
    )
    .await;
    assert_eq!(page["icon_url"], Value::Null);
    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/apps").cookie(&cookie))
            .await,
    )
    .await;
    assert_eq!(mine[0]["icon_url"], Value::Null, "{mine}");
    let queue = body_json(
        app.send(req(Method::GET, "/v1/admin/review").cookie(&admin_cookie))
            .await,
    )
    .await;
    let media = queue["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["kind"] == "media")
        .expect("pictures waiting");
    assert_eq!(media["detail"]["pictures"]["icon"]["url"], url.as_str());

    let res = app
        .send(req(Method::POST, format!("/v1/admin/media/{set}/approve")).cookie(&admin_cookie))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let page = body_json(
        app.send(req(Method::GET, "/v1/apps/iconic").cookie(&cookie))
            .await,
    )
    .await;
    assert_eq!(page["icon_url"], url.as_str());
    assert_eq!(
        page["thumbnail_url"],
        format!("{}raw/octo/game/{commit}/.slop/thumb.png", app.fake_base)
    );
    // The library, the maker's list and the store all carry it.
    let res = app
        .send(req(Method::PUT, "/v1/me/library/iconic").cookie(&cookie))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let library = body_json(
        app.send(req(Method::GET, "/v1/me/library").cookie(&cookie))
            .await,
    )
    .await;
    assert_eq!(library[0]["icon_url"], url.as_str(), "{library}");
    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/apps").cookie(&cookie))
            .await,
    )
    .await;
    assert_eq!(mine[0]["icon_url"], url.as_str());
    sqlx::query("update apps set status = 'listed' where id = $1")
        .bind(app_id)
        .execute(&app.db)
        .await
        .unwrap();
    let store = body_json(app.send(req(Method::GET, "/v1/apps?q=iconic")).await).await;
    assert_eq!(store["apps"][0]["slug"], "iconic", "{store}");
    assert_eq!(store["apps"][0]["icon_url"], url.as_str());

    // A newer set turned down leaves the approved icon showing.
    let next = "2".repeat(40);
    app_with_pictures_files(&next);
    put_raw(&next, "icon.png", icon(256, 40));
    manifest(
        &next,
        json!({ "icon": "icon.png", "thumbnail": "thumb.png" }),
    );
    let newer = read_at(&app, app_id, &next).await.unwrap();
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/media/{newer}/reject"))
                .cookie(&admin_cookie)
                .json(json!({ "reason": "The icon is another game's logo." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let page = body_json(
        app.send(req(Method::GET, "/v1/apps/iconic").cookie(&cookie))
            .await,
    )
    .await;
    assert_eq!(page["icon_url"], url.as_str());
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn icons_that_cant_be_used_are_explained(db: PgPool) {
    let app = test_app(db).await;
    let (maker, cookie) = app.user("Maker", "US").await;
    let commit = "3".repeat(40);
    let app_id = app_with_pictures(&app, maker, "bad-icons", &commit).await;
    put_raw(&commit, "wide.png", png(600, 300));
    put_raw(&commit, "tiny.png", png(200, 200));
    put_raw(&commit, "huge.png", png(2048, 2048));
    put_raw(
        &commit,
        "anim.gif",
        encoded(300, 300, image::ImageFormat::Gif),
    );
    put_raw(
        &commit,
        "fake.png",
        encoded(300, 300, image::ImageFormat::Gif),
    );
    put_raw(&commit, "heavy.png", vec![7u8; 5 * 1024 * 1024]);
    for (path, expect) in [
        ("wide.png", "600×300; the icon has to be square"),
        ("tiny.png", "at least 256×256"),
        ("huge.png", "up to 1024×1024"),
        ("anim.gif", "isn't a PNG, JPEG or WebP picture"),
        ("icon.svg", "isn't a PNG, JPEG or WebP picture"),
        ("fake.png", "is a GIF"),
        ("heavy.png", "is over 4 MB"),
        ("missing.png", "isn't in the repository"),
    ] {
        manifest(&commit, json!({ "icon": path, "thumbnail": "thumb.png" }));
        let problem = read_at(&app, app_id, &commit).await.unwrap_err();
        assert!(problem.starts_with("icon: "), "{path}: {problem}");
        assert!(problem.contains(expect), "{path}: {problem}");
    }
    // The maker's page shows the last one on the icon.
    let overview = body_json(
        app.send(req(Method::GET, "/v1/apps/bad-icons/media").cookie(&cookie))
            .await,
    )
    .await;
    assert!(
        overview["icon_problem"]
            .as_str()
            .unwrap()
            .starts_with(".slop/missing.png isn't in the repository"),
        "{overview}"
    );
    assert_eq!(overview["pending"], Value::Null);

    // Square JPEGs and WebPs are fine too.
    put_raw(
        &commit,
        "icon.jpg",
        encoded(300, 300, image::ImageFormat::Jpeg),
    );
    manifest(&commit, json!({ "icon": "icon.jpg" }));
    read_at(&app, app_id, &commit).await.unwrap();
    let other = "4".repeat(40);
    put_raw(
        &other,
        "icon.webp",
        encoded(1024, 1024, image::ImageFormat::WebP),
    );
    manifest(&other, json!({ "icon": "icon.webp" }));
    read_at(&app, app_id, &other).await.unwrap();
    let overview = body_json(
        app.send(req(Method::GET, "/v1/apps/bad-icons/media").cookie(&cookie))
            .await,
    )
    .await;
    assert_eq!(overview["pending"]["icon"]["format"], "webp");
    assert_eq!(overview["icon_problem"], Value::Null);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn an_icon_like_another_makers_is_pointed_out(db: PgPool) {
    let app = test_app(db).await;
    let (famous_maker, _) = app.user("Famous Studio", "US").await;
    let (copycat, _) = app.user("Copy Cat", "US").await;
    listed_with(&app, famous_maker, "famous", "released", "").await;
    let hash = format!("{:016x}", slop_api::media::dhash(&icon(512, 0)).unwrap());
    sqlx::query(
        "with s as (
             insert into media_sets (id, app_id, commit_sha, icon, fingerprint, source, status)
             select $1, a.id, repeat('c', 40),
                    jsonb_build_object('path', 'icon.png', 'url', 'https://example.com/i.png', 'sha256', 'x',
                                       'width', 512, 'height', 512, 'format', 'png', 'dhash', $2::text),
                    'f', 'maker', 'approved'
             from apps a where a.slug = 'famous' returning id, app_id)
         update apps set media_set_id = s.id from s where apps.id = s.app_id",
    )
    .bind(Uuid::now_v7())
    .bind(&hash)
    .execute(&app.db)
    .await
    .unwrap();

    // The copy: the same icon, smaller and tinted.
    let commit = "5".repeat(40);
    let app_id = app_with_pictures(&app, copycat, "not-famous", &commit).await;
    put_raw(&commit, "icon.png", icon(300, 10));
    manifest(&commit, json!({ "icon": "icon.png" }));
    let set = read_at(&app, app_id, &commit).await.unwrap();
    let like = slop_api::media::load_set(&app.db, set)
        .await
        .unwrap()
        .lookalikes;
    assert_eq!(like.len(), 1, "{like:?}");
    assert_eq!(like[0].slug, "famous");
    assert_eq!(like[0].kind, "icon");
    assert!(like[0].distance <= 10);

    // A different icon isn't.
    let other = "6".repeat(40);
    let stripes = image::RgbImage::from_fn(256, 256, |x, y| {
        let v = if (x / 32 + y / 48).is_multiple_of(2) {
            230
        } else {
            30
        };
        image::Rgb([v, (255 - y) as u8, v / 2])
    });
    let mut png_bytes = Vec::new();
    stripes
        .write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    put_raw(&other, "icon.png", png_bytes);
    manifest(&other, json!({ "icon": "icon.png" }));
    let set = read_at(&app, app_id, &other).await.unwrap();
    let like = slop_api::media::load_set(&app.db, set)
        .await
        .unwrap()
        .lookalikes;
    assert!(like.iter().all(|l| l.kind != "icon"), "{like:?}");
}
