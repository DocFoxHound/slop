// SPDX-License-Identifier: AGPL-3.0-or-later
//! Fix bounties: listing, opening, pledging (simulated and with saved cards on Stripe),
//! withdrawing, claiming deliveries, the app maker's answer, backers' votes and voting windows,
//! the charger, staff hiding one, and the maker agent's bounty tools.

use super::*;
use slop_api::limits::Action;

/// A store with a released app "tower" (maker Mia, with an approved release) and a community
/// build "tower-fix" by Forker. Returns (maker, forker) as (id, cookie).
async fn tower(app: &TestApp) -> ((Uuid, String), (Uuid, String)) {
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    listed_with(app, maker_id, "tower", "released", "").await;
    approved_release(app, "tower", maker_id).await;
    let (forker_id, forker) = app.user("Forker", "US").await;
    community_build(app, "tower", "tower-fix", forker_id).await;
    ((maker_id, maker), (forker_id, forker))
}

async fn open_bounty(app: &TestApp, cookie: &str, slug: &str, title: &str) -> String {
    let res = app
        .send(
            req(Method::POST, format!("/v1/apps/{slug}/bounties"))
                .cookie(cookie)
                .json(json!({ "title": title, "body": "Saves vanish when the game crashes.", "currency": "usd" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    body_json(res).await["id"].as_str().unwrap().to_string()
}

/// A bounty made directly, in any state.
async fn insert_bounty(app: &TestApp, slug: &str, by: Uuid, title: &str, status: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into bounties (id, app_id, created_by, title, body, currency, status)
         select $1, id, $2, $3, 'Wanted.', 'usd', $4 from apps where slug = $5",
    )
    .bind(id)
    .bind(by)
    .bind(title)
    .bind(status)
    .bind(slug)
    .execute(&app.db)
    .await
    .unwrap();
    id
}

/// A pledge made directly, in any state.
async fn insert_pledge(app: &TestApp, bounty: &str, user: Uuid, amount: i32, status: &str) {
    sqlx::query(
        "insert into bounty_pledges (id, bounty_id, user_id, amount_minor, status, simulated)
         values ($1, $2, $3, $4, $5, true)",
    )
    .bind(Uuid::now_v7())
    .bind(bounty.parse::<Uuid>().unwrap())
    .bind(user)
    .bind(amount)
    .bind(status)
    .execute(&app.db)
    .await
    .unwrap();
}

async fn pledge(app: &TestApp, cookie: &str, bounty: &str, body: Value) -> (StatusCode, Value) {
    let res = app
        .send(
            req(Method::POST, format!("/v1/bounties/{bounty}/pledge"))
                .cookie(cookie)
                .json(body),
        )
        .await;
    let status = res.status();
    (status, body_json(res).await)
}

async fn pledge_ok(app: &TestApp, cookie: &str, bounty: &str, amount: i64) {
    let (status, body) = pledge(
        app,
        cookie,
        bounty,
        json!({ "amount_minor": amount, "confirm_adult": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

async fn claim(app: &TestApp, cookie: &str, bounty: &str, slug: &str) -> StatusCode {
    app.send(
        req(Method::POST, format!("/v1/bounties/{bounty}/claims"))
            .cookie(cookie)
            .json(json!({ "app_slug": slug, "note": "1.0.0 writes saves safely." })),
    )
    .await
    .status()
}

/// The bounty's pending claim.
async fn pending_claim(app: &TestApp, bounty: &str) -> Uuid {
    sqlx::query_scalar("select id from bounty_claims where bounty_id = $1 and status = 'pending'")
        .bind(bounty.parse::<Uuid>().unwrap())
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn vote(app: &TestApp, cookie: &str, bounty: &str, claim: Uuid, accept: bool) -> StatusCode {
    app.send(
        req(
            Method::POST,
            format!("/v1/bounties/{bounty}/claims/{claim}/vote"),
        )
        .cookie(cookie)
        .json(json!({ "accept": accept })),
    )
    .await
    .status()
}

async fn answer(
    app: &TestApp,
    cookie: &str,
    bounty: &str,
    claim: Uuid,
    decision: &str,
) -> StatusCode {
    app.send(
        req(
            Method::POST,
            format!("/v1/bounties/{bounty}/claims/{claim}/answer"),
        )
        .cookie(cookie)
        .json(json!({ "decision": decision })),
    )
    .await
    .status()
}

async fn bounty_status(app: &TestApp, bounty: &str) -> String {
    sqlx::query_scalar("select status from bounties where id = $1")
        .bind(bounty.parse::<Uuid>().unwrap())
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn pledge_status(app: &TestApp, bounty: &str, user: Uuid) -> String {
    sqlx::query_scalar(
        "select status from bounty_pledges where bounty_id = $1 and user_id = $2 order by created_at desc limit 1",
    )
    .bind(bounty.parse::<Uuid>().unwrap())
    .bind(user)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

/// Moves a claim, and the bounty's pledges with it, `days` into the past.
async fn age_claim(app: &TestApp, claim: Uuid, days: i64) {
    sqlx::query(
        "update bounty_pledges set created_at = created_at - make_interval(days => $2)
         where bounty_id = (select bounty_id from bounty_claims where id = $1)",
    )
    .bind(claim)
    .bind(days as i32)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query("update bounty_claims set created_at = created_at - make_interval(days => $2) where id = $1")
        .bind(claim)
        .bind(days as i32)
        .execute(&app.db)
        .await
        .unwrap();
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn an_apps_bounties_put_open_ones_first_and_leave_hidden_ones_out(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let ((maker_id, _), _) = tower(&app).await;
    let (_, ann) = adult(&app, "Ann Backer", "US").await;
    let (_, bob) = adult(&app, "Bob Backer", "US").await;
    let (_, kit) = app.user("Kit UK", "GB").await;
    let small = open_bounty(&app, &ann, "tower", "Fix lost saves").await;
    pledge_ok(&app, &ann, &small, 1000).await;
    let big = open_bounty(&app, &ann, "tower", "Colourblind mode").await;
    pledge_ok(&app, &bob, &big, 3000).await;
    pledge_ok(&app, &ann, &big, 500).await;
    let old = open_bounty(&app, &bob, "tower", "Port to Linux").await;
    sqlx::query("update bounties set status = 'expired' where id = $1")
        .bind(old.parse::<Uuid>().unwrap())
        .execute(&app.db)
        .await
        .unwrap();
    insert_bounty(&app, "tower", maker_id, "Spam spam", "hidden").await;
    let list = body_json(app.send(req(Method::GET, "/v1/apps/tower/bounties")).await).await;
    let titles: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["title"].as_str().unwrap())
        .collect();
    assert_eq!(
        titles,
        ["Colourblind mode", "Fix lost saves", "Port to Linux"]
    );
    assert_eq!(list[0]["pledged_minor"], 3500);
    assert_eq!(list[0]["backers"], 2);

    listed_with(&app, maker_id, "gone", "released", "status = 'delisted'").await;
    listed_with(&app, maker_id, "grown", "released", "min_age = 18").await;
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/gone/bounties"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/grown/bounties").cookie(&kit))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/grown/bounties").cookie(&ann))
            .await
            .status(),
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_bounty_board_lists_open_bounties_the_viewer_may_see(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let ((maker_id, _), _) = tower(&app).await;
    let (ann_id, _) = adult(&app, "Ann Backer", "US").await;
    let (bob_id, _) = adult(&app, "Bob Backer", "US").await;
    let (carl_id, _) = adult(&app, "Carl Backer", "US").await;
    let (_, yank) = app.user("Yank", "US").await;
    listed_with(
        &app,
        maker_id,
        "secret",
        "released",
        "visibility = 'private'",
    )
    .await;
    listed_with(&app, maker_id, "grown", "released", "min_age = 18").await;
    let open = insert_bounty(&app, "tower", maker_id, "T open", "open").await;
    insert_bounty(&app, "tower", maker_id, "T claimed", "claimed").await;
    for status in ["paid", "expired", "hidden", "paying"] {
        insert_bounty(&app, "tower", maker_id, &format!("T {status}"), status).await;
    }
    insert_bounty(&app, "secret", maker_id, "Secret", "open").await;
    insert_bounty(&app, "grown", maker_id, "Grown", "open").await;
    // Only pledges still standing count.
    let open = open.to_string();
    insert_pledge(&app, &open, ann_id, 1000, "active").await;
    insert_pledge(&app, &open, bob_id, 5000, "withdrawn").await;
    insert_pledge(&app, &open, carl_id, 7000, "released").await;
    let titles = |viewer: Option<String>, q: &'static str| {
        let app = &app;
        async move {
            let mut r = req(Method::GET, format!("/v1/bounties{q}"));
            if let Some(c) = viewer {
                r = r.cookie(c);
            }
            let list = body_json(app.send(r).await).await;
            list.as_array()
                .unwrap()
                .iter()
                .map(|b| b["title"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(titles(None, "").await, ["T open", "T claimed"]);
    let mut theirs = titles(Some(yank.clone()), "").await;
    theirs.sort();
    assert_eq!(
        theirs,
        ["Grown", "T claimed", "T open"],
        "18+ for an adult's account"
    );
    let list = body_json(app.send(req(Method::GET, "/v1/bounties")).await).await;
    assert_eq!(
        (
            list[0]["backers"].as_i64(),
            list[0]["pledged_minor"].as_i64()
        ),
        (Some(1), Some(1000))
    );

    // Fifty to a page.
    for n in 0..50 {
        insert_bounty(&app, "tower", maker_id, &format!("Bulk {n}"), "open").await;
    }
    assert_eq!(titles(None, "").await.len(), 50);
    assert_eq!(titles(None, "?offset=50").await.len(), 2);
    assert_eq!(titles(None, "?offset=-5").await, titles(None, "").await);
    assert!(titles(None, "?offset=99999999").await.is_empty());
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn bounty_detail_shows_the_vote_and_the_viewers_part(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let ((_, maker), _) = tower(&app).await;
    let (_, ann) = adult(&app, "Ann Backer", "US").await;
    let (_, bob) = adult(&app, "Bob Backer", "US").await;
    let (_, carl) = adult(&app, "Carl Looker", "US").await;
    let id = open_bounty(&app, &ann, "tower", "Fix lost saves").await;
    pledge_ok(&app, &ann, &id, 1000).await;
    pledge_ok(&app, &bob, &id, 3000).await;
    assert_eq!(
        claim(&app, &maker, &id, "tower").await,
        StatusCode::NO_CONTENT
    );
    let c = pending_claim(&app, &id).await;
    assert_eq!(vote(&app, &ann, &id, c, true).await, StatusCode::NO_CONTENT);
    let detail = |cookie: String| {
        let app = &app;
        let id = id.clone();
        async move { get_json(app, &cookie, &format!("/v1/bounties/{id}")).await }
    };
    let d = detail(ann.clone()).await;
    assert_eq!(d["claims"][0]["accept_minor"], 1000);
    assert_eq!(d["claims"][0]["reject_minor"], 0);
    assert_eq!(d["claims"][0]["builder"], "Mia Maker");
    assert_eq!(d["claims"][0]["version"], "1.0.0");
    assert_eq!(
        (d["can_vote"].as_bool(), d["my_vote"].as_bool()),
        (Some(true), Some(true))
    );
    assert_eq!(d["my_pledge"]["amount_minor"], 1000);
    assert_eq!(d["payments"], "simulated");
    let d = detail(bob.clone()).await;
    assert_eq!(
        (d["can_vote"].as_bool(), d["my_vote"].is_null()),
        (Some(true), true)
    );
    let d = detail(carl.clone()).await;
    assert_eq!(d["can_vote"], false);
    assert!(d["my_pledge"].is_null());
    let d = detail(maker.clone()).await;
    assert_eq!(
        (d["can_vote"].as_bool(), d["is_app_maker"].as_bool()),
        (Some(false), Some(true))
    );
    assert_eq!(
        app.send(req(Method::GET, format!("/v1/bounties/{}", Uuid::now_v7())))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    sqlx::query("update apps set status = 'delisted' where slug = 'tower'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        app.send(req(Method::GET, format!("/v1/bounties/{id}")).cookie(&ann))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

/// b35: the page offers a vote only to backers whose pledge counts (made before the delivery,
/// and not the deliverer's or the app maker's own).
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_bounty_page_offers_a_vote_only_where_it_would_count(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let ((_, maker), (_, forker)) = tower(&app).await;
    let (_, ann) = adult(&app, "Ann Backer", "US").await;
    let (dan_id, dan) = adult(&app, "Dan Late", "US").await;
    sqlx::query(
        "update users set adult_confirmed_at = now() where display_name in ('Mia Maker', 'Forker')",
    )
    .execute(&app.db)
    .await
    .unwrap();
    let can_vote = |cookie: String, id: String| {
        let app = &app;
        async move { get_json(app, &cookie, &format!("/v1/bounties/{id}")).await["can_vote"].clone() }
    };
    // The maker backs it, then delivers it.
    let own = open_bounty(&app, &ann, "tower", "Fix lost saves").await;
    pledge_ok(&app, &ann, &own, 1000).await;
    pledge_ok(&app, &maker, &own, 500).await;
    assert_eq!(
        claim(&app, &maker, &own, "tower").await,
        StatusCode::NO_CONTENT
    );
    let c = pending_claim(&app, &own).await;
    assert_eq!(
        vote(&app, &maker, &own, c, true).await,
        StatusCode::CONFLICT
    );
    assert_eq!(can_vote(ann.clone(), own.clone()).await, true);
    assert_eq!(
        can_vote(maker.clone(), own.clone()).await,
        false,
        "the maker"
    );
    // A pledge that came after the delivery.
    insert_pledge(&app, &own, dan_id, 4000, "active").await;
    assert_eq!(vote(&app, &dan, &own, c, true).await, StatusCode::CONFLICT);
    assert_eq!(
        can_vote(dan.clone(), own.clone()).await,
        false,
        "a late pledge"
    );
    // The builder backs it, then delivers it, and the maker doesn't answer in 14 days.
    let theirs = open_bounty(&app, &ann, "tower", "Colourblind mode").await;
    pledge_ok(&app, &ann, &theirs, 1000).await;
    pledge_ok(&app, &forker, &theirs, 500).await;
    assert_eq!(
        claim(&app, &forker, &theirs, "tower-fix").await,
        StatusCode::NO_CONTENT
    );
    let c = pending_claim(&app, &theirs).await;
    age_claim(&app, c, 15).await;
    assert_eq!(
        vote(&app, &forker, &theirs, c, true).await,
        StatusCode::CONFLICT
    );
    assert_eq!(
        can_vote(forker.clone(), theirs.clone()).await,
        false,
        "the builder"
    );
    assert_eq!(can_vote(ann.clone(), theirs.clone()).await, true);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn opening_a_bounty_checks_the_app_the_words_and_the_limits(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let ((maker_id, _), _) = tower(&app).await;
    let (ann_id, ann) = adult(&app, "Ann Backer", "US").await;
    listed_with(&app, maker_id, "gone", "released", "status = 'delisted'").await;
    listed_with(
        &app,
        maker_id,
        "secret",
        "released",
        "visibility = 'private'",
    )
    .await;
    approved_release(&app, "secret", maker_id).await;
    sqlx::query(
        "insert into app_shares (app_id, user_id) select id, $1 from apps where slug = 'secret'",
    )
    .bind(ann_id)
    .execute(&app.db)
    .await
    .unwrap();
    let open = |cookie: Option<&str>, slug: &str, body: Value| {
        let mut r = req(Method::POST, format!("/v1/apps/{slug}/bounties")).json(body);
        if let Some(c) = cookie {
            r = r.cookie(c);
        }
        app.send(r)
    };
    let good = json!({ "title": "Fix saves", "body": "They vanish.", "currency": "eur" });
    assert_eq!(
        open(None, "tower", good.clone()).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        open(Some(&ann), "gone", good.clone()).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        open(Some(&ann), "secret", good.clone()).await.status(),
        StatusCode::CONFLICT
    );
    for (body, field) in [
        (
            json!({ "title": "  ", "body": "x", "currency": "usd" }),
            "title",
        ),
        (
            json!({ "title": "x".repeat(101), "body": "x", "currency": "usd" }),
            "title",
        ),
        (
            json!({ "title": "x", "body": " ", "currency": "usd" }),
            "body",
        ),
        (
            json!({ "title": "x", "body": "x".repeat(2001), "currency": "usd" }),
            "body",
        ),
        (
            json!({ "title": "x", "body": "x", "currency": "jpy" }),
            "currency",
        ),
    ] {
        let res = open(Some(&ann), "tower", body).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(res).await["error"]["field"], field);
    }
    let res = open(Some(&ann), "tower", good.clone()).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let made = body_json(res).await;
    assert_eq!(
        (made["status"].as_str(), made["currency"].as_str()),
        (Some("open"), Some("eur"))
    );

    // Ten open at once is the most.
    let (bea_id, bea) = adult(&app, "Bea Busy", "US").await;
    for n in 0..10 {
        insert_bounty(
            &app,
            "tower",
            bea_id,
            &format!("Mine {n}"),
            if n % 2 == 0 { "open" } else { "claimed" },
        )
        .await;
    }
    let res = open(Some(&bea), "tower", good.clone()).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    // And the account's own rate limit.
    let (cy_id, cy) = adult(&app, "Cy Fast", "US").await;
    let now = std::time::Instant::now();
    while app
        .state
        .account_limiter
        .take(cy_id, Action::Bounty, now)
        .is_ok()
    {}
    let res = open(Some(&cy), "tower", good.clone()).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
}

/// b36: opening bounties has its own allowance, apart from reviews.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn writing_reviews_doesnt_use_up_opening_bounties(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    tower(&app).await;
    let (ann_id, ann) = adult(&app, "Ann Reviewer", "US").await;
    let now = std::time::Instant::now();
    while app
        .state
        .account_limiter
        .take(ann_id, Action::Review, now)
        .is_ok()
    {}
    let res = app
        .send(
            req(Method::POST, "/v1/apps/tower/bounties")
                .cookie(&ann)
                .json(json!({ "title": "Fix saves", "body": "They vanish.", "currency": "usd" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn pledges_need_an_adult_an_open_bounty_and_a_sensible_amount(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let ((maker_id, _), _) = tower(&app).await;
    let (_, ann) = adult(&app, "Ann Backer", "US").await;
    let (_, dan) = app.user("Dan Young", "US").await;
    let id = open_bounty(&app, &ann, "tower", "Fix lost saves").await;
    let (status, body) = pledge(&app, &dan, &id, json!({ "amount_minor": 1000 })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["field"], "confirm_adult");
    for amount in [99, 50001] {
        let (status, body) = pledge(&app, &ann, &id, json!({ "amount_minor": amount })).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error"]["field"], "amount_minor");
    }
    pledge_ok(&app, &dan, &id, 1000).await;
    // Withdrawn, then pledged again.
    let res = app
        .send(req(Method::DELETE, format!("/v1/bounties/{id}/pledge")).cookie(&dan))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let (status, body) = pledge(&app, &dan, &id, json!({ "amount_minor": 2000 })).await;
    assert_eq!(
        (status, body["status"].as_str()),
        (StatusCode::OK, Some("active"))
    );
    let d = get_json(&app, &dan, &format!("/v1/bounties/{id}")).await;
    assert_eq!(d["my_pledge"]["amount_minor"], 2000);

    // Hidden or ended bounties take no pledges.
    let hidden = insert_bounty(&app, "tower", maker_id, "Hidden", "hidden").await;
    let (status, _) = pledge(
        &app,
        &ann,
        &hidden.to_string(),
        json!({ "amount_minor": 1000 }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for state in ["expired", "paid", "paying"] {
        let b = insert_bounty(&app, "tower", maker_id, state, state).await;
        let (status, _) = pledge(&app, &ann, &b.to_string(), json!({ "amount_minor": 1000 })).await;
        assert_eq!(status, StatusCode::CONFLICT, "{state}");
    }

    // Saving a card counts against the same hourly allowance as donating.
    let (eve_id, eve) = adult(&app, "Eve Fast", "US").await;
    let now = std::time::Instant::now();
    for _ in 0..10 {
        app.state
            .account_limiter
            .take(eve_id, Action::Donation, now)
            .unwrap();
    }
    let (status, _) = pledge(&app, &eve, &id, json!({ "amount_minor": 1000 })).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

    // Three unfinished pledges in a day: a proof of work first.
    let (fay_id, fay) = adult(&app, "Fay Flaky", "US").await;
    for n in 0..3 {
        let b = insert_bounty(&app, "tower", maker_id, &format!("Old {n}"), "open").await;
        insert_pledge(&app, &b.to_string(), fay_id, 500, "withdrawn").await;
    }
    let (status, body) = pledge(&app, &fay, &id, json!({ "amount_minor": 1000 })).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "challenge_required");
    let challenge = solved_challenge(&app, &fay).await;
    let (status, body) = pledge(
        &app,
        &fay,
        &id,
        json!({ "amount_minor": 1000, "challenge": challenge }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn with_payments_off_nobody_can_pledge(db: PgPool) {
    let app = test_app(db).await;
    tower(&app).await;
    let (_, ann) = adult(&app, "Ann Backer", "US").await;
    let id = open_bounty(&app, &ann, "tower", "Fix lost saves").await;
    let (status, _) = pledge(&app, &ann, &id, json!({ "amount_minor": 1000 })).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(count(&app, "select count(*) from bounty_pledges").await, 0);
}

/// Pledges on Stripe and comes back: (pledge's Checkout session).
async fn stripe_pledge(app: &TestApp, cookie: &str, bounty: &str, amount: i64) -> String {
    let (status, body) = pledge(
        app,
        cookie,
        bounty,
        json!({ "amount_minor": amount, "confirm_adult": true }),
    )
    .await;
    assert_eq!(
        (status, body["status"].as_str()),
        (StatusCode::OK, Some("saving")),
        "{body}"
    );
    body["url"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_string()
}

/// The card is saved at Stripe and the backer comes back to Slop.
async fn card_saved(
    app: &TestApp,
    fake: &FakeStripe,
    cookie: &str,
    bounty: &str,
    session: &str,
    card: &str,
) {
    fake.patch_session(
        session,
        json!({ "status": "complete", "setup_intent": { "payment_method": card } }),
    );
    let res = app
        .send(req(Method::POST, format!("/v1/bounties/{bounty}/pledge/check")).cookie(cookie))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn stripe_pledges_save_cards_on_one_customer_per_backer(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    tower(&app).await;
    let (ann_id, ann) = adult(&app, "Ann Backer", "US").await;
    let (_, bob) = adult(&app, "Bob Backer", "US").await;
    let first = open_bounty(&app, &ann, "tower", "Fix lost saves").await;
    let second = open_bounty(&app, &ann, "tower", "Colourblind mode").await;
    let s1 = stripe_pledge(&app, &ann, &first, 1000).await;
    let s2 = stripe_pledge(&app, &ann, &second, 700).await;
    assert_eq!(fake.calls("POST", "v1/customers").len(), 1, "made once");
    let customer: String =
        sqlx::query_scalar("select customer_id from stripe_customers where user_id = $1")
            .bind(ann_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    for s in [&s1, &s2] {
        let session = fake.session(s);
        assert!(session["account"].is_null(), "on Slop's own account");
        assert_eq!(session["form"]["customer"], customer.as_str());
        assert_eq!(session["form"]["mode"], "setup");
    }

    // Back from Checkout before finishing: still saving. Someone else's check touches nothing.
    let check = |cookie: &str, id: &str| {
        req(Method::POST, format!("/v1/bounties/{id}/pledge/check")).cookie(cookie)
    };
    assert_eq!(
        app.send(check(&ann, &first)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(pledge_status(&app, &first, ann_id).await, "saving");
    fake.patch_session(
        &s1,
        json!({ "status": "complete", "setup_intent": { "payment_method": "pm_ann" } }),
    );
    let reads = fake.calls("GET", "v1/checkout/sessions").len();
    assert_eq!(
        app.send(check(&bob, &first)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(fake.calls("GET", "v1/checkout/sessions").len(), reads);
    assert_eq!(pledge_status(&app, &first, ann_id).await, "saving");
    assert_eq!(
        app.send(check(&ann, &first)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(pledge_status(&app, &first, ann_id).await, "active");

    // Withdrawing forgets the card at Stripe.
    let res = app
        .send(req(Method::DELETE, format!("/v1/bounties/{first}/pledge")).cookie(&ann))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        fake.calls("POST", "v1/payment_methods/pm_ann/detach").len(),
        1
    );
    // A pledge still at Checkout has no card to forget.
    let res = app
        .send(req(Method::DELETE, format!("/v1/bounties/{second}/pledge")).cookie(&ann))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(fake.calls("POST", "v1/payment_methods").len(), 1);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn withdrawing_a_pledge_works_until_its_delivery_is_accepted(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let ((_, maker), _) = tower(&app).await;
    let (_, ann) = adult(&app, "Ann Backer", "US").await;
    let (bob_id, bob) = adult(&app, "Bob Backer", "US").await;
    let id = open_bounty(&app, &ann, "tower", "Fix lost saves").await;
    let withdraw =
        |cookie: &str| req(Method::DELETE, format!("/v1/bounties/{id}/pledge")).cookie(cookie);
    assert_eq!(
        app.send(withdraw(&ann)).await.status(),
        StatusCode::CONFLICT,
        "nothing to withdraw"
    );
    pledge_ok(&app, &ann, &id, 1000).await;
    pledge_ok(&app, &bob, &id, 3000).await;
    assert_eq!(
        claim(&app, &maker, &id, "tower").await,
        StatusCode::NO_CONTENT
    );
    let c = pending_claim(&app, &id).await;
    assert_eq!(
        vote(&app, &ann, &id, c, false).await,
        StatusCode::NO_CONTENT
    );
    let d = get_json(&app, &bob, &format!("/v1/bounties/{id}")).await;
    assert_eq!(
        (
            d["claims"][0]["reject_minor"].as_i64(),
            d["pledged_minor"].as_i64()
        ),
        (Some(1000), Some(4000))
    );
    // Ann withdraws while the vote is open: her vote goes with her pledge.
    assert_eq!(
        app.send(withdraw(&ann)).await.status(),
        StatusCode::NO_CONTENT
    );
    let d = get_json(&app, &bob, &format!("/v1/bounties/{id}")).await;
    assert_eq!(
        (
            d["claims"][0]["reject_minor"].as_i64(),
            d["pledged_minor"].as_i64()
        ),
        (Some(0), Some(3000))
    );
    // Bob is now every counted backer: his vote decides, and his pledge is due.
    assert_eq!(vote(&app, &bob, &id, c, true).await, StatusCode::NO_CONTENT);
    assert_eq!(bounty_status(&app, &id).await, "paying");
    assert_eq!(pledge_status(&app, &id, bob_id).await, "due");
    assert_eq!(
        app.send(withdraw(&bob)).await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(pledge_status(&app, &id, bob_id).await, "due");
}

/// b34: when every counted pledge is withdrawn from a delivery the backers decide, nobody is left
/// to settle it: the delivery is declined and the bounty reopens (or ends, past its deadline),
/// and the builder is told why.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_delivery_with_no_pledges_left_doesnt_hold_the_bounty_for_ever(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let ((maker_id, maker), (forker_id, forker)) = tower(&app).await;
    let (_, ann) = adult(&app, "Ann Backer", "US").await;
    let withdraw = |id: &str| req(Method::DELETE, format!("/v1/bounties/{id}/pledge")).cookie(&ann);
    let told = |who: Uuid| {
        let app = &app;
        async move {
            sqlx::query_scalar::<_, String>(
                "select body from notifications where user_id = $1 and title like '%was declined'",
            )
            .bind(who)
            .fetch_all(&app.db)
            .await
            .unwrap()
        }
    };
    // The maker's own delivery: the last backer withdrawing settles it at once.
    let id = open_bounty(&app, &ann, "tower", "Fix lost saves").await;
    pledge_ok(&app, &ann, &id, 1000).await;
    assert_eq!(
        claim(&app, &maker, &id, "tower").await,
        StatusCode::NO_CONTENT
    );
    let c = pending_claim(&app, &id).await;
    assert_eq!(
        app.send(withdraw(&id)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(bounty_status(&app, &id).await, "open");
    let (status, by): (String, Option<String>) =
        sqlx::query_as("select status, decided_by from bounty_claims where id = $1")
            .bind(c)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        (status.as_str(), by.as_deref()),
        ("declined", Some("backers"))
    );
    assert_eq!(
        told(maker_id).await,
        ["Every backer withdrew their pledge. The bounty is open again."]
    );
    // Someone else's delivery the maker never answered, on a bounty past its deadline: the
    // charger declines it once the backers would decide, and the bounty ends as usual.
    let id = open_bounty(&app, &ann, "tower", "Add a map").await;
    pledge_ok(&app, &ann, &id, 1000).await;
    assert_eq!(
        claim(&app, &forker, &id, "tower-fix").await,
        StatusCode::NO_CONTENT
    );
    let c = pending_claim(&app, &id).await;
    assert_eq!(
        app.send(withdraw(&id)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        bounty_status(&app, &id).await,
        "claimed",
        "the maker decides first"
    );
    age_claim(&app, c, 30).await;
    sqlx::query("update bounties set expires_at = now() - interval '1 day' where id = $1")
        .bind(id.parse::<Uuid>().unwrap())
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::bounties::settle_due(&app.state).await.unwrap();
    assert_eq!(bounty_status(&app, &id).await, "open");
    assert_eq!(
        told(forker_id).await,
        [
            "Every backer withdrew their pledge. The bounty has passed its deadline, so it has ended."
        ]
    );
    slop_api::bounties::charge_due(&app.state).await.unwrap();
    assert_eq!(bounty_status(&app, &id).await, "expired");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn claims_need_an_approved_release_of_the_bounty_app_or_a_build_of_it(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let ((maker_id, _), (forker_id, forker)) = tower(&app).await;
    let (ann_id, ann) = adult(&app, "Ann Backer", "US").await;
    let (bob_id, bob) = adult(&app, "Bob Backer", "US").await;
    let id = open_bounty(&app, &ann, "tower", "Fix lost saves").await;
    pledge_ok(&app, &ann, &id, 1000).await;
    pledge_ok(&app, &bob, &id, 3000).await;
    let claim_with = |cookie: &str, bounty: &str, slug: &str, note: String| {
        app.send(
            req(Method::POST, format!("/v1/bounties/{bounty}/claims"))
                .cookie(cookie)
                .json(json!({ "app_slug": slug, "note": note })),
        )
    };
    assert_eq!(
        claim(&app, &forker, &Uuid::now_v7().to_string(), "tower-fix").await,
        StatusCode::NOT_FOUND
    );
    for note in ["  ".to_string(), "x".repeat(2001)] {
        let res = claim_with(&forker, &id, "tower-fix", note).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(res).await["error"]["field"], "note");
    }
    // An app of theirs that has nothing to do with this one.
    listed_with(&app, forker_id, "side", "released", "").await;
    approved_release(&app, "side", forker_id).await;
    let res = claim_with(&forker, &id, "side", "Done.".into()).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(res).await["error"]["field"], "app_slug");
    // A build of it with no approved release yet.
    sqlx::query(
        "insert into apps (id, slug, title, summary, kind, stage, status, maker_id, repo_url, license_spdx, parent_app_id, branch_kind, branch_note)
         select $1, 'tower-wip', 'WIP', 'WIP', 'game', 'beta', 'listed', $2, 'https://github.com/f/w', 'MIT', id, 'fix', 'WIP'
         from apps where slug = 'tower'",
    )
    .bind(Uuid::now_v7())
    .bind(forker_id)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(
        claim(&app, &forker, &id, "tower-wip").await,
        StatusCode::CONFLICT
    );
    // Ended bounties take no deliveries.
    for state in ["expired", "paid"] {
        let b = insert_bounty(&app, "tower", maker_id, state, state).await;
        assert_eq!(
            claim(&app, &forker, &b.to_string(), "tower-fix").await,
            StatusCode::CONFLICT,
            "{state}"
        );
    }

    assert_eq!(
        claim(&app, &forker, &id, "tower-fix").await,
        StatusCode::NO_CONTENT
    );
    let told = "A delivery for \"Fix lost saves\"".to_string();
    for (who, expect) in [
        (ann_id, true),
        (bob_id, true),
        (maker_id, true),
        (forker_id, false),
    ] {
        assert_eq!(notices(&app, who).await.contains(&told), expect, "{who}");
    }
    assert_eq!(bounty_status(&app, &id).await, "claimed");
}

/// b41: two deliveries at the same moment: the second is told one is waiting, not a server
/// error.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_second_delivery_at_the_same_moment_is_a_conflict(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let ((_, maker), (forker_id, _)) = tower(&app).await;
    let (_, ann) = adult(&app, "Ann Backer", "US").await;
    let id = open_bounty(&app, &ann, "tower", "Fix lost saves").await;
    // The other request's claim is in, its bounty not yet marked claimed.
    sqlx::query(
        "insert into bounty_claims (id, bounty_id, claimed_by, app_id, release_id, stripe_account_id, note)
         select $1, $2, $3, a.id, r.id, 'acct_sim', 'Done.' from apps a join releases r on r.app_id = a.id
         where a.slug = 'tower-fix'",
    )
    .bind(Uuid::now_v7())
    .bind(id.parse::<Uuid>().unwrap())
    .bind(forker_id)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(
        claim(&app, &maker, &id, "tower").await,
        StatusCode::CONFLICT
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_app_maker_accepts_or_declines_someone_elses_delivery(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let ((_, maker), (forker_id, forker)) = tower(&app).await;
    let (ann_id, ann) = adult(&app, "Ann Backer", "US").await;
    let (carl_id, _) = adult(&app, "Carl Checkout", "US").await;
    let id = open_bounty(&app, &ann, "tower", "Fix lost saves").await;
    pledge_ok(&app, &ann, &id, 1000).await;
    assert_eq!(
        claim(&app, &forker, &id, "tower-fix").await,
        StatusCode::NO_CONTENT
    );
    let c = pending_claim(&app, &id).await;
    assert_eq!(
        answer(&app, &maker, &id, c, "maybe").await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        answer(&app, &forker, &id, c, "accepted").await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        answer(&app, &maker, &id, c, "declined").await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(bounty_status(&app, &id).await, "open");
    assert!(
        notices(&app, forker_id)
            .await
            .contains(&"Your delivery for \"Fix lost saves\" was declined".to_string())
    );
    assert_eq!(
        answer(&app, &maker, &id, c, "accepted").await,
        StatusCode::NOT_FOUND,
        "decided"
    );

    // Delivered again and accepted: saved cards are due; one still at Checkout is let go.
    assert_eq!(
        claim(&app, &forker, &id, "tower-fix").await,
        StatusCode::NO_CONTENT
    );
    insert_pledge(&app, &id, carl_id, 2000, "saving").await;
    let c = pending_claim(&app, &id).await;
    assert_eq!(
        answer(&app, &maker, &id, c, "accepted").await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(bounty_status(&app, &id).await, "paying");
    assert_eq!(pledge_status(&app, &id, ann_id).await, "due");
    assert_eq!(pledge_status(&app, &id, carl_id).await, "released");
    let decided: (String, Option<String>) =
        sqlx::query_as("select status, decided_by from bounty_claims where id = $1")
            .bind(c)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(decided, ("accepted".into(), Some("maker".into())));
    assert!(
        notices(&app, forker_id)
            .await
            .contains(&"Your delivery for \"Fix lost saves\" was accepted".to_string())
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn backers_vote_by_amount_when_the_maker_cant_or_doesnt_answer(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let ((_, maker), (_, forker)) = tower(&app).await;
    let (_, ann) = adult(&app, "Ann Backer", "US").await;
    let (_, bob) = adult(&app, "Bob Backer", "US").await;
    let (_, carl) = adult(&app, "Carl Looker", "US").await;
    let (dan_id, dan) = adult(&app, "Dan Late", "US").await;

    // Someone else's delivery: the maker answers first.
    let theirs = open_bounty(&app, &ann, "tower", "Colourblind mode").await;
    pledge_ok(&app, &ann, &theirs, 1000).await;
    assert_eq!(
        claim(&app, &forker, &theirs, "tower-fix").await,
        StatusCode::NO_CONTENT
    );
    let c = pending_claim(&app, &theirs).await;
    assert_eq!(
        vote(&app, &ann, &theirs, c, true).await,
        StatusCode::CONFLICT
    );
    age_claim(&app, c, 15).await;
    assert_eq!(
        vote(&app, &ann, &theirs, c, true).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        bounty_status(&app, &theirs).await,
        "paying",
        "the only backer, after 14 days"
    );

    // The maker's own: backers at once.
    let id = open_bounty(&app, &ann, "tower", "Fix lost saves").await;
    pledge_ok(&app, &ann, &id, 3000).await;
    pledge_ok(&app, &bob, &id, 1000).await;
    assert_eq!(
        claim(&app, &maker, &id, "tower").await,
        StatusCode::NO_CONTENT
    );
    let c = pending_claim(&app, &id).await;
    assert_eq!(
        vote(&app, &carl, &id, c, true).await,
        StatusCode::CONFLICT,
        "not a backer"
    );
    insert_pledge(&app, &id, dan_id, 9000, "active").await;
    assert_eq!(
        vote(&app, &dan, &id, c, true).await,
        StatusCode::CONFLICT,
        "pledged after"
    );
    // 3,000 of 4,000 is a majority, but Bob hasn't voted and the window is open.
    assert_eq!(vote(&app, &ann, &id, c, true).await, StatusCode::NO_CONTENT);
    assert_eq!(bounty_status(&app, &id).await, "claimed");
    // Votes can change.
    assert_eq!(
        vote(&app, &ann, &id, c, false).await,
        StatusCode::NO_CONTENT
    );
    let d = get_json(&app, &ann, &format!("/v1/bounties/{id}")).await;
    assert_eq!(
        (
            d["my_vote"].as_bool(),
            d["claims"][0]["reject_minor"].as_i64()
        ),
        (Some(false), Some(3000))
    );
    // Everyone has voted: declined by the majority, and open again.
    assert_eq!(vote(&app, &bob, &id, c, true).await, StatusCode::NO_CONTENT);
    assert_eq!(bounty_status(&app, &id).await, "open");
    let decided: (String, Option<String>) =
        sqlx::query_as("select status, decided_by from bounty_claims where id = $1")
            .bind(c)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(decided, ("declined".into(), Some("backers".into())));
    assert_eq!(
        vote(&app, &bob, &id, c, true).await,
        StatusCode::NOT_FOUND,
        "decided"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn voting_windows_close_on_their_own_when_a_majority_agrees(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let ((_, maker), (_, forker)) = tower(&app).await;
    let (ann_id, ann) = adult(&app, "Ann Backer", "US").await;
    let (bob_id, bob) = adult(&app, "Bob Backer", "US").await;
    let settle = || slop_api::bounties::settle_due(&app.state);

    // A majority by amount, Bob silent: decided once the 3 days are up.
    let majority = open_bounty(&app, &ann, "tower", "Fix lost saves").await;
    pledge_ok(&app, &ann, &majority, 3000).await;
    pledge_ok(&app, &bob, &majority, 1000).await;
    assert_eq!(
        claim(&app, &maker, &majority, "tower").await,
        StatusCode::NO_CONTENT
    );
    let c1 = pending_claim(&app, &majority).await;
    vote(&app, &ann, &majority, c1, true).await;
    settle().await.unwrap();
    assert_eq!(bounty_status(&app, &majority).await, "claimed");
    age_claim(&app, c1, 4).await;
    settle().await.unwrap();
    assert_eq!(bounty_status(&app, &majority).await, "paying");
    assert_eq!(pledge_status(&app, &majority, ann_id).await, "due");
    assert_eq!(pledge_status(&app, &majority, bob_id).await, "due");

    // Split down the middle: no majority, still pending.
    let split = open_bounty(&app, &ann, "tower", "Colourblind mode").await;
    pledge_ok(&app, &ann, &split, 1000).await;
    pledge_ok(&app, &bob, &split, 1000).await;
    assert_eq!(
        claim(&app, &maker, &split, "tower").await,
        StatusCode::NO_CONTENT
    );
    let c2 = pending_claim(&app, &split).await;
    vote(&app, &ann, &split, c2, true).await;
    vote(&app, &bob, &split, c2, false).await;
    age_claim(&app, c2, 4).await;
    settle().await.unwrap();
    assert_eq!(bounty_status(&app, &split).await, "claimed");

    // Someone else's delivery the maker still has time to answer: not the backers' to settle.
    let theirs = open_bounty(&app, &ann, "tower", "Port to Linux").await;
    pledge_ok(&app, &ann, &theirs, 1000).await;
    assert_eq!(
        claim(&app, &forker, &theirs, "tower-fix").await,
        StatusCode::NO_CONTENT
    );
    let c3 = pending_claim(&app, &theirs).await;
    sqlx::query("insert into bounty_votes (claim_id, user_id, accept) values ($1, $2, true)")
        .bind(c3)
        .bind(ann_id)
        .execute(&app.db)
        .await
        .unwrap();
    age_claim(&app, c3, 4).await;
    settle().await.unwrap();
    assert_eq!(bounty_status(&app, &theirs).await, "claimed");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn builders_withdraw_their_own_waiting_deliveries(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let ((_, maker), (_, forker)) = tower(&app).await;
    let (_, ann) = adult(&app, "Ann Backer", "US").await;
    let id = open_bounty(&app, &ann, "tower", "Fix lost saves").await;
    pledge_ok(&app, &ann, &id, 1000).await;
    assert_eq!(
        claim(&app, &forker, &id, "tower-fix").await,
        StatusCode::NO_CONTENT
    );
    let c = pending_claim(&app, &id).await;
    // The builder sees it's theirs (the website's Withdraw button); nobody else does.
    for (cookie, mine) in [(&forker, true), (&ann, false), (&maker, false)] {
        let res = app
            .send(req(Method::GET, format!("/v1/bounties/{id}")).cookie(cookie))
            .await;
        assert_eq!(body_json(res).await["claims"][0]["mine"], mine);
    }
    let res = app
        .send(req(Method::GET, format!("/v1/bounties/{id}")))
        .await;
    assert_eq!(body_json(res).await["claims"][0]["mine"], false);
    let withdraw = |cookie: &str, claim: Uuid| {
        req(
            Method::POST,
            format!("/v1/bounties/{id}/claims/{claim}/withdraw"),
        )
        .cookie(cookie)
    };
    assert_eq!(
        app.send(withdraw(&ann, c)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(withdraw(&maker, c)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(withdraw(&forker, c)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(bounty_status(&app, &id).await, "open");
    let status: String = sqlx::query_scalar("select status from bounty_claims where id = $1")
        .bind(c)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "withdrawn");
    assert_eq!(
        app.send(withdraw(&forker, c)).await.status(),
        StatusCode::NOT_FOUND
    );
    // A decided one can't be.
    assert_eq!(
        claim(&app, &forker, &id, "tower-fix").await,
        StatusCode::NO_CONTENT
    );
    let c = pending_claim(&app, &id).await;
    answer(&app, &maker, &id, c, "declined").await;
    assert_eq!(
        app.send(withdraw(&forker, c)).await.status(),
        StatusCode::NOT_FOUND
    );
}

/// A bounty on Stripe whose delivery by Forker (paid to acct_fix) was accepted, with saved
/// cards from these backers. Returns the bounty.
async fn accepted_on_stripe(
    app: &TestApp,
    fake: &FakeStripe,
    maker: &str,
    forker_id: Uuid,
    forker: &str,
    title: &str,
    backers: &[(&str, &str)],
) -> String {
    sqlx::query(
        "insert into donation_accounts (user_id, stripe_account_id, stripe_status, tax_notice_ack_at)
         values ($1, 'acct_fix', 'active', now()) on conflict do nothing",
    )
    .bind(forker_id)
    .execute(&app.db)
    .await
    .unwrap();
    let id = open_bounty(app, backers[0].0, "tower", title).await;
    for (cookie, card) in backers {
        let session = stripe_pledge(app, cookie, &id, 1000).await;
        card_saved(app, fake, cookie, &id, &session, card).await;
    }
    assert_eq!(
        claim(app, forker, &id, "tower-fix").await,
        StatusCode::NO_CONTENT
    );
    let c = pending_claim(app, &id).await;
    assert_eq!(
        answer(app, maker, &id, c, "accepted").await,
        StatusCode::NO_CONTENT
    );
    id
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_charger_charges_each_card_once_and_a_refusal_fails_only_that_pledge(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let ((_, maker), (forker_id, forker)) = tower(&app).await;
    let (ann_id, ann) = adult(&app, "Ann Backer", "US").await;
    let (bob_id, bob) = adult(&app, "Bob Backer", "US").await;
    fake.with(|s| s.declined_cards.push("pm_bob".into()));
    let id = accepted_on_stripe(
        &app,
        &fake,
        &maker,
        forker_id,
        &forker,
        "Fix lost saves",
        &[(&ann, "pm_ann"), (&bob, "pm_bob")],
    )
    .await;
    assert_eq!(slop_api::bounties::charge_due(&app.state).await.unwrap(), 1);
    assert_eq!(pledge_status(&app, &id, ann_id).await, "paid");
    assert_eq!(pledge_status(&app, &id, bob_id).await, "failed");
    assert!(
        notices(&app, bob_id)
            .await
            .contains(&"Your pledge to \"Fix lost saves\" couldn't be charged".to_string())
    );
    assert!(
        notices(&app, ann_id)
            .await
            .iter()
            .all(|t| !t.contains("couldn't be charged"))
    );
    assert_eq!(bounty_status(&app, &id).await, "paid", "nothing left due");
    let d = get_json(&app, &bob, &format!("/v1/bounties/{id}")).await;
    assert!(
        d["my_pledge"]["failure"].as_str().unwrap().contains("402"),
        "{d}"
    );

    // Each step has its own idempotency key, so a retry can't charge twice.
    let ann_pledge: Uuid = sqlx::query_scalar("select id from bounty_pledges where user_id = $1")
        .bind(ann_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    let clone = fake
        .calls("POST", "v1/payment_methods")
        .into_iter()
        .find(|c| c.form.get("payment_method").map(String::as_str) == Some("pm_ann"))
        .unwrap();
    assert_eq!(clone.account.as_deref(), Some("acct_fix"));
    assert_eq!(
        clone.idempotency_key,
        Some(format!("bounty-pledge-{ann_pledge}-card"))
    );
    let charge = fake
        .calls("POST", "v1/payment_intents")
        .into_iter()
        .find(|c| c.idempotency_key == Some(format!("bounty-pledge-{ann_pledge}")))
        .unwrap();
    assert_eq!(charge.account.as_deref(), Some("acct_fix"));
    assert_eq!(charge.form["application_fee_amount"], "50");
    assert_eq!(charge.form["description"], "Slop bounty: Fix lost saves");
    // Nothing left to charge.
    assert_eq!(slop_api::bounties::charge_due(&app.state).await.unwrap(), 0);
    assert_eq!(
        count(&app, "select count(*) from payments where kind = 'bounty'").await,
        1
    );

    // A payment Stripe didn't complete (it wants the backer back) counts as refused too.
    let (carl_id, carl) = adult(&app, "Carl Backer", "US").await;
    let id = accepted_on_stripe(
        &app,
        &fake,
        &maker,
        forker_id,
        &forker,
        "Colourblind mode",
        &[(&carl, "pm_carl")],
    )
    .await;
    fake.with(|s| s.intent_status = Some("requires_action"));
    assert_eq!(slop_api::bounties::charge_due(&app.state).await.unwrap(), 0);
    assert_eq!(pledge_status(&app, &id, carl_id).await, "failed");
}

/// b33: Stripe being down isn't a refused card: the pledge waits for the next run, and the
/// backer isn't told their card was refused.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_stripe_outage_leaves_pledges_due_for_the_next_run(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let ((_, maker), (forker_id, forker)) = tower(&app).await;
    let (ann_id, ann) = adult(&app, "Ann Backer", "US").await;
    let id = accepted_on_stripe(
        &app,
        &fake,
        &maker,
        forker_id,
        &forker,
        "Fix lost saves",
        &[(&ann, "pm_ann")],
    )
    .await;
    fake.fail("POST", "v1/payment_intents", 500);
    assert_eq!(slop_api::bounties::charge_due(&app.state).await.unwrap(), 0);
    assert_eq!(pledge_status(&app, &id, ann_id).await, "due");
    assert!(
        notices(&app, ann_id)
            .await
            .iter()
            .all(|t| !t.contains("couldn't be charged"))
    );
    assert_eq!(bounty_status(&app, &id).await, "paying");
    fake.heal();
    assert_eq!(slop_api::bounties::charge_due(&app.state).await.unwrap(), 1);
    assert_eq!(pledge_status(&app, &id, ann_id).await, "paid");
    assert_eq!(bounty_status(&app, &id).await, "paid");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_charger_waits_while_the_delivering_app_is_held_and_runs_one_at_a_time(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let ((_, maker), (_, forker)) = tower(&app).await;
    let (ann_id, ann) = adult(&app, "Ann Backer", "US").await;
    let id = open_bounty(&app, &ann, "tower", "Fix lost saves").await;
    pledge_ok(&app, &ann, &id, 1000).await;
    claim(&app, &forker, &id, "tower-fix").await;
    let c = pending_claim(&app, &id).await;
    answer(&app, &maker, &id, c, "accepted").await;
    sqlx::query(
        "insert into donation_holds (id, app_id, kind, reason, placed_by)
         select $1, id, 'review', 'Checking a complaint.', 'test' from apps where slug = 'tower-fix'",
    )
    .bind(Uuid::now_v7())
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(slop_api::bounties::charge_due(&app.state).await.unwrap(), 0);
    assert_eq!(pledge_status(&app, &id, ann_id).await, "due");
    assert_eq!(bounty_status(&app, &id).await, "paying");
    sqlx::query("update donation_holds set lifted_at = now(), lifted_by = 'test'")
        .execute(&app.db)
        .await
        .unwrap();
    // Another copy of the API is charging: this one leaves it.
    let mut other = app.db.acquire().await.unwrap();
    sqlx::query("select pg_advisory_lock(7341001)")
        .execute(&mut *other)
        .await
        .unwrap();
    assert_eq!(slop_api::bounties::charge_due(&app.state).await.unwrap(), 0);
    assert_eq!(pledge_status(&app, &id, ann_id).await, "due");
    sqlx::query("select pg_advisory_unlock(7341001)")
        .execute(&mut *other)
        .await
        .unwrap();
    drop(other);
    assert_eq!(slop_api::bounties::charge_due(&app.state).await.unwrap(), 1);
    assert_eq!(bounty_status(&app, &id).await, "paid");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn expired_bounties_forget_their_saved_cards_at_stripe(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    tower(&app).await;
    let (ann_id, ann) = adult(&app, "Ann Backer", "US").await;
    let id = open_bounty(&app, &ann, "tower", "Fix lost saves").await;
    let session = stripe_pledge(&app, &ann, &id, 1000).await;
    card_saved(&app, &fake, &ann, &id, &session, "pm_ann").await;
    sqlx::query("update bounties set expires_at = now() - interval '1 day'")
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::bounties::charge_due(&app.state).await.unwrap();
    assert_eq!(bounty_status(&app, &id).await, "expired");
    assert_eq!(pledge_status(&app, &id, ann_id).await, "released");
    assert_eq!(
        fake.calls("POST", "v1/payment_methods/pm_ann/detach").len(),
        1
    );
}

/// Reports a bounty and has staff hide it. Returns the decision's ID.
async fn hide(app: &TestApp, reporter: &str, admin: &str, id: &str) -> Uuid {
    let res = app
        .send(
            req(Method::POST, "/v1/reports")
                .cookie(reporter)
                .json(json!({ "kind": "bounty", "subject": id, "reason": "spam", "details": "Not a real request.", "good_faith": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let queue = get_json(app, admin, "/v1/admin/queue").await;
    let report = queue["reports"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["link"] == format!("/bounties/{id}"))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/reports/{report}/act")).cookie(admin).json(json!({
                "action": "hide_bounty", "rule": "spam", "facts": "Not a bounty.", "outcome": "Hidden."
            })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{}", body_json(res).await);
    sqlx::query_scalar("select id from moderation_decisions where subject_id = $1")
        .bind(id.parse::<Uuid>().unwrap())
        .fetch_one(&app.db)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_hide_a_bounty_and_can_reverse_it_on_appeal(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    tower(&app).await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    let (_, ann) = adult(&app, "Ann Opener", "US").await;
    let (bob_id, bob) = adult(&app, "Bob Backer", "US").await;
    let id = open_bounty(&app, &ann, "tower", "Fix lost saves").await;
    pledge_ok(&app, &bob, &id, 1000).await;
    let decision = hide(&app, &bob, &admin, &id).await;
    assert_eq!(bounty_status(&app, &id).await, "hidden");
    assert_eq!(pledge_status(&app, &id, bob_id).await, "released");
    assert_eq!(
        app.send(req(Method::GET, format!("/v1/bounties/{id}")))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert!(
        body_json(app.send(req(Method::GET, "/v1/apps/tower/bounties")).await)
            .await
            .as_array()
            .unwrap()
            .is_empty()
    );
    // Ann appeals; staff reverse it.
    let res = app
        .send(
            req(Method::POST, format!("/v1/me/decisions/{decision}/appeal"))
                .cookie(&ann)
                .json(json!({ "appeal": "It's a real request: saves really vanish." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{:?}", body_json(res).await);
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/decisions/{decision}/appeal"),
            )
            .cookie(&admin)
            .json(json!({ "reverse": true, "reason": "It is a real bug." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(bounty_status(&app, &id).await, "open");
    assert_eq!(
        app.send(req(Method::GET, format!("/v1/bounties/{id}")))
            .await
            .status(),
        StatusCode::OK
    );
}

/// b31: hiding a bounty also forgets its saved cards at Stripe and stops any charges still due;
/// reversing it doesn't leave it to expire at once.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn hiding_a_bounty_lets_go_of_its_cards_and_charges(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let ((_, maker), (forker_id, forker)) = tower(&app).await;
    let (_, admin) = make_admin(&app, "Ada Admin").await;
    let (_, ann) = adult(&app, "Ann Backer", "US").await;
    let (bob_id, bob) = adult(&app, "Bob Backer", "US").await;
    // An open one with a saved card: hiding it forgets the card.
    let open = open_bounty(&app, &ann, "tower", "Fix lost saves").await;
    let session = stripe_pledge(&app, &ann, &open, 1000).await;
    card_saved(&app, &fake, &ann, &open, &session, "pm_ann").await;
    let decision = hide(&app, &bob, &admin, &open).await;
    assert_eq!(
        fake.calls("POST", "v1/payment_methods/pm_ann/detach").len(),
        1
    );
    // Reversed on appeal: open, and still open after the charger's next run.
    app.send(
        req(Method::POST, format!("/v1/me/decisions/{decision}/appeal"))
            .cookie(&ann)
            .json(json!({ "appeal": "It's real." })),
    )
    .await;
    app.send(
        req(
            Method::POST,
            format!("/v1/admin/decisions/{decision}/appeal"),
        )
        .cookie(&admin)
        .json(json!({ "reverse": true, "reason": "It is." })),
    )
    .await;
    slop_api::bounties::charge_due(&app.state).await.unwrap();
    assert_eq!(bounty_status(&app, &open).await, "open");
    // One being paid: hiding it stops the charges still due.
    let paying = accepted_on_stripe(
        &app,
        &fake,
        &maker,
        forker_id,
        &forker,
        "Colourblind mode",
        &[(&bob, "pm_bob")],
    )
    .await;
    hide(&app, &ann, &admin, &paying).await;
    slop_api::bounties::charge_due(&app.state).await.unwrap();
    assert_eq!(pledge_status(&app, &paying, bob_id).await, "released");
    assert!(fake.calls("POST", "v1/payment_intents").is_empty());
    assert_eq!(
        fake.calls("POST", "v1/payment_methods/pm_bob/detach").len(),
        1
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_makers_agent_reads_and_claims_bounties_with_a_publish_token(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let ((maker_id, maker), _) = tower(&app).await;
    listed_with(&app, maker_id, "other", "released", "").await;
    let (_, ann) = adult(&app, "Ann Backer", "US").await;
    let id = open_bounty(&app, &ann, "tower", "Fix lost saves").await;
    pledge_ok(&app, &ann, &id, 1000).await;
    let token = |body: Value| {
        let app = &app;
        let maker = maker.clone();
        async move {
            let res = app
                .send(
                    req(Method::POST, "/v1/me/maker-tokens")
                        .cookie(&maker)
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
    let full = token(json!({ "name": "Claude", "scope": "publish" })).await;
    let reader = token(json!({ "name": "Watcher", "scope": "read" })).await;
    let other_only = token(json!({ "name": "Other", "scope": "publish", "apps": ["other"] })).await;
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
    let (error, text) = call(&reader, "list_bounties", json!({})).await;
    assert!(!error && text.contains("Fix lost saves"), "{text}");
    let (error, text) = call(&reader, "get_bounty", json!({ "id": id })).await;
    assert!(!error && text.contains("\"pledged_minor\":1000"), "{text}");
    let delivery = json!({ "id": id, "app_slug": "tower", "note": "Saves are safe in 1.0.0." });
    let (error, _) = call(&reader, "claim_bounty", delivery.clone()).await;
    assert!(error, "a read token can't claim");
    let (error, _) = call(&other_only, "claim_bounty", delivery.clone()).await;
    assert!(error, "a token for another app can't claim with this one");
    assert_eq!(bounty_status(&app, &id).await, "open");
    let (error, text) = call(&full, "claim_bounty", delivery).await;
    assert!(!error, "{text}");
    assert_eq!(bounty_status(&app, &id).await, "claimed");
}
