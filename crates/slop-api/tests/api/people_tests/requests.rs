// SPDX-License-Identifier: AGPL-3.0-or-later
//! Open requests on People (people/work_requests.rs, migration 0098, DESIGN §15.9): who may post
//! and what, the cap of three and the daily limits, the board's one order and its filter, who
//! sees a request (signed out, under-18 views, blocks, the project out of the store), putting a
//! name forward and taking it back, the maker's list of names and their one notice, and running
//! out and being deleted.

use super::part1::{
    Hiring, call, count, creative_with, exec, fetch, field, hiring, notices, people_app, post,
};
use crate::*;

// ---------------------------------------------------------------------------------------------
// Helpers for this file

/// A request for Tavern Dev's "tavern", with `extra` fields put over it.
fn request_body(extra: Value) -> Value {
    let mut body = json!({
        "app_slug": "tavern", "skill": "pixel-art", "title": "Sign",
        "description_md": "A tavern sign.", "confirm_adult": true
    });
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    body
}

/// POST /v1/work-requests: the status and body.
async fn try_request(app: &TestApp, cookie: &str, extra: Value) -> (StatusCode, Value) {
    call(app, post("/v1/work-requests", cookie, request_body(extra))).await
}

/// Posts a request, which must work, and returns it.
async fn request(app: &TestApp, cookie: &str, extra: Value) -> Value {
    let (status, body) = try_request(app, cookie, extra).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

fn id_of(r: &Value) -> String {
    r["id"].as_str().unwrap().to_string()
}

/// GET of one request, signed out (add a cookie or header to the result).
fn one(id: &str) -> Req {
    req(Method::GET, format!("/v1/work-requests/{id}"))
}

/// The board as `r` asks for it: the titles, in order.
async fn titles(app: &TestApp, r: Req) -> Vec<String> {
    let (status, body) = call(app, r).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body.as_array()
        .unwrap()
        .iter()
        .map(|r| r["title"].as_str().unwrap().to_string())
        .collect()
}

fn board() -> Req {
    req(Method::GET, "/v1/work-requests")
}

/// PUT …/response with this package (or null).
async fn put_name(app: &TestApp, cookie: &str, id: &str, service: Value) -> (StatusCode, Value) {
    call(
        app,
        req(Method::PUT, format!("/v1/work-requests/{id}/response"))
            .cookie(cookie)
            .json(json!({ "service_id": service })),
    )
    .await
}

async fn take_back(app: &TestApp, cookie: &str, id: &str) -> StatusCode {
    app.send(req(Method::DELETE, format!("/v1/work-requests/{id}/response")).cookie(cookie))
        .await
        .status()
}

async fn close(app: &TestApp, cookie: &str, id: &str) -> (StatusCode, Value) {
    call(
        app,
        post(format!("/v1/work-requests/{id}/close"), cookie, json!({})),
    )
    .await
}

/// Another maker with a listed app of their own.
async fn maker(app: &TestApp, name: &str, slug: &str) -> (Uuid, String) {
    let (id, cookie) = app.user(name, "US").await;
    listed_with(app, id, slug, "released", "").await;
    (id, cookie)
}

/// A second creative who can be hired, with pixel art and a "Sprite sheet" package of their own.
async fn wren(app: &TestApp) -> (Uuid, String) {
    let mut p = profile("wren", true);
    p["name"] = json!("Wren Ash");
    creative_with(app, "wren", "US", p).await
}

/// The id of a creative's first package.
async fn first_package(app: &TestApp, handle: &str) -> String {
    sqlx::query_scalar("select services->0->>'id' from creatives where handle = $1")
        .bind(handle)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// The handles in a request's `owner.responses`, in order.
fn handles(r: &Value) -> Vec<String> {
    r["owner"]["responses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["person"]["handle"].as_str().unwrap().to_string())
        .collect()
}

/// The maker's own view of one request.
async fn as_maker(app: &TestApp, cookie: &str, id: &str) -> Value {
    let (status, body) = call(app, one(id).cookie(cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

async fn status_of(app: &TestApp, r: Req) -> StatusCode {
    app.send(r).await.status()
}

/// A report about a request, as `cookie`. Returns the status and body.
async fn report(app: &TestApp, cookie: &str, id: &str, reason: &str) -> (StatusCode, Value) {
    call(
        app,
        req(Method::POST, "/v1/reports").cookie(cookie).json(json!({
            "kind": "work_request", "subject": id, "reason": reason,
            "details": "It isn't for real work on the project.", "good_faith": true
        })),
    )
    .await
}

/// Every field a request has. Nothing about a budget, a price or how many answered.
const FIELDS: [&str; 12] = [
    "created_at",
    "description_md",
    "expires_at",
    "id",
    "maker",
    "mine",
    "owner",
    "project",
    "put_forward",
    "skill",
    "title",
    "wanted_by",
];

fn keys(v: &Value) -> Vec<&str> {
    let mut k: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    k.sort();
    k
}

// ---------------------------------------------------------------------------------------------
// Posting

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_maker_posts_a_request_and_everyone_can_read_it(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let wanted = (OffsetDateTime::now_utc().date() + time::Duration::days(10)).to_string();
    let r = request(app, &h.tavern, json!({ "wanted_by": wanted })).await;
    assert_eq!(keys(&r), FIELDS);
    assert_eq!(r["title"], "Sign");
    assert_eq!(r["description_md"], "A tavern sign.");
    assert_eq!(
        r["skill"],
        json!({ "slug": "pixel-art", "name": "Pixel Art" })
    );
    assert_eq!(r["project"], json!({ "slug": "tavern", "title": "tavern" }));
    assert_eq!(r["maker"], "Tavern Dev");
    assert_eq!(r["wanted_by"], json!(wanted));
    assert_eq!(r["mine"], true);
    assert_eq!(r["put_forward"], Value::Null);
    assert_eq!(
        r["owner"],
        json!({ "status": "open", "on_board": true, "ended_at": null, "hidden_reason": null,
                "responses": [], "responses_total": 0 })
    );
    // 30 days, from the column's default.
    let created = OffsetDateTime::parse(r["created_at"].as_str().unwrap(), &Rfc3339).unwrap();
    let expires = OffsetDateTime::parse(r["expires_at"].as_str().unwrap(), &Rfc3339).unwrap();
    assert_eq!(expires - created, time::Duration::days(30));
    let id = id_of(&r);

    // Signed out, even with no country (the under-18 view): on the board, about nobody.
    for r in [board(), board().header("x-country", "US")] {
        let (status, list) = call(app, r).await;
        assert_eq!(status, StatusCode::OK);
        let list = list.as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(keys(&list[0]), FIELDS);
        assert_eq!(list[0]["id"], json!(id));
        assert_eq!(
            (&list[0]["mine"], &list[0]["put_forward"], &list[0]["owner"]),
            (&json!(false), &Value::Null, &Value::Null)
        );
    }
    let (status, got) = call(app, one(&id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(keys(&got), FIELDS);
    assert_eq!((&got["mine"], &got["owner"]), (&json!(false), &Value::Null));
    assert_eq!(got["description_md"], "A tavern sign.");
    // Signed in, someone else: the same, never the owner part.
    let (_, got) = call(app, one(&id).cookie(&h.vera)).await;
    assert_eq!((&got["mine"], &got["owner"]), (&json!(false), &Value::Null));
    // On the board for its maker too, marked as theirs, without the owner part.
    let (_, list) = call(app, fetch("/v1/work-requests", &h.tavern)).await;
    assert_eq!(
        (&list[0]["mine"], &list[0]["owner"]),
        (&json!(true), &Value::Null)
    );
    // A UUID that isn't one, and a made-up id, are 404 alike.
    assert_eq!(status_of(app, one("x")).await, StatusCode::NOT_FOUND);
    assert_eq!(
        status_of(app, one(&Uuid::now_v7().to_string())).await,
        StatusCode::NOT_FOUND
    );
    // Signed out, nothing can be written.
    for r in [
        req(Method::POST, "/v1/work-requests").json(request_body(json!({}))),
        req(Method::POST, format!("/v1/work-requests/{id}/close")),
        req(Method::PUT, format!("/v1/work-requests/{id}/response")).json(json!({})),
        req(Method::DELETE, format!("/v1/work-requests/{id}/response")),
        req(Method::GET, "/v1/me/work-requests"),
    ] {
        assert_eq!(status_of(app, r).await, StatusCode::UNAUTHORIZED);
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn posting_needs_an_adult_the_people_terms_and_no_debt(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    // Test accounts haven't said they're 18 or older yet.
    let (status, body) = try_request(app, &h.tavern, json!({ "confirm_adult": false })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(field(&body), "confirm_adult");
    assert_eq!(
        body["error"]["message"],
        "People is for adults: confirm you're 18 or older"
    );

    sqlx::query("delete from legal_acceptances where user_id = $1 and kind = 'people'")
        .bind(h.client)
        .execute(&app.db)
        .await
        .unwrap();
    let (status, body) = try_request(app, &h.tavern, json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        (body["error"]["code"].as_str(), field(&body)),
        (Some("terms_required"), "people")
    );
    sqlx::query(
        "insert into legal_acceptances (user_id, kind, version)
         select $1, kind, version from legal_documents where kind = 'people'",
    )
    .bind(h.client)
    .execute(&app.db)
    .await
    .unwrap();

    // A job paid on delivery, delivered and completed by itself, never paid: owed.
    h.job("on_delivery", "delivered").await;
    slop_api::people::jobs::auto_complete(
        &app.db,
        OffsetDateTime::now_utc() + time::Duration::days(8),
    )
    .await
    .unwrap();
    let (status, body) = try_request(app, &h.tavern, json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let message = body["error"]["message"].as_str().unwrap();
    assert!(
        message.starts_with("you still owe Vera Okafor "),
        "{message}"
    );
    assert!(
        message.ends_with(" for finished work; pay it from your jobs before posting a request"),
        "{message}"
    );
    // Hiring says the same, its own way, as it did before.
    let (status, body) = h.hire(json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .ends_with("pay it from your jobs before hiring again"),
        "{body}"
    );
    assert_eq!(count(app, "select count(*) from work_requests").await, 0);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_request_is_for_one_of_your_own_projects_in_the_store(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    maker(app, "Mill Dev", "mill").await;
    for (slug, set) in [
        ("draft-one", "status = 'draft'"),
        ("gone-one", "status = 'delisted'"),
        ("private-one", "visibility = 'private'"),
        (
            "server-one",
            "parent_app_id = (select id from apps where slug = 'tavern'), branch_kind = 'server', platforms = '{linux}'",
        ),
    ] {
        listed_with(app, h.client, slug, "released", set).await;
    }
    for slug in [
        "mill",
        "draft-one",
        "gone-one",
        "private-one",
        "server-one",
        "nothing-here",
    ] {
        let (status, body) = try_request(app, &h.tavern, json!({ "app_slug": slug })).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{slug}: {body}");
        assert_eq!(field(&body), "app_slug", "{slug}");
        assert_eq!(
            body["error"]["message"], "pick one of your projects that's in the store",
            "{slug}"
        );
    }
    // An 18+ project can have one (under-18 viewers just don't see it).
    listed_with(app, h.client, "late-night", "released", "min_age = 18").await;
    request(app, &h.tavern, json!({ "app_slug": "late-night" })).await;
    assert_eq!(count(app, "select count(*) from work_requests").await, 1);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_request_checks_its_fields(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    // Two accounts, so neither gets near the daily limit.
    let (_, mill) = maker(app, "Mill Dev", "mill").await;
    let long = "x".repeat(2001);
    let today = OffsetDateTime::now_utc().date();
    let cases: [(&str, Value, &str, &str); 11] = [
        (&h.tavern, json!({ "title": "  " }), "title", "write it out"),
        (
            &h.tavern,
            json!({ "title": "x".repeat(101) }),
            "title",
            "up to 100 characters",
        ),
        (
            &h.tavern,
            json!({ "title": "Sign\nboard" }),
            "title",
            "one line",
        ),
        (
            &h.tavern,
            json!({ "description_md": "" }),
            "description_md",
            "write it out",
        ),
        (
            &h.tavern,
            json!({ "description_md": long }),
            "description_md",
            "up to 2000 characters",
        ),
        (
            &h.tavern,
            json!({ "description_md": "A bell\u{7}" }),
            "description_md",
            "has characters that aren't allowed",
        ),
        (
            &mill,
            json!({ "app_slug": "mill", "skill": "juggling" }),
            "skill",
            "pick a skill from the list",
        ),
        (
            &mill,
            json!({ "app_slug": "mill", "wanted_by": (today - time::Duration::days(1)).to_string() }),
            "wanted_by",
            "from today to two years ahead",
        ),
        (
            &mill,
            json!({ "app_slug": "mill", "wanted_by": (today + time::Duration::days(3 * 365)).to_string() }),
            "wanted_by",
            "from today to two years ahead",
        ),
        (
            &mill,
            json!({ "app_slug": "mill", "wanted_by": "soon" }),
            "wanted_by",
            "a date, YYYY-MM-DD",
        ),
        (
            &mill,
            json!({ "app_slug": "mill", "skill": "" }),
            "skill",
            "pick a skill from the list",
        ),
    ];
    for (cookie, extra, want_field, message) in cases {
        let (status, body) = try_request(app, cookie, extra.clone()).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {body}");
        assert_eq!(field(&body), want_field, "{extra}");
        assert_eq!(body["error"]["message"], message, "{extra}");
    }
    // A budget is no field at all: the body is refused before anything else.
    let (status, _) = try_request(app, &h.tavern, json!({ "budget": 500 })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(count(app, "select count(*) from work_requests").await, 0);

    // Line breaks and tabs are fine in the text; both texts are trimmed; a blank date is none;
    // today is allowed.
    let r = request(
        app,
        &h.tavern,
        json!({ "title": "  Sign  ", "description_md": "  Two lines:\n\tindented.  ", "wanted_by": " " }),
    )
    .await;
    assert_eq!(
        (&r["title"], &r["description_md"], &r["wanted_by"]),
        (
            &json!("Sign"),
            &json!("Two lines:\n\tindented."),
            &Value::Null
        )
    );
    let r = request(
        app,
        &mill,
        json!({ "app_slug": "mill", "wanted_by": today.to_string() }),
    )
    .await;
    assert_eq!(r["wanted_by"], json!(today.to_string()));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn three_open_requests_at_most(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let mut ids = Vec::new();
    for n in 1..=3 {
        ids.push(id_of(
            &request(app, &h.tavern, json!({ "title": format!("Sign {n}") })).await,
        ));
    }
    let (status, body) = try_request(app, &h.tavern, json!({ "title": "Sign 4" })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        body["error"]["message"],
        "you already have 3 open requests: close one first"
    );
    // Closing one makes room.
    assert_eq!(close(app, &h.tavern, &ids[0]).await.0, StatusCode::OK);
    request(app, &h.tavern, json!({ "title": "Sign 4" })).await;
    // A hidden request and one past its 30 days don't count, even before the hourly job.
    exec(
        app,
        &format!(
            "update work_requests set hidden_reason = 'Spam', hidden_at = now() where id = '{}'",
            ids[1]
        ),
    )
    .await;
    request(app, &h.tavern, json!({ "title": "Sign 5" })).await;
    exec(
        app,
        &format!(
            "update work_requests set expires_at = now() - interval '1 minute' where id = '{}'",
            ids[2]
        ),
    )
    .await;
    request(app, &h.tavern, json!({ "title": "Sign 6" })).await;
    assert_eq!(
        try_request(app, &h.tavern, json!({ "title": "Sign 7" }))
            .await
            .0,
        StatusCode::CONFLICT
    );

    // Two posts at once, with two open: exactly one more lands.
    let (mill_id, mill) = maker(app, "Mill Dev", "mill").await;
    for n in 1..=2 {
        request(
            app,
            &mill,
            json!({ "app_slug": "mill", "title": format!("Mill {n}") }),
        )
        .await;
    }
    let both = |title: &str| {
        post(
            "/v1/work-requests",
            &mill,
            request_body(json!({ "app_slug": "mill", "title": title })),
        )
    };
    let (a, b) = tokio::join!(app.send(both("Mill 3")), app.send(both("Mill 4")));
    let mut got = [a.status(), b.status()];
    got.sort();
    assert_eq!(got, [StatusCode::OK, StatusCode::CONFLICT]);
    let open: i64 = sqlx::query_scalar(
        "select count(*) from work_requests where user_id = $1 and status = 'open'",
    )
    .bind(mill_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(open, 3);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn posting_and_putting_a_name_forward_have_daily_limits(db: PgPool) {
    // This test means to reach both limits, so it makes more calls than any other may (A.13).
    let h = hiring(db, "US").await;
    let app = &h.app;
    let mut last = String::new();
    for n in 1..=10 {
        let r = request(app, &h.tavern, json!({ "title": format!("Sign {n}") })).await;
        last = id_of(&r);
        if n < 10 {
            assert_eq!(close(app, &h.tavern, &last).await.0, StatusCode::OK);
        }
    }
    let (status, body) = try_request(app, &h.tavern, json!({ "title": "Sign 11" })).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(
        body["error"]["message"],
        "too many new requests for now: try again in a while"
    );

    // Putting a name forward again counts too: the limit is checked first.
    for n in 1..=30 {
        let (status, body) = put_name(app, &h.vera, &last, Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{n}: {body}");
    }
    let (status, body) = put_name(app, &h.vera, &last, Value::Null).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(
        body["error"]["message"],
        "too many names put forward for now: try again in a while"
    );
    // Taking it back and closing have no limit of their own.
    assert_eq!(take_back(app, &h.vera, &last).await, StatusCode::NO_CONTENT);
    assert_eq!(close(app, &h.tavern, &last).await.0, StatusCode::OK);
}

// ---------------------------------------------------------------------------------------------
// The board

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_board_is_newest_first_and_filters_by_skill(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let (_, mill) = maker(app, "Mill Dev", "mill").await;
    // Posted in this order, hours apart (set in the database, so no clock is raced).
    let posts = [
        (&h.tavern, "tavern", "pixel-art", "Oldest", 4),
        (&mill, "mill", "pixel-art", "Older", 3),
        (&h.tavern, "tavern", "music", "Newer", 2),
        (&mill, "mill", "sound", "Newest", 1),
    ];
    for (cookie, slug, skill, title, hours) in posts {
        let r = request(
            app,
            cookie,
            json!({ "app_slug": slug, "skill": skill, "title": title }),
        )
        .await;
        exec(
            app,
            &format!(
                "update work_requests set created_at = now() - interval '{hours} hours',
                        expires_at = now() - interval '{hours} hours' + interval '30 days'
                 where id = '{}'",
                id_of(&r)
            ),
        )
        .await;
    }
    assert_eq!(
        titles(app, board()).await,
        ["Newest", "Newer", "Older", "Oldest"]
    );
    assert_eq!(
        titles(app, req(Method::GET, "/v1/work-requests?skill=music")).await,
        ["Newer"]
    );
    assert_eq!(
        titles(app, req(Method::GET, "/v1/work-requests?skill=pixel-art")).await,
        ["Older", "Oldest"]
    );
    assert!(
        titles(app, req(Method::GET, "/v1/work-requests?skill=juggling"))
            .await
            .is_empty()
    );
    // An empty filter is no filter, as in the directory.
    assert_eq!(
        titles(app, req(Method::GET, "/v1/work-requests?skill="))
            .await
            .len(),
        4
    );
    assert_eq!(
        titles(app, req(Method::GET, "/v1/work-requests?offset=2")).await,
        ["Older", "Oldest"]
    );
    assert_eq!(
        titles(app, req(Method::GET, "/v1/work-requests?offset=-5"))
            .await
            .len(),
        4
    );
    // The same order whoever asks, Vera included (she can be hired and lists pixel art).
    assert_eq!(
        titles(app, fetch("/v1/work-requests", &h.vera)).await,
        ["Newest", "Newer", "Older", "Oldest"]
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_request_leaves_the_board_with_its_project(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let id = id_of(&request(app, &h.tavern, json!({})).await);
    let (_, teen) = app.user("Teen", "GB").await;
    let on_board = |r: Req| async move { titles(app, r).await == ["Sign"] };
    assert!(on_board(board()).await);
    assert!(on_board(fetch("/v1/work-requests", &teen)).await);

    for set in [
        "status = 'delisted'",
        "visibility = 'private'",
        // Past its stage deadline: out of the store as if delisted.
        "stage = 'beta', stage_entered_at = now() - interval '400 days'",
    ] {
        exec(app, &format!("update apps set {set} where slug = 'tavern'")).await;
        for r in [
            board().header("x-country", "US"),
            fetch("/v1/work-requests", &h.vera),
        ] {
            assert!(titles(app, r).await.is_empty(), "{set}");
        }
        assert_eq!(
            status_of(app, one(&id).header("x-country", "US")).await,
            StatusCode::NOT_FOUND,
            "{set}"
        );
        assert_eq!(
            status_of(app, one(&id).cookie(&h.vera)).await,
            StatusCode::NOT_FOUND,
            "{set}"
        );
        // A name can't be put forward on it either, nor can it be reported.
        assert_eq!(
            put_name(app, &h.vera, &id, Value::Null).await.0,
            StatusCode::NOT_FOUND,
            "{set}"
        );
        assert_eq!(
            report(app, &h.vera, &id, "spam").await.0,
            StatusCode::NOT_FOUND,
            "{set}"
        );
        // Its maker still has it, told it isn't showing.
        let mine = as_maker(app, &h.tavern, &id).await;
        assert_eq!(
            (&mine["owner"]["status"], &mine["owner"]["on_board"]),
            (&json!("open"), &json!(false)),
            "{set}"
        );
        // Back in the store: back on the board, by itself.
        exec(
            app,
            "update apps set status = 'listed', visibility = 'public', stage = 'released'
             where slug = 'tavern'",
        )
        .await;
        assert!(on_board(board()).await, "{set}");
        assert_eq!(
            as_maker(app, &h.tavern, &id).await["owner"]["on_board"],
            true
        );
    }

    // 18+: gone for the under-18 views (signed out with no country or from the UK, and a UK
    // account), there for the full view. Still on the board as its maker is told.
    exec(app, "update apps set min_age = 18 where slug = 'tavern'").await;
    for r in [
        board(),
        board().header("x-country", "GB"),
        fetch("/v1/work-requests", &teen),
    ] {
        assert!(titles(app, r).await.is_empty());
    }
    for r in [one(&id), one(&id).cookie(&teen)] {
        assert_eq!(status_of(app, r).await, StatusCode::NOT_FOUND);
    }
    // An under-18 view can't report what it isn't shown.
    assert_eq!(
        report(app, &teen, &id, "spam").await.0,
        StatusCode::NOT_FOUND
    );
    assert!(on_board(board().header("x-country", "US")).await);
    assert!(on_board(fetch("/v1/work-requests", &h.vera)).await);
    assert_eq!(
        status_of(app, one(&id).cookie(&h.vera)).await,
        StatusCode::OK
    );
    assert_eq!(
        as_maker(app, &h.tavern, &id).await["owner"]["on_board"],
        true
    );

    // Closed, hidden or run out: off the board, so not reportable either. On the board, it is.
    exec(app, "update apps set min_age = 0 where slug = 'tavern'").await;
    let closed = id_of(&request(app, &h.tavern, json!({ "title": "Closed" })).await);
    close(app, &h.tavern, &closed).await;
    let hidden = id_of(&request(app, &h.tavern, json!({ "title": "Hidden" })).await);
    exec(
        app,
        &format!(
            "update work_requests set hidden_reason = 'Spam', hidden_at = now() where id = '{hidden}'"
        ),
    )
    .await;
    let ran_out = id_of(&request(app, &h.tavern, json!({ "title": "Ran out" })).await);
    exec(
        app,
        &format!(
            "update work_requests set expires_at = now() - interval '1 minute' where id = '{ran_out}'"
        ),
    )
    .await;
    for gone in [&closed, &hidden, &ran_out] {
        assert_eq!(
            report(app, &h.vera, gone, "spam").await.0,
            StatusCode::NOT_FOUND
        );
    }
    let (status, body) = report(app, &h.vera, &id, "spam").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        count(
            app,
            "select count(*) from reports where subject_kind = 'work_request'"
        )
        .await,
        1
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_suspended_makers_requests_leave_the_board(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let id = id_of(&request(app, &h.tavern, json!({})).await);
    for (set, lift) in [
        ("suspended_at = now()", "suspended_at = null"),
        ("closed_at = now()", "closed_at = null"),
    ] {
        exec(
            app,
            &format!("update users set {set} where id = '{}'", h.client),
        )
        .await;
        assert!(titles(app, board()).await.is_empty(), "{set}");
        assert_eq!(
            status_of(app, one(&id).cookie(&h.vera)).await,
            StatusCode::NOT_FOUND,
            "{set}"
        );
        assert_eq!(
            put_name(app, &h.vera, &id, Value::Null).await.0,
            StatusCode::NOT_FOUND,
            "{set}"
        );
        exec(
            app,
            &format!("update users set {lift} where id = '{}'", h.client),
        )
        .await;
        assert_eq!(titles(app, board()).await, ["Sign"], "{set}");
        assert_eq!(
            status_of(app, one(&id).cookie(&h.vera)).await,
            StatusCode::OK,
            "{set}"
        );
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_block_keeps_two_people_apart_on_the_board(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let id = id_of(&request(app, &h.tavern, json!({})).await);
    // Vera put her name forward before any block.
    assert_eq!(
        put_name(app, &h.vera, &id, Value::Null).await.0,
        StatusCode::OK
    );
    let (_, bystander) = app.user("Bystander", "US").await;
    for (by, blocked) in [(h.creative, h.client), (h.client, h.creative)] {
        exec(
            app,
            &format!("insert into user_blocks (user_id, blocked_id) values ('{by}', '{blocked}')"),
        )
        .await;
        assert!(
            titles(app, fetch("/v1/work-requests", &h.vera))
                .await
                .is_empty()
        );
        assert_eq!(
            status_of(app, one(&id).cookie(&h.vera)).await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            put_name(app, &h.vera, &id, Value::Null).await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            report(app, &h.vera, &id, "spam").await.0,
            StatusCode::NOT_FOUND
        );
        let (_, mine) = call(app, fetch("/v1/me/work-requests", &h.vera)).await;
        assert_eq!(mine["put_forward"], json!([]));
        // The block stops contact; it doesn't hide public text.
        assert_eq!(titles(app, board()).await, ["Sign"]);
        assert_eq!(
            titles(app, fetch("/v1/work-requests", &bystander)).await,
            ["Sign"]
        );
        // The maker doesn't see her name, nor count it; the row stays.
        let mine = as_maker(app, &h.tavern, &id).await;
        assert!(handles(&mine).is_empty());
        assert_eq!(mine["owner"]["responses_total"], 0);
        assert_eq!(
            count(app, "select count(*) from work_request_responses").await,
            1
        );
        exec(app, "delete from user_blocks").await;
        let mine = as_maker(app, &h.tavern, &id).await;
        assert_eq!(handles(&mine), ["vera"]);
        assert_eq!(mine["owner"]["responses_total"], 1);
        assert_eq!(
            titles(app, fetch("/v1/work-requests", &h.vera)).await,
            ["Sign"]
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Putting a name forward

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_creative_puts_their_name_forward_and_takes_it_back(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let (_, wren_cookie) = wren(app).await;
    let id = id_of(&request(app, &h.tavern, json!({})).await);
    let (status, put) = put_name(app, &h.vera, &id, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{put}");
    assert_eq!(keys(&put), ["at", "service_id"]);
    assert_eq!(put["service_id"], Value::Null);
    let at = put["at"].clone();

    // Only Vera sees that she did, on the board and by id.
    let (_, list) = call(app, fetch("/v1/work-requests", &h.vera)).await;
    assert_eq!(
        list[0]["put_forward"],
        json!({ "service_id": null, "at": at })
    );
    let (_, got) = call(app, one(&id).cookie(&h.vera)).await;
    assert_eq!(got["put_forward"]["at"], at);
    for r in [
        board(),
        fetch("/v1/work-requests", &wren_cookie),
        fetch("/v1/work-requests", &h.tavern),
    ] {
        let (_, list) = call(app, r).await;
        assert_eq!(list[0]["put_forward"], Value::Null);
    }
    let (_, mine) = call(app, fetch("/v1/me/work-requests", &h.vera)).await;
    assert_eq!(mine["put_forward"].as_array().unwrap().len(), 1);
    assert_eq!(mine["put_forward"][0]["id"], json!(id));
    assert_eq!(mine["put_forward"][0]["owner"], Value::Null);
    assert_eq!(handles(&as_maker(app, &h.tavern, &id).await), ["vera"]);

    // Taken back: 204 every time, and for a request that isn't there.
    assert_eq!(take_back(app, &h.vera, &id).await, StatusCode::NO_CONTENT);
    assert_eq!(take_back(app, &h.vera, &id).await, StatusCode::NO_CONTENT);
    assert_eq!(take_back(app, &h.vera, "x").await, StatusCode::NO_CONTENT);
    assert_eq!(
        take_back(app, &wren_cookie, &id).await,
        StatusCode::NO_CONTENT
    );
    let mine = as_maker(app, &h.tavern, &id).await;
    assert!(handles(&mine).is_empty());
    assert_eq!(mine["owner"]["responses_total"], 0);
    let (_, got) = call(app, one(&id).cookie(&h.vera)).await;
    assert_eq!(got["put_forward"], Value::Null);
    let (_, mine) = call(app, fetch("/v1/me/work-requests", &h.vera)).await;
    assert_eq!(mine["put_forward"], json!([]));
    // The row is kept, marked as taken back.
    assert_eq!(
        count(
            app,
            "select count(*) from work_request_responses where withdrawn_at is not null"
        )
        .await,
        1
    );

    // The maker read the notice. Putting it forward again keeps the first place and time and
    // tells nobody: still one notice, still one name in it.
    exec(app, "update notifications set read_at = now()").await;
    let (status, again) = put_name(app, &h.vera, &id, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["at"], at);
    let mine = as_maker(app, &h.tavern, &id).await;
    assert_eq!(handles(&mine), ["vera"]);
    assert_eq!(mine["owner"]["responses"][0]["at"], at);
    let told: Vec<(i32, bool)> = sqlx::query_as(
        "select count, read_at is null from notifications where user_id = $1 and category = 'jobs'",
    )
    .bind(h.client)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(told, [(1, false)]);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn only_a_hireable_creative_with_the_skill_can_put_their_name_forward(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let id = id_of(&request(app, &h.tavern, json!({})).await);
    let cannot =
        "your People page has to be live and taking paid work before you can put your name forward";

    // No People page at all.
    let (_, nobody) = app.user("Nobody", "US").await;
    let (status, body) = put_name(app, &nobody, &id, Value::Null).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["message"], cannot);
    // A page that isn't live yet (no portfolio approved).
    let (_, draft) = app.user("Draft Page", "US").await;
    let mut p = profile("drafty", true);
    p["confirm_adult"] = json!(true);
    let (status, _) = call(
        app,
        req(Method::PUT, "/v1/me/creative").cookie(&draft).json(p),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        put_name(app, &draft, &id, Value::Null).await.1["error"]["message"],
        cannot
    );
    // Vera not taking paid work, then with payouts not active.
    for (set, back) in [
        ("taking_work = false", "taking_work = true"),
        ("stripe_status = 'pending'", "stripe_status = 'active'"),
        ("hidden_reason = 'Spam'", "hidden_reason = null"),
    ] {
        exec(
            app,
            &format!("update creatives set {set} where handle = 'vera'"),
        )
        .await;
        let (status, body) = put_name(app, &h.vera, &id, Value::Null).await;
        assert_eq!(status, StatusCode::CONFLICT, "{set}: {body}");
        assert_eq!(body["error"]["message"], cannot, "{set}");
        exec(
            app,
            &format!("update creatives set {back} where handle = 'vera'"),
        )
        .await;
    }
    // A skill her page doesn't list.
    let (_, mill) = maker(app, "Mill Dev", "mill").await;
    let models = id_of(
        &request(
            app,
            &mill,
            json!({ "app_slug": "mill", "skill": "3d", "title": "Models" }),
        )
        .await,
    );
    let (status, body) = put_name(app, &h.vera, &models, Value::Null).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        body["error"]["message"],
        "your People page doesn't list this skill"
    );
    // The maker, on their own request (Tavern Dev with a page of their own).
    exec(
        app,
        &format!(
            "insert into creatives (user_id, handle, name, ai_use, trader, skills, stripe_status, portfolio_set_id)
             select '{}', 'tavern-dev', 'Tavern Dev', 'none', false, '{{pixel-art}}', 'active', portfolio_set_id
             from creatives where handle = 'vera'",
            h.client
        ),
    )
    .await;
    let (status, body) = put_name(app, &h.tavern, &id, Value::Null).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["message"], "this is your own request");
    // The People Terms not accepted.
    sqlx::query("delete from legal_acceptances where user_id = $1 and kind = 'people'")
        .bind(h.creative)
        .execute(&app.db)
        .await
        .unwrap();
    let (status, body) = put_name(app, &h.vera, &id, Value::Null).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        (body["error"]["code"].as_str(), field(&body)),
        (Some("terms_required"), "people")
    );
    // No free text can come with a name.
    let (status, _) = call(
        app,
        req(Method::PUT, format!("/v1/work-requests/{id}/response"))
            .cookie(&nobody)
            .json(json!({ "message": "Hire me" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        count(app, "select count(*) from work_request_responses").await,
        0
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_named_package_must_be_theirs_and_for_the_skill(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    wren(app).await;
    let id = id_of(&request(app, &h.tavern, json!({})).await);
    let sprites = first_package(app, "vera").await;
    let wrens = first_package(app, "wren").await;
    assert_eq!(sprites.len(), 8);
    // A package of hers for another skill.
    exec(
        app,
        r#"update creatives set services = services || '[{"id": "0000abcd", "name": "Theme", "description": "", "price_minor": 5000, "currency": "usd", "days": 5, "skill": "music"}]'::jsonb
           where handle = 'vera'"#,
    )
    .await;
    for wrong in [wrens.as_str(), "deadbeef", "0000abcd"] {
        let (status, body) = put_name(app, &h.vera, &id, json!(wrong)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{wrong}: {body}");
        assert_eq!(field(&body), "service_id", "{wrong}");
        assert_eq!(
            body["error"]["message"],
            "pick one of your packages for this skill"
        );
    }
    let (status, put) = put_name(app, &h.vera, &id, json!(sprites)).await;
    assert_eq!(status, StatusCode::OK, "{put}");
    assert_eq!(put["service_id"], json!(sprites));
    let stored: Option<String> =
        sqlx::query_scalar("select service_id from work_request_responses")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(stored.as_deref(), Some(sprites.as_str()));
    let mine = as_maker(app, &h.tavern, &id).await;
    assert_eq!(
        mine["owner"]["responses"][0]["service"],
        json!({ "id": sprites, "name": "Sprite sheet" })
    );
    // Again with none: cleared, and the time doesn't move.
    let (status, again) = put_name(app, &h.vera, &id, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(
        (&again["service_id"], &again["at"]),
        (&Value::Null, &put["at"])
    );
    let mine = as_maker(app, &h.tavern, &id).await;
    assert_eq!(mine["owner"]["responses"][0]["service"], Value::Null);
    assert_eq!(mine["owner"]["responses"][0]["at"], put["at"]);
    // A package she no longer offers shows as none.
    put_name(app, &h.vera, &id, json!(sprites)).await;
    exec(
        app,
        "update creatives set services = services - 0 where handle = 'vera'",
    )
    .await;
    let mine = as_maker(app, &h.tavern, &id).await;
    assert_eq!(handles(&mine), ["vera"]);
    assert_eq!(mine["owner"]["responses"][0]["service"], Value::Null);
}

// ---------------------------------------------------------------------------------------------
// The maker's side

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn only_the_maker_sees_who_put_their_name_forward(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let (_, wren_cookie) = wren(app).await;
    // Vera has finished a job and has a review; Wren has neither.
    let job = h.job("on_delivery", "completed").await;
    let (status, _) = call(
        app,
        req(Method::PUT, format!("/v1/jobs/{job}/review"))
            .cookie(&h.tavern)
            .json(json!({ "vote": "not_slop", "body": "Lovely knight." })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = id_of(&request(app, &h.tavern, json!({})).await);
    // Wren first, then Vera: the order received is the only order.
    let wrens = first_package(app, "wren").await;
    assert_eq!(
        put_name(app, &wren_cookie, &id, json!(wrens)).await.0,
        StatusCode::OK
    );
    assert_eq!(
        put_name(app, &h.vera, &id, Value::Null).await.0,
        StatusCode::OK
    );
    exec(
        app,
        "update work_request_responses r set created_at = now() - case c.handle
                when 'wren' then interval '2 hours' else interval '1 hour' end
         from creatives c where c.user_id = r.creative_id",
    )
    .await;
    let mine = as_maker(app, &h.tavern, &id).await;
    assert_eq!(handles(&mine), ["wren", "vera"]);
    assert_eq!(mine["owner"]["responses_total"], 2);
    let names = mine["owner"]["responses"].as_array().unwrap();
    // The usual card, rating line and all; the list isn't ordered by it.
    assert_eq!(names[0]["person"]["name"], "Wren Ash");
    assert_eq!(names[0]["person"]["clients"], 0);
    assert_eq!(names[1]["person"]["clients"], 1);
    assert_eq!(names[1]["person"]["can_hire"], true);
    assert_eq!(
        names[0]["service"],
        json!({ "id": wrens, "name": "Sprite sheet" })
    );
    assert_eq!(names[1]["service"], Value::Null);
    let at = |n: &Value| OffsetDateTime::parse(n["at"].as_str().unwrap(), &Rfc3339).unwrap();
    assert!(at(&names[0]) < at(&names[1]));
    // The same in the maker's own list.
    let (_, my) = call(app, fetch("/v1/me/work-requests", &h.tavern)).await;
    assert_eq!(handles(&my["posted"][0]), ["wren", "vera"]);

    // Nobody else gets the owner part: another person, a creative who put their name forward,
    // and a visitor.
    let (_, other) = app.user("Other", "US").await;
    for r in [
        one(&id).cookie(&other),
        one(&id).cookie(&h.vera),
        one(&id).cookie(&wren_cookie),
        one(&id),
    ] {
        let (status, got) = call(app, r).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(got["owner"], Value::Null);
    }
    for cookie in [&other, &h.vera] {
        let (_, list) = call(app, fetch("/v1/work-requests", cookie)).await;
        assert_eq!(list[0]["owner"], Value::Null);
    }

    // A page hidden afterwards drops out of the list; the row stays.
    exec(
        app,
        "update creatives set hidden_reason = 'Spam' where handle = 'wren'",
    )
    .await;
    let mine = as_maker(app, &h.tavern, &id).await;
    assert_eq!(handles(&mine), ["vera"]);
    assert_eq!(mine["owner"]["responses_total"], 1);
    assert_eq!(
        count(app, "select count(*) from work_request_responses").await,
        2
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_names_list_is_bounded(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let id = id_of(&request(app, &h.tavern, json!({})).await);
    // 101 live creatives who put their name forward, a minute apart.
    for sql in [
        "insert into users (id, display_name, country_code, age_16_confirmed_at)
         select md5('bulk' || g)::uuid, 'Bulk ' || g, 'US', now() from generate_series(1, 101) g",
        "insert into creatives (user_id, handle, name, ai_use, trader, skills, stripe_status)
         select md5('bulk' || g)::uuid, 'bulk-' || g, 'Bulk ' || g, 'none', false, '{pixel-art}', 'active'
         from generate_series(1, 101) g",
        "insert into portfolio_sets (id, creative_id, repo_url, commit_sha, works, fingerprint, status, source)
         select md5('set' || g)::uuid, md5('bulk' || g)::uuid, 'https://github.com/x/y', repeat('c', 40),
                '[]', 'f', 'approved', 'sim'
         from generate_series(1, 101) g",
        "update creatives set portfolio_set_id = md5('set' || substr(handle, 6))::uuid
         where handle like 'bulk-%'",
    ] {
        exec(app, sql).await;
    }
    exec(
        app,
        &format!(
            "insert into work_request_responses (request_id, creative_id, created_at)
             select '{id}', md5('bulk' || g)::uuid, now() - interval '1 day' + g * interval '1 minute'
             from generate_series(1, 101) g"
        ),
    )
    .await;
    let mine = as_maker(app, &h.tavern, &id).await;
    let names = handles(&mine);
    assert_eq!(names.len(), 100);
    assert_eq!(
        (names[0].as_str(), names[99].as_str()),
        ("bulk-1", "bulk-100")
    );
    assert_eq!(mine["owner"]["responses_total"], 101);
    let (_, my) = call(app, fetch("/v1/me/work-requests", &h.tavern)).await;
    assert_eq!(handles(&my["posted"][0]).len(), 100);
    assert_eq!(my["posted"][0]["owner"]["responses_total"], 101);
    let (status, rest) = call(
        app,
        fetch(
            format!("/v1/work-requests/{id}?responses_offset=100"),
            &h.tavern,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(handles(&rest), ["bulk-101"]);
    assert_eq!(rest["owner"]["responses_total"], 101);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_maker_gets_one_merged_notice_and_creatives_get_none(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let (wren_id, wren_cookie) = wren(app).await;
    let before = |user: Uuid| async move { notices(app, user).await.len() };
    let (vera_before, wren_before) = (before(h.creative).await, before(wren_id).await);
    let maker_before = before(h.client).await;

    let id = id_of(&request(app, &h.tavern, json!({})).await);
    let other = id_of(&request(app, &h.tavern, json!({ "title": "Menu" })).await);
    assert_eq!(notices(app, h.client).await.len(), maker_before);
    put_name(app, &h.vera, &id, Value::Null).await;
    put_name(app, &wren_cookie, &id, Value::Null).await;
    // Putting again, or changing the package, adds nothing.
    put_name(app, &h.vera, &id, json!(first_package(app, "vera").await)).await;
    let told: Vec<(String, String, String, i32, Option<String>)> = sqlx::query_as(
        "select category, title, link, count, collapse_key from notifications
         where user_id = $1 and read_at is null and category = 'jobs'",
    )
    .bind(h.client)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        told,
        [(
            "jobs".to_string(),
            "New names put forward for “Sign”".to_string(),
            format!("/people?view=requests&request={id}"),
            2,
            Some(format!("work_request:{id}")),
        )]
    );
    let body: String = sqlx::query_scalar(
        "select body from notifications where user_id = $1 and category = 'jobs'",
    )
    .bind(h.client)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(body, "", "names nobody");
    // A name on the other request is its own notice.
    put_name(app, &h.vera, &other, Value::Null).await;
    assert_eq!(
        notices(app, h.client)
            .await
            .iter()
            .filter(|t| t.starts_with("New names put forward"))
            .count(),
        2
    );
    // Taking a name back, closing and running out tell the maker nothing.
    let maker_now = notices(app, h.client).await.len();
    take_back(app, &wren_cookie, &id).await;
    close(app, &h.tavern, &other).await;
    slop_api::people::work_requests::expire(
        &app.db,
        OffsetDateTime::now_utc() + time::Duration::days(31),
    )
    .await
    .unwrap();
    assert_eq!(notices(app, h.client).await.len(), maker_now);
    // No creative was told anything about any request: posted, answered, closed or run out.
    assert_eq!(notices(app, h.creative).await.len(), vera_before);
    assert_eq!(notices(app, wren_id).await.len(), wren_before);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn closing_a_request_takes_it_off_the_board(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let id = id_of(&request(app, &h.tavern, json!({})).await);
    // Nobody else can close it, and they learn nothing.
    let (_, other) = app.user("Other", "US").await;
    for cookie in [&h.vera, &other] {
        assert_eq!(close(app, cookie, &id).await.0, StatusCode::NOT_FOUND);
    }
    assert_eq!(close(app, &h.tavern, "x").await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        close(app, &h.tavern, &Uuid::now_v7().to_string()).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(titles(app, board()).await, ["Sign"]);

    let (status, closed) = close(app, &h.tavern, &id).await;
    assert_eq!(status, StatusCode::OK, "{closed}");
    assert_eq!(closed["owner"]["status"], "closed");
    assert_eq!(closed["owner"]["on_board"], false);
    let ended = closed["owner"]["ended_at"].clone();
    assert!(ended.is_string());
    // Twice is fine and changes nothing.
    let (status, again) = close(app, &h.tavern, &id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (&again["owner"]["status"], &again["owner"]["ended_at"]),
        (&json!("closed"), &ended)
    );
    assert!(titles(app, board()).await.is_empty());
    for r in [one(&id), one(&id).cookie(&h.vera)] {
        assert_eq!(status_of(app, r).await, StatusCode::NOT_FOUND);
    }
    assert_eq!(
        put_name(app, &h.vera, &id, Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    // Its maker still has it, closed.
    assert_eq!(
        as_maker(app, &h.tavern, &id).await["owner"]["status"],
        "closed"
    );
    let (_, my) = call(app, fetch("/v1/me/work-requests", &h.tavern)).await;
    assert_eq!(my["posted"][0]["owner"]["status"], "closed");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn requests_run_out_after_30_days_and_are_deleted_later(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let id = id_of(&request(app, &h.tavern, json!({})).await);
    let hidden = id_of(&request(app, &h.tavern, json!({ "title": "Hidden" })).await);
    let late = id_of(&request(app, &h.tavern, json!({ "title": "Late" })).await);
    put_name(app, &h.vera, &id, Value::Null).await;
    exec(
        app,
        &format!(
            "update work_requests set hidden_reason = 'Spam', hidden_at = now() where id = '{hidden}'"
        ),
    )
    .await;
    // Past its 30 days before the hourly job has run: off the board, and its maker is told it
    // ran out.
    exec(
        app,
        &format!(
            "update work_requests set expires_at = now() - interval '1 minute' where id = '{late}'"
        ),
    )
    .await;
    assert_eq!(titles(app, board()).await, ["Sign"]);
    assert_eq!(status_of(app, one(&late)).await, StatusCode::NOT_FOUND);
    assert_eq!(
        as_maker(app, &h.tavern, &late).await["owner"]["status"],
        "expired"
    );
    assert_eq!(
        as_maker(app, &h.tavern, &hidden).await["owner"]["status"],
        "hidden"
    );

    let state = |id: &str| {
        let sql = format!(
            "select status || ':' || (ended_at is not null)::text from work_requests where id = '{id}'"
        );
        async move {
            sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(sql))
                .fetch_optional(&app.db)
                .await
                .unwrap()
        }
    };
    let now = OffsetDateTime::now_utc();
    // `late` ran out already, so it is counted now too; the hidden one is left for its appeal.
    let ran_out = slop_api::people::work_requests::expire(&app.db, now)
        .await
        .unwrap();
    assert_eq!(ran_out, 1);
    assert_eq!(state(&late).await.as_deref(), Some("expired:true"));
    let ran_out = slop_api::people::work_requests::expire(&app.db, now + time::Duration::days(31))
        .await
        .unwrap();
    assert_eq!(ran_out, 1);
    assert_eq!(state(&id).await.as_deref(), Some("expired:true"));
    assert_eq!(state(&hidden).await.as_deref(), Some("open:false"));
    let mine = as_maker(app, &h.tavern, &id).await;
    assert_eq!(mine["owner"]["status"], "expired");
    assert!(mine["owner"]["ended_at"].is_string());
    // Again: nothing more to do.
    assert_eq!(
        slop_api::people::work_requests::expire(&app.db, now + time::Duration::days(31))
            .await
            .unwrap(),
        0
    );
    // 90 days after it ended: gone with its names. The hidden one stays for the appeal window.
    slop_api::people::work_requests::expire(&app.db, now + time::Duration::days(31 + 91))
        .await
        .unwrap();
    assert_eq!(state(&id).await, None);
    assert_eq!(state(&late).await, None);
    assert_eq!(
        count(app, "select count(*) from work_request_responses").await,
        0
    );
    assert_eq!(state(&hidden).await.as_deref(), Some("open:false"));
    assert_eq!(
        status_of(app, one(&id).cookie(&h.tavern)).await,
        StatusCode::NOT_FOUND
    );
    slop_api::people::work_requests::expire(&app.db, now + time::Duration::days(212))
        .await
        .unwrap();
    assert_eq!(state(&hidden).await.as_deref(), Some("open:false"));
    slop_api::people::work_requests::expire(&app.db, now + time::Duration::days(214))
        .await
        .unwrap();
    assert_eq!(state(&hidden).await, None);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn your_requests_list_what_you_posted_put_forward_and_can_post_for(db: PgPool) {
    let app = people_app(db).await;
    let Hiring {
        app,
        vera,
        tavern,
        client,
        ..
    } = super::part1::hiring_on(app, "US").await;
    // Projects a request can be for: in the store, not a server; any stage; 18+ too.
    for (slug, set) in [
        ("b-draft", "status = 'draft'"),
        ("c-private", "visibility = 'private'"),
        (
            "d-server",
            "parent_app_id = (select id from apps where slug = 'tavern'), branch_kind = 'server', platforms = '{linux}'",
        ),
        ("a-grown-up", "min_age = 18, stage = 'beta'"),
    ] {
        listed_with(&app, client, slug, "released", set).await;
    }
    let first = id_of(&request(&app, &tavern, json!({})).await);
    close(&app, &tavern, &first).await;
    let second = id_of(&request(&app, &tavern, json!({ "title": "Menu" })).await);
    put_name(&app, &vera, &second, Value::Null).await;

    let (status, my) = call(&app, fetch("/v1/me/work-requests", &tavern)).await;
    assert_eq!(status, StatusCode::OK, "{my}");
    let mut k: Vec<&str> = my.as_object().unwrap().keys().map(String::as_str).collect();
    k.sort();
    assert_eq!(
        k,
        ["has_apps", "max_open", "posted", "projects", "put_forward"]
    );
    assert_eq!(my["max_open"], 3);
    assert_eq!(my["has_apps"], true);
    assert_eq!(
        my["projects"],
        json!([{ "slug": "a-grown-up", "title": "a-grown-up" }, { "slug": "tavern", "title": "tavern" }])
    );
    // Every state, each with the owner part: the open ones first, then newest first.
    let posted = my["posted"].as_array().unwrap();
    assert_eq!(
        posted.iter().map(|p| p["id"].clone()).collect::<Vec<_>>(),
        [json!(second), json!(first)]
    );
    assert_eq!(
        (&posted[0]["owner"]["status"], &posted[1]["owner"]["status"]),
        (&json!("open"), &json!("closed"))
    );
    assert_eq!(handles(&posted[0]), ["vera"]);
    assert_eq!(my["put_forward"], json!([]));
    // At most 50, the open ones first: 55 newer ended ones don't push the open one out, so the
    // page counts it toward the cap and it can be closed from there.
    exec(
        &app,
        &format!(
            "insert into work_requests (id, user_id, app_id, skill, title, description_md, status, ended_at, created_at)
             select gen_random_uuid(), '{client}', (select id from apps where slug = 'tavern'), 'music',
                    'Old ' || n, 'Done.', 'closed', now(), now() + n * interval '1 second'
             from generate_series(1, 55) n"
        ),
    )
    .await;
    let (_, my) = call(&app, fetch("/v1/me/work-requests", &tavern)).await;
    let posted = my["posted"].as_array().unwrap();
    assert_eq!(posted.len(), 50);
    assert_eq!(
        (&posted[0]["id"], &posted[0]["owner"]["status"]),
        (&json!(second), &json!("open"))
    );
    assert_eq!(
        (&posted[1]["title"], &posted[49]["title"]),
        (&json!("Old 55"), &json!("Old 7"))
    );
    assert!(posted[1..].iter().all(|p| p["owner"]["status"] == "closed"));
    exec(&app, "delete from work_requests where title like 'Old %'").await;

    // Vera: nothing posted, no apps, one request she put her name forward for.
    let (_, my) = call(&app, fetch("/v1/me/work-requests", &vera)).await;
    assert_eq!(
        (&my["posted"], &my["projects"], &my["has_apps"]),
        (&json!([]), &json!([]), &json!(false))
    );
    assert_eq!(my["put_forward"][0]["id"], json!(second));
    assert_eq!(
        my["put_forward"][0]["put_forward"]["service_id"],
        Value::Null
    );
    // A maker whose only app is a draft has apps but no project for a request.
    let (drafter, drafter_cookie) = app.user("Drafter", "US").await;
    listed_with(&app, drafter, "sketch", "released", "status = 'draft'").await;
    let (_, my) = call(&app, fetch("/v1/me/work-requests", &drafter_cookie)).await;
    assert_eq!(
        (&my["projects"], &my["has_apps"]),
        (&json!([]), &json!(true))
    );
}

// ---------------------------------------------------------------------------------------------
// Reports and moderation

/// The admin queue, as staff see it.
async fn queue(app: &TestApp, admin: &str) -> Value {
    let (status, body) = call(app, fetch("/v1/admin/queue", admin)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// Staff act on the newest open report with `action` under `rule`. Returns the decision's ID.
async fn act(app: &TestApp, admin: &str, action: &str, rule: &str) -> String {
    let id: Uuid = sqlx::query_scalar(
        "select id from reports where status = 'open' order by created_at desc limit 1",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    let (status, body) = call(
        app,
        post(
            format!("/v1/admin/reports/{id}/act"),
            admin,
            json!({
                "action": action, "rule": rule,
                "facts": "It advertises another site instead of asking for work.",
                "outcome": "We hid it."
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["decision"].as_str().unwrap().to_string()
}

/// The maker appeals a decision, and staff answer it.
async fn appeal_and_answer(app: &TestApp, maker: &str, admin: &str, decision: &str, reverse: bool) {
    let (status, body) = call(
        app,
        post(
            format!("/v1/me/decisions/{decision}/appeal"),
            maker,
            json!({ "appeal": "It's a real request for my game." }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = call(
        app,
        post(
            format!("/v1/admin/decisions/{decision}/appeal"),
            admin,
            json!({ "reverse": reverse, "reason": "We looked again." }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// A terrorist content removal order naming `subject` of `kind`.
fn removal_order(admin: &str, kind: &str, subject: &str) -> Req {
    post(
        "/v1/admin/removal-orders",
        admin,
        json!({
            "received_at": (OffsetDateTime::now_utc() - time::Duration::minutes(5)).format(&Rfc3339).unwrap(),
            "country": "FR", "authority": "OCLCTIC", "order": "Order 2026-119",
            "subject_kind": kind, "subject": subject
        }),
    )
}

/// `hidden_reason` and `hidden_at` of a request.
async fn hidden(app: &TestApp, id: &str) -> (Option<String>, Option<OffsetDateTime>) {
    sqlx::query_as("select hidden_reason, hidden_at from work_requests where id = $1::uuid")
        .bind(id)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_hide_a_request_and_can_reverse_it_on_appeal(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let (_, admin) = make_admin(app, "Ada Admin").await;
    let (_, bystander) = app.user("Bea Bystander", "US").await;
    let id = id_of(&request(app, &h.tavern, json!({})).await);
    // Only reasons that fit a request: never a fake review or copyright.
    for reason in ["fake_review", "copyright", "malware"] {
        let (status, body) = report(app, &bystander, &id, reason).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{reason}");
        assert_eq!(field(&body), "reason");
    }
    let (status, body) = report(app, &bystander, &id, "misleading").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Staff see the text as it was, where it is, and what they can do.
    let queued = queue(app, &admin).await;
    let r = &queued["reports"][0];
    assert_eq!(r["subject_kind"], "work_request");
    assert_eq!(r["subject_label"], "Open request: Sign");
    assert_eq!(r["quoted"], "Sign\n\nA tavern sign.");
    assert_eq!(r["link"], format!("/people?view=requests&request={id}"));
    assert_eq!(
        r["actions"],
        json!(["hide_work_request", "warn", "suspend_account"])
    );
    assert_eq!(r["account"]["id"], h.client.to_string());

    let decision = act(app, &admin, "hide_work_request", "misleading").await;
    // Gone for everyone but its maker, staff too; the maker sees why.
    assert!(titles(app, board()).await.is_empty());
    for r in [one(&id), one(&id).cookie(&h.vera), one(&id).cookie(&admin)] {
        assert_eq!(status_of(app, r).await, StatusCode::NOT_FOUND);
    }
    assert_eq!(
        put_name(app, &h.vera, &id, Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        report(app, &h.vera, &id, "spam").await.0,
        StatusCode::NOT_FOUND
    );
    let mine = as_maker(app, &h.tavern, &id).await;
    assert_eq!(
        (
            &mine["owner"]["status"],
            &mine["owner"]["hidden_reason"],
            &mine["owner"]["on_board"]
        ),
        (&json!("hidden"), &json!("Misleading pages"), &json!(false))
    );
    let told: Vec<String> = sqlx::query_scalar(
        "select title from notifications where user_id = $1 and category = 'moderation'",
    )
    .bind(h.client)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(told, ["Request hidden: Open request: Sign"]);
    let (action, kind, rule): (String, String, String) = sqlx::query_as(
        "select action, subject_kind, rule from moderation_decisions where id = $1::uuid",
    )
    .bind(&decision)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        (action.as_str(), kind.as_str(), rule.as_str()),
        ("hide_work_request", "work_request", "misleading")
    );
    assert_eq!(
        count(
            app,
            &format!(
                "select count(*) from audit_log where action = 'moderation.hide_work_request' and subject = '{id}'"
            )
        )
        .await,
        1
    );
    // Still open underneath, so the hourly job leaves it for the appeal.
    assert_eq!(
        slop_api::people::work_requests::expire(
            &app.db,
            OffsetDateTime::now_utc() + time::Duration::days(31)
        )
        .await
        .unwrap(),
        0
    );

    // A hidden request doesn't count toward the three.
    for title in ["Menu", "Mugs", "Map"] {
        request(app, &h.tavern, json!({ "title": title })).await;
    }
    assert_eq!(
        try_request(app, &h.tavern, json!({ "title": "Fourth" }))
            .await
            .0,
        StatusCode::CONFLICT
    );

    // The appeal: the reviewer reads what was hidden (its address answers 404 to them).
    exec(
        app,
        &format!(
            "update work_requests set expires_at = now() + interval '1 day' where id = '{id}'"
        ),
    )
    .await;
    let (status, body) = call(
        app,
        post(
            format!("/v1/me/decisions/{decision}/appeal"),
            &h.tavern,
            json!({ "appeal": "It's a real request for my game." }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let queued = queue(app, &admin).await;
    let appeal = &queued["appeals"][0];
    assert_eq!(appeal["action"], "hide_work_request");
    assert_eq!(appeal["quoted"], "Sign\n\nA tavern sign.");
    assert_eq!(
        appeal["link"],
        format!("/people?view=requests&request={id}")
    );
    let (status, body) = call(
        app,
        post(
            format!("/v1/admin/decisions/{decision}/appeal"),
            &admin,
            json!({ "reverse": true, "reason": "It asks for real work." }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Back on the board for at least a week, with the three posted since: four open, and the
    // cap only stops a new one.
    assert_eq!(hidden(app, &id).await, (None, None));
    let mine = as_maker(app, &h.tavern, &id).await;
    assert_eq!(mine["owner"]["status"], "open");
    let expires = OffsetDateTime::parse(mine["expires_at"].as_str().unwrap(), &Rfc3339).unwrap();
    assert!(
        expires > OffsetDateTime::now_utc() + time::Duration::days(7) - time::Duration::minutes(5),
        "{expires}"
    );
    assert_eq!(titles(app, board()).await.len(), 4);
    let (status, body) = try_request(app, &h.tavern, json!({ "title": "Fifth" })).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        body["error"]["message"],
        "you already have 3 open requests: close one first"
    );
}

/// A hidden request is deleted 213 days after it was hidden, but not while an appeal about it
/// still waits: reversing it must find the request.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_hidden_request_waiting_for_its_appeal_is_kept(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let (_, admin) = make_admin(app, "Ada Admin").await;
    let id = id_of(&request(app, &h.tavern, json!({})).await);
    report(app, &h.vera, &id, "spam").await;
    let decision = act(app, &admin, "hide_work_request", "spam").await;
    let (status, body) = call(
        app,
        post(
            format!("/v1/me/decisions/{decision}/appeal"),
            &h.tavern,
            json!({ "appeal": "It's a real request for my game." }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let later = OffsetDateTime::now_utc() + time::Duration::days(214);
    slop_api::people::work_requests::expire(&app.db, later)
        .await
        .unwrap();
    assert!(hidden(app, &id).await.0.is_some(), "kept for the appeal");
    let (status, body) = call(
        app,
        post(
            format!("/v1/admin/decisions/{decision}/appeal"),
            &admin,
            json!({ "reverse": false, "reason": "It advertises another site." }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    slop_api::people::work_requests::expire(&app.db, later)
        .await
        .unwrap();
    assert_eq!(
        count(app, "select count(*) from work_requests").await,
        0,
        "the appeal was answered"
    );
}

/// A removal order and a report can both hide one request: reversing one leaves it hidden under
/// the other, kept for that one's appeal.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_request_stays_hidden_while_another_decision_hides_it(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let (_, admin) = make_admin(app, "Ada Owner").await;
    let id = id_of(&request(app, &h.tavern, json!({})).await);
    report(app, &h.vera, &id, "spam").await;
    let by_report = act(app, &admin, "hide_work_request", "spam").await;
    let res = app.send(removal_order(&admin, "work_request", &id)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let by_order: (String, OffsetDateTime) = sqlx::query_as(
        "select id::text, created_at from moderation_decisions where subject_id = $1::uuid and rule = 'illegal'",
    )
    .bind(&id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(hidden(app, &id).await.0.as_deref(), Some("Illegal content"));
    let before: OffsetDateTime =
        sqlx::query_scalar("select expires_at from work_requests where id = $1::uuid")
            .bind(&id)
            .fetch_one(&app.db)
            .await
            .unwrap();

    appeal_and_answer(app, &h.tavern, &admin, &by_report, true).await;
    assert_eq!(
        hidden(app, &id).await,
        (Some("Illegal content".into()), Some(by_order.1)),
        "the order stands"
    );
    assert!(titles(app, board()).await.is_empty());
    assert_eq!(
        as_maker(app, &h.tavern, &id).await["owner"]["status"],
        "hidden"
    );
    let after: OffsetDateTime =
        sqlx::query_scalar("select expires_at from work_requests where id = $1::uuid")
            .bind(&id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(after, before, "not put back, so not run longer");

    appeal_and_answer(app, &h.tavern, &admin, &by_order.0, true).await;
    assert_eq!(hidden(app, &id).await, (None, None));
    assert_eq!(titles(app, board()).await, ["Sign"]);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_maker_cant_report_their_own_request(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let id = id_of(&request(app, &h.tavern, json!({})).await);
    let (status, body) = report(app, &h.tavern, &id, "spam").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        (field(&body), body["error"]["message"].as_str()),
        ("subject", Some("you can't report your own"))
    );
    // Off the board, it isn't there to report, for its maker as for anyone.
    close(app, &h.tavern, &id).await;
    assert_eq!(
        report(app, &h.tavern, &id, "spam").await.0,
        StatusCode::NOT_FOUND
    );
    // Not a request's ID at all.
    assert_eq!(
        report(app, &h.vera, "x", "spam").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(count(app, "select count(*) from reports").await, 0);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_removal_order_can_name_an_open_request(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let (_, admin) = make_admin(app, "Ada Owner").await;
    let id = id_of(&request(app, &h.tavern, json!({})).await);
    let (status, body) = call(app, removal_order(&admin, "bounty", &id)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body["error"]["message"],
        "app, app_review, news, profile, team or work_request"
    );
    let (status, body) = call(
        app,
        removal_order(&admin, "work_request", &Uuid::now_v7().to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(field(&body), "subject");

    let (status, body) = call(app, removal_order(&admin, "work_request", &id)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["label"], "Open request: Sign");
    assert_eq!(hidden(app, &id).await.0.as_deref(), Some("Illegal content"));
    let decision: (Uuid, String, String) =
        sqlx::query_as("select user_id, action, subject_kind from moderation_decisions")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        decision,
        (h.client, "hide_work_request".into(), "work_request".into())
    );
    let (kind, title): (String, String) = sqlx::query_as(
        "select subject_kind, snapshot->>'title' from removed_content where subject_id = $1::uuid",
    )
    .bind(&id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((kind.as_str(), title.as_str()), ("work_request", "Sign"));
    assert!(titles(app, board()).await.is_empty());
    assert_eq!(
        as_maker(app, &h.tavern, &id).await["owner"]["status"],
        "hidden"
    );
}

/// The hourly bad-link scan reads open requests on the board, under their maker's account and
/// named by their title; a closed one isn't scanned.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_link_scan_reads_open_requests(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    // Each request links to a listed address of its own, so a scan that read the closed one
    // would add a finding of its own (a finding's key is the account and the address).
    exec(
        app,
        "insert into bad_links (url, host, source) values
           ('evil.example/steal', 'evil.example', 'URLhaus'), ('evil.example/old', 'evil.example', 'URLhaus')",
    )
    .await;
    let text = "Brief and references here: https://evil.example/steal";
    request(app, &h.tavern, json!({ "description_md": text })).await;
    let closed = id_of(
        &request(
            app,
            &h.tavern,
            json!({ "title": "Old", "description_md": "References: https://evil.example/old" }),
        )
        .await,
    );
    close(app, &h.tavern, &closed).await;

    slop_api::abuse::run_once(&app.state).await.unwrap();
    let found: Vec<(String, Uuid, String, Value)> = sqlx::query_as(
        "select subject_kind, subject_id, subject_label, detail from abuse_signals where kind = 'bad_link'",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        found,
        [(
            "account".to_string(),
            h.client,
            "Tavern Dev: request “Sign”".to_string(),
            json!({ "link": "https://evil.example/steal", "listed_by": "URLhaus" })
        )]
    );
}

// ---------------------------------------------------------------------------------------------
// The account: export, closing, combining

/// GET /v1/me/export as `cookie`.
async fn export(app: &TestApp, cookie: &str) -> Value {
    let (status, body) = call(app, fetch("/v1/me/export", cookie)).await;
    assert_eq!(status, StatusCode::OK);
    body
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_export_has_requests_and_names_put_forward(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let wanted = (OffsetDateTime::now_utc().date() + time::Duration::days(10)).to_string();
    let id = id_of(&request(app, &h.tavern, json!({ "wanted_by": wanted })).await);
    let package = first_package(app, "vera").await;
    put_name(app, &h.vera, &id, json!(package)).await;

    // The maker's: their requests, every column but the two ids, with the project's address.
    // Who put their name forward is the creatives' data, not the maker's.
    let mine = export(app, &h.tavern).await;
    let posted = mine["work_requests"].as_array().unwrap();
    assert_eq!(posted.len(), 1, "{mine}");
    assert_eq!(posted[0]["id"], json!(id));
    assert_eq!(posted[0]["project"], "tavern");
    assert_eq!(posted[0]["title"], "Sign");
    assert_eq!(posted[0]["description_md"], "A tavern sign.");
    assert_eq!(posted[0]["wanted_by"], json!(wanted));
    assert_eq!(posted[0]["status"], "open");
    assert!(posted[0].get("user_id").is_none() && posted[0].get("app_id").is_none());
    assert_eq!(mine["work_request_responses"], json!([]));
    let text = mine["work_requests"].to_string();
    assert!(
        !text.contains(&h.creative.to_string()) && !text.contains("vera"),
        "{text}"
    );

    // The creative's: the names she put forward, taken back ones too.
    let theirs = export(app, &h.vera).await;
    assert_eq!(theirs["work_requests"], json!([]));
    let names = theirs["work_request_responses"].as_array().unwrap();
    assert_eq!(names.len(), 1);
    assert_eq!(
        keys(&names[0]),
        ["package", "project", "request", "since", "taken_back"]
    );
    assert_eq!(
        (
            &names[0]["request"],
            &names[0]["project"],
            &names[0]["package"]
        ),
        (&json!("Sign"), &json!("tavern"), &json!(package))
    );
    assert!(names[0]["since"].is_string());
    assert_eq!(names[0]["taken_back"], Value::Null);
    take_back(app, &h.vera, &id).await;
    assert!(export(app, &h.vera).await["work_request_responses"][0]["taken_back"].is_string());
    // Once staff hid the request, her copy no longer carries its title.
    exec(
        app,
        &format!(
            "update work_requests set hidden_reason = 'Spam', hidden_at = now() where id = '{id}'"
        ),
    )
    .await;
    let names = export(app, &h.vera).await["work_request_responses"].clone();
    assert_eq!(
        (&names[0]["request"], &names[0]["project"]),
        (&Value::Null, &json!("tavern"))
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn closing_an_account_deletes_requests_and_names(db: PgPool) {
    let h = hiring(db, "US").await;
    let app = &h.app;
    let (_, otto) = maker(app, "Otto Other", "ottos").await;
    let (_, wren_cookie) = wren(app).await;
    let open = id_of(&request(app, &h.tavern, json!({})).await);
    let hidden_one = id_of(&request(app, &h.tavern, json!({ "title": "Hidden" })).await);
    exec(
        app,
        &format!(
            "update work_requests set hidden_reason = 'Spam', hidden_at = now() where id = '{hidden_one}'"
        ),
    )
    .await;
    let ottos = id_of(
        &request(
            app,
            &otto,
            json!({ "app_slug": "ottos", "title": "Otto's" }),
        )
        .await,
    );
    for (cookie, id) in [(&h.vera, &open), (&wren_cookie, &open), (&h.vera, &ottos)] {
        assert_eq!(
            put_name(app, cookie, id, Value::Null).await.0,
            StatusCode::OK
        );
    }
    let close_account = |cookie: &str| {
        post(
            "/v1/me/close",
            cookie,
            json!({ "confirm": "Close my account" }),
        )
    };
    let left = || async {
        sqlx::query_as::<_, (String, String)>(
            "select w.title, coalesce(c.handle, '-') from work_requests w
             left join work_request_responses r on r.request_id = w.id
             left join creatives c on c.user_id = r.creative_id
             order by w.title, c.handle",
        )
        .fetch_all(&app.db)
        .await
        .unwrap()
    };

    // An open request doesn't stop the maker closing their account; it goes, hidden ones too,
    // with every name put forward for them.
    assert_eq!(
        status_of(app, close_account(&h.tavern)).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(left().await, [("Otto's".to_string(), "vera".to_string())]);
    // A creative closing hers: her names leave other people's requests, which stay.
    assert_eq!(
        status_of(app, close_account(&h.vera)).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(left().await, [("Otto's".to_string(), "-".to_string())]);
    assert_eq!(
        count(app, "select count(*) from work_request_responses").await,
        0
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn combining_accounts_moves_requests_and_drops_a_self_response(db: PgPool) {
    let app = test_app_with(db, |c| {
        c.payments.simulated = true;
        c.rate_limits.write_per_minute = 10_000;
        c.rate_limits.read_per_minute = 10_000;
        with_providers(c);
    })
    .await;
    let h = super::part1::hiring_on(app, "US").await;
    let app = &h.app;
    let (_, otto) = maker(app, "Otto Other", "ottos").await;
    let (wren_id, wren_cookie) = wren(app).await;
    // Tavern Dev's request, answered by Vera (the account it will be combined with) and Wren.
    let own = id_of(&request(app, &h.tavern, json!({})).await);
    for cookie in [&h.vera, &wren_cookie] {
        assert_eq!(
            put_name(app, cookie, &own, Value::Null).await.0,
            StatusCode::OK
        );
    }
    // Otto's two: both accounts have a name on the first (set in the database: it needs two
    // People pages), only Tavern Dev's account on the second.
    let both = id_of(&request(app, &otto, json!({ "app_slug": "ottos", "title": "Both" })).await);
    let only = id_of(&request(app, &otto, json!({ "app_slug": "ottos", "title": "Only" })).await);
    let package = first_package(app, "vera").await;
    put_name(app, &h.vera, &both, json!(package)).await;
    for id in [&both, &only] {
        exec(
            app,
            &format!(
                "insert into work_request_responses (request_id, creative_id, created_at)
                 values ('{id}', '{}', now() - interval '1 hour')",
                h.client
            ),
        )
        .await;
    }

    // Vera links Tavern Dev's sign-in and combines the two: Vera's account is the one kept.
    exec(
        app,
        &format!(
            "insert into identities (provider, provider_subject, user_id) values ('gitlab', '502', '{}')",
            h.client
        ),
    )
    .await;
    let res = oauth_flow(app, "/v1/auth/gitlab/link", &[&h.vera], "gl-bob").await;
    assert_eq!(location(&res), "/account/combine");
    let combine = set_cookie(&res, "slop_combine").unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/auth/combine")
                .cookie(&h.vera)
                .cookie(&combine),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    // The request is the combined account's now, and nobody has put their name forward for
    // their own request; Wren's name stays.
    let owner: Uuid = sqlx::query_scalar("select user_id from work_requests where id = $1::uuid")
        .bind(&own)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(owner, h.creative);
    let names = |id: String| async move {
        sqlx::query_as::<_, (Uuid, Option<String>)>(
            "select creative_id, service_id from work_request_responses where request_id = $1::uuid
             order by created_at",
        )
        .bind(id)
        .fetch_all(&app.db)
        .await
        .unwrap()
    };
    assert_eq!(names(own.clone()).await, [(wren_id, None)]);
    // One name per request: the kept account's, with its package; the other's moved.
    assert_eq!(names(both).await, [(h.creative, Some(package))]);
    assert_eq!(names(only).await, [(h.creative, None)]);
    let mine = as_maker(app, &h.vera, &own).await;
    assert_eq!(handles(&mine), ["wren"]);
}

// ---------------------------------------------------------------------------------------------
// The simulation

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_simulation_puts_requests_on_the_board(db: PgPool) {
    let app = test_app(db).await;
    let made = slop_api::sim::generate(
        &app.db,
        slop_api::sim::Settings {
            apps: 60,
            users: 60,
            days: 30,
            seed: 5,
        },
    )
    .await
    .unwrap();
    let n = made.people.work_requests as i64;
    assert!(n > 0, "{made:?}");
    assert_eq!(count(&app, "select count(*) from work_requests").await, n);
    // Each by its app's maker, open, not a server, for an app that was in the store when it was
    // posted: one is out of it now only if a later step of the fill took its app out, as it
    // would for real (the private apps, sim.rs, or a delisting, sim_moderation.rs), which picks
    // apps by their random ids. Names only from the simulation's own creatives, never the maker;
    // nobody told.
    assert_eq!(
        count(
            &app,
            "select count(*) from work_requests w join apps a on a.id = w.app_id
             where a.branch_kind = 'server' or w.user_id <> a.maker_id or w.status <> 'open'
                or w.expires_at <= now()
                or ((a.status <> 'listed' or a.visibility <> 'public')
                    and not (a.visibility = 'private' and a.status = 'draft')
                    and not exists (select 1 from moderation_decisions d
                                    where d.subject_kind = 'app' and d.subject_id = a.id
                                      and d.action = 'delist_app'))"
        )
        .await,
        0
    );
    assert!(count(&app, "select count(*) from work_request_responses").await > 0);
    assert_eq!(
        count(
            &app,
            "select count(*) from work_request_responses r join work_requests w on w.id = r.request_id
             where r.creative_id not in (select user_id from sim_creatives) or r.creative_id = w.user_id"
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from notifications where link like '/people?view=requests%'"
        )
        .await,
        0
    );
    let (status, list) = call(&app, board().header("x-country", "US")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!list.as_array().unwrap().is_empty());

    // The purge (sim.rs) has no statement for these: `delete from apps` takes the requests and
    // they take their names, and deleting the simulation's users takes theirs. (A second fill
    // can't run here: the purge only runs on a database named *_sim.)
    let cascades: Vec<(String, String)> = sqlx::query_as(
        "select conrelid::regclass::text, confrelid::regclass::text from pg_constraint
         where contype = 'f' and confdeltype = 'c'
           and conrelid in ('work_requests'::regclass, 'work_request_responses'::regclass)
         order by 1, 2",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        cascades,
        [
            ("work_request_responses".to_string(), "users".to_string()),
            ("work_request_responses".into(), "work_requests".into()),
            ("work_requests".into(), "apps".into()),
            ("work_requests".into(), "users".into()),
        ]
    );
}
