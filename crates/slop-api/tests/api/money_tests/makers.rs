// SPDX-License-Identifier: AGPL-3.0-or-later
//! The maker's side of money: donation settings, shutting servers down, connecting and changing
//! Stripe accounts (for donations and for People), VAT numbers, fee statements, reconciliation.

use super::*;

/// A People profile (not yet live): enough to connect Stripe for People payments.
async fn creative(app: &TestApp, cookie: &str, handle: &str) {
    let mut p = profile(handle, false);
    p["confirm_adult"] = json!(true);
    let res = app
        .send(req(Method::PUT, "/v1/me/creative").cookie(cookie).json(p))
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{:?}", body_json(res).await);
}

fn settings(on: bool, servers: bool, statement: Option<&str>) -> Value {
    json!({
        "donations_on": on, "donation_suggested_minor": 300, "donation_currency": "usd",
        "runs_servers": servers, "servers_statement": statement
    })
}

const STATEMENT: &str = "Pays for two game servers and the leaderboard.";

async fn put_settings(app: &TestApp, cookie: &str, slug: &str, body: Value) -> (StatusCode, Value) {
    let res = app
        .send(
            req(Method::PUT, format!("/v1/apps/{slug}/donations/settings"))
                .cookie(cookie)
                .json(body),
        )
        .await;
    let status = res.status();
    (status, body_json(res).await)
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn donation_settings_are_read_by_the_maker_alone(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (_, other) = app.user("Otto Other", "US").await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    listed_with(&app, maker_id, "arena", "released", "").await;
    let read = |cookie: String| {
        let app = &app;
        async move { get_json(app, &cookie, "/v1/apps/arena/donations/settings").await }
    };
    let s = read(maker.clone()).await;
    assert_eq!(s["account_status"], "none");
    assert_eq!(s["payments"], "simulated");
    assert_eq!(s["donations_on"], false);
    assert_eq!(s["active_server_donations"], 0);
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/arena/donations/settings"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    for cookie in [&other, &admin] {
        assert_eq!(
            app.send(req(Method::GET, "/v1/apps/arena/donations/settings").cookie(cookie))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    sqlx::query(
        "insert into donation_accounts (user_id, stripe_account_id, stripe_status, tax_notice_ack_at)
         values ($1, 'acct_mia', 'pending', now())",
    )
    .bind(maker_id)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(read(maker.clone()).await["account_status"], "pending");
    donations_on(&app, &maker, "arena", true).await;
    let s = read(maker.clone()).await;
    assert_eq!(s["account_status"], "active");
    assert_eq!(s["runs_servers"], true);
    assert_eq!(s["servers_statement"], STATEMENT);
    let (_, dee) = adult(&app, "Dee Donor", "US").await;
    let (status, _) = donate(
        &app,
        &dee,
        "arena",
        json!({ "amount_minor": 500, "monthly": true, "consent": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read(maker.clone()).await["active_server_donations"], 1);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn saving_donation_settings_checks_every_field(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (_, other) = app.user("Otto Other", "US").await;
    listed_with(&app, maker_id, "arena", "released", "").await;

    // Without an account, donations can't be turned on (other settings can be saved).
    let (status, _) = put_settings(&app, &maker, "arena", settings(true, false, None)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = put_settings(&app, &maker, "arena", settings(false, false, None)).await;
    assert_eq!(status, StatusCode::OK);
    let res = app
        .send(
            req(Method::POST, "/v1/me/donation-account")
                .cookie(&maker)
                .json(json!({ "tax_notice_ack": true, "confirm_adult": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);

    let field_refused = |body: Value, field: &'static str| {
        let app = &app;
        let maker = maker.clone();
        async move {
            let (status, res) = put_settings(app, &maker, "arena", body.clone()).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
            assert_eq!(res["error"]["field"], field, "{body}: {res}");
        }
    };
    for amount in [99, 50001] {
        let mut b = settings(true, false, None);
        b["donation_suggested_minor"] = json!(amount);
        field_refused(b, "donation_suggested_minor").await;
    }
    let mut b = settings(true, false, None);
    b["donation_currency"] = json!("jpy");
    field_refused(b, "donation_currency").await;
    for statement in [None, Some("Servers."), Some(&*"x".repeat(301))] {
        field_refused(settings(true, true, statement), "servers_statement").await;
    }
    // Spaces around it don't count.
    field_refused(
        settings(true, true, Some("   Servers.        ")),
        "servers_statement",
    )
    .await;
    let mut b = settings(true, false, None);
    b["payout_to"] = json!("acct_evil");
    let (status, _) = put_settings(&app, &maker, "arena", b).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "unknown fields");
    let (status, _) = put_settings(&app, &other, "arena", settings(true, false, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, saved) = put_settings(
        &app,
        &maker,
        "arena",
        json!({
            "donations_on": true, "donation_suggested_minor": 1000, "donation_currency": "gbp",
            "runs_servers": true, "servers_statement": format!("  {STATEMENT}  ")
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["servers_statement"], STATEMENT);
    assert_eq!(saved["donation_currency"], "gbp");
    assert_eq!(saved["donation_suggested_minor"], 1000);

    // Monthly donors: the servers can't just be switched off.
    let (_, dee) = adult(&app, "Dee Donor", "US").await;
    let (status, _) = donate(
        &app,
        &dee,
        "arena",
        json!({ "amount_minor": 500, "monthly": true, "consent": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, res) = put_settings(&app, &maker, "arena", settings(true, false, None)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        res["error"]["message"]
            .as_str()
            .unwrap()
            .contains("The servers are shut down"),
        "{res}"
    );

    // Shut down, then running servers again: the shut-down date goes.
    let res = app
        .send(req(Method::POST, "/v1/apps/arena/servers/shut-down").cookie(&maker))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let s = get_json(&app, &maker, "/v1/apps/arena/donations/settings").await;
    assert!(s["servers_shut_down_at"].is_string());
    assert_eq!(s["runs_servers"], false);
    let (status, s) =
        put_settings(&app, &maker, "arena", settings(true, true, Some(STATEMENT))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(s["servers_shut_down_at"].is_null(), "{s}");
    assert!(get_json(&app, &dee, "/v1/apps/arena/donate").await["servers"].is_object());

    // The settings say whether the app is private (no donations until it goes public), so the
    // Publish page offers to set donations up only where they can be.
    assert_eq!(s["private"], false);
    sqlx::query("update apps set visibility = 'private' where slug = 'arena'")
        .execute(&app.db)
        .await
        .unwrap();
    let s = get_json(&app, &maker, "/v1/apps/arena/donations/settings").await;
    assert_eq!(s["private"], true);
}

/// b39: a monthly donation still at Stripe Checkout becomes active when it's paid, so switching
/// the servers off must wait for it too.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn servers_cant_be_switched_off_while_a_monthly_donation_is_starting(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (_, maker) = maker_taking_donations(&app, "arena", true).await;
    let (dee_id, _) = adult(&app, "Dee Donor", "US").await;
    sqlx::query(
        "insert into server_donations (id, app_id, donor_id, stripe_account_id, currency, donation_minor,
                                       slop_fee_minor, total_minor, consent_text, status)
         select $1, id, $2, 'acct_x', 'usd', 500, 25, 546, 'terms', 'pending' from apps where slug = 'arena'",
    )
    .bind(Uuid::now_v7())
    .bind(dee_id)
    .execute(&app.db)
    .await
    .unwrap();
    let (status, _) = put_settings(&app, &maker, "arena", settings(true, false, None)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    // A checkout abandoned days ago will never complete (Stripe's sessions last a day).
    sqlx::query("update server_donations set created_at = now() - interval '2 days'")
        .execute(&app.db)
        .await
        .unwrap();
    let (status, _) = put_settings(&app, &maker, "arena", settings(true, false, None)).await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn shutting_the_servers_down_ends_every_server_donation_and_is_audited(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (_, maker) = maker_taking_donations(&app, "arena", true).await;
    let (_, other) = app.user("Otto Other", "US").await;
    let (_, dee) = adult(&app, "Dee Donor", "US").await;
    let (eve_id, _) = adult(&app, "Eve Donor", "US").await;
    donate(
        &app,
        &dee,
        "arena",
        json!({ "amount_minor": 500, "monthly": true, "consent": true }),
    )
    .await;
    // One still at checkout.
    sqlx::query(
        "insert into server_donations (id, app_id, donor_id, stripe_account_id, currency, donation_minor,
                                       slop_fee_minor, total_minor, consent_text, status)
         select $1, id, $2, 'acct_x', 'usd', 500, 25, 546, 'terms', 'pending' from apps where slug = 'arena'",
    )
    .bind(Uuid::now_v7())
    .bind(eve_id)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(
        app.send(req(Method::POST, "/v1/apps/arena/servers/shut-down").cookie(&other))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let res = app
        .send(req(Method::POST, "/v1/apps/arena/servers/shut-down").cookie(&maker))
        .await;
    assert_eq!(body_json(res).await["cancelled"], 2);
    let by: Vec<(String, String)> =
        sqlx::query_as("select status, cancelled_by from server_donations")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(
        by,
        vec![("cancelled".to_string(), "servers_shut_down".to_string()); 2]
    );
    let (actor, subject, detail): (String, String, Value) = sqlx::query_as(
        "select actor, subject, detail from audit_log where action = 'donations.servers_shut_down'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        (actor.as_str(), subject.as_str(), detail),
        ("Mia Maker", "arena", json!({ "cancelled": 2 }))
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn stripe_subscriptions_end_at_stripe_before_the_servers_are_marked_shut_down(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (_, maker) = maker_taking_donations(&app, "arena", true).await;
    let acct: String = sqlx::query_scalar("select stripe_account_id from donation_accounts")
        .fetch_one(&app.db)
        .await
        .unwrap();
    for (name, sub) in [("Dee Donor", "sub_dee"), ("Eve Donor", "sub_eve")] {
        let (_, cookie) = adult(&app, name, "US").await;
        let (_, started) = donate(
            &app,
            &cookie,
            "arena",
            json!({ "amount_minor": 500, "monthly": true, "consent": true }),
        )
        .await;
        let session = started["url"].as_str().unwrap().rsplit('/').next().unwrap();
        fake.patch_session(
            session,
            json!({ "status": "complete", "subscription": sub }),
        );
        let id = started["id"].as_str().unwrap();
        app.send(req(Method::POST, format!("/v1/me/donations/{id}/check")).cookie(&cookie))
            .await;
    }
    assert_eq!(
        count(
            &app,
            "select count(*) from server_donations where status = 'active'"
        )
        .await,
        2
    );
    fake.fail("DELETE", "v1/subscriptions/sub_eve", 500);
    let res = app
        .send(req(Method::POST, "/v1/apps/arena/servers/shut-down").cookie(&maker))
        .await;
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        count(
            &app,
            "select count(*) from server_donations where status = 'active'"
        )
        .await,
        2,
        "nothing changed here"
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from apps where runs_servers and servers_shut_down_at is null"
        )
        .await,
        1
    );
    fake.heal();
    let res = app
        .send(req(Method::POST, "/v1/apps/arena/servers/shut-down").cookie(&maker))
        .await;
    assert_eq!(body_json(res).await["cancelled"], 2);
    for sub in ["sub_dee", "sub_eve"] {
        let calls = fake.calls("DELETE", &format!("v1/subscriptions/{sub}"));
        assert!(!calls.is_empty(), "{sub}");
        assert!(
            calls
                .iter()
                .all(|c| c.account.as_deref() == Some(acct.as_str()))
        );
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn my_donation_account_shows_its_status_and_any_change_waiting(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let mine = || {
        let app = &app;
        let maker = maker.clone();
        async move { get_json(app, &maker, "/v1/me/donation-account").await }
    };
    let a = mine().await;
    assert_eq!(
        (a["status"].as_str(), a["payments"].as_str()),
        (Some("none"), Some("stripe"))
    );
    assert!(a["change"].is_null());
    assert_eq!(
        app.send(req(Method::GET, "/v1/me/donation-account"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let res = app
        .send(
            req(Method::POST, "/v1/me/donation-account")
                .cookie(&maker)
                .json(json!({ "tax_notice_ack": true, "confirm_adult": true })),
        )
        .await;
    assert_eq!(body_json(res).await["status"], "active");
    assert_eq!(mine().await["status"], "active");

    // A switch waits on the new account's sign-up.
    fake.with(|s| s.new_account_status = Some("pending"));
    let res = app
        .send(
            req(Method::POST, "/v1/me/donation-account/change")
                .cookie(&maker)
                .json(json!({ "kind": "switch" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let a = mine().await;
    assert_eq!(a["change"]["kind"], "switch");
    assert_eq!(a["change"]["new_account_ready"], false);
    let new_account: String =
        sqlx::query_scalar("select new_account_id from stripe_account_changes where user_id = $1")
            .bind(maker_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    fake.set_account(&new_account, "active");
    let status = hook(
        &app,
        event(
            "evt_acct",
            "account.updated",
            Some(&new_account),
            json!({ "id": new_account }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mine().await["change"]["new_account_ready"], true);
    // A disconnection has no new account to wait for.
    app.send(req(Method::POST, "/v1/me/donation-account/change/cancel").cookie(&maker))
        .await;
    app.send(
        req(Method::POST, "/v1/me/donation-account/change")
            .cookie(&maker)
            .json(json!({ "kind": "disconnect" })),
    )
    .await;
    let a = mine().await;
    assert_eq!(a["change"]["kind"], "disconnect");
    assert!(a["change"]["new_account_ready"].is_null());
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn connecting_a_stripe_account_for_donations_needs_an_adult_a_fresh_sign_in_and_a_passkey(
    db: PgPool,
) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    fake.with(|s| s.new_account_status = Some("pending"));
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let connect = |body: Value| {
        req(Method::POST, "/v1/me/donation-account")
            .cookie(&maker)
            .json(body)
    };
    let res = app.send(connect(json!({ "tax_notice_ack": true }))).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "confirm_adult");

    // The first connection is a step-up action.
    old_sign_in(&app, maker_id).await;
    let ok =
        json!({ "tax_notice_ack": true, "confirm_adult": true, "return_to": "//evil.example/x" });
    let res = app.send(connect(ok.clone())).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(res).await["error"]["code"], "step_up_required");
    sqlx::query("update sessions set created_at = now() where user_id = $1")
        .bind(maker_id)
        .execute(&app.db)
        .await
        .unwrap();
    // Someone with a passkey confirms with it.
    passkey_holder(&app, maker_id).await;
    let res = app.send(connect(ok.clone())).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(res).await["error"]["code"], "passkey_required");
    assert!(fake.calls("POST", "v1/accounts").is_empty());
    passkey_confirmed(&app, maker_id).await;

    // A new account still signing up: Stripe's sign-up link, coming back to Slop only.
    let res = app.send(connect(ok)).await;
    let on = body_json(res).await;
    assert_eq!(on["status"], "pending", "{on}");
    let created = fake.calls("POST", "v1/accounts");
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].form["controller[losses][payments]"], "stripe");
    assert_eq!(created[0].form["metadata[slop_user]"], maker_id.to_string());
    let acct: String = sqlx::query_scalar("select stripe_account_id from donation_accounts")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(
        on["url"],
        format!("https://connect.stripe.example/setup/{acct}")
    );
    let link = fake.calls("POST", "v1/account_links");
    assert_eq!(link[0].form["return_url"], "http://localhost:8080/publish");
    assert_eq!(link[0].form["refresh_url"], "http://localhost:8080/publish");
    // Finishing it later reuses the account and needs no fresh sign-in.
    old_sign_in(&app, maker_id).await;
    passkey_confirmed(&app, maker_id).await;
    for (back, want) in [
        ("https://evil.example", "http://localhost:8080/publish"),
        (
            "/publish/arena/manage#donations",
            "http://localhost:8080/publish/arena/manage#donations",
        ),
    ] {
        let res = app
            .send(connect(
                json!({ "tax_notice_ack": true, "return_to": back }),
            ))
            .await;
        assert_eq!(res.status(), StatusCode::OK, "{back}");
        let link = fake.calls("POST", "v1/account_links");
        assert_eq!(link.last().unwrap().form["return_url"], want);
    }
    assert_eq!(fake.calls("POST", "v1/accounts").len(), 1);
    assert_eq!(
        count(
            &app,
            "select count(*) from donation_accounts where stripe_status = 'pending'"
        )
        .await,
        1
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_creatives_stripe_account_is_reused_for_donations(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (vera_id, vera) = app.user("Vera", "US").await;
    creative(&app, &vera, "vera").await;
    let res = app
        .send(req(Method::POST, "/v1/me/creative/stripe").cookie(&vera))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let people: String =
        sqlx::query_scalar("select stripe_account_id from creatives where user_id = $1")
            .bind(vera_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    // An account already connected needs no fresh sign-in either.
    old_sign_in(&app, vera_id).await;
    let res = app
        .send(
            req(Method::POST, "/v1/me/donation-account")
                .cookie(&vera)
                .json(json!({ "tax_notice_ack": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{:?}", body_json(res).await);
    let donations: String =
        sqlx::query_scalar("select stripe_account_id from donation_accounts where user_id = $1")
            .bind(vera_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(donations, people);
    assert_eq!(fake.calls("POST", "v1/accounts").len(), 1, "one account");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn refreshing_an_account_reads_it_from_stripe_everywhere_it_is_used(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (_, vera) = app.user("Vera", "US").await;
    let refresh = || req(Method::POST, "/v1/me/donation-account/refresh").cookie(&vera);
    assert_eq!(app.send(refresh()).await.status(), StatusCode::NOT_FOUND);
    creative(&app, &vera, "vera").await;
    app.send(req(Method::POST, "/v1/me/creative/stripe").cookie(&vera))
        .await;
    app.send(
        req(Method::POST, "/v1/me/donation-account")
            .cookie(&vera)
            .json(json!({ "tax_notice_ack": true })),
    )
    .await;
    let acct: String = sqlx::query_scalar("select stripe_account_id from donation_accounts")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let statuses = || {
        let app = &app;
        async move {
            sqlx::query_as::<_, (String, String)>(
                "select (select stripe_status from donation_accounts), (select stripe_status from creatives)",
            )
            .fetch_one(&app.db)
            .await
            .unwrap()
        }
    };
    for status in ["restricted", "pending", "active"] {
        fake.set_account(&acct, status);
        let res = body_json(app.send(refresh()).await).await;
        assert_eq!(res["status"], status);
        assert_eq!(statuses().await, (status.to_string(), status.to_string()));
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn without_stripe_refreshing_returns_the_stored_status(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (_, maker) = app.user("Mia Maker", "US").await;
    app.send(
        req(Method::POST, "/v1/me/donation-account")
            .cookie(&maker)
            .json(json!({ "tax_notice_ack": true, "confirm_adult": true })),
    )
    .await;
    sqlx::query("update donation_accounts set stripe_status = 'restricted'")
        .execute(&app.db)
        .await
        .unwrap();
    let res = body_json(
        app.send(req(Method::POST, "/v1/me/donation-account/refresh").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(res["status"], "restricted");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn creatives_connect_stripe_for_people_payments(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (vera_id, vera) = app.user("Vera", "US").await;
    let connect = || req(Method::POST, "/v1/me/creative/stripe").cookie(&vera);
    let refresh = || req(Method::POST, "/v1/me/creative/stripe/refresh").cookie(&vera);
    assert_eq!(
        app.send(connect()).await.status(),
        StatusCode::NOT_FOUND,
        "no profile"
    );
    assert_eq!(app.send(refresh()).await.status(), StatusCode::NOT_FOUND);
    creative(&app, &vera, "vera").await;
    // No account yet: the stored status.
    let res = body_json(app.send(refresh()).await).await;
    assert_eq!(res["status"], "none", "{res}");
    passkey_holder(&app, vera_id).await;
    let res = app.send(connect()).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(res).await["error"]["code"], "passkey_required");
    passkey_confirmed(&app, vera_id).await;
    fake.with(|s| s.new_account_status = Some("pending"));
    let on = body_json(app.send(connect()).await).await;
    assert_eq!(on["status"], "pending");
    let acct: String = sqlx::query_scalar("select stripe_account_id from creatives")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(
        on["url"],
        format!("https://connect.stripe.example/setup/{acct}")
    );
    // Pressed again: the same account, a new sign-up link.
    let again = body_json(app.send(connect()).await).await;
    assert_eq!(again["url"], on["url"]);
    assert_eq!(fake.calls("POST", "v1/accounts").len(), 1);
    for status in ["restricted", "pending", "active"] {
        fake.set_account(&acct, status);
        assert_eq!(body_json(app.send(refresh()).await).await["status"], status);
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn with_payments_off_creatives_cant_connect_stripe(db: PgPool) {
    let app = test_app(db).await;
    let (_, vera) = app.user("Vera", "US").await;
    creative(&app, &vera, "vera").await;
    assert_eq!(
        app.send(req(Method::POST, "/v1/me/creative/stripe").cookie(&vera))
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn asking_to_change_where_donations_go_is_checked_announced_and_audited(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app_with(db, &fake, with_email).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    with_address(&app, maker_id, "mia@example.com").await;
    sqlx::query("update users set email_prefs = '{\"donations\": false}' where id = $1")
        .bind(maker_id)
        .execute(&app.db)
        .await
        .unwrap();
    let change = |body: Value| {
        req(Method::POST, "/v1/me/donation-account/change")
            .cookie(&maker)
            .json(body)
    };
    let res = app.send(change(json!({ "kind": "steal" }))).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "kind");
    assert_eq!(
        app.send(change(json!({ "kind": "switch" }))).await.status(),
        StatusCode::CONFLICT,
        "nothing to change"
    );
    app.send(
        req(Method::POST, "/v1/me/donation-account")
            .cookie(&maker)
            .json(json!({ "tax_notice_ack": true, "confirm_adult": true })),
    )
    .await;
    passkey_holder(&app, maker_id).await;
    let res = app.send(change(json!({ "kind": "switch" }))).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(res).await["error"]["code"], "passkey_required");
    passkey_confirmed(&app, maker_id).await;

    fake.with(|s| s.new_account_status = Some("pending"));
    let res = app
        .send(change(
            json!({ "kind": "switch", "return_to": "//evil.example" }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let answer = body_json(res).await;
    assert_eq!(answer["status"], "waiting");
    let (new_account, new_status): (String, String) =
        sqlx::query_as("select new_account_id, new_account_status from stripe_account_changes")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(new_status, "pending");
    assert_eq!(
        answer["url"],
        format!("https://connect.stripe.example/setup/{new_account}")
    );
    assert_eq!(fake.calls("POST", "v1/accounts").len(), 2);
    let link = fake.calls("POST", "v1/account_links");
    assert_eq!(
        link.last().unwrap().form["return_url"],
        "http://localhost:8080/account"
    );
    // The email goes out even with donation emails switched off: it might not have been them.
    assert_eq!(
        count(
            &app,
            "select count(*) from email_outbox where to_address = 'mia@example.com'
             and subject = 'Your donations will be paid to a different Stripe account'"
        )
        .await,
        1
    );
    assert_eq!(
        audit_actions(&app, "donations.account_change").await,
        ["donations.account_change_requested"]
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn cancelling_an_account_change_needs_one_waiting(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (_, maker) = maker_taking_donations(&app, "arena", false).await;
    let cancel = || req(Method::POST, "/v1/me/donation-account/change/cancel").cookie(&maker);
    assert_eq!(app.send(cancel()).await.status(), StatusCode::NOT_FOUND);
    app.send(
        req(Method::POST, "/v1/me/donation-account/change")
            .cookie(&maker)
            .json(json!({ "kind": "switch" })),
    )
    .await;
    assert_eq!(app.send(cancel()).await.status(), StatusCode::NO_CONTENT);
    assert_eq!(app.send(cancel()).await.status(), StatusCode::NOT_FOUND);
    // Once it has happened, it can't be cancelled.
    app.send(
        req(Method::POST, "/v1/me/donation-account/change")
            .cookie(&maker)
            .json(json!({ "kind": "switch" })),
    )
    .await;
    sqlx::query(
        "update stripe_account_changes set effective_at = now() where cancelled_at is null",
    )
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(
        slop_api::donations::apply_account_changes(&app.state)
            .await
            .unwrap(),
        1
    );
    assert_eq!(app.send(cancel()).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        audit_actions(&app, "donations.account_change").await,
        [
            "donations.account_change_requested",
            "donations.account_change_cancelled",
            "donations.account_change_requested",
            "donations.account_change_applied"
        ]
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_switch_waits_for_the_new_account_and_rereads_it_from_stripe(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (_, maker) = maker_taking_donations(&app, "arena", false).await;
    let old: String = sqlx::query_scalar("select stripe_account_id from donation_accounts")
        .fetch_one(&app.db)
        .await
        .unwrap();
    fake.with(|s| s.new_account_status = Some("pending"));
    app.send(
        req(Method::POST, "/v1/me/donation-account/change")
            .cookie(&maker)
            .json(json!({ "kind": "switch" })),
    )
    .await;
    let new: String = sqlx::query_scalar("select new_account_id from stripe_account_changes")
        .fetch_one(&app.db)
        .await
        .unwrap();
    sqlx::query("update stripe_account_changes set effective_at = now() - interval '1 day'")
        .execute(&app.db)
        .await
        .unwrap();
    // Past 72 hours, but the new account isn't signed up: still the old one, checked at Stripe.
    let apply = || slop_api::donations::apply_account_changes(&app.state);
    assert_eq!(apply().await.unwrap(), 0);
    assert_eq!(
        fake.calls("GET", &format!("v1/accounts/{new}")).len(),
        1,
        "asked Stripe in case a webhook went missing"
    );
    let current: String = sqlx::query_scalar("select stripe_account_id from donation_accounts")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(current, old);
    // Signed up (and the webhook lost): the next run switches.
    fake.set_account(&new, "active");
    assert_eq!(apply().await.unwrap(), 1);
    let (current, status): (String, String) =
        sqlx::query_as("select stripe_account_id, stripe_status from donation_accounts")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((current, status.as_str()), (new, "active"));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn disconnecting_ends_subscriptions_at_stripe_first_then_tells_the_maker(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app_with(db, &fake, with_email).await;
    let (maker_id, maker) = maker_taking_donations(&app, "arena", true).await;
    with_address(&app, maker_id, "mia@example.com").await;
    let acct: String = sqlx::query_scalar("select stripe_account_id from donation_accounts")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let (_, dee) = adult(&app, "Dee Donor", "US").await;
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
    app.send(
        req(Method::POST, "/v1/me/donation-account/change")
            .cookie(&maker)
            .json(json!({ "kind": "disconnect" })),
    )
    .await;
    sqlx::query("update stripe_account_changes set effective_at = now()")
        .execute(&app.db)
        .await
        .unwrap();

    fake.fail("DELETE", "v1/subscriptions", 500);
    assert!(
        slop_api::donations::apply_account_changes(&app.state)
            .await
            .is_err()
    );
    assert_eq!(
        count(&app, "select count(*) from donation_accounts").await,
        1
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from stripe_account_changes where applied_at is null"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from server_donations where status = 'active'"
        )
        .await,
        1
    );
    fake.heal();
    assert_eq!(
        slop_api::donations::apply_account_changes(&app.state)
            .await
            .unwrap(),
        1
    );
    let deleted = fake.calls("DELETE", "v1/subscriptions/sub_dee");
    assert_eq!(
        deleted.last().unwrap().account.as_deref(),
        Some(acct.as_str())
    );
    assert_eq!(
        count(&app, "select count(*) from donation_accounts").await,
        0
    );
    assert_eq!(
        count(&app, "select count(*) from apps where donations_on").await,
        0
    );
    assert!(
        notices(&app, maker_id)
            .await
            .contains(&"Your Stripe account is disconnected".to_string())
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from email_outbox where to_address = 'mia@example.com'
             and subject = 'Your Stripe account is disconnected'"
        )
        .await,
        1
    );
    let detail: Value = sqlx::query_scalar(
        "select detail from audit_log where action = 'donations.account_change_applied'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(detail, json!({ "kind": "disconnect" }));
    assert_eq!(
        get_json(&app, &maker, "/v1/me/donation-account").await["status"],
        "none"
    );
}

/// A delivery claimed before a switch is paid where it said, to the old account (still the
/// maker's own).
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_bounty_claimed_before_a_switch_pays_the_account_it_was_claimed_with(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (maker_id, maker) = maker_taking_donations(&app, "arena", false).await;
    approved_release(&app, "arena", maker_id).await;
    let old: String = sqlx::query_scalar("select stripe_account_id from donation_accounts")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let (_, ann) = adult(&app, "Ann Backer", "US").await;
    let bounty = body_json(
        app.send(
            req(Method::POST, "/v1/apps/arena/bounties")
                .cookie(&ann)
                .json(json!({ "title": "Fix saves", "body": "They vanish.", "currency": "usd" })),
        )
        .await,
    )
    .await;
    let id = bounty["id"].as_str().unwrap();
    app.send(
        req(Method::POST, format!("/v1/bounties/{id}/pledge"))
            .cookie(&ann)
            .json(json!({ "amount_minor": 1000 })),
    )
    .await;
    let res = app
        .send(
            req(Method::POST, format!("/v1/bounties/{id}/claims"))
                .cookie(&maker)
                .json(json!({ "app_slug": "arena", "note": "Saves are safe in 1.0.0." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    app.send(
        req(Method::POST, "/v1/me/donation-account/change")
            .cookie(&maker)
            .json(json!({ "kind": "switch" })),
    )
    .await;
    sqlx::query("update stripe_account_changes set effective_at = now()")
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::donations::apply_account_changes(&app.state)
        .await
        .unwrap();
    let new: String = sqlx::query_scalar("select stripe_account_id from donation_accounts")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_ne!(new, old);
    let claim: Uuid = sqlx::query_scalar("select id from bounty_claims")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/bounties/{id}/claims/{claim}/vote"),
            )
            .cookie(&ann)
            .json(json!({ "accept": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(slop_api::bounties::charge_due(&app.state).await.unwrap(), 1);
    let paid_to: String =
        sqlx::query_scalar("select stripe_account_id from payments where kind = 'bounty'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(paid_to, old);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn vat_numbers_are_cleaned_checked_and_can_be_removed(db: PgPool) {
    let app = test_app_with(db, |c| {
        simulated(c);
        // The fake GitHub server also plays VIES.
        c.payments.vies_url = Some(c.doh_url.join("vies").unwrap());
    })
    .await;
    let (de_id, de) = app.user("Dora DE", "DE").await;
    let (_, gr) = app.user("Gia GR", "GR").await;
    let (_, gb) = app.user("Gus GB", "GB").await;
    let vat = |cookie: &str, number: Value| {
        req(Method::PUT, "/v1/me/vat")
            .cookie(cookie)
            .json(json!({ "vat_number": number }))
    };
    for bad in [
        "D1",
        "1DE2345678",
        "DE12345678901234",
        "DE123_456",
        "D3123456",
    ] {
        let res = app.send(vat(&de, json!(bad))).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
        assert_eq!(body_json(res).await["error"]["field"], "vat_number");
    }
    let saved = body_json(app.send(vat(&de, json!("de-123.456.789"))).await).await;
    assert_eq!(saved["vat_number"], "DE123456789");
    assert_eq!(saved["checked"], true);
    assert!(saved["note"].is_null());
    // Greek numbers start with EL; VIES says this one isn't valid.
    let saved = body_json(app.send(vat(&gr, json!("EL 094 259 216"))).await).await;
    assert_eq!(saved["vat_number"], "EL094259216");
    assert_eq!(saved["checked"], false);
    assert!(saved["note"].as_str().unwrap().contains("isn't valid"));
    // UK numbers can't be checked automatically.
    let saved = body_json(app.send(vat(&gb, json!("GB123456789"))).await).await;
    assert_eq!(saved["checked"], false);
    assert!(saved["note"].as_str().unwrap().contains("UK VAT"));

    // Removing it: null or empty.
    for empty in [Value::Null, json!(""), json!("  ")] {
        app.send(vat(&de, json!("DE123456789"))).await;
        let saved = body_json(app.send(vat(&de, empty.clone())).await).await;
        assert!(saved["vat_number"].is_null(), "{empty}");
        let (number, checked): (Option<String>, Option<OffsetDateTime>) =
            sqlx::query_as("select vat_number, vat_checked_at from users where id = $1")
                .bind(de_id)
                .fetch_one(&app.db)
                .await
                .unwrap();
        assert_eq!((number, checked), (None, None));
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn when_vies_doesnt_answer_the_number_is_saved_unchecked(db: PgPool) {
    let app = test_app_with(db, |c| {
        simulated(c);
        // Nothing listens here.
        c.payments.vies_url = Some("http://127.0.0.1:9/vies".parse().unwrap());
    })
    .await;
    let (_, de) = app.user("Dora DE", "DE").await;
    let saved = body_json(
        app.send(
            req(Method::PUT, "/v1/me/vat")
                .cookie(&de)
                .json(json!({ "vat_number": "DE123456789" })),
        )
        .await,
    )
    .await;
    assert_eq!(saved["vat_number"], "DE123456789");
    assert_eq!(saved["checked"], false);
    assert!(
        saved["note"].as_str().unwrap().contains("didn't answer"),
        "{saved}"
    );
}

/// b32: the first two characters were sliced as bytes before checking they're letters.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_vat_number_starting_with_a_symbol_is_refused_not_a_crash(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (_, de) = app.user("Dora DE", "DE").await;
    for number in ["€1234567", "Ü12345678", "D€12345678"] {
        let res = app
            .send(
                req(Method::PUT, "/v1/me/vat")
                    .cookie(&de)
                    .json(json!({ "vat_number": number })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{number}");
        assert_eq!(body_json(res).await["error"]["field"], "vat_number");
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn fee_statements_cover_people_jobs_and_only_the_users_own_accounts(db: PgPool) {
    let app = test_app_with(db, |c| {
        simulated(c);
        c.payments.vat_id = Some("EU372000041".into());
    })
    .await;
    let (vera_id, vera) = live_creative(&app, "vera", "US", false).await;
    let (otto_id, _) = live_creative(&app, "otto", "US", false).await;
    for (id, amount) in [(vera_id, 12000), (otto_id, 99999)] {
        sqlx::query(
            "insert into payments (id, kind, stripe_account_id, stripe_object_id, amount_minor, slop_fee_minor, currency, simulated)
             select $1, 'people_job', stripe_account_id, $2, $3, $3 / 20, 'usd', true from creatives where user_id = $4",
        )
        .bind(Uuid::now_v7())
        .bind(format!("sim_{}", Uuid::now_v7().simple()))
        .bind(amount)
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
    }
    let s = get_json(&app, &vera, "/v1/me/fee-statements").await;
    let rows = s["statements"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{s}");
    assert_eq!(rows[0]["kind"], "people_job");
    assert_eq!(rows[0]["received_minor"], 12000);
    assert_eq!(rows[0]["slop_fee_minor"], 600);
    assert_eq!(s["vat"]["slop_vat_id"], "EU372000041");
    assert_eq!(
        app.send(req(Method::GET, "/v1/me/fee-statements"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

/// b30: Slop's fee on bounty payouts is a fee like any other: in the builder's statements, and in
/// the daily comparison with Stripe's application fees.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn bounty_fees_are_in_statements_and_reconciliation(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (builder_id, builder) = app.user("Bea Builder", "US").await;
    sqlx::query(
        "insert into donation_accounts (user_id, stripe_account_id, stripe_status, tax_notice_ack_at)
         values ($1, 'acct_bea', 'active', now())",
    )
    .bind(builder_id)
    .execute(&app.db)
    .await
    .unwrap();
    let yesterday =
        OffsetDateTime::now_utc().replace_time(time::Time::MIDNIGHT) - time::Duration::hours(12);
    sqlx::query(
        "insert into payments (id, kind, stripe_account_id, stripe_object_id, amount_minor, slop_fee_minor, currency, created_at)
         values ($1, 'bounty', 'acct_bea', 'pi_bounty', 2000, 100, 'usd', $2)",
    )
    .bind(Uuid::now_v7())
    .bind(yesterday)
    .execute(&app.db)
    .await
    .unwrap();
    let s = get_json(&app, &builder, "/v1/me/fee-statements").await;
    assert_eq!(s["statements"][0]["kind"], "bounty", "{s}");
    assert_eq!(s["statements"][0]["slop_fee_minor"], 100);
    fake.with(|s| {
        s.application_fees = vec![
            json!({ "id": "fee_1", "account": "acct_bea", "amount": 100, "amount_refunded": 0 }),
        ]
    });
    assert_eq!(slop_api::donations::reconcile(&app.state).await.unwrap(), 0);
    assert!(audit_actions(&app, "payments.mismatch").await.is_empty());
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn reconciliation_pages_through_stripes_fees_and_records_mismatches(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let midnight = OffsetDateTime::now_utc().replace_time(time::Time::MIDNIGHT);
    let yesterday = midnight - time::Duration::hours(12);
    let pay = |account: &'static str, fee_minor: i32, simulated: bool, at: OffsetDateTime| {
        let app = &app;
        async move {
            sqlx::query(
                "insert into payments (id, kind, stripe_account_id, stripe_object_id, amount_minor, slop_fee_minor,
                                       currency, simulated, created_at)
                 values ($1, 'donation', $2, $3, $4 * 20, $4, 'usd', $5, $6)",
            )
            .bind(Uuid::now_v7())
            .bind(account)
            .bind(format!("ch_{}", Uuid::now_v7().simple()))
            .bind(fee_minor)
            .bind(simulated)
            .bind(at)
            .execute(&app.db)
            .await
            .unwrap();
        }
    };
    pay("acct_a", 15, false, yesterday).await;
    pay("acct_a", 25, false, yesterday).await;
    // Simulated payments never reached Stripe; today's are tomorrow's business.
    pay("acct_b", 5, true, yesterday).await;
    pay("acct_c", 9, false, OffsetDateTime::now_utc()).await;
    fake.with(|s| {
        s.application_fees = vec![
            json!({ "id": "fee_1", "account": "acct_a", "amount": 15, "amount_refunded": 0 }),
            json!({ "id": "fee_2", "account": "acct_a", "amount": 30, "amount_refunded": 5 }),
            json!({ "id": "fee_3", "account": "acct_b", "amount": 5, "amount_refunded": 0 }),
        ]
    });
    assert_eq!(slop_api::donations::reconcile(&app.state).await.unwrap(), 1);
    let pages = fake.calls("GET", "v1/application_fees");
    assert_eq!(pages.len(), 2);
    assert!(!pages[0].form.contains_key("starting_after"));
    assert_eq!(pages[1].form["starting_after"], "fee_2");
    assert_eq!(
        pages[0].form["created[lt]"],
        midnight.unix_timestamp().to_string()
    );
    assert_eq!(
        pages[0].form["created[gte]"],
        (midnight - time::Duration::days(1))
            .unix_timestamp()
            .to_string()
    );
    let (actor, detail): (String, Value) =
        sqlx::query_as("select actor, detail from audit_log where action = 'payments.mismatch'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(actor, "reconciliation");
    assert_eq!(detail, json!({ "accounts": [["acct_b", 5, 0]] }));

    // Stripe not answering: an error, for the job loop to try again.
    fake.fail("GET", "v1/application_fees", 500);
    assert!(slop_api::donations::reconcile(&app.state).await.is_err());
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn without_stripe_there_is_nothing_to_reconcile(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    assert_eq!(slop_api::donations::reconcile(&app.state).await.unwrap(), 0);
}
