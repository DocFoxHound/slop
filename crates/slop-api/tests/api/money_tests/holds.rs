// SPDX-License-Identifier: AGPL-3.0-or-later
//! Donation holds: staff pausing or turning off an app's donations, two-person approval, lifting,
//! makers asking for them back, admin agents doing the same through tokens, and the money
//! overview.

use super::*;

const REVIEW: &str = "A trademark complaint says the name is theirs.";

fn hold_body(kind: &str) -> Value {
    json!({ "kind": kind, "reason": REVIEW, "ground": "Complaint TM-1" })
}

async fn place(app: &TestApp, cookie: &str, slug: &str, body: Value) -> (StatusCode, Value) {
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/apps/{slug}/donation-hold"))
                .cookie(cookie)
                .json(body),
        )
        .await;
    let status = res.status();
    (status, body_json(res).await)
}

async fn ask(app: &TestApp, cookie: &str, slug: &str, message: &str) -> StatusCode {
    app.send(
        req(
            Method::POST,
            format!("/v1/apps/{slug}/donations/hold/request"),
        )
        .cookie(cookie)
        .json(json!({ "message": message })),
    )
    .await
    .status()
}

async fn hold_kind(app: &TestApp) -> Option<String> {
    sqlx::query_scalar("select kind from donation_holds where lifted_at is null")
        .fetch_optional(&app.db)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_list_open_holds_with_waiting_requests_first(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (maker_id, maker) = maker_taking_donations(&app, "tower", false).await;
    listed_with(&app, maker_id, "arena", "released", "").await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (_, player) = app.user("Pat Player", "US").await;
    assert_eq!(
        place(&app, &owner, "tower", hold_body("review")).await.0,
        StatusCode::OK
    );
    assert_eq!(
        place(&app, &owner, "arena", hold_body("review")).await.0,
        StatusCode::OK
    );
    let list = |q: &'static str, cookie: String| {
        let app = &app;
        async move {
            let rows = get_json(app, &cookie, &format!("/v1/admin/donation-holds{q}")).await;
            rows.as_array()
                .unwrap()
                .iter()
                .map(|r| r["slug"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(
        list("", owner.clone()).await,
        ["arena", "tower"],
        "newest first"
    );
    assert_eq!(
        ask(&app, &maker, "tower", "Renamed it.").await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        list("", reviewer.clone()).await,
        ["tower", "arena"],
        "a waiting request first; any staff member may look"
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/donation-holds").cookie(&player))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let id = get_json(&app, &owner, "/v1/admin/donation-holds").await[1]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/donation-holds/{id}/lift"))
                .cookie(&owner)
                .json(json!({ "note": "Not theirs after all." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(list("", owner.clone()).await, ["tower"]);
    let all = get_json(&app, &owner, "/v1/admin/donation-holds?show=all").await;
    assert_eq!(all.as_array().unwrap().len(), 2);
    let arena = all
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["slug"] == "arena")
        .unwrap();
    assert!(arena["lifted_at"].is_string());
    assert_eq!(arena["lift_note"], "Not theirs after all.");
    assert_eq!(all[0]["requests"][0]["message"], "Renamed it.");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn pausing_for_review_stops_collection_at_stripe_and_closes_open_checkouts(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (maker_id, _) = maker_taking_donations(&app, "arena", true).await;
    let acct: String = sqlx::query_scalar("select stripe_account_id from donation_accounts")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let (modder_id, modder) = app.user("Bo Modder", "US").await;
    community_build(&app, "arena", "arena-mod", modder_id).await;
    donations_on(&app, &modder, "arena-mod", false).await;
    let (_, finance) = staff(&app, "Fin Finance", "finance").await;
    let (_, reviewer) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (dee_id, dee) = adult(&app, "Dee Donor", "US").await;
    let (_, eve) = adult(&app, "Eve Donor", "US").await;
    let (_, started) = donate(
        &app,
        &dee,
        "arena",
        json!({ "amount_minor": 500, "monthly": true, "consent": true }),
    )
    .await;
    let session = started["url"].as_str().unwrap().rsplit('/').next().unwrap();
    fake.patch_session(
        session,
        json!({ "status": "complete", "subscription": "sub_dee" }),
    );
    let id = started["id"].as_str().unwrap();
    app.send(req(Method::POST, format!("/v1/me/donations/{id}/check")).cookie(&dee))
        .await;
    // Eve is on a Checkout page when the hold comes.
    let (_, open) = donate(&app, &eve, "arena", json!({ "amount_minor": 300 })).await;
    let open_session = open["url"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_string();
    assert_eq!(
        get_json(&app, &dee, "/v1/apps/arena-mod/donate").await["shared"]["slug"],
        "arena"
    );

    let mut long = hold_body("review");
    long["ground"] = json!("x".repeat(301));
    let (status, body) = place(&app, &finance, "arena", long).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["field"], "ground");
    assert_eq!(
        place(&app, &finance, "no-such-app", hold_body("review"))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        place(&app, &reviewer, "arena", hold_body("review")).await.0,
        StatusCode::FORBIDDEN
    );
    // Stripe won't pause the subscription: nothing changes here.
    fake.fail("POST", "v1/subscriptions", 500);
    assert_eq!(
        place(&app, &finance, "arena", hold_body("review")).await.0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(hold_kind(&app).await, None);
    assert!(
        notices(&app, maker_id)
            .await
            .iter()
            .all(|t| !t.contains("paused"))
    );
    fake.heal();

    let (status, placed) = place(&app, &finance, "arena", hold_body("review")).await;
    assert_eq!(status, StatusCode::OK, "{placed}");
    let paused = fake.calls("POST", "v1/subscriptions/sub_dee");
    let last = paused.last().unwrap();
    assert_eq!(last.account.as_deref(), Some(acct.as_str()));
    assert_eq!(last.form["pause_collection[behavior]"], "void");
    let expired = fake.calls(
        "POST",
        &format!("v1/checkout/sessions/{open_session}/expire"),
    );
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].account.as_deref(), Some(acct.as_str()));
    assert!(
        notices(&app, dee_id)
            .await
            .contains(&"Your server donation to arena is paused".to_string())
    );
    assert!(
        notices(&app, maker_id)
            .await
            .contains(&"Donations to arena are paused while Slop reviews it".to_string())
    );
    // The original's share of donations to its community builds stops too.
    assert!(get_json(&app, &dee, "/v1/apps/arena-mod/donate").await["shared"].is_null());
    let detail: Value =
        sqlx::query_scalar("select detail from audit_log where action = 'donations.hold_placed'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(detail["server_donations_paused"], 1);
    assert_eq!(detail["ground"], "Complaint TM-1");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn turning_donations_off_takes_a_second_staff_member(db: PgPool) {
    let app = test_app_with(db, |c| {
        simulated(c);
        c.approvals_required = 2;
    })
    .await;
    let (maker_id, maker) = maker_taking_donations(&app, "arena", false).await;
    let (_, olive) = make_admin(&app, "Olive Owner").await;
    let (_, moe) = staff(&app, "Moe Moderator", "moderator").await;
    let (_, sue) = staff(&app, "Sue Support", "support").await;
    let (_, fin) = staff(&app, "Fin Finance", "finance").await;
    for cookie in [&sue, &fin] {
        assert_eq!(
            place(&app, cookie, "arena", hold_body("off")).await.0,
            StatusCode::FORBIDDEN,
            "turning off is a sensitive action"
        );
    }
    assert_eq!(
        place(&app, &olive, "no-such-app", hold_body("off")).await.0,
        StatusCode::NOT_FOUND
    );
    let (status, pending) = place(&app, &olive, "arena", hold_body("off")).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{pending}");
    assert_eq!(pending["approvals_still_needed"], 1);
    assert_eq!(hold_kind(&app).await, None, "nothing yet");
    assert!(get_json(&app, &maker, "/v1/apps/arena/donations/hold").await["open"].is_null());
    let action = pending["pending_action"].as_str().unwrap();
    let approve = |cookie: &str| {
        req(Method::POST, format!("/v1/admin/actions/{action}/approve")).cookie(cookie)
    };
    assert_eq!(
        app.send(approve(&olive)).await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(approve(&fin)).await.status(),
        StatusCode::FORBIDDEN
    );
    let res = app.send(approve(&moe)).await;
    assert_eq!(res.status(), StatusCode::OK, "{:?}", body_json(res).await);
    assert_eq!(hold_kind(&app).await.as_deref(), Some("off"));
    assert!(
        notices(&app, maker_id)
            .await
            .contains(&"Donations to arena are turned off until the problem is fixed".to_string())
    );
    let placed_by: String = sqlx::query_scalar("select placed_by from donation_holds")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(!placed_by.is_empty());
}

/// b28: a pause for review placed over a hold that turned donations off must not turn it back
/// into a pause (which the same person could then lift on their own).
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_pause_for_review_cant_soften_a_hold_that_turned_donations_off(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    maker_taking_donations(&app, "arena", false).await;
    let (_, olive) = make_admin(&app, "Olive Owner").await;
    let (_, fin) = staff(&app, "Fin Finance", "finance").await;
    assert_eq!(
        place(&app, &olive, "arena", hold_body("off")).await.0,
        StatusCode::OK
    );
    let (status, _) = place(&app, &fin, "arena", hold_body("review")).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(hold_kind(&app).await.as_deref(), Some("off"));
    let id: Uuid = sqlx::query_scalar("select id from donation_holds")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/donation-holds/{id}/lift"))
                .cookie(&fin)
                .json(json!({ "note": "Fine now." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(hold_kind(&app).await.as_deref(), Some("off"));
    // A review hold can still be turned off, and its reason changed.
    let (_, moe) = staff(&app, "Moe Moderator", "moderator").await;
    let mut changed = hold_body("off");
    changed["reason"] = json!("The old name is still in the window title.");
    assert_eq!(place(&app, &moe, "arena", changed).await.0, StatusCode::OK);
    let reason: String = sqlx::query_scalar("select reason from donation_holds")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(reason, "The old name is still in the window title.");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn lifting_a_hold_restarts_collection_and_closes_waiting_requests(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (maker_id, maker) = maker_taking_donations(&app, "arena", true).await;
    let acct: String = sqlx::query_scalar("select stripe_account_id from donation_accounts")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let (_, moe) = staff(&app, "Moe Moderator", "moderator").await;
    let (dee_id, dee) = adult(&app, "Dee Donor", "US").await;
    let (_, started) = donate(
        &app,
        &dee,
        "arena",
        json!({ "amount_minor": 500, "monthly": true, "consent": true }),
    )
    .await;
    let session = started["url"].as_str().unwrap().rsplit('/').next().unwrap();
    fake.patch_session(
        session,
        json!({ "status": "complete", "subscription": "sub_dee" }),
    );
    let id = started["id"].as_str().unwrap();
    app.send(req(Method::POST, format!("/v1/me/donations/{id}/check")).cookie(&dee))
        .await;
    assert_eq!(
        place(&app, &moe, "arena", hold_body("review")).await.0,
        StatusCode::OK
    );
    assert_eq!(
        ask(&app, &maker, "arena", "Renamed everywhere.").await,
        StatusCode::NO_CONTENT
    );
    let hold: Uuid = sqlx::query_scalar("select id from donation_holds")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let lift = |note: &str| {
        req(
            Method::POST,
            format!("/v1/admin/donation-holds/{hold}/lift"),
        )
        .cookie(&moe)
        .json(json!({ "note": note }))
    };
    let res = app.send(lift("   ")).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "note");
    fake.fail("POST", "v1/subscriptions", 500);
    assert_eq!(
        app.send(lift("Fixed.")).await.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        hold_kind(&app).await.as_deref(),
        Some("review"),
        "still held"
    );
    fake.heal();
    assert_eq!(
        app.send(lift("Renamed everywhere, thanks.")).await.status(),
        StatusCode::NO_CONTENT
    );
    let resumed = fake.calls("POST", "v1/subscriptions/sub_dee");
    let last = resumed.last().unwrap();
    assert_eq!(last.account.as_deref(), Some(acct.as_str()));
    assert_eq!(last.form["pause_collection"], "", "cleared");
    assert!(
        notices(&app, dee_id)
            .await
            .contains(&"Your server donation to arena has restarted".to_string())
    );
    assert!(
        notices(&app, maker_id)
            .await
            .contains(&"Donations to arena are back on".to_string())
    );
    let (outcome, answer): (String, String) =
        sqlx::query_as("select outcome, answer from donation_hold_requests")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        (outcome.as_str(), answer.as_str()),
        ("lifted", "Renamed everywhere, thanks.")
    );
    assert_eq!(
        app.send(lift("Again.")).await.status(),
        StatusCode::NOT_FOUND
    );
    assert!(get_json(&app, &dee, "/v1/apps/arena/donate").await["on"] == true);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn answering_a_makers_request_needs_the_right_to_lift_that_hold(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (maker_id, maker) = maker_taking_donations(&app, "arena", false).await;
    let (_, olive) = make_admin(&app, "Olive Owner").await;
    let (_, sue) = staff(&app, "Sue Support", "support").await;
    assert_eq!(
        place(&app, &olive, "arena", hold_body("off")).await.0,
        StatusCode::OK
    );
    assert_eq!(
        ask(&app, &maker, "arena", "Fixed the title.").await,
        StatusCode::NO_CONTENT
    );
    let request: Uuid = sqlx::query_scalar("select id from donation_hold_requests")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let answer = |cookie: &str, body: Value| {
        req(
            Method::POST,
            format!("/v1/admin/donation-hold-requests/{request}/answer"),
        )
        .cookie(cookie)
        .json(body)
    };
    let res = app
        .send(answer(&sue, json!({ "lift": true, "answer": " " })))
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "answer");
    // Support looks after pauses for review; lifting "turned off" is a sensitive action.
    assert_eq!(
        app.send(answer(
            &sue,
            json!({ "lift": true, "answer": "Looks fixed." })
        ))
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(hold_kind(&app).await.as_deref(), Some("off"));
    assert_eq!(
        app.send(answer(
            &sue,
            json!({ "lift": false, "answer": "The title still says Arena." })
        ))
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert!(
        notices(&app, maker_id)
            .await
            .contains(&"Donations to arena stay paused for now".to_string())
    );
    assert_eq!(
        app.send(answer(
            &olive,
            json!({ "lift": true, "answer": "Now it's fixed." })
        ))
        .await
        .status(),
        StatusCode::NOT_FOUND,
        "already answered"
    );

    // The maker asks again a day later; the owner lifts it from the answer.
    assert_eq!(
        ask(&app, &maker, "arena", "Now really.").await,
        StatusCode::CONFLICT
    );
    sqlx::query("update donation_hold_requests set answered_at = now() - interval '25 hours'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        ask(&app, &maker, "arena", "Now really.").await,
        StatusCode::NO_CONTENT
    );
    let second: Uuid =
        sqlx::query_scalar("select id from donation_hold_requests where outcome is null")
            .fetch_one(&app.db)
            .await
            .unwrap();
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/donation-hold-requests/{second}/answer"),
            )
            .cookie(&olive)
            .json(json!({ "lift": true, "answer": "Thanks, all fixed." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(hold_kind(&app).await, None);
    assert!(
        notices(&app, maker_id)
            .await
            .contains(&"Donations to arena are back on".to_string())
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn makers_see_their_hold_and_ask_for_review_once_a_day(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (_, maker) = maker_taking_donations(&app, "arena", false).await;
    let (_, other) = app.user("Otto Other", "US").await;
    let (olive_id, olive) = make_admin(&app, "Olive Owner").await;
    let (moe_id, _) = staff(&app, "Moe Moderator", "moderator").await;
    let (fin_id, _) = staff(&app, "Fin Finance", "finance").await;
    let (rex_id, _) = staff(&app, "Rex Reviewer", "reviewer").await;
    let (sue_id, _) = staff(&app, "Sue Support", "support").await;
    let mine = || {
        let app = &app;
        let maker = maker.clone();
        async move { get_json(app, &maker, "/v1/apps/arena/donations/hold").await }
    };
    let m = mine().await;
    assert!(m["open"].is_null() && m["can_ask_at"].is_null(), "{m}");
    assert_eq!(
        ask(&app, &maker, "arena", "Please?").await,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/arena/donations/hold").cookie(&other))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    place(&app, &olive, "arena", hold_body("review")).await;
    assert_eq!(
        ask(&app, &other, "arena", "Please?").await,
        StatusCode::NOT_FOUND
    );
    for bad in ["   ", &"x".repeat(4001)] {
        assert_eq!(
            ask(&app, &maker, "arena", bad).await,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert!(mine().await["can_ask_at"].is_null(), "may ask now");
    assert_eq!(
        ask(&app, &maker, "arena", "Renamed it.").await,
        StatusCode::NO_CONTENT
    );
    assert!(mine().await["can_ask_at"].is_string(), "one waiting");
    // The staff who can answer are told; the others aren't.
    for (id, told) in [
        (olive_id, true),
        (moe_id, true),
        (fin_id, true),
        (rex_id, false),
        (sue_id, false),
    ] {
        assert_eq!(
            notices(&app, id)
                .await
                .contains(&"A maker asks for donations back".to_string()),
            told,
            "{id}"
        );
    }
    // Answered: a day before asking again.
    sqlx::query(
        "update donation_hold_requests set outcome = 'kept', answer = 'No.', answered_by = 'x', answered_at = now()",
    )
    .execute(&app.db)
    .await
    .unwrap();
    assert!(mine().await["can_ask_at"].is_string());
    assert_eq!(
        ask(&app, &maker, "arena", "Again.").await,
        StatusCode::CONFLICT
    );
    sqlx::query("update donation_hold_requests set answered_at = now() - interval '25 hours'")
        .execute(&app.db)
        .await
        .unwrap();
    assert!(mine().await["can_ask_at"].is_null());
    assert_eq!(
        ask(&app, &maker, "arena", "Again.").await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        mine().await["open"]["requests"].as_array().unwrap().len(),
        2
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn admin_agents_pause_answer_and_lift_with_the_right_token(db: PgPool) {
    let app = test_app_with(db, |c| {
        simulated(c);
        c.approvals_required = 2;
    })
    .await;
    let (_, maker) = maker_taking_donations(&app, "arena", false).await;
    let (_, olive) = make_admin(&app, "Olive Owner").await;
    let mut tokens = HashMap::new();
    for scope in ["read", "review", "moderate"] {
        let res = app
            .send(
                req(Method::POST, "/v1/admin/tokens")
                    .cookie(&olive)
                    .json(json!({ "name": format!("Claude {scope}"), "scope": scope, "days": 7 })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::CREATED);
        tokens.insert(
            scope,
            format!("Bearer {}", body_json(res).await["token"].as_str().unwrap()),
        );
    }
    let call = |scope: &str, name: &str, args: Value| {
        let app = &app;
        let token = tokens[scope].clone();
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                           "params": { "name": name, "arguments": args } });
        async move {
            let res = body_json(
                app.send(
                    req(Method::POST, "/v1/admin/mcp")
                        .header("authorization", token)
                        .json(body),
                )
                .await,
            )
            .await;
            (
                res["result"]["isError"].as_bool().unwrap(),
                res["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            )
        }
    };
    let pause = |kind: &str| json!({ "slug": "arena", "kind": kind, "reason": REVIEW });

    let (error, _) = call("read", "pause_donations", pause("review")).await;
    assert!(error, "a read token only looks");
    let (error, _) = call("review", "pause_donations", pause("off")).await;
    assert!(error, "turning off needs a moderate token");
    // A moderate token asks; a person still has to approve.
    let (error, text) = call("moderate", "pause_donations", pause("off")).await;
    assert!(!error, "{text}");
    assert!(text.contains("pending_action"), "{text}");
    assert_eq!(hold_kind(&app).await, None);

    let (error, text) = call("review", "pause_donations", pause("review")).await;
    assert!(!error, "{text}");
    assert_eq!(hold_kind(&app).await.as_deref(), Some("review"));
    let (error, text) = call("read", "donation_pauses", json!({})).await;
    assert!(!error && text.contains("arena"), "{text}");
    let hold: Uuid = sqlx::query_scalar("select id from donation_holds")
        .fetch_one(&app.db)
        .await
        .unwrap();

    assert_eq!(
        ask(&app, &maker, "arena", "Renamed it.").await,
        StatusCode::NO_CONTENT
    );
    let request: Uuid = sqlx::query_scalar("select id from donation_hold_requests")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let answer =
        json!({ "id": request, "lift": false, "answer": "Still the old name in the title." });
    let (error, _) = call("read", "answer_donation_request", answer.clone()).await;
    assert!(error);
    let (error, text) = call("review", "answer_donation_request", answer).await;
    assert!(!error, "{text}");
    let outcome: Option<String> = sqlx::query_scalar("select outcome from donation_hold_requests")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(outcome.as_deref(), Some("kept"));

    let lift = json!({ "id": hold, "note": "Fixed now." });
    let (error, _) = call("read", "lift_donation_pause", lift.clone()).await;
    assert!(error);
    let (error, text) = call("review", "lift_donation_pause", lift).await;
    assert!(!error, "{text}");
    assert_eq!(hold_kind(&app).await, None);
    let actors: Vec<String> = sqlx::query_scalar(
        "select actor from audit_log where action in ('donations.hold_placed', 'donations.hold_kept', 'donations.hold_lifted') order by id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(actors.len(), 3);
    assert!(
        actors.iter().all(|a| a.contains("Claude review")),
        "{actors:?}"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_money_overview_is_for_finance_staff(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (_, maker) = maker_taking_donations(&app, "arena", true).await;
    let (_, fin) = staff(&app, "Fin Finance", "finance").await;
    let (_, moe) = staff(&app, "Moe Moderator", "moderator").await;
    let (_, olive) = make_admin(&app, "Olive Owner").await;
    let (_, dee) = adult(&app, "Dee Donor", "US").await;
    donate(&app, &dee, "arena", json!({ "amount_minor": 300 })).await;
    donate(
        &app,
        &dee,
        "arena",
        json!({ "amount_minor": 500, "monthly": true, "consent": true }),
    )
    .await;
    place(&app, &olive, "arena", hold_body("review")).await;
    ask(&app, &maker, "arena", "Renamed it.").await;
    sqlx::query("insert into stripe_events (id, kind) values ('evt_1', 'invoice.paid')")
        .execute(&app.db)
        .await
        .unwrap();
    let m = get_json(&app, &fin, "/v1/admin/money").await;
    assert_eq!(m["payments"], "simulated");
    assert_eq!(m["active_server_donations"], 1);
    assert_eq!(m["open_holds"], 1);
    assert_eq!(m["hold_requests_waiting"], 1);
    assert_eq!(m["stripe_events_today"], 1);
    let months = m["months"].as_array().unwrap();
    assert_eq!(months.len(), 1, "{m}");
    assert_eq!(months[0]["kind"], "donation");
    assert_eq!(months[0]["currency"], "usd");
    assert_eq!(months[0]["payments"], 2);
    assert_eq!(months[0]["amount_minor"], 340 + 546);
    assert_eq!(months[0]["slop_fee_minor"], 15 + 25);
    assert_eq!(m["recent"].as_array().unwrap().len(), 2);
    assert_eq!(m["recent"][0]["simulated"], true);
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/money").cookie(&moe))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/money").cookie(&dee))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}
