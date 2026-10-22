// SPDX-License-Identifier: AGPL-3.0-or-later
//! Offering a community build's changes back to the original: listing, offering, answering,
//! withdrawing, the daily limit, and the maker agent's offer tools.

use super::*;

/// "game" by Mia (repository github.com/octo/game) and its community build "game-fix" by
/// Forker. Returns (Mia, Forker) as (id, cookie).
async fn original_and_build(app: &TestApp) -> ((Uuid, String), (Uuid, String)) {
    let (mia_id, mia) = app.user("Mia Maker", "US").await;
    listed_with(
        app,
        mia_id,
        "game",
        "released",
        "repo_url = 'https://github.com/octo/game'",
    )
    .await;
    let (forker_id, forker) = app.user("Forker", "US").await;
    community_build(app, "game", "game-fix", forker_id).await;
    ((mia_id, mia), (forker_id, forker))
}

async fn offer(
    app: &TestApp,
    cookie: &str,
    slug: &str,
    pr: u32,
    message: &str,
) -> (StatusCode, Value) {
    let res = app
        .send(
            req(Method::POST, format!("/v1/apps/{slug}/offers"))
                .cookie(cookie)
                .json(json!({
                    "pull_request_url": format!("https://github.com/octo/game/pull/{pr}"),
                    "message": message
                })),
        )
        .await;
    let status = res.status();
    (status, body_json(res).await)
}

async fn answer(app: &TestApp, cookie: &str, slug: &str, id: &str, body: Value) -> StatusCode {
    app.send(
        req(Method::POST, format!("/v1/apps/{slug}/offers/{id}/answer"))
            .cookie(cookie)
            .json(body),
    )
    .await
    .status()
}

async fn withdraw(app: &TestApp, cookie: &str, slug: &str, id: &str) -> StatusCode {
    app.send(
        req(
            Method::POST,
            format!("/v1/apps/{slug}/offers/{id}/withdraw"),
        )
        .cookie(cookie),
    )
    .await
    .status()
}

async fn offer_status(app: &TestApp, id: &str) -> String {
    sqlx::query_scalar("select status from upstream_offers where id = $1")
        .bind(id.parse::<Uuid>().unwrap())
        .fetch_one(&app.db)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn offered_fixes_are_listed_for_the_apps_own_makers_only(db: PgPool) {
    let app = test_app(db).await;
    let ((_, mia), (_, forker)) = original_and_build(&app).await;
    let (_, other) = app.user("Otto Other", "US").await;
    let (status, _) = offer(&app, &forker, "game-fix", 7, "Fixes the floaty jump.").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/game/offers"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    for slug in ["game", "game-fix"] {
        assert_eq!(
            app.send(req(Method::GET, format!("/v1/apps/{slug}/offers")).cookie(&other))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    let received = get_json(&app, &mia, "/v1/apps/game/offers").await;
    assert_eq!(received[0]["direction"], "received");
    assert_eq!(received[0]["offered_by"], "Forker");
    assert_eq!(received[0]["message"], "Fixes the floaty jump.");
    let sent = get_json(&app, &forker, "/v1/apps/game-fix/offers").await;
    assert_eq!(sent[0]["direction"], "sent");
    assert_eq!(sent[0]["status"], "open");
    // The original's maker can't read the build's list as if it were theirs.
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/game-fix/offers").cookie(&mia))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn offering_a_fix_checks_the_app_the_message_and_a_daily_limit(db: PgPool) {
    let app = test_app(db).await;
    let ((mia_id, mia), (forker_id, forker)) = original_and_build(&app).await;
    let (_, other) = app.user("Otto Other", "US").await;
    listed_with(&app, forker_id, "solo", "released", "").await;
    let res = app
        .send(
            req(Method::POST, "/v1/apps/solo/offers")
                .cookie(&forker)
                .json(json!({ "pull_request_url": "https://github.com/octo/game/pull/1", "message": "Hi." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body_json(res).await["error"]["field"],
        "slug",
        "not a community build"
    );
    // The original's own maker isn't the build's.
    assert_eq!(
        offer(&app, &mia, "game-fix", 1, "Mine.").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        offer(&app, &other, "game-fix", 1, "Theirs.").await.0,
        StatusCode::NOT_FOUND
    );
    for message in ["   ".to_string(), "x".repeat(2001)] {
        let (status, body) = offer(&app, &forker, "game-fix", 1, &message).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error"]["field"], "message");
    }
    // Each one tells the original's maker, in one notice while it's unread.
    for pr in 1..=2 {
        let (status, body) = offer(&app, &forker, "game-fix", pr, "Another fix.").await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }
    let told = notices(&app, mia_id)
        .await
        .into_iter()
        .filter(|t| t == "game-fix offered you a fix")
        .count();
    assert_eq!(told, 1);
    assert_eq!(count(&app, "select count(*) from upstream_offers").await, 2);

    // Five a day, refused ones included: three went to the checks above.
    let res = offer(&app, &forker, "game-fix", 3, "One more.").await;
    assert_eq!(res.0, StatusCode::TOO_MANY_REQUESTS, "{}", res.1);
    assert_eq!(res.1["error"]["code"], "rate_limited");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_original_answers_an_offer_once_with_an_optional_reply(db: PgPool) {
    let app = test_app(db).await;
    let ((_, mia), (forker_id, forker)) = original_and_build(&app).await;
    let (_, first) = offer(&app, &forker, "game-fix", 1, "Fixes the jump.").await;
    let id = first["id"].as_str().unwrap().to_string();
    assert_eq!(
        answer(&app, &mia, "game", &id, json!({ "decision": "maybe" })).await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        answer(
            &app,
            &mia,
            "game",
            &id,
            json!({ "decision": "declined", "message": "x".repeat(2001) })
        )
        .await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Only through the original, by its maker.
    assert_eq!(
        answer(
            &app,
            &forker,
            "game-fix",
            &id,
            json!({ "decision": "accepted" })
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        answer(
            &app,
            &mia,
            "game",
            &id,
            json!({ "decision": "declined", "message": "  We went another way.  " })
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let o = get_json(&app, &forker, "/v1/apps/game-fix/offers").await;
    assert_eq!(o[0]["status"], "declined");
    assert_eq!(o[0]["answer"], "We went another way.");
    assert!(o[0]["answered_at"].is_string());
    assert!(
        notices(&app, forker_id)
            .await
            .contains(&"game declined your fix".to_string())
    );
    assert_eq!(
        answer(&app, &mia, "game", &id, json!({ "decision": "accepted" })).await,
        StatusCode::NOT_FOUND,
        "answered already"
    );
    // A withdrawn one can't be answered either.
    let (_, second) = offer(&app, &forker, "game-fix", 2, "Fixes the camera.").await;
    let second = second["id"].as_str().unwrap().to_string();
    assert_eq!(
        withdraw(&app, &forker, "game-fix", &second).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        answer(
            &app,
            &mia,
            "game",
            &second,
            json!({ "decision": "accepted" })
        )
        .await,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_build_withdraws_its_own_open_offers(db: PgPool) {
    let app = test_app(db).await;
    let ((_, mia), (_, forker)) = original_and_build(&app).await;
    let (_, first) = offer(&app, &forker, "game-fix", 1, "Fixes the jump.").await;
    let id = first["id"].as_str().unwrap().to_string();
    assert_eq!(
        withdraw(&app, &mia, "game", &id).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        withdraw(&app, &mia, "game-fix", &id).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        withdraw(&app, &forker, "game-fix", &id).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(offer_status(&app, &id).await, "withdrawn");
    assert_eq!(
        withdraw(&app, &forker, "game-fix", &id).await,
        StatusCode::NOT_FOUND
    );
    let (_, second) = offer(&app, &forker, "game-fix", 2, "Fixes the camera.").await;
    let second = second["id"].as_str().unwrap().to_string();
    answer(
        &app,
        &mia,
        "game",
        &second,
        json!({ "decision": "accepted" }),
    )
    .await;
    assert_eq!(
        withdraw(&app, &forker, "game-fix", &second).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(offer_status(&app, &second).await, "accepted");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn makers_agents_offer_answer_and_withdraw_fixes_within_their_tokens(db: PgPool) {
    let app = test_app(db).await;
    let ((mia_id, mia), (forker_id, forker)) = original_and_build(&app).await;
    listed_with(&app, mia_id, "other", "released", "").await;
    listed_with(&app, forker_id, "solo", "released", "").await;
    let token = |cookie: String, body: Value| {
        let app = &app;
        async move {
            let res = app
                .send(
                    req(Method::POST, "/v1/me/maker-tokens")
                        .cookie(&cookie)
                        .json(body),
                )
                .await;
            assert_eq!(res.status(), StatusCode::CREATED);
            format!(
                "Bearer {}",
                body_json(res).await["secret"].as_str().unwrap()
            )
        }
    };
    let forker_full = token(
        forker.clone(),
        json!({ "name": "Claude", "scope": "publish" }),
    )
    .await;
    let forker_solo = token(
        forker.clone(),
        json!({ "name": "Solo only", "scope": "publish", "apps": ["solo"] }),
    )
    .await;
    let mia_read = token(mia.clone(), json!({ "name": "Watcher", "scope": "read" })).await;
    let mia_other = token(
        mia.clone(),
        json!({ "name": "Other only", "scope": "publish", "apps": ["other"] }),
    )
    .await;
    let mia_full = token(mia.clone(), json!({ "name": "Claude", "scope": "publish" })).await;
    let call = |bearer: &str, name: &str, args: Value| {
        let app = &app;
        let r = req(Method::POST, "/v1/maker/mcp")
            .header("authorization", bearer.to_string())
            .json(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                          "params": { "name": name, "arguments": args } }));
        async move {
            let res = body_json(app.send(r).await).await;
            (
                res["result"]["isError"].as_bool().unwrap(),
                res["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            )
        }
    };
    let fix = |pr: u32| {
        json!({ "slug": "game-fix", "pull_request_url": format!("https://github.com/octo/game/pull/{pr}"),
                "message": "Fixes the floaty jump." })
    };
    let (error, _) = call(&forker_solo, "offer_fix_to_original", fix(1)).await;
    assert!(error, "a token for another app");
    let (error, text) = call(&forker_full, "offer_fix_to_original", fix(1)).await;
    assert!(!error, "{text}");
    let id = serde_json::from_str::<Value>(&text).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (error, text) = call(&mia_read, "list_offers", json!({ "slug": "game" })).await;
    assert!(!error && text.contains("Fixes the floaty jump."), "{text}");
    let decide = json!({ "slug": "game", "id": id, "decision": "accepted", "message": "Merged!" });
    let (error, _) = call(&mia_read, "answer_offer", decide.clone()).await;
    assert!(error, "a read token can't answer");
    let (error, _) = call(&mia_other, "answer_offer", decide.clone()).await;
    assert!(error, "a token for another app can't answer");
    let (error, text) = call(&mia_full, "answer_offer", decide).await;
    assert!(!error, "{text}");
    assert_eq!(offer_status(&app, &id).await, "accepted");

    let (_, text) = call(&forker_full, "offer_fix_to_original", fix(2)).await;
    let second = serde_json::from_str::<Value>(&text).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let back = json!({ "slug": "game-fix", "id": second });
    let (error, _) = call(&forker_solo, "withdraw_offer", back.clone()).await;
    assert!(error);
    let (error, text) = call(&forker_full, "withdraw_offer", back).await;
    assert!(!error, "{text}");
    assert_eq!(offer_status(&app, &second).await, "withdrawn");
}
