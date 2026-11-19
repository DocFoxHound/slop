// SPDX-License-Identifier: AGPL-3.0-or-later
//! App of the Month (awards.rs, migration 0105, DESIGN §5): who may nominate, vote and confirm
//! (staff who review; the owner, signed in, for winners and prizes), the three-vote and
//! three-winner limits, never your own app, which apps can win, what the winners' makers are
//! told, and the published awards on store pages, profiles and /awards.

use crate::*;

// ---------------------------------------------------------------------------------------------
// Helpers for this area

/// A staff member with one role.
async fn staff(app: &TestApp, name: &str, role: &str) -> (Uuid, String) {
    let (id, cookie) = app.user(name, "US").await;
    sqlx::query("update users set is_admin = true where id = $1")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("insert into staff_roles (user_id, role, granted_by) values ($1, $2, 'test')")
        .bind(id)
        .bind(role)
        .execute(&app.db)
        .await
        .unwrap();
    (id, cookie)
}

async fn admin_token(app: &TestApp, cookie: &str, scope: &str) -> String {
    let res = app
        .send(
            req(Method::POST, "/v1/admin/tokens")
                .cookie(cookie)
                .json(json!({ "name": format!("{scope} agent"), "scope": scope, "days": 7 })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    format!("Bearer {}", body_json(res).await["token"].as_str().unwrap())
}

fn this_month() -> String {
    let d = OffsetDateTime::now_utc().date();
    format!("{:04}-{:02}", d.year(), u8::from(d.month()))
}

/// "November 2026" for this month.
fn this_month_label() -> String {
    let d = OffsetDateTime::now_utc().date();
    format!("{} {}", d.month(), d.year())
}

async fn nominate(app: &TestApp, cookie: &str, slug: &str) -> Response {
    app.send(
        req(Method::POST, "/v1/admin/awards/nominations")
            .cookie(cookie)
            .json(json!({ "slug": slug, "note": "Players love it." })),
    )
    .await
}

async fn vote(app: &TestApp, cookie: &str, slug: &str) -> Response {
    app.send(req(Method::PUT, format!("/v1/admin/awards/votes/{slug}")).cookie(cookie))
        .await
}

async fn confirm(app: &TestApp, auth: Req, winners: Value) -> Response {
    app.send(auth.json(json!({ "winners": winners }))).await
}

fn confirm_req(month: &str) -> Req {
    req(Method::PUT, format!("/v1/admin/awards/{month}/winners"))
}

async fn month_view(app: &TestApp, cookie: &str) -> Value {
    let res = app
        .send(req(Method::GET, "/v1/admin/awards").cookie(cookie))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    body_json(res).await
}

async fn error_message(res: Response) -> String {
    body_json(res).await["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

// ---------------------------------------------------------------------------------------------

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn only_reviewing_staff_nominate_and_vote_and_never_for_their_own_apps(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (rita_id, rita) = staff(&app, "Rita Reviewer", "reviewer").await;
    let (_, sue) = staff(&app, "Sue Support", "support").await;
    let (mia, player) = app.user("Mia Maker", "US").await;
    for slug in ["tea-shop", "moss", "lanterns", "kites"] {
        listed_with(&app, mia, slug, "released", "").await;
    }
    listed_with(&app, mia, "soon", "coming_soon", "").await;
    listed_with(&app, mia, "betty", "beta", "").await;
    listed_with(&app, mia, "drafty", "released", "status = 'draft'").await;
    listed_with(&app, mia, "gone", "released", "status = 'delisted'").await;
    listed_with(&app, mia, "secret", "released", "visibility = 'private'").await;
    listed_with(&app, rita_id, "ritas-game", "released", "").await;

    // Not staff: nothing here. Staff who only look: refused.
    for r in [
        req(Method::GET, "/v1/admin/awards").cookie(&player),
        req(Method::POST, "/v1/admin/awards/nominations")
            .cookie(&player)
            .json(json!({ "slug": "tea-shop" })),
        req(Method::GET, "/v1/admin/awards"),
    ] {
        assert_eq!(app.send(r).await.status(), StatusCode::NOT_FOUND);
    }
    assert_eq!(
        app.send(req(Method::GET, "/v1/admin/awards").cookie(&sue))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        nominate(&app, &sue, "tea-shop").await.status(),
        StatusCode::FORBIDDEN
    );
    // A read token neither.
    let read = admin_token(&app, &owner, "read").await;
    let res = app
        .send(
            req(Method::POST, "/v1/admin/awards/nominations")
                .header("authorization", read)
                .json(json!({ "slug": "tea-shop" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // Only finished (Released) public apps in the store: never Coming soon or Beta.
    assert_eq!(
        nominate(&app, &rita, "nowhere").await.status(),
        StatusCode::NOT_FOUND
    );
    for slug in ["soon", "betty", "drafty", "gone", "secret"] {
        let res = nominate(&app, &rita, slug).await;
        assert_eq!(res.status(), StatusCode::CONFLICT, "{slug}");
    }
    // Never your own app: nominating it, or voting for it once someone else did.
    let res = nominate(&app, &rita, "ritas-game").await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert!(error_message(res).await.contains("you make this app"));
    assert_eq!(
        nominate(&app, &owner, "ritas-game").await.status(),
        StatusCode::CREATED
    );
    assert_eq!(
        vote(&app, &rita, "ritas-game").await.status(),
        StatusCode::FORBIDDEN
    );

    let res = nominate(&app, &rita, "tea-shop").await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let view = body_json(res).await;
    assert_eq!(view["month"], this_month());
    assert_eq!(view["current"], true);
    assert_eq!(view["can_confirm"], false);
    let res = nominate(&app, &owner, "tea-shop").await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(error_message(res).await.contains("already nominated"));
    // Voting is for nominated apps only.
    assert_eq!(
        vote(&app, &rita, "moss").await.status(),
        StatusCode::NOT_FOUND
    );

    let view = month_view(&app, &rita).await;
    let noms = view["nominations"].as_array().unwrap();
    assert_eq!(noms.len(), 2);
    let ritas = noms.iter().find(|n| n["slug"] == "ritas-game").unwrap();
    assert_eq!(ritas["mine"], true);
    let tea = noms.iter().find(|n| n["slug"] == "tea-shop").unwrap();
    assert_eq!(tea["nominated_by"], "Rita Reviewer");
    assert_eq!(tea["note"], "Players love it.");
    assert_eq!(tea["eligible"], true);

    // Everything is in the audit log.
    let logged: i64 =
        sqlx::query_scalar("select count(*) from audit_log where action = 'awards.nominate'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(logged, 2);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn each_staff_member_has_three_votes_a_month(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, rita) = staff(&app, "Rita Reviewer", "reviewer").await;
    let (mia, _) = app.user("Mia Maker", "US").await;
    let slugs = ["tea-shop", "moss", "lanterns", "kites", "owls"];
    for slug in slugs {
        listed_with(&app, mia, slug, "released", "").await;
        assert_eq!(
            nominate(&app, &owner, slug).await.status(),
            StatusCode::CREATED
        );
    }
    for slug in &slugs[..3] {
        assert_eq!(
            vote(&app, &rita, slug).await.status(),
            StatusCode::NO_CONTENT
        );
    }
    // Voting again for the same app changes nothing.
    assert_eq!(
        vote(&app, &rita, "tea-shop").await.status(),
        StatusCode::NO_CONTENT
    );
    let res = vote(&app, &rita, "kites").await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(error_message(res).await.contains("used your 3 votes"));
    // Taking one back frees it.
    let res = app
        .send(req(Method::DELETE, "/v1/admin/awards/votes/moss").cookie(&rita))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        vote(&app, &rita, "kites").await.status(),
        StatusCode::NO_CONTENT
    );
    // Other staff have their own three.
    assert_eq!(
        vote(&app, &owner, "kites").await.status(),
        StatusCode::NO_CONTENT
    );

    let view = month_view(&app, &rita).await;
    assert_eq!(view["my_votes"], 3);
    assert_eq!(view["max_votes"], 3);
    let noms = view["nominations"].as_array().unwrap();
    // Most votes first.
    assert_eq!(noms[0]["slug"], "kites");
    assert_eq!(noms[0]["votes"], 2);
    assert_eq!(noms[0]["voters"], json!(["Rita Reviewer", "Olive Owner"]));
    assert_eq!(noms[0]["my_vote"], true);
    let moss = noms.iter().find(|n| n["slug"] == "moss").unwrap();
    assert_eq!(
        (moss["votes"].clone(), moss["my_vote"].clone()),
        (json!(0), json!(false))
    );

    // Many votes at once still stop at three (counted under a lock).
    let (_, ned) = staff(&app, "Ned Reviewer", "reviewer").await;
    let (a, b, c, d, e) = tokio::join!(
        vote(&app, &ned, "tea-shop"),
        vote(&app, &ned, "moss"),
        vote(&app, &ned, "lanterns"),
        vote(&app, &ned, "kites"),
        vote(&app, &ned, "owls"),
    );
    let ok = [a, b, c, d, e]
        .iter()
        .filter(|r| r.status() == StatusCode::NO_CONTENT)
        .count();
    assert_eq!(ok, 3);
    let theirs: i64 = sqlx::query_scalar(
        "select count(*) from award_votes v join users u on u.id = v.voter_id where u.display_name = 'Ned Reviewer'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(theirs, 3);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_owner_confirms_up_to_three_winners_and_their_makers_are_told(db: PgPool) {
    let app = test_app(db).await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, rita) = staff(&app, "Rita Reviewer", "reviewer").await;
    let (_, mod_cookie) = staff(&app, "Mo Moderator", "moderator").await;
    let (mia, _) = app.user("Mia Maker", "US").await;
    let (max, _) = app.user("Max Maker", "US").await;
    listed_with(&app, mia, "tea-shop", "released", "").await;
    listed_with(&app, mia, "moss", "released", "").await;
    listed_with(&app, max, "lanterns", "released", "").await;
    listed_with(&app, max, "kites", "released", "").await;
    listed_with(&app, max, "unnominated", "released", "").await;
    for slug in ["tea-shop", "moss", "lanterns", "kites"] {
        assert_eq!(
            nominate(&app, &rita, slug).await.status(),
            StatusCode::CREATED
        );
    }
    let month = this_month();
    let label = this_month_label();
    let two = json!([{ "slug": "tea-shop" }, { "slug": "lanterns", "amount_minor": 3000, "currency": "EUR", "blurb": "Glows." }]);

    // Staff who aren't the owner can't, and neither can any token, even the owner's.
    for cookie in [&rita, &mod_cookie] {
        let res = confirm(&app, confirm_req(&month).cookie(cookie), two.clone()).await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }
    let moderate = admin_token(&app, &owner, "moderate").await;
    let res = confirm(
        &app,
        confirm_req(&month).header("authorization", moderate.clone()),
        two.clone(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    // People who aren't staff don't find it.
    let (_, player) = app.user("Pat Player", "US").await;
    let res = confirm(&app, confirm_req(&month).cookie(&player), two.clone()).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // At most three; each nominated for that month; not a month still to come.
    let four = json!([{ "slug": "tea-shop" }, { "slug": "moss" }, { "slug": "lanterns" }, { "slug": "kites" }]);
    let res = confirm(&app, confirm_req(&month).cookie(&owner), four).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let res = confirm(
        &app,
        confirm_req(&month).cookie(&owner),
        json!([{ "slug": "unnominated" }]),
    )
    .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(error_message(res).await.contains("wasn't nominated"));
    let res = confirm(&app, confirm_req("2999-01").cookie(&owner), two.clone()).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let res = confirm(
        &app,
        confirm_req(&month).cookie(&owner),
        json!([{ "slug": "tea-shop", "amount_minor": 50 }]),
    )
    .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("select count(*) from app_awards")
            .fetch_one(&app.db)
            .await
            .unwrap(),
        0
    );

    let res = confirm(&app, confirm_req(&month).cookie(&owner), two.clone()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let view = body_json(res).await;
    assert_eq!(view["can_confirm"], true);
    let winners = view["winners"].as_array().unwrap();
    assert_eq!(winners.len(), 2);
    assert_eq!(winners[0]["slug"], "tea-shop");
    assert_eq!(winners[0]["position"], 1);
    assert_eq!(
        (
            winners[0]["amount_minor"].clone(),
            winners[0]["currency"].clone()
        ),
        (json!(2500), json!("usd"))
    );
    assert_eq!(
        (
            winners[1]["amount_minor"].clone(),
            winners[1]["currency"].clone(),
            winners[1]["blurb"].clone()
        ),
        (json!(3000), json!("eur"), json!("Glows."))
    );
    assert_eq!(winners[0]["confirmed_by"], "Olive Owner");
    assert_eq!(winners[0]["prize_paid_at"], Value::Null);
    // Donations aren't on for these apps (nor Stripe on this server).
    assert_eq!(winners[0]["accepts_donations"], false);
    assert_eq!(winners[0]["donation_currency"], "usd");

    // Each winning maker is told the month, the award and how it's paid.
    let told: Vec<(Uuid, String, String, String, String)> = sqlx::query_as(
        "select user_id, category, title, body, link from notifications where title like '%App of the Month%' order by title",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(told.len(), 2, "{told:?}");
    let (who, category, title, body, link) = &told[0];
    assert_eq!((who, category.as_str()), (&max, "apps"));
    assert_eq!(title, &format!("lanterns is App of the Month · {label}"));
    assert!(body.contains("€30.00 award"), "{body}");
    assert!(
        body.contains("donation from SLAPorSLOP to lanterns"),
        "{body}"
    );
    assert_eq!(link, "/app/lanterns");
    assert_eq!(told[1].0, mia);
    assert!(told[1].3.contains("$25.00 award"), "{}", told[1].3);

    // Replacing them: the maker taken off hears it; an unchanged winner isn't told again.
    let res = confirm(
        &app,
        confirm_req(&month).cookie(&owner),
        json!([{ "slug": "tea-shop" }, { "slug": "moss" }, { "slug": "kites" }]),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let mias: Vec<String> =
        sqlx::query_scalar("select title from notifications where user_id = $1 order by title")
            .bind(mia)
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(
        mias,
        vec![
            format!("moss is App of the Month · {label}"),
            format!("tea-shop is App of the Month · {label}")
        ]
    );
    let maxs: Vec<(String, i32)> = sqlx::query_as(
        "select title, count from notifications where user_id = $1 order by created_at",
    )
    .bind(max)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(maxs.len(), 2, "{maxs:?}");
    assert!(
        maxs.iter()
            .any(|(t, _)| t == &format!("kites is App of the Month · {label}"))
    );
    // lanterns' unread notice now says it was corrected.
    assert!(
        maxs.iter()
            .any(|(t, _)| t == &format!("App of the Month for {label} was corrected"))
    );

    // Marking a prize paid: the owner, signed in. It fixes the month's winners.
    let paid = |slug: &str, paid: bool| {
        req(
            Method::PUT,
            format!("/v1/admin/awards/{month}/winners/{slug}/paid"),
        )
        .json(json!({ "paid": paid }))
    };
    assert_eq!(
        app.send(paid("tea-shop", true).cookie(&rita))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(paid("tea-shop", true).header("authorization", moderate))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(paid("lanterns", true).cookie(&owner))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let res = app.send(paid("tea-shop", true).cookie(&owner)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let view = body_json(res).await;
    assert!(view["winners"][0]["prize_paid_at"].is_string());
    let res = confirm(&app, confirm_req(&month).cookie(&owner), two.clone()).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(error_message(res).await.contains("already marked paid"));
    // Unmarked (a mistake), they can change again.
    let res = app.send(paid("tea-shop", false).cookie(&owner)).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        body_json(res).await["winners"][0]["prize_paid_at"],
        Value::Null
    );

    let actions: Vec<String> = sqlx::query_scalar(
        "select action from audit_log where action like 'awards.%' and action <> 'awards.nominate' order by id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        actions,
        [
            "awards.confirm",
            "awards.confirm",
            "awards.prize_paid",
            "awards.prize_unpaid"
        ]
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_owner_cannot_confirm_their_own_app(db: PgPool) {
    let app = test_app(db).await;
    let (olive, owner) = make_admin(&app, "Olive Owner").await;
    let (_, rita) = staff(&app, "Rita Reviewer", "reviewer").await;
    listed_with(&app, olive, "olives-game", "released", "").await;
    assert_eq!(
        nominate(&app, &rita, "olives-game").await.status(),
        StatusCode::CREATED
    );
    let month = this_month();
    let res = confirm(
        &app,
        confirm_req(&month).cookie(&owner),
        json!([{ "slug": "olives-game" }]),
    )
    .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

/// An award, as the owner confirmed it, straight into the database (for months gone by).
async fn award(app: &TestApp, month: &str, slug: &str, position: i16, amount: i32, currency: &str) {
    sqlx::query(
        "insert into app_awards (month, app_id, maker_id, position, amount_minor, currency, blurb)
         select to_date($1 || '-01', 'YYYY-MM-DD'), id, maker_id, $3, $4, $5, 'So good.' from apps where slug = $2",
    )
    .bind(month)
    .bind(slug)
    .bind(position)
    .bind(amount)
    .bind(currency)
    .execute(&app.db)
    .await
    .unwrap();
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn awards_show_on_the_store_page_the_profile_and_the_awards_page(db: PgPool) {
    let app = test_app(db).await;
    let (mia, _) = app.user("Mia Maker", "US").await;
    let (max, _) = app.user("Max Maker", "US").await;
    listed_with(&app, mia, "tea-shop", "released", "").await;
    listed_with(&app, mia, "moss", "released", "").await;
    listed_with(&app, mia, "gone", "released", "").await;
    listed_with(&app, max, "lanterns", "released", "").await;
    award(&app, "2026-09", "tea-shop", 1, 2500, "usd").await;
    award(&app, "2026-10", "tea-shop", 2, 2000, "gbp").await;
    award(&app, "2026-10", "lanterns", 1, 2500, "usd").await;
    award(&app, "2026-10", "moss", 3, 2500, "usd").await;
    award(&app, "2026-08", "gone", 1, 2500, "usd").await;
    sqlx::query("update apps set status = 'delisted' where slug = 'gone'")
        .execute(&app.db)
        .await
        .unwrap();

    // The store page: every month it won, newest first, with the award.
    let page = body_json(app.send(req(Method::GET, "/v1/apps/tea-shop")).await).await;
    assert_eq!(
        page["awards"],
        json!([
            { "month": "2026-10", "position": 2, "amount_minor": 2000, "currency": "gbp", "blurb": "So good." },
            { "month": "2026-09", "position": 1, "amount_minor": 2500, "currency": "usd", "blurb": "So good." }
        ])
    );

    // The maker's profile counts them across months and apps, only apps in the store.
    let profile = body_json(app.send(req(Method::GET, format!("/v1/users/{mia}"))).await).await;
    assert_eq!(profile["award_count"], 3);
    let months: Vec<&str> = profile["awards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["month"].as_str().unwrap())
        .collect();
    assert_eq!(months, ["2026-10", "2026-10", "2026-09"]);
    assert_eq!(profile["awards"][0]["slug"], "tea-shop");
    assert_eq!(profile["awards"][0]["amount_minor"], 2000);
    let profile = body_json(app.send(req(Method::GET, format!("/v1/users/{max}"))).await).await;
    assert_eq!(profile["award_count"], 1);

    // The awards page: winners by month, in order, signed out too. Not the delisted one.
    let all = body_json(app.send(req(Method::GET, "/v1/awards")).await).await;
    let months = all.as_array().unwrap();
    assert_eq!(months.len(), 2);
    assert_eq!(months[0]["month"], "2026-10");
    let slugs: Vec<&str> = months[0]["winners"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["slug"].as_str().unwrap())
        .collect();
    assert_eq!(slugs, ["lanterns", "tea-shop", "moss"]);
    let w = &months[0]["winners"][1];
    assert_eq!(
        (
            w["amount_minor"].clone(),
            w["currency"].clone(),
            w["maker_name"].clone()
        ),
        (json!(2000), json!("gbp"), json!("Mia Maker"))
    );
    assert_eq!(w["maker_id"], mia.to_string());
    assert_eq!(months[1]["month"], "2026-09");
    assert!(!all.to_string().contains("\"gone\""));
}
