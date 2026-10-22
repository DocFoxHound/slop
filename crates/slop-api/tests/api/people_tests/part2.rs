// SPDX-License-Identifier: AGPL-3.0-or-later
//! Part 2 of the people tests.

use crate::*;

// ---------------------------------------------------------------------------------------------
// Helpers

/// A POST with a JSON body, signed in.
fn post(path: impl Into<String>, cookie: &str, body: Value) -> Req {
    req(Method::POST, path).cookie(cookie).json(body)
}

/// The field a 422 names.
async fn field_of(res: Response) -> String {
    body_json(res).await["error"]["field"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

/// A team made through the API, by `cookie`.
async fn make_team(app: &TestApp, cookie: &str, slug: &str, extra: Value) {
    let mut body = json!({ "slug": slug, "name": format!("Team {slug}"), "kind": "player" });
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    let res = app.send(post("/v1/teams", cookie, body)).await;
    assert_eq!(
        res.status(),
        StatusCode::CREATED,
        "{}",
        body_json(res).await
    );
}

/// Puts someone straight into a team with a role.
async fn add_to_team(app: &TestApp, slug: &str, user: Uuid, role: &str) {
    sqlx::query(
        "insert into team_members (team_id, user_id, role) select id, $2, $3 from teams where slug = $1",
    )
    .bind(slug)
    .bind(user)
    .bind(role)
    .execute(&app.db)
    .await
    .unwrap();
}

async fn team_id(app: &TestApp, slug: &str) -> Uuid {
    sqlx::query_scalar("select id from teams where slug = $1")
        .bind(slug)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// A user's notifications: (title, count), oldest first.
async fn notifications(app: &TestApp, user: Uuid) -> Vec<(String, i32)> {
    sqlx::query_as("select title, count from notifications where user_id = $1 order by created_at")
        .bind(user)
        .fetch_all(&app.db)
        .await
        .unwrap()
}

/// What `/v1/users/{other}/connection` says to `cookie`.
async fn connection(app: &TestApp, cookie: &str, other: Uuid) -> Value {
    let res = app
        .send(req(Method::GET, format!("/v1/users/{other}/connection")).cookie(cookie))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    body_json(res).await
}

/// A private app owned by `owner` with an approved release, so it can be shared.
async fn private_app(app: &TestApp, owner: Uuid, cookie: &str, slug: &str) {
    let mut body = new_app(slug);
    body["visibility"] = json!("private");
    let res = app.send(post("/v1/apps", cookie, body)).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    sqlx::query(
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by, reviewed_at, commit_verified)
         select $1, id, '1.0.0', repeat('e', 40), 'approved', $2, now(), true from apps where slug = $3",
    )
    .bind(Uuid::now_v7())
    .bind(owner)
    .bind(slug)
    .execute(&app.db)
    .await
    .unwrap();
}

/// A staff member with one role (and nothing else).
async fn staff_with_role(app: &TestApp, name: &str, role: &str) -> (Uuid, String) {
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

// ---------------------------------------------------------------------------------------------
// Friends and who may message whom

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_connection_shows_requests_both_ways_and_blocks(db: PgPool) {
    let app = test_app(db).await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (bob_id, bob) = app.user("Bob", "US").await;

    // Signed out: no connection to show.
    assert_eq!(
        app.send(req(Method::GET, format!("/v1/users/{bob_id}/connection")))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let none = connection(&app, &ann, bob_id).await;
    assert_eq!(
        none,
        json!({ "friend": "none", "blocked": false, "can_message": false })
    );

    // Ann asks: she sees "requested", Bob "incoming".
    app.send(post(
        "/v1/me/friends",
        &ann,
        json!({ "user": bob_id.to_string() }),
    ))
    .await;
    assert_eq!(connection(&app, &ann, bob_id).await["friend"], "requested");
    assert_eq!(connection(&app, &bob, ann_id).await["friend"], "incoming");

    // Blocking shows on the blocker's side only, and stops messages both ways.
    assert_eq!(
        app.send(req(Method::PUT, format!("/v1/me/blocks/{bob_id}")).cookie(&ann))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let c = connection(&app, &ann, bob_id).await;
    assert_eq!(
        (c["blocked"].as_bool(), c["can_message"].as_bool()),
        (Some(true), Some(false))
    );
    assert_eq!(c["friend"], "none", "blocking drops the request");
    let c = connection(&app, &bob, ann_id).await;
    assert_eq!(
        (c["blocked"].as_bool(), c["can_message"].as_bool()),
        (Some(false), Some(false)),
        "the blocked person isn't told they're blocked"
    );

    // An account that doesn't exist looks like a stranger: nothing to do, nothing revealed.
    let ghost = connection(&app, &ann, Uuid::now_v7()).await;
    assert_eq!(
        ghost,
        json!({ "friend": "none", "blocked": false, "can_message": false })
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn invite_only_teammates_and_job_partners_who_went_ahead_may_message(db: PgPool) {
    let app = test_app_with(db, |c| {
        c.payments.simulated = true;
        chat_limits(c);
    })
    .await;
    let (ann_id, ann) = app.user("Ann", "US").await;
    let (bob_id, _) = app.user("Bob", "US").await;
    let (cara_id, _) = app.user("Cara", "US").await;

    // An open team anyone can join isn't a connection; an invite-only one is.
    make_team(&app, &ann, "open-house", json!({ "join_policy": "open" })).await;
    add_to_team(&app, "open-house", bob_id, "member").await;
    assert_eq!(connection(&app, &ann, bob_id).await["can_message"], false);
    make_team(&app, &ann, "inner-circle", json!({})).await;
    add_to_team(&app, "inner-circle", bob_id, "member").await;
    assert_eq!(connection(&app, &ann, bob_id).await["can_message"], true);
    // An official team isn't one either.
    make_team(
        &app,
        &ann,
        "studio",
        json!({ "kind": "official", "github_org": "studio" }),
    )
    .await;
    add_to_team(&app, "studio", cara_id, "member").await;
    assert_eq!(connection(&app, &ann, cara_id).await["can_message"], false);

    // A People job connects its two sides only once it went ahead (accepted), not on request.
    let (vera_id, vera) = live_creative(&app, "vera", "US", false).await;
    let (tom_id, tom) = app.user("Tom Client", "US").await;
    listed_with(&app, tom_id, "toms-game", "released", "").await;
    let res = app
        .send(post(
            "/v1/people/vera/hire",
            &tom,
            json!({ "app_slug": "toms-game", "skill": "pixel-art", "request_md": "A knight.",
                    "confirm_adult": true, "exact_wording_ack": true }),
        ))
        .await;
    let job = body_json(res).await;
    let id = job["id"].as_str().unwrap().to_string();
    assert_eq!(connection(&app, &tom, vera_id).await["can_message"], false);
    let res = app
        .send(post(
            format!("/v1/jobs/{id}/offer"),
            &vera,
            sprite_offer("upfront"),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{}", body_json(res).await);
    assert_eq!(
        connection(&app, &vera, tom_id).await["can_message"],
        false,
        "an offer isn't agreed yet"
    );
    let res = app
        .send(post(
            format!("/v1/jobs/{id}/accept"),
            &tom,
            json!({ "offer_count": 1, "exact_terms_ack": true, "start_now": true }),
        ))
        .await;
    assert_eq!(body_json(res).await["status"], "awaiting_payment");
    assert_eq!(connection(&app, &vera, tom_id).await["can_message"], true);
    assert_eq!(connection(&app, &tom, vera_id).await["can_message"], true);

    // In the under-18 view, a teammate or job partner isn't enough: only friends.
    sqlx::query("update users set country_code = 'GB' where id = $1")
        .bind(tom_id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(connection(&app, &vera, tom_id).await["can_message"], false);
    assert_eq!(connection(&app, &tom, vera_id).await["can_message"], false);
    sqlx::query("update users set country_code = 'GB' where id = $1")
        .bind(bob_id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(connection(&app, &ann, bob_id).await["can_message"], false);
    befriend(&app, ann_id, bob_id).await;
    assert_eq!(
        connection(&app, &ann, bob_id).await["can_message"],
        true,
        "friends always may"
    );

    // A closed or suspended account takes no messages, even from a friend.
    befriend(&app, ann_id, cara_id).await;
    assert_eq!(connection(&app, &ann, cara_id).await["can_message"], true);
    sqlx::query("update users set suspended_at = now() where id = $1")
        .bind(cara_id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(connection(&app, &ann, cara_id).await["can_message"], false);
    sqlx::query("update users set suspended_at = null, closed_at = now() where id = $1")
        .bind(cara_id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(connection(&app, &ann, cara_id).await["can_message"], false);
}

// ---------------------------------------------------------------------------------------------
// Teams

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_team_list_shows_open_and_verified_teams_and_mine_shows_my_own(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (_, owner) = app.user("Tess Owner", "US").await;
    let (_, pal) = app.user("Pat Pal", "US").await;
    make_team(
        &app,
        &owner,
        "open-crew",
        json!({ "join_policy": "open", "name": "Open Crew" }),
    )
    .await;
    make_team(
        &app,
        &owner,
        "closed-crew",
        json!({ "name": "Closed Crew" }),
    )
    .await;
    make_team(
        &app,
        &owner,
        "hidden-crew",
        json!({ "join_policy": "open", "name": "Hidden Crew" }),
    )
    .await;
    sqlx::query("update teams set hidden_reason = 'Spam' where slug = 'hidden-crew'")
        .execute(&app.db)
        .await
        .unwrap();
    make_team(
        &app,
        &owner,
        "acme",
        json!({ "kind": "official", "github_org": "acme", "name": "Acme" }),
    )
    .await;
    make_team(
        &app,
        &owner,
        "globex",
        json!({ "kind": "official", "github_org": "globex", "name": "Globex" }),
    )
    .await;
    sqlx::query("update teams set verified_at = now() where slug = 'acme'")
        .execute(&app.db)
        .await
        .unwrap();

    // Anyone, signed out: open player teams and verified official ones.
    let listed = body_json(app.send(req(Method::GET, "/v1/teams")).await).await;
    let mut slugs: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["slug"].as_str().unwrap())
        .collect();
    slugs.sort();
    assert_eq!(slugs, ["acme", "open-crew"]);
    let acme = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["slug"] == "acme")
        .unwrap();
    assert_eq!(
        (acme["verified"].as_bool(), acme["members"].as_i64()),
        (Some(true), Some(1))
    );

    // Mine: every team I'm in, by name, hidden ones too; invites separately.
    assert_eq!(
        app.send(req(Method::GET, "/v1/me/teams")).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/teams").cookie(&owner))
            .await,
    )
    .await;
    let names: Vec<&str> = mine["teams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["Acme", "Closed Crew", "Globex", "Hidden Crew", "Open Crew"]
    );
    assert!(mine["invites"].as_array().unwrap().is_empty());
    let theirs = body_json(
        app.send(req(Method::GET, "/v1/me/teams").cookie(&pal))
            .await,
    )
    .await;
    assert!(theirs["teams"].as_array().unwrap().is_empty());
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn creating_a_team_checks_its_address_name_and_kind(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (owner_id, owner) = app.user("Tess Owner", "US").await;
    let (_, other) = app.user("Oscar Other", "US").await;
    let create = |cookie: &str, body: Value| post("/v1/teams", cookie, body);
    let team = |slug: &str| json!({ "slug": slug, "name": "Night Owls", "kind": "player" });

    assert_eq!(
        app.send(req(Method::POST, "/v1/teams").json(team("night-owls")))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    // The address is lowercased; the creator is its owner.
    let res = app.send(create(&owner, team("Night-Owls"))).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    assert_eq!(body_json(res).await["slug"], "night-owls");
    let (role, policy): (String, String) = sqlx::query_as(
        "select m.role, t.join_policy from team_members m join teams t on t.id = m.team_id
         where t.slug = 'night-owls' and m.user_id = $1",
    )
    .bind(owner_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((role.as_str(), policy.as_str()), ("owner", "invite"));
    let res = app.send(create(&other, team("night-owls"))).await;
    assert_eq!(res.status(), StatusCode::CONFLICT, "taken");

    let long = "a".repeat(41);
    for bad in [
        "ab",
        "night owls",
        "night--owls",
        "-owls",
        "owls-",
        "nïght",
        long.as_str(),
    ] {
        let res = app.send(create(&owner, team(bad))).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
        assert_eq!(field_of(res).await, "slug", "{bad}");
    }
    for (change, field) in [
        (json!({ "name": "" }), "name"),
        (json!({ "name": "   " }), "name"),
        (json!({ "name": "x".repeat(61) }), "name"),
        (json!({ "name": "Bell\u{7}" }), "name"),
        (json!({ "about": "x".repeat(2001) }), "about"),
        (json!({ "about": "tab\tin it" }), "about"),
        (json!({ "join_policy": "secret" }), "join_policy"),
        (json!({ "kind": "studio" }), "kind"),
        (json!({ "kind": "official" }), "github_org"),
        (
            json!({ "kind": "official", "github_org": "acme/evil" }),
            "github_org",
        ),
        (
            json!({ "kind": "official", "github_org": "a".repeat(40) }),
            "github_org",
        ),
    ] {
        let mut body = team("good-name");
        for (k, v) in change.as_object().unwrap() {
            body[k] = v.clone();
        }
        let res = app.send(create(&owner, body.clone())).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(field_of(res).await, field, "{body}");
    }
    // Sixty characters and a multi-line about are fine.
    let mut body = team("good-name");
    body["name"] = json!("x".repeat(60));
    body["about"] = json!("Line one.\nLine two.");
    assert_eq!(
        app.send(create(&owner, body)).await.status(),
        StatusCode::CREATED
    );
    let teams: i64 = sqlx::query_scalar("select count(*) from teams")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(teams, 2, "nothing made by the refused requests");

    // An organization can be named again until one of the teams naming it is verified.
    let official = |slug: &str| json!({ "slug": slug, "name": "Acme", "kind": "official", "github_org": "acme" });
    assert_eq!(
        app.send(create(&owner, official("acme"))).await.status(),
        StatusCode::CREATED
    );
    assert_eq!(
        app.send(create(&other, official("acme-two")))
            .await
            .status(),
        StatusCode::CREATED
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn team_pages_hide_hidden_teams_and_show_strangers_only_adult_runners(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (_, owner) = app.user("Tess Owner", "US").await;
    let (teen_id, _) = app.user("Tia Teen", "GB").await;
    let (pal_id, pal) = app.user("Pat Pal", "US").await;
    let (admin_id, _) = app.user("Ada Admin", "US").await;
    let (_, stranger) = app.user("Sam Stranger", "US").await;
    make_team(
        &app,
        &owner,
        "open-crew",
        json!({ "join_policy": "open", "name": "Open Crew" }),
    )
    .await;
    add_to_team(&app, "open-crew", teen_id, "admin").await;
    add_to_team(&app, "open-crew", admin_id, "admin").await;
    add_to_team(&app, "open-crew", pal_id, "member").await;

    // An open team: anyone sees it, and who runs it (never an under-18 admin, never members).
    for r in [
        req(Method::GET, "/v1/teams/open-crew"),
        req(Method::GET, "/v1/teams/open-crew").cookie(&stranger),
    ] {
        let res = app.send(r).await;
        assert_eq!(res.status(), StatusCode::OK);
        let page = body_json(res).await;
        let names: Vec<&str> = page["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["Tess Owner", "Ada Admin"]);
        assert_eq!(page["member_count"], 4);
        assert!(page["role"].is_null());
        assert!(page["picks"].as_array().unwrap().is_empty());
    }
    // Members see everyone.
    let page = body_json(
        app.send(req(Method::GET, "/v1/teams/open-crew").cookie(&pal))
            .await,
    )
    .await;
    assert_eq!(page["members"].as_array().unwrap().len(), 4);
    assert_eq!(page["role"], "member");

    // Hidden by moderation: gone for everyone but its members.
    sqlx::query(
        "update teams set hidden_reason = 'Hate speech in the name' where slug = 'open-crew'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    for r in [
        req(Method::GET, "/v1/teams/open-crew"),
        req(Method::GET, "/v1/teams/open-crew").cookie(&stranger),
    ] {
        assert_eq!(app.send(r).await.status(), StatusCode::NOT_FOUND);
    }
    let page = body_json(
        app.send(req(Method::GET, "/v1/teams/open-crew").cookie(&pal))
            .await,
    )
    .await;
    assert_eq!(page["hidden"], true);
    assert_eq!(
        app.send(req(Method::POST, "/v1/teams/open-crew/join").cookie(&stranger))
            .await
            .status(),
        StatusCode::NOT_FOUND,
        "a hidden team can't be joined"
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/teams/no-such-team"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn only_a_teams_owner_and_admins_change_its_settings(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (owner_id, owner) = app.user("Tess Owner", "US").await;
    let (pal_id, pal) = app.user("Pat Pal", "US").await;
    let (admin_id, admin) = app.user("Ada Admin", "US").await;
    let (_, stranger) = app.user("Sam Stranger", "US").await;
    make_team(&app, &owner, "night-owls", json!({})).await;
    add_to_team(&app, "night-owls", pal_id, "member").await;
    add_to_team(&app, "night-owls", admin_id, "admin").await;
    let patch = |cookie: &str, body: Value| {
        req(Method::PATCH, "/v1/teams/night-owls")
            .cookie(cookie)
            .json(body)
    };

    assert_eq!(
        app.send(patch(&pal, json!({ "name": "Pal's Owls" })))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(patch(&stranger, json!({ "name": "Mine now" })))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    for (body, field) in [
        (json!({ "name": "" }), "name"),
        (json!({ "name": "x".repeat(61) }), "name"),
        (json!({ "name": "New\u{1b}line" }), "name"),
        (json!({ "about": "x".repeat(2001) }), "about"),
        (json!({ "join_policy": "public" }), "join_policy"),
        (json!({ "dns_domain": "owls.example" }), "dns_domain"),
    ] {
        let res = app.send(patch(&owner, body.clone())).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(field_of(res).await, field, "{body}");
    }
    let res = app
        .send(patch(
            &admin,
            json!({ "name": " Night Owls Club ", "about": "We play late.", "join_policy": "open" }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let (name, about, policy): (String, String, String) =
        sqlx::query_as("select name, about, join_policy from teams where slug = 'night-owls'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        (name.as_str(), about.as_str(), policy.as_str()),
        ("Night Owls Club", "We play late.", "open")
    );

    // Private Slop shared with it keeps it invite-only.
    app.send(patch(&owner, json!({ "join_policy": "invite" })))
        .await;
    private_app(&app, owner_id, &owner, "owl-tool").await;
    let res = app
        .send(post(
            "/v1/apps/owl-tool/shares",
            &owner,
            json!({ "team": "night-owls" }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let res = app
        .send(patch(&owner, json!({ "join_policy": "open" })))
        .await;
    assert_eq!(field_of(res).await, "join_policy");

    // Official teams: a domain to prove, checked; empty clears it.
    make_team(
        &app,
        &owner,
        "acme",
        json!({ "kind": "official", "github_org": "acme" }),
    )
    .await;
    let patch_acme = |body: Value| {
        req(Method::PATCH, "/v1/teams/acme")
            .cookie(&owner)
            .json(body)
    };
    let too_long = format!("{}.example", "a".repeat(250));
    for bad in ["localhost", "acme_games.example", too_long.as_str()] {
        let res = app.send(patch_acme(json!({ "dns_domain": bad }))).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }
    let res = app.send(patch_acme(json!({ "join_policy": "open" }))).await;
    assert_eq!(
        field_of(res).await,
        "join_policy",
        "official teams are by invitation"
    );
    app.send(patch_acme(json!({ "dns_domain": "Acme.Example." })))
        .await;
    let domain: Option<String> =
        sqlx::query_scalar("select dns_domain from teams where slug = 'acme'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(domain.as_deref(), Some("acme.example"));
    assert_eq!(
        app.send(patch_acme(json!({ "dns_domain": "" })))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let domain: Option<String> =
        sqlx::query_scalar("select dns_domain from teams where slug = 'acme'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(domain, None);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn anyone_joins_an_open_player_team_once(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (_, owner) = app.user("Tess Owner", "US").await;
    let (pal_id, pal) = app.user("Pat Pal", "US").await;
    make_team(&app, &owner, "open-crew", json!({ "join_policy": "open" })).await;
    make_team(
        &app,
        &owner,
        "acme",
        json!({ "kind": "official", "github_org": "acme" }),
    )
    .await;
    sqlx::query("update teams set verified_at = now() where slug = 'acme'")
        .execute(&app.db)
        .await
        .unwrap();

    assert_eq!(
        app.send(req(Method::POST, "/v1/teams/open-crew/join"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    for _ in 0..2 {
        assert_eq!(
            app.send(req(Method::POST, "/v1/teams/open-crew/join").cookie(&pal))
                .await
                .status(),
            StatusCode::NO_CONTENT
        );
    }
    let rows: Vec<String> = sqlx::query_scalar(
        "select m.role from team_members m join teams t on t.id = m.team_id where t.slug = 'open-crew' and m.user_id = $1",
    )
    .bind(pal_id)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(rows, ["member"], "joined once, as a member");
    let page = body_json(
        app.send(req(Method::GET, "/v1/teams/open-crew").cookie(&pal))
            .await,
    )
    .await;
    assert_eq!(page["role"], "member");

    // A studio's team is never joined by asking.
    assert_eq!(
        app.send(req(Method::POST, "/v1/teams/acme/join").cookie(&pal))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(req(Method::POST, "/v1/teams/nobody/join").cookie(&pal))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn team_invites_go_only_to_people_the_inviter_may_reach(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (owner_id, owner) = app.user("Tess Owner", "US").await;
    let (pal_id, pal) = app.user("Pat Pal", "US").await;
    let (member_id, member) = app.user("Mo Member", "US").await;
    let (blocker_id, blocker) = app.user("Bea Blocker", "US").await;
    let (blocked_id, _) = app.user("Bo Blocked", "US").await;
    let (teen_id, _) = app.user("Tia Teen", "GB").await;
    let (teen_friend_id, _) = app.user("Tom Teen", "GB").await;
    let (closed_id, _) = app.user("Cleo Closed", "US").await;
    let (suspended_id, _) = app.user("Sid Suspended", "US").await;
    make_team(&app, &owner, "night-owls", json!({ "name": "Night Owls" })).await;
    add_to_team(&app, "night-owls", member_id, "member").await;
    let invite = |cookie: &str, user: Uuid| {
        post(
            "/v1/teams/night-owls/invites",
            cookie,
            json!({ "user": format!("http://localhost:8080/u/{user}") }),
        )
    };

    // Blocked either way, under 18 and not a friend, closed, suspended, already in, or no such
    // account: all the same answer, so none of them is revealed.
    app.send(req(Method::PUT, format!("/v1/me/blocks/{owner_id}")).cookie(&blocker))
        .await;
    app.send(req(Method::PUT, format!("/v1/me/blocks/{blocked_id}")).cookie(&owner))
        .await;
    sqlx::query("update users set closed_at = now() where id = $1")
        .bind(closed_id)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("update users set suspended_at = now() where id = $1")
        .bind(suspended_id)
        .execute(&app.db)
        .await
        .unwrap();
    let mut messages = std::collections::HashSet::new();
    for who in [
        blocker_id,
        blocked_id,
        teen_id,
        closed_id,
        suspended_id,
        member_id,
        Uuid::now_v7(),
    ] {
        let res = app.send(invite(&owner, who)).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{who}");
        let err = body_json(res).await;
        assert_eq!(err["error"]["field"], "user");
        messages.insert(err["error"]["message"].as_str().unwrap().to_string());
    }
    assert_eq!(messages.len(), 1, "one answer for all: {messages:?}");
    let res = app
        .send(post(
            "/v1/teams/night-owls/invites",
            &owner,
            json!({ "user": "not a link" }),
        ))
        .await;
    assert_eq!(field_of(res).await, "user");
    let invites: i64 = sqlx::query_scalar("select count(*) from team_invites")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(invites, 0);

    // A friend in the under-18 view may be invited.
    befriend(&app, owner_id, teen_friend_id).await;
    assert_eq!(
        app.send(invite(&owner, teen_friend_id)).await.status(),
        StatusCode::NO_CONTENT
    );
    // Plain members don't invite; strangers don't see the team.
    assert_eq!(
        app.send(invite(&member, pal_id)).await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(invite(&blocker, pal_id)).await.status(),
        StatusCode::NOT_FOUND
    );

    // Asked again after saying no: still one notification.
    assert_eq!(
        app.send(invite(&owner, pal_id)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(invite(&owner, pal_id)).await.status(),
        StatusCode::NO_CONTENT,
        "twice is fine"
    );
    assert_eq!(
        notifications(&app, pal_id).await,
        [("Tess Owner invited you to Night Owls".to_string(), 1)]
    );
    app.send(req(Method::POST, "/v1/teams/night-owls/invites/decline").cookie(&pal))
        .await;
    app.send(invite(&owner, pal_id)).await;
    assert_eq!(
        notifications(&app, pal_id).await,
        [("Tess Owner invited you to Night Owls".to_string(), 2)]
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn team_invites_are_rate_limited_per_account(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (_, owner) = app.user("Tess Owner", "US").await;
    make_team(&app, &owner, "night-owls", json!({})).await;
    let mut ids = Vec::new();
    for i in 0..51 {
        let id = Uuid::now_v7();
        sqlx::query(
            "insert into users (id, display_name, country_code, age_16_confirmed_at) values ($1, $2, 'US', now())",
        )
        .bind(id)
        .bind(format!("Player {i}"))
        .execute(&app.db)
        .await
        .unwrap();
        ids.push(id);
    }
    let invite = |id: Uuid| {
        post(
            "/v1/teams/night-owls/invites",
            &owner,
            json!({ "user": id.to_string() }),
        )
    };
    for id in &ids[..50] {
        assert_eq!(app.send(invite(*id)).await.status(), StatusCode::NO_CONTENT);
    }
    assert_eq!(
        app.send(invite(ids[50])).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    let invites: i64 = sqlx::query_scalar("select count(*) from team_invites")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(invites, 50);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn invitees_accept_or_decline_and_only_with_an_invite(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (_, owner) = app.user("Tess Owner", "US").await;
    let (pal_id, pal) = app.user("Pat Pal", "US").await;
    let (_, stranger) = app.user("Sam Stranger", "US").await;
    make_team(&app, &owner, "night-owls", json!({})).await;
    make_team(&app, &owner, "open-crew", json!({ "join_policy": "open" })).await;
    let invite = || {
        post(
            "/v1/teams/night-owls/invites",
            &owner,
            json!({ "user": pal_id.to_string() }),
        )
    };
    let answer = |cookie: &str, team: &str, what: &str| {
        req(Method::POST, format!("/v1/teams/{team}/invites/{what}")).cookie(cookie)
    };

    // No invite: nothing to accept (even where the team is visible).
    for team in ["open-crew", "night-owls"] {
        assert_eq!(
            app.send(answer(&stranger, team, "accept")).await.status(),
            StatusCode::NOT_FOUND,
            "{team}"
        );
    }
    let in_teams: i64 = sqlx::query_scalar("select count(*) from team_members")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(in_teams, 2, "only the two owners");

    // Declining removes the invite, hides the team again, and can be repeated.
    app.send(invite()).await;
    assert_eq!(
        app.send(req(Method::GET, "/v1/teams/night-owls").cookie(&pal))
            .await
            .status(),
        StatusCode::OK
    );
    for _ in 0..2 {
        assert_eq!(
            app.send(answer(&pal, "night-owls", "decline"))
                .await
                .status(),
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(
        app.send(req(Method::GET, "/v1/teams/night-owls").cookie(&pal))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/teams").cookie(&pal))
            .await,
    )
    .await;
    assert!(mine["invites"].as_array().unwrap().is_empty());
    assert_eq!(
        app.send(answer(&pal, "night-owls", "accept"))
            .await
            .status(),
        StatusCode::NOT_FOUND,
        "a declined invite can't be accepted"
    );
    assert_eq!(
        app.send(answer(&pal, "no-such-team", "decline"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // Accepting makes them a member.
    app.send(invite()).await;
    assert_eq!(
        app.send(answer(&pal, "night-owls", "accept"))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let mine = body_json(
        app.send(req(Method::GET, "/v1/me/teams").cookie(&pal))
            .await,
    )
    .await;
    assert_eq!(mine["teams"][0]["slug"], "night-owls");
    assert!(mine["invites"].as_array().unwrap().is_empty());
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn removing_team_members_follows_roles_and_takes_away_what_the_team_shares(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (owner_id, owner) = app.user("Tess Owner", "US").await;
    let (co_owner_id, _) = app.user("Cody Co-owner", "US").await;
    let (admin_id, admin) = app.user("Ada Admin", "US").await;
    let (admin2_id, _) = app.user("Abe Admin", "US").await;
    let (pal_id, pal) = app.user("Pat Pal", "US").await;
    let (mo_id, mo) = app.user("Mo Member", "US").await;
    let (_, stranger) = app.user("Sam Stranger", "US").await;
    make_team(&app, &owner, "night-owls", json!({})).await;
    add_to_team(&app, "night-owls", co_owner_id, "owner").await;
    add_to_team(&app, "night-owls", admin_id, "admin").await;
    add_to_team(&app, "night-owls", admin2_id, "admin").await;
    add_to_team(&app, "night-owls", pal_id, "member").await;
    add_to_team(&app, "night-owls", mo_id, "member").await;
    private_app(&app, owner_id, &owner, "owl-tool").await;
    app.send(post(
        "/v1/apps/owl-tool/shares",
        &owner,
        json!({ "team": "night-owls" }),
    ))
    .await;
    let chat = Uuid::now_v7();
    sqlx::query(
        "insert into chat_conversations (id, kind, team_id, created_by) values ($1, 'team', $2, $3)",
    )
    .bind(chat)
    .bind(team_id(&app, "night-owls").await)
    .bind(owner_id)
    .execute(&app.db)
    .await
    .unwrap();
    let get = |cookie: &str, path: String| req(Method::GET, path).cookie(cookie);
    for c in [&pal, &mo] {
        assert_eq!(
            app.send(get(c, "/v1/apps/owl-tool".into())).await.status(),
            StatusCode::OK
        );
        assert_eq!(
            app.send(get(c, format!("/v1/chat/conversations/{chat}")))
                .await
                .status(),
            StatusCode::OK
        );
    }
    let remove = |cookie: &str, who: Uuid| {
        req(
            Method::DELETE,
            format!("/v1/teams/night-owls/members/{who}"),
        )
        .cookie(cookie)
    };

    assert_eq!(
        app.send(remove(&pal, mo_id)).await.status(),
        StatusCode::CONFLICT,
        "members can't"
    );
    assert_eq!(
        app.send(remove(&stranger, mo_id)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(remove(&admin, admin2_id)).await.status(),
        StatusCode::CONFLICT,
        "an admin can't remove an admin"
    );
    assert_eq!(
        app.send(remove(&admin, owner_id)).await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(remove(&owner, co_owner_id)).await.status(),
        StatusCode::CONFLICT,
        "nobody removes an owner"
    );
    assert_eq!(
        app.send(remove(&owner, Uuid::now_v7())).await.status(),
        StatusCode::NOT_FOUND
    );

    // An admin removes a member; an owner removes an admin.
    assert_eq!(
        app.send(remove(&admin, pal_id)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(remove(&owner, admin2_id)).await.status(),
        StatusCode::NO_CONTENT
    );
    let left: Vec<Uuid> = sqlx::query_scalar("select user_id from team_members where team_id = $1")
        .bind(team_id(&app, "night-owls").await)
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert!(!left.contains(&pal_id) && !left.contains(&admin2_id));
    assert_eq!(left.len(), 4);
    // The removed member loses the team's Private Slop and its chat; the others keep them.
    for path in [
        "/v1/apps/owl-tool".to_string(),
        format!("/v1/chat/conversations/{chat}"),
        "/v1/teams/night-owls".to_string(),
    ] {
        assert_eq!(
            app.send(get(&pal, path.clone())).await.status(),
            StatusCode::NOT_FOUND,
            "{path}"
        );
        assert_eq!(
            app.send(get(&mo, path.clone())).await.status(),
            StatusCode::OK,
            "{path}"
        );
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn only_an_owner_changes_roles_and_never_their_own(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (owner_id, owner) = app.user("Tess Owner", "US").await;
    let (admin_id, admin) = app.user("Ada Admin", "US").await;
    let (pal_id, pal) = app.user("Pat Pal", "US").await;
    let (stranger_id, stranger) = app.user("Sam Stranger", "US").await;
    make_team(&app, &owner, "night-owls", json!({})).await;
    add_to_team(&app, "night-owls", admin_id, "admin").await;
    add_to_team(&app, "night-owls", pal_id, "member").await;
    let set = |cookie: &str, who: Uuid, role: &str| {
        req(Method::PUT, format!("/v1/teams/night-owls/members/{who}"))
            .cookie(cookie)
            .json(json!({ "role": role }))
    };
    let role_of = |who: Uuid| {
        let db = app.db.clone();
        async move {
            sqlx::query_scalar::<_, String>(
                "select m.role from team_members m join teams t on t.id = m.team_id
                 where t.slug = 'night-owls' and m.user_id = $1",
            )
            .bind(who)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };

    assert_eq!(
        app.send(set(&admin, pal_id, "admin")).await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(set(&pal, pal_id, "owner")).await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(set(&stranger, pal_id, "admin")).await.status(),
        StatusCode::NOT_FOUND
    );
    let res = app.send(set(&owner, pal_id, "captain")).await;
    assert_eq!(field_of(res).await, "role");
    let res = app.send(set(&owner, owner_id, "member")).await;
    assert_eq!(
        res.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "not your own role"
    );
    assert_eq!(
        app.send(set(&owner, stranger_id, "admin")).await.status(),
        StatusCode::NOT_FOUND,
        "not a member"
    );
    assert_eq!(role_of(pal_id).await, "member");

    assert_eq!(
        app.send(set(&owner, pal_id, "admin")).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(role_of(pal_id).await, "admin");
    assert_eq!(
        app.send(set(&owner, admin_id, "member")).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(role_of(admin_id).await, "member");
    assert_eq!(role_of(owner_id).await, "owner");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn team_picks_are_store_apps_from_members_of_player_teams(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (owner_id, owner) = app.user("Tess Owner", "US").await;
    let (pal_id, pal) = app.user("Pat Pal", "US").await;
    let (mo_id, mo) = app.user("Mo Member", "US").await;
    let (_, stranger) = app.user("Sam Stranger", "US").await;
    make_team(&app, &owner, "night-owls", json!({})).await;
    add_to_team(&app, "night-owls", pal_id, "member").await;
    add_to_team(&app, "night-owls", mo_id, "member").await;
    make_team(
        &app,
        &owner,
        "acme",
        json!({ "kind": "official", "github_org": "acme" }),
    )
    .await;
    listed_with(&app, owner_id, "co-op-caves", "released", "").await;
    listed_with(&app, owner_id, "dark-woods", "released", "").await;
    let res = app
        .send(post("/v1/apps", &stranger, new_app("drafty")))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    private_app(&app, owner_id, &owner, "owl-tool").await;
    let pick = |cookie: &str, slug: &str, note: Option<String>| {
        let mut body = json!({ "app_slug": slug });
        if let Some(n) = note {
            body["note"] = json!(n);
        }
        post("/v1/teams/night-owls/picks", cookie, body)
    };

    assert_eq!(
        app.send(post(
            "/v1/teams/acme/picks",
            &owner,
            json!({ "app_slug": "co-op-caves" })
        ))
        .await
        .status(),
        StatusCode::CONFLICT,
        "official teams have no picks"
    );
    assert_eq!(
        app.send(pick(&stranger, "co-op-caves", None))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    for slug in ["no-such-app", "drafty"] {
        assert_eq!(
            app.send(pick(&pal, slug, None)).await.status(),
            StatusCode::NOT_FOUND,
            "{slug}"
        );
    }
    let res = app.send(pick(&owner, "owl-tool", None)).await;
    assert_eq!(field_of(res).await, "app_slug", "Private Slop isn't picked");
    let res = app
        .send(pick(&pal, "co-op-caves", Some("x".repeat(281))))
        .await;
    assert_eq!(field_of(res).await, "note");
    assert_eq!(
        app.send(pick(&pal, "co-op-caves", Some("Great with 4".into())))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(pick(&mo, "dark-woods", None)).await.status(),
        StatusCode::NO_CONTENT
    );
    // Picking it again updates the note (one pick per app).
    assert_eq!(
        app.send(pick(&pal, "co-op-caves", Some("Best with 3".into())))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let page = body_json(
        app.send(req(Method::GET, "/v1/teams/night-owls").cookie(&owner))
            .await,
    )
    .await;
    let picks = page["picks"].as_array().unwrap();
    assert_eq!(picks.len(), 2);
    let caves = picks.iter().find(|p| p["slug"] == "co-op-caves").unwrap();
    assert_eq!(
        (caves["note"].as_str(), caves["by"].as_str()),
        (Some("Best with 3"), Some("Pat Pal"))
    );
    let caves_id = caves["id"].as_str().unwrap().to_string();
    let woods_id = picks.iter().find(|p| p["slug"] == "dark-woods").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Removing: the member who added it, or an admin; nobody else.
    let remove = |cookie: &str, id: &str| {
        req(Method::DELETE, format!("/v1/teams/night-owls/picks/{id}")).cookie(cookie)
    };
    assert_eq!(
        app.send(remove(&mo, &caves_id)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(remove(&stranger, &caves_id)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(remove(&pal, &Uuid::now_v7().to_string()))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(remove(&pal, &caves_id)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(remove(&owner, &woods_id)).await.status(),
        StatusCode::NO_CONTENT
    );
    let left: i64 = sqlx::query_scalar("select count(*) from team_picks")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn game_nights_are_in_the_coming_year_and_removed_by_their_creator_or_an_admin(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (owner_id, owner) = app.user("Tess Owner", "US").await;
    let (pal_id, pal) = app.user("Pat Pal", "US").await;
    let (mo_id, mo) = app.user("Mo Member", "US").await;
    let (_, stranger) = app.user("Sam Stranger", "US").await;
    make_team(&app, &owner, "night-owls", json!({ "name": "Night Owls" })).await;
    add_to_team(&app, "night-owls", pal_id, "member").await;
    add_to_team(&app, "night-owls", mo_id, "member").await;
    make_team(
        &app,
        &owner,
        "acme",
        json!({ "kind": "official", "github_org": "acme" }),
    )
    .await;
    listed_with(&app, owner_id, "co-op-caves", "released", "").await;
    private_app(&app, owner_id, &owner, "owl-tool").await;
    let at = |d: time::Duration| (OffsetDateTime::now_utc() + d).format(&Rfc3339).unwrap();
    let night = |cookie: &str, team: &str, change: Value| {
        let mut body = json!({ "title": "Cave night", "starts_at": at(time::Duration::days(2)) });
        for (k, v) in change.as_object().unwrap() {
            body[k] = v.clone();
        }
        post(format!("/v1/teams/{team}/events"), cookie, body)
    };

    assert_eq!(
        app.send(night(&owner, "acme", json!({}))).await.status(),
        StatusCode::CONFLICT,
        "official teams have no game nights"
    );
    assert_eq!(
        app.send(night(&stranger, "night-owls", json!({})))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    for (change, field) in [
        (
            json!({ "starts_at": at(-time::Duration::hours(2)) }),
            "starts_at",
        ),
        (
            json!({ "starts_at": at(time::Duration::days(366)) }),
            "starts_at",
        ),
        (json!({ "title": "" }), "title"),
        (json!({ "title": "x".repeat(101) }), "title"),
    ] {
        let res = app.send(night(&pal, "night-owls", change.clone())).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{change}");
        assert_eq!(field_of(res).await, field, "{change}");
    }
    // Private Slop is never named, even by its owner (to others it doesn't exist).
    let private = json!({ "app_slug": "owl-tool" });
    let res = app.send(night(&owner, "night-owls", private.clone())).await;
    assert_eq!(field_of(res).await, "app_slug");
    assert_eq!(
        app.send(night(&pal, "night-owls", private)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(night(
            &pal,
            "night-owls",
            json!({ "app_slug": "no-such-app" })
        ))
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let events: i64 = sqlx::query_scalar("select count(*) from team_events")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(events, 0);
    assert!(
        notifications(&app, mo_id).await.is_empty(),
        "nobody is told about refused ones"
    );

    // Just started (within the hour) is fine, and an empty app means none.
    let res = app
        .send(night(
            &pal,
            "night-owls",
            json!({ "starts_at": at(-time::Duration::minutes(30)), "app_slug": " " }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let res = app
        .send(night(
            &mo,
            "night-owls",
            json!({ "title": "Mo's night", "app_slug": "co-op-caves" }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let page = body_json(
        app.send(req(Method::GET, "/v1/teams/night-owls").cookie(&pal))
            .await,
    )
    .await;
    let events = page["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert!(events[0]["app_slug"].is_null());
    assert_eq!(
        (events[0]["mine"].as_bool(), events[1]["mine"].as_bool()),
        (Some(true), Some(false))
    );
    assert_eq!(events[1]["app_slug"], "co-op-caves");
    let pal_night = events[0]["id"].as_str().unwrap().to_string();
    let mo_night = events[1]["id"].as_str().unwrap().to_string();

    let remove = |cookie: &str, id: &str| {
        req(Method::DELETE, format!("/v1/teams/night-owls/events/{id}")).cookie(cookie)
    };
    assert_eq!(
        app.send(remove(&mo, &pal_night)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(remove(&stranger, &pal_night)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(remove(&pal, &Uuid::now_v7().to_string()))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(remove(&pal, &pal_night)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(remove(&owner, &mo_night)).await.status(),
        StatusCode::NO_CONTENT,
        "an admin may"
    );
    let events: i64 = sqlx::query_scalar("select count(*) from team_events")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(events, 0);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn stopping_a_team_share_takes_private_slop_away_from_its_members(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (owner_id, owner) = app.user("Olive Owner", "US").await;
    let (pal_id, pal) = app.user("Pat Pal", "US").await;
    make_team(&app, &owner, "night-owls", json!({})).await;
    add_to_team(&app, "night-owls", pal_id, "member").await;
    private_app(&app, owner_id, &owner, "owl-tool").await;
    app.send(post(
        "/v1/apps/owl-tool/shares",
        &owner,
        json!({ "team": "night-owls" }),
    ))
    .await;
    let pal_sees = || async {
        app.send(req(Method::GET, "/v1/apps/owl-tool").cookie(&pal))
            .await
            .status()
    };
    assert_eq!(pal_sees().await, StatusCode::OK);
    let unshare = |cookie: &str, team: &str| {
        req(
            Method::DELETE,
            format!("/v1/apps/owl-tool/team-shares/{team}"),
        )
        .cookie(cookie)
    };

    // Only the owner: not a member of the team, not anyone signed out.
    assert_eq!(
        app.send(unshare(&pal, "night-owls")).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(req(
            Method::DELETE,
            "/v1/apps/owl-tool/team-shares/night-owls"
        ))
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    // An unknown team, or one it isn't shared with, is nothing to undo.
    assert_eq!(
        app.send(unshare(&owner, "no-such-team")).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(pal_sees().await, StatusCode::OK);

    assert_eq!(
        app.send(unshare(&owner, "night-owls")).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(pal_sees().await, StatusCode::NOT_FOUND);
    let shares = body_json(
        app.send(req(Method::GET, "/v1/apps/owl-tool/shares").cookie(&owner))
            .await,
    )
    .await;
    assert!(shares.as_array().unwrap().is_empty());
    let page = body_json(
        app.send(req(Method::GET, "/v1/teams/night-owls").cookie(&pal))
            .await,
    )
    .await;
    assert!(page["shared"].as_array().unwrap().is_empty());
    // Now it can be opened to anyone.
    assert_eq!(
        app.send(
            req(Method::PATCH, "/v1/teams/night-owls")
                .cookie(&owner)
                .json(json!({ "join_policy": "open" }))
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );

    // A public app isn't shared this way at all.
    listed_with(&app, owner_id, "co-op-caves", "released", "").await;
    assert_eq!(
        app.send(
            req(
                Method::DELETE,
                "/v1/apps/co-op-caves/team-shares/night-owls"
            )
            .cookie(&owner)
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
}

// ---------------------------------------------------------------------------------------------
// Staff fixes to teams and profiles

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_fixing_a_team_need_moderation_rights_and_a_reason(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (owner_id, owner) = app.user("Tess Owner", "US").await;
    let (admin_id, _) = app.user("Ada Admin", "US").await;
    let (pal_id, _) = app.user("Pat Pal", "US").await;
    let (_, reviewer) = staff_with_role(&app, "Rex Reviewer", "reviewer").await;
    let (_, support) = staff_with_role(&app, "Sue Support", "support").await;
    make_team(
        &app,
        &owner,
        "valve-official",
        json!({ "name": "Official Valve Team" }),
    )
    .await;
    add_to_team(&app, "valve-official", admin_id, "admin").await;
    add_to_team(&app, "valve-official", pal_id, "member").await;
    let fix = |cookie: &str, slug: &str, body: Value| {
        req(Method::PATCH, format!("/v1/admin/teams/{slug}"))
            .cookie(cookie)
            .json(body)
    };
    let rename = json!({ "name": "Tess's Team", "reason": "Not affiliated with Valve." });

    assert_eq!(
        app.send(fix(&owner, "valve-official", rename.clone()))
            .await
            .status(),
        StatusCode::NOT_FOUND,
        "not staff: no such route"
    );
    assert_eq!(
        app.send(fix(&reviewer, "valve-official", rename.clone()))
            .await
            .status(),
        StatusCode::FORBIDDEN,
        "reviewers review; they don't moderate"
    );
    for (body, field) in [
        (json!({ "name": "Tess's Team", "reason": "" }), "reason"),
        (json!({ "name": "Tess's Team", "reason": "  " }), "reason"),
        (
            json!({ "name": "Tess's Team", "reason": "x".repeat(2001) }),
            "reason",
        ),
        (json!({ "name": "Two\nlines", "reason": "Fix" }), "name"),
        (json!({ "name": "", "reason": "Fix" }), "name"),
        (
            json!({ "name": "Official Valve Team", "reason": "Fix" }),
            "reason",
        ),
        (json!({ "reason": "Fix" }), "reason"),
    ] {
        let res = app
            .send(fix(&support, "valve-official", body.clone()))
            .await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(field_of(res).await, field, "{body}");
    }
    let res = app
        .send(fix(&support, "valve-official", json!({ "name": "x" })))
        .await;
    assert_eq!(
        res.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "the reason is required"
    );
    assert_eq!(
        app.send(fix(&support, "no-such-team", rename.clone()))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // Support may: the runners are told, and the audit log keeps the old values.
    let res = app
        .send(fix(&support, "valve-official", rename.clone()))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["changed"], json!(["name"]));
    let name: String = sqlx::query_scalar("select name from teams where slug = 'valve-official'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(name, "Tess's Team");
    for (who, told) in [(owner_id, true), (admin_id, true), (pal_id, false)] {
        let titles: Vec<String> = notifications(&app, who)
            .await
            .into_iter()
            .map(|n| n.0)
            .collect();
        assert_eq!(
            titles.contains(&"Slop changed your team's page".to_string()),
            told,
            "{titles:?}"
        );
    }
    let (actor, subject, details): (String, String, Value) =
        sqlx::query_as("select actor, subject, detail from audit_log where action = 'team.edited'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(actor.contains("Sue Support"), "{actor}");
    assert_eq!(subject, "team valve-official");
    assert_eq!(
        details["changed"]["name"],
        json!({ "from": "Official Valve Team", "to": "Tess's Team" })
    );
    assert_eq!(details["reason"], "Not affiliated with Valve.");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn staff_fixing_a_profile_need_moderation_rights_a_reason_and_valid_text(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (mia_id, _) = app.user("Mia Maker", "US").await;
    let (plain_id, _) = app.user("Pat Plain", "US").await;
    let (_, reviewer) = staff_with_role(&app, "Rex Reviewer", "reviewer").await;
    let (_, support) = staff_with_role(&app, "Sue Support", "support").await;
    sqlx::query(
        "insert into creatives (user_id, handle, name, headline, ai_use, trader, location)
         values ($1, 'mia', 'Mia', 'Totally the real Studio Ghibli', 'none', false, 'Tokyo')",
    )
    .bind(mia_id)
    .execute(&app.db)
    .await
    .unwrap();
    let url = format!("/v1/admin/users/{mia_id}/profile");
    let fix = |cookie: &str, body: Value| req(Method::PATCH, url.clone()).cookie(cookie).json(body);

    assert_eq!(
        app.send(req(Method::GET, url.clone()).cookie(&reviewer))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(fix(
            &reviewer,
            json!({ "display_name": "Mia", "reason": "Fix" })
        ))
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(
            req(
                Method::GET,
                format!("/v1/admin/users/{}/profile", Uuid::now_v7())
            )
            .cookie(&support)
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let seen = body_json(
        app.send(req(Method::GET, url.clone()).cookie(&support))
            .await,
    )
    .await;
    assert_eq!(
        seen["creative"]["headline"],
        "Totally the real Studio Ghibli"
    );
    let plain = body_json(
        app.send(req(Method::GET, format!("/v1/admin/users/{plain_id}/profile")).cookie(&support))
            .await,
    )
    .await;
    assert!(plain["creative"].is_null());

    for (body, field) in [
        (json!({ "display_name": "Mia", "reason": "" }), "reason"),
        (
            json!({ "display_name": "", "reason": "Fix" }),
            "display_name",
        ),
        (
            json!({ "display_name": "x".repeat(41), "reason": "Fix" }),
            "display_name",
        ),
        (
            json!({ "creative": { "name": "" }, "reason": "Fix" }),
            "name",
        ),
        (
            json!({ "creative": { "name": "x".repeat(61) }, "reason": "Fix" }),
            "name",
        ),
        (
            json!({ "creative": { "headline": "Two\nlines" }, "reason": "Fix" }),
            "headline",
        ),
        (
            json!({ "creative": { "headline": "x".repeat(141) }, "reason": "Fix" }),
            "headline",
        ),
        (
            json!({ "creative": { "location": "x".repeat(61) }, "reason": "Fix" }),
            "location",
        ),
        (
            json!({ "creative": { "location": "A\tB" }, "reason": "Fix" }),
            "location",
        ),
        (
            json!({ "creative": { "about_md": "x".repeat(6001) }, "reason": "Fix" }),
            "about_md",
        ),
        (
            json!({ "creative": { "headline": "Totally the real Studio Ghibli" }, "reason": "Fix" }),
            "reason",
        ),
        (
            json!({ "creative": { "remove_links": true }, "reason": "Fix" }),
            "reason",
        ),
    ] {
        let res = app.send(fix(&support, body.clone())).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(field_of(res).await, field, "{body}");
    }
    let res = app
        .send(
            req(Method::PATCH, format!("/v1/admin/users/{plain_id}/profile"))
                .cookie(&support)
                .json(json!({ "creative": { "headline": "x" }, "reason": "Fix" })),
        )
        .await;
    assert_eq!(field_of(res).await, "creative", "no People page to fix");
    let unchanged: (String, String) = sqlx::query_as(
        "select u.display_name, c.headline from users u join creatives c on c.user_id = u.id where u.id = $1",
    )
    .bind(mia_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        unchanged,
        (
            "Mia Maker".to_string(),
            "Totally the real Studio Ghibli".to_string()
        )
    );

    // Support may fix it: told why, and audited with the old values.
    let res = app
        .send(fix(
            &support,
            json!({ "display_name": "Mia M", "creative": { "name": "Mia M", "location": "", "about_md": "Pixel art." },
                    "reason": "Impersonation." }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        body_json(res).await["changed"],
        json!(["display name", "People name", "about", "location"])
    );
    let (title, body): (String, String) =
        sqlx::query_as("select title, body from notifications where user_id = $1")
            .bind(mia_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(title, "Slop changed your profile");
    assert!(body.contains("Impersonation."), "{body}");
    let details: Value = sqlx::query_scalar(
        "select detail from audit_log where action = 'user.profile_edited' and subject = $1",
    )
    .bind(mia_id.to_string())
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        details["changed"]["display_name"],
        json!({ "from": "Mia Maker", "to": "Mia M" })
    );
    assert_eq!(
        details["changed"]["location"],
        json!({ "from": "Tokyo", "to": "" })
    );
}

// ---------------------------------------------------------------------------------------------
// Official team verification, against a GitHub of our own

/// What the stand-in GitHub says about one organization. Organizations not in the map don't have
/// Slop's App installed.
#[derive(Clone, Default)]
struct Org {
    verified_domain: bool,
    two_factor: bool,
    /// Logins that are active owners (admins) of the organization.
    owners: Vec<&'static str>,
    /// GitHub answers 500 about it.
    broken: bool,
}

type Orgs = std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, Org>>>;

/// A GitHub with Slop's App installed on the organizations in `orgs`. GitHub account 42 is
/// "octo", 43 "bob".
async fn fake_orgs(orgs: Orgs) -> String {
    use axum::extract::{Path, State};
    let app = Router::new()
        .route(
            "/orgs/{org}/installation",
            get(|State(orgs): State<Orgs>, Path(org): Path<String>| async move {
                if orgs.lock().unwrap().contains_key(&org) {
                    axum::Json(json!({ "id": 77 })).into_response()
                } else {
                    StatusCode::NOT_FOUND.into_response()
                }
            }),
        )
        .route(
            "/app/installations/{id}/access_tokens",
            axum::routing::post(|| async {
                (
                    StatusCode::CREATED,
                    axum::Json(json!({ "token": "ghs_org", "expires_at": "2099-01-01T00:00:00Z" })),
                )
            }),
        )
        .route(
            "/user/{id}",
            get(|Path(id): Path<i64>| async move {
                let login = match id {
                    42 => "octo",
                    43 => "bob",
                    _ => "someone",
                };
                axum::Json(json!({ "id": id, "login": login }))
            }),
        )
        .route(
            "/orgs/{org}/memberships/{login}",
            get(
                |State(orgs): State<Orgs>, Path((org, login)): Path<(String, String)>| async move {
                    let owner = orgs
                        .lock()
                        .unwrap()
                        .get(&org)
                        .is_some_and(|o| o.owners.contains(&login.as_str()));
                    if owner {
                        axum::Json(json!({ "role": "admin", "state": "active" })).into_response()
                    } else {
                        StatusCode::NOT_FOUND.into_response()
                    }
                },
            ),
        )
        .route(
            "/orgs/{org}",
            get(|State(orgs): State<Orgs>, Path(org): Path<String>| async move {
                let o = orgs.lock().unwrap().get(&org).cloned().unwrap_or_default();
                if o.broken {
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
                axum::Json(json!({
                    "login": org, "is_verified": o.verified_domain,
                    "two_factor_requirement_enabled": o.two_factor,
                }))
                .into_response()
            }),
        )
        .route(
            "/dns-query",
            get(|| async { axum::Json(json!({ "Answer": [] })) }),
        )
        .with_state(orgs);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/")
}

/// An app whose GitHub is `fake_orgs`, with Slop's GitHub App set up.
async fn official_app(db: PgPool, orgs: Orgs) -> TestApp {
    let base = fake_orgs(orgs).await;
    test_app_with(db, |c| {
        chat_limits(c);
        c.releases.github_api_base = base.parse().unwrap();
        c.doh_url = format!("{base}dns-query").parse().unwrap();
        c.releases.github_app = Some(slop_api::config::GithubAppConfig {
            app_id: "123".into(),
            slug: "slop-test".into(),
            private_key: jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!(
                "../../fixtures/github-app-test-key.pem"
            ))
            .unwrap(),
            webhook_secret: "hook-secret".into(),
        });
    })
    .await
}

/// Links a user to a GitHub account.
async fn github_account(app: &TestApp, user: Uuid, github_id: &str) {
    sqlx::query(
        "insert into identities (provider, provider_subject, user_id) values ('github', $1, $2)",
    )
    .bind(github_id)
    .bind(user)
    .execute(&app.db)
    .await
    .unwrap();
}

/// A check's result from a verify response: (ok, detail).
fn check(res: &Value, name: &str) -> (bool, String) {
    let c = res["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["check"] == name)
        .unwrap_or_else(|| panic!("no {name} check in {res}"));
    (
        c["ok"].as_bool().unwrap(),
        c["detail"].as_str().unwrap().to_string(),
    )
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn official_teams_need_an_organization_owner_two_factor_and_a_verified_domain(db: PgPool) {
    let orgs = Orgs::default();
    orgs.lock().unwrap().insert(
        "acme".into(),
        Org {
            verified_domain: true,
            two_factor: true,
            owners: vec!["octo"],
            broken: false,
        },
    );
    orgs.lock().unwrap().insert(
        "globex".into(),
        Org {
            verified_domain: true,
            two_factor: true,
            owners: vec!["octo"],
            broken: false,
        },
    );
    let app = official_app(db, orgs.clone()).await;
    let (ada_id, ada) = app.user("Ada Acme", "US").await;
    let (bob_id, bob) = app.user("Bob Globex", "US").await;
    let (_, carl) = app.user("Carl NoGitHub", "US").await;
    let (_, stranger) = app.user("Sam Stranger", "US").await;
    github_account(&app, ada_id, "42").await;
    github_account(&app, bob_id, "43").await;
    let verify = |cookie: &str, slug: &str| {
        req(Method::POST, format!("/v1/teams/{slug}/verify")).cookie(cookie)
    };

    // Player teams have nothing to verify.
    make_team(&app, &ada, "night-owls", json!({})).await;
    assert_eq!(
        app.send(verify(&ada, "night-owls")).await.status(),
        StatusCode::CONFLICT
    );

    // Only the team's owners and admins ask.
    make_team(
        &app,
        &ada,
        "acme",
        json!({ "kind": "official", "github_org": "acme", "name": "Acme" }),
    )
    .await;
    add_to_team(&app, "acme", bob_id, "member").await;
    assert_eq!(
        app.send(verify(&bob, "acme")).await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(verify(&stranger, "acme")).await.status(),
        StatusCode::NOT_FOUND
    );

    // GitHub's verified-domain badge is enough for the domain; no DNS record needed.
    let res = body_json(app.send(verify(&ada, "acme")).await).await;
    assert_eq!(res["verified"], true, "{res}");
    assert_eq!(
        check(&res, "domain"),
        (
            true,
            "GitHub shows the organization's domain as verified.".into()
        )
    );
    assert_eq!(check(&res, "owner").1, "@octo is an owner of acme.");
    let listed = body_json(app.send(req(Method::GET, "/v1/teams")).await).await;
    assert_eq!(listed[0]["slug"], "acme");

    // Two-factor switched off on GitHub: the badge goes, and its owners are told.
    orgs.lock().unwrap().get_mut("acme").unwrap().two_factor = false;
    let res = body_json(app.send(verify(&ada, "acme")).await).await;
    assert_eq!(res["verified"], false, "{res}");
    assert!(!check(&res, "two_factor").0);
    assert!(check(&res, "owner").0 && check(&res, "domain").0);
    let titles: Vec<String> = notifications(&app, ada_id)
        .await
        .into_iter()
        .map(|n| n.0)
        .collect();
    assert_eq!(titles, ["Your team's official badge was removed"]);
    assert!(
        notifications(&app, bob_id).await.is_empty(),
        "members aren't owners"
    );
    let listed = body_json(app.send(req(Method::GET, "/v1/teams")).await).await;
    assert!(listed.as_array().unwrap().is_empty());
    // Asking again while still failing doesn't tell them twice.
    app.send(verify(&ada, "acme")).await;
    assert_eq!(notifications(&app, ada_id).await.len(), 1);

    // Someone who isn't an owner of the organization on GitHub can't claim it.
    make_team(
        &app,
        &bob,
        "globex",
        json!({ "kind": "official", "github_org": "globex" }),
    )
    .await;
    let res = body_json(app.send(verify(&bob, "globex")).await).await;
    assert_eq!(res["verified"], false);
    assert_eq!(
        check(&res, "owner"),
        (false, "@bob isn't an owner of globex on GitHub.".into())
    );

    // An owner without a GitHub sign-in can't be checked at all.
    sqlx::query("update teams set github_org = 'acme' where slug = 'globex'")
        .execute(&app.db)
        .await
        .unwrap();
    make_team(
        &app,
        &carl,
        "carls-acme",
        json!({ "kind": "official", "github_org": "acme" }),
    )
    .await;
    let res = body_json(app.send(verify(&carl, "carls-acme")).await).await;
    assert_eq!(res["verified"], false);
    let (ok, detail) = check(&res, "owner");
    assert!(!ok);
    assert!(detail.contains("couldn't find their GitHub account"), "{detail}");

    // An organization without Slop's App: nothing can be checked until it's installed.
    make_team(
        &app,
        &carl,
        "initech",
        json!({ "kind": "official", "github_org": "initech" }),
    )
    .await;
    let res = body_json(app.send(verify(&carl, "initech")).await).await;
    assert_eq!(res["verified"], false);
    assert_eq!(res["checks"].as_array().unwrap().len(), 1);
    let (ok, detail) = check(&res, "app_installed");
    assert!(!ok && detail.contains("Install Slop's GitHub App on the initech organization"));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_second_team_for_an_already_verified_organization_is_refused_not_an_error(db: PgPool) {
    let orgs = Orgs::default();
    orgs.lock().unwrap().insert(
        "acme".into(),
        Org {
            verified_domain: true,
            two_factor: true,
            owners: vec!["octo", "bob"],
            broken: false,
        },
    );
    let app = official_app(db, orgs).await;
    let (ada_id, ada) = app.user("Ada Acme", "US").await;
    let (bob_id, bob) = app.user("Bob Acme", "US").await;
    github_account(&app, ada_id, "42").await;
    github_account(&app, bob_id, "43").await;
    make_team(
        &app,
        &ada,
        "acme",
        json!({ "kind": "official", "github_org": "acme" }),
    )
    .await;
    let res = body_json(
        app.send(req(Method::POST, "/v1/teams/acme/verify").cookie(&ada))
            .await,
    )
    .await;
    assert_eq!(res["verified"], true, "{res}");

    // Another owner of the organization makes a second team for it: every check passes for
    // them too, but an organization has one verified team.
    make_team(
        &app,
        &bob,
        "acme-games",
        json!({ "kind": "official", "github_org": "acme" }),
    )
    .await;
    let res = app
        .send(req(Method::POST, "/v1/teams/acme-games/verify").cookie(&bob))
        .await;
    let status = res.status();
    let body = body_json(res).await;
    assert_ne!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a clear answer, not a server error: {body}"
    );
    assert_eq!(body["verified"], false, "{body}");
    let (ok, detail) = check(&body, "organization");
    assert!(!ok && detail.contains("already the verified team for acme"), "{detail}");
    let verified: Vec<String> =
        sqlx::query_scalar("select slug from teams where verified_at is not null")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(verified, ["acme"]);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_daily_recheck_drops_badges_that_stopped_passing_and_skips_fresh_ones(db: PgPool) {
    let orgs = Orgs::default();
    let passing = Org {
        verified_domain: true,
        two_factor: true,
        owners: vec!["octo"],
        broken: false,
    };
    orgs.lock()
        .unwrap()
        .insert("acme".into(), passing.clone());
    orgs.lock()
        .unwrap()
        .insert("fresh".into(), passing.clone());
    orgs.lock().unwrap().insert(
        "broken".into(),
        Org {
            broken: true,
            ..passing.clone()
        },
    );
    let app = official_app(db, orgs.clone()).await;
    let (ada_id, ada) = app.user("Ada Acme", "US").await;
    github_account(&app, ada_id, "42").await;
    for org in ["acme", "fresh", "broken"] {
        make_team(
            &app,
            &ada,
            org,
            json!({ "kind": "official", "github_org": org }),
        )
        .await;
    }
    sqlx::query(
        "update teams set verified_at = now() - interval '30 days',
                checked_at = case slug when 'fresh' then now() - interval '1 hour' else now() - interval '2 days' end",
    )
    .execute(&app.db)
    .await
    .unwrap();
    let checked_before: Vec<(String, OffsetDateTime)> =
        sqlx::query_as("select slug, checked_at from teams order by slug")
            .fetch_all(&app.db)
            .await
            .unwrap();
    // Since the last check, acme turned off two-factor, and so did fresh (not due yet).
    orgs.lock().unwrap().get_mut("acme").unwrap().two_factor = false;
    orgs.lock().unwrap().get_mut("fresh").unwrap().two_factor = false;

    let job = tokio::spawn(slop_api::teams::run(app.state.clone()));
    let mut done = false;
    for _ in 0..100 {
        let checked: bool = sqlx::query_scalar(
            "select checked_at > now() - interval '1 minute' from teams where slug = 'acme'",
        )
        .fetch_one(&app.db)
        .await
        .unwrap();
        if checked {
            done = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    // Let the round finish (broken may come after acme).
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    job.abort();
    assert!(done, "acme was rechecked");
    assert!(!job.await.unwrap_err().is_panic(), "an error is logged, not fatal");

    let after: Vec<(String, OffsetDateTime, bool)> = sqlx::query_as(
        "select slug, checked_at, verified_at is not null from teams order by slug",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    let get = |slug: &str| after.iter().find(|t| t.0 == slug).unwrap().clone();
    let before = |slug: &str| checked_before.iter().find(|t| t.0 == slug).unwrap().1;
    assert!(!get("acme").2, "the badge went");
    assert!(get("fresh").2, "checked within a day: left alone");
    assert_eq!(get("fresh").1, before("fresh"));
    assert!(get("broken").2, "GitHub's error isn't a failed check");
    assert_eq!(get("broken").1, before("broken"));
    let titles: Vec<String> = notifications(&app, ada_id)
        .await
        .into_iter()
        .map(|n| n.0)
        .collect();
    assert_eq!(titles, ["Your team's official badge was removed"]);
}
