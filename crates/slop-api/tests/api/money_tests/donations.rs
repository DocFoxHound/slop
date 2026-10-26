// SPDX-License-Identifier: AGPL-3.0-or-later
//! Donations from the donor's side: the donate box, quotes, starting one-off, monthly and shared
//! donations, checking and cancelling them, the maker's summary and refunds, reminders, limits.

use super::*;
use slop_api::limits::Action;

/// A donation row made directly, as a payment or checkout left behind would be.
async fn insert_donation(app: &TestApp, donor: Uuid, slug: &str, kind: &str, status: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into donations (id, kind, app_id, donor_id, stripe_account_id, currency, donation_minor,
                                slop_fee_minor, processing_minor, total_minor, donor_covers_fees, status, paid_at)
         select $1, $2, id, $3, 'acct_x', 'usd', 300, 15, 40, 340, true, $4,
                case when $4 = 'paid' then now() end
         from apps where slug = $5",
    )
    .bind(id)
    .bind(kind)
    .bind(donor)
    .bind(status)
    .bind(slug)
    .execute(&app.db)
    .await
    .unwrap();
    id
}

/// A server donation row made directly.
async fn insert_server_donation(app: &TestApp, donor: Uuid, slug: &str, status: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into server_donations (id, app_id, donor_id, stripe_account_id, currency, donation_minor,
                                       slop_fee_minor, total_minor, consent_text, status, simulated, started_at,
                                       next_charge_at)
         select $1, id, $2, 'acct_x', 'usd', 500, 25, 546, 'terms', $3, true,
                case when $3 = 'active' then now() end, case when $3 = 'active' then now() + interval '30 days' end
         from apps where slug = $4",
    )
    .bind(id)
    .bind(donor)
    .bind(status)
    .bind(slug)
    .execute(&app.db)
    .await
    .unwrap();
    id
}

async fn place_hold(app: &TestApp, slug: &str, kind: &str) {
    sqlx::query(
        "insert into donation_holds (id, app_id, kind, reason, placed_by)
         select $1, id, $2, 'Checking a complaint.', 'test' from apps where slug = $3",
    )
    .bind(Uuid::now_v7())
    .bind(kind)
    .bind(slug)
    .execute(&app.db)
    .await
    .unwrap();
}

async fn lift_holds(app: &TestApp) {
    sqlx::query("update donation_holds set lifted_at = now(), lifted_by = 'test'")
        .execute(&app.db)
        .await
        .unwrap();
}

/// A donation account in this state for a user, without Stripe.
async fn account_row(app: &TestApp, user: Uuid, status: &str) {
    sqlx::query(
        "insert into donation_accounts (user_id, stripe_account_id, stripe_status, tax_notice_ack_at)
         values ($1, 'acct_' || replace($1::text, '-', ''), $2, now())
         on conflict (user_id) do update set stripe_status = $2",
    )
    .bind(user)
    .bind(status)
    .execute(&app.db)
    .await
    .unwrap();
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn with_payments_off_the_donate_box_says_so_and_nothing_starts(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (_, donor) = adult(&app, "Dee Donor", "US").await;
    listed_with(&app, maker_id, "arena", "released", "donations_on = true").await;
    account_row(&app, maker_id, "active").await;

    let info = body_json(app.send(req(Method::GET, "/v1/apps/arena/donate")).await).await;
    assert_eq!(info["on"], false);
    assert!(
        info["off_reason"]
            .as_str()
            .unwrap()
            .contains("aren't switched on"),
        "{info}"
    );
    assert_eq!(info["cant"], "Sign in to donate.");
    assert_eq!(info["simulated"], false);
    let (status, _) = donate(&app, &donor, "arena", json!({ "amount_minor": 300 })).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    // Nor can a maker connect an account.
    let res = app
        .send(
            req(Method::POST, "/v1/me/donation-account")
                .cookie(&maker)
                .json(json!({ "tax_notice_ack": true, "confirm_adult": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(count(&app, "select count(*) from donations").await, 0);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_donate_box_says_why_donations_are_off_and_follows_the_store_rules(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    let (_, donor) = app.user("Dee Donor", "US").await;
    let (_, brit) = app.user("Kit UK", "GB").await;
    listed_with(&app, maker_id, "arena", "released", "").await;
    let reason = |cookie: String| {
        let app = &app;
        async move {
            let info = get_json(app, &cookie, "/v1/apps/arena/donate").await;
            (info["on"].as_bool().unwrap(), info["off_reason"].clone())
        }
    };
    assert_eq!(
        reason(donor.clone()).await,
        (false, json!("The maker hasn't turned on donations."))
    );
    sqlx::query("update apps set donations_on = true where slug = 'arena'")
        .execute(&app.db)
        .await
        .unwrap();
    let not_ready = json!("The maker's Stripe account isn't ready to receive donations yet.");
    assert_eq!(reason(donor.clone()).await, (false, not_ready.clone()));
    account_row(&app, maker_id, "pending").await;
    assert_eq!(reason(donor.clone()).await, (false, not_ready.clone()));
    account_row(&app, maker_id, "restricted").await;
    assert_eq!(reason(donor.clone()).await, (false, not_ready));
    account_row(&app, maker_id, "active").await;
    assert_eq!(reason(donor.clone()).await, (true, Value::Null));
    let info = body_json(app.send(req(Method::GET, "/v1/apps/arena/donate")).await).await;
    assert_eq!(info["cant"], "Sign in to donate.");
    assert!(get_json(&app, &donor, "/v1/apps/arena/donate").await["cant"].is_null());

    // Presets in the app's currency, each with the total the donor pays first.
    for (currency, total) in [("eur", 541), ("gbp", 536)] {
        sqlx::query(
            "update apps set donation_currency = $1, donation_suggested_minor = 500 where slug = 'arena'",
        )
        .bind(currency)
        .execute(&app.db)
        .await
        .unwrap();
        let info = get_json(&app, &donor, "/v1/apps/arena/donate").await;
        assert_eq!(info["currency"], currency);
        assert_eq!(info["presets"][1]["donation"], 500);
        assert_eq!(info["presets"][1]["total"], total, "{currency}: {info}");
        assert_eq!(info["presets"][1]["slop_fee"], 25);
    }

    // The store's rules decide who can see it at all.
    for set in [
        "status = 'delisted'",
        "visibility = 'private'",
        "status = 'draft'",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "update apps set {set} where slug = 'arena'"
        )))
        .execute(&app.db)
        .await
        .unwrap();
        assert_eq!(
            app.send(req(Method::GET, "/v1/apps/arena/donate").cookie(&donor))
                .await
                .status(),
            StatusCode::NOT_FOUND,
            "{set}"
        );
        sqlx::query(
            "update apps set status = 'listed', visibility = 'public' where slug = 'arena'",
        )
        .execute(&app.db)
        .await
        .unwrap();
    }
    sqlx::query("update apps set min_age = 18 where slug = 'arena'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/arena/donate").cookie(&brit))
            .await
            .status(),
        StatusCode::NOT_FOUND,
        "18+ in the under-18 view"
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/arena/donate").cookie(&donor))
            .await
            .status(),
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn anyone_signed_out_sees_a_beta_apps_donate_box_without_personal_data(db: PgPool) {
    // The store page shows Donate on Beta and released apps to everyone (owner decision
    // 2026-10-25), so the box's details load signed out too.
    let app = test_app_with(db, simulated).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    let (_, donor) = adult(&app, "Dee Donor", "GB").await;
    listed_with(&app, maker_id, "arena", "beta", "").await;

    let signed_out = || async {
        let res = app.send(req(Method::GET, "/v1/apps/arena/donate")).await;
        assert_eq!(res.status(), StatusCode::OK);
        body_json(res).await
    };
    let info = signed_out().await;
    assert_eq!(info["on"], false);
    assert_eq!(info["off_reason"], "The maker hasn't turned on donations.");
    assert_eq!(info["cant"], "Sign in to donate.");
    // Only the app's public details: no viewer, no Stripe account, no donors.
    let mut keys: Vec<&str> = info
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "cant",
            "currency",
            "donor_covers_fees",
            "maker",
            "max_minor",
            "min_minor",
            "off_reason",
            "on",
            "presets",
            "servers",
            "shared",
            "simulated"
        ]
    );
    assert!(!info.to_string().contains("acct_"), "{info}");

    // Once the maker takes donations, signed out it's on, with the same amounts a donor sees.
    donations_on(&app, &maker, "arena", false).await;
    let info = signed_out().await;
    assert_eq!(
        (info["on"].as_bool(), info["off_reason"].clone()),
        (Some(true), Value::Null)
    );
    assert_eq!(info["cant"], "Sign in to donate.");
    let theirs = get_json(&app, &donor, "/v1/apps/arena/donate").await;
    assert!(theirs["cant"].is_null());
    assert_eq!(
        theirs["presets"][0]["donation"],
        info["presets"][0]["donation"]
    );
    // The maker's own view says so.
    assert_eq!(
        get_json(&app, &maker, "/v1/apps/arena/donate").await["cant"],
        "It's your own app."
    );
    // Private apps have no store page and no donations: 404, signed in or out.
    sqlx::query("update apps set visibility = 'private' where slug = 'arena'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/arena/donate"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_donate_box_tells_each_viewer_their_own_donations_for_the_button(db: PgPool) {
    // Next to Play, the Donate button turns grey once the viewer has donated, and a monthly server
    // donation shows with Cancel. Only the viewer's own, never anyone else's.
    let app = test_app_with(db, simulated).await;
    let (_, maker) = maker_taking_donations(&app, "arena", true).await;
    let (_, dee) = adult(&app, "Dee Donor", "US").await;
    let (_, eve) = adult(&app, "Eve Donor", "US").await;
    let mine = |cookie: String| {
        let app = &app;
        async move { get_json(app, &cookie, "/v1/apps/arena/donate").await["mine"].clone() }
    };
    assert_eq!(
        mine(dee.clone()).await,
        json!({ "last": null, "monthly": null })
    );
    let signed_out = body_json(app.send(req(Method::GET, "/v1/apps/arena/donate")).await).await;
    assert!(signed_out.get("mine").is_none(), "{signed_out}");

    // A one-off donation: what Dee paid and when.
    assert_eq!(
        donate(&app, &dee, "arena", json!({ "amount_minor": 300 }))
            .await
            .0,
        StatusCode::OK
    );
    let after = mine(dee.clone()).await;
    assert_eq!(after["last"]["total_minor"], 340);
    assert_eq!(after["last"]["currency"], "usd");
    assert!(after["last"]["paid_at"].is_string(), "{after}");
    assert!(after["monthly"].is_null());
    assert_eq!(
        mine(eve.clone()).await,
        json!({ "last": null, "monthly": null })
    );
    assert_eq!(
        mine(maker.clone()).await,
        json!({ "last": null, "monthly": null })
    );

    // A monthly server donation, with what cancelling it needs.
    let (_, started) = donate(
        &app,
        &dee,
        "arena",
        json!({ "amount_minor": 500, "monthly": true, "consent": true }),
    )
    .await;
    let id = started["id"].as_str().unwrap();
    let after = mine(dee.clone()).await;
    assert_eq!(after["monthly"]["id"], id);
    assert_eq!(after["monthly"]["total_minor"], 546);
    assert!(after["monthly"]["next_charge_at"].is_string(), "{after}");
    assert!(mine(eve.clone()).await["monthly"].is_null());
    // Eve can't cancel it; Dee can, at once, and the button goes back to the last donation.
    let cancel = |cookie: &str| {
        req(Method::POST, format!("/v1/me/server-donations/{id}/cancel")).cookie(cookie)
    };
    assert_eq!(app.send(cancel(&eve)).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        app.send(cancel(&dee)).await.status(),
        StatusCode::NO_CONTENT
    );
    let after = mine(dee.clone()).await;
    assert!(after["monthly"].is_null(), "{after}");
    assert!(after["last"].is_object(), "{after}");

    // Refunded donations don't count as given.
    sqlx::query("update donations set status = 'refunded'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        mine(dee.clone()).await,
        json!({ "last": null, "monthly": null })
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_branch_shares_with_its_original_only_while_the_original_takes_donations(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (_, _) = maker_taking_donations(&app, "arena", false).await;
    let (modder_id, modder) = app.user("Bo Modder", "US").await;
    community_build(&app, "arena", "arena-mod", modder_id).await;
    donations_on(&app, &modder, "arena-mod", false).await;
    let (_, donor) = adult(&app, "Dee Donor", "US").await;
    let shared = || {
        let app = &app;
        let donor = donor.clone();
        async move { get_json(app, &donor, "/v1/apps/arena-mod/donate").await["shared"].clone() }
    };
    assert_eq!(shared().await["slug"], "arena");
    assert_eq!(shared().await["percent"], 30);

    // Paused: the original's share stops, and the whole donation goes to the branch.
    place_hold(&app, "arena", "review").await;
    assert!(shared().await.is_null());
    assert_eq!(
        get_json(&app, &donor, "/v1/apps/arena/donate").await["off_reason"],
        "Donations to this app are paused."
    );
    let (status, body) = donate(&app, &donor, "arena-mod", json!({ "amount_minor": 700 })).await;
    assert_eq!(status, StatusCode::OK, "no share needed: {body}");
    lift_holds(&app).await;
    assert_eq!(shared().await["slug"], "arena");

    // Not taking donations: the same.
    sqlx::query("update apps set donations_on = false where slug = 'arena'")
        .execute(&app.db)
        .await
        .unwrap();
    assert!(shared().await.is_null());
    let (status, body) = donate(&app, &donor, "arena-mod", json!({ "amount_minor": 700 })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn quotes_check_the_amount_and_carry_the_monthly_terms(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (maker_id, maker) = maker_taking_donations(&app, "arena", true).await;
    listed_with(&app, maker_id, "gone", "released", "status = 'delisted'").await;
    let quote = |q: &str| {
        let app = &app;
        let uri = format!("/v1/apps/arena/donate/quote{q}");
        async move { app.send(req(Method::GET, uri)).await }
    };
    for bad in ["?amount_minor=99", "?amount_minor=50001"] {
        let res = quote(bad).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
        assert_eq!(body_json(res).await["error"]["field"], "amount_minor");
    }
    assert_eq!(quote("").await.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        quote("?amount_minor=lots").await.status(),
        StatusCode::BAD_REQUEST
    );

    // The donor covers the card fees by default: the total comes first.
    let q = body_json(quote("?amount_minor=300").await).await;
    assert_eq!(
        (
            q["donation"].as_i64(),
            q["total"].as_i64(),
            q["slop_fee"].as_i64(),
            q["maker_receives"].as_i64()
        ),
        (Some(300), Some(340), Some(15), Some(285))
    );
    assert_eq!(q["donor_covers_fees"], true);
    assert_eq!(q["currency"], "usd");
    assert!(
        q["consent"]
            .as_str()
            .unwrap()
            .contains("$3.40 every month until I cancel"),
        "{q}"
    );
    assert_eq!(
        app.send(req(
            Method::GET,
            "/v1/apps/gone/donate/quote?amount_minor=300"
        ))
        .await
        .status(),
        StatusCode::NOT_FOUND
    );

    // Once the servers are shut down there are no monthly terms to show.
    let res = app
        .send(req(Method::POST, "/v1/apps/arena/servers/shut-down").cookie(&maker))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let q = body_json(quote("?amount_minor=300").await).await;
    assert!(q["consent"].is_null(), "{q}");
    assert_eq!(q["total"], 340);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn one_off_donations_are_refused_until_everything_is_in_place(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (maker_id, _) = maker_taking_donations(&app, "arena", false).await;
    listed_with(
        &app,
        maker_id,
        "secret",
        "released",
        "visibility = 'private'",
    )
    .await;
    let (dee_id, dee) = adult(&app, "Dee Donor", "US").await;
    let ok = json!({ "amount_minor": 300 });

    let res = app
        .send(req(Method::POST, "/v1/apps/arena/donate").json(ok.clone()))
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        donate(&app, &dee, "secret", ok.clone()).await.0,
        StatusCode::NOT_FOUND
    );
    let (status, body) = donate(&app, &dee, "arena", json!({ "amount_minor": 50001 })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["field"], "amount_minor");
    let (status, body) = donate(
        &app,
        &dee,
        "arena",
        json!({ "amount_minor": 300, "share_name": true, "message": "x".repeat(501) }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["field"], "message");

    // A message without the name isn't kept: it would say who it's from.
    let (status, body) = donate(
        &app,
        &dee,
        "arena",
        json!({ "amount_minor": 300, "share_name": false, "message": "From Dee!" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id: Uuid = body["id"].as_str().unwrap().parse().unwrap();
    let (message, share_name): (Option<String>, bool) =
        sqlx::query_as("select message, share_name from donations where id = $1")
            .bind(id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((message, share_name), (None, false));
    let (status, _) = donate(
        &app,
        &dee,
        "arena",
        json!({ "amount_minor": 300, "share_name": true, "message": "  " }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Someone confirmed 18+ once doesn't confirm again.
    let (_, ed) = app.user("Ed Donor", "US").await;
    let (status, _) = donate(
        &app,
        &ed,
        "arena",
        json!({ "amount_minor": 300, "confirm_adult": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        donate(&app, &ed, "arena", ok.clone()).await.0,
        StatusCode::OK
    );

    // Donations off, or the maker's account not active: refused.
    for sql in [
        "update apps set donations_on = false where slug = 'arena'",
        "update donation_accounts set stripe_status = 'pending'",
        "update donation_accounts set stripe_status = 'restricted'",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
            .execute(&app.db)
            .await
            .unwrap();
        let (status, body) = donate(&app, &dee, "arena", ok.clone()).await;
        assert_eq!(status, StatusCode::CONFLICT, "{sql}: {body}");
        sqlx::query("update apps set donations_on = true where slug = 'arena'")
            .execute(&app.db)
            .await
            .unwrap();
        sqlx::query("update donation_accounts set stripe_status = 'active'")
            .execute(&app.db)
            .await
            .unwrap();
    }
    // Where payments aren't allowed yet.
    sqlx::query("update country_policies set payments_allowed = false where country_code = 'US'")
        .execute(&app.db)
        .await
        .unwrap();
    let (status, body) = donate(&app, &dee, "arena", ok.clone()).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("country")
    );
    sqlx::query("update country_policies set payments_allowed = true where country_code = 'US'")
        .execute(&app.db)
        .await
        .unwrap();

    // Ten in an hour is the most (card testing), however they were paid.
    let (flo_id, flo) = adult(&app, "Flo Donor", "US").await;
    for _ in 0..10 {
        insert_donation(&app, flo_id, "arena", "one_off", "paid").await;
    }
    let (status, body) = donate(&app, &flo, "arena", ok.clone()).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("a lot of donations"),
        "{body}"
    );
    sqlx::query("update donations set created_at = now() - interval '2 hours' where donor_id = $1")
        .bind(flo_id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        donate(&app, &flo, "arena", ok.clone()).await.0,
        StatusCode::OK
    );

    // And the account's own rate limit: 429 with when to come back.
    let (gus_id, gus) = adult(&app, "Gus Donor", "US").await;
    let now = std::time::Instant::now();
    for _ in 0..10 {
        app.state
            .account_limiter
            .take(gus_id, Action::Donation, now)
            .unwrap();
    }
    let res = app
        .send(
            req(Method::POST, "/v1/apps/arena/donate")
                .cookie(&gus)
                .json(ok.clone()),
        )
        .await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    let wait: u64 = res.headers()["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(wait >= 1, "{wait}");
    assert_eq!(body_json(res).await["error"]["code"], "rate_limited");
    let mine: i64 = sqlx::query_scalar("select count(*) from donations where donor_id = $1")
        .bind(dee_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(mine, 2, "only the two that went through");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn monthly_donations_need_servers_and_are_never_shared(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (maker_id, maker) = maker_taking_donations(&app, "arena", true).await;
    listed_with(&app, maker_id, "solo", "released", "").await;
    let res = app
        .send(
            req(Method::PUT, "/v1/apps/solo/donations/settings")
                .cookie(&maker)
                .json(json!({
                    "donations_on": true, "donation_suggested_minor": 300, "donation_currency": "usd",
                    "runs_servers": false, "servers_statement": null
                })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let (modder_id, _) = app.user("Bo Modder", "US").await;
    community_build(&app, "arena", "arena-mod", modder_id).await;
    let (_, dee) = adult(&app, "Dee Donor", "US").await;
    let monthly = json!({ "amount_minor": 500, "monthly": true, "consent": true });

    let (status, body) = donate(&app, &dee, "solo", monthly.clone()).await;
    assert_eq!(status, StatusCode::CONFLICT, "no servers: {body}");
    let mut shared = monthly.clone();
    shared["share_for"] = json!("arena-mod");
    shared["rest_minor"] = json!(700);
    let (status, body) = donate(&app, &dee, "arena", shared).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["field"], "share_for");

    // Shut down: no more monthly donations, though one-off ones go on.
    sqlx::query("update apps set servers_shut_down_at = now() where slug = 'arena'")
        .execute(&app.db)
        .await
        .unwrap();
    let (status, _) = donate(&app, &dee, "arena", monthly).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        count(&app, "select count(*) from server_donations").await,
        0
    );
    let (status, _) = donate(&app, &dee, "arena", json!({ "amount_minor": 300 })).await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn stripe_monthly_donations_are_subscriptions_taking_five_percent(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (maker_id, _) = maker_taking_donations(&app, "arena", true).await;
    let acct: String =
        sqlx::query_scalar("select stripe_account_id from donation_accounts where user_id = $1")
            .bind(maker_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    let (_, dee) = adult(&app, "Dee Donor", "US").await;
    let (status, started) = donate(
        &app,
        &dee,
        "arena",
        json!({ "amount_minor": 500, "monthly": true, "consent": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{started}");
    assert_eq!(started["done"], false);
    let id = started["id"].as_str().unwrap();
    let session = started["url"].as_str().unwrap().rsplit('/').next().unwrap();
    let s = fake.session(session);
    assert_eq!(s["account"], acct.as_str(), "on the maker's own account");
    let form = &s["form"];
    assert_eq!(form["mode"], "subscription");
    assert_eq!(
        form["line_items[0][price_data][recurring][interval]"],
        "month"
    );
    assert_eq!(form["line_items[0][price_data][unit_amount]"], "546");
    assert_eq!(
        form["line_items[0][price_data][product_data][name]"],
        "Monthly server donation to arena"
    );
    assert_eq!(form["metadata[server_donation_id]"], id);
    assert_eq!(form["subscription_data[metadata][server_donation_id]"], id);
    assert!(
        form.get("payment_intent_data[application_fee_amount]")
            .is_none()
    );
    assert!(
        form.get("payment_intent_data[transfer_data][destination]")
            .is_none()
    );
    // Stripe takes the percentage of each month's total: 4.58% of 5.46 is 0.25, 5% of 5.00.
    let percent: f64 = form["subscription_data[application_fee_percent]"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(percent, 4.58);
    assert_eq!((546.0 * percent / 100.0).round() as i64, 25);
    let (stored, status): (Option<String>, String) =
        sqlx::query_as("select checkout_session_id, status from server_donations where id = $1")
            .bind(id.parse::<Uuid>().unwrap())
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        (stored.as_deref(), status.as_str()),
        (Some(session), "pending")
    );
}

/// The monthly fee is sent as a percentage with two decimals, so for large donations Stripe takes
/// a cent less than the 5% Slop records (and reconciliation then reports a mismatch).
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
#[ignore = "new: application_fee_percent rounding makes Stripe's fee differ from Slop's 5% on large monthly donations"]
async fn large_monthly_donations_also_take_exactly_five_percent(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    maker_taking_donations(&app, "arena", true).await;
    let (_, dee) = adult(&app, "Dee Donor", "US").await;
    let (_, started) = donate(
        &app,
        &dee,
        "arena",
        json!({ "amount_minor": 40000, "monthly": true, "consent": true }),
    )
    .await;
    let session = started["url"].as_str().unwrap().rsplit('/').next().unwrap();
    let form = fake.session(session)["form"].clone();
    let total: f64 = form["line_items[0][price_data][unit_amount]"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let percent: f64 = form["subscription_data[application_fee_percent]"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let recorded: i32 = sqlx::query_scalar("select slop_fee_minor from server_donations")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(recorded, 2000);
    assert_eq!(
        (total * percent / 100.0).round() as i32,
        recorded,
        "Stripe's fee on {total} at {percent}%"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn when_stripe_refuses_a_checkout_the_donation_stays_unpaid(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    maker_taking_donations(&app, "arena", false).await;
    let (_, dee) = adult(&app, "Dee Donor", "US").await;
    fake.fail("POST", "v1/checkout/sessions", 400);
    let (status, _) = donate(&app, &dee, "arena", json!({ "amount_minor": 300 })).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let (status, session): (String, Option<String>) =
        sqlx::query_as("select status, checkout_session_id from donations")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((status.as_str(), session), ("pending", None));
    assert_eq!(count(&app, "select count(*) from payments").await, 0);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn shares_for_the_original_are_checked_and_used_once(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    maker_taking_donations(&app, "arena", false).await;
    let (modder_id, modder) = app.user("Bo Modder", "US").await;
    community_build(&app, "arena", "arena-mod", modder_id).await;
    donations_on(&app, &modder, "arena-mod", false).await;
    let (nia_id, nia) = app.user("Nia Maker", "US").await;
    listed_with(&app, nia_id, "other", "released", "").await;
    donations_on(&app, &nia, "other", false).await;
    community_build(&app, "other", "other-mod", modder_id).await;
    let (dee_id, dee) = adult(&app, "Dee Donor", "US").await;
    let (_, eve) = adult(&app, "Eve Donor", "US").await;
    // Each account may start 10 donations an hour: the refusals are someone else's.
    let (_, val) = adult(&app, "Val Donor", "US").await;
    let share = |body: Value| {
        let mut b = json!({ "amount_minor": 300 });
        b.as_object_mut()
            .unwrap()
            .extend(body.as_object().unwrap().clone());
        b
    };

    for (body, field) in [
        (
            json!({ "share_for": "arena", "rest_minor": 700 }),
            "share_for",
        ),
        (
            json!({ "share_for": "other-mod", "rest_minor": 700 }),
            "share_for",
        ),
        (
            json!({ "share_for": "no-such-app", "rest_minor": 700 }),
            "share_for",
        ),
        (json!({ "share_for": "arena-mod" }), "rest_minor"),
        (
            json!({ "share_for": "arena-mod", "rest_minor": 99 }),
            "rest_minor",
        ),
        (
            json!({ "share_for": "arena-mod", "rest_minor": 50001 }),
            "rest_minor",
        ),
    ] {
        let (status, res) = donate(&app, &val, "arena", share(body.clone())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(res["error"]["field"], field, "{body}");
    }
    let (status, paid) = donate(
        &app,
        &dee,
        "arena",
        share(json!({ "share_for": "arena-mod", "rest_minor": 700 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let share_id: Uuid = paid["id"].as_str().unwrap().parse().unwrap();
    let rest = |id: Uuid| json!({ "amount_minor": 700, "share_donation": id });
    let refused = |res: (StatusCode, Value)| {
        assert_eq!(res.0, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.1);
        assert_eq!(res.1["error"]["field"], "share_donation");
    };

    // Too old, someone else's, or not paid: no.
    sqlx::query("update donations set created_at = now() - interval '3 hours' where id = $1")
        .bind(share_id)
        .execute(&app.db)
        .await
        .unwrap();
    refused(donate(&app, &dee, "arena-mod", rest(share_id)).await);
    sqlx::query("update donations set created_at = now() where id = $1")
        .bind(share_id)
        .execute(&app.db)
        .await
        .unwrap();
    refused(donate(&app, &eve, "arena-mod", rest(share_id)).await);
    let pending = insert_donation(&app, dee_id, "arena", "one_off", "pending").await;
    sqlx::query(
        "update donations set share_for = (select id from apps where slug = 'arena-mod') where id = $1",
    )
    .bind(pending)
    .execute(&app.db)
    .await
    .unwrap();
    refused(donate(&app, &dee, "arena-mod", rest(pending)).await);

    // Two at once with the same share: one goes through.
    let (a, b) = tokio::join!(
        donate(&app, &dee, "arena-mod", rest(share_id)),
        donate(&app, &dee, "arena-mod", rest(share_id))
    );
    let mut statuses = [a.0, b.0];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::OK, StatusCode::UNPROCESSABLE_ENTITY],
        "{} {}",
        a.1,
        b.1
    );
    let used: i64 =
        sqlx::query_scalar("select count(*) from donations where share_donation_id = $1")
            .bind(share_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(used, 1);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn stripe_shares_send_the_donor_back_for_the_branch(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    maker_taking_donations(&app, "arena", false).await;
    let (modder_id, modder) = app.user("Bo Modder", "US").await;
    community_build(&app, "arena", "arena-mod", modder_id).await;
    donations_on(&app, &modder, "arena-mod", false).await;
    let (_, dee) = adult(&app, "Dee Donor", "US").await;
    let (status, started) = donate(
        &app,
        &dee,
        "arena",
        json!({ "amount_minor": 300, "share_for": "arena-mod", "rest_minor": 700 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{started}");
    let id = started["id"].as_str().unwrap();
    let session = started["url"].as_str().unwrap().rsplit('/').next().unwrap();
    assert_eq!(
        fake.session(session)["form"]["success_url"],
        format!("http://localhost:8080/app/arena-mod?shared={id}&rest=700")
    );
    // Until it's paid, the branch's donation can't follow it.
    let (status, body) = donate(
        &app,
        &dee,
        "arena-mod",
        json!({ "amount_minor": 700, "share_donation": id }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["field"], "share_donation");
    fake.pay(session);
    let res = app
        .send(req(Method::POST, format!("/v1/me/donations/{id}/check")).cookie(&dee))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let (status, body) = donate(
        &app,
        &dee,
        "arena-mod",
        json!({ "amount_minor": 700, "share_donation": id }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let second = body["url"].as_str().unwrap().rsplit('/').next().unwrap();
    assert_eq!(
        fake.session(second)["form"]["success_url"],
        format!(
            "http://localhost:8080/app/arena-mod?donated={}",
            body["id"].as_str().unwrap()
        )
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn proof_of_work_challenges_belong_to_one_account_and_expire(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    maker_taking_donations(&app, "arena", false).await;
    let (dee_id, dee) = adult(&app, "Dee Donor", "US").await;
    let (_, eve) = adult(&app, "Eve Donor", "US").await;
    assert_eq!(
        app.send(req(Method::POST, "/v1/donations/challenge"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    for _ in 0..3 {
        insert_donation(&app, dee_id, "arena", "one_off", "expired").await;
    }
    let with = |challenge: Value| json!({ "amount_minor": 300, "challenge": challenge });
    let needs_one = |res: (StatusCode, Value)| {
        assert_eq!(res.0, StatusCode::CONFLICT, "{}", res.1);
        assert_eq!(res.1["error"]["code"], "challenge_required");
    };
    needs_one(donate(&app, &dee, "arena", json!({ "amount_minor": 300 })).await);
    // Eve's solved challenge isn't Dee's.
    let evens = solved_challenge(&app, &eve).await;
    needs_one(donate(&app, &dee, "arena", with(evens)).await);
    // Ten minutes and it's gone.
    let late = solved_challenge(&app, &dee).await;
    sqlx::query("update donation_challenges set expires_at = now() - interval '1 second'")
        .execute(&app.db)
        .await
        .unwrap();
    needs_one(donate(&app, &dee, "arena", with(late)).await);
    // An absurdly long answer isn't even looked at.
    let mut long = solved_challenge(&app, &dee).await;
    long["nonce"] = json!("1".repeat(41));
    needs_one(donate(&app, &dee, "arena", with(long)).await);
    let good = solved_challenge(&app, &dee).await;
    let (status, body) = donate(&app, &dee, "arena", with(good)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn my_donations_leave_out_the_unfinished_and_other_peoples(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (maker_id, maker) = maker_taking_donations(&app, "arena", true).await;
    listed_with(&app, maker_id, "arena2", "released", "").await;
    donations_on(&app, &maker, "arena2", true).await;
    listed_with(&app, maker_id, "arena3", "released", "").await;
    let (dee_id, dee) = adult(&app, "Dee Donor", "US").await;
    let (_, eve) = adult(&app, "Eve Donor", "US").await;
    assert_eq!(
        app.send(req(Method::GET, "/v1/me/donations"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        donate(&app, &dee, "arena", json!({ "amount_minor": 300 }))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        donate(&app, &eve, "arena", json!({ "amount_minor": 900 }))
            .await
            .0,
        StatusCode::OK
    );
    insert_donation(&app, dee_id, "arena", "one_off", "expired").await;
    let pending_old = insert_server_donation(&app, dee_id, "arena2", "pending").await;
    sqlx::query("update server_donations set created_at = now() - interval '2 days' where id = $1")
        .bind(pending_old)
        .execute(&app.db)
        .await
        .unwrap();
    let pending_new = insert_server_donation(&app, dee_id, "arena3", "pending").await;

    let mine = get_json(&app, &dee, "/v1/me/donations").await;
    let donations = mine["donations"].as_array().unwrap();
    assert_eq!(donations.len(), 1, "{mine}");
    assert_eq!(donations[0]["total_minor"], 340);
    assert_eq!(donations[0]["status"], "paid");
    let servers: Vec<&str> = mine["server_donations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(servers, [pending_new.to_string().as_str()]);
    let theirs = get_json(&app, &eve, "/v1/me/donations").await;
    assert_eq!(theirs["donations"].as_array().unwrap().len(), 1);
    assert_eq!(theirs["donations"][0]["donation_minor"], 900);
    assert!(theirs["server_donations"].as_array().unwrap().is_empty());
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn checking_a_simulated_donation_is_only_for_its_donor(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    maker_taking_donations(&app, "arena", true).await;
    let (_, dee) = adult(&app, "Dee Donor", "US").await;
    let (_, eve) = adult(&app, "Eve Donor", "US").await;
    let (_, one) = donate(&app, &dee, "arena", json!({ "amount_minor": 300 })).await;
    let (_, monthly) = donate(
        &app,
        &dee,
        "arena",
        json!({ "amount_minor": 500, "monthly": true, "consent": true }),
    )
    .await;
    for id in [&one["id"], &monthly["id"]] {
        let id = id.as_str().unwrap();
        let check =
            |cookie: &str| req(Method::POST, format!("/v1/me/donations/{id}/check")).cookie(cookie);
        assert_eq!(app.send(check(&eve)).await.status(), StatusCode::NOT_FOUND);
        assert_eq!(app.send(check(&dee)).await.status(), StatusCode::NO_CONTENT);
    }
    assert_eq!(
        app.send(
            req(
                Method::POST,
                format!("/v1/me/donations/{}/check", Uuid::now_v7())
            )
            .cookie(&dee)
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn coming_back_from_stripe_reads_the_session_slop_made(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (maker_id, _) = maker_taking_donations(&app, "arena", true).await;
    let acct: String = sqlx::query_scalar("select stripe_account_id from donation_accounts")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let (_, dee) = adult(&app, "Dee Donor", "US").await;
    let (_, eve) = adult(&app, "Eve Donor", "US").await;
    let (_, fay) = adult(&app, "Fay Donor", "US").await;
    let start = |cookie: String, body: Value| {
        let app = &app;
        async move {
            let (status, started) = donate(app, &cookie, "arena", body).await;
            assert_eq!(status, StatusCode::OK, "{started}");
            let session = started["url"]
                .as_str()
                .unwrap()
                .rsplit('/')
                .next()
                .unwrap()
                .to_string();
            (started["id"].as_str().unwrap().to_string(), session)
        }
    };
    let check = |cookie: String, id: String| {
        let app = &app;
        async move {
            app.send(req(Method::POST, format!("/v1/me/donations/{id}/check")).cookie(cookie))
                .await
                .status()
        }
    };
    let status_of = |id: String| {
        let app = &app;
        async move {
            sqlx::query_scalar::<_, String>(
                "select status from donations where id = $1
                 union all select status from server_donations where id = $1",
            )
            .bind(id.parse::<Uuid>().unwrap())
            .fetch_one(&app.db)
            .await
            .unwrap()
        }
    };

    // Paid: recorded from Stripe's own answer, with the charge.
    let (paid, paid_session) = start(dee.clone(), json!({ "amount_minor": 300 })).await;
    assert_eq!(
        check(dee.clone(), paid.clone()).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(status_of(paid.clone()).await, "pending", "not paid yet");
    fake.pay(&paid_session);
    assert_eq!(
        check(eve.clone(), paid.clone()).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        check(dee.clone(), paid.clone()).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(status_of(paid.clone()).await, "paid");
    let (object, account, amount, fee_minor): (String, String, i32, i32) = sqlx::query_as(
        "select stripe_object_id, stripe_account_id, amount_minor, slop_fee_minor from payments",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        (object, account, amount, fee_minor),
        (format!("ch_{paid_session}"), acct.clone(), 340, 15)
    );
    assert!(
        notices(&app, maker_id)
            .await
            .contains(&"New donations to arena".to_string())
    );

    // Expired: marked so, and gone from the donor's list.
    let (expired, expired_session) = start(dee.clone(), json!({ "amount_minor": 300 })).await;
    fake.patch_session(&expired_session, json!({ "status": "expired" }));
    assert_eq!(
        check(dee.clone(), expired.clone()).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(status_of(expired.clone()).await, "expired");

    // A monthly one: the subscription starts.
    let (monthly, monthly_session) = start(
        dee.clone(),
        json!({ "amount_minor": 500, "monthly": true, "consent": true }),
    )
    .await;
    fake.patch_session(
        &monthly_session,
        json!({ "status": "complete", "subscription": "sub_dee" }),
    );
    assert_eq!(
        check(dee.clone(), monthly.clone()).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(status_of(monthly.clone()).await, "active");
    let sub: Option<String> =
        sqlx::query_scalar("select stripe_subscription_id from server_donations")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(sub.as_deref(), Some("sub_dee"));

    // A session changed to name another donation, or for another amount, is ignored.
    let (first, first_session) = start(eve.clone(), json!({ "amount_minor": 300 })).await;
    let (second, _) = start(eve.clone(), json!({ "amount_minor": 300 })).await;
    fake.patch_session(
        &first_session,
        json!({ "metadata": { "donation_id": second } }),
    );
    fake.pay(&first_session);
    assert_eq!(
        check(eve.clone(), first.clone()).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(status_of(first.clone()).await, "pending");
    assert_eq!(status_of(second.clone()).await, "pending");
    let (cheap, cheap_session) = start(fay.clone(), json!({ "amount_minor": 300 })).await;
    fake.patch_session(&cheap_session, json!({ "amount_total": 100 }));
    fake.pay(&cheap_session);
    assert_eq!(
        check(fay.clone(), cheap.clone()).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(status_of(cheap.clone()).await, "pending");
    let (other_currency, other_session) = start(fay.clone(), json!({ "amount_minor": 300 })).await;
    fake.patch_session(&other_session, json!({ "currency": "huf" }));
    fake.pay(&other_session);
    assert_eq!(
        check(fay.clone(), other_currency.clone()).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(status_of(other_currency).await, "pending");
    assert_eq!(count(&app, "select count(*) from payments").await, 1);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn donors_cancel_server_donations_at_once_and_are_told_by_email(db: PgPool) {
    let app = test_app_with(db, |c| {
        simulated(c);
        with_email(c);
    })
    .await;
    maker_taking_donations(&app, "arena", true).await;
    let (dee_id, dee) = adult(&app, "Dee Donor", "US").await;
    let (_, eve) = adult(&app, "Eve Donor", "US").await;
    with_address(&app, dee_id, "dee@example.com").await;
    // Dee switched off donation emails; this one comes anyway.
    sqlx::query("update users set email_prefs = '{\"donations\": false}' where id = $1")
        .bind(dee_id)
        .execute(&app.db)
        .await
        .unwrap();
    let (_, started) = donate(
        &app,
        &dee,
        "arena",
        json!({ "amount_minor": 500, "monthly": true, "consent": true }),
    )
    .await;
    let id = started["id"].as_str().unwrap();
    let cancel = |cookie: &str| {
        req(Method::POST, format!("/v1/me/server-donations/{id}/cancel")).cookie(cookie)
    };
    assert_eq!(
        app.send(req(
            Method::POST,
            format!("/v1/me/server-donations/{id}/cancel")
        ))
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(app.send(cancel(&eve)).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        app.send(cancel(&dee)).await.status(),
        StatusCode::NO_CONTENT
    );
    let (status, by): (String, String) =
        sqlx::query_as("select status, cancelled_by from server_donations")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((status.as_str(), by.as_str()), ("cancelled", "donor"));
    assert!(
        notices(&app, dee_id)
            .await
            .contains(&"Your server donation to arena is cancelled".to_string())
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from email_outbox where to_address = 'dee@example.com' and subject like '%is cancelled'"
        )
        .await,
        1
    );
    assert_eq!(app.send(cancel(&dee)).await.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn cancelling_a_stripe_server_donation_ends_the_subscription_first(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    maker_taking_donations(&app, "arena", true).await;
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
    let id = started["id"].as_str().unwrap();
    let session = started["url"].as_str().unwrap().rsplit('/').next().unwrap();
    fake.patch_session(
        session,
        json!({ "status": "complete", "subscription": "sub_9" }),
    );
    app.send(req(Method::POST, format!("/v1/me/donations/{id}/check")).cookie(&dee))
        .await;
    let cancel = || req(Method::POST, format!("/v1/me/server-donations/{id}/cancel")).cookie(&dee);

    // Stripe down: nothing changes here, so the donor isn't told it stopped when it didn't.
    fake.fail("DELETE", "v1/subscriptions", 500);
    assert_eq!(
        app.send(cancel()).await.status(),
        StatusCode::INTERNAL_SERVER_ERROR
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
    assert_eq!(app.send(cancel()).await.status(), StatusCode::NO_CONTENT);
    let deleted = fake.calls("DELETE", "v1/subscriptions/sub_9");
    assert_eq!(deleted.len(), 2);
    assert_eq!(deleted[1].account.as_deref(), Some(acct.as_str()));
    assert_eq!(
        count(
            &app,
            "select count(*) from server_donations where status = 'cancelled'"
        )
        .await,
        1
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_makers_summary_hides_small_groups_and_unshared_names(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (_, maker) = maker_taking_donations(&app, "arena", true).await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    let mut donors = Vec::new();
    for i in 0..10 {
        let (id, cookie) = adult(&app, &format!("Quiet Donor {i}"), "US").await;
        donors.push((id, cookie));
    }
    let summary = || {
        let app = &app;
        let maker = maker.clone();
        async move { get_json(app, &maker, "/v1/apps/arena/donations").await }
    };
    // Nine donors, one of whom shares their name.
    for (i, (_, cookie)) in donors.iter().take(9).enumerate() {
        let body = if i == 0 {
            json!({ "amount_minor": 300, "share_name": true, "message": "Keep going!" })
        } else {
            json!({ "amount_minor": 300, "message": "secret words" })
        };
        assert_eq!(donate(&app, cookie, "arena", body).await.0, StatusCode::OK);
    }
    let s = summary().await;
    assert!(s["months"].as_array().unwrap().is_empty(), "{s}");
    assert_eq!(s["currency_totals"][0]["donations"], 9);
    let shared = s["shared"].as_array().unwrap();
    assert_eq!(shared.len(), 1);
    assert_eq!(shared[0]["name"], "Quiet Donor 0");
    assert_eq!(shared[0]["message"], "Keep going!");
    let text = s.to_string();
    for i in 1..10 {
        assert!(!text.contains(&format!("Quiet Donor {i}")), "{text}");
    }
    assert!(!text.contains("secret words"));
    // The tenth: the month can be shown.
    donate(&app, &donors[9].1, "arena", json!({ "amount_minor": 300 })).await;
    let s = summary().await;
    assert_eq!(s["months"].as_array().unwrap().len(), 1, "{s}");
    assert_eq!(s["months"][0]["donations"], 10);
    assert_eq!(s["months"][0]["donated_minor"], 3000);

    // Monthly donors, counted only from 10.
    for (id, _) in donors.iter().take(9) {
        insert_server_donation(&app, *id, "arena", "active").await;
    }
    assert!(summary().await["active_server_donations"].is_null());
    insert_server_donation(&app, donors[9].0, "arena", "active").await;
    assert_eq!(summary().await["active_server_donations"], 10);

    // Refunds are for one-off donations still paid.
    let monthly = insert_donation(&app, donors[0].0, "arena", "monthly", "paid").await;
    let s = summary().await;
    let recent = s["recent"].as_array().unwrap();
    let refundable =
        |id: &str| recent.iter().find(|r| r["id"] == id).unwrap()["refundable"].clone();
    assert_eq!(refundable(&monthly.to_string()), false);
    let one_off = recent.iter().find(|r| r["kind"] == "one_off").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(refundable(&one_off), true);
    assert_eq!(
        app.send(req(Method::POST, format!("/v1/donations/{one_off}/refund")).cookie(&maker))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let s = summary().await;
    let refunded = s["recent"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == one_off.as_str())
        .unwrap()
        .clone();
    assert_eq!(
        (
            refunded["status"].as_str(),
            refunded["refundable"].as_bool()
        ),
        (Some("refunded"), Some(false))
    );

    // Only the maker: not staff, not a donor.
    for cookie in [&admin, &donors[0].1] {
        assert_eq!(
            app.send(req(Method::GET, "/v1/apps/arena/donations").cookie(cookie))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn makers_refund_one_off_donations_once(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (_, maker) = maker_taking_donations(&app, "arena", true).await;
    let (nia_id, nia) = app.user("Nia Maker", "US").await;
    listed_with(&app, nia_id, "other", "released", "").await;
    let (dee_id, dee) = adult(&app, "Dee Donor", "US").await;
    let (_, one) = donate(&app, &dee, "arena", json!({ "amount_minor": 300 })).await;
    let id: Uuid = one["id"].as_str().unwrap().parse().unwrap();
    donate(
        &app,
        &dee,
        "arena",
        json!({ "amount_minor": 500, "monthly": true, "consent": true }),
    )
    .await;
    let monthly: Uuid =
        sqlx::query_scalar("select id from donations where kind = 'monthly' and donor_id = $1")
            .bind(dee_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    let refund = |cookie: &str, id: Uuid| {
        req(Method::POST, format!("/v1/donations/{id}/refund")).cookie(cookie)
    };
    assert_eq!(
        app.send(refund(&maker, monthly)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(refund(&nia, id)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(refund(&dee, id)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(refund(&maker, id)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(refund(&maker, id)).await.status(),
        StatusCode::NOT_FOUND
    );
    let original: Uuid =
        sqlx::query_scalar("select id from payments where donation_id = $1 and kind = 'donation'")
            .bind(id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    let (amount, refund_of, donation): (i32, Option<Uuid>, Option<Uuid>) = sqlx::query_as(
        "select amount_minor, refund_of, donation_id from payments where kind = 'refund'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        (amount, refund_of, donation),
        (-340, Some(original), Some(id))
    );
    let (status, refunded): (String, i32) =
        sqlx::query_as("select status, refunded_minor from donations where id = $1")
            .bind(id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((status.as_str(), refunded), ("refunded", 340));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn stripe_refunds_are_made_on_the_makers_account_and_recorded_once(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (_, maker) = maker_taking_donations(&app, "arena", false).await;
    let acct: String = sqlx::query_scalar("select stripe_account_id from donation_accounts")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let (_, dee) = adult(&app, "Dee Donor", "US").await;
    let (_, started) = donate(&app, &dee, "arena", json!({ "amount_minor": 300 })).await;
    let id = started["id"].as_str().unwrap();
    let session = started["url"].as_str().unwrap().rsplit('/').next().unwrap();
    fake.pay(session);
    app.send(req(Method::POST, format!("/v1/me/donations/{id}/check")).cookie(&dee))
        .await;
    let charge = format!("ch_{session}");
    let refund = || req(Method::POST, format!("/v1/donations/{id}/refund")).cookie(&maker);

    fake.fail("POST", "v1/refunds", 402);
    assert_eq!(
        app.send(refund()).await.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        count(&app, "select count(*) from payments where kind = 'refund'").await,
        0
    );
    assert_eq!(
        count(&app, "select count(*) from donations where status = 'paid'").await,
        1
    );
    fake.heal();
    assert_eq!(app.send(refund()).await.status(), StatusCode::NO_CONTENT);
    let calls = fake.calls("POST", "v1/refunds");
    let made = calls.last().unwrap();
    assert_eq!(made.account.as_deref(), Some(acct.as_str()), "as the maker");
    assert_eq!(made.form["charge"], charge);

    // Stripe's webhook about the same refund doesn't record it twice.
    fake.with(|s| {
        s.charges.insert(
            charge.clone(),
            json!({ "id": charge, "amount_refunded": 340 }),
        )
    });
    let status = hook(
        &app,
        event(
            "evt_refund",
            "charge.refunded",
            Some(&acct),
            json!({ "id": charge }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        count(&app, "select count(*) from payments where kind = 'refund'").await,
        1
    );
    assert_eq!(
        count(
            &app,
            "select coalesce(sum(amount_minor), 0)::int8 from payments where kind = 'refund'"
        )
        .await,
        -340
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn renewal_reminders_go_once_then_yearly_and_overdue_months_renew_one_at_a_time(db: PgPool) {
    let app = test_app_with(db, |c| {
        simulated(c);
        with_email(c);
    })
    .await;
    maker_taking_donations(&app, "arena", true).await;
    let (dee_id, dee) = adult(&app, "Dee Donor", "US").await;
    with_address(&app, dee_id, "dee@example.com").await;
    donate(
        &app,
        &dee,
        "arena",
        json!({ "amount_minor": 500, "monthly": true, "consent": true }),
    )
    .await;
    let reminders = || {
        count(
            &app,
            "select count(*) from email_outbox where subject like '%renews on%'",
        )
    };
    let set = |sql: &'static str| {
        let app = &app;
        async move {
            sqlx::query(sqlx::AssertSqlSafe(sql))
                .execute(&app.db)
                .await
                .unwrap();
        }
    };
    slop_api::donations::hourly(&app.state).await.unwrap();
    assert_eq!(reminders().await, 0, "a month away");
    set("update server_donations set next_charge_at = now() + interval '3 days'").await;
    slop_api::donations::hourly(&app.state).await.unwrap();
    assert_eq!(reminders().await, 1, "before the first renewal");
    slop_api::donations::hourly(&app.state).await.unwrap();
    assert_eq!(reminders().await, 1, "once");

    // Then a year on, once a year.
    set("update server_donations set started_at = now() - interval '300 days'").await;
    slop_api::donations::hourly(&app.state).await.unwrap();
    assert_eq!(reminders().await, 1, "not before a year");
    set("update server_donations set started_at = now() - interval '359 days'").await;
    slop_api::donations::hourly(&app.state).await.unwrap();
    assert_eq!(reminders().await, 2);
    slop_api::donations::hourly(&app.state).await.unwrap();
    assert_eq!(reminders().await, 2);
    assert_eq!(
        count(
            &app,
            "select count(*) from server_donations where last_yearly_reminder_at is not null"
        )
        .await,
        1
    );

    // Months behind (the server was down): one month per run, not all of them at once.
    set("update server_donations set next_charge_at = now() - interval '90 days'").await;
    let months = || {
        count(
            &app,
            "select count(*) from donations where kind = 'monthly'",
        )
    };
    let before = months().await;
    slop_api::donations::hourly(&app.state).await.unwrap();
    assert_eq!(months().await, before + 1);
    let behind: bool = sqlx::query_scalar(
        "select next_charge_at between now() - interval '61 days' and now() - interval '59 days' from server_donations",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(behind, "moved on by one month");
    slop_api::donations::hourly(&app.state).await.unwrap();
    assert_eq!(months().await, before + 2);
}

/// b38: while an app's donations are held, monthly donations aren't charged, so a reminder that
/// one "is taken again" is wrong.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn no_renewal_reminders_while_donations_are_held(db: PgPool) {
    let app = test_app_with(db, |c| {
        simulated(c);
        with_email(c);
    })
    .await;
    maker_taking_donations(&app, "arena", true).await;
    let (dee_id, dee) = adult(&app, "Dee Donor", "US").await;
    with_address(&app, dee_id, "dee@example.com").await;
    donate(
        &app,
        &dee,
        "arena",
        json!({ "amount_minor": 500, "monthly": true, "consent": true }),
    )
    .await;
    place_hold(&app, "arena", "review").await;
    sqlx::query("update server_donations set next_charge_at = now() + interval '3 days'")
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::donations::hourly(&app.state).await.unwrap();
    assert_eq!(
        count(
            &app,
            "select count(*) from email_outbox where subject like '%renews on%'"
        )
        .await,
        0
    );
    lift_holds(&app).await;
    slop_api::donations::hourly(&app.state).await.unwrap();
    assert_eq!(
        count(
            &app,
            "select count(*) from email_outbox where subject like '%renews on%'"
        )
        .await,
        1,
        "once it's lifted"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn closing_an_account_waits_for_server_donations_and_withdraws_pledges(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    maker_taking_donations(&app, "arena", true).await;
    let (dee_id, dee) = adult(&app, "Dee Donor", "US").await;
    let (_, started) = donate(
        &app,
        &dee,
        "arena",
        json!({ "amount_minor": 500, "monthly": true, "consent": true }),
    )
    .await;
    let bounty = body_json(
        app.send(
            req(Method::POST, "/v1/apps/arena/bounties")
                .cookie(&dee)
                .json(json!({ "title": "Fix saves", "body": "They vanish.", "currency": "usd" })),
        )
        .await,
    )
    .await;
    let bounty_id = bounty["id"].as_str().unwrap();
    let res = app
        .send(
            req(Method::POST, format!("/v1/bounties/{bounty_id}/pledge"))
                .cookie(&dee)
                .json(json!({ "amount_minor": 1000 })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let close = || {
        req(Method::POST, "/v1/me/close")
            .cookie(&dee)
            .json(json!({ "confirm": "close my account" }))
    };
    let res = app.send(close()).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("you give a monthly server donation")
    );
    let id = started["id"].as_str().unwrap();
    app.send(req(Method::POST, format!("/v1/me/server-donations/{id}/cancel")).cookie(&dee))
        .await;
    assert_eq!(app.send(close()).await.status(), StatusCode::NO_CONTENT);
    let pledge: String = sqlx::query_scalar("select status from bounty_pledges where user_id = $1")
        .bind(dee_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(pledge, "withdrawn");
}

/// b37: a pledge whose delivery was confirmed is due: the builder is owed it, so closing waits
/// until it's charged, as it does for an owed job payment. A closed account is never charged.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_closed_account_is_never_charged_for_a_pledge(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "arena", "released", "").await;
    let (forker_id, forker) = app.user("Forker", "US").await;
    community_build(&app, "arena", "arena-fix", forker_id).await;
    let (eve_id, eve) = adult(&app, "Eve Backer", "US").await;
    let bounty = body_json(
        app.send(
            req(Method::POST, "/v1/apps/arena/bounties")
                .cookie(&eve)
                .json(json!({ "title": "Fix saves", "body": "They vanish.", "currency": "usd" })),
        )
        .await,
    )
    .await;
    let id = bounty["id"].as_str().unwrap();
    app.send(
        req(Method::POST, format!("/v1/bounties/{id}/pledge"))
            .cookie(&eve)
            .json(json!({ "amount_minor": 1000 })),
    )
    .await;
    app.send(
        req(Method::POST, format!("/v1/bounties/{id}/claims"))
            .cookie(&forker)
            .json(json!({ "app_slug": "arena-fix", "note": "Done." })),
    )
    .await;
    let claim: Uuid = sqlx::query_scalar("select id from bounty_claims")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/bounties/{id}/claims/{claim}/answer"),
            )
            .cookie(&maker)
            .json(json!({ "decision": "accepted" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let close = || {
        req(Method::POST, "/v1/me/close")
            .cookie(&eve)
            .json(json!({ "confirm": "close my account" }))
    };
    let res = app.send(close()).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("your pledge is still being charged")
    );
    let charged = || {
        let app = &app;
        async move {
            sqlx::query_scalar::<_, i64>("select count(*) from payments where payer_id = $1")
                .bind(eve_id)
                .fetch_one(&app.db)
                .await
                .unwrap()
        }
    };
    assert_eq!(charged().await, 0);
    // Charged, it's settled: now the account closes, and nothing more is charged.
    slop_api::bounties::charge_due(&app.state).await.unwrap();
    assert_eq!(charged().await, 1);
    assert_eq!(app.send(close()).await.status(), StatusCode::NO_CONTENT);
    slop_api::bounties::charge_due(&app.state).await.unwrap();
    assert_eq!(charged().await, 1);
}

/// b37: a pledge confirmed between the check and the closing still stops it: the account isn't
/// closed with a pledge due.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn closing_never_leaves_a_due_pledge_behind(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "arena", "released", "").await;
    let (eve_id, eve) = adult(&app, "Eve Backer", "US").await;
    let bounty = body_json(
        app.send(
            req(Method::POST, "/v1/apps/arena/bounties")
                .cookie(&eve)
                .json(json!({ "title": "Fix saves", "body": "They vanish.", "currency": "usd" })),
        )
        .await,
    )
    .await;
    let id = bounty["id"].as_str().unwrap();
    app.send(
        req(Method::POST, format!("/v1/bounties/{id}/pledge"))
            .cookie(&eve)
            .json(json!({ "amount_minor": 1000 })),
    )
    .await;
    sqlx::query("update bounty_pledges set status = 'due' where user_id = $1")
        .bind(eve_id)
        .execute(&app.db)
        .await
        .unwrap();
    let err = slop_api::account::close_account(&app.db, eve_id, "test")
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("your pledge is still being charged")
    );
    let closed: bool = sqlx::query_scalar("select closed_at is not null from users where id = $1")
        .bind(eve_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(!closed);
}

/// Pruning the limiter's buckets (at 200,000) keeps every account that's still limited.
#[test]
fn pruning_the_account_limiter_keeps_limited_accounts_limited() {
    use std::time::{Duration, Instant};
    let limiter = slop_api::limits::AccountLimiter::default();
    let t0 = Instant::now();
    let busy = Uuid::now_v7();
    for _ in 0..10 {
        limiter.take(busy, Action::Donation, t0).unwrap();
    }
    assert!(limiter.take(busy, Action::Donation, t0).is_err());
    // Lots of other accounts, long enough ago that their buckets are full again.
    let earlier = t0.checked_sub(Duration::from_secs(7200)).unwrap_or(t0);
    for _ in 0..200_000 {
        limiter.take(Uuid::now_v7(), Action::Vote, earlier).unwrap();
    }
    assert!(
        limiter
            .take(busy, Action::Donation, t0 + Duration::from_secs(1))
            .is_err(),
        "still limited after the prune"
    );
    assert!(limiter.take(Uuid::now_v7(), Action::Donation, t0).is_ok());
    let frequent = limiter.frequent(2);
    assert_eq!(frequent, vec![(busy, 2)], "two refusals today");
}
