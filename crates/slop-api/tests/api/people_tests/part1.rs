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
