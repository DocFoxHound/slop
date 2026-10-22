// SPDX-License-Identifier: AGPL-3.0-or-later
//! Part 1 of the people tests: the directory and profiles, portfolios and their review, Stripe
//! onboarding, every step of a job, mediation, the Stripe webhook's People events, and friends
//! and blocks.

use crate::*;

// ---------------------------------------------------------------------------------------------
// Helpers

/// Simulated payments, and room for long walk-throughs under the per-IP limits.
async fn people_app(db: PgPool) -> TestApp {
    test_app_with(db, |c| {
        c.payments.simulated = true;
        roomy(c);
    })
    .await
}

fn roomy(c: &mut Config) {
    c.rate_limits.write_per_minute = 10_000;
    c.rate_limits.read_per_minute = 10_000;
}

/// The status and JSON body of a request.
async fn call(app: &TestApp, r: Req) -> (StatusCode, Value) {
    let res = app.send(r).await;
    let status = res.status();
    (status, body_json(res).await)
}

/// The text body of a request, with its status and Content-Disposition.
async fn call_text(app: &TestApp, r: Req) -> (StatusCode, Option<String>, String) {
    let res = app.send(r).await;
    let status = res.status();
    let disposition = res
        .headers()
        .get(header::CONTENT_DISPOSITION)
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        disposition,
        String::from_utf8(bytes.to_vec()).unwrap(),
    )
}

fn post(path: impl Into<String>, cookie: &str, body: Value) -> Req {
    req(Method::POST, path).cookie(cookie).json(body)
}

fn fetch(path: impl Into<String>, cookie: &str) -> Req {
    req(Method::GET, path).cookie(cookie)
}

/// The `error.field` of a 422.
fn field(body: &Value) -> &str {
    body["error"]["field"].as_str().unwrap_or("")
}

async fn count(app: &TestApp, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn exec(app: &TestApp, sql: &str) {
    sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
        .execute(&app.db)
        .await
        .unwrap();
}

/// Notification titles a user has, newest first.
async fn notices(app: &TestApp, user: Uuid) -> Vec<String> {
    sqlx::query_scalar(
        "select title from notifications where user_id = $1 order by created_at desc",
    )
    .bind(user)
    .fetch_all(&app.db)
    .await
    .unwrap()
}

/// A staff member with one role (staff.rs), not an owner.
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

/// A creative whose profile is live (an approved portfolio set) and who can be paid, made
/// straight in the database rather than through onboarding.
async fn creative_with(app: &TestApp, handle: &str, country: &str, p: Value) -> (Uuid, String) {
    let (id, cookie) = app.user(handle, country).await;
    let mut p = p;
    p["handle"] = json!(handle);
    p["confirm_adult"] = json!(true);
    let (status, body) = call(
        app,
        req(Method::PUT, "/v1/me/creative").cookie(&cookie).json(p),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let set = Uuid::now_v7();
    sqlx::query(
        "insert into portfolio_sets (id, creative_id, repo_url, commit_sha, works, fingerprint, status, source)
         values ($1, $2, 'https://github.com/x/y', repeat('c', 40), '[]', 'f', 'approved', 'sim')",
    )
    .bind(set)
    .bind(id)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query(
        "update creatives set portfolio_set_id = $2, stripe_status = 'active', stripe_account_id = 'acct_' || $3
         where user_id = $1",
    )
    .bind(id)
    .bind(set)
    .bind(handle)
    .execute(&app.db)
    .await
    .unwrap();
    (id, cookie)
}

/// A creative ("vera", a trader in the US) and a client with a project ("tavern"), on a server
/// with simulated payments.
struct Hiring {
    app: TestApp,
    creative: Uuid,
    vera: String,
    client: Uuid,
    tavern: String,
}

async fn hiring(db: PgPool, client_country: &str) -> Hiring {
    let app = people_app(db).await;
    hiring_on(app, client_country).await
}

async fn hiring_on(app: TestApp, client_country: &str) -> Hiring {
    let (creative, vera) = live_creative(&app, "vera", "US", true).await;
    let (client, tavern) = app.user("Tavern Dev", client_country).await;
    listed_with(&app, client, "tavern", "released", "").await;
    Hiring {
        app,
        creative,
        vera,
        client,
        tavern,
    }
}

fn hire_body(extra: Value) -> Value {
    let mut body = json!({
        "app_slug": "tavern", "skill": "pixel-art", "request_md": "A knight, 4 directions.",
        "confirm_adult": true, "exact_wording_ack": true
    });
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    body
}

const STEPS: [&str; 6] = [
    "requested",
    "offered",
    "accepted",
    "in_progress",
    "delivered",
    "completed",
];

impl Hiring {
    async fn hire(&self, extra: Value) -> (StatusCode, Value) {
        call(
            &self.app,
            post("/v1/people/vera/hire", &self.tavern, hire_body(extra)),
        )
        .await
    }

    /// A job taken through the steps to `step`: requested, offered, accepted (whatever accepting
    /// leads to), in_progress (paid what's due on accepting), delivered, completed.
    async fn job(&self, payment: &str, step: &str) -> String {
        self.job_with(sprite_offer(payment), step).await
    }

    async fn job_with(&self, offer: Value, step: &str) -> String {
        drive(
            &self.app,
            "vera",
            &self.vera,
            &self.tavern,
            json!({}),
            offer,
            step,
        )
        .await
    }

    /// POST /v1/jobs/{id}/{step}.
    async fn step(&self, id: &str, cookie: &str, step: &str, body: Value) -> (StatusCode, Value) {
        call(
            &self.app,
            post(format!("/v1/jobs/{id}/{step}"), cookie, body),
        )
        .await
    }

    /// A step that must work; the job (or the pay answer) it returns.
    async fn ok(&self, id: &str, cookie: &str, step: &str, body: Value) -> Value {
        let (status, body) = self.step(id, cookie, step, body).await;
        assert_eq!(status, StatusCode::OK, "{step}: {body}");
        body
    }

    /// A step that must be refused with `want`.
    async fn refused(&self, id: &str, cookie: &str, step: &str, body: Value, want: StatusCode) {
        let (status, body) = self.step(id, cookie, step, body).await;
        assert_eq!(status, want, "{step}: {body}");
    }

    async fn view(&self, id: &str, cookie: &str) -> Value {
        let (status, body) = call(&self.app, fetch(format!("/v1/jobs/{id}"), cookie)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    /// The job's status straight from the database.
    async fn status(&self, id: &str) -> String {
        sqlx::query_scalar("select status from jobs where id = $1::uuid")
            .bind(id)
            .fetch_one(&self.app.db)
            .await
            .unwrap()
    }
}

/// A job between a creative and a client taken through the steps to `step` (see `Hiring::job`).
async fn drive(
    app: &TestApp,
    handle: &str,
    creative: &str,
    client: &str,
    hire: Value,
    offer: Value,
    step: &str,
) -> String {
    let (status, job) = call(
        app,
        post(format!("/v1/people/{handle}/hire"), client, hire_body(hire)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{job}");
    let id = job["id"].as_str().unwrap().to_string();
    let ok = |cookie: &str, path: &str, body: Value| {
        let r = post(format!("/v1/jobs/{id}/{path}"), cookie, body);
        let path = path.to_string();
        async move {
            let (status, body) = call(app, r).await;
            assert_eq!(status, StatusCode::OK, "{path}: {body}");
            body
        }
    };
    let want = STEPS.iter().position(|s| *s == step).unwrap();
    let at = |s: &str| STEPS.iter().position(|x| *x == s).unwrap() <= want;
    if at("offered") {
        ok(creative, "offer", offer).await;
    }
    let mut job = job;
    if at("accepted") {
        job = ok(
            client,
            "accept",
            json!({ "offer_count": 1, "exact_terms_ack": true }),
        )
        .await;
    }
    if at("in_progress") && job["status"] == "awaiting_payment" {
        ok(client, "pay", json!({})).await;
    }
    if at("delivered") {
        job = ok(
            creative,
            "deliver",
            json!({ "links": ["https://drive.example/preview.png"] }),
        )
        .await;
    }
    if at("completed") {
        let (_, mine) = call(app, fetch(format!("/v1/jobs/{id}"), client)).await;
        if actions(&mine).contains(&"pay") {
            ok(client, "pay", json!({})).await;
            job = ok(
                creative,
                "final-files",
                json!({ "links": ["https://drive.example/final.zip"] }),
            )
            .await;
        } else {
            job = ok(client, "accept-delivery", json!({})).await;
        }
        assert_eq!(job["status"], "completed");
    }
    id
}

fn actions(job: &Value) -> Vec<&str> {
    job["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect()
}

fn handles(v: &Value) -> Vec<String> {
    let mut h: Vec<String> = v["people"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["handle"].as_str().unwrap().to_string())
        .collect();
    h.sort();
    h
}

fn skill_count(v: &Value, slug: &str) -> i64 {
    v["skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["slug"] == slug)
        .unwrap()["count"]
        .as_i64()
        .unwrap()
}

// ---------------------------------------------------------------------------------------------
// The directory and profiles (people-001 … people-005)

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_directory_filters_searches_and_leaves_out_hidden_suspended_and_closed_creatives(
    db: PgPool,
) {
    let app = people_app(db).await;
    // Vera: individual, no AI, taking work. Forge: a studio, AI-assisted, not taking work.
    // Genny: mostly AI.
    creative_with(&app, "vera", "US", profile("vera", true)).await;
    let mut studio = profile("x", true);
    studio["name"] = json!("Pixel_Forge 100%");
    studio["kind"] = json!("studio");
    studio["team_size"] = json!(4);
    studio["ai_use"] = json!("assisted");
    studio["taking_work"] = json!(false);
    studio["headline"] = json!("Sprites for hire");
    studio["skills"] = json!(["music"]);
    studio["services"] = json!([]);
    creative_with(&app, "forge", "US", studio).await;
    let mut heavy = profile("x", false);
    heavy["ai_use"] = json!("heavy");
    heavy["name"] = json!("Gen Nakamura");
    heavy["headline"] = json!("Generated art");
    creative_with(&app, "genny", "US", heavy).await;
    let dir = |q: &str| {
        let q = q.to_string();
        let app = &app;
        async move {
            let (status, body) = call(app, req(Method::GET, format!("/v1/people{q}"))).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            body
        }
    };
    assert_eq!(handles(&dir("").await), ["forge", "genny", "vera"]);
    assert_eq!(handles(&dir("?ai=none").await), ["vera"]);
    assert_eq!(handles(&dir("?ai=assisted").await), ["forge", "vera"]);
    assert_eq!(handles(&dir("?studios=true").await), ["forge"]);
    assert_eq!(handles(&dir("?taking_work=true").await), ["genny", "vera"]);
    // Text search over the name, headline and handle; % and _ are dropped from the search.
    assert_eq!(handles(&dir("?q=sprites").await), ["forge"]);
    assert_eq!(handles(&dir("?q=GENERATED").await), ["genny"]);
    assert_eq!(handles(&dir("?q=ver").await), ["vera"]);
    assert_eq!(
        handles(&dir("?q=%25").await),
        ["forge", "genny", "vera"],
        "a lone % isn't a wildcard: it's dropped, leaving an empty search"
    );
    assert!(
        handles(&dir("?q=Pixel_Forge").await).is_empty(),
        "the _ is dropped, not a one-character wildcard"
    );
    assert_eq!(handles(&dir("?q=100%25").await), ["forge"]);
    // Skill counts cover everyone open to hiring.
    let all = dir("").await;
    assert_eq!(skill_count(&all, "music"), 3);
    assert_eq!(skill_count(&all, "voice"), 0);

    // Hidden by staff, suspended, closed, or waiting for a first approved portfolio: not listed.
    exec(
        &app,
        "update creatives set hidden_reason = 'Stolen art' where handle = 'genny'",
    )
    .await;
    exec(
        &app,
        "update users set suspended_at = now() where id = (select user_id from creatives where handle = 'forge')",
    )
    .await;
    assert_eq!(handles(&dir("").await), ["vera"]);
    let (closed_id, _) = creative_with(&app, "gone", "US", profile("x", true)).await;
    slop_api::account::close_account(&app.db, closed_id, "test")
        .await
        .unwrap();
    let (waiting, waiting_cookie) = app.user("waits", "US").await;
    let mut p = profile("waits", true);
    p["confirm_adult"] = json!(true);
    app.send(
        req(Method::PUT, "/v1/me/creative")
            .cookie(&waiting_cookie)
            .json(p),
    )
    .await;
    sqlx::query(
        "insert into portfolio_sets (id, creative_id, repo_url, commit_sha, works, fingerprint, status, source)
         values ($1, $2, 'https://github.com/x/y', repeat('c', 40), '[]', 'f', 'pending', 'sim')",
    )
    .bind(Uuid::now_v7())
    .bind(waiting)
    .execute(&app.db)
    .await
    .unwrap();
    let all = dir("").await;
    assert_eq!(handles(&all), ["vera"]);
    assert_eq!(skill_count(&all, "music"), 1);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_directory_is_a_daily_shuffle_of_at_most_200(db: PgPool) {
    let app = people_app(db).await;
    // 205 live creatives straight in the database.
    exec(
        &app,
        "insert into users (id, display_name, country_code, age_16_confirmed_at)
         select gen_random_uuid(), 'P' || n, 'US', now() from generate_series(1, 205) n",
    )
    .await;
    exec(
        &app,
        "insert into creatives (user_id, handle, name, kind, skills, ai_use, trader, taking_work)
         select id, 'pp' || substr(display_name, 2), display_name, 'individual', '{pixel-art}', 'none', false, true
         from users where display_name ~ '^P[0-9]+$'",
    )
    .await;
    exec(
        &app,
        "insert into portfolio_sets (id, creative_id, repo_url, commit_sha, works, fingerprint, status, source)
         select gen_random_uuid(), user_id, 'https://github.com/x/y', repeat('c', 40), '[]', 'f', 'approved', 'sim'
         from creatives",
    )
    .await;
    exec(
        &app,
        "update creatives c set portfolio_set_id = ps.id from portfolio_sets ps where ps.creative_id = c.user_id",
    )
    .await;
    let order = |body: &Value| -> Vec<String> {
        body["people"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["handle"].as_str().unwrap().to_string())
            .collect()
    };
    let (_, first) = call(&app, req(Method::GET, "/v1/people")).await;
    let (_, second) = call(&app, req(Method::GET, "/v1/people")).await;
    assert_eq!(order(&first).len(), 200, "one page of 200");
    assert_eq!(order(&first), order(&second), "the same order all day");
    let today = OffsetDateTime::now_utc().date().to_string();
    let expected: Vec<String> =
        sqlx::query_scalar("select handle from creatives order by md5(handle || $1) limit 200")
            .bind(&today)
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(
        order(&first),
        expected,
        "the order is the day's shuffle, not anything about performance"
    );
    let mut alphabetical = expected.clone();
    alphabetical.sort();
    assert_ne!(expected, alphabetical);
    assert_eq!(
        skill_count(&first, "pixel-art"),
        205,
        "counts aren't cut at the page"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn skills_are_listed_in_the_directorys_order(db: PgPool) {
    let app = test_app(db).await;
    let (status, skills) = call(&app, req(Method::GET, "/v1/people/skills")).await;
    assert_eq!(status, StatusCode::OK);
    let skills = skills.as_array().unwrap();
    assert_eq!(skills.len(), slop_api::people::SKILLS.len());
    for (got, (slug, name)) in skills.iter().zip(slop_api::people::SKILLS) {
        assert_eq!(got["slug"], *slug);
        assert_eq!(got["name"], *name);
    }
    assert_eq!(skills[0], json!({ "slug": "2d-art", "name": "2D Art" }));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn creative_pages_hide_for_rule_breaking_and_freeze_when_the_account_closes(db: PgPool) {
    let app = people_app(db).await;
    let (vera, vera_cookie) = creative_with(&app, "vera", "US", profile("vera", true)).await;
    let (_, visitor) = app.user("Visitor", "US").await;
    let page = |cookie: Option<&str>, handle: &str| {
        let mut r = req(Method::GET, format!("/v1/people/{handle}"));
        if let Some(c) = cookie {
            r = r.cookie(c);
        }
        call(&app, r)
    };

    // Handles are found whatever their case.
    let (status, body) = page(Some(&visitor), "VeRa").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (
            body["handle"].as_str(),
            body["status"].as_str(),
            body["is_me"].as_bool()
        ),
        (Some("vera"), Some("live"), Some(false))
    );
    assert_eq!(body["hidden_reason"], Value::Null);

    // Hidden for breaking the rules: gone for others; its owner sees why.
    exec(
        &app,
        "update creatives set hidden_reason = 'Copied someone else''s portfolio'",
    )
    .await;
    assert_eq!(page(Some(&visitor), "vera").await.0, StatusCode::NOT_FOUND);
    assert_eq!(page(None, "vera").await.0, StatusCode::NOT_FOUND);
    let (status, own) = page(Some(&vera_cookie), "vera").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(own["status"], "hidden");
    assert_eq!(own["hidden_reason"], "Copied someone else's portfolio");
    let (_, mine) = call(&app, fetch("/v1/me/creative", &vera_cookie)).await;
    assert_eq!(
        (mine["live"].as_bool(), mine["hidden_reason"].as_str()),
        (Some(false), Some("Copied someone else's portfolio"))
    );
    exec(&app, "update creatives set hidden_reason = null").await;

    // Suspended: the same, without a reason on the page (the decision says why).
    sqlx::query("update users set suspended_at = now() where id = $1")
        .bind(vera)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(page(Some(&visitor), "vera").await.0, StatusCode::NOT_FOUND);
    let (status, own) = page(Some(&vera_cookie), "vera").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(own["status"], "hidden");
    assert_eq!(own["hidden_reason"], Value::Null);
    sqlx::query("update users set suspended_at = null where id = $1")
        .bind(vera)
        .execute(&app.db)
        .await
        .unwrap();

    // Closed: the page stays, frozen, and takes no new work.
    let (client, client_cookie) = app.user("Client", "US").await;
    listed_with(&app, client, "tavern", "released", "").await;
    slop_api::account::close_account(&app.db, vera, "test")
        .await
        .unwrap();
    let (status, page_now) = page(Some(&visitor), "vera").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (
            page_now["closed"].as_bool(),
            page_now["can_hire"].as_bool(),
            page_now["taking_work"].as_bool()
        ),
        (Some(true), Some(false), Some(false))
    );
    let (status, _) = call(
        &app,
        post("/v1/people/vera/hire", &client_cookie, hire_body(json!({}))),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        page(Some(&visitor), "nobody").await.0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn ratings_show_from_three_clients_and_adult_projects_stay_out_of_the_under_18_view(
    db: PgPool,
) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    // Three clients' completed jobs on three projects; the first is 18+.
    let id = h.job("upfront", "completed").await;
    exec(app, "update apps set min_age = 18 where slug = 'tavern'").await;
    let mut others = Vec::new();
    for (name, slug) in [("Bo", "bos-game"), ("Cy", "cys-game")] {
        let (cid, cookie) = app.user(name, "US").await;
        listed_with(app, cid, slug, "released", "").await;
        let job = drive(
            app,
            "vera",
            &h.vera,
            &cookie,
            json!({ "app_slug": slug }),
            sprite_offer("on_delivery"),
            "completed",
        )
        .await;
        others.push((cookie, job));
    }
    let review = |id: &str, cookie: &str, vote: &str, body: &str| {
        req(Method::PUT, format!("/v1/jobs/{id}/review"))
            .cookie(cookie)
            .json(json!({ "vote": vote, "body": body }))
    };
    let (_, us) = app.user("Grown-up", "US").await;
    let (_, teen) = app.user("Teen", "GB").await;
    let rating = |cookie: &str| {
        let r = fetch("/v1/people/vera", cookie);
        async move { call(app, r).await.1 }
    };

    // Two reviews: counted, but no percentage yet.
    assert_eq!(
        call(app, review(&id, &h.tavern, "not_slop", "Lovely knight."))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(app, review(&others[0].1, &others[0].0, "slop", "Late."))
            .await
            .0,
        StatusCode::OK
    );
    let page = rating(&us).await;
    assert_eq!(
        (
            page["rating"]["not_slop"].as_i64(),
            page["rating"]["slop"].as_i64()
        ),
        (Some(1), Some(1))
    );
    assert_eq!(page["rating"]["percent"], Value::Null);
    assert_eq!(page["rating"]["min_reviews"], 3);
    // The third client: now it shows.
    call(
        app,
        review(&others[1].1, &others[1].0, "not_slop", "Great."),
    )
    .await;
    let page = rating(&us).await;
    assert_eq!(page["rating"]["percent"], 67);
    assert_eq!(page["clients"], 3);
    // Saying it again replaces the review: still one per client.
    let (status, job) = call(
        app,
        review(&others[0].1, &others[0].0, "not_slop", "Worth the wait."),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(job["review"]["vote"], "not_slop");
    let page = rating(&us).await;
    assert_eq!(
        (
            page["rating"]["not_slop"].as_i64(),
            page["rating"]["percent"].as_i64()
        ),
        (Some(3), Some(100))
    );
    assert_eq!(page["reviews"].as_array().unwrap().len(), 3);
    assert_eq!(
        slop_api::people::rating_of(&app.db, h.creative)
            .await
            .unwrap()
            .percent,
        Some(100)
    );

    // An 18+ project is named and listed only in the full view.
    let tavern_review = |page: &Value| {
        page["reviews"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["job_id"] == id.as_str())
            .unwrap()
            .clone()
    };
    let worked_on = |page: &Value| -> Vec<String> {
        let mut v: Vec<String> = page["worked_on"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["slug"].as_str().unwrap().to_string())
            .collect();
        v.sort();
        v
    };
    let full = rating(&us).await;
    assert_eq!(tavern_review(&full)["project_slug"], "tavern");
    assert_eq!(worked_on(&full), ["bos-game", "cys-game", "tavern"]);
    assert_eq!(full["worked_on"][0]["role"], "Art");
    let under_18 = rating(&teen).await;
    assert_eq!(tavern_review(&under_18)["project"], Value::Null);
    assert_eq!(tavern_review(&under_18)["body"], "Lovely knight.");
    assert_eq!(worked_on(&under_18), ["bos-game", "cys-game"]);
    // Signed out with no country: the under-18 view too.
    let (_, anonymous) = call(app, req(Method::GET, "/v1/people/vera")).await;
    assert_eq!(worked_on(&anonymous), ["bos-game", "cys-game"]);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn creatives_read_their_own_profile_and_agents_need_people_access(db: PgPool) {
    let app = people_app(db).await;
    let (_, nobody) = app.user("Not a creative", "US").await;
    let (status, _) = call(&app, fetch("/v1/me/creative", &nobody)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        call(&app, req(Method::GET, "/v1/me/creative")).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (_, vera) = live_creative(&app, "vera", "US", true).await;
    let (status, mine) = call(&app, fetch("/v1/me/creative", &vera)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (
            mine["handle"].as_str(),
            mine["live"].as_bool(),
            mine["stripe_status"].as_str(),
            mine["payments"].as_str()
        ),
        (Some("vera"), Some(true), Some("active"), Some("simulated"))
    );
    assert_eq!(mine["services"][0]["name"], "Sprite sheet");

    // A maker agent reads it only with People access.
    let token = |people: bool| {
        let r = post(
            "/v1/me/maker-tokens",
            &vera,
            json!({ "name": "Agent", "scope": "read", "people": people }),
        );
        let app = &app;
        async move {
            let (status, body) = call(app, r).await;
            assert_eq!(status, StatusCode::CREATED, "{body}");
            format!("Bearer {}", body["secret"].as_str().unwrap())
        }
    };
    let with = token(true).await;
    let without = token(false).await;
    let (status, body) = call(
        &app,
        req(Method::GET, "/v1/me/creative").header("authorization", with),
    )
    .await;
    assert_eq!(
        (status, body["handle"].as_str()),
        (StatusCode::OK, Some("vera"))
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/me/creative").header("authorization", without))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn saving_a_profile_checks_every_field(db: PgPool) {
    let app = people_app(db).await;
    let (_, cookie) = app.user("Vera", "US").await;
    let save = |p: Value| {
        let mut p = p;
        p["confirm_adult"] = json!(true);
        call(
            &app,
            req(Method::PUT, "/v1/me/creative").cookie(&cookie).json(p),
        )
    };
    let (status, saved) = save(profile("vera", true)).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let with = |key: &str, value: Value| {
        let mut p = profile("vera", true);
        p[key] = value;
        p
    };
    let service = |price: i64, currency: &str, skill: &str| json!({ "name": "Sprite", "price_minor": price, "currency": currency, "days": 7, "skill": skill });
    let link = json!({ "label": "", "url": "https://example.com/me" });
    let refused: Vec<(&str, Value)> = vec![
        ("handle", with("handle", json!("Vera Okafor"))),
        ("handle", with("handle", json!("ve"))),
        ("handle", with("handle", json!("vera--o"))),
        ("name", with("name", json!("   "))),
        ("name", with("name", json!("x".repeat(61)))),
        ("kind", with("kind", json!("collective"))),
        ("skills", with("skills", json!(["juggling"]))),
        ("skills", with("skills", json!([]))),
        ("ai_use", with("ai_use", json!("some"))),
        ("accent", with("accent", json!("neon-pink"))),
        ("sections", with("sections", json!(["portfolio", "blog"]))),
        ("about_md", with("about_md", json!("x".repeat(6001)))),
        ("headline", with("headline", json!("x".repeat(141)))),
        ("links", with("links", json!(vec![link.clone(); 6]))),
        (
            "links",
            with(
                "links",
                json!([{ "label": "", "url": "http://example.com" }]),
            ),
        ),
        (
            "services",
            with("services", json!(vec![service(2000, "usd", "music"); 9])),
        ),
        (
            "services",
            with("services", json!([service(999, "usd", "music")])),
        ),
        (
            "services",
            with("services", json!([service(1_000_001, "usd", "music")])),
        ),
        (
            "services",
            with("services", json!([service(2000, "jpy", "music")])),
        ),
        (
            "services",
            with("services", json!([service(2000, "usd", "voice")])),
        ),
        (
            "services",
            with(
                "services",
                json!([{ "name": "", "price_minor": 2000, "currency": "usd", "days": 7, "skill": "music" }]),
            ),
        ),
        (
            "services",
            with(
                "services",
                json!([{ "name": "X", "price_minor": 2000, "currency": "usd", "days": 0, "skill": "music" }]),
            ),
        ),
    ];
    for (want, p) in refused {
        let (status, body) = save(p.clone()).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{p}: {body}");
        assert_eq!(field(&body), want, "{body}");
    }
    // A studio says how many people, 2 to 500.
    for size in [Value::Null, json!(1), json!(501)] {
        let mut p = with("kind", json!("studio"));
        p["team_size"] = size;
        let (status, body) = save(p).await;
        assert_eq!(
            (status, field(&body)),
            (StatusCode::UNPROCESSABLE_ENTITY, "team_size")
        );
    }
    // The limits themselves are fine; none of the refusals changed anything.
    let (_, before) = call(&app, fetch("/v1/me/creative", &cookie)).await;
    assert_eq!(before, saved);
    let mut p = with("kind", json!("studio"));
    p["team_size"] = json!(2);
    p["services"] = json!(vec![service(1000, "eur", "music"); 8]);
    p["links"] = json!(vec![link; 5]);
    p["about_md"] = json!("x".repeat(6000));
    p["skills"] = json!(["music", "music", "voice"]);
    let (status, body) = save(p).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["team_size"], 2);
    assert_eq!(
        body["skills"],
        json!(["music", "voice"]),
        "duplicates dropped"
    );
    assert_eq!(body["services"].as_array().unwrap().len(), 8);
    assert_eq!(body["links"][0]["label"], "example.com");
    // A known service ID is kept; a made-up one is replaced.
    let kept = body["services"][0]["id"].as_str().unwrap().to_string();
    let mut p = profile("vera", true);
    p["services"] = json!([
        { "id": kept, "name": "A", "price_minor": 1000, "currency": "gbp", "days": 3, "skill": "music" },
        { "id": "not-hex!", "name": "B", "price_minor": 1000, "currency": "gbp", "days": 3, "skill": "music" }
    ]);
    let (_, body) = save(p).await;
    assert_eq!(body["services"][0]["id"], kept.as_str());
    assert_ne!(body["services"][1]["id"], "not-hex!");
    assert_eq!(body["services"][1]["id"].as_str().unwrap().len(), 8);
    // Unknown fields are refused outright.
    let (status, _) = save(with("verified", json!(true))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn saving_a_profile_needs_the_people_terms_and_a_passkey_once_due(db: PgPool) {
    let app = people_app(db).await;
    let (id, cookie) = app.user("Vera", "US").await;
    let mut p = profile("vera", true);
    p["confirm_adult"] = json!(true);
    let save = || {
        call(
            &app,
            req(Method::PUT, "/v1/me/creative")
                .cookie(&cookie)
                .json(p.clone()),
        )
    };
    // The People terms, accepted in their current version.
    sqlx::query("delete from legal_acceptances where user_id = $1 and kind = 'people'")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
    let (status, body) = save().await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        (body["error"]["code"].as_str(), field(&body)),
        (Some("terms_required"), "people")
    );
    assert_eq!(
        count(&app, "select count(*) from creatives").await,
        0,
        "nothing saved"
    );
    sqlx::query(
        "insert into legal_acceptances (user_id, kind, version)
         select $1, kind, version from legal_documents where kind = 'people'",
    )
    .bind(id)
    .execute(&app.db)
    .await
    .unwrap();
    // The first save starts the passkey clock.
    let (status, _) = save().await;
    assert_eq!(status, StatusCode::OK);
    let due: Option<OffsetDateTime> =
        sqlx::query_scalar("select passkey_due_at from users where id = $1")
            .bind(id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(due.unwrap() > OffsetDateTime::now_utc());
    // Past it, without a passkey: refused.
    sqlx::query("update users set passkey_due_at = now() - interval '1 day' where id = $1")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
    let (status, body) = save().await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        (body["error"]["code"].as_str(), field(&body)),
        (Some("passkey_required"), "add")
    );
    // A grace period from staff lets it through.
    sqlx::query("update users set passkey_grace_until = now() + interval '1 day' where id = $1")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(save().await.0, StatusCode::OK);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn creatives_reply_once_to_each_review(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let id = h.job("upfront", "completed").await;
    let (_, stranger) = app.user("Stranger", "US").await;
    let reply = |cookie: &str, text: String| {
        req(Method::PUT, format!("/v1/people/reviews/{id}/reply"))
            .cookie(cookie)
            .json(json!({ "reply": text }))
    };
    // No review yet: nothing to reply to.
    assert_eq!(
        call(app, reply(&h.vera, "Thanks!".into())).await.0,
        StatusCode::NOT_FOUND
    );
    call(
        app,
        req(Method::PUT, format!("/v1/jobs/{id}/review"))
            .cookie(&h.tavern)
            .json(json!({ "vote": "slop", "body": "The knight has 3 legs." })),
    )
    .await;
    for who in [&h.tavern, &stranger] {
        assert_eq!(
            call(app, reply(who, "Not mine to answer".into())).await.0,
            StatusCode::NOT_FOUND
        );
    }
    let (status, body) = call(app, reply(&h.vera, "x".repeat(1001))).await;
    assert_eq!(
        (status, field(&body)),
        (StatusCode::UNPROCESSABLE_ENTITY, "reply")
    );
    assert_eq!(
        call(
            app,
            reply(&h.vera, "  It's a centaur knight, as agreed.  ".into())
        )
        .await
        .0,
        StatusCode::OK
    );
    let page = call(app, fetch("/v1/people/vera", &stranger)).await.1;
    assert_eq!(
        page["reviews"][0]["reply"],
        "It's a centaur knight, as agreed."
    );
    assert!(page["reviews"][0]["replied_at"].is_string());
    // The client sees it on the job too.
    assert_eq!(
        h.view(&id, &h.tavern).await["review"]["reply"],
        "It's a centaur knight, as agreed."
    );
    // Saying nothing takes it back.
    assert_eq!(call(app, reply(&h.vera, "".into())).await.0, StatusCode::OK);
    let page = call(app, fetch("/v1/people/vera", &stranger)).await.1;
    assert_eq!(page["reviews"][0]["reply"], Value::Null);
    assert_eq!(page["reviews"][0]["replied_at"], Value::Null);
    // A review hidden by a decision isn't shown, whatever the reply.
    exec(app, "update creative_reviews set hidden_at = now()").await;
    assert_eq!(
        call(app, reply(&h.vera, "Still here.".into())).await.0,
        StatusCode::OK
    );
    let page = call(app, fetch("/v1/people/vera", &stranger)).await.1;
    assert_eq!(page["reviews"], json!([]));
}

// ---------------------------------------------------------------------------------------------
// Portfolios and their review (people-006 … people-011)

type Files = std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>>;

/// GitHub's raw files for these tests, kept apart from the shared RAW_FILES so tests running at
/// the same time can't change each other's portfolio.
async fn raw_files() -> (String, Files) {
    let files: Files = Default::default();
    let served = files.clone();
    let app = Router::new().route(
        "/raw/{*path}",
        get(
            move |axum::extract::Path(path): axum::extract::Path<String>| {
                let files = served.clone();
                async move {
                    match files.lock().unwrap().get(&path) {
                        Some(bytes) => (StatusCode::OK, bytes.clone()).into_response(),
                        None => StatusCode::NOT_FOUND.into_response(),
                    }
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/raw/"), files)
}

/// A file in octo/game's `.slop/` at its newest commit (the fake GitHub's "b…b").
fn put_file(files: &Files, path: &str, bytes: Vec<u8>) {
    files
        .lock()
        .unwrap()
        .insert(format!("octo/game/{}/.slop/{path}", "b".repeat(40)), bytes);
}

fn manifest(files: &Files, works: Value) {
    put_file(
        files,
        "portfolio.json",
        json!({ "avatar": "avatar.png", "works": works })
            .to_string()
            .into_bytes(),
    );
}

/// A creative (not live yet) on a server whose raw files are `files`.
async fn portfolio_app(db: PgPool) -> (TestApp, Files, Uuid, String) {
    let (raw, files) = raw_files().await;
    let app = test_app_with(db, |c| {
        c.media.github_raw_base = raw.parse().unwrap();
        c.payments.simulated = true;
        roomy(c);
    })
    .await;
    let (id, cookie) = app.user("Vera", "US").await;
    let mut p = profile("vera", true);
    p["confirm_adult"] = json!(true);
    let res = app
        .send(req(Method::PUT, "/v1/me/creative").cookie(&cookie).json(p))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    put_file(&files, "avatar.png", png(256, 256));
    put_file(&files, "knight.png", png(64, 64));
    put_file(&files, "tiny.png", png(16, 16));
    (app, files, id, cookie)
}

async fn link_github(app: &TestApp, user: Uuid, account: &str) {
    sqlx::query(
        "insert into identities (provider, provider_subject, user_id) values ('github', $1, $2)",
    )
    .bind(account)
    .bind(user)
    .execute(&app.db)
    .await
    .unwrap();
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn portfolios_come_from_the_creatives_own_repository_and_are_checked(db: PgPool) {
    let (app, files, vera, cookie) = portfolio_app(db).await;
    let (_, nobody) = app.user("Not a creative", "US").await;
    let check = |cookie: &str, body: Value| {
        call(&app, post("/v1/me/creative/portfolio/check", cookie, body))
    };
    let octo = json!({ "repo_url": "https://github.com/octo/game" });

    // Only creatives have a portfolio.
    assert_eq!(
        call(&app, fetch("/v1/me/creative/portfolio", &nobody))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(check(&nobody, octo.clone()).await.0, StatusCode::NOT_FOUND);
    let (status, overview) = call(&app, fetch("/v1/me/creative/portfolio", &cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (
            overview["repo_url"].clone(),
            overview["live"].clone(),
            overview["pending"].clone(),
            overview["manifest_path"].as_str()
        ),
        (
            Value::Null,
            Value::Null,
            Value::Null,
            Some(".slop/portfolio.json")
        )
    );

    // A repository first, on GitHub, belonging to the creative's linked GitHub account.
    let (status, body) = check(&cookie, json!({})).await;
    assert_eq!(
        (status, field(&body)),
        (StatusCode::UNPROCESSABLE_ENTITY, "repo_url")
    );
    let (status, body) = check(
        &cookie,
        json!({ "repo_url": "https://gitlab.com/octo/game" }),
    )
    .await;
    assert_eq!(
        (status, field(&body)),
        (StatusCode::UNPROCESSABLE_ENTITY, "repo_url")
    );
    let (status, body) = check(&cookie, octo.clone()).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("link the GitHub account"),
        "{body}"
    );
    link_github(&app, vera, "43").await;
    let (status, body) = check(&cookie, octo.clone()).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "bob's account doesn't own octo/game"
    );
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("only the repository's owner can put it on Slop"),
        "{body}"
    );
    let (status, _) = call(&app, fetch("/v1/me/creative/portfolio", &cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        count(
            &app,
            "select count(*) from creatives where portfolio_repo is not null"
        )
        .await,
        0,
        "a refused repository isn't kept"
    );
    link_github(&app, vera, "42").await;

    // No portfolio.json yet.
    let (status, body) = check(&cookie, octo).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"], "none");
    assert_eq!(body["repo_url"], "https://github.com/octo/game");

    // Each kind of problem is explained, and nothing is queued.
    let problems = [
        (
            json!([{ "image": "tiny.png", "title": "Tiny" }]),
            "tiny.png is 16×16; it needs to be at least 32×32",
        ),
        (
            json!([{ "image": "knight.png", "title": "x".repeat(81) }]),
            "titles are one line of up to 80 characters",
        ),
        (
            json!([{ "image": "knight.png", "skill": "juggling" }]),
            "\"juggling\" isn't one of the skills",
        ),
        (
            json!([{ "image": "knight.png", "youtube": "https://youtu.be/dQw4w9WgXcQ" }]),
            "needs exactly one of",
        ),
        (json!([]), "needs at least one work"),
        (
            json!([{ "image": "knight.png" }, { "image": "knight.png" }]),
            "lists image:knight.png twice",
        ),
    ];
    for (works, want) in problems {
        manifest(&files, works);
        let (status, body) = check(&cookie, json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"], "problem", "{body}");
        assert!(body["problem"].as_str().unwrap().contains(want), "{body}");
    }
    put_file(&files, "small-avatar.png", png(64, 64));
    put_file(
        &files,
        "portfolio.json",
        json!({ "avatar": "small-avatar.png", "works": [{ "image": "knight.png" }] })
            .to_string()
            .into_bytes(),
    );
    let (_, body) = check(&cookie, json!({})).await;
    assert!(
        body["problem"]
            .as_str()
            .unwrap()
            .contains("at least 128×128"),
        "{body}"
    );
    assert_eq!(count(&app, "select count(*) from portfolio_sets").await, 0);

    // A good one waits for review; checking it again doesn't queue it twice.
    manifest(
        &files,
        json!([{ "image": "knight.png", "title": "Knight", "skill": "pixel-art" }]),
    );
    let (_, body) = check(&cookie, json!({})).await;
    assert_eq!(body["result"], "pending", "{body}");
    assert_eq!(body["problem"], Value::Null);
    let (_, again) = check(&cookie, json!({})).await;
    assert_eq!(again["result"], "unchanged");
    assert_eq!(again["pending"]["id"], body["pending"]["id"]);
    assert_eq!(count(&app, "select count(*) from portfolio_sets").await, 1);
    let (_, overview) = call(&app, fetch("/v1/me/creative/portfolio", &cookie)).await;
    assert_eq!(overview["pending"]["works"][0]["title"], "Knight");
    assert_eq!(overview["live"], Value::Null);
    assert!(overview["checked_at"].is_string());
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_who_review_approve_or_turn_down_portfolios(db: PgPool) {
    let (app, files, vera, cookie) = portfolio_app(db).await;
    link_github(&app, vera, "42").await;
    let (_, stranger) = app.user("Stranger", "US").await;
    let (_, support) = staff(&app, "Sue Support", "support").await;
    let (_, finance) = staff(&app, "Fin Finance", "finance").await;
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
    manifest(
        &files,
        json!([{ "image": "knight.png", "title": "Knight" }]),
    );
    let set = check().await.1["pending"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let approve = |cookie: Option<&str>, set: &str| {
        let mut r = req(Method::POST, format!("/v1/admin/portfolio/{set}/approve"));
        if let Some(c) = cookie {
            r = r.cookie(c);
        }
        app.send(r)
    };
    let reject = |cookie: &str, set: &str, reason: &str| {
        app.send(post(
            format!("/v1/admin/portfolio/{set}/reject"),
            cookie,
            json!({ "reason": reason }),
        ))
    };

    // To everyone but staff, it doesn't exist; staff need the Review permission.
    assert_eq!(approve(None, &set).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        approve(Some(&stranger), &set).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        approve(Some(&cookie), &set).await.status(),
        StatusCode::NOT_FOUND
    );
    for who in [&support, &finance] {
        assert_eq!(
            approve(Some(who), &set).await.status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            reject(who, &set, "No").await.status(),
            StatusCode::FORBIDDEN
        );
    }
    // Support can see the queue but not decide it.
    let (status, queue) = call(&app, fetch("/v1/admin/queue", &support)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(queue["portfolios"][0]["set"]["id"], set.as_str());
    assert_eq!(
        call(&app, fetch("/v1/people/vera", &stranger)).await.0,
        StatusCode::NOT_FOUND
    );

    // The first approval puts the profile live, written to the audit log.
    assert_eq!(
        approve(Some(&reviewer), &set).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, fetch("/v1/people/vera", &stranger)).await.1["status"],
        "live"
    );
    let audit: (String, String) =
        sqlx::query_as("select actor, subject from audit_log where action = 'portfolio.approve'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(audit.0.starts_with("Rae Reviewer"), "{audit:?}");
    assert_eq!(audit.1, "vera");
    // Decided once.
    let res = approve(Some(&reviewer), &set).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(
        reject(&reviewer, &set, "Changed my mind").await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        approve(Some(&reviewer), &Uuid::now_v7().to_string())
            .await
            .status(),
        StatusCode::CONFLICT
    );

    // A newer portfolio: the approved one stays live until it's decided; turned down, it stays.
    manifest(
        &files,
        json!([{ "image": "knight.png", "title": "Knight" }, { "youtube": "https://youtu.be/dQw4w9WgXcQ" }]),
    );
    let newer = check().await.1["pending"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let page = call(&app, fetch("/v1/people/vera", &stranger)).await.1;
    assert_eq!(page["works"].as_array().unwrap().len(), 1);
    assert_eq!(
        reject(&reviewer, &newer, "   ").await.status(),
        StatusCode::CONFLICT,
        "a reason is required"
    );
    assert_eq!(
        reject(&reviewer, &newer, "The video isn't your work.")
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let page = call(&app, fetch("/v1/people/vera", &stranger)).await.1;
    assert_eq!(
        (
            page["status"].as_str(),
            page["works"].as_array().unwrap().len()
        ),
        (Some("live"), 1)
    );
    let (_, overview) = call(&app, fetch("/v1/me/creative/portfolio", &cookie)).await;
    assert_eq!(
        overview["rejected"]["status_reason"],
        "The video isn't your work."
    );
    assert_eq!(overview["live"]["id"], set.as_str());
    assert_eq!(
        count(
            &app,
            "select count(*) from audit_log where action = 'portfolio.reject'"
        )
        .await,
        1
    );
    // Checking the turned-down portfolio again says why, rather than queueing it.
    let (_, again) = check().await;
    assert_eq!(again["result"], "problem");
    assert!(
        again["problem"]
            .as_str()
            .unwrap()
            .contains("The video isn't your work.")
    );

    // A profile whose first portfolio is turned down stays waiting.
    let (_, second) = live_creative(&app, "other", "US", false).await;
    exec(
        &app,
        "update creatives set portfolio_set_id = null where handle = 'other'",
    )
    .await;
    let other_set: Uuid = sqlx::query_scalar(
        "insert into portfolio_sets (id, creative_id, repo_url, commit_sha, works, fingerprint, status, source)
         select gen_random_uuid(), user_id, 'https://github.com/x/y', repeat('d', 40), '[]', 'g', 'pending', 'sim'
         from creatives where handle = 'other' returning id",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        reject(&reviewer, &other_set.to_string(), "Not your work.")
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, fetch("/v1/people/other", &stranger)).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, fetch("/v1/people/other", &second)).await.1["status"],
        "waiting"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
#[ignore = "nb1: a creative isn't told when their portfolio is turned down (or approved); only the portfolio page shows it"]
async fn creatives_hear_when_their_portfolio_is_turned_down(db: PgPool) {
    let (app, files, vera, cookie) = portfolio_app(db).await;
    link_github(&app, vera, "42").await;
    let (_, reviewer) = staff(&app, "Rae Reviewer", "reviewer").await;
    manifest(&files, json!([{ "image": "knight.png" }]));
    let (_, body) = call(
        &app,
        post(
            "/v1/me/creative/portfolio/check",
            &cookie,
            json!({ "repo_url": "https://github.com/octo/game" }),
        ),
    )
    .await;
    let set = body["pending"]["id"].as_str().unwrap().to_string();
    app.send(post(
        format!("/v1/admin/portfolio/{set}/reject"),
        &reviewer,
        json!({ "reason": "Not your work." }),
    ))
    .await;
    assert!(
        notices(&app, vera)
            .await
            .iter()
            .any(|t| t.contains("portfolio")),
        "the creative is told, with the reason"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn creatives_put_their_reviewed_works_in_order(db: PgPool) {
    let (app, files, vera, cookie) = portfolio_app(db).await;
    link_github(&app, vera, "42").await;
    let (_, nobody) = app.user("Not a creative", "US").await;
    let (_, admin) = make_admin(&app, "Admin").await;
    manifest(
        &files,
        json!([{ "image": "knight.png", "title": "Knight" }, { "youtube": "https://youtu.be/dQw4w9WgXcQ", "title": "Reel" }]),
    );
    let (_, body) = call(
        &app,
        post(
            "/v1/me/creative/portfolio/check",
            &cookie,
            json!({ "repo_url": "https://github.com/octo/game" }),
        ),
    )
    .await;
    let set = body["pending"]["id"].as_str().unwrap().to_string();
    app.send(req(Method::POST, format!("/v1/admin/portfolio/{set}/approve")).cookie(&admin))
        .await;
    let order = |cookie: &str, keys: Value| {
        call(
            &app,
            req(Method::PUT, "/v1/me/creative/portfolio/order")
                .cookie(cookie)
                .json(json!({ "keys": keys })),
        )
    };
    assert_eq!(
        order(&nobody, json!(["video:dQw4w9WgXcQ"])).await.0,
        StatusCode::NOT_FOUND
    );
    for bad in [
        json!(["knight.png"]),
        json!([format!("image:{}", "x".repeat(220))]),
        json!(vec!["image:a.png"; 61]),
    ] {
        let (status, body) = order(&cookie, bad).await;
        assert_eq!(
            (status, field(&body)),
            (StatusCode::UNPROCESSABLE_ENTITY, "keys")
        );
    }
    // Keys for works that aren't in the portfolio are kept but move nothing.
    let (status, body) = order(&cookie, json!(["image:gone.png", "video:dQw4w9WgXcQ"])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["order"],
        json!(["image:gone.png", "video:dQw4w9WgXcQ"])
    );
    assert_eq!(body["live"]["works"][0]["kind"], "video");
    let page = call(&app, req(Method::GET, "/v1/people/vera")).await.1;
    let titles: Vec<&str> = page["works"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Reel", "Knight"]);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_admin_queue_lists_people_items_oldest_first_until_decided(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let (_, admin) = make_admin(app, "Ada Admin").await;
    let (_, stranger) = app.user("Stranger", "US").await;
    // Two pending portfolios (the second creative's is older).
    let (_, _) = live_creative(app, "other", "US", false).await;
    exec(
        app,
        "insert into portfolio_sets (id, creative_id, repo_url, commit_sha, works, fingerprint, status, source, created_at)
         select gen_random_uuid(), user_id, 'https://github.com/x/y', repeat('d', 40), '[]', handle, 'pending', 'sim',
                case when handle = 'other' then now() - interval '1 day' else now() end
         from creatives",
    )
    .await;
    // Two problems, on two jobs; the second opened first.
    let first = h.job("upfront", "in_progress").await;
    let second = h.job("upfront", "in_progress").await;
    h.ok(
        &second,
        &h.vera,
        "problem",
        json!({ "statement": "No reply for a week." }),
    )
    .await;
    h.ok(
        &first,
        &h.tavern,
        "problem",
        json!({ "statement": "The knight is late." }),
    )
    .await;
    // A review dispute.
    let done = h.job("upfront", "completed").await;
    call(
        app,
        req(Method::PUT, format!("/v1/jobs/{done}/review"))
            .cookie(&h.tavern)
            .json(json!({ "vote": "slop", "body": "Do it free or this stays." })),
    )
    .await;
    app.send(post(
        format!("/v1/people/reviews/{done}/dispute"),
        &h.vera,
        json!({ "reason": "extortion", "statement": "A threat." }),
    ))
    .await;

    for who in [None, Some(&stranger), Some(&h.vera)] {
        let mut r = req(Method::GET, "/v1/admin/queue");
        if let Some(c) = who {
            r = r.cookie(c);
        }
        assert_eq!(app.send(r).await.status(), StatusCode::NOT_FOUND);
        let mut r = req(Method::GET, "/v1/admin/review?kind=problem");
        if let Some(c) = who {
            r = r.cookie(c);
        }
        assert_eq!(app.send(r).await.status(), StatusCode::NOT_FOUND);
    }
    let (_, queue) = call(app, fetch("/v1/admin/queue", &admin)).await;
    let handles: Vec<&str> = queue["portfolios"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["handle"].as_str().unwrap())
        .collect();
    assert_eq!(handles, ["other", "vera"]);
    let jobs: Vec<&str> = queue["problems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["job_id"].as_str().unwrap())
        .collect();
    assert_eq!(jobs, [second.as_str(), first.as_str()]);
    assert_eq!(queue["problems"][0]["opened_by_creative"], true);
    assert_eq!(queue["review_disputes"][0]["job_id"], done.as_str());
    assert_eq!(
        queue["review_disputes"][0]["reason_text"],
        "Was used as a threat (\"do more for free or I'll leave a bad review\")"
    );
    let (status, feed) = call(app, fetch("/v1/admin/review?kind=problem", &admin)).await;
    assert_eq!(status, StatusCode::OK, "{feed}");
    assert_eq!(feed["items"].as_array().unwrap().len(), 2, "{feed}");
    assert_eq!(
        feed["items"][0]["detail"]["job_id"],
        second.as_str(),
        "{feed}"
    );
    assert_eq!(feed["items"][0]["kind"], "problem");

    // Decided items leave the queue.
    let problem = queue["problems"][0]["id"].as_str().unwrap();
    app.send(post(
        format!("/v1/admin/problems/{problem}/close"),
        &admin,
        json!({ "note": "They stopped answering." }),
    ))
    .await;
    let other_set = queue["portfolios"][0]["set"]["id"].as_str().unwrap();
    app.send(
        req(
            Method::POST,
            format!("/v1/admin/portfolio/{other_set}/approve"),
        )
        .cookie(&admin),
    )
    .await;
    app.send(post(
        format!("/v1/admin/review-disputes/{done}/decide"),
        &admin,
        json!({ "outcome": "keep", "reason": "An honest opinion." }),
    ))
    .await;
    let (_, queue) = call(app, fetch("/v1/admin/queue", &admin)).await;
    assert_eq!(queue["portfolios"].as_array().unwrap().len(), 1);
    assert_eq!(queue["problems"][0]["job_id"], first.as_str());
    assert_eq!(queue["problems"].as_array().unwrap().len(), 1);
    assert_eq!(queue["review_disputes"], json!([]));
    let (_, feed) = call(app, fetch("/v1/admin/review?kind=review_dispute", &admin)).await;
    assert_eq!(feed["items"], json!([]));
}

// ---------------------------------------------------------------------------------------------
// Stripe: onboarding, paying, checking a payment, the webhook (people-012, -013, -022, -023,
// -053)

/// What the fake Stripe holds, for a test to read and change.
#[derive(Default)]
struct Stripe {
    /// Checkout sessions by ID, each with the connected account it was made on.
    sessions: std::collections::HashMap<String, Value>,
    accounts_created: usize,
    sessions_created: usize,
    /// What GET /v1/accounts/{id} answers.
    account: Value,
    /// Stripe refuses new Checkout sessions.
    fail_checkout: bool,
    amount_refunded: i64,
}

type FakeStripe = std::sync::Arc<std::sync::Mutex<Stripe>>;

/// A fake Stripe that, like Stripe, answers for a Checkout session only on its own account.
async fn stripe_stand_in() -> (String, FakeStripe) {
    let fake: FakeStripe = Default::default();
    fake.lock().unwrap().account = json!({ "charges_enabled": true, "details_submitted": true });
    let f = fake.clone();
    let create_account = move || {
        let f = f.clone();
        async move {
            f.lock().unwrap().accounts_created += 1;
            axum::Json(json!({ "id": "acct_vera" }))
        }
    };
    let f = fake.clone();
    let get_account = move |axum::extract::Path(id): axum::extract::Path<String>| {
        let f = f.clone();
        async move {
            let mut a = f.lock().unwrap().account.clone();
            a["id"] = json!(id);
            axum::Json(a)
        }
    };
    let f = fake.clone();
    let create_session = move |headers: axum::http::HeaderMap, body: String| {
        let f = f.clone();
        async move {
            // Long enough for a second press to arrive while the first is here.
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let form: std::collections::HashMap<String, String> =
                url::form_urlencoded::parse(body.as_bytes())
                    .into_owned()
                    .collect();
            let mut s = f.lock().unwrap();
            if s.fail_checkout {
                return (
                    StatusCode::BAD_REQUEST,
                    axum::Json(json!({ "error": { "message": "Your account cannot currently make charges." } })),
                )
                    .into_response();
            }
            s.sessions_created += 1;
            let id = format!("cs_{}", s.sessions_created);
            let session = json!({
                "id": id, "status": "open", "payment_status": "unpaid",
                "account": headers["stripe-account"].to_str().unwrap(),
                "url": format!("https://checkout.stripe.example/{id}"),
                "amount_total": form["line_items[0][price_data][unit_amount]"].parse::<i64>().unwrap(),
                "currency": form["line_items[0][price_data][currency]"],
                "fee": form["payment_intent_data[application_fee_amount]"].parse::<i64>().unwrap(),
                "metadata": { "job_payment_id": form["metadata[job_payment_id]"], "payer": form["metadata[payer]"] },
            });
            s.sessions.insert(id, session.clone());
            axum::Json(session).into_response()
        }
    };
    let f = fake.clone();
    let get_session =
        move |headers: axum::http::HeaderMap,
              axum::extract::Path(id): axum::extract::Path<String>| {
            let f = f.clone();
            async move {
                let s = f.lock().unwrap().sessions.get(&id).cloned();
                let account = headers
                    .get("stripe-account")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                match s {
                    Some(mut s) if s["account"] == account => {
                        s["payment_intent"] = json!({ "latest_charge": format!("ch_{id}"), "application_fee_amount": s["fee"] });
                        axum::Json(s).into_response()
                    }
                    _ => (
                        StatusCode::NOT_FOUND,
                        axum::Json(json!({ "error": { "message": "No such checkout.session" } })),
                    )
                        .into_response(),
                }
            }
        };
    let f = fake.clone();
    let get_charge = move |axum::extract::Path(id): axum::extract::Path<String>| {
        let f = f.clone();
        async move {
            let refunded = f.lock().unwrap().amount_refunded;
            axum::Json(json!({ "id": id, "amount_refunded": refunded }))
        }
    };
    let app = Router::new()
        .route("/v1/accounts", axum::routing::post(create_account))
        .route("/v1/accounts/{id}", axum::routing::get(get_account))
        .route(
            "/v1/account_links",
            axum::routing::post(|body: String| async move {
                let form: std::collections::HashMap<String, String> =
                    url::form_urlencoded::parse(body.as_bytes())
                        .into_owned()
                        .collect();
                axum::Json(json!({ "url": format!("https://connect.stripe.example/setup/{}", form["account"]) }))
            }),
        )
        .route("/v1/checkout/sessions", axum::routing::post(create_session))
        .route("/v1/checkout/sessions/{id}", axum::routing::get(get_session))
        .route("/v1/charges/{id}", axum::routing::get(get_charge));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/"), fake)
}

/// Marks a session paid, as if the client finished Stripe Checkout.
fn pay_on_stripe(fake: &FakeStripe, session: &str) {
    let mut f = fake.lock().unwrap();
    let s = f.sessions.get_mut(session).unwrap();
    s["status"] = json!("complete");
    s["payment_status"] = json!("paid");
}

/// The same database served with Stripe payments, and the fake Stripe behind it.
async fn with_stripe(db: PgPool) -> (TestApp, FakeStripe) {
    let (base, fake) = stripe_stand_in().await;
    let app = test_app_with(db, |c| {
        c.payments.stripe = Some(slop_api::config::StripeConfig {
            secret_key: "sk_test_slop".into(),
            webhook_secret: "whsec_test".into(),
            api_base: base.parse().unwrap(),
        });
        roomy(c);
    })
    .await;
    (app, fake)
}

/// A signed Stripe webhook; its status.
async fn webhook(app: &TestApp, event: &Value) -> StatusCode {
    let (body, sig) = stripe_signed(event, "whsec_test");
    webhook_raw(app, body, &sig).await
}

async fn webhook_raw(app: &TestApp, body: String, sig: &str) -> StatusCode {
    let r = Request::builder()
        .method(Method::POST)
        .uri("/v1/stripe/webhook")
        .header(header::HOST, "localhost:8080")
        .header("stripe-signature", sig)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap();
    app.router.clone().oneshot(r).await.unwrap().status()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn getting_paid_sets_up_one_stripe_account(db: PgPool) {
    // Payments off: nothing to set up.
    let off = test_app_with(db.clone(), roomy).await;
    let (_, nobody) = off.user("Not a creative", "US").await;
    let (vera, cookie) = off.user("Vera", "US").await;
    let mut p = profile("vera", true);
    p["confirm_adult"] = json!(true);
    off.send(req(Method::PUT, "/v1/me/creative").cookie(&cookie).json(p))
        .await;
    let (status, body) = call(&off, post("/v1/me/creative/stripe", &cookie, json!({}))).await;
    assert_eq!(
        (status, body["error"]["code"].as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, Some("unavailable"))
    );
    assert_eq!(
        call(&off, post("/v1/me/creative/stripe", &nobody, json!({})))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    // Off, refreshing just says what's stored.
    let (status, body) = call(
        &off,
        post("/v1/me/creative/stripe/refresh", &cookie, json!({})),
    )
    .await;
    assert_eq!(
        (status, body["status"].as_str()),
        (StatusCode::OK, Some("none"))
    );

    // With Stripe: one account, however often the creative presses "Get paid".
    let (app, fake) = with_stripe(db).await;
    assert_eq!(
        call(&app, post("/v1/me/creative/stripe", &nobody, json!({})))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            post("/v1/me/creative/stripe/refresh", &nobody, json!({}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    // Before there's an account, refreshing asks Stripe nothing.
    let (_, body) = call(
        &app,
        post("/v1/me/creative/stripe/refresh", &cookie, json!({})),
    )
    .await;
    assert_eq!(body["status"], "none");
    for _ in 0..2 {
        let (status, body) = call(&app, post("/v1/me/creative/stripe", &cookie, json!({}))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            (body["url"].as_str(), body["status"].as_str()),
            (
                Some("https://connect.stripe.example/setup/acct_vera"),
                Some("pending")
            )
        );
    }
    assert_eq!(fake.lock().unwrap().accounts_created, 1);
    let stored: (Option<String>, String) =
        sqlx::query_as("select stripe_account_id, stripe_status from creatives where user_id = $1")
            .bind(vera)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(stored, (Some("acct_vera".into()), "pending".into()));

    // Once their passkey is due, "Get paid" needs one.
    sqlx::query("update users set passkey_due_at = now() - interval '1 day' where id = $1")
        .bind(vera)
        .execute(&app.db)
        .await
        .unwrap();
    let (status, body) = call(&app, post("/v1/me/creative/stripe", &cookie, json!({}))).await;
    assert_eq!(
        (status, body["error"]["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("passkey_required"))
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn refreshing_reads_the_account_status_from_stripe(db: PgPool) {
    let (app, fake) = with_stripe(db).await;
    let (vera, cookie) = app.user("Vera", "US").await;
    let mut p = profile("vera", true);
    p["confirm_adult"] = json!(true);
    app.send(req(Method::PUT, "/v1/me/creative").cookie(&cookie).json(p))
        .await;
    app.send(post("/v1/me/creative/stripe", &cookie, json!({})))
        .await;
    let cases = [
        (
            json!({ "charges_enabled": false, "details_submitted": false }),
            "pending",
        ),
        (
            json!({ "charges_enabled": false, "details_submitted": true,
                    "requirements": { "disabled_reason": "requirements.pending_verification" } }),
            "pending",
        ),
        (
            json!({ "charges_enabled": false, "details_submitted": true,
                    "requirements": { "disabled_reason": "rejected.fraud" } }),
            "restricted",
        ),
        (
            json!({ "charges_enabled": false, "details_submitted": false,
                    "requirements": { "disabled_reason": "rejected.fraud" } }),
            "pending",
        ),
        (
            json!({ "charges_enabled": true, "details_submitted": true }),
            "active",
        ),
    ];
    for (account, want) in cases {
        fake.lock().unwrap().account = account.clone();
        let (status, body) = call(
            &app,
            post("/v1/me/creative/stripe/refresh", &cookie, json!({})),
        )
        .await;
        assert_eq!(
            (status, body["status"].as_str()),
            (StatusCode::OK, Some(want)),
            "{account}"
        );
        let stored: String =
            sqlx::query_scalar("select stripe_status from creatives where user_id = $1")
                .bind(vera)
                .fetch_one(&app.db)
                .await
                .unwrap();
        assert_eq!(stored, want);
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn paying_is_for_the_client_when_something_is_due(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let (_, stranger) = app.user("Stranger", "US").await;
    let id = h.job("split", "accepted").await;
    // Only the client pays, and only what's due.
    h.refused(&id, &h.vera, "pay", json!({}), StatusCode::CONFLICT)
        .await;
    h.refused(&id, &stranger, "pay", json!({}), StatusCode::NOT_FOUND)
        .await;
    let paid = h.ok(&id, &h.tavern, "pay", json!({})).await;
    assert_eq!(paid["url"], Value::Null, "simulated: no Checkout");
    assert_eq!(paid["job"]["status"], "in_progress");
    assert_eq!(paid["job"]["paid_minor"], 6001);
    assert_eq!(paid["job"]["payments"][0]["simulated"], true);
    let (status, body) = h.step(&id, &h.tavern, "pay", json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["message"], "nothing is due right now");
    let fee: i32 = sqlx::query_scalar("select slop_fee_minor from payments")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(fee, 300, "5% of 60.01, rounded");
    // An ended job takes no payments.
    let declined = h.job("upfront", "offered").await;
    h.ok(&declined, &h.tavern, "decline", json!({})).await;
    h.refused(&declined, &h.tavern, "pay", json!({}), StatusCode::CONFLICT)
        .await;

    // With payments off, nothing can be paid.
    let waiting = h.job("upfront", "accepted").await;
    let off = test_app_with(app.db.clone(), roomy).await;
    let (status, body) = call(
        &off,
        post(format!("/v1/jobs/{waiting}/pay"), &h.tavern, json!({})),
    )
    .await;
    assert_eq!(
        (status, body["error"]["code"].as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, Some("unavailable"))
    );
    assert_eq!(h.status(&waiting).await, "awaiting_payment");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn paying_through_stripe_makes_one_checkout_and_says_when_it_cannot(db: PgPool) {
    let h = hiring(db.clone(), "US").await;
    let (app, fake) = with_stripe(db).await;
    exec(&app, "update creatives set stripe_account_id = 'acct_vera'").await;
    let id = h.job("upfront", "accepted").await;
    let pay = || {
        call(
            &app,
            post(format!("/v1/jobs/{id}/pay"), &h.tavern, json!({})),
        )
    };

    // Two presses at once: one Checkout session, the same link for both.
    let ((s1, a), (s2, b)) = tokio::join!(pay(), pay());
    assert_eq!((s1, s2), (StatusCode::OK, StatusCode::OK), "{a} {b}");
    assert_eq!(a["url"], "https://checkout.stripe.example/cs_1");
    assert_eq!(b["url"], a["url"]);
    assert_eq!(fake.lock().unwrap().sessions_created, 1);
    {
        let f = fake.lock().unwrap();
        let s = &f.sessions["cs_1"];
        assert_eq!(
            (s["amount_total"].as_i64(), s["fee"].as_i64()),
            (Some(12001), Some(600))
        );
        assert_eq!(s["account"], "acct_vera");
    }

    // A session that has expired is replaced; when Stripe refuses, the client hears why.
    fake.lock().unwrap().sessions.get_mut("cs_1").unwrap()["status"] = json!("expired");
    fake.lock().unwrap().fail_checkout = true;
    let (status, body) = pay().await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("cannot currently make charges"),
        "{body}"
    );
    fake.lock().unwrap().fail_checkout = false;
    let (_, body) = pay().await;
    assert_eq!(body["url"], "https://checkout.stripe.example/cs_2");

    // A session already paid isn't offered again: it's recorded.
    pay_on_stripe(&fake, "cs_2");
    let (status, body) = pay().await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("already been made")
    );
    assert_eq!(h.status(&id).await, "in_progress");

    // A creative without a Stripe account can't be paid.
    let other = h.job("upfront", "accepted").await;
    exec(&app, "update creatives set stripe_account_id = null").await;
    let (status, body) = call(
        &app,
        post(format!("/v1/jobs/{other}/pay"), &h.tavern, json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        body["error"]["message"],
        "they can't take payments right now"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn checking_a_payment_reads_the_session_slop_made(db: PgPool) {
    let h = hiring(db.clone(), "US").await;
    let (app, fake) = with_stripe(db).await;
    exec(&app, "update creatives set stripe_account_id = 'acct_vera'").await;
    let (_, stranger) = app.user("Stranger", "US").await;
    let id = h.job("upfront", "accepted").await;
    let check = |cookie: &str| {
        call(
            &app,
            post(format!("/v1/jobs/{id}/payment/check"), cookie, json!({})),
        )
    };
    assert_eq!(check(&stranger).await.0, StatusCode::NOT_FOUND);
    // Nothing started yet: nothing changes.
    let (status, job) = check(&h.tavern).await;
    assert_eq!(
        (status, job["status"].as_str()),
        (StatusCode::OK, Some("awaiting_payment"))
    );
    let (_, paid) = call(
        &app,
        post(format!("/v1/jobs/{id}/pay"), &h.tavern, json!({})),
    )
    .await;
    assert_eq!(paid["url"], "https://checkout.stripe.example/cs_1");
    let (_, job) = check(&h.tavern).await;
    assert_eq!(job["status"], "awaiting_payment", "not paid yet");

    // A session changed on the creative's side (a cheaper amount) isn't taken as the payment.
    pay_on_stripe(&fake, "cs_1");
    fake.lock().unwrap().sessions.get_mut("cs_1").unwrap()["amount_total"] = json!(100);
    let (_, job) = check(&h.vera).await;
    assert_eq!(job["status"], "awaiting_payment");
    assert_eq!(count(&app, "select count(*) from payments").await, 0);

    // Paid as Slop asked, without a webhook: checking moves the job on, once.
    fake.lock().unwrap().sessions.get_mut("cs_1").unwrap()["amount_total"] = json!(12001);
    let (status, job) = check(&h.tavern).await;
    assert_eq!(
        (status, job["status"].as_str()),
        (StatusCode::OK, Some("in_progress"))
    );
    assert_eq!(job["paid_minor"], 12001);
    check(&h.vera).await;
    assert_eq!(count(&app, "select count(*) from payments").await, 1);
    let row: (String, i32, bool) =
        sqlx::query_as("select stripe_object_id, slop_fee_minor, simulated from payments")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(row, ("ch_cs_1".into(), 600, false));

    // In the simulated mode there's nothing to check.
    let other = h.job("upfront", "accepted").await;
    let (status, job) = h.step(&other, &h.tavern, "payment/check", json!({})).await;
    assert_eq!(
        (status, job["status"].as_str()),
        (StatusCode::OK, Some("awaiting_payment"))
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_stripe_webhook_handles_people_events_once_and_only_from_the_right_account(db: PgPool) {
    let h = hiring(db.clone(), "US").await;
    let (app, fake) = with_stripe(db).await;
    exec(&app, "update creatives set stripe_account_id = 'acct_vera'").await;
    // Without Stripe there's no webhook.
    assert_eq!(
        webhook(&h.app, &json!({ "id": "evt_0", "type": "account.updated", "data": { "object": { "id": "acct_vera" } } })).await,
        StatusCode::NOT_FOUND
    );
    // Signed but not an event: refused.
    let (body, sig) = stripe_signed(&json!({ "hello": "world" }), "whsec_test");
    assert_eq!(webhook_raw(&app, body, &sig).await, StatusCode::BAD_REQUEST);
    let (body, sig) = stripe_signed(&json!("x"), "whsec_test");
    assert_eq!(
        webhook_raw(&app, body.replace('"', ""), &sig).await,
        StatusCode::BAD_REQUEST
    );

    let id = h.job("upfront", "accepted").await;
    call(
        &app,
        post(format!("/v1/jobs/{id}/pay"), &h.tavern, json!({})),
    )
    .await;
    let payment: String =
        sqlx::query_scalar("select id::text from job_payments where job_id = $1::uuid")
            .bind(&id)
            .fetch_one(&app.db)
            .await
            .unwrap();

    // Someone's own session on another account, naming this payment: ignored.
    fake.lock().unwrap().sessions.insert(
        "cs_evil".into(),
        json!({ "id": "cs_evil", "account": "acct_evil", "status": "complete", "payment_status": "paid",
                "amount_total": 12001, "currency": "usd", "fee": 0,
                "metadata": { "job_payment_id": payment } }),
    );
    let evil = json!({ "id": "evt_evil", "type": "checkout.session.completed", "account": "acct_evil",
                       "data": { "object": { "id": "cs_evil" } } });
    assert_eq!(webhook(&app, &evil).await, StatusCode::OK);
    assert_eq!(h.status(&id).await, "awaiting_payment");
    assert_eq!(count(&app, "select count(*) from payments").await, 0);

    // A session Stripe can't find: an error, so Stripe retries; the event isn't marked handled.
    let missing = json!({ "id": "evt_missing", "type": "checkout.session.completed", "account": "acct_vera",
                          "data": { "object": { "id": "cs_missing" } } });
    assert_eq!(
        webhook(&app, &missing).await,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from stripe_events where id = 'evt_missing'"
        )
        .await,
        0
    );

    // A delayed payment method succeeding pays the job like a completed Checkout.
    pay_on_stripe(&fake, "cs_1");
    let delayed = json!({ "id": "evt_async", "type": "checkout.session.async_payment_succeeded", "account": "acct_vera",
                          "data": { "object": { "id": "cs_1" } } });
    assert_eq!(webhook(&app, &delayed).await, StatusCode::OK);
    assert_eq!(h.status(&id).await, "in_progress");
    assert_eq!(
        count(
            &app,
            "select count(*) from stripe_events where id = 'evt_async'"
        )
        .await,
        1
    );

    // A card dispute is recorded against the charge, once; both sides hear of it.
    let dispute = json!({ "id": "evt_dp", "type": "charge.dispute.created", "account": "acct_vera",
                          "data": { "object": { "id": "dp_1", "charge": "ch_cs_1", "amount": 12001 } } });
    assert_eq!(webhook(&app, &dispute).await, StatusCode::OK);
    let mut again = dispute.clone();
    again["id"] = json!("evt_dp_again");
    assert_eq!(webhook(&app, &again).await, StatusCode::OK);
    let rows: Vec<(String, i32)> = sqlx::query_as(
        "select stripe_object_id, amount_minor from payments where kind = 'dispute'",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(rows, [("dp_1".to_string(), -12001)]);
    let job = h.view(&id, &h.vera).await;
    let disputes = job["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "disputed")
        .count();
    assert_eq!(disputes, 1);
    assert_eq!(
        job["events"].as_array().unwrap().last().unwrap()["note"],
        "$120.01"
    );
    assert!(
        notices(&app, h.creative).await[0].contains("disputed"),
        "the creative, whose money it is, hears of it"
    );
    assert!(
        !notices(&app, h.client)
            .await
            .iter()
            .any(|t| t.contains("disputed your review")),
        "a card dispute isn't a review dispute"
    );
    // A dispute about a charge Slop doesn't know: nothing recorded.
    let unknown = json!({ "id": "evt_dp2", "type": "charge.dispute.created", "account": "acct_vera",
                          "data": { "object": { "id": "dp_2", "charge": "ch_unknown", "amount": 500 } } });
    assert_eq!(webhook(&app, &unknown).await, StatusCode::OK);
    assert_eq!(
        count(&app, "select count(*) from payments where kind = 'dispute'").await,
        1
    );

    // account.updated re-reads the account from Stripe.
    fake.lock().unwrap().account = json!({ "charges_enabled": false, "details_submitted": true,
                                           "requirements": { "disabled_reason": "rejected.other" } });
    let updated = json!({ "id": "evt_acct", "type": "account.updated", "account": "acct_vera",
                          "data": { "object": { "id": "acct_vera" } } });
    assert_eq!(webhook(&app, &updated).await, StatusCode::OK);
    let status: String = sqlx::query_scalar("select stripe_status from creatives")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "restricted");
    let (_, dir) = call(&app, req(Method::GET, "/v1/people")).await;
    assert_eq!(dir["people"][0]["can_hire"], false);
}

// ---------------------------------------------------------------------------------------------
// Requests, offers, accepting (people-015 … people-021)

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn hiring_checks_the_client_the_creative_and_the_request(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    assert_eq!(
        app.send(req(Method::POST, "/v1/people/vera/hire").json(hire_body(json!({}))))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    // Adults only, confirmed once.
    let (status, body) = h.hire(json!({ "confirm_adult": false })).await;
    assert_eq!(
        (status, field(&body)),
        (StatusCode::UNPROCESSABLE_ENTITY, "confirm_adult")
    );
    // The People terms, accepted.
    sqlx::query("delete from legal_acceptances where user_id = $1 and kind = 'people'")
        .bind(h.client)
        .execute(&app.db)
        .await
        .unwrap();
    let (status, body) = h.hire(json!({})).await;
    assert_eq!((status, field(&body)), (StatusCode::CONFLICT, "people"));
    sqlx::query(
        "insert into legal_acceptances (user_id, kind, version) select $1, kind, version from legal_documents where kind = 'people'",
    )
    .bind(h.client)
    .execute(&app.db)
    .await
    .unwrap();

    // What's asked for: one of their skills, a service they offer, a date, a request.
    let refused = [
        (json!({ "skill": "voice" }), "skill"),
        (json!({ "service_id": "deadbeef" }), "service_id"),
        (json!({ "wanted_by": "next tuesday" }), "wanted_by"),
        (json!({ "wanted_by": "2026-02-30" }), "wanted_by"),
        (json!({ "request_md": "   " }), "request_md"),
        (json!({ "request_md": "x".repeat(4001) }), "request_md"),
        (json!({ "app_slug": "nope" }), "app_slug"),
    ];
    for (extra, want) in refused {
        let (status, body) = h.hire(extra.clone()).await;
        assert_eq!(
            (status, field(&body)),
            (StatusCode::UNPROCESSABLE_ENTITY, want),
            "{extra}: {body}"
        );
    }
    assert_eq!(count(app, "select count(*) from jobs").await, 0);
    let service = call(app, fetch("/v1/me/creative", &h.vera)).await.1["services"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, job) = h
        .hire(json!({ "service_id": service, "wanted_by": "2027-01-31", "request_md": "  A knight.  " }))
        .await;
    assert_eq!(status, StatusCode::OK, "{job}");
    assert_eq!(
        (
            job["status"].as_str(),
            job["wanted_by"].as_str(),
            job["request_md"].as_str()
        ),
        (Some("requested"), Some("2027-01-31"), Some("A knight."))
    );
    assert_eq!(job["role"], "client");
    assert_eq!(job["inquiry"], false);
    assert_eq!(job["conversation"]["requested_at"], Value::Null);
    assert_eq!(
        job["events"][0]["note"],
        "Package: Sprite sheet (from $120, about 7 days)"
    );
    assert!(
        notices(app, h.creative).await[0].starts_with("Tavern Dev asked you for work"),
        "{:?}",
        notices(app, h.creative).await
    );

    // At most 3 open requests with one creative.
    h.hire(json!({})).await;
    h.hire(json!({})).await;
    let (status, body) = h.hire(json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("3 open requests")
    );

    // Not taking work, not able to take payments, hidden or not live: no hiring.
    let (other, other_cookie) = app.user("Other Client", "US").await;
    listed_with(app, other, "other-game", "released", "").await;
    let hire_as_other = || {
        call(
            app,
            post(
                "/v1/people/vera/hire",
                &other_cookie,
                hire_body(json!({ "app_slug": "other-game" })),
            ),
        )
    };
    exec(app, "update creatives set taking_work = false").await;
    assert_eq!(hire_as_other().await.0, StatusCode::CONFLICT);
    exec(
        app,
        "update creatives set taking_work = true, stripe_status = 'restricted'",
    )
    .await;
    let (status, body) = hire_as_other().await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        body["error"]["message"],
        "they aren't taking paid work right now"
    );
    exec(
        app,
        "update creatives set stripe_status = 'active', hidden_reason = 'Spam'",
    )
    .await;
    assert_eq!(hire_as_other().await.0, StatusCode::NOT_FOUND);
    exec(
        app,
        "update creatives set hidden_reason = null, portfolio_set_id = null",
    )
    .await;
    assert_eq!(hire_as_other().await.0, StatusCode::NOT_FOUND);
    exec(
        app,
        "update creatives set portfolio_set_id = (select id from portfolio_sets limit 1)",
    )
    .await;
    assert_eq!(hire_as_other().await.0, StatusCode::OK);
    let (status, _) = call(
        app,
        post(
            "/v1/people/nobody/hire",
            &other_cookie,
            hire_body(json!({ "app_slug": "other-game" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn hiring_requests_are_rate_limited_per_account(db: PgPool) {
    let h = hiring(db, "US").await;
    // Twenty a day, counted even when refused.
    for _ in 0..20 {
        let (status, _) = h.hire(json!({ "exact_wording_ack": false })).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }
    let res = h
        .app
        .send(post(
            "/v1/people/vera/hire",
            &h.tavern,
            hire_body(json!({})),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(res.headers().contains_key(header::RETRY_AFTER));
    assert_eq!(body_json(res).await["error"]["code"], "rate_limited");
    // Someone else isn't held back.
    let (other, other_cookie) = h.app.user("Other", "US").await;
    listed_with(&h.app, other, "other-game", "released", "").await;
    let (status, _) = call(
        &h.app,
        post(
            "/v1/people/vera/hire",
            &other_cookie,
            hire_body(json!({ "app_slug": "other-game" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn my_jobs_say_which_wait_on_me(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    // Vera hires someone too, so she's on both sides.
    let (_, _) = creative_with(app, "omar", "US", profile("x", false)).await;
    listed_with(app, h.creative, "veras-own", "released", "").await;
    let as_client_of_omar = {
        let (status, job) = call(
            app,
            post(
                "/v1/people/omar/hire",
                &h.vera,
                hire_body(json!({ "app_slug": "veras-own" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{job}");
        job["id"].as_str().unwrap().to_string()
    };
    let requested = h.job("upfront", "requested").await;
    let offered = h.job("upfront", "offered").await;
    let awaiting = h.job("upfront", "accepted").await;
    let in_progress = h.job("upfront", "in_progress").await;
    let delivered = h.job("upfront", "delivered").await;
    let completed = h.job("upfront", "completed").await;
    let reviewed = h.job("upfront", "completed").await;
    call(
        app,
        req(Method::PUT, format!("/v1/jobs/{reviewed}/review"))
            .cookie(&h.tavern)
            .json(json!({ "vote": "not_slop" })),
    )
    .await;
    // A change the client asked for waits on the creative; one the creative offered, on the client.
    let change_asked = h.job("upfront", "delivered").await;
    h.ok(
        &change_asked,
        &h.tavern,
        "change/request",
        json!({ "note": "Taller?" }),
    )
    .await;
    let change_offered = h.job("upfront", "in_progress").await;
    h.ok(
        &change_offered,
        &h.vera,
        "change/offer",
        json!({ "title": "Taller", "extra_price_minor": 0,
                "new_due_date": (OffsetDateTime::now_utc().date() + time::Duration::days(20)).to_string() }),
    )
    .await;
    // A conversation the client asked for waits on the creative.
    let talk = h.job("upfront", "accepted").await;
    h.ok(&talk, &h.tavern, "conversation/request", json!({}))
        .await;

    let card = |list: &Value, id: &str| {
        list.as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id)
            .unwrap_or_else(|| panic!("{id} not in {list}"))
            .clone()
    };
    let (_, client) = call(app, fetch("/v1/me/jobs", &h.tavern)).await;
    assert_eq!(client["as_creative"], json!([]));
    let needs = |list: &Value, id: &str| card(list, id)["needs_you"].as_bool().unwrap();
    let c = &client["as_client"];
    assert_eq!(c.as_array().unwrap().len(), 10);
    for (id, want) in [
        (&requested, false),
        (&offered, true),
        (&awaiting, true),
        (&in_progress, false),
        (&delivered, true),
        (&completed, true),
        (&reviewed, false),
        (&change_asked, true),
        (&change_offered, true),
        (&talk, true),
    ] {
        assert_eq!(needs(c, id), want, "client, {}", card(c, id));
    }
    assert_eq!(client["waiting_on_you"], 7);
    assert_eq!(
        card(c, &awaiting)["status_text"],
        "Pay $120.01 to start the work."
    );
    assert_eq!(card(c, &awaiting)["price"], "$120.01");
    assert_eq!(card(c, &requested)["title"], "A knight, 4 directions.");
    assert_eq!(card(c, &offered)["title"], "Knight sprite sheet");
    assert_eq!(card(c, &offered)["other_name"], "Vera Okafor");

    let (_, creative) = call(app, fetch("/v1/me/jobs", &h.vera)).await;
    let v = &creative["as_creative"];
    for (id, want) in [
        (&requested, true),
        (&offered, false),
        (&awaiting, false),
        (&in_progress, true),
        (&delivered, false),
        (&completed, false),
        (&change_asked, true),
        (&change_offered, true),
        (&talk, true),
    ] {
        assert_eq!(needs(v, id), want, "creative, {}", card(v, id));
    }
    assert_eq!(card(v, &talk)["conversation_requested"], true);
    assert_eq!(card(v, &in_progress)["other_name"], "Tavern Dev");
    assert_eq!(creative["as_client"][0]["id"], as_client_of_omar.as_str());
    assert_eq!(creative["as_client"][0]["needs_you"], false);
    assert_eq!(creative["waiting_on_you"], 5);

    // At most 200, newest first.
    exec(
        app,
        "insert into jobs (id, creative_id, client_id, app_id, skill, status, request_md, updated_at)
         select gen_random_uuid(), creative_id, client_id, app_id, skill, 'declined', 'Old request', now() - interval '1 year'
         from (select creative_id, client_id, app_id, skill from jobs
               where app_id = (select id from apps where slug = 'tavern') limit 1) j,
              generate_series(1, 200)",
    )
    .await;
    let (_, client) = call(app, fetch("/v1/me/jobs", &h.tavern)).await;
    assert_eq!(client["as_client"].as_array().unwrap().len(), 200);
    assert_ne!(client["as_client"][0]["title"], "Old request");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_job_shows_each_side_what_it_can_do(db: PgPool) {
    let h = hiring(db, "DE").await;
    let app = &h.app;
    let (_, stranger) = app.user("Stranger", "US").await;
    let id = h.job("split", "requested").await;
    assert_eq!(
        call(app, fetch(format!("/v1/jobs/{id}"), &stranger))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            app,
            fetch(format!("/v1/jobs/{}", Uuid::now_v7()), &h.tavern)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let hr = &h;
    let both =
        |id: String| async move { (hr.view(&id, &hr.tavern).await, hr.view(&id, &hr.vera).await) };
    let (client, creative) = both(id.clone()).await;
    assert_eq!(actions(&client), ["decline", "request_conversation"]);
    assert_eq!(
        actions(&creative),
        ["offer", "decline", "open_conversation"]
    );
    // The creative sees the client's payment record and what they'll receive; the client doesn't.
    assert_eq!(client["client_record"], Value::Null);
    assert_eq!(
        creative["client_record"],
        json!({ "completed_jobs": 0, "paid_in_full": 0, "owing": 0 })
    );

    h.ok(&id, &h.vera, "offer", sprite_offer("split")).await;
    let (client, creative) = both(id.clone()).await;
    assert_eq!(
        actions(&client),
        ["accept", "decline", "request_conversation"]
    );
    assert_eq!(
        client["withdrawal_days"], 14,
        "a German consumer hiring a trader"
    );
    assert_eq!(client["creative_receives_minor"], Value::Null);
    assert_eq!(creative["creative_receives_minor"], 12001 - 600);
    assert_eq!(client["agreement"]["sha256"], Value::Null, "not agreed yet");

    h.ok(
        &id,
        &h.tavern,
        "accept",
        json!({ "offer_count": 1, "exact_terms_ack": true }),
    )
    .await;
    let (client, creative) = both(id.clone()).await;
    assert_eq!(
        actions(&client),
        [
            "pay",
            "cancel",
            "request_conversation",
            "withdraw",
            "open_problem"
        ]
    );
    assert_eq!(
        actions(&creative),
        ["cancel", "open_conversation", "open_problem"]
    );
    assert_eq!(client["withdrawal"]["open"], true);
    assert_eq!(client["withdrawal"]["started_early"], false);
    assert_eq!(
        client["withdrawal_days"],
        Value::Null,
        "only shown before accepting"
    );
    assert_eq!(client["status_text"], "Pay $60.01 to start the work.");

    h.ok(&id, &h.tavern, "pay", json!({})).await;
    let (client, creative) = both(id.clone()).await;
    assert_eq!(
        actions(&client),
        [
            "request_change",
            "request_conversation",
            "withdraw",
            "open_problem"
        ]
    );
    assert_eq!(
        actions(&creative),
        [
            "deliver",
            "cancel",
            "propose_change",
            "open_conversation",
            "open_problem"
        ]
    );
    h.ok(&id, &h.vera, "conversation/open", json!({})).await;
    h.ok(
        &id,
        &h.vera,
        "deliver",
        json!({ "note": "In the shared folder." }),
    )
    .await;
    let (client, creative) = both(id.clone()).await;
    assert_eq!(
        actions(&client),
        [
            "pay",
            "request_change",
            "message",
            "withdraw",
            "open_problem"
        ]
    );
    assert_eq!(
        actions(&creative),
        [
            "cancel",
            "propose_change",
            "close_conversation",
            "message",
            "open_problem"
        ]
    );
    assert!(client["auto_complete_at"].is_string());
    h.ok(&id, &h.tavern, "pay", json!({})).await;
    let (client, creative) = both(id.clone()).await;
    assert_eq!(
        actions(&creative),
        [
            "final_files",
            "close_conversation",
            "message",
            "open_problem"
        ]
    );
    assert_eq!(
        client["status_text"],
        "Paid in full. Waiting for Vera Okafor to send the final files."
    );
    assert_eq!(creative["client_record"]["completed_jobs"], 1);

    // Hiring as a business, or from a creative who isn't a trader: no withdrawal right.
    let business = {
        let (_, job) = h.hire(json!({ "client_is_business": true })).await;
        job["id"].as_str().unwrap().to_string()
    };
    h.ok(&business, &h.vera, "offer", sprite_offer("upfront"))
        .await;
    assert_eq!(
        h.view(&business, &h.tavern).await["withdrawal_days"],
        Value::Null
    );
    exec(app, "update creatives set trader = false").await;
    let private = h.job("upfront", "accepted").await;
    let job = h.view(&private, &h.tavern).await;
    assert_eq!(job["withdrawal"], Value::Null);
    assert!(!actions(&job).contains(&"withdraw"));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_job_record_is_for_its_two_sides(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let (_, stranger) = app.user("Stranger", "US").await;
    let (_, admin) = make_admin(app, "Admin").await;
    let id = h.job("upfront", "in_progress").await;
    for who in [&stranger, &admin] {
        assert_eq!(
            call_text(app, fetch(format!("/v1/jobs/{id}/record"), who))
                .await
                .0,
            StatusCode::NOT_FOUND
        );
    }
    h.ok(&id, &h.vera, "conversation/open", json!({})).await;
    h.ok(
        &id,
        &h.tavern,
        "messages",
        json!({ "body": "Blue armour, please.", "recording_ack": true }),
    )
    .await;
    h.ok(&id, &h.vera, "conversation/close", json!({})).await;
    let (status, disposition, record) =
        call_text(app, fetch(format!("/v1/jobs/{id}/record"), &h.tavern)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        disposition.unwrap(),
        format!("attachment; filename=\"slop-job-{id}.md\"")
    );
    assert!(record.starts_with("# Job record: Knight sprite sheet"));
    assert!(record.contains("## Service Agreement (accepted"));
    assert!(record.contains("**Tavern Dev**"));
    assert!(record.contains("Blue armour, please."));
    assert!(record.contains("- full: $120.01 · paid"));
    // Thirty days after the conversation closed, only fingerprints are left in it.
    slop_api::people::jobs::forget_conversations(
        &app.db,
        OffsetDateTime::now_utc() + time::Duration::days(31),
    )
    .await
    .unwrap();
    let (_, _, record) = call_text(app, fetch(format!("/v1/jobs/{id}/record"), &h.vera)).await;
    assert!(!record.contains("Blue armour, please."));
    assert!(record.contains("deleted from Slop; fingerprint `"));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn offers_come_from_the_creative_until_one_is_accepted(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let (_, stranger) = app.user("Stranger", "US").await;
    let id = h.job("upfront", "requested").await;
    h.refused(
        &id,
        &h.tavern,
        "offer",
        sprite_offer("upfront"),
        StatusCode::CONFLICT,
    )
    .await;
    h.refused(
        &id,
        &stranger,
        "offer",
        sprite_offer("upfront"),
        StatusCode::NOT_FOUND,
    )
    .await;
    // The offer is checked; unknown fields are refused.
    let mut bad = sprite_offer("upfront");
    bad["price_minor"] = json!(999);
    let (status, body) = h.step(&id, &h.vera, "offer", bad).await;
    assert_eq!(
        (status, field(&body)),
        (StatusCode::UNPROCESSABLE_ENTITY, "price_minor")
    );
    let mut bad = sprite_offer("monthly");
    bad["payment"] = json!("monthly");
    let (status, body) = h.step(&id, &h.vera, "offer", bad).await;
    assert_eq!(
        (status, field(&body)),
        (StatusCode::UNPROCESSABLE_ENTITY, "payment")
    );
    let mut bad = sprite_offer("upfront");
    bad["due_date"] = json!("2020-01-01");
    let (status, body) = h.step(&id, &h.vera, "offer", bad).await;
    assert_eq!(
        (status, field(&body)),
        (StatusCode::UNPROCESSABLE_ENTITY, "due_date")
    );
    let mut bad = sprite_offer("upfront");
    bad["discount"] = json!(10);
    h.refused(&id, &h.vera, "offer", bad, StatusCode::UNPROCESSABLE_ENTITY)
        .await;
    assert_eq!(h.status(&id).await, "requested");

    let job = h.ok(&id, &h.vera, "offer", sprite_offer("upfront")).await;
    assert_eq!(
        (job["status"].as_str(), job["offer_count"].as_i64()),
        (Some("offered"), Some(1))
    );
    assert!(notices(app, h.client).await[0].starts_with("Vera Okafor sent you an offer"));
    // A new offer replaces it; accepting the one read before is refused.
    let mut cheaper = sprite_offer("upfront");
    cheaper["price_minor"] = json!(9000);
    let job = h.ok(&id, &h.vera, "offer", cheaper).await;
    assert_eq!(
        (
            job["offer_count"].as_i64(),
            job["offer"]["price_minor"].as_i64()
        ),
        (Some(2), Some(9000))
    );
    let (status, body) = h
        .step(
            &id,
            &h.tavern,
            "accept",
            json!({ "offer_count": 1, "exact_terms_ack": true }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("the offer changed")
    );
    h.ok(
        &id,
        &h.tavern,
        "accept",
        json!({ "offer_count": 2, "exact_terms_ack": true }),
    )
    .await;
    // Once accepted, offers are over: changes are Changes to Agreement.
    h.refused(
        &id,
        &h.vera,
        "offer",
        sprite_offer("split"),
        StatusCode::CONFLICT,
    )
    .await;
    let price: i64 = sqlx::query_scalar("select (offer->>'price_minor')::int8 from jobs")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(price, 9000);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn either_side_declines_before_anything_is_agreed(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let (_, stranger) = app.user("Stranger", "US").await;
    // The client turns an offer down, with a note; the conversation closes.
    let id = h.job("upfront", "offered").await;
    h.ok(&id, &h.vera, "conversation/open", json!({})).await;
    h.refused(&id, &stranger, "decline", json!({}), StatusCode::NOT_FOUND)
        .await;
    let (status, body) = h
        .step(
            &id,
            &h.tavern,
            "decline",
            json!({ "note": "x".repeat(1001) }),
        )
        .await;
    assert_eq!(
        (status, field(&body)),
        (StatusCode::UNPROCESSABLE_ENTITY, "note")
    );
    let job = h
        .ok(
            &id,
            &h.tavern,
            "decline",
            json!({ "note": "Too dear for us." }),
        )
        .await;
    assert_eq!(job["status"], "declined");
    assert_eq!(job["conversation"]["open"], false);
    assert!(job["conversation"]["text_deleted_after"].is_string());
    let last = job["events"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(
        (last["kind"].as_str(), last["note"].as_str()),
        (Some("declined"), Some("Too dear for us."))
    );
    let ended_by: Uuid = sqlx::query_scalar("select ended_by from jobs where id = $1::uuid")
        .bind(&id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(ended_by, h.client);
    assert!(notices(app, h.creative).await[0].starts_with("Tavern Dev declined your offer"));
    // Declined is the end.
    h.refused(&id, &h.vera, "decline", json!({}), StatusCode::CONFLICT)
        .await;
    h.refused(
        &id,
        &h.vera,
        "offer",
        sprite_offer("upfront"),
        StatusCode::CONFLICT,
    )
    .await;

    // The creative declines a request.
    let id = h.job("upfront", "requested").await;
    let job = h.ok(&id, &h.vera, "decline", json!({})).await;
    assert_eq!(job["status"], "declined");
    assert!(notices(app, h.client).await[0].starts_with("Vera Okafor declined"));
    // After accepting, it's cancelling, not declining.
    let id = h.job("upfront", "accepted").await;
    for who in [&h.tavern, &h.vera] {
        h.refused(&id, who, "decline", json!({}), StatusCode::CONFLICT)
            .await;
    }
    assert_eq!(h.status(&id).await, "awaiting_payment");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn accepting_signs_the_agreement_and_sets_up_the_payments(db: PgPool) {
    let h = hiring(db, "DE").await;
    let app = &h.app;
    let accept = json!({ "offer_count": 1, "exact_terms_ack": true });
    // Only the client, only an offer.
    let id = h.job("split", "requested").await;
    h.refused(
        &id,
        &h.tavern,
        "accept",
        accept.clone(),
        StatusCode::CONFLICT,
    )
    .await;
    h.ok(&id, &h.vera, "offer", sprite_offer("split")).await;
    h.refused(&id, &h.vera, "accept", accept.clone(), StatusCode::CONFLICT)
        .await;
    // An adult, confirmed (here, again after it was cleared).
    sqlx::query("update users set adult_confirmed_at = null where id = $1")
        .bind(h.client)
        .execute(&app.db)
        .await
        .unwrap();
    let (status, body) = h.step(&id, &h.tavern, "accept", accept.clone()).await;
    assert_eq!(
        (status, field(&body)),
        (StatusCode::UNPROCESSABLE_ENTITY, "confirm_adult")
    );
    let job = h
        .ok(
            &id,
            &h.tavern,
            "accept",
            json!({ "offer_count": 1, "exact_terms_ack": true, "confirm_adult": true }),
        )
        .await;
    assert_eq!(job["status"], "awaiting_payment");
    // The agreement is kept word for word, with its fingerprint.
    let (text, sha, template): (String, String, String) = sqlx::query_as(
        "select agreement_text, agreement_sha256, agreement_template from jobs where id = $1::uuid",
    )
    .bind(&id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        sha,
        hex::encode(<sha2::Sha256 as sha2::Digest>::digest(text.as_bytes()))
    );
    assert_eq!(template, slop_api::people::jobs::TEMPLATE_VERSION);
    assert_eq!(job["agreement"]["sha256"], sha.as_str());
    assert!(
        job["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["note"] == format!("Agreement SHA-256 {sha}"))
    );
    assert!(notices(app, h.creative).await[0].starts_with("Tavern Dev accepted your offer"));
    // Half now, half on delivery.
    let payments = |id: String| async move {
        sqlx::query_as::<_, (String, String, i32, String)>(
            "select part, due_on, amount_minor, status from job_payments where job_id = $1::uuid order by due_on, part",
        )
        .bind(id)
        .fetch_all(&app.db)
        .await
        .unwrap()
    };
    assert_eq!(
        payments(id.clone()).await,
        [
            ("deposit".into(), "accept".into(), 6001, "due".into()),
            ("final".into(), "delivery".into(), 6000, "waiting".into())
        ]
    );
    // No "start now" ticked: the withdrawal right isn't waived.
    let start_now: Option<OffsetDateTime> =
        sqlx::query_scalar("select start_now_ack_at from jobs where id = $1::uuid")
            .bind(&id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(start_now.is_none());
    // Accepted once.
    h.refused(
        &id,
        &h.tavern,
        "accept",
        accept.clone(),
        StatusCode::CONFLICT,
    )
    .await;

    let upfront = h.job("upfront", "offered").await;
    let job = h
        .ok(
            &upfront,
            &h.tavern,
            "accept",
            json!({ "offer_count": 1, "exact_terms_ack": true, "start_now": true }),
        )
        .await;
    assert_eq!(job["withdrawal"]["started_early"], true);
    assert_eq!(
        payments(upfront).await,
        [("full".into(), "accept".into(), 12001, "due".into())]
    );
    let on_delivery = h.job("on_delivery", "accepted").await;
    assert_eq!(h.status(&on_delivery).await, "in_progress");
    assert_eq!(
        payments(on_delivery).await,
        [("full".into(), "delivery".into(), 12001, "waiting".into())]
    );
    // Hiring as a business: "start now" isn't recorded, as there's no right to waive.
    let (_, business) = h.hire(json!({ "client_is_business": true })).await;
    let business = business["id"].as_str().unwrap().to_string();
    h.ok(&business, &h.vera, "offer", sprite_offer("upfront"))
        .await;
    let job = h
        .ok(
            &business,
            &h.tavern,
            "accept",
            json!({ "offer_count": 1, "exact_terms_ack": true, "start_now": true }),
        )
        .await;
    assert_eq!(job["withdrawal"], Value::Null);
    let start_now: Option<OffsetDateTime> =
        sqlx::query_scalar("select start_now_ack_at from jobs where id = $1::uuid")
            .bind(&business)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(start_now.is_none());
}

// ---------------------------------------------------------------------------------------------
// Sign-off, delivery, final files, cancelling, withdrawing, reviewing (people-024 … people-031)

fn days_ahead(days: i64) -> String {
    (OffsetDateTime::now_utc().date() + time::Duration::days(days)).to_string()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_direction_is_sent_once_and_signed_off_by_the_client(db: PgPool) {
    let h = hiring(db, "US").await;
    let mut o = sprite_offer("upfront");
    o["sign_off"] = json!(true);
    let id = h.job_with(o, "in_progress").await;
    let link = |url: &str| json!({ "links": [url] });
    // The creative sends it, with files or a note saying where they are.
    h.refused(
        &id,
        &h.tavern,
        "sign-off/send",
        link("https://a.example/s.png"),
        StatusCode::CONFLICT,
    )
    .await;
    for (body, want) in [
        (json!({}), "links"),
        (json!({ "note": "   " }), "links"),
        (link("http://a.example/s.png"), "links"),
        (link("not a link"), "links"),
        (
            json!({ "links": vec!["https://a.example/s.png"; 6] }),
            "links",
        ),
        (json!({ "note": "x".repeat(2001) }), "note"),
    ] {
        let (status, err) = h.step(&id, &h.vera, "sign-off/send", body.clone()).await;
        assert_eq!(
            (status, field(&err)),
            (StatusCode::UNPROCESSABLE_ENTITY, want),
            "{body}"
        );
    }
    // The client can't sign off on what wasn't sent.
    h.refused(&id, &h.tavern, "sign-off", json!({}), StatusCode::CONFLICT)
        .await;
    let job = h
        .ok(
            &id,
            &h.vera,
            "sign-off/send",
            json!({ "note": "Sketch in the folder.", "links": vec!["https://a.example/s.png"; 5] }),
        )
        .await;
    assert_eq!(job["status"], "awaiting_sign_off");
    assert!(notices(&h.app, h.client).await[0].contains("for your sign-off"));
    // Sent once.
    h.refused(
        &id,
        &h.vera,
        "sign-off/send",
        link("https://a.example/t.png"),
        StatusCode::CONFLICT,
    )
    .await;
    h.refused(&id, &h.vera, "sign-off", json!({}), StatusCode::CONFLICT)
        .await;
    let job = h.ok(&id, &h.tavern, "sign-off", json!({})).await;
    assert_eq!(job["status"], "in_progress");
    assert!(job["signed_off_at"].is_string());
    assert!(notices(&h.app, h.creative).await[0].contains("signed off on"));
    h.refused(&id, &h.tavern, "sign-off", json!({}), StatusCode::CONFLICT)
        .await;
    h.refused(
        &id,
        &h.vera,
        "sign-off/send",
        link("https://a.example/u.png"),
        StatusCode::CONFLICT,
    )
    .await;
    // An offer without a sign-off has nothing to send.
    let plain = h.job("upfront", "in_progress").await;
    h.refused(
        &plain,
        &h.vera,
        "sign-off/send",
        link("https://a.example/s.png"),
        StatusCode::CONFLICT,
    )
    .await;
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn delivering_and_accepting_complete_a_paid_job(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let id = h.job("upfront", "accepted").await;
    let files = json!({ "links": ["https://drive.example/knight.zip"] });
    // Not before it's paid for; only by the creative; links checked.
    h.refused(&id, &h.vera, "deliver", files.clone(), StatusCode::CONFLICT)
        .await;
    h.ok(&id, &h.tavern, "pay", json!({})).await;
    h.refused(
        &id,
        &h.tavern,
        "deliver",
        files.clone(),
        StatusCode::CONFLICT,
    )
    .await;
    let (status, body) = h
        .step(
            &id,
            &h.vera,
            "deliver",
            json!({ "links": ["ftp://x.example/a"] }),
        )
        .await;
    assert_eq!(
        (status, field(&body)),
        (StatusCode::UNPROCESSABLE_ENTITY, "links")
    );
    let (status, _) = h.step(&id, &h.vera, "deliver", json!({})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    // Nothing to accept yet.
    h.refused(
        &id,
        &h.tavern,
        "accept-delivery",
        json!({}),
        StatusCode::CONFLICT,
    )
    .await;
    let job = h.ok(&id, &h.vera, "deliver", files.clone()).await;
    assert_eq!(job["status"], "delivered");
    assert_eq!(
        job["events"].as_array().unwrap().last().unwrap()["kind"],
        "delivered"
    );
    assert!(notices(app, h.client).await[0].contains("delivered"));
    h.refused(&id, &h.vera, "deliver", files.clone(), StatusCode::CONFLICT)
        .await;
    // Only the client accepts; with nothing owed it completes and credits the work.
    h.refused(
        &id,
        &h.vera,
        "accept-delivery",
        json!({}),
        StatusCode::CONFLICT,
    )
    .await;
    let job = h.ok(&id, &h.tavern, "accept-delivery", json!({})).await;
    assert_eq!(
        (
            job["status"].as_str(),
            job["credited"].as_bool(),
            job["completed_automatically"].as_bool()
        ),
        (Some("completed"), Some(true), Some(false))
    );
    assert!(notices(app, h.creative).await[0].contains("is complete"));
    h.refused(
        &id,
        &h.tavern,
        "accept-delivery",
        json!({}),
        StatusCode::CONFLICT,
    )
    .await;
    let page = call(app, req(Method::GET, "/v1/apps/tavern")).await.1;
    assert_eq!(page["credits"][0]["role"], "Art");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn final_files_follow_the_last_payment_and_credit_only_if_agreed(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let mut o = sprite_offer("on_delivery");
    o["credit"] = json!(false);
    let id = h.job_with(o, "delivered").await;
    let files = json!({ "links": ["https://drive.example/final.zip"] });
    // Not while the last payment is owed.
    h.refused(
        &id,
        &h.vera,
        "final-files",
        files.clone(),
        StatusCode::CONFLICT,
    )
    .await;
    h.ok(&id, &h.tavern, "pay", json!({})).await;
    h.refused(
        &id,
        &h.tavern,
        "final-files",
        files.clone(),
        StatusCode::CONFLICT,
    )
    .await;
    let (status, _) = h
        .step(&id, &h.vera, "final-files", json!({ "note": "" }))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let job = h.ok(&id, &h.vera, "final-files", files.clone()).await;
    assert_eq!(
        (job["status"].as_str(), job["credited"].as_bool()),
        (Some("completed"), Some(true))
    );
    assert!(job["final_files_at"].is_string());
    h.refused(&id, &h.vera, "final-files", files, StatusCode::CONFLICT)
        .await;
    // Without credit, the project page doesn't name them, nor does "since the new work".
    let page = call(app, req(Method::GET, "/v1/apps/tavern")).await.1;
    assert_eq!(page["credits"], json!([]));
    let (_, ratings) = call(app, req(Method::GET, "/v1/apps/tavern/ratings")).await;
    assert_eq!(ratings["since_new_work"]["credits"], json!([]));
    let (_, vera_page) = call(app, req(Method::GET, "/v1/people/vera")).await;
    assert_eq!(vera_page["worked_on"], json!([]));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn creatives_cancel_with_a_reason_and_clients_only_before_paying(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let (_, stranger) = app.user("Stranger", "US").await;

    // The client backs out before paying anything.
    let id = h.job("upfront", "accepted").await;
    h.refused(&id, &stranger, "cancel", json!({}), StatusCode::NOT_FOUND)
        .await;
    let job = h.ok(&id, &h.tavern, "cancel", json!({})).await;
    assert_eq!(job["status"], "cancelled");
    assert_eq!(
        job["payments"][0]["status"], "waiting",
        "nothing is due any more"
    );
    assert!(notices(app, h.creative).await[0].contains("was cancelled"));

    // Once something's paid, only the creative can cancel, and must say why.
    let id = h.job("on_delivery", "delivered").await;
    h.ok(&id, &h.vera, "conversation/open", json!({})).await;
    h.ok(
        &id,
        &h.tavern,
        "change/request",
        json!({ "note": "Add a shield?" }),
    )
    .await;
    h.refused(
        &id,
        &h.tavern,
        "cancel",
        json!({ "note": "Changed my mind" }),
        StatusCode::CONFLICT,
    )
    .await;
    let (status, body) = h
        .step(&id, &h.vera, "cancel", json!({ "note": "  " }))
        .await;
    assert_eq!(
        (status, field(&body)),
        (StatusCode::UNPROCESSABLE_ENTITY, "note")
    );
    let job = h
        .ok(
            &id,
            &h.vera,
            "cancel",
            json!({ "note": "I'm ill and can't finish." }),
        )
        .await;
    assert_eq!(job["status"], "cancelled");
    assert_eq!(
        job["payments"][0]["status"], "waiting",
        "the delivery payment isn't owed"
    );
    assert_eq!(job["changes"][0]["status"], "withdrawn");
    assert_eq!(job["conversation"]["open"], false);
    assert!(actions(&job).iter().all(|a| *a != "open_problem"));
    let ended: (Option<Uuid>, bool) =
        sqlx::query_as("select ended_by, ended_at is not null from jobs where id = $1::uuid")
            .bind(&id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(ended, (Some(h.creative), true));
    // Nothing more owed: hiring again is fine.
    assert_eq!(h.hire(json!({})).await.0, StatusCode::OK);

    // A client who paid the first half can't cancel when a change's price is due.
    let id = h.job("split", "in_progress").await;
    h.ok(
        &id,
        &h.vera,
        "change/offer",
        json!({ "title": "Shield", "extra_price_minor": 2000, "new_due_date": days_ahead(30) }),
    )
    .await;
    h.ok(
        &id,
        &h.tavern,
        "change/accept",
        json!({ "number": 1, "exact_terms_ack": true }),
    )
    .await;
    assert_eq!(h.status(&id).await, "awaiting_payment");
    h.refused(&id, &h.tavern, "cancel", json!({}), StatusCode::CONFLICT)
        .await;

    // Finished work can't be cancelled.
    let id = h.job("upfront", "completed").await;
    h.refused(
        &id,
        &h.vera,
        "cancel",
        json!({ "note": "Oops" }),
        StatusCode::CONFLICT,
    )
    .await;
    h.refused(&id, &h.tavern, "cancel", json!({}), StatusCode::CONFLICT)
        .await;
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn consumers_withdraw_only_within_their_window(db: PgPool) {
    let h = hiring(db, "GB").await;
    let app = &h.app;
    // Paid, then withdrawn: the job ends; the refund is the creative's to make (nothing is moved).
    let id = h.job("upfront", "in_progress").await;
    h.refused(&id, &h.vera, "withdraw", json!({}), StatusCode::CONFLICT)
        .await;
    let job = h.ok(&id, &h.tavern, "withdraw", json!({})).await;
    assert_eq!(job["status"], "withdrawn");
    assert_eq!(
        job["status_text"],
        "Withdrawn by the client. Vera Okafor refunds what's owed within 14 days."
    );
    assert_eq!(
        (job["paid_minor"].as_i64(), job["refunded_minor"].as_i64()),
        (Some(12001), Some(0))
    );
    assert!(notices(app, h.creative).await[0].starts_with("Tavern Dev withdrew from"));
    let withdrawn_at: Option<OffsetDateTime> =
        sqlx::query_scalar("select withdrawn_at from jobs where id = $1::uuid")
            .bind(&id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(withdrawn_at.is_some());
    h.refused(&id, &h.tavern, "withdraw", json!({}), StatusCode::CONFLICT)
        .await;

    // After the 14 days: gone.
    let id = h.job("upfront", "in_progress").await;
    sqlx::query(
        "update jobs set withdrawal_until = now() - interval '1 minute' where id = $1::uuid",
    )
    .bind(&id)
    .execute(&app.db)
    .await
    .unwrap();
    let job = h.view(&id, &h.tavern).await;
    assert_eq!(job["withdrawal"]["open"], false);
    h.refused(&id, &h.tavern, "withdraw", json!({}), StatusCode::CONFLICT)
        .await;

    // Hiring as a business, or from someone who isn't a trader: no right to withdraw.
    let (_, job) = h.hire(json!({ "client_is_business": true })).await;
    let business = job["id"].as_str().unwrap().to_string();
    h.ok(&business, &h.vera, "offer", sprite_offer("upfront"))
        .await;
    h.ok(
        &business,
        &h.tavern,
        "accept",
        json!({ "offer_count": 1, "exact_terms_ack": true }),
    )
    .await;
    h.refused(
        &business,
        &h.tavern,
        "withdraw",
        json!({}),
        StatusCode::CONFLICT,
    )
    .await;
    exec(app, "update creatives set trader = false").await;
    let private = h.job("upfront", "accepted").await;
    h.refused(
        &private,
        &h.tavern,
        "withdraw",
        json!({}),
        StatusCode::CONFLICT,
    )
    .await;
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn only_the_client_reviews_and_only_finished_work(db: PgPool) {
    let h = hiring(db, "US").await;
    let review = |id: &str, cookie: &str, body: Value| {
        call(
            &h.app,
            req(Method::PUT, format!("/v1/jobs/{id}/review"))
                .cookie(cookie)
                .json(body),
        )
    };
    let id = h.job("upfront", "delivered").await;
    let (status, _) = review(&id, &h.tavern, json!({ "vote": "not_slop" })).await;
    assert_eq!(status, StatusCode::CONFLICT, "not before it's accepted");
    // Final files pending counts as accepted (it's been paid in full).
    let paid = h.job("on_delivery", "delivered").await;
    h.ok(&paid, &h.tavern, "pay", json!({})).await;
    assert_eq!(h.status(&paid).await, "final_files");
    let (status, job) = review(
        &paid,
        &h.tavern,
        json!({ "vote": "not_slop", "body": "Fast." }),
    )
    .await;
    assert_eq!(
        (status, job["review"]["body"].as_str()),
        (StatusCode::OK, Some("Fast."))
    );

    h.ok(&id, &h.tavern, "accept-delivery", json!({})).await;
    let (status, _) = review(&id, &h.vera, json!({ "vote": "not_slop" })).await;
    assert_eq!(status, StatusCode::CONFLICT, "not the creative");
    for (body, want) in [
        (json!({ "vote": "meh" }), "vote"),
        (json!({ "vote": "slop", "body": "x".repeat(2001) }), "body"),
    ] {
        let (status, err) = review(&id, &h.tavern, body).await;
        assert_eq!(
            (status, field(&err)),
            (StatusCode::UNPROCESSABLE_ENTITY, want)
        );
    }
    let (status, _) = review(&id, &h.tavern, json!({ "vote": "slop", "stars": 1 })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, job) = review(&id, &h.tavern, json!({ "vote": "slop" })).await;
    assert_eq!(
        (status, job["review"]["vote"].as_str()),
        (StatusCode::OK, Some("slop"))
    );
    assert_eq!(
        count(&h.app, "select count(*) from creative_reviews").await,
        2
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
#[ignore = "nb2: withdrawal ignores the start-now acknowledgement and completion (HUM-2, HUM-4): allowed after the work is fully done and paid, refused once auto-completion ends the job inside the 14 days"]
async fn withdrawing_follows_the_start_now_acknowledgement_not_completion(db: PgPool) {
    let h = hiring(db, "DE").await;
    // Asked to start now, then the work was fully done and paid for: the right is gone.
    let (_, job) = h.hire(json!({})).await;
    let started = job["id"].as_str().unwrap().to_string();
    h.ok(&started, &h.vera, "offer", sprite_offer("on_delivery"))
        .await;
    h.ok(
        &started,
        &h.tavern,
        "accept",
        json!({ "offer_count": 1, "exact_terms_ack": true, "start_now": true }),
    )
    .await;
    h.ok(
        &started,
        &h.vera,
        "deliver",
        json!({ "note": "In the folder." }),
    )
    .await;
    h.ok(&started, &h.tavern, "pay", json!({})).await;
    h.refused(
        &started,
        &h.tavern,
        "withdraw",
        json!({}),
        StatusCode::CONFLICT,
    )
    .await;
    // Didn't ask to start now: completing automatically after 7 days doesn't shorten the 14.
    let waiting = h.job("upfront", "delivered").await;
    slop_api::people::jobs::auto_complete(
        &h.app.db,
        OffsetDateTime::now_utc() + time::Duration::days(8),
    )
    .await
    .unwrap();
    assert_eq!(h.status(&waiting).await, "completed");
    let job = h.ok(&waiting, &h.tavern, "withdraw", json!({})).await;
    assert_eq!(job["status"], "withdrawn");
}

// ---------------------------------------------------------------------------------------------
// Changes to Agreement (people-032 … people-035)

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn clients_ask_for_changes_while_work_is_under_way(db: PgPool) {
    let h = hiring(db, "US").await;
    let ask = |note: &str| json!({ "note": note });
    // Not before work starts, nor after it's done.
    for step in ["requested", "offered", "accepted"] {
        let id = h.job("upfront", step).await;
        h.refused(
            &id,
            &h.tavern,
            "change/request",
            ask("Bigger?"),
            StatusCode::CONFLICT,
        )
        .await;
    }
    let done = h.job("upfront", "completed").await;
    h.refused(
        &done,
        &h.tavern,
        "change/request",
        ask("Bigger?"),
        StatusCode::CONFLICT,
    )
    .await;
    let id = h.job("upfront", "in_progress").await;
    h.refused(
        &id,
        &h.vera,
        "change/request",
        ask("Bigger?"),
        StatusCode::CONFLICT,
    )
    .await;
    for (note, want) in [("  ", "note"), (&*"x".repeat(2001), "note")] {
        let (status, body) = h.step(&id, &h.tavern, "change/request", ask(note)).await;
        assert_eq!(
            (status, field(&body)),
            (StatusCode::UNPROCESSABLE_ENTITY, want)
        );
    }
    let job = h
        .ok(&id, &h.tavern, "change/request", ask("Could it be bigger?"))
        .await;
    assert_eq!(
        (
            job["changes"][0]["number"].as_i64(),
            job["changes"][0]["requested_note"].as_str()
        ),
        (Some(1), Some("Could it be bigger?"))
    );
    assert!(notices(&h.app, h.creative).await[0].contains("asked for a change"));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn creatives_offer_changes_with_a_price_and_date(db: PgPool) {
    let h = hiring(db, "US").await;
    let change = |price: i64, due: String| json!({ "title": "Add a shield", "scope_md": "One shield.", "extra_price_minor": price, "new_due_date": due });
    let id = h.job("upfront", "in_progress").await;
    h.refused(
        &id,
        &h.tavern,
        "change/offer",
        change(2000, days_ahead(30)),
        StatusCode::CONFLICT,
    )
    .await;
    for (body, want) in [
        (change(999, days_ahead(30)), "extra_price_minor"),
        (change(1_000_001, days_ahead(30)), "extra_price_minor"),
        (change(-5, days_ahead(30)), "extra_price_minor"),
        (change(2000, days_ahead(-1)), "new_due_date"),
        (change(2000, "soon".into()), "new_due_date"),
        (
            json!({ "title": " ", "extra_price_minor": 0, "new_due_date": days_ahead(3) }),
            "title",
        ),
    ] {
        let (status, err) = h.step(&id, &h.vera, "change/offer", body.clone()).await;
        assert_eq!(
            (status, field(&err)),
            (StatusCode::UNPROCESSABLE_ENTITY, want),
            "{body}"
        );
    }
    // Unasked: it becomes its own numbered change.
    let job = h
        .ok(&id, &h.vera, "change/offer", change(2000, days_ahead(30)))
        .await;
    let c = &job["changes"][0];
    assert_eq!(
        (
            c["number"].as_i64(),
            c["status"].as_str(),
            c["requested_note"].clone(),
            c["extra_price_minor"].as_i64()
        ),
        (Some(1), Some("offered"), Value::Null, Some(2000))
    );
    assert!(notices(&h.app, h.client).await[0].contains("offered a Change to Agreement"));
    // One at a time.
    h.refused(
        &id,
        &h.vera,
        "change/offer",
        change(0, days_ahead(30)),
        StatusCode::CONFLICT,
    )
    .await;
    h.refused(
        &id,
        &h.tavern,
        "change/request",
        json!({ "note": "Also a hat" }),
        StatusCode::CONFLICT,
    )
    .await;
    // Nothing to change on finished work.
    let done = h.job("upfront", "completed").await;
    h.refused(
        &done,
        &h.vera,
        "change/offer",
        change(0, days_ahead(30)),
        StatusCode::CONFLICT,
    )
    .await;
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn changes_are_declined_or_withdrawn_by_the_right_side(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let id = h.job("upfront", "in_progress").await;
    h.refused(
        &id,
        &h.tavern,
        "change/decline",
        json!({}),
        StatusCode::CONFLICT,
    )
    .await;
    let offer =
        json!({ "title": "Shield", "extra_price_minor": 1500, "new_due_date": days_ahead(20) });

    // The client asked; the creative declines (the work stays as agreed); the client is told.
    h.ok(
        &id,
        &h.tavern,
        "change/request",
        json!({ "note": "A shield?" }),
    )
    .await;
    h.refused(
        &id,
        &h.tavern,
        "change/decline",
        json!({}),
        StatusCode::CONFLICT,
    )
    .await;
    let (status, body) = h
        .step(
            &id,
            &h.vera,
            "change/decline",
            json!({ "note": "x".repeat(1001) }),
        )
        .await;
    assert_eq!(
        (status, field(&body)),
        (StatusCode::UNPROCESSABLE_ENTITY, "note")
    );
    let job = h
        .ok(
            &id,
            &h.vera,
            "change/decline",
            json!({ "note": "Not in scope." }),
        )
        .await;
    assert_eq!(
        (
            job["changes"][0]["status"].as_str(),
            job["changes"][0]["decline_note"].as_str()
        ),
        (Some("declined"), Some("Not in scope."))
    );
    assert!(
        notices(app, h.client).await[0].contains("declined"),
        "{:?}",
        notices(app, h.client).await
    );

    // The creative offered; the client declines; the creative is told.
    h.ok(&id, &h.vera, "change/offer", offer.clone()).await;
    let job = h.ok(&id, &h.tavern, "change/decline", json!({})).await;
    assert_eq!(job["changes"][1]["status"], "declined");
    assert_eq!(job["changes"][1]["decline_note"], Value::Null);
    assert!(
        notices(app, h.creative).await[0]
            .starts_with("Tavern Dev declined the Change to Agreement")
    );

    // The creative takes back their own offer; the client is told.
    h.ok(&id, &h.vera, "change/offer", offer).await;
    let job = h.ok(&id, &h.vera, "change/decline", json!({})).await;
    assert_eq!(job["changes"][2]["status"], "withdrawn");
    assert!(
        notices(app, h.client).await[0].contains("withdrew"),
        "{:?}",
        notices(app, h.client).await
    );
    assert_eq!(h.status(&id).await, "in_progress");
    assert!(
        job["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "change_withdrawn")
    );
    // Nothing waiting now.
    h.refused(
        &id,
        &h.vera,
        "change/decline",
        json!({}),
        StatusCode::CONFLICT,
    )
    .await;
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn accepting_a_change_restarts_the_work_on_the_new_terms(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    // Delivered with half still to pay; a free change sends it back to work, nothing due till
    // the next delivery.
    let id = h.job("split", "delivered").await;
    let job = h.view(&id, &h.tavern).await;
    assert_eq!(job["payments"][1]["status"], "due");
    let due = days_ahead(40);
    h.ok(
        &id,
        &h.vera,
        "change/offer",
        json!({ "title": "Fix the cape", "extra_price_minor": 0, "new_due_date": due }),
    )
    .await;
    let accept = |n: i64| json!({ "number": n, "exact_terms_ack": true });
    h.refused(
        &id,
        &h.vera,
        "change/accept",
        accept(1),
        StatusCode::CONFLICT,
    )
    .await;
    h.refused(
        &id,
        &h.tavern,
        "change/accept",
        accept(2),
        StatusCode::CONFLICT,
    )
    .await;
    let job = h.ok(&id, &h.tavern, "change/accept", accept(1)).await;
    assert_eq!(job["status"], "in_progress");
    assert_eq!(job["due_date"], due);
    assert_eq!(
        job["payments"].as_array().unwrap().len(),
        2,
        "no payment for a free change"
    );
    assert_eq!(job["payments"][1]["status"], "waiting");
    assert!(
        job["changes"][0]["text"]
            .as_str()
            .unwrap()
            .contains("**Extra price:** none.")
    );
    assert_eq!(job["changes"][0]["status"], "accepted");
    assert!(notices(app, h.creative).await[0].contains("accepted the Change to Agreement"));
    let delivered_at: Option<OffsetDateTime> =
        sqlx::query_scalar("select delivered_at from jobs where id = $1::uuid")
            .bind(&id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(delivered_at.is_none());
    // Accepted once.
    h.refused(
        &id,
        &h.tavern,
        "change/accept",
        accept(1),
        StatusCode::CONFLICT,
    )
    .await;

    // Two presses at once: one acceptance, one payment.
    h.ok(
        &id,
        &h.vera,
        "change/offer",
        json!({ "title": "Add a cape", "extra_price_minor": 3000, "new_due_date": days_ahead(50) }),
    )
    .await;
    let press = || h.step(&id, &h.tavern, "change/accept", accept(2));
    let ((a, _), (b, _)) = tokio::join!(press(), press());
    let mut statuses = [a, b];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::OK, StatusCode::CONFLICT]);
    assert_eq!(
        count(
            app,
            "select count(*) from job_payments where part = 'change'"
        )
        .await,
        1
    );
    assert_eq!(h.status(&id).await, "awaiting_payment");
}

// ---------------------------------------------------------------------------------------------
// Job conversations (people-036 … people-041)

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn clients_ask_for_a_conversation_and_creatives_open_and_close_it(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let id = h.job("upfront", "requested").await;
    h.refused(
        &id,
        &h.vera,
        "conversation/request",
        json!({}),
        StatusCode::CONFLICT,
    )
    .await;
    let (status, body) = h
        .step(
            &id,
            &h.tavern,
            "conversation/request",
            json!({ "note": "x".repeat(301) }),
        )
        .await;
    assert_eq!(
        (status, field(&body)),
        (StatusCode::UNPROCESSABLE_ENTITY, "note")
    );
    let job = h
        .ok(
            &id,
            &h.tavern,
            "conversation/request",
            json!({ "note": "Can we talk about the style?" }),
        )
        .await;
    assert_eq!(
        job["conversation"]["request_note"],
        "Can we talk about the style?"
    );
    assert!(!actions(&job).contains(&"request_conversation"));
    assert!(notices(app, h.creative).await[0].contains("asked for a conversation"));
    h.refused(
        &id,
        &h.tavern,
        "conversation/request",
        json!({}),
        StatusCode::CONFLICT,
    )
    .await;

    // The creative opens it (not the client), once; asking again while it's open is pointless.
    h.refused(
        &id,
        &h.tavern,
        "conversation/open",
        json!({}),
        StatusCode::CONFLICT,
    )
    .await;
    let job = h.ok(&id, &h.vera, "conversation/open", json!({})).await;
    assert_eq!(
        (
            job["conversation"]["open"].as_bool(),
            job["conversation"]["requested_at"].clone()
        ),
        (Some(true), Value::Null)
    );
    assert!(notices(app, h.client).await[0].contains("opened the conversation"));
    h.refused(
        &id,
        &h.vera,
        "conversation/open",
        json!({}),
        StatusCode::CONFLICT,
    )
    .await;
    h.refused(
        &id,
        &h.tavern,
        "conversation/request",
        json!({}),
        StatusCode::CONFLICT,
    )
    .await;

    // The creative closes it; the client can't.
    h.refused(
        &id,
        &h.tavern,
        "conversation/close",
        json!({}),
        StatusCode::CONFLICT,
    )
    .await;
    let job = h.ok(&id, &h.vera, "conversation/close", json!({})).await;
    assert_eq!(job["conversation"]["open"], false);
    h.refused(
        &id,
        &h.vera,
        "conversation/close",
        json!({}),
        StatusCode::CONFLICT,
    )
    .await;
    // Asked again later, and reopened.
    h.ok(&id, &h.tavern, "conversation/request", json!({}))
        .await;
    h.ok(&id, &h.vera, "conversation/open", json!({})).await;
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
#[ignore = "nb3: a creative can reopen the conversation on a declined, cancelled or withdrawn job, after ending it closed it"]
async fn conversations_stay_closed_once_a_job_has_ended(db: PgPool) {
    let h = hiring(db, "US").await;
    let id = h.job("upfront", "offered").await;
    h.ok(&id, &h.tavern, "decline", json!({ "note": "No thanks." }))
        .await;
    h.refused(
        &id,
        &h.vera,
        "conversation/open",
        json!({}),
        StatusCode::CONFLICT,
    )
    .await;
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn job_messages_are_checked_and_rate_limited(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let id = h.job("upfront", "requested").await;
    h.ok(&id, &h.vera, "conversation/open", json!({})).await;
    let say = |body: &str| json!({ "body": body, "recording_ack": true });
    for (body, want) in [(say("   "), "body"), (say(&"x".repeat(4001)), "body")] {
        let (status, err) = h.step(&id, &h.vera, "messages", body).await;
        assert_eq!(
            (status, field(&err)),
            (StatusCode::UNPROCESSABLE_ENTITY, want)
        );
    }
    let job = h.ok(&id, &h.vera, "messages", say(&"x".repeat(4000))).await;
    assert_eq!(job["conversation"]["acknowledged"], true);
    // Sixty a minute, refused ones counted.
    for _ in 0..59 {
        h.step(&id, &h.vera, "messages", say("")).await;
    }
    let (status, body) = h.step(&id, &h.vera, "messages", say("one more")).await;
    assert_eq!(
        (status, body["error"]["code"].as_str()),
        (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited"))
    );
    assert_eq!(count(app, "select count(*) from job_messages").await, 1);
    // The client has their own allowance.
    h.ok(&id, &h.tavern, "messages", say("Hello!")).await;
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_makers_agent_writes_in_the_job_conversation_for_its_person(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let (_, body) = call(
        app,
        post(
            "/v1/me/maker-tokens",
            &h.vera,
            json!({ "name": "Agent", "scope": "publish", "people": true }),
        ),
    )
    .await;
    let agent = format!("Bearer {}", body["secret"].as_str().unwrap());
    let as_agent = |path: String, body: Value| {
        call(
            app,
            req(Method::POST, path)
                .header("authorization", agent.clone())
                .json(body),
        )
    };
    let (_, job) = h.hire(json!({})).await;
    let id = job["id"].as_str().unwrap().to_string();
    // Quoting, opening the conversation and writing: yes. Its ticking the recording box is the
    // person's acknowledgement.
    let (status, _) = as_agent(format!("/v1/jobs/{id}/offer"), sprite_offer("upfront")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = as_agent(format!("/v1/jobs/{id}/conversation/open"), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, err) = as_agent(format!("/v1/jobs/{id}/messages"), json!({ "body": "Hi" })).await;
    assert_eq!(
        (status, field(&err)),
        (StatusCode::UNPROCESSABLE_ENTITY, "recording_ack")
    );
    let (status, job) = as_agent(
        format!("/v1/jobs/{id}/messages"),
        json!({ "body": "Hi, I'm drafting the quote.", "recording_ack": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(job["conversation"]["messages"][0]["author"], "vera");
    let acked: Option<OffsetDateTime> =
        sqlx::query_scalar("select creative_recording_ack_at from jobs where id = $1::uuid")
            .bind(&id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(acked.is_some());
    // Agreeing to things stays with the person.
    for step in ["pay", "deliver", "cancel", "withdraw"] {
        let (status, _) = as_agent(format!("/v1/jobs/{id}/{step}"), json!({})).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{step}");
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn each_side_downloads_the_conversation_while_slop_keeps_the_text(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let (_, stranger) = app.user("Stranger", "US").await;
    let id = h.job("upfront", "in_progress").await;
    h.ok(&id, &h.vera, "conversation/open", json!({})).await;
    h.ok(
        &id,
        &h.tavern,
        "messages",
        json!({ "body": "Blue, please.", "recording_ack": true }),
    )
    .await;
    assert_eq!(
        call(app, fetch(format!("/v1/jobs/{id}/conversation"), &stranger))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    // While it's still open.
    let res = app
        .send(fetch(format!("/v1/jobs/{id}/conversation"), &h.vera))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers()[header::CONTENT_DISPOSITION],
        format!("attachment; filename=\"slop-conversation-{id}.json\"").as_str()
    );
    let copy = body_json(res).await;
    assert_eq!(copy["job_id"], id.as_str());
    assert_eq!(copy["messages"][0]["body"], "Blue, please.");
    assert_eq!(copy["messages"][0]["author_id"], h.client.to_string());
    // Closed less than 30 days ago: still there.
    h.ok(&id, &h.vera, "conversation/close", json!({})).await;
    let n = slop_api::people::jobs::forget_conversations(
        &app.db,
        OffsetDateTime::now_utc() + time::Duration::days(29),
    )
    .await
    .unwrap();
    assert_eq!(n, 0);
    assert_eq!(
        call(app, fetch(format!("/v1/jobs/{id}/conversation"), &h.tavern))
            .await
            .0,
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn conversation_copies_are_evidence_only_for_an_open_problem_and_must_match(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let id = h.job("upfront", "in_progress").await;
    h.ok(&id, &h.vera, "conversation/open", json!({})).await;
    for (who, text) in [(&h.tavern, "One"), (&h.vera, "Two"), (&h.tavern, "Three")] {
        h.ok(
            &id,
            who,
            "messages",
            json!({ "body": text, "recording_ack": true }),
        )
        .await;
    }
    let copy = call(app, fetch(format!("/v1/jobs/{id}/conversation"), &h.tavern))
        .await
        .1;
    let evidence = |copy: Value| h.step(&id, &h.tavern, "evidence", json!({ "copy": copy }));
    // Only for an open problem or review dispute.
    let (status, _) = evidence(copy.clone()).await;
    assert_eq!(status, StatusCode::CONFLICT);
    h.ok(
        &id,
        &h.tavern,
        "problem",
        json!({ "statement": "Not as agreed." }),
    )
    .await;
    // Another job's copy isn't this job's.
    let other = h.job("upfront", "in_progress").await;
    let mut theirs = copy.clone();
    theirs["job_id"] = json!(other);
    let (status, body) = evidence(theirs).await;
    assert_eq!(
        (status, field(&body)),
        (StatusCode::UNPROCESSABLE_ENTITY, "copy")
    );
    // Dropping or reordering messages shows: each fingerprint includes the one before.
    let mut dropped = copy.clone();
    dropped["messages"].as_array_mut().unwrap().remove(1);
    let (_, checked) = evidence(dropped).await;
    assert_eq!(
        (
            checked["verified_messages"].as_i64(),
            checked["unverified_messages"].as_i64()
        ),
        (Some(1), Some(1))
    );
    let mut reordered = copy.clone();
    reordered["messages"].as_array_mut().unwrap().swap(0, 1);
    let (_, checked) = evidence(reordered).await;
    assert_eq!(checked["verified_messages"], 0);
    let (status, checked) = evidence(copy.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (
            checked["verified_messages"].as_i64(),
            checked["unverified_messages"].as_i64()
        ),
        (Some(3), Some(0))
    );
    assert_eq!(count(app, "select count(*) from job_evidence").await, 3);
    // Copies can be larger than other requests; what the record can keep is 400,000 characters,
    // and the request itself at most 2 MB.
    let mut long = copy.clone();
    long["messages"][0]["body"] = json!("x".repeat(300_000));
    let (status, checked) = evidence(long).await;
    assert_eq!(
        (status, checked["unverified_messages"].as_i64()),
        (StatusCode::OK, Some(3))
    );
    let mut longer = copy.clone();
    longer["messages"][0]["body"] = json!("x".repeat(500_000));
    let (status, body) = evidence(longer).await;
    assert_eq!(
        (status, field(&body)),
        (StatusCode::UNPROCESSABLE_ENTITY, "copy"),
        "{body}"
    );
    assert_eq!(count(app, "select count(*) from job_evidence").await, 4);
    let mut huge = copy;
    huge["messages"][0]["body"] = json!("x".repeat(2_100_000));
    let (status, _) = evidence(huge).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}
