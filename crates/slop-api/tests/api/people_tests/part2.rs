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
            get(
                |State(orgs): State<Orgs>, Path(org): Path<String>| async move {
                    if orgs.lock().unwrap().contains_key(&org) {
                        axum::Json(json!({ "id": 77 })).into_response()
                    } else {
                        StatusCode::NOT_FOUND.into_response()
                    }
                },
            ),
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
            get(
                |State(orgs): State<Orgs>, Path(org): Path<String>| async move {
                    let o = orgs.lock().unwrap().get(&org).cloned().unwrap_or_default();
                    if o.broken {
                        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                    }
                    axum::Json(json!({
                        "login": org, "is_verified": o.verified_domain,
                        "two_factor_requirement_enabled": o.two_factor,
                    }))
                    .into_response()
                },
            ),
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
    assert!(
        detail.contains("couldn't find their GitHub account"),
        "{detail}"
    );

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
    assert!(
        !ok && detail.contains("already the verified team for acme"),
        "{detail}"
    );
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
    orgs.lock().unwrap().insert("acme".into(), passing.clone());
    orgs.lock().unwrap().insert("fresh".into(), passing.clone());
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
    assert!(
        !job.await.unwrap_err().is_panic(),
        "an error is logged, not fatal"
    );

    let after: Vec<(String, OffsetDateTime, bool)> =
        sqlx::query_as("select slug, checked_at, verified_at is not null from teams order by slug")
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

// ---------------------------------------------------------------------------------------------
// Chat's delivery service, driven like the app would without the encryption: raw devices

fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// A fresh Ed25519 signing key.
fn signing_key() -> ed25519_dalek::SigningKey {
    use sha2::Digest;
    let seed = sha2::Sha256::digest(Uuid::now_v7().as_bytes());
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&seed);
    ed25519_dalek::SigningKey::from_bytes(&bytes)
}

/// A desktop sign-in's `Authorization` header.
async fn app_bearer(app: &TestApp, web: &str) -> String {
    let tokens = desktop_sign_in(app, web).await;
    format!("Bearer {}", tokens["access_token"].as_str().unwrap())
}

/// One of a user's chat devices, registered through the API from its own desktop sign-in.
struct Dev {
    id: Uuid,
    bearer: String,
    key: ed25519_dalek::SigningKey,
}

impl Dev {
    /// A request as this device.
    fn call(&self, method: Method, path: impl Into<String>) -> Req {
        req(method, path)
            .header("authorization", self.bearer.clone())
            .header("x-slop-chat-device", self.id.to_string())
    }
    /// A request from this device's sign-in, without naming the device.
    fn signed_in(&self, method: Method, path: impl Into<String>) -> Req {
        req(method, path).header("authorization", self.bearer.clone())
    }
    fn public(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }
}

/// Registers a new chat device for a user, with `packages` key packages. Its status is the
/// returned device's second value (active for a first device, pending after).
async fn chat_device(app: &TestApp, web: &str, packages: usize) -> (Dev, String) {
    let bearer = app_bearer(app, web).await;
    let key = signing_key();
    let id = Uuid::now_v7();
    let kps: Vec<String> = (0..packages)
        .map(|i| b64(format!("kp-{id}-{i}").as_bytes()))
        .collect();
    let res = app
        .send(
            req(Method::POST, "/v1/chat/devices")
                .header("authorization", bearer.clone())
                .json(json!({
                    "id": id, "name": "PC", "signature_key": b64(&key.verifying_key().to_bytes()),
                    "key_packages": kps, "last_resort": b64(format!("lr-{id}").as_bytes()),
                })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let status = body_json(res).await["status"].as_str().unwrap().to_string();
    (Dev { id, bearer, key }, status)
}

/// An active first device.
async fn first_device(app: &TestApp, web: &str) -> Dev {
    let (dev, status) = chat_device(app, web, 5).await;
    assert_eq!(status, "active");
    dev
}

/// Starts a chat from a device's sign-in; returns (status, conversation).
async fn start_chat(app: &TestApp, dev: &Dev, body: Value) -> (StatusCode, Value) {
    let res = app
        .send(
            dev.signed_in(Method::POST, "/v1/chat/conversations")
                .json(body),
        )
        .await;
    let status = res.status();
    (status, body_json(res).await)
}

async fn chat_id(app: &TestApp, dev: &Dev, body: Value) -> Uuid {
    let (status, c) = start_chat(app, dev, body).await;
    assert!(status.is_success(), "{status} {c}");
    c["id"].as_str().unwrap().parse().unwrap()
}

/// An encrypted message's body as the app sends it (here: opaque bytes).
fn message(epoch: i64, text: &str) -> Value {
    json!({ "epoch": epoch, "body": b64(text.as_bytes()), "franking_tag": b64(&[0; 32]) })
}

async fn send_message(app: &TestApp, dev: &Dev, conv: Uuid, text: &str) -> Response {
    app.send(
        dev.call(
            Method::POST,
            format!("/v1/chat/conversations/{conv}/messages"),
        )
        .json(message(0, text)),
    )
    .await
}

/// A request with a raw body (attachments and backups).
async fn send_bytes(app: &TestApp, r: Req, body: Vec<u8>) -> Response {
    let mut builder = Request::builder()
        .method(r.method)
        .uri(r.uri)
        .header(header::HOST, "localhost:8080");
    if !r.cookies.is_empty() {
        builder = builder.header(header::COOKIE, r.cookies.join("; "));
    }
    for (name, value) in r.headers {
        builder = builder.header(name, value);
    }
    app.router
        .clone()
        .oneshot(builder.body(Body::from(body)).unwrap())
        .await
        .unwrap()
}

async fn body_bytes(res: Response) -> Vec<u8> {
    res.into_body().collect().await.unwrap().to_bytes().to_vec()
}

/// A device's sync, at once.
async fn sync_now(app: &TestApp, dev: &Dev) -> Value {
    let res = app
        .send(dev.call(Method::GET, "/v1/chat/sync?after=0"))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    body_json(res).await
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn chat_devices_are_registered_with_a_name_a_key_and_a_few_packages(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (_, ann_web) = app.user("Ann", "US").await;
    let bearer = app_bearer(&app, &ann_web).await;
    let register = |body: Value| {
        req(Method::POST, "/v1/chat/devices")
            .header("authorization", bearer.clone())
            .json(body)
    };
    let key = signing_key().verifying_key().to_bytes();
    let device = |id: Uuid, key: &[u8], name: &str| {
        json!({ "id": id, "name": name, "signature_key": b64(key),
                "key_packages": [b64(b"kp1"), b64(b"kp2")], "last_resort": b64(b"lr") })
    };

    assert_eq!(
        app.send(req(Method::POST, "/v1/chat/devices").json(device(Uuid::now_v7(), &key, "PC")))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let with = |change: Value| {
        let mut body = device(Uuid::now_v7(), &key, "PC");
        for (k, v) in change.as_object().unwrap() {
            body[k] = v.clone();
        }
        body
    };
    for (change, field) in [
        (json!({ "name": "" }), "name"),
        (json!({ "name": "   " }), "name"),
        (json!({ "name": "x".repeat(61) }), "name"),
        (json!({ "signature_key": "not base64!" }), "signature_key"),
        (json!({ "signature_key": b64(&[1; 31]) }), "signature_key"),
        (json!({ "signature_key": b64(&[1; 33]) }), "signature_key"),
        (
            json!({ "key_packages": vec![b64(b"kp"); 21] }),
            "key_packages",
        ),
        (
            json!({ "key_packages": [b64(&vec![1; 4097])] }),
            "key_packages",
        ),
        (json!({ "key_packages": ["%%%"] }), "key_packages"),
        (json!({ "last_resort": b64(&vec![1; 4097]) }), "last_resort"),
    ] {
        let res = app.send(register(with(change.clone()))).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{change}");
        assert_eq!(field_of(res).await, field, "{change}");
    }
    let stored: (i64, i64) = sqlx::query_as(
        "select (select count(*) from chat_devices), (select count(*) from chat_key_packages)",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(stored, (0, 0), "nothing kept from refused registrations");

    // The first: active at once, with the code from its key; 60 characters of name is fine.
    let first = Uuid::now_v7();
    let res = app
        .send(register(device(first, &key, &"x".repeat(60))))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let created = body_json(res).await;
    assert_eq!(created["status"], "active");
    assert_eq!(created["code"], slop_api::chat::device_code(&key));
    let packages: Vec<bool> = sqlx::query_scalar(
        "select last_resort from chat_key_packages where device_id = $1 order by last_resort",
    )
    .bind(first)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(packages, [false, false, true]);
    // The same device, or the same key under another ID, isn't registered twice.
    let other_key = signing_key().verifying_key().to_bytes();
    assert_eq!(
        app.send(register(device(first, &other_key, "PC")))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(register(device(Uuid::now_v7(), &key, "PC")))
            .await
            .status(),
        StatusCode::CONFLICT
    );

    // Later ones wait for approval; five at most wait at once.
    let mut pending = Vec::new();
    for _ in 0..5 {
        let (dev, status) = chat_device(&app, &ann_web, 1).await;
        assert_eq!(status, "pending");
        pending.push(dev);
    }
    let res = app
        .send(register(device(
            Uuid::now_v7(),
            &signing_key().verifying_key().to_bytes(),
            "One too many",
        )))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);

    // The list: every device but removed ones, each with its code.
    let list = |b: String| {
        let app = &app;
        async move {
            body_json(
                app.send(req(Method::GET, "/v1/chat/devices").header("authorization", b))
                    .await,
            )
            .await
        }
    };
    let devices = list(bearer.clone()).await;
    assert_eq!(devices.as_array().unwrap().len(), 6);
    assert_eq!(devices[0]["id"], first.to_string());
    assert_eq!(devices[0]["status"], "active");
    assert_eq!(devices[1]["status"], "pending");
    assert_eq!(
        devices[1]["code"],
        slop_api::chat::device_code(&pending[0].public())
    );
    let res = app
        .send(
            req(
                Method::DELETE,
                format!("/v1/chat/devices/{}", pending[0].id),
            )
            .header("authorization", bearer.clone()),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let devices = list(bearer.clone()).await;
    assert_eq!(devices.as_array().unwrap().len(), 5);
    assert!(
        devices
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["id"] != pending[0].id.to_string()),
        "removed devices aren't listed"
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/chat/devices"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_new_chat_device_is_approved_only_with_an_active_devices_signature(db: PgPool) {
    use ed25519_dalek::Signer;
    let app = test_app_with(db, chat_limits).await;
    let (ann_id, ann_web) = app.user("Ann", "US").await;
    let (bob_id, bob_web) = app.user("Bob", "US").await;
    let first = first_device(&app, &ann_web).await;
    let (laptop, status) = chat_device(&app, &ann_web, 3).await;
    assert_eq!(status, "pending");
    let (phone, _) = chat_device(&app, &ann_web, 3).await;
    let bob_first = first_device(&app, &bob_web).await;
    let (bob_pending, _) = chat_device(&app, &bob_web, 3).await;
    let sign = |by: &Dev, user: Uuid, device: &Dev| {
        b64(&by
            .key
            .sign(&slop_api::chat::approval_message(
                user,
                device.id,
                &device.public(),
            ))
            .to_bytes())
    };
    let approve = |as_dev: &Dev, device: Uuid, by: Uuid, signature: String| {
        as_dev
            .signed_in(Method::POST, format!("/v1/chat/devices/{device}/approve"))
            .json(json!({ "by": by, "signature": signature }))
    };

    // A forged signature: made with the new device's own key, or over another device.
    let res = app
        .send(approve(
            &first,
            laptop.id,
            first.id,
            sign(&laptop, ann_id, &laptop),
        ))
        .await;
    assert_eq!(field_of(res).await, "signature");
    let res = app
        .send(approve(
            &first,
            laptop.id,
            first.id,
            sign(&first, ann_id, &phone),
        ))
        .await;
    assert_eq!(field_of(res).await, "signature");
    let res = app
        .send(approve(&first, laptop.id, first.id, "not base64!".into()))
        .await;
    assert_eq!(field_of(res).await, "signature");
    // Approved by a device that's itself waiting: no.
    assert_eq!(
        app.send(approve(
            &first,
            laptop.id,
            phone.id,
            sign(&phone, ann_id, &laptop)
        ))
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    // Someone else's device, either side of it: no.
    assert_eq!(
        app.send(approve(
            &first,
            bob_pending.id,
            first.id,
            sign(&first, bob_id, &bob_pending)
        ))
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(approve(
            &bob_first,
            laptop.id,
            bob_first.id,
            sign(&bob_first, ann_id, &laptop)
        ))
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let status: String = sqlx::query_scalar("select status from chat_devices where id = $1")
        .bind(laptop.id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "pending");
    // A pending device can't use chat yet.
    assert_eq!(
        app.send(laptop.call(Method::GET, "/v1/chat/sync"))
            .await
            .status(),
        StatusCode::CONFLICT
    );

    // The real approval: active, with who approved it and the signature kept for others to check.
    let signature = sign(&first, ann_id, &laptop);
    assert_eq!(
        app.send(approve(&first, laptop.id, first.id, signature.clone()))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let (status, by, sig): (String, Option<Uuid>, Option<Vec<u8>>) =
        sqlx::query_as("select status, approved_by, approval_sig from chat_devices where id = $1")
            .bind(laptop.id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((status.as_str(), by), ("active", Some(first.id)));
    assert_eq!(b64(&sig.unwrap()), signature);
    assert_eq!(
        app.send(laptop.call(Method::GET, "/v1/chat/sync"))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        app.send(approve(&first, laptop.id, first.id, signature))
            .await
            .status(),
        StatusCode::NOT_FOUND,
        "only waiting devices are approved"
    );

    // Waiting more than a day: it expires, and can't be approved any more.
    sqlx::query("update chat_devices set created_at = now() - interval '25 hours' where id = $1")
        .bind(phone.id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        slop_api::chat::expire_pending(&app.db).await.unwrap(),
        1,
        "only the old one"
    );
    assert_eq!(
        app.send(approve(
            &first,
            phone.id,
            first.id,
            sign(&first, ann_id, &phone)
        ))
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let (bob_status,): (String,) = sqlx::query_as("select status from chat_devices where id = $1")
        .bind(bob_pending.id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(bob_status, "pending", "a new pending device stays");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn starting_over_needs_a_fresh_sign_in_and_replaces_every_other_device(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (ann_id, ann_web) = app.user("Ann", "US").await;
    let (bob_id, bob_web) = app.user("Bob", "US").await;
    befriend(&app, ann_id, bob_id).await;
    let old = first_device(&app, &ann_web).await;
    let bob = first_device(&app, &bob_web).await;
    chat_id(&app, &bob, json!({ "kind": "direct", "user": ann_id })).await;
    let (new, status) = chat_device(&app, &ann_web, 3).await;
    assert_eq!(status, "pending");
    let start_over = |dev: &Dev, id: Uuid| {
        dev.signed_in(Method::POST, format!("/v1/chat/devices/{id}/start-over"))
    };

    // Not with a sign-in from yesterday.
    sqlx::query(
        "update sessions set created_at = now() - interval '1 day' where user_id = $1 and kind = 'desktop'",
    )
    .bind(ann_id)
    .execute(&app.db)
    .await
    .unwrap();
    let res = app.send(start_over(&new, new.id)).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(res).await["error"]["code"], "step_up_required");

    // Freshly signed in: only a waiting device of their own.
    let fresh = Dev {
        bearer: app_bearer(&app, &ann_web).await,
        ..new
    };
    for id in [old.id, bob.id, Uuid::now_v7()] {
        assert_eq!(
            app.send(start_over(&fresh, id)).await.status(),
            StatusCode::NOT_FOUND,
            "{id}"
        );
    }

    // Bob's app is waiting for news: the change of devices wakes it.
    let router = app.router.clone();
    let (bob_bearer, bob_device) = (bob.bearer.clone(), bob.id);
    let after: i64 = sqlx::query_scalar("select coalesce(max(seq), 0) from chat_messages")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let before = sync_now(&app, &bob).await["roster"].clone();
    let waiting = tokio::spawn(async move {
        let started = std::time::Instant::now();
        let res = router
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/chat/sync?after={after}&wait=20"))
                    .header(header::HOST, "localhost:8080")
                    .header("authorization", bob_bearer)
                    .header("x-slop-chat-device", bob_device.to_string())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        (started.elapsed(), body_json(res).await)
    });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(
        app.send(start_over(&fresh, fresh.id)).await.status(),
        StatusCode::NO_CONTENT
    );
    let (took, woke) = waiting.await.unwrap();
    assert!(
        took < std::time::Duration::from_secs(10),
        "woken, not timed out: {took:?}"
    );
    assert_ne!(
        woke["roster"], before,
        "Bob's app sees Ann's devices changed"
    );

    let devices: Vec<(Uuid, String, Option<Uuid>)> = sqlx::query_as(
        "select id, status, approved_by from chat_devices where user_id = $1 order by created_at",
    )
    .bind(ann_id)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        devices,
        [
            (old.id, "revoked".to_string(), None),
            (fresh.id, "active".to_string(), None)
        ]
    );
    let old_packages: i64 =
        sqlx::query_scalar("select count(*) from chat_key_packages where device_id = $1")
            .bind(old.id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(old_packages, 0, "the lost devices' key packages are gone");
    assert_eq!(
        app.send(old.call(Method::GET, "/v1/chat/sync"))
            .await
            .status(),
        StatusCode::CONFLICT
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn removing_a_chat_device_stops_it_and_deletes_its_keys(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (ann_id, ann_web) = app.user("Ann", "US").await;
    let (bob_id, bob_web) = app.user("Bob", "US").await;
    let ann = first_device(&app, &ann_web).await;
    let bob = first_device(&app, &bob_web).await;
    befriend(&app, ann_id, bob_id).await;
    let conv = chat_id(&app, &ann, json!({ "kind": "direct", "user": bob_id })).await;
    // A Welcome waiting for Bob's device.
    let res = app
        .send(
            ann.call(
                Method::POST,
                format!("/v1/chat/conversations/{conv}/commit"),
            )
            .json(json!({ "epoch": 0, "commit": b64(b"commit"),
                              "welcomes": [{ "device_id": bob.id, "welcome": b64(b"welcome") }] })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let remove =
        |as_dev: &Dev, id: Uuid| as_dev.signed_in(Method::DELETE, format!("/v1/chat/devices/{id}"));

    assert_eq!(
        app.send(remove(&ann, bob.id)).await.status(),
        StatusCode::NOT_FOUND,
        "not someone else's"
    );
    assert_eq!(
        app.send(remove(&ann, Uuid::now_v7())).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(remove(&bob, bob.id)).await.status(),
        StatusCode::NO_CONTENT
    );
    let (status, revoked, packages, welcomes): (String, bool, i64, i64) = sqlx::query_as(
        "select status, revoked_at is not null,
                (select count(*) from chat_key_packages where device_id = $1),
                (select count(*) from chat_welcomes where device_id = $1)
         from chat_devices where id = $1",
    )
    .bind(bob.id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        (status.as_str(), revoked, packages, welcomes),
        ("revoked", true, 0, 0)
    );
    assert_eq!(
        app.send(bob.call(Method::GET, "/v1/chat/sync"))
            .await
            .status(),
        StatusCode::CONFLICT,
        "a removed device can't sync"
    );
    assert_eq!(
        send_message(&app, &bob, conv, "still here").await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(remove(&bob, bob.id)).await.status(),
        StatusCode::NOT_FOUND,
        "removed once"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn key_packages_are_topped_up_by_their_own_device_within_a_limit(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (_, ann_web) = app.user("Ann", "US").await;
    let (_, bob_web) = app.user("Bob", "US").await;
    let ann = first_device(&app, &ann_web).await;
    let bob = first_device(&app, &bob_web).await;
    let top_up = |as_dev: &Dev, id: Uuid, n: usize, last_resort: Option<&str>| {
        let mut body = json!({ "key_packages": vec![b64(b"kp"); n] });
        if let Some(lr) = last_resort {
            body["last_resort"] = json!(b64(lr.as_bytes()));
        }
        as_dev
            .signed_in(Method::POST, format!("/v1/chat/devices/{id}/key-packages"))
            .json(body)
    };
    let stored = |id: Uuid| {
        let db = app.db.clone();
        async move {
            sqlx::query_as::<_, (i64, i64)>(
                "select count(*) filter (where not last_resort), count(*) filter (where last_resort)
                 from chat_key_packages where device_id = $1",
            )
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    assert_eq!(stored(ann.id).await, (5, 1));

    assert_eq!(
        app.send(top_up(&bob, ann.id, 1, None)).await.status(),
        StatusCode::NOT_FOUND,
        "not someone else's device"
    );
    assert_eq!(
        app.send(top_up(&ann, ann.id, 15, Some("new last resort")))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(stored(ann.id).await, (20, 1), "one last resort, replaced");
    let lr: Vec<u8> = sqlx::query_scalar(
        "select key_package from chat_key_packages where device_id = $1 and last_resort",
    )
    .bind(ann.id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(lr, b"new last resort");
    // At most twice the usual number waiting.
    let res = app.send(top_up(&ann, ann.id, 21, None)).await;
    assert_eq!(field_of(res).await, "key_packages");
    assert_eq!(
        app.send(top_up(&ann, ann.id, 20, None)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(stored(ann.id).await, (40, 1));
    let res = app.send(top_up(&ann, ann.id, 1, None)).await;
    assert_eq!(field_of(res).await, "key_packages");
    // A removed device takes no more.
    app.send(bob.signed_in(Method::DELETE, format!("/v1/chat/devices/{}", bob.id)))
        .await;
    assert_eq!(
        app.send(top_up(&bob, bob.id, 1, None)).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn starting_chats_reuses_one_per_pair_team_and_self_and_checks_groups(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (ann_id, ann_web) = app.user("Ann", "US").await;
    let (bob_id, _) = app.user("Bob", "US").await;
    let (cara_id, _) = app.user("Cara", "US").await;
    let (_, stranger_web) = app.user("Sam Stranger", "US").await;
    befriend(&app, ann_id, bob_id).await;
    befriend(&app, ann_id, cara_id).await;
    let ann = first_device(&app, &ann_web).await;

    // One direct chat per pair, one self chat: asking again returns it.
    let (status, direct) =
        start_chat(&app, &ann, json!({ "kind": "direct", "user": bob_id })).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, again) = start_chat(
        &app,
        &ann,
        json!({ "kind": "direct", "user": format!("http://localhost:8080/u/{bob_id}") }),
    )
    .await;
    assert_eq!((status, &again["id"]), (StatusCode::OK, &direct["id"]));
    let (status, me) = start_chat(&app, &ann, json!({ "kind": "self" })).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, me_again) = start_chat(&app, &ann, json!({ "kind": "self" })).await;
    assert_eq!((status, &me_again["id"]), (StatusCode::OK, &me["id"]));

    // Groups: someone besides you, fewer than fifty, everyone someone you may message.
    let (status, err) = start_chat(
        &app,
        &ann,
        json!({ "kind": "group", "users": [ann_id.to_string()] }),
    )
    .await;
    assert_eq!(
        (status, err["error"]["field"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("users"))
    );
    let many: Vec<String> = (0..50).map(|_| Uuid::now_v7().to_string()).collect();
    let (status, err) = start_chat(&app, &ann, json!({ "kind": "group", "users": many })).await;
    assert_eq!(
        (status, err["error"]["field"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("users"))
    );
    let (status, _) = start_chat(
        &app,
        &ann,
        json!({ "kind": "group", "users": [bob_id, Uuid::now_v7()] }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let groups: i64 =
        sqlx::query_scalar("select count(*) from chat_conversations where kind = 'group'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(groups, 0);
    let (status, group) = start_chat(
        &app,
        &ann,
        json!({ "kind": "group", "users": [bob_id, cara_id, bob_id] }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(group["members"].as_array().unwrap().len(), 3, "Bob once");
    let (status, _) = start_chat(&app, &ann, json!({ "kind": "chatroom" })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Team chats: for members of a visible player team, one per team.
    make_team(
        &app,
        &ann_web,
        "night-owls",
        json!({ "name": "Night Owls" }),
    )
    .await;
    add_to_team(&app, "night-owls", bob_id, "member").await;
    let (status, team) =
        start_chat(&app, &ann, json!({ "kind": "team", "team": "Night-Owls" })).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        (team["kind"].as_str(), team["team_name"].as_str()),
        (Some("team"), Some("Night Owls"))
    );
    assert_eq!(team["members"].as_array().unwrap().len(), 2);
    let (status, team_again) =
        start_chat(&app, &ann, json!({ "kind": "team", "team": "night-owls" })).await;
    assert_eq!((status, &team_again["id"]), (StatusCode::OK, &team["id"]));
    make_team(
        &app,
        &ann_web,
        "acme",
        json!({ "kind": "official", "github_org": "acme" }),
    )
    .await;
    make_team(&app, &stranger_web, "elsewhere", json!({})).await;
    for slug in ["acme", "elsewhere", "no-such-team"] {
        let (status, _) = start_chat(&app, &ann, json!({ "kind": "team", "team": slug })).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{slug}");
    }
    sqlx::query("update teams set hidden_reason = 'Spam' where slug = 'night-owls'")
        .execute(&app.db)
        .await
        .unwrap();
    let (status, _) = start_chat(&app, &ann, json!({ "kind": "team", "team": "night-owls" })).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "hidden teams have no chat");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_chat_list_leaves_out_hidden_chats_and_marks_closed_ones(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (ann_id, ann_web) = app.user("Ann", "US").await;
    let (bob_id, bob_web) = app.user("Bob", "US").await;
    let (dan_id, dan_web) = app.user("Dan", "US").await;
    let (_, stranger_web) = app.user("Sam Stranger", "US").await;
    befriend(&app, ann_id, bob_id).await;
    befriend(&app, ann_id, dan_id).await;
    let ann = first_device(&app, &ann_web).await;
    let with_bob = chat_id(&app, &ann, json!({ "kind": "direct", "user": bob_id })).await;
    let with_dan = chat_id(&app, &ann, json!({ "kind": "direct", "user": dan_id })).await;
    make_team(
        &app,
        &ann_web,
        "night-owls",
        json!({ "name": "Night Owls" }),
    )
    .await;
    add_to_team(&app, "night-owls", bob_id, "member").await;
    let team = chat_id(&app, &ann, json!({ "kind": "team", "team": "night-owls" })).await;
    let list = |web: &str| {
        let r = req(Method::GET, "/v1/chat/conversations").cookie(web);
        let app = &app;
        async move { body_json(app.send(r).await).await }
    };
    let ids = |v: &Value| -> Vec<Uuid> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_str().unwrap().parse().unwrap())
            .collect()
    };

    // Team chats are listed for the team's members.
    let bobs = list(&bob_web).await;
    assert_eq!(ids(&bobs).len(), 2);
    let t = bobs
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "team")
        .unwrap();
    assert_eq!(
        (t["team_slug"].as_str(), t["team_name"].as_str()),
        (Some("night-owls"), Some("Night Owls"))
    );
    assert!(ids(&list(&stranger_web).await).is_empty());

    // Hidden: gone from the list until a new message arrives.
    let hide = |web: &str, id: Uuid| {
        req(Method::POST, format!("/v1/chat/conversations/{id}/hide")).cookie(web)
    };
    assert_eq!(
        app.send(hide(&stranger_web, team)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(hide(&bob_web, team)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(ids(&list(&bob_web).await), [with_bob]);
    assert_eq!(
        send_message(&app, &ann, team, "game tonight?")
            .await
            .status(),
        StatusCode::OK
    );
    assert!(ids(&list(&bob_web).await).contains(&team));

    // Closed: a direct chat with someone who blocked you, or who closed their account.
    let anns = list(&ann_web).await;
    let closed = |v: &Value, id: Uuid| {
        v.as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id.to_string())
            .unwrap()["closed"]
            .as_bool()
            .unwrap()
    };
    assert!(!closed(&anns, with_dan) && !closed(&anns, with_bob));
    app.send(req(Method::PUT, format!("/v1/me/blocks/{ann_id}")).cookie(&dan_web))
        .await;
    sqlx::query("update users set closed_at = now() where id = $1")
        .bind(bob_id)
        .execute(&app.db)
        .await
        .unwrap();
    let anns = list(&ann_web).await;
    assert!(closed(&anns, with_dan), "blocked by Dan");
    assert!(closed(&anns, with_bob), "Bob closed his account");
    assert!(!closed(&anns, team), "only direct chats close");
    assert!(
        !ids(&list(&dan_web).await).contains(&with_dan),
        "the blocker doesn't see it"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn chat_details_and_member_devices_are_for_members_only(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (ann_id, ann_web) = app.user("Ann", "US").await;
    let (bob_id, bob_web) = app.user("Bob", "US").await;
    let (_, stranger_web) = app.user("Sam Stranger", "US").await;
    befriend(&app, ann_id, bob_id).await;
    let ann = first_device(&app, &ann_web).await;
    let bob = first_device(&app, &bob_web).await;
    let (bob_pending, _) = chat_device(&app, &bob_web, 1).await;
    let (bob_old, _) = chat_device(&app, &bob_web, 1).await;
    sqlx::query("update chat_devices set status = 'revoked' where id = $1")
        .bind(bob_old.id)
        .execute(&app.db)
        .await
        .unwrap();
    let conv = chat_id(&app, &ann, json!({ "kind": "direct", "user": bob_id })).await;

    let detail = body_json(
        app.send(req(Method::GET, format!("/v1/chat/conversations/{conv}")).cookie(&bob_web))
            .await,
    )
    .await;
    assert_eq!(detail["kind"], "direct");
    let names: Vec<&str> = detail["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Ann", "Bob"]);
    assert_eq!(detail["closed"], false);
    for path in [
        format!("/v1/chat/conversations/{conv}"),
        format!("/v1/chat/conversations/{conv}/devices"),
        format!("/v1/chat/conversations/{}", Uuid::now_v7()),
    ] {
        assert_eq!(
            app.send(req(Method::GET, path.clone()).cookie(&stranger_web))
                .await
                .status(),
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }

    // The devices the encryption should include: active and removed ones (to check approval
    // chains), never one still waiting.
    let devices = body_json(
        app.send(
            req(
                Method::GET,
                format!("/v1/chat/conversations/{conv}/devices"),
            )
            .cookie(&ann_web),
        )
        .await,
    )
    .await;
    let listed: Vec<(String, String)> = devices
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["id"].as_str().unwrap().to_string(),
                d["status"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert!(listed.contains(&(ann.id.to_string(), "active".into())));
    assert!(listed.contains(&(bob.id.to_string(), "active".into())));
    assert!(listed.contains(&(bob_old.id.to_string(), "revoked".into())));
    assert!(!listed.iter().any(|d| d.0 == bob_pending.id.to_string()));
    let bobs = devices
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == bob.id.to_string())
        .unwrap();
    assert_eq!(bobs["signature_key"], b64(&bob.public()));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn key_packages_are_claimed_only_for_active_devices_in_the_chat(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (ann_id, ann_web) = app.user("Ann", "US").await;
    let (bob_id, bob_web) = app.user("Bob", "US").await;
    let (_, cara_web) = app.user("Cara", "US").await;
    befriend(&app, ann_id, bob_id).await;
    let ann = first_device(&app, &ann_web).await;
    let (bob, _) = chat_device(&app, &bob_web, 1).await;
    let (bob_pending, _) = chat_device(&app, &bob_web, 1).await;
    let cara = first_device(&app, &cara_web).await;
    let conv = chat_id(&app, &ann, json!({ "kind": "direct", "user": bob_id })).await;
    let claim = |as_dev: &Dev, conv: Uuid, devices: Vec<Uuid>| {
        as_dev
            .call(
                Method::POST,
                format!("/v1/chat/conversations/{conv}/key-packages"),
            )
            .json(json!({ "devices": devices }))
    };

    let res = app
        .send(claim(
            &ann,
            conv,
            (0..201).map(|_| Uuid::now_v7()).collect(),
        ))
        .await;
    assert_eq!(field_of(res).await, "devices");
    assert_eq!(
        app.send(claim(&cara, conv, vec![bob.id])).await.status(),
        StatusCode::NOT_FOUND,
        "not in the chat"
    );

    // Bob's one key package, then his last-resort one, which is never used up; never Cara's
    // (not in the chat) or a device still waiting.
    let mut got = Vec::new();
    for _ in 0..3 {
        let res = app
            .send(claim(&ann, conv, vec![bob.id, bob_pending.id, cara.id]))
            .await;
        assert_eq!(res.status(), StatusCode::OK);
        let claimed = body_json(res).await;
        assert_eq!(claimed.as_array().unwrap().len(), 1, "{claimed}");
        assert_eq!(claimed[0]["device_id"], bob.id.to_string());
        got.push(claimed[0]["key_package"].as_str().unwrap().to_string());
    }
    assert_eq!(got[0], b64(format!("kp-{}-0", bob.id).as_bytes()));
    assert_eq!(got[1], b64(format!("lr-{}", bob.id).as_bytes()));
    assert_eq!(got[2], got[1]);
    let left: Vec<bool> =
        sqlx::query_scalar("select last_resort from chat_key_packages where device_id = $1")
            .bind(bob.id)
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(left, [true]);
    let cara_left: i64 =
        sqlx::query_scalar("select count(*) from chat_key_packages where device_id = $1")
            .bind(cara.id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(cara_left, 6);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn group_members_add_people_they_may_message_who_havent_blocked_anyone_there(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (ann_id, ann_web) = app.user("Ann", "US").await;
    let (bob_id, bob_web) = app.user("Bob", "US").await;
    let (cara_id, _) = app.user("Cara", "US").await;
    let (dan_id, _) = app.user("Dan", "US").await;
    let (eve_id, _) = app.user("Eve", "US").await;
    let (_, stranger_web) = app.user("Sam Stranger", "US").await;
    for friend in [bob_id, cara_id, eve_id] {
        befriend(&app, ann_id, friend).await;
    }
    let ann = first_device(&app, &ann_web).await;
    let group = chat_id(&app, &ann, json!({ "kind": "group", "users": [bob_id] })).await;
    let direct = chat_id(&app, &ann, json!({ "kind": "direct", "user": cara_id })).await;
    make_team(&app, &ann_web, "night-owls", json!({})).await;
    let team = chat_id(&app, &ann, json!({ "kind": "team", "team": "night-owls" })).await;
    let add = |web: &str, conv: Uuid, users: Vec<String>| {
        post(
            format!("/v1/chat/conversations/{conv}/members"),
            web,
            json!({ "users": users }),
        )
    };
    let members = |conv: Uuid| {
        let db = app.db.clone();
        async move {
            sqlx::query_scalar::<_, Uuid>(
                "select user_id from chat_members where conversation_id = $1 order by joined_at",
            )
            .bind(conv)
            .fetch_all(&db)
            .await
            .unwrap()
        }
    };

    assert_eq!(
        app.send(add(&stranger_web, group, vec![cara_id.to_string()]))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    for conv in [direct, team] {
        assert_eq!(
            app.send(add(&ann_web, conv, vec![eve_id.to_string()]))
                .await
                .status(),
            StatusCode::CONFLICT
        );
    }
    let res = app
        .send(add(&ann_web, group, vec![dan_id.to_string()]))
        .await;
    assert_eq!(field_of(res).await, "users", "Ann may not message Dan");
    // Eve is Ann's friend, but Bob (already in the group) blocked her.
    app.send(req(Method::PUT, format!("/v1/me/blocks/{eve_id}")).cookie(&bob_web))
        .await;
    let res = app
        .send(add(&ann_web, group, vec![eve_id.to_string()]))
        .await;
    assert_eq!(field_of(res).await, "users");
    let too_many: Vec<String> = (0..49).map(|_| Uuid::now_v7().to_string()).collect();
    let res = app.send(add(&ann_web, group, too_many)).await;
    assert_eq!(field_of(res).await, "users", "fifty at most");
    assert_eq!(members(group).await, [ann_id, bob_id]);

    let res = app
        .send(add(&ann_web, group, vec![format!("/u/{cara_id}")]))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["members"].as_array().unwrap().len(), 3);
    assert_eq!(members(group).await, [ann_id, bob_id, cara_id]);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn people_leave_group_chats_and_the_last_one_out_ends_it(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (ann_id, ann_web) = app.user("Ann", "US").await;
    let (bob_id, bob_web) = app.user("Bob", "US").await;
    let (cara_id, cara_web) = app.user("Cara", "US").await;
    let (_, stranger_web) = app.user("Sam Stranger", "US").await;
    befriend(&app, ann_id, bob_id).await;
    befriend(&app, ann_id, cara_id).await;
    let ann = first_device(&app, &ann_web).await;
    let group = chat_id(
        &app,
        &ann,
        json!({ "kind": "group", "users": [bob_id, cara_id] }),
    )
    .await;
    let direct = chat_id(&app, &ann, json!({ "kind": "direct", "user": bob_id })).await;
    make_team(&app, &ann_web, "night-owls", json!({})).await;
    let team = chat_id(&app, &ann, json!({ "kind": "team", "team": "night-owls" })).await;
    let leave = |web: &str, conv: Uuid, who: Uuid| {
        req(
            Method::DELETE,
            format!("/v1/chat/conversations/{conv}/members/{who}"),
        )
        .cookie(web)
    };
    let roles = |conv: Uuid| {
        let db = app.db.clone();
        async move {
            sqlx::query_as::<_, (Uuid, String)>(
                "select user_id, role from chat_members where conversation_id = $1 order by joined_at",
            )
            .bind(conv)
            .fetch_all(&db)
            .await
            .unwrap()
        }
    };

    assert_eq!(
        app.send(leave(&stranger_web, group, bob_id)).await.status(),
        StatusCode::NOT_FOUND
    );
    for conv in [direct, team] {
        assert_eq!(
            app.send(leave(&ann_web, conv, ann_id)).await.status(),
            StatusCode::CONFLICT,
            "direct and team chats are hidden, not left"
        );
    }
    // Anyone leaves by themselves.
    assert_eq!(
        app.send(leave(&bob_web, group, bob_id)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(req(Method::GET, format!("/v1/chat/conversations/{group}")).cookie(&bob_web))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    // The owner leaving hands the group on to whoever has been in it longest.
    assert_eq!(
        app.send(leave(&ann_web, group, ann_id)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(roles(group).await, [(cara_id, "owner".to_string())]);
    assert_eq!(
        app.send(leave(&cara_web, group, cara_id)).await.status(),
        StatusCode::NO_CONTENT
    );
    let left: i64 = sqlx::query_scalar("select count(*) from chat_conversations where id = $1")
        .bind(group)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(left, 0, "the last one out ends it");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn away_members_get_a_content_free_notice_unless_muted_and_reading_clears_it(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (ann_id, ann_web) = app.user("Ann", "US").await;
    let (bob_id, bob_web) = app.user("Bob", "US").await;
    let (cara_id, cara_web) = app.user("Cara", "US").await;
    let (_, stranger_web) = app.user("Sam Stranger", "US").await;
    befriend(&app, ann_id, bob_id).await;
    befriend(&app, ann_id, cara_id).await;
    let ann = first_device(&app, &ann_web).await;
    // Bob and Cara have no app online: they're away.
    let group = chat_id(
        &app,
        &ann,
        json!({ "kind": "group", "users": [bob_id, cara_id] }),
    )
    .await;
    let mute = |web: &str, muted: bool| {
        req(Method::PUT, format!("/v1/chat/conversations/{group}/mute"))
            .cookie(web)
            .json(json!({ "muted": muted }))
    };
    assert_eq!(
        app.send(mute(&stranger_web, true)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(mute(&bob_web, true)).await.status(),
        StatusCode::NO_CONTENT
    );

    for text in ["one", "two"] {
        assert_eq!(
            send_message(&app, &ann, group, text).await.status(),
            StatusCode::OK
        );
    }
    let notices = |user: Uuid| {
        let db = app.db.clone();
        async move {
            sqlx::query_as::<_, (String, String, i32, bool)>(
                "select title, body, count, read_at is not null from notifications
                 where user_id = $1 and category = 'chat'",
            )
            .bind(user)
            .fetch_all(&db)
            .await
            .unwrap()
        }
    };
    let cara = notices(cara_id).await;
    assert_eq!(cara.len(), 1, "one notice per chat until read");
    assert_eq!(cara[0].0, "New messages in a group chat from Ann");
    assert_eq!(cara[0].2, 2);
    assert!(!cara[0].1.contains("one") && cara[0].1.contains("end-to-end encrypted"));
    assert!(notices(bob_id).await.is_empty(), "Bob muted it");
    assert!(notices(ann_id).await.is_empty(), "not for the sender");
    let muted = body_json(
        app.send(req(Method::GET, format!("/v1/chat/conversations/{group}")).cookie(&bob_web))
            .await,
    )
    .await;
    assert_eq!(muted["muted"], true);

    // Unmuted: Bob hears again. Someone with the app open isn't told at all.
    app.send(mute(&bob_web, false)).await;
    let bob = first_device(&app, &bob_web).await;
    send_message(&app, &ann, group, "three").await;
    assert!(notices(bob_id).await.is_empty(), "Bob's app is online");
    sqlx::query(
        "update chat_devices set last_seen_at = now() - interval '10 minutes' where id = $1",
    )
    .bind(bob.id)
    .execute(&app.db)
    .await
    .unwrap();
    send_message(&app, &ann, group, "four").await;
    assert_eq!(notices(bob_id).await.len(), 1);

    // Reading clears the bell's notice; only members mark read.
    let read = |web: &str, seq: i64| {
        post(
            format!("/v1/chat/conversations/{group}/read"),
            web,
            json!({ "seq": seq }),
        )
    };
    assert_eq!(
        app.send(read(&stranger_web, 1)).await.status(),
        StatusCode::NOT_FOUND
    );
    let last: i64 = sqlx::query_scalar("select max(seq) from chat_messages")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(
        app.send(read(&cara_web, last)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert!(notices(cara_id).await[0].3, "read");
    let state = body_json(
        app.send(req(Method::GET, format!("/v1/chat/conversations/{group}")).cookie(&cara_web))
            .await,
    )
    .await;
    assert_eq!(
        (state["read_seq"].as_i64(), state["unread"].as_i64()),
        (Some(last), Some(0))
    );
    // Never backwards.
    app.send(read(&cara_web, 1)).await;
    let read_seq: i64 = sqlx::query_scalar(
        "select read_seq from chat_member_state where conversation_id = $1 and user_id = $2",
    )
    .bind(group)
    .bind(cara_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(read_seq, last);

    // A direct chat's notice names the sender; a team chat's says so.
    let direct = chat_id(&app, &ann, json!({ "kind": "direct", "user": cara_id })).await;
    send_message(&app, &ann, direct, "hi").await;
    make_team(&app, &ann_web, "night-owls", json!({})).await;
    add_to_team(&app, "night-owls", cara_id, "member").await;
    let team = chat_id(&app, &ann, json!({ "kind": "team", "team": "night-owls" })).await;
    send_message(&app, &ann, team, "hi team").await;
    let titles: Vec<String> = notices(cara_id).await.into_iter().map(|n| n.0).collect();
    assert!(
        titles.contains(&"New message from Ann".to_string()),
        "{titles:?}"
    );
    assert!(
        titles.contains(&"New messages in your team chat from Ann".to_string()),
        "{titles:?}"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn typing_shows_for_a_few_seconds_and_never_in_a_closed_chat(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (ann_id, ann_web) = app.user("Ann", "US").await;
    let (bob_id, bob_web) = app.user("Bob", "US").await;
    let (cara_id, cara_web) = app.user("Cara", "US").await;
    befriend(&app, ann_id, bob_id).await;
    befriend(&app, ann_id, cara_id).await;
    let ann = first_device(&app, &ann_web).await;
    let bob = first_device(&app, &bob_web).await;
    let cara = first_device(&app, &cara_web).await;
    let with_bob = chat_id(&app, &ann, json!({ "kind": "direct", "user": bob_id })).await;
    let with_cara = chat_id(&app, &ann, json!({ "kind": "direct", "user": cara_id })).await;
    let typing = |dev: &Dev, conv: Uuid| {
        dev.call(
            Method::POST,
            format!("/v1/chat/conversations/{conv}/typing"),
        )
        .json(json!({ "typing": true }))
    };

    assert_eq!(
        app.send(typing(&cara, with_bob)).await.status(),
        StatusCode::NOT_FOUND,
        "not her chat"
    );
    assert_eq!(
        app.send(typing(&ann, with_bob)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        sync_now(&app, &bob).await["typing"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // A few seconds after the last signal, it's gone.
    tokio::time::sleep(slop_api::chat::TYPING_FOR + std::time::Duration::from_millis(300)).await;
    assert!(
        sync_now(&app, &bob).await["typing"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // Cara blocked Ann: Ann's typing goes nowhere (and doesn't say why).
    app.send(req(Method::PUT, format!("/v1/me/blocks/{ann_id}")).cookie(&cara_web))
        .await;
    assert_eq!(
        app.send(typing(&ann, with_cara)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert!(
        sync_now(&app, &cara).await["typing"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn commits_are_refused_in_closed_chats_and_with_welcomes_for_outsiders(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (ann_id, ann_web) = app.user("Ann", "US").await;
    let (bob_id, bob_web) = app.user("Bob", "US").await;
    let (_, cara_web) = app.user("Cara", "US").await;
    befriend(&app, ann_id, bob_id).await;
    let ann = first_device(&app, &ann_web).await;
    let bob = first_device(&app, &bob_web).await;
    let cara = first_device(&app, &cara_web).await;
    let conv = chat_id(&app, &ann, json!({ "kind": "direct", "user": bob_id })).await;
    let commit = |dev: &Dev, conv: Uuid, body: Value| {
        dev.call(
            Method::POST,
            format!("/v1/chat/conversations/{conv}/commit"),
        )
        .json(body)
    };
    let state = || {
        let db = app.db.clone();
        async move {
            sqlx::query_as::<_, (i64, i64, i64)>(
                "select (select epoch from chat_conversations where id = $1),
                        (select count(*) from chat_messages where conversation_id = $1),
                        (select count(*) from chat_welcomes where conversation_id = $1)",
            )
            .bind(conv)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };

    // A Welcome for a device not in the chat: nothing is kept.
    let res = app
        .send(commit(
            &ann,
            conv,
            json!({ "epoch": 0, "commit": b64(b"c"),
                    "welcomes": [{ "device_id": cara.id, "welcome": b64(b"w") }] }),
        ))
        .await;
    assert_eq!(field_of(res).await, "welcomes");
    assert_eq!(state().await, (0, 0, 0));
    assert_eq!(
        app.send(commit(
            &cara,
            conv,
            json!({ "epoch": 0, "commit": b64(b"c") })
        ))
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(commit(
            &ann,
            Uuid::now_v7(),
            json!({ "epoch": 0, "commit": b64(b"c") })
        ))
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let res = app
        .send(commit(
            &ann,
            conv,
            json!({ "epoch": 0, "commit": b64(&vec![0; slop_api::chat::MAX_COMMIT_BODY + 1]) }),
        ))
        .await;
    assert_eq!(field_of(res).await, "commit");
    let res = app
        .send(commit(
            &ann,
            conv,
            json!({ "epoch": 0, "commit": b64(b"c"),
                    "welcomes": [{ "device_id": bob.id, "welcome": b64(b"w") }] }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(state().await, (1, 1, 1));

    // Bob blocks Ann: no more changes to the chat from her.
    app.send(req(Method::PUT, format!("/v1/me/blocks/{ann_id}")).cookie(&bob_web))
        .await;
    let res = app
        .send(commit(
            &ann,
            conv,
            json!({ "epoch": 1, "commit": b64(b"c") }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(state().await, (1, 1, 1));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_large_group_commit_within_the_limit_is_accepted(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (_, ann_web) = app.user("Ann", "US").await;
    let ann = first_device(&app, &ann_web).await;
    let conv = chat_id(&app, &ann, json!({ "kind": "self" })).await;
    // Adding many devices at once makes a commit of a few hundred KiB: under MAX_COMMIT_BODY.
    let big = vec![7u8; 300 * 1024];
    let res = app
        .send(
            ann.call(
                Method::POST,
                format!("/v1/chat/conversations/{conv}/commit"),
            )
            .json(json!({ "epoch": 0, "commit": b64(&big) })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{}", body_json(res).await);
    let stored: i32 = sqlx::query_scalar("select length(body) from chat_messages")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(stored as usize, big.len());
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn messages_are_checked_for_size_tag_budget_and_pace(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (ann_id, ann_web) = app.user("Ann", "US").await;
    let (_, stranger_web) = app.user("Sam Stranger", "US").await;
    let ann = first_device(&app, &ann_web).await;
    let stranger = first_device(&app, &stranger_web).await;
    let conv = chat_id(&app, &ann, json!({ "kind": "self" })).await;
    let send = |dev: &Dev, conv: Uuid, body: Value| {
        dev.call(
            Method::POST,
            format!("/v1/chat/conversations/{conv}/messages"),
        )
        .json(body)
    };

    for (body, field) in [
        (
            json!({ "epoch": 0, "body": b64(b"x"), "franking_tag": b64(&[0; 31]) }),
            "franking_tag",
        ),
        (
            json!({ "epoch": 0, "body": b64(b"x"), "franking_tag": b64(&[0; 33]) }),
            "franking_tag",
        ),
        (
            json!({ "epoch": 0, "body": b64(b"x"), "franking_tag": "nope!" }),
            "franking_tag",
        ),
        (
            json!({ "epoch": 0, "body": b64(&vec![0; slop_api::chat::MAX_MESSAGE_BYTES + 1]), "franking_tag": b64(&[0; 32]) }),
            "body",
        ),
    ] {
        let res = app.send(send(&ann, conv, body)).await;
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(field_of(res).await, field);
    }
    assert_eq!(
        app.send(send(&stranger, conv, message(0, "hi")))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(send(&ann, Uuid::now_v7(), message(0, "hi")))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let res = app
        .send(send(
            &ann,
            conv,
            json!({ "epoch": 0, "body": b64(&vec![0; slop_api::chat::MAX_MESSAGE_BYTES]), "franking_tag": b64(&[0; 32]) }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK, "32 KiB exactly is fine");

    // Forty messages a minute; the forty-first waits.
    for i in 1..40 {
        let res = app
            .send(send(&ann, conv, message(0, &format!("m{i}"))))
            .await;
        assert_eq!(res.status(), StatusCode::OK, "{i}");
    }
    let res = app.send(send(&ann, conv, message(0, "41"))).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    // Messages and commits share the account's chat limit (60 a minute): when it's spent, 429.
    // (It refills one a second, so spend it again right before each request.)
    let spend = || {
        while app
            .state
            .account_limiter
            .take(
                ann_id,
                slop_api::limits::Action::ChatMessage,
                std::time::Instant::now(),
            )
            .is_ok()
        {}
    };
    spend();
    let res = app
        .send(
            ann.call(
                Method::POST,
                format!("/v1/chat/conversations/{conv}/commit"),
            )
            .json(json!({ "epoch": 0, "commit": b64(b"c") })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    spend();
    let res = app.send(send(&ann, conv, message(0, "later"))).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_days_chat_budget_stops_messages_commits_and_attachments(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (ann_id, ann_web) = app.user("Ann", "US").await;
    let ann = first_device(&app, &ann_web).await;
    let conv = chat_id(&app, &ann, json!({ "kind": "self" })).await;
    assert_eq!(
        send_message(&app, &ann, conv, "before").await.status(),
        StatusCode::OK
    );
    // Today's uploads add up to the whole budget.
    sqlx::query(
        "insert into chat_attachments (id, conversation_id, uploader_id, size_bytes, body)
         values ($1, $2, $3, $4, '\\x00')",
    )
    .bind(Uuid::now_v7())
    .bind(conv)
    .bind(ann_id)
    .bind((slop_api::chat::DAILY_BYTES - 10) as i32)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(
        send_message(&app, &ann, conv, "this is over")
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let res = app
        .send(
            ann.call(
                Method::POST,
                format!("/v1/chat/conversations/{conv}/commit"),
            )
            .json(json!({ "epoch": 0, "commit": b64(b"twelve bytes") })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let res = send_bytes(
        &app,
        ann.call(
            Method::POST,
            format!("/v1/chat/conversations/{conv}/attachments"),
        ),
        vec![1; 100],
    )
    .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    // Yesterday's don't count.
    sqlx::query("update chat_attachments set created_at = now() - interval '25 hours'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        send_message(&app, &ann, conv, "a new day").await.status(),
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn sync_waits_for_news_and_stops_for_people_who_left(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (ann_id, ann_web) = app.user("Ann", "US").await;
    let (bob_id, bob_web) = app.user("Bob", "US").await;
    let (cara_id, cara_web) = app.user("Cara", "US").await;
    befriend(&app, ann_id, bob_id).await;
    befriend(&app, ann_id, cara_id).await;
    let ann = first_device(&app, &ann_web).await;
    let bob = first_device(&app, &bob_web).await;
    let cara = first_device(&app, &cara_web).await;
    let group = chat_id(
        &app,
        &ann,
        json!({ "kind": "group", "users": [bob_id, cara_id] }),
    )
    .await;
    let wait = |dev: &Dev, after: i64, secs: u64| {
        let router = app.router.clone();
        let r = Request::builder()
            .uri(format!("/v1/chat/sync?after={after}&wait={secs}"))
            .header(header::HOST, "localhost:8080")
            .header("authorization", dev.bearer.clone())
            .header("x-slop-chat-device", dev.id.to_string())
            .body(Body::empty())
            .unwrap();
        tokio::spawn(async move {
            let started = std::time::Instant::now();
            let res = router.oneshot(r).await.unwrap();
            (started.elapsed(), body_json(res).await)
        })
    };

    // Nothing new: an empty answer when the wait is up.
    let (took, empty) = wait(&bob, 0, 1).await.unwrap();
    assert!(took >= std::time::Duration::from_millis(900), "{took:?}");
    assert!(empty["events"].as_array().unwrap().is_empty());
    assert_eq!(empty["next"], 0);

    // A message wakes a waiting device at once.
    let waiting = wait(&bob, 0, 20);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    send_message(&app, &ann, group, "hello").await;
    let (took, woke) = waiting.await.unwrap();
    assert!(took < std::time::Duration::from_secs(10), "{took:?}");
    assert_eq!(woke["events"].as_array().unwrap().len(), 1);
    let next = woke["next"].as_i64().unwrap();
    assert!(next > 0);
    // Asking from there: nothing more.
    let res = app
        .send(bob.call(Method::GET, format!("/v1/chat/sync?after={next}")))
        .await;
    assert!(
        body_json(res).await["events"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // Cara is removed: what's said after isn't delivered to her.
    app.send(
        req(
            Method::DELETE,
            format!("/v1/chat/conversations/{group}/members/{cara_id}"),
        )
        .cookie(&ann_web),
    )
    .await;
    send_message(&app, &ann, group, "after cara").await;
    let caras = sync_now(&app, &cara).await;
    assert_eq!(
        caras["events"].as_array().unwrap().len(),
        0,
        "nothing from a chat she's no longer in: {caras}"
    );
    assert_eq!(
        sync_now(&app, &bob).await["events"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn attachments_are_for_the_chats_members_only(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (ann_id, ann_web) = app.user("Ann", "US").await;
    let (bob_id, bob_web) = app.user("Bob", "US").await;
    let (cara_id, cara_web) = app.user("Cara", "US").await;
    befriend(&app, ann_id, bob_id).await;
    befriend(&app, ann_id, cara_id).await;
    let ann = first_device(&app, &ann_web).await;
    let cara = first_device(&app, &cara_web).await;
    let group = chat_id(
        &app,
        &ann,
        json!({ "kind": "group", "users": [bob_id, cara_id] }),
    )
    .await;
    let upload = |dev: &Dev, conv: Uuid| {
        dev.call(
            Method::POST,
            format!("/v1/chat/conversations/{conv}/attachments"),
        )
    };

    assert_eq!(
        send_bytes(&app, upload(&ann, group), vec![]).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let res = send_bytes(
        &app,
        upload(&ann, group),
        vec![0; slop_api::chat::MAX_ATTACHMENT_BYTES + 1],
    )
    .await;
    assert!(
        matches!(
            res.status(),
            StatusCode::PAYLOAD_TOO_LARGE | StatusCode::UNPROCESSABLE_ENTITY
        ),
        "{}",
        res.status()
    );
    let stranger_chat = {
        let (_, dan_web) = app.user("Dan", "US").await;
        let dan = first_device(&app, &dan_web).await;
        let conv = chat_id(&app, &dan, json!({ "kind": "self" })).await;
        assert_eq!(
            send_bytes(&app, upload(&ann, conv), vec![1, 2, 3])
                .await
                .status(),
            StatusCode::NOT_FOUND,
            "not her chat"
        );
        conv
    };
    let res = send_bytes(&app, upload(&ann, group), b"encrypted meme".to_vec()).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let id = body_json(res).await["id"].as_str().unwrap().to_string();

    // Members download it; anyone else, or someone removed, finds nothing.
    let get =
        |web: &str, id: &str| req(Method::GET, format!("/v1/chat/attachments/{id}")).cookie(web);
    let res = app.send(get(&bob_web, &id)).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_bytes(res).await, b"encrypted meme");
    let (_, stranger_web) = app.user("Sam Stranger", "US").await;
    assert_eq!(
        app.send(get(&stranger_web, &id)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(get(&bob_web, &Uuid::now_v7().to_string()))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    app.send(
        req(
            Method::DELETE,
            format!("/v1/chat/conversations/{group}/members/{cara_id}"),
        )
        .cookie(&ann_web),
    )
    .await;
    assert_eq!(
        app.send(get(&cara_web, &id)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send_bytes(&app, upload(&cara, group), vec![1])
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // A direct chat with someone who blocked you takes no more files.
    let direct = chat_id(&app, &ann, json!({ "kind": "direct", "user": bob_id })).await;
    app.send(req(Method::PUT, format!("/v1/me/blocks/{ann_id}")).cookie(&bob_web))
        .await;
    assert_eq!(
        send_bytes(&app, upload(&ann, direct), vec![1])
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let _ = stranger_chat;
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_chat_backup_is_saved_from_a_device_and_read_only_by_its_owner(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (_, ann_web) = app.user("Ann", "US").await;
    let (_, bob_web) = app.user("Bob", "US").await;
    let ann = first_device(&app, &ann_web).await;
    let get = |web: &str| req(Method::GET, "/v1/chat/backup").cookie(web);

    assert_eq!(
        app.send(get(&ann_web)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send_bytes(&app, ann.call(Method::PUT, "/v1/chat/backup"), vec![])
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let res = send_bytes(
        &app,
        ann.call(Method::PUT, "/v1/chat/backup"),
        vec![0; slop_api::chat::MAX_ATTACHMENT_BYTES + 1],
    )
    .await;
    assert!(
        matches!(
            res.status(),
            StatusCode::PAYLOAD_TOO_LARGE | StatusCode::UNPROCESSABLE_ENTITY
        ),
        "{}",
        res.status()
    );
    // Only from a chat device, not the website.
    let res = send_bytes(
        &app,
        req(Method::PUT, "/v1/chat/backup").cookie(&ann_web),
        b"sealed".to_vec(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

    for body in [b"sealed backup v1".to_vec(), b"sealed backup v2".to_vec()] {
        assert_eq!(
            send_bytes(&app, ann.call(Method::PUT, "/v1/chat/backup"), body)
                .await
                .status(),
            StatusCode::NO_CONTENT
        );
    }
    let res = app.send(get(&ann_web)).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        body_bytes(res).await,
        b"sealed backup v2",
        "the newest replaces"
    );
    assert_eq!(
        app.send(get(&bob_web)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/chat/backup")).await.status(),
        StatusCode::UNAUTHORIZED
    );

    // Bob deleting "his" backup doesn't touch Ann's; Ann's delete does.
    app.send(req(Method::DELETE, "/v1/chat/backup").cookie(&bob_web))
        .await;
    assert_eq!(app.send(get(&ann_web)).await.status(), StatusCode::OK);
    assert_eq!(
        app.send(req(Method::DELETE, "/v1/chat/backup").cookie(&ann_web))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(get(&ann_web)).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn reported_chat_messages_are_checked_for_who_may_report_and_whether_they_still_exist(
    db: PgPool,
) {
    use hmac::{Hmac, KeyInit, Mac};
    let app = test_app_with(db, chat_limits).await;
    let (ann_id, ann_web) = app.user("Ann", "US").await;
    let (bob_id, bob_web) = app.user("Bob", "US").await;
    let (_, stranger_web) = app.user("Sam Stranger", "US").await;
    befriend(&app, ann_id, bob_id).await;
    let ann = first_device(&app, &ann_web).await;
    let conv = chat_id(&app, &ann, json!({ "kind": "direct", "user": bob_id })).await;
    let plaintext = r#"{"text":"send me your password"}"#;
    let key = [9u8; 32];
    let mut mac = <Hmac<sha2::Sha256> as KeyInit>::new_from_slice(&key).unwrap();
    mac.update(plaintext.as_bytes());
    let tag = mac.finalize().into_bytes();
    let res = app
        .send(
            ann.call(
                Method::POST,
                format!("/v1/chat/conversations/{conv}/messages"),
            )
            .json(json!({ "epoch": 0, "body": b64(b"ciphertext"), "franking_tag": b64(&tag) })),
        )
        .await;
    let seq = body_json(res).await["seq"].as_i64().unwrap();
    let report = |web: &str, franking_key: String| {
        post(
            "/v1/reports",
            web,
            json!({
                "kind": "chat_message", "subject": conv.to_string(), "reason": "harassment",
                "details": "Phishing.", "good_faith": true,
                "chat": { "conversation_id": conv, "seq": seq, "plaintext": plaintext, "franking_key": franking_key },
            }),
        )
    };

    let res = app.send(report(&ann_web, b64(&key))).await;
    assert_eq!(field_of(res).await, "subject", "not your own message");
    assert_eq!(
        app.send(report(&stranger_web, b64(&key))).await.status(),
        StatusCode::NOT_FOUND,
        "not in the chat"
    );
    let res = app.send(report(&bob_web, "not base64!".into())).await;
    assert_eq!(field_of(res).await, "franking_key");
    let res = app.send(report(&bob_web, b64(&[8; 32]))).await;
    assert_eq!(field_of(res).await, "plaintext", "the wrong key");
    let reports: i64 = sqlx::query_scalar("select count(*) from reports")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(reports, 0);

    // Older than 30 days, it's gone: Slop says so rather than taking it on trust.
    sqlx::query("update chat_messages set created_at = now() - interval '31 days'")
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::chat::cleanup(&app.db).await.unwrap();
    let res = app.send(report(&bob_web, b64(&key))).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("older than 30 days")
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn chat_cleanup_deletes_only_what_is_over_thirty_days_and_expires_waiting_devices(
    db: PgPool,
) {
    let app = test_app_with(db, chat_limits).await;
    let (ann_id, ann_web) = app.user("Ann", "US").await;
    let (bob_id, bob_web) = app.user("Bob", "US").await;
    befriend(&app, ann_id, bob_id).await;
    let ann = first_device(&app, &ann_web).await;
    let bob = first_device(&app, &bob_web).await;
    let (bob_waiting, _) = chat_device(&app, &bob_web, 1).await;
    let (bob_new, _) = chat_device(&app, &bob_web, 1).await;
    let conv = chat_id(&app, &ann, json!({ "kind": "direct", "user": bob_id })).await;
    for _ in 0..2 {
        let epoch: i64 = sqlx::query_scalar("select epoch from chat_conversations where id = $1")
            .bind(conv)
            .fetch_one(&app.db)
            .await
            .unwrap();
        app.send(
            ann.call(
                Method::POST,
                format!("/v1/chat/conversations/{conv}/commit"),
            )
            .json(json!({ "epoch": epoch, "commit": b64(b"c"),
                          "welcomes": [{ "device_id": bob.id, "welcome": b64(b"w") }] })),
        )
        .await;
        app.send(
            ann.call(
                Method::POST,
                format!("/v1/chat/conversations/{conv}/messages"),
            )
            .json(message(epoch + 1, "m")),
        )
        .await;
        send_bytes(
            &app,
            ann.call(
                Method::POST,
                format!("/v1/chat/conversations/{conv}/attachments"),
            ),
            vec![1; 10],
        )
        .await;
    }
    let counts = || {
        let db = app.db.clone();
        async move {
            sqlx::query_as::<_, (i64, i64, i64)>(
                "select (select count(*) from chat_messages), (select count(*) from chat_welcomes),
                        (select count(*) from chat_attachments)",
            )
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    assert_eq!(counts().await, (4, 2, 2));
    // Half of it is 31 days old; the rest 29.
    for table in ["chat_messages", "chat_welcomes", "chat_attachments"] {
        let key = if table == "chat_messages" {
            "seq"
        } else {
            "id"
        };
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "update {table} set created_at = now() - interval '29 days'"
        )))
        .execute(&app.db)
        .await
        .unwrap();
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "update {table} set created_at = now() - interval '31 days'
             where {key} in (select {key} from {table} order by {key} limit (select count(*) / 2 from {table}))"
        )))
        .execute(&app.db)
        .await
        .unwrap();
    }
    // Bob removed his phone: its key packages are left over; a device waited more than a day.
    sqlx::query("update chat_devices set status = 'revoked' where id = $1")
        .bind(bob_new.id)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("update chat_devices set created_at = now() - interval '2 days' where id = $1")
        .bind(bob_waiting.id)
        .execute(&app.db)
        .await
        .unwrap();

    let deleted = slop_api::chat::cleanup(&app.db).await.unwrap();
    assert_eq!(counts().await, (2, 1, 1), "the newer half is kept");
    assert_eq!(
        deleted,
        4 + 2,
        "four old rows and the removed device's two packages"
    );
    let packages: i64 =
        sqlx::query_scalar("select count(*) from chat_key_packages where device_id = $1")
            .bind(bob_new.id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(packages, 0);
    let ann_packages: i64 =
        sqlx::query_scalar("select count(*) from chat_key_packages where device_id = $1")
            .bind(ann.id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(ann_packages, 6, "active devices keep theirs");

    assert_eq!(slop_api::chat::expire_pending(&app.db).await.unwrap(), 1);
    let statuses: Vec<(Uuid, String)> =
        sqlx::query_as("select id, status from chat_devices where user_id = $1 order by id")
            .bind(bob_id)
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(
        statuses,
        [
            (bob.id, "active".to_string()),
            (bob_waiting.id, "revoked".to_string()),
            (bob_new.id, "revoked".to_string())
        ]
    );
}

// ---------------------------------------------------------------------------------------------
// The Slop app's chat library (slop-client), against the API

/// Like `chat_app`, with the data folder and secure store given, to reopen or lose them.
async fn chat_in(
    app: &TestApp,
    base: &str,
    user: Uuid,
    cookie: &str,
    dir: &std::path::Path,
    secrets: &MemorySecrets,
) -> slop_client::Result<Chat> {
    let tokens = desktop_sign_in(app, cookie).await;
    let store = slop_client::api::MemoryTokenStore::default();
    use slop_client::api::TokenStore;
    store.save(tokens["refresh_token"].as_str().unwrap());
    let api = slop_client::api::ApiClient::new(base.parse().unwrap(), std::sync::Arc::new(store))
        .unwrap();
    Chat::open(api, dir.to_path_buf(), secrets, user)
}

fn temp_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("slop-chat-test-{}", Uuid::now_v7()))
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_chat_library_backs_up_and_restores_after_starting_over(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let base = serve(&app).await;
    let (ann_id, ann_web) = app.user("Ann", "US").await;
    let (bob_id, bob_web) = app.user("Bob", "US").await;
    befriend(&app, ann_id, bob_id).await;
    let (dir, secrets) = (temp_dir(), MemorySecrets::default());
    let ann = chat_in(&app, &base, ann_id, &ann_web, &dir, &secrets)
        .await
        .unwrap();
    let bob = chat_app(&app, &base, bob_id, &bob_web).await;
    ann.setup("Ann's PC").await.unwrap();
    bob.setup("Bob's PC").await.unwrap();
    let conv = ann.start_direct(&bob_id.to_string()).await.unwrap();
    bob.sync(0).await.unwrap();
    ann.send(conv, "the secret plan", None, vec![], false)
        .await
        .unwrap();
    bob.sync(0).await.unwrap();
    bob.send(conv, "sounds good", None, vec![], false)
        .await
        .unwrap();
    ann.sync(0).await.unwrap();

    // The backup is sealed with the recovery code: Slop can't read it.
    let code = ann.set_backups(true).await.unwrap().unwrap();
    assert!(ann.status().await.unwrap().backups);
    let sealed: Vec<u8> = sqlx::query_scalar("select body from chat_backups where user_id = $1")
        .bind(ann_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(!sealed.windows(6).any(|w| w == b"secret" || w == b"sounds"));

    // The app's own copy on disk is sealed with a key from the system's secure store.
    let file = std::fs::read(dir.join(format!("chat-{ann_id}.bin"))).unwrap();
    assert!(!file.windows(6).any(|w| w == b"secret" || w == b"sounds"));
    drop(ann);
    let reopened = chat_in(&app, &base, ann_id, &ann_web, &dir, &secrets)
        .await
        .unwrap();
    assert!(
        reopened
            .messages(conv, None, 10)
            .await
            .unwrap()
            .iter()
            .any(|m| m.text == "the secret plan")
    );
    drop(reopened);
    assert!(
        chat_in(
            &app,
            &base,
            ann_id,
            &ann_web,
            &dir,
            &MemorySecrets::default()
        )
        .await
        .is_err(),
        "without the stored key the file can't be opened"
    );

    // Every device lost: a new one starts over (fresh sign-in), then restores with the code.
    let ann2 = chat_in(
        &app,
        &base,
        ann_id,
        &ann_web,
        &temp_dir(),
        &MemorySecrets::default(),
    )
    .await
    .unwrap();
    assert_eq!(ann2.setup("Ann's new PC").await.unwrap().state, "pending");
    ann2.start_over().await.unwrap();
    let status = ann2.status().await.unwrap();
    assert_eq!(status.state, "active");
    assert_eq!(status.devices.len(), 1, "the lost device is gone");
    assert!(ann2.messages(conv, None, 10).await.unwrap().is_empty());
    assert!(ann2.restore_backup("WRONG-CODE-0000").await.is_err());
    let restored = ann2.restore_backup(&code).await.unwrap();
    assert!(restored >= 2, "{restored}");
    let texts: Vec<String> = ann2
        .messages(conv, None, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.text)
        .collect();
    assert!(
        texts.contains(&"the secret plan".to_string())
            && texts.contains(&"sounds good".to_string()),
        "{texts:?}"
    );

    // Turning backups off deletes the copy at Slop.
    ann2.set_backups(false).await.unwrap();
    assert_eq!(
        app.send(req(Method::GET, "/v1/chat/backup").cookie(&ann_web))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert!(!ann2.status().await.unwrap().backups);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_chat_library_renames_adds_leaves_hides_mutes_and_starts_team_chats(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let base = serve(&app).await;
    let mut people = Vec::new();
    for name in ["Ann", "Bob", "Cara"] {
        people.push(app.user(name, "US").await);
    }
    let (ann_id, bob_id, cara_id) = (people[0].0, people[1].0, people[2].0);
    befriend(&app, ann_id, bob_id).await;
    befriend(&app, ann_id, cara_id).await;
    let mut apps = Vec::new();
    for (id, web) in &people {
        let chat = chat_app(&app, &base, *id, web).await;
        chat.setup("PC").await.unwrap();
        apps.push(chat);
    }
    let (ann, bob, cara) = (&apps[0], &apps[1], &apps[2]);
    let sync_all = || async {
        for _ in 0..2 {
            for a in &apps {
                a.sync(0).await.unwrap();
            }
        }
    };
    let titles = |list: Vec<slop_client::chat::ConversationView>| -> Vec<(Uuid, String)> {
        list.into_iter().map(|c| (c.id, c.title)).collect()
    };

    // The name of a group travels encrypted.
    let group = ann
        .start_group(&[bob_id.to_string()], Some("Raid night"))
        .await
        .unwrap();
    sync_all().await;
    ann.rename(group, "Raid night 2").await.unwrap();
    assert!(ann.rename(group, "  ").await.is_err());
    sync_all().await;
    assert_eq!(
        titles(bob.conversations().await.unwrap()),
        [(group, "Raid night 2".to_string())]
    );
    let stored: Vec<Vec<u8>> = sqlx::query_scalar("select body from chat_messages")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert!(
        stored
            .iter()
            .all(|b| !b.windows(10).any(|w| w == b"Raid night"))
    );

    // Adding someone: they read what's said after.
    ann.add_people(group, &[cara_id.to_string()]).await.unwrap();
    sync_all().await;
    ann.send(group, "welcome cara", None, vec![], false)
        .await
        .unwrap();
    sync_all().await;
    assert!(
        cara.messages(group, None, 10)
            .await
            .unwrap()
            .iter()
            .any(|m| m.text == "welcome cara")
    );

    // Leaving: gone from Bob's list and the members.
    bob.leave(group).await.unwrap();
    assert!(
        bob.conversations()
            .await
            .unwrap()
            .iter()
            .all(|c| c.id != group)
    );
    let members: Vec<Uuid> = sqlx::query_scalar(
        "select user_id from chat_members where conversation_id = $1 order by joined_at",
    )
    .bind(group)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(members, [ann_id, cara_id]);

    // Hiding until the next message; muting.
    cara.hide(group).await.unwrap();
    assert!(
        cara.conversations()
            .await
            .unwrap()
            .iter()
            .all(|c| c.id != group)
    );
    cara.set_muted(group, true).await.unwrap();
    sync_all().await;
    ann.send(group, "back again", None, vec![], false)
        .await
        .unwrap();
    let list = cara.conversations().await.unwrap();
    let g = list.iter().find(|c| c.id == group).unwrap();
    assert!(g.muted);

    // A team chat: everyone in the team.
    make_team(
        &app,
        &people[0].1,
        "night-owls",
        json!({ "name": "Night Owls" }),
    )
    .await;
    add_to_team(&app, "night-owls", bob_id, "member").await;
    let team = ann.start_team("night-owls").await.unwrap();
    sync_all().await;
    ann.send(team, "game tonight?", None, vec![], false)
        .await
        .unwrap();
    sync_all().await;
    let bobs = bob.conversations().await.unwrap();
    let t = bobs.iter().find(|c| c.id == team).unwrap();
    assert_eq!((t.kind.as_str(), t.title.as_str()), ("team", "Night Owls"));
    assert!(
        bob.messages(team, None, 10)
            .await
            .unwrap()
            .iter()
            .any(|m| m.text == "game tonight?")
    );
    assert!(cara.start_team("night-owls").await.is_err(), "not in it");

    // Removing one of your devices.
    let ann_laptop = chat_app(&app, &base, ann_id, &people[0].1).await;
    let pending = ann_laptop.setup("Laptop").await.unwrap();
    ann.remove_device(pending.device_id.unwrap()).await.unwrap();
    assert_eq!(ann_laptop.status().await.unwrap().state, "revoked");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_chat_library_tops_up_key_packages_below_half(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let base = serve(&app).await;
    let (ann_id, ann_web) = app.user("Ann", "US").await;
    let (bob_id, bob_web) = app.user("Bob", "US").await;
    befriend(&app, ann_id, bob_id).await;
    let ann = chat_app(&app, &base, ann_id, &ann_web).await;
    let ann_device = ann.setup("PC").await.unwrap().device_id.unwrap();
    let bob = first_device(&app, &bob_web).await;
    let conv = chat_id(&app, &bob, json!({ "kind": "direct", "user": ann_id })).await;
    let left = || {
        let db = app.db.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "select count(*) from chat_key_packages where device_id = $1 and not last_resort",
            )
            .bind(ann_device)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    assert_eq!(left().await, 20);
    // Others add Ann's device to chats: ten left is still half.
    for _ in 0..10 {
        app.send(
            bob.call(
                Method::POST,
                format!("/v1/chat/conversations/{conv}/key-packages"),
            )
            .json(json!({ "devices": [ann_device] })),
        )
        .await;
    }
    ann.sync(0).await.unwrap();
    assert_eq!(left().await, 10, "not below half: no top-up");
    app.send(
        bob.call(
            Method::POST,
            format!("/v1/chat/conversations/{conv}/key-packages"),
        )
        .json(json!({ "devices": [ann_device] })),
    )
    .await;
    ann.sync(0).await.unwrap();
    assert_eq!(left().await, 29, "nine left, topped up by twenty");
}

#[test]
fn payment_warnings_catch_every_off_platform_payment_phrase() {
    use slop_client::chat::payment_warning;
    for phrase in [
        "PayPal",
        "venmo",
        "Cash App",
        "cashapp",
        "zelle",
        "a gift card",
        "giftcard",
        "Bitcoin",
        "my crypto wallet",
        "USDT",
        "wire transfer",
        "Western Union",
        "moneygram",
        "IBAN",
        "bank details",
        "send money",
        "pay me first",
    ] {
        let text = format!("hey, about the sprites: {phrase} ok?");
        let warning = payment_warning(&text).unwrap_or_else(|| panic!("{phrase}"));
        assert!(warning.contains("outside Slop"), "{warning}");
    }
    for fine in [
        "see you at game night",
        "payment through Slop is fine",
        "the bank in the game",
    ] {
        assert_eq!(payment_warning(fine), None, "{fine}");
    }
}

// ---------------------------------------------------------------------------------------------
// Bug reports

/// Records that a user played an app (as a launch through the Slop app would).
async fn played(app: &TestApp, user: Uuid, slug: &str) {
    sqlx::query(
        "insert into user_app_usage (user_id, app_id, days_launched) select $1, id, 1 from apps where slug = $2",
    )
    .bind(user)
    .bind(slug)
    .execute(&app.db)
    .await
    .unwrap();
}

/// Files a bug report through the API; returns its ID.
async fn file_bug(app: &TestApp, cookie: &str, slug: &str, title: &str) -> String {
    let res = app
        .send(post(
            format!("/v1/apps/{slug}/bug-reports"),
            cookie,
            json!({ "title": title, "body": "It breaks.", "version": "1.0.0", "platform": "linux",
                    "log": "panic at main.rs:1" }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    body_json(res).await["id"].as_str().unwrap().to_string()
}

fn report_titles(list: &Value) -> Vec<String> {
    let mut t: Vec<String> = list["reports"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["title"].as_str().unwrap().to_string())
        .collect();
    t.sort();
    t
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn bug_report_lists_filter_by_status_and_show_players_only_open_ones_and_their_own(
    db: PgPool,
) {
    let app = test_app_with(db, chat_limits).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    let (pat_id, pat) = app.user("Pat Player", "US").await;
    let (sam_id, sam) = app.user("Sam Player", "US").await;
    let (_, never) = app.user("Nev Never", "US").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    approved_release(&app, "tower", maker_id).await;
    played(&app, pat_id, "tower").await;
    played(&app, sam_id, "tower").await;
    let pats_fixed = file_bug(&app, &pat, "tower", "Pat: fixed one").await;
    file_bug(&app, &pat, "tower", "Pat: open one").await;
    let sams_wontfix = file_bug(&app, &sam, "tower", "Sam: won't fix").await;
    let sams_dupe = file_bug(&app, &sam, "tower", "Sam: duplicate").await;
    for (id, status) in [
        (&pats_fixed, "fixed"),
        (&sams_wontfix, "wont_fix"),
        (&sams_dupe, "duplicate"),
    ] {
        let res = app
            .send(
                req(Method::PATCH, format!("/v1/bug-reports/{id}"))
                    .cookie(&maker)
                    .json(json!({ "status": status })),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK);
    }
    let list = |cookie: &str, q: &str| {
        let r = req(Method::GET, format!("/v1/apps/tower/bug-reports{q}")).cookie(cookie);
        let app = &app;
        async move {
            let res = app.send(r).await;
            (res.status(), body_json(res).await)
        }
    };

    assert_eq!(
        list(&never, "").await.0,
        StatusCode::NOT_FOUND,
        "never played"
    );
    assert_eq!(
        app.send(req(Method::GET, "/v1/apps/tower/bug-reports"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let (status, err) = list(&maker, "?status=closed").await;
    assert_eq!(
        (status, err["error"]["field"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("status"))
    );

    // The maker: each status, or all.
    for (q, want) in [
        ("", vec!["Pat: open one"]),
        ("?status=open", vec!["Pat: open one"]),
        ("?status=fixed", vec!["Pat: fixed one"]),
        ("?status=wont_fix", vec!["Sam: won't fix"]),
        ("?status=duplicate", vec!["Sam: duplicate"]),
        (
            "?status=all",
            vec![
                "Pat: fixed one",
                "Pat: open one",
                "Sam: duplicate",
                "Sam: won't fix",
            ],
        ),
    ] {
        let (status, l) = list(&maker, q).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(report_titles(&l), want, "{q}");
        assert_eq!(l["is_maker"], true);
    }
    // A player: open reports, and their own closed ones; never someone else's closed ones.
    let (_, l) = list(&sam, "?status=all").await;
    assert_eq!(
        report_titles(&l),
        ["Pat: open one", "Sam: duplicate", "Sam: won't fix"]
    );
    assert_eq!(
        (l["is_maker"].as_bool(), l["can_report"].as_bool()),
        (Some(false), Some(true))
    );
    let (_, l) = list(&sam, "?status=fixed").await;
    assert!(report_titles(&l).is_empty(), "Pat's fixed report is Pat's");
    let (_, l) = list(&pat, "?status=fixed").await;
    assert_eq!(report_titles(&l), ["Pat: fixed one"]);
    let mine = &l["reports"][0];
    assert_eq!(mine["mine"], true);
    assert_eq!(mine["log"], "panic at main.rs:1", "their own in full");

    // Private Slop: players it's shared with, while it's shared.
    let (_, olive) = (maker_id, maker.clone());
    private_app(&app, maker_id, &olive, "secret-tool").await;
    let (finn_id, finn) = app.user("Finn Friend", "US").await;
    let (tia_id, tia) = app.user("Tia Teammate", "US").await;
    make_team(&app, &maker, "testers", json!({})).await;
    add_to_team(&app, "testers", tia_id, "member").await;
    app.send(post(
        "/v1/apps/secret-tool/shares",
        &maker,
        json!({ "user": finn_id.to_string() }),
    ))
    .await;
    app.send(post(
        "/v1/apps/secret-tool/shares",
        &maker,
        json!({ "team": "testers" }),
    ))
    .await;
    for (id, _) in [(finn_id, &finn), (tia_id, &tia)] {
        played(&app, id, "secret-tool").await;
    }
    file_bug(&app, &finn, "secret-tool", "Private crash").await;
    for c in [&finn, &tia] {
        let r = app
            .send(req(Method::GET, "/v1/apps/secret-tool/bug-reports").cookie(c))
            .await;
        assert_eq!(r.status(), StatusCode::OK);
    }
    app.send(
        req(
            Method::DELETE,
            format!("/v1/apps/secret-tool/shares/{finn_id}"),
        )
        .cookie(&maker),
    )
    .await;
    app.send(req(Method::DELETE, "/v1/apps/secret-tool/team-shares/testers").cookie(&maker))
        .await;
    for c in [&finn, &tia] {
        let r = app
            .send(req(Method::GET, "/v1/apps/secret-tool/bug-reports").cookie(c))
            .await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "no longer shared");
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn makers_update_bug_reports_and_the_right_players_are_told(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    let (pat_id, pat) = app.user("Pat Player", "US").await;
    let (sam_id, sam) = app.user("Sam Player", "US").await;
    let (_, other_maker) = app.user("Other Maker", "US").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    approved_release(&app, "tower", maker_id).await;
    played(&app, pat_id, "tower").await;
    played(&app, sam_id, "tower").await;
    let id = file_bug(&app, &pat, "tower", "Crash on wave 3").await;
    app.send(req(Method::POST, format!("/v1/bug-reports/{id}/me-too")).cookie(&sam))
        .await;
    let update = |cookie: &str, body: Value| {
        req(Method::PATCH, format!("/v1/bug-reports/{id}"))
            .cookie(cookie)
            .json(body)
    };
    let told = |user: Uuid| {
        let db = app.db.clone();
        async move {
            sqlx::query_as::<_, (String, String)>(
                "select title, body from notifications where user_id = $1 and title like 'Your bug report%'
                 order by created_at",
            )
            .bind(user)
            .fetch_all(&db)
            .await
            .unwrap()
        }
    };

    // The maker sees it in full.
    let full = body_json(
        app.send(req(Method::GET, format!("/v1/bug-reports/{id}")).cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(
        (
            full["reporter"].as_str(),
            full["log"].as_str(),
            full["me_too"].as_i64()
        ),
        (Some("Pat Player"), Some("panic at main.rs:1"), Some(1))
    );
    assert_eq!(
        app.send(req(Method::GET, format!("/v1/bug-reports/{id}")).cookie(&other_maker))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(update(&other_maker, json!({ "status": "fixed" })))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let res = app
        .send(update(&maker, json!({ "status": "closed" })))
        .await;
    assert_eq!(field_of(res).await, "status");
    let res = app
        .send(update(&maker, json!({ "maker_note": "x".repeat(2001) })))
        .await;
    assert_eq!(field_of(res).await, "maker_note");

    // Won't fix: the reporter is told with the note; me-too players only hear about fixes.
    let res = app
        .send(update(
            &maker,
            json!({ "status": "wont_fix", "maker_note": "Works as designed." }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let r = body_json(res).await;
    assert_eq!(
        (r["status"].as_str(), r["maker_note"].as_str()),
        (Some("wont_fix"), Some("Works as designed."))
    );
    assert_eq!(
        told(pat_id).await,
        [(
            "Your bug report won't be fixed".to_string(),
            "\"Crash on wave 3\": Works as designed.".to_string()
        )]
    );
    assert!(told(sam_id).await.is_empty());
    // The same status again tells nobody; an empty note clears it.
    let res = app
        .send(update(
            &maker,
            json!({ "status": "wont_fix", "maker_note": "" }),
        ))
        .await;
    assert!(body_json(res).await["maker_note"].is_null());
    assert_eq!(told(pat_id).await.len(), 1);
    // Leaving the note out keeps it.
    app.send(update(&maker, json!({ "maker_note": "Kept." })))
        .await;
    let res = app
        .send(update(&maker, json!({ "status": "duplicate" })))
        .await;
    assert_eq!(body_json(res).await["maker_note"], "Kept.");
    let res = app.send(update(&maker, json!({ "status": "open" }))).await;
    assert_eq!(res.status(), StatusCode::OK);
    // Unread notices about one report merge into one, saying what happened last.
    assert_eq!(
        notifications(&app, pat_id).await,
        [("Your bug report is open again".to_string(), 3)]
    );
    assert!(told(sam_id).await.is_empty());
    // Once read, the next change is a new notice; fixed tells the me-too players too.
    sqlx::query("update notifications set read_at = now()")
        .execute(&app.db)
        .await
        .unwrap();
    app.send(update(&maker, json!({ "status": "fixed" }))).await;
    for user in [pat_id, sam_id] {
        let titles: Vec<String> = told(user).await.into_iter().map(|t| t.0).collect();
        assert_eq!(
            titles.last().unwrap(),
            "Your bug report was fixed",
            "{titles:?}"
        );
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn me_too_is_for_players_of_open_reports_once_and_can_be_taken_back(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    let (pat_id, pat) = app.user("Pat Player", "US").await;
    let (sam_id, sam) = app.user("Sam Player", "US").await;
    let (_, never) = app.user("Nev Never", "US").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    played(&app, pat_id, "tower").await;
    played(&app, sam_id, "tower").await;
    let id = file_bug(&app, &pat, "tower", "Crash").await;
    let me_too = |cookie: &str, id: &str, m: Method| {
        req(m, format!("/v1/bug-reports/{id}/me-too")).cookie(cookie)
    };
    let count = || {
        let db = app.db.clone();
        let id: Uuid = id.parse().unwrap();
        async move {
            sqlx::query_scalar::<_, i64>(
                "select count(*) from bug_report_votes where report_id = $1",
            )
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };

    assert_eq!(
        app.send(me_too(&never, &id, Method::POST)).await.status(),
        StatusCode::NOT_FOUND,
        "never played"
    );
    assert_eq!(
        app.send(me_too(&sam, &Uuid::now_v7().to_string(), Method::POST))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    for _ in 0..2 {
        assert_eq!(
            app.send(me_too(&sam, &id, Method::POST)).await.status(),
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(count().await, 1, "once");
    let l = body_json(
        app.send(req(Method::GET, "/v1/apps/tower/bug-reports").cookie(&sam))
            .await,
    )
    .await;
    assert_eq!(
        (
            l["reports"][0]["me_too"].as_i64(),
            l["reports"][0]["i_have_it_too"].as_bool()
        ),
        (Some(1), Some(true))
    );

    // Taking it back; again is nothing; so is a report that doesn't exist.
    for _ in 0..2 {
        assert_eq!(
            app.send(me_too(&sam, &id, Method::DELETE)).await.status(),
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(count().await, 0);
    assert_eq!(
        app.send(me_too(&sam, &Uuid::now_v7().to_string(), Method::DELETE))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send(me_too(&pat, &id, Method::DELETE)).await.status(),
        StatusCode::NO_CONTENT,
        "not voted: nothing to undo"
    );

    // Closed: no more me-toos.
    app.send(
        req(Method::PATCH, format!("/v1/bug-reports/{id}"))
            .cookie(&maker)
            .json(json!({ "status": "fixed" })),
    )
    .await;
    assert_eq!(
        app.send(me_too(&sam, &id, Method::POST)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(count().await, 0);
}

// ---------------------------------------------------------------------------------------------
// Makers' AI agents: People and bug-report tools

/// A maker token's `Authorization` header.
async fn maker_token(app: &TestApp, cookie: &str, body: Value) -> String {
    let res = app.send(post("/v1/me/maker-tokens", cookie, body)).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    format!(
        "Bearer {}",
        body_json(res).await["secret"].as_str().unwrap()
    )
}

/// Calls an MCP tool: (ok, the API's answer as text).
async fn tool(
    app: &TestApp,
    bearer: &str,
    endpoint: &str,
    name: &str,
    args: Value,
) -> (bool, String) {
    let res = app
        .send(
            req(Method::POST, endpoint)
                .header("authorization", bearer.to_string())
                .json(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                              "params": { "name": name, "arguments": args } })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res).await;
    (
        body["result"]["isError"] == false,
        body["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string(),
    )
}

/// A job request from `client` to the creative `handle`, for `slug`; returns its ID.
async fn hire(app: &TestApp, client: &str, handle: &str, slug: &str) -> String {
    let res = app
        .send(post(
            format!("/v1/people/{handle}/hire"),
            client,
            json!({ "app_slug": slug, "skill": "pixel-art", "request_md": "A knight.",
                    "confirm_adult": true, "exact_wording_ack": true }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK, "hire");
    body_json(res).await["id"].as_str().unwrap().to_string()
}

async fn job_status(app: &TestApp, id: &str) -> String {
    sqlx::query_scalar("select status from jobs where id = $1::uuid")
        .bind(id)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_makers_agent_with_people_access_answers_jobs_but_never_agrees_pays_or_delivers(
    db: PgPool,
) {
    let app = test_app_with(db, |c| {
        c.payments.simulated = true;
        chat_limits(c);
    })
    .await;
    let (_, vera) = live_creative(&app, "vera", "US", false).await;
    let (tom_id, tom) = app.user("Tom Client", "US").await;
    listed_with(&app, tom_id, "toms-game", "released", "").await;
    let mcp = "/v1/maker/mcp";
    let agent = maker_token(
        &app,
        &vera,
        json!({ "name": "Claude", "scope": "publish", "people": true }),
    )
    .await;
    let toms_agent = maker_token(
        &app,
        &tom,
        json!({ "name": "Claude", "scope": "publish", "people": true }),
    )
    .await;
    let no_people = maker_token(
        &app,
        &vera,
        json!({ "name": "No people", "scope": "publish" }),
    )
    .await;
    let reader = maker_token(
        &app,
        &vera,
        json!({ "name": "Reader", "scope": "read", "people": true }),
    )
    .await;
    let job = hire(&app, &tom, "vera", "toms-game").await;
    let other = hire(&app, &tom, "vera", "toms-game").await;

    // Reading: the jobs, one job.
    let (ok, text) = tool(&app, &agent, mcp, "my_jobs", json!({})).await;
    assert!(ok && text.contains(&job), "{text}");
    let (ok, text) = tool(&app, &agent, mcp, "get_job", json!({ "id": job })).await;
    assert!(ok && text.contains("requested"), "{text}");

    // Without People access, or read-only, no People work.
    let (ok, _) = tool(&app, &no_people, mcp, "my_jobs", json!({})).await;
    assert!(!ok);
    assert_eq!(
        app.send(req(Method::GET, "/v1/me/jobs").header("authorization", no_people.clone()))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let (ok, _) = tool(&app, &reader, mcp, "my_jobs", json!({})).await;
    assert!(ok, "reading is fine");
    let mut quote = sprite_offer("upfront");
    quote["id"] = json!(job);
    let (ok, _) = tool(&app, &reader, mcp, "send_quote", quote.clone()).await;
    assert!(!ok, "a read token sends nothing");
    assert_eq!(job_status(&app, &job).await, "requested");

    // A quote; declining another request.
    let (ok, text) = tool(&app, &agent, mcp, "send_quote", quote).await;
    assert!(ok, "{text}");
    assert_eq!(job_status(&app, &job).await, "offered");
    let (ok, text) = tool(
        &app,
        &agent,
        mcp,
        "decline_job",
        json!({ "id": other, "note": "Booked up this month." }),
    )
    .await;
    assert!(ok, "{text}");
    assert_eq!(job_status(&app, &other).await, "declined");

    // Agreeing, paying, delivering and the like stay with the person.
    let as_agent = |bearer: &str, path: String, body: Value| {
        req(Method::POST, path)
            .header("authorization", bearer.to_string())
            .json(body)
    };
    assert_eq!(
        app.send(as_agent(
            &toms_agent,
            format!("/v1/jobs/{job}/accept"),
            json!({ "offer_count": 1, "exact_terms_ack": true, "start_now": true })
        ))
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(job_status(&app, &job).await, "offered");
    app.send(post(
        format!("/v1/jobs/{job}/accept"),
        &tom,
        json!({ "offer_count": 1, "exact_terms_ack": true, "start_now": true }),
    ))
    .await;
    for (bearer, step) in [
        (&toms_agent, "pay"),
        (&toms_agent, "payment/check"),
        (&toms_agent, "withdraw"),
        (&toms_agent, "cancel"),
        (&agent, "cancel"),
        (&agent, "deliver"),
        (&agent, "sign-off/send"),
        (&toms_agent, "sign-off"),
        (&toms_agent, "accept-delivery"),
        (&agent, "final-files"),
        (&toms_agent, "change/accept"),
        (&toms_agent, "problem"),
    ] {
        assert_eq!(
            app.send(as_agent(
                bearer,
                format!("/v1/jobs/{job}/{step}"),
                json!({})
            ))
            .await
            .status(),
            StatusCode::FORBIDDEN,
            "{step}"
        );
    }
    assert_eq!(job_status(&app, &job).await, "awaiting_payment");
    app.send(post(format!("/v1/jobs/{job}/pay"), &tom, json!({})))
        .await;
    assert_eq!(job_status(&app, &job).await, "in_progress");

    // Changes: the client's agent asks; the creative's declines it, then offers one.
    let (ok, text) = tool(
        &app,
        &toms_agent,
        mcp,
        "request_change",
        json!({ "id": job, "note": "Could he also jump?" }),
    )
    .await;
    assert!(ok, "{text}");
    let (ok, text) = tool(
        &app,
        &agent,
        mcp,
        "decline_change",
        json!({ "id": job, "note": "Out of scope." }),
    )
    .await;
    assert!(ok, "{text}");
    let due = (OffsetDateTime::now_utc().date() + time::Duration::days(30)).to_string();
    let (ok, text) = tool(
        &app,
        &agent,
        mcp,
        "propose_change",
        json!({ "id": job, "title": "Jumping", "scope_md": "8 frames.", "extra_price_minor": 3000, "new_due_date": due }),
    )
    .await;
    assert!(ok, "{text}");
    let changes: Vec<String> = sqlx::query_scalar(
        "select status from job_changes where job_id = $1::uuid order by number",
    )
    .bind(&job)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(changes, ["declined", "offered"]);

    // The job conversation: asked for, opened, written in, read, closed.
    let (ok, text) = tool(
        &app,
        &toms_agent,
        mcp,
        "ask_for_job_conversation",
        json!({ "id": job, "note": "About the colours." }),
    )
    .await;
    assert!(ok, "{text}");
    let (ok, text) = tool(
        &app,
        &agent,
        mcp,
        "open_job_conversation",
        json!({ "id": job }),
    )
    .await;
    assert!(ok, "{text}");
    let (ok, text) = tool(
        &app,
        &agent,
        mcp,
        "send_job_message",
        json!({ "id": job, "body": "I'm Vera's assistant: which palette?", "recording_ack": true }),
    )
    .await;
    assert!(ok, "{text}");
    let (ok, text) = tool(
        &app,
        &toms_agent,
        mcp,
        "read_job_conversation",
        json!({ "id": job }),
    )
    .await;
    assert!(ok && text.contains("which palette?"), "{text}");
    let (ok, text) = tool(
        &app,
        &agent,
        mcp,
        "close_job_conversation",
        json!({ "id": job }),
    )
    .await;
    assert!(ok, "{text}");
    let (open, closed): (bool, bool) = sqlx::query_as(
        "select conversation_open, conversation_closed_at is not null from jobs where id = $1::uuid",
    )
    .bind(&job)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((open, closed), (false, true));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_makers_agent_reads_and_updates_bug_reports_only_on_its_own_apps(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    let (rival_id, _) = app.user("Rival Maker", "US").await;
    let (pat_id, pat) = app.user("Pat Player", "US").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    listed_with(&app, maker_id, "castle", "released", "").await;
    listed_with(&app, rival_id, "rivals", "released", "").await;
    for slug in ["tower", "castle", "rivals"] {
        played(&app, pat_id, slug).await;
    }
    let mine = file_bug(&app, &pat, "tower", "Tower crash").await;
    let castle = file_bug(&app, &pat, "castle", "Castle crash").await;
    let theirs = file_bug(&app, &pat, "rivals", "Rival crash").await;
    let mcp = "/v1/maker/mcp";
    let agent = maker_token(
        &app,
        &maker,
        json!({ "name": "Claude", "scope": "publish" }),
    )
    .await;
    let tower_only = maker_token(
        &app,
        &maker,
        json!({ "name": "Tower", "scope": "publish", "apps": ["tower"] }),
    )
    .await;
    let reader = maker_token(&app, &maker, json!({ "name": "Reader", "scope": "read" })).await;

    let (ok, text) = tool(
        &app,
        &agent,
        mcp,
        "list_bug_reports",
        json!({ "slug": "tower" }),
    )
    .await;
    assert!(ok && text.contains("panic at main.rs:1"), "{text}");
    let (ok, _) = tool(
        &app,
        &agent,
        mcp,
        "list_bug_reports",
        json!({ "slug": "rivals" }),
    )
    .await;
    assert!(!ok, "another maker's app");
    let (ok, _) = tool(
        &app,
        &agent,
        mcp,
        "update_bug_report",
        json!({ "id": theirs, "status": "wont_fix" }),
    )
    .await;
    assert!(!ok);
    let (ok, _) = tool(
        &app,
        &reader,
        mcp,
        "update_bug_report",
        json!({ "id": mine, "status": "fixed" }),
    )
    .await;
    assert!(!ok, "a read token changes nothing");
    let (ok, text) = tool(
        &app,
        &reader,
        mcp,
        "release_feedback",
        json!({ "slug": "tower" }),
    )
    .await;
    assert!(ok, "{text}");
    let (ok, text) = tool(
        &app,
        &agent,
        mcp,
        "update_bug_report",
        json!({ "id": mine, "status": "fixed", "maker_note": "Fixed in 1.0.1." }),
    )
    .await;
    assert!(ok, "{text}");

    // A token for the tower only: its reports, not the castle's.
    let get = |bearer: &str, id: &str| {
        req(Method::GET, format!("/v1/bug-reports/{id}"))
            .header("authorization", bearer.to_string())
    };
    assert_eq!(
        app.send(get(&tower_only, &mine)).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        app.send(get(&tower_only, &castle)).await.status(),
        StatusCode::NOT_FOUND
    );
    let (ok, _) = tool(
        &app,
        &tower_only,
        mcp,
        "list_bug_reports",
        json!({ "slug": "castle" }),
    )
    .await;
    assert!(!ok);
    let (ok, _) = tool(
        &app,
        &tower_only,
        mcp,
        "update_bug_report",
        json!({ "id": castle, "status": "fixed" }),
    )
    .await;
    assert!(!ok);
    let statuses: Vec<String> = sqlx::query_scalar("select status from bug_reports order by title")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(statuses, ["open", "open", "fixed"], "Castle, Rival, Tower");
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_token_for_some_apps_updates_bug_reports_on_those_apps(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (maker_id, maker) = app.user("Maker", "US").await;
    let (pat_id, pat) = app.user("Pat Player", "US").await;
    listed_with(&app, maker_id, "tower", "released", "").await;
    played(&app, pat_id, "tower").await;
    let id = file_bug(&app, &pat, "tower", "Tower crash").await;
    let tower_only = maker_token(
        &app,
        &maker,
        json!({ "name": "Tower", "scope": "publish", "apps": ["tower"] }),
    )
    .await;
    let (ok, text) = tool(
        &app,
        &tower_only,
        "/v1/maker/mcp",
        "update_bug_report",
        json!({ "id": id, "status": "fixed", "maker_note": "Fixed in 1.0.1." }),
    )
    .await;
    assert!(ok, "the token's own app: {text}");
    let status: String = sqlx::query_scalar("select status from bug_reports")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "fixed");
}

// ---------------------------------------------------------------------------------------------
// Admin agents: People tools

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn admin_agents_decide_portfolios_and_fix_profiles_and_teams_within_their_tokens(db: PgPool) {
    let app = test_app_with(db, |c| {
        c.payments.simulated = true;
        chat_limits(c);
    })
    .await;
    let (_, owner) = make_admin(&app, "Olive Owner").await;
    let (_, reviewer) = staff_with_role(&app, "Rex Reviewer", "reviewer").await;
    let (mia_id, mia) = app.user("Mia Maker", "US").await;
    sqlx::query(
        "insert into creatives (user_id, handle, name, headline, ai_use, trader)
         values ($1, 'mia', 'Mia', 'The real Studio Ghibli', 'none', false)",
    )
    .bind(mia_id)
    .execute(&app.db)
    .await
    .unwrap();
    let pending_set = || {
        let db = app.db.clone();
        async move {
            let id = Uuid::now_v7();
            sqlx::query(
                "insert into portfolio_sets (id, creative_id, repo_url, commit_sha, works, fingerprint, status, source)
                 values ($1, $2, 'https://github.com/x/y', repeat('c', 40), '[]', $3, 'pending', 'sim')",
            )
            .bind(id)
            .bind(mia_id)
            .bind(id.to_string())
            .execute(&db)
            .await
            .unwrap();
            id
        }
    };
    make_team(
        &app,
        &mia,
        "mias-crew",
        json!({ "name": "Official Valve Team" }),
    )
    .await;
    let mut tokens = std::collections::HashMap::new();
    for scope in ["read", "review", "moderate"] {
        let res = app
            .send(post(
                "/v1/admin/tokens",
                &owner,
                json!({ "name": format!("Claude {scope}"), "scope": scope, "days": 7 }),
            ))
            .await;
        assert_eq!(res.status(), StatusCode::CREATED);
        tokens.insert(
            scope,
            format!("Bearer {}", body_json(res).await["token"].as_str().unwrap()),
        );
    }
    // Rex reviews, and may make tokens (security), but doesn't moderate.
    sqlx::query("insert into staff_roles (user_id, role, granted_by) select user_id, 'security', 'test' from staff_roles where role = 'reviewer'")
        .execute(&app.db)
        .await
        .unwrap();
    let res = app
        .send(post(
            "/v1/admin/tokens",
            &reviewer,
            json!({ "name": "Rex's agent", "scope": "moderate", "days": 7 }),
        ))
        .await;
    let rex = format!("Bearer {}", body_json(res).await["token"].as_str().unwrap());
    let mcp = "/v1/admin/mcp";

    // Portfolios: the queue shows it; a review token decides; a read token can't.
    let first = pending_set().await;
    let (ok, text) = tool(
        &app,
        &tokens["read"],
        mcp,
        "review_queue",
        json!({ "kind": "portfolio" }),
    )
    .await;
    assert!(ok && text.contains(&first.to_string()), "{text}");
    let (ok, _) = tool(
        &app,
        &tokens["read"],
        mcp,
        "decide_portfolio",
        json!({ "id": first, "decision": "approve" }),
    )
    .await;
    assert!(!ok);
    let (ok, _) = tool(
        &app,
        &tokens["review"],
        mcp,
        "decide_portfolio",
        json!({ "id": first, "decision": "reject" }),
    )
    .await;
    assert!(!ok, "turning one down needs a reason");
    let (ok, text) = tool(
        &app,
        &tokens["review"],
        mcp,
        "decide_portfolio",
        json!({ "id": first, "decision": "reject", "reason": "These works aren't in the repository." }),
    )
    .await;
    assert!(ok, "{text}");
    let second = pending_set().await;
    let (ok, text) = tool(
        &app,
        &tokens["review"],
        mcp,
        "decide_portfolio",
        json!({ "id": second, "decision": "approve" }),
    )
    .await;
    assert!(ok, "{text}");
    let sets: Vec<(Uuid, String, Option<String>)> = sqlx::query_as(
        "select id, status, reviewed_by from portfolio_sets where creative_id = $1 order by id",
    )
    .bind(mia_id)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(sets[0].1, "rejected");
    assert_eq!(sets[1].1, "approved");
    assert!(
        sets[1].2.as_deref().unwrap().contains("Claude review"),
        "{:?}",
        sets[1].2
    );
    let live: Option<Uuid> =
        sqlx::query_scalar("select portfolio_set_id from creatives where user_id = $1")
            .bind(mia_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(live, Some(second));

    // Profiles and teams: a review or moderate token may fix; a read token may not look.
    let (ok, _) = tool(
        &app,
        &tokens["read"],
        mcp,
        "user_profile",
        json!({ "id": mia_id }),
    )
    .await;
    assert!(!ok, "reading an account's details is moderation");
    let (ok, text) = tool(
        &app,
        &tokens["review"],
        mcp,
        "user_profile",
        json!({ "id": mia_id }),
    )
    .await;
    assert!(ok && text.contains("The real Studio Ghibli"), "{text}");
    let fix = json!({ "id": mia_id, "creative": { "headline": "Pixel artist" }, "reason": "Impersonation." });
    let (ok, _) = tool(&app, &tokens["read"], mcp, "edit_profile", fix.clone()).await;
    assert!(!ok);
    let (ok, _) = tool(&app, &rex, mcp, "edit_profile", fix.clone()).await;
    assert!(!ok, "a reviewer's token is capped by the reviewer's role");
    let (ok, text) = tool(&app, &tokens["review"], mcp, "edit_profile", fix).await;
    assert!(ok, "{text}");
    let team_fix = json!({ "slug": "mias-crew", "name": "Mia's Crew", "reason": "Not Valve." });
    let (ok, _) = tool(&app, &tokens["read"], mcp, "edit_team", team_fix.clone()).await;
    assert!(!ok);
    let (ok, text) = tool(&app, &tokens["moderate"], mcp, "edit_team", team_fix).await;
    assert!(ok, "{text}");
    let (headline, team): (String, String) = sqlx::query_as(
        "select (select headline from creatives where user_id = $1), (select name from teams where slug = 'mias-crew')",
    )
    .bind(mia_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        (headline.as_str(), team.as_str()),
        ("Pixel artist", "Mia's Crew")
    );
    let actors: Vec<String> = sqlx::query_scalar(
        "select actor from audit_log where action in ('user.profile_edited', 'team.edited') order by id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(actors.len(), 2);
    assert!(
        actors[0].contains("Claude review") && actors[1].contains("Claude moderate"),
        "{actors:?}"
    );

    // Notes on a People problem, for a person to decide.
    let (_, vera) = live_creative(&app, "vera", "US", false).await;
    let (tom_id, tom) = app.user("Tom Client", "US").await;
    listed_with(&app, tom_id, "toms-game", "released", "").await;
    let job = hire(&app, &tom, "vera", "toms-game").await;
    app.send(post(
        format!("/v1/jobs/{job}/offer"),
        &vera,
        sprite_offer("upfront"),
    ))
    .await;
    app.send(post(
        format!("/v1/jobs/{job}/accept"),
        &tom,
        json!({ "offer_count": 1, "exact_terms_ack": true, "start_now": true }),
    ))
    .await;
    app.send(post(format!("/v1/jobs/{job}/pay"), &tom, json!({})))
        .await;
    let res = app
        .send(post(
            format!("/v1/jobs/{job}/problem"),
            &tom,
            json!({ "statement": "Nothing delivered after the due date." }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{}", body_json(res).await);
    let problem: Uuid = sqlx::query_scalar("select id from job_problems")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let (ok, text) = tool(
        &app,
        &tokens["read"],
        mcp,
        "add_note",
        json!({ "kind": "problem", "id": problem, "verdict": "concern", "note": "The record shows no delivery." }),
    )
    .await;
    assert!(ok, "{text}");
    let (ok, _) = tool(
        &app,
        &tokens["read"],
        mcp,
        "add_note",
        json!({ "kind": "problem", "id": Uuid::now_v7(), "note": "Nothing there." }),
    )
    .await;
    assert!(!ok);
    let (ok, _) = tool(
        &app,
        &tokens["read"],
        mcp,
        "add_note",
        json!({ "kind": "problem", "id": problem, "verdict": "guilty", "note": "x" }),
    )
    .await;
    assert!(!ok, "pass, concern or unsure");
    let notes: Vec<(String, Option<String>, String)> = sqlx::query_as(
        "select subject_kind, verdict, author from review_notes where subject_id = $1",
    )
    .bind(problem)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(
        (notes[0].0.as_str(), notes[0].1.as_deref()),
        ("problem", Some("concern"))
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
#[ignore = "new-1: anyone who joins an open team can message every member (teens too) in its team chat; needs a product decision"]
async fn joining_an_open_team_is_not_a_way_to_message_its_members(db: PgPool) {
    let app = test_app_with(db, chat_limits).await;
    let (_, owner_web) = app.user("Tess Owner", "US").await;
    let (teen_id, teen_web) = app.user("Tia Teen", "GB").await;
    let (_, stranger_web) = app.user("Sam Stranger", "US").await;
    make_team(
        &app,
        &owner_web,
        "open-crew",
        json!({ "join_policy": "open" }),
    )
    .await;
    add_to_team(&app, "open-crew", teen_id, "member").await;
    let owner = first_device(&app, &owner_web).await;
    let teen = first_device(&app, &teen_web).await;
    chat_id(&app, &owner, json!({ "kind": "team", "team": "open-crew" })).await;
    // Neither may message the teen directly (friends.rs connected: open teams aren't connections).
    assert_eq!(
        connection(&app, &stranger_web, teen_id).await["can_message"],
        false
    );
    assert_eq!(
        connection(&app, &owner_web, teen_id).await["can_message"],
        false
    );

    // A stranger joins and writes in the team chat: that reaches the teen all the same.
    app.send(req(Method::POST, "/v1/teams/open-crew/join").cookie(&stranger_web))
        .await;
    let stranger = first_device(&app, &stranger_web).await;
    let (status, team) = start_chat(
        &app,
        &stranger,
        json!({ "kind": "team", "team": "open-crew" }),
    )
    .await;
    let refused = !status.is_success()
        || send_message(
            &app,
            &stranger,
            team["id"].as_str().unwrap().parse().unwrap(),
            "hi, how old are you?",
        )
        .await
        .status()
            != StatusCode::OK;
    assert!(
        refused,
        "a stranger reached a teen through an open team's chat"
    );
    assert!(
        sync_now(&app, &teen).await["events"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
