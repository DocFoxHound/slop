// SPDX-License-Identifier: AGPL-3.0-or-later
//! Releases on hold and failed releases a maker asks about (holds.rs): what holds them in plain
//! words, the conversation with SLAPorSLOP staff, running the scans again on the same files,
//! Microsoft Defender's check arriving late, the staff queue and feed, and makers putting their
//! own drafts in the store (manage.rs `list_app`). Also the startup settle for releases left
//! waiting by the old flow (publish.rs), and what the account export and closing do with the
//! conversations (account.rs).

use crate::*;

/// A reviewer (staff with the reviewer role). Returns (ID, cookie).
async fn reviewer(app: &TestApp, name: &str) -> (Uuid, String) {
    let (id, cookie) = app.user(name, "US").await;
    sqlx::query("update users set is_admin = true where id = $1")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "insert into staff_roles (user_id, role, granted_by) values ($1, 'reviewer', 'test')",
    )
    .bind(id)
    .execute(&app.db)
    .await
    .unwrap();
    (id, cookie)
}

async fn hold(app: &TestApp, cookie: &str, id: &str) -> Value {
    let res = app
        .send(req(Method::GET, format!("/v1/apps/game/releases/{id}/hold")).cookie(cookie))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    body_json(res).await
}

fn write(cookie: &str, id: &str, body: &str) -> Req {
    req(
        Method::POST,
        format!("/v1/apps/game/releases/{id}/hold/messages"),
    )
    .cookie(cookie)
    .json(json!({ "body": body }))
}

async fn notices(app: &TestApp, user: Uuid) -> Vec<(String, String)> {
    sqlx::query_as("select title, body from notifications where user_id = $1 order by created_at")
        .bind(user)
        .fetch_all(&app.db)
        .await
        .unwrap()
}

async fn queue(app: &TestApp, cookie: &str) -> Value {
    body_json(
        app.send(req(Method::GET, "/v1/admin/queue").cookie(cookie))
            .await,
    )
    .await
}

/// A scanner stand-in that holds every file while `mode` says "held" (with a ClamAV finding),
/// and passes them otherwise.
fn mode_scanner(dir: &std::path::Path) -> (String, std::path::PathBuf) {
    std::fs::create_dir_all(dir).unwrap();
    let mode = dir.join("mode");
    std::fs::write(&mode, "held").unwrap();
    let script = dir.join("slop-scan");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nif grep -q held {mode}; then echo '{{\"result\":\"on_hold\",\"detail\":\"Flagged by ClamAV: Win.Test in game.exe. A person checks it before it can be approved; it may be a false positive.\",\"findings\":[{{\"engine\":\"ClamAV\",\"file\":\"bin/game.exe\",\"name\":\"Win.Trojan.Test\"}}],\"capabilities\":[],\"engines\":{{}}}}'; else echo '{{\"result\":\"passed\",\"detail\":\"nothing found\",\"findings\":[],\"capabilities\":[],\"engines\":{{}}}}'; fi\n",
            mode = mode.display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    (script.to_str().unwrap().to_string(), mode)
}

/// Microsoft Defender's clean check of release `id`'s Windows file, as Slop Check would attach
/// it, then its checks end again.
async fn defender_clean(app: &TestApp, id: &str) {
    sqlx::query(
        "update release_files set defender = jsonb_build_object('sha256', sha256, 'engine', '1.1',
                'signatures', '1.4', 'detections', '[]'::jsonb)
         where release_id = $1::uuid",
    )
    .bind(id)
    .execute(&app.db)
    .await
    .unwrap();
    slop_api::publish::finish_checks(
        &app.db,
        admin::ReviewRules::from_config(&app.config),
        id.parse().unwrap(),
        "slop",
    )
    .await
    .unwrap();
}

/// A held release says what holds it and what its maker can do; the maker writes to staff (not
/// more than 5 times a day per release, plain text, few links), staff are told without the
/// maker's words, see them as untrusted text in the queue and answer; accepting with a note
/// clears the file and keeps the note.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_held_release_explains_itself_and_its_maker_talks_to_staff(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "beta").await;
    let (staff_id, staff) = reviewer(&app, "Rita Reviewer").await;
    let (_, stranger) = app.user("Stranger", "US").await;
    let id = submit_and_fetch(&app, &maker, "1.0.0").await;
    sqlx::query(
        "update release_files set scan_report = '{\"findings\": [{\"engine\": \"ClamAV\", \"name\": \"Win.Trojan.Test\", \"file\": \"bin/game.exe\"},
                                                                  {\"engine\": \"YARA\", \"name\": \"Packed_UPX\", \"file\": \"bin/game.exe\"}],
                                                    \"capabilities\": [], \"engines\": {}}'::jsonb
         where release_id = $1::uuid",
    )
    .bind(&id)
    .execute(&app.db)
    .await
    .unwrap();
    flag_release(&app, &id).await;
    // Staff were told, without anything the maker wrote.
    let told = notices(&app, staff_id).await;
    assert_eq!(
        told,
        [(
            "Releases on hold".to_string(),
            "Newest: Game 1.0.0 (a scanner finding).".to_string()
        )]
    );

    let h = hold(&app, &maker, &id).await;
    assert_eq!(h["status"], "open");
    assert_eq!(h["kinds"], json!(["scan"]));
    assert_eq!(h["needs_staff"], true);
    assert_eq!(h["problems"][0]["kind"], "scan");
    assert_eq!(h["problems"][0]["file_label"], "Windows (.zip)");
    assert_eq!(
        h["problems"][0]["what"],
        "A scanner flagged the Windows (.zip) build"
    );
    assert_eq!(
        h["problems"][0]["explained"],
        "ClamAV, a virus scanner, matched the signature `Win.Trojan.Test` in `bin/game.exe`. Scanners sometimes match harmless programs. A YARA rule (`Packed_UPX`) matched `bin/game.exe`. These rules look for patterns seen in malware. They also match packed programs, for example ones compressed with UPX."
    );
    // The test server doesn't scan, so there's nothing to run again.
    assert_eq!(
        h["you_can"],
        json!(["write_to_staff", "new_release", "withdraw"])
    );
    assert_eq!(h["rescans_left"], 3);
    assert_eq!(h["messages"][0]["from"], "slaporslop");
    assert_eq!(
        h["messages"][0]["body"],
        "On hold: a scanner flagged the Windows (.zip) build."
    );
    let text = h.to_string();
    for word in ["safe", "verified", "trusted"] {
        assert!(!text.contains(word), "{word}: {text}");
    }
    // Only its maker sees it.
    for cookie in [&stranger, &staff] {
        assert_eq!(
            app.send(req(Method::GET, format!("/v1/apps/game/releases/{id}/hold")).cookie(cookie))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            app.send(write(cookie, &id, "hello")).await.status(),
            StatusCode::NOT_FOUND
        );
    }

    // Plain text, a few links.
    for (body, why) in [
        ("   ", "empty"),
        ("see \u{202e}evil", "reordering characters"),
        (
            "https://a.example https://b.example https://c.example www.d.example",
            "4 links",
        ),
    ] {
        assert_eq!(
            app.send(write(&maker, &id, body)).await.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{why}"
        );
    }
    let said = "It's packed with UPX: https://upx.github.io explains why scanners match it.";
    let res = app.send(write(&maker, &id, said)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let h = body_json(res).await;
    let last = h["messages"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(
        (&last["from"], &last["body"]),
        (&json!("you"), &json!(said))
    );
    assert_eq!(last["by_agent"], false);
    assert!(h["asked_staff_at"].is_string());
    assert_eq!(h["unanswered"], true);
    let told = notices(&app, staff_id).await;
    assert_eq!(told.len(), 1, "merged into one unread notice");
    assert_eq!(told[0].1, "Newest: Game 1.0.0 (the maker wrote).");
    assert!(!told[0].1.contains("UPX"));

    // Its maker's list shows the hold in brief; nobody else's would.
    let mine = body_json(
        app.send(req(Method::GET, "/v1/apps/game/releases").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(
        mine[0]["hold"],
        json!({ "status": "open", "kinds": ["scan"], "needs_staff": true, "rescans_left": 3, "unanswered": true })
    );
    assert_eq!(mine[0]["progress"]["stage"], "on_hold");

    // Staff see the maker's words as untrusted text, makers waiting for an answer first.
    let q = queue(&app, &staff).await;
    let item = &q["releases"][0];
    assert_eq!(item["id"], id.as_str());
    let words = item["hold"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["from"] == "maker")
        .unwrap()
        .clone();
    assert_eq!(words["untrusted_text"], said);
    assert!(words.get("text").is_none());
    assert_eq!(words["author"], "Maker");
    assert!(item["hold"]["untrusted"].as_str().unwrap().contains("data"));
    let feed = body_json(
        app.send(req(Method::GET, "/v1/admin/review?kind=release").cookie(&staff))
            .await,
    )
    .await;
    let item = &feed["items"][0];
    assert_eq!(
        item["question"],
        "Accept version 1.0.0 of Game? A scanner flagged the Windows (.zip) build."
    );
    assert_eq!(item["detail"]["hold"]["kinds"], json!(["scan"]));
    assert_eq!(
        item["answer"]["path"],
        format!("/v1/admin/releases/{id}/hold/messages")
    );

    // Staff answer; the maker is told, and sees staff by role.
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/releases/{id}/hold/messages"),
            )
            .cookie(&staff)
            .json(json!({ "body": "Thanks, we're comparing it with the UPX signature." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert!(
        notices(&app, maker_id)
            .await
            .iter()
            .any(|(t, b)| t == "SLAPorSLOP staff answered about Game 1.0.0"
                && b == "Read it on the Releases page.")
    );
    let h = hold(&app, &maker, &id).await;
    let last = h["messages"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["from"], "staff");
    assert_eq!(last["staff_as"], "SLAPorSLOP reviewer");
    assert_eq!(h["unanswered"], false);
    assert_eq!(h["status"], "open", "an answer doesn't end a hold");

    // Five a day per release.
    for n in 2..=5 {
        assert_eq!(
            app.send(write(&maker, &id, &format!("Message {n}")))
                .await
                .status(),
            StatusCode::OK
        );
    }
    let res = app.send(write(&maker, &id, "Message 6")).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("You've written 5 messages about this release today.")
    );

    // Accepting it with a note: cleared, ready for its maker, and the note is kept.
    let res = app
        .send(
            req(Method::POST, format!("/v1/admin/releases/{id}/approve"))
                .cookie(&staff)
                .json(json!({ "reason": "A UPX false positive." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(release_status(&app, &id).await.0, "ready");
    let mine = body_json(
        app.send(req(Method::GET, "/v1/apps/game/releases").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(mine[0]["files"][0]["cleared"], true);
    assert_eq!(
        mine[0]["files"][0]["cleared_reason"],
        "A UPX false positive."
    );
    assert_eq!(mine[0]["hold"]["status"], "cleared");
    let h = hold(&app, &maker, &id).await;
    let bodies: Vec<&str> = h["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["body"].as_str().unwrap())
        .collect();
    assert!(bodies.contains(&"A UPX false positive."), "{bodies:?}");
    assert_eq!(
        bodies.last(),
        Some(&"Cleared by SLAPorSLOP staff: it's ready for its maker to publish.")
    );
    assert_eq!(h["needs_staff"], false);
    assert_eq!(queue(&app, &staff).await["releases"], json!([]));
    // A closed hold takes no more messages.
    assert_eq!(
        app.send(write(&maker, &id, "One more thing"))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    // Published, players see the release but never its hold or the note.
    approve_release(&app, lax(), id.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(
        app.send(req(Method::POST, "/v1/apps/game/list").cookie(&maker))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let public = body_json(
        app.send(req(Method::GET, "/v1/apps/game/releases").cookie(&stranger))
            .await,
    )
    .await;
    assert_eq!(public[0]["id"], id.as_str());
    assert!(public[0].get("hold").is_none(), "{}", public[0]);
    assert!(public[0]["files"][0]["cleared_reason"].is_null());
}

/// A failed release offers "ask SLAPorSLOP staff" (CON-16, ruling 7): a question of its own in
/// the staff queue, closed by an answer, and the maker may ask again.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_failed_release_can_be_asked_about(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "beta").await;
    let (_, staff) = reviewer(&app, "Rita Reviewer").await;
    let mut body = release_body(&app, "1.0.0", "x.zip");
    body["files"][0]["url"] = json!(format!("{}octo/game/missing/x.zip", app.fake_base));
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases")
                .cookie(&maker)
                .json(body),
        )
        .await;
    let id = body_json(res).await["id"].as_str().unwrap().to_string();
    releases::fetch_next(&app.state).await.unwrap();
    assert_eq!(release_status(&app, &id).await.0, "failed");
    let h = hold(&app, &maker, &id).await;
    assert_eq!(h["status"], Value::Null);
    assert_eq!(h["problems"][0]["kind"], "failed");
    assert!(
        h["problems"][0]["explained"]
            .as_str()
            .unwrap()
            .contains("404")
    );
    assert_eq!(h["you_can"], json!(["new_release", "write_to_staff"]));

    let res = app
        .send(write(
            &maker,
            &id,
            "The file is there now: was it a hiccup?",
        ))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let h = body_json(res).await;
    assert_eq!(
        (&h["status"], &h["kinds"], &h["needs_staff"]),
        (&json!("open"), &json!(["question"]), &json!(true))
    );
    let feed = body_json(
        app.send(req(Method::GET, "/v1/admin/review?kind=release").cookie(&staff))
            .await,
    )
    .await;
    let item = &feed["items"][0];
    assert!(
        item["question"].as_str().unwrap().starts_with(
            "Answer the maker's question about version 1.0.0 of Game: its checks failed"
        ),
        "{item}"
    );
    assert!(item.get("approve").is_none() && item.get("reject").is_none());
    let answer = |text: &str| {
        req(
            Method::POST,
            format!("/v1/admin/releases/{id}/hold/messages"),
        )
        .cookie(&staff)
        .json(json!({ "body": text }))
    };
    assert_eq!(
        app.send(answer("Yes: send it again as 1.0.1."))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let h = hold(&app, &maker, &id).await;
    assert_eq!(h["status"], "answered");
    assert_eq!(h["needs_staff"], false);
    assert!(
        notices(&app, maker_id)
            .await
            .iter()
            .any(|(t, _)| t == "SLAPorSLOP staff answered about Game 1.0.0")
    );
    assert_eq!(
        app.send(answer("And another thing")).await.status(),
        StatusCode::CONFLICT,
        "answered: nothing open"
    );
    // Asking again opens it again.
    assert_eq!(
        app.send(write(&maker, &id, "Thanks: one more question."))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(hold(&app, &maker, &id).await["status"], "open");
    let pending = admin::pending(&app.db).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].kinds, ["question"]);
    assert!(pending[0].unanswered);
    // A release that passed has nothing to ask about, and passing closes the older question.
    let ok = submit_and_fetch(&app, &maker, "1.0.1").await;
    assert_eq!(
        app.send(write(&maker, &ok, "Hello")).await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(hold(&app, &maker, &id).await["status"], "replaced");
    assert!(admin::pending(&app.db).await.unwrap().is_empty());
}

/// A lowered age rating is a staff hold (ruling 2a): the maker reads exactly why, staff see the
/// ages and labels, and running the scans again doesn't apply.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_lowered_rating_is_explained_to_both_sides(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "released").await;
    let (_, staff) = reviewer(&app, "Rita Reviewer").await;
    let submit = |version: &str, rating: Value| {
        let mut body = release_body(&app, version, "game-windows.zip");
        body["rating"] = rating;
        req(Method::POST, "/v1/apps/game/releases")
            .cookie(&maker)
            .json(body)
    };
    let res = app
        .send(submit(
            "1.0.0",
            json!({ "min_age": 16, "content_labels": ["violence", "horror"] }),
        ))
        .await;
    let first = body_json(res).await["id"].as_str().unwrap().to_string();
    releases::fetch_next(&app.state).await.unwrap();
    approve_release(&app, lax(), first.parse().unwrap())
        .await
        .unwrap();
    let res = app
        .send(submit(
            "1.1.0",
            json!({ "min_age": 12, "content_labels": ["horror", "romance"] }),
        ))
        .await;
    let id = body_json(res).await["id"].as_str().unwrap().to_string();
    releases::fetch_next(&app.state).await.unwrap();
    assert_eq!(release_status(&app, &id).await.0, "awaiting_review");
    let h = hold(&app, &maker, &id).await;
    assert_eq!(h["kinds"], json!(["rating_lowered"]));
    assert_eq!(
        h["problems"][0]["explained"],
        "Lowers the age rating from 16+ to 12+ (drops: violence). A SLAPorSLOP staff member checks a lowered rating before you can publish."
    );
    assert_eq!(
        h["problems"][0]["you_can"],
        json!(["write_to_staff", "new_release"])
    );
    let res = app
        .send(req(Method::POST, format!("/v1/apps/game/releases/{id}/rescan")).cookie(&maker))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let staff_hold = body_json(
        app.send(req(Method::GET, format!("/v1/admin/releases/{id}/hold")).cookie(&staff))
            .await,
    )
    .await;
    assert_eq!(
        staff_hold["rating"],
        json!({ "from_age": 16, "to_age": 12, "dropped_labels": ["violence"], "content_labels": ["horror", "romance"] })
    );
    let q = queue(&app, &staff).await;
    assert_eq!(q["releases"][0]["hold"]["rating"]["to_age"], 12);
}

/// "Run the scans again" downloads the same file, checks its SHA-256, scans it and deletes it,
/// up to 3 times, 6 hours after the last scan; a file that changed on the release page keeps
/// the hold, and a download that fails gives the run back.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_scans_run_again_on_the_same_files(db: PgPool) {
    let dir = std::env::temp_dir().join(format!("slop-holds-{}", Uuid::now_v7()));
    let (command, mode) = mode_scanner(&dir);
    let app = test_app_with(db, move |c| {
        c.releases.scanner = slop_api::scan::Scanner::from_setting("local", &command).unwrap();
    })
    .await;
    let (maker_id, maker) = maker_with_app(&app, "beta").await;
    let (_, staff) = reviewer(&app, "Rita Reviewer").await;
    let rescan =
        |id: &str| req(Method::POST, format!("/v1/apps/game/releases/{id}/rescan")).cookie(&maker);
    let id = submit_and_fetch(&app, &maker, "1.0.0").await;
    let h = hold(&app, &maker, &id).await;
    assert_eq!(h["kinds"], json!(["scan", "defender_missing"]));
    defender_clean(&app, &id).await;
    let h = hold(&app, &maker, &id).await;
    assert_eq!(h["kinds"], json!(["scan"]));
    assert!(h["you_can"].as_array().unwrap().contains(&json!("rescan")));
    assert!(h["next_rescan_at"].is_string(), "{h}");

    // Just scanned: not for 6 hours.
    let res = app.send(rescan(&id)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("Next re-run allowed at ")
    );
    sqlx::query(
        "update release_files set checked_at = now() - interval '7 hours' where release_id = $1::uuid",
    )
    .bind(&id)
    .execute(&app.db)
    .await
    .unwrap();
    let res = app.send(rescan(&id)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let h = body_json(res).await;
    assert_eq!(
        (&h["rescan_requested"], &h["rescans_left"]),
        (&json!(true), &json!(2))
    );
    assert_eq!(app.send(rescan(&id)).await.status(), StatusCode::CONFLICT);

    // Today's signatures find nothing: it's ready, its maker and staff are told.
    std::fs::write(&mode, "clean").unwrap();
    assert!(slop_api::holds::rescan_next(&app.state).await.unwrap());
    assert!(!slop_api::holds::rescan_next(&app.state).await.unwrap());
    assert_eq!(release_status(&app, &id).await.0, "ready");
    let h = hold(&app, &maker, &id).await;
    assert_eq!(h["status"], "passed");
    let line = h["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["detail"]["after"] == "passed")
        .unwrap()
        .clone();
    assert_eq!(
        line["body"],
        "Windows (.zip): scanned again with today's signatures: nothing found."
    );
    assert_eq!(line["detail"]["before"], "on_hold");
    let audited: i64 =
        sqlx::query_scalar("select count(*) from audit_log where action = 'release.rescan_held'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(audited, 1);
    assert!(
        notices(&app, maker_id)
            .await
            .iter()
            .any(|(t, _)| t == "Game 1.0.0 passed SLAPorSLOP's checks")
    );
    assert_eq!(queue(&app, &staff).await["releases"], json!([]));

    // 1.1.0 is held, and the file on the release page isn't the one checked any more.
    std::fs::write(&mode, "held").unwrap();
    let next = submit_and_fetch(&app, &maker, "1.1.0").await;
    defender_clean(&app, &next).await;
    sqlx::query(
        "update release_files set checked_at = now() - interval '7 hours', sha256 = repeat('f', 64),
                defender = jsonb_set(defender, '{sha256}', to_jsonb(repeat('f', 64)))
         where release_id = $1::uuid",
    )
    .bind(&next)
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(app.send(rescan(&next)).await.status(), StatusCode::OK);
    std::fs::write(&mode, "clean").unwrap();
    assert!(slop_api::holds::rescan_next(&app.state).await.unwrap());
    assert_eq!(release_status(&app, &next).await.0, "awaiting_review");
    let h = hold(&app, &maker, &next).await;
    assert_eq!(
        (&h["status"], &h["rescans_left"]),
        (&json!("open"), &json!(2))
    );
    let bodies: Vec<&str> = h["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["body"].as_str().unwrap())
        .collect();
    assert!(
        bodies.contains(&"Windows (.zip): the file on your release page isn't the one SLAPorSLOP checked (its SHA-256 changed). A changed build is a new version: submit it."),
        "{bodies:?}"
    );
    assert_eq!(bodies.last(), Some(&"Still on hold."));
    assert!(
        notices(&app, maker_id)
            .await
            .iter()
            .any(|(t, _)| t == "Game 1.1.0 couldn't be scanned again")
    );

    // No re-runs left.
    sqlx::query("update release_holds set rescans_used = 3 where release_id = $1::uuid")
        .bind(&next)
        .execute(&app.db)
        .await
        .unwrap();
    let res = app.send(rescan(&next)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("No re-runs left")
    );
    assert!(
        !hold(&app, &maker, &next).await["you_can"]
            .as_array()
            .unwrap()
            .contains(&json!("rescan"))
    );
    // A download that fails gives the run back.
    sqlx::query("update release_holds set rescans_used = 1 where release_id = $1::uuid")
        .bind(&next)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "update release_files set url = replace(url, 'releases/download', 'missing'),
                checked_at = now() - interval '7 hours'
         where release_id = $1::uuid",
    )
    .bind(&next)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query("update release_holds set last_rescan_at = null where release_id = $1::uuid")
        .bind(&next)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(app.send(rescan(&next)).await.status(), StatusCode::OK);
    assert!(slop_api::holds::rescan_next(&app.state).await.unwrap());
    let h = hold(&app, &maker, &next).await;
    assert_eq!(h["rescans_left"], 2, "{h}");
    // The attempt counts for the spacing all the same (B-9).
    let res = app.send(rescan(&next)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("Next re-run allowed at ")
    );
    // A run is given back once per hold: the next download that fails counts.
    let allow_now = || async {
        sqlx::query("update release_holds set last_rescan_at = null where release_id = $1::uuid")
            .bind(&next)
            .execute(&app.db)
            .await
            .unwrap();
    };
    allow_now().await;
    assert_eq!(app.send(rescan(&next)).await.status(), StatusCode::OK);
    assert!(slop_api::holds::rescan_next(&app.state).await.unwrap());
    let h = hold(&app, &maker, &next).await;
    assert_eq!(h["rescans_left"], 1, "{h}");
    let said: Vec<&str> = h["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["body"].as_str().unwrap())
        .collect();
    assert!(
        said.iter().any(|b| b
            .ends_with("This re-run counts: one that couldn't download was given back already.")),
        "{said:?}"
    );
    // SLAPorSLOP's day's share of re-runs is used up: a request waits for the next day.
    sqlx::query(
        "update release_files set url = replace(url, 'missing', 'releases/download') where release_id = $1::uuid",
    )
    .bind(&next)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query("update release_holds set rescans_used = 2 where release_id = $1::uuid")
        .bind(&next)
        .execute(&app.db)
        .await
        .unwrap();
    allow_now().await;
    assert_eq!(app.send(rescan(&next)).await.status(), StatusCode::OK);
    sqlx::query(
        "insert into release_rescan_runs (release_id) select $1::uuid from generate_series(1, 200)",
    )
    .bind(&next)
    .execute(&app.db)
    .await
    .unwrap();
    assert!(!slop_api::holds::rescan_next(&app.state).await.unwrap());
    sqlx::query("delete from release_rescan_runs")
        .execute(&app.db)
        .await
        .unwrap();
    // Staff cleared the file meanwhile: nothing is left to scan, so no run is spent and the maker
    // isn't told it was scanned again.
    sqlx::query("update release_files set cleared_at = now(), cleared_by = 'rev' where release_id = $1::uuid")
        .bind(&next)
        .execute(&app.db)
        .await
        .unwrap();
    assert!(slop_api::holds::rescan_next(&app.state).await.unwrap());
    let h = hold(&app, &maker, &next).await;
    assert_eq!(h["rescans_left"], 1, "{h}");
    assert_eq!(release_status(&app, &next).await.0, "ready");
    assert!(
        !notices(&app, maker_id)
            .await
            .iter()
            .any(|(t, _)| t == "Game 1.1.0 was scanned again")
    );
    // Someone else's release, or none: 404.
    let (_, stranger) = app.user("Stranger", "US").await;
    assert_eq!(
        app.send(
            req(
                Method::POST,
                format!("/v1/apps/game/releases/{next}/rescan")
            )
            .cookie(&stranger)
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A missing Defender check is the maker's (ruling 3) where Slop Check can run (GitHub): not in
/// the staff queue until they write, back with the maker once staff answer (B-7), and the release
/// goes on by itself when Slop Check reports.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_missing_defender_check_resumes_when_it_arrives(db: PgPool) {
    let dir = std::env::temp_dir().join(format!("slop-holds-{}", Uuid::now_v7()));
    let (command, mode) = mode_scanner(&dir);
    std::fs::write(&mode, "clean").unwrap();
    let app = test_app_with(db, move |c| {
        c.releases.scanner = slop_api::scan::Scanner::from_setting("local", &command).unwrap();
    })
    .await;
    let (maker_id, maker) = maker_with_app(&app, "beta").await;
    let (staff_id, staff) = reviewer(&app, "Rita Reviewer").await;
    let id = submit_and_fetch(&app, &maker, "1.0.0").await;
    // Fetched from the stand-in host; the app is on GitHub, where Slop Check runs.
    sqlx::query("update apps set repo_url = 'https://github.com/octo/game'")
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("delete from notifications where user_id = $1")
        .bind(staff_id)
        .execute(&app.db)
        .await
        .unwrap();
    slop_api::publish::finish_checks(
        &app.db,
        admin::ReviewRules::from_config(&app.config),
        id.parse().unwrap(),
        "slop",
    )
    .await
    .unwrap();
    let h = hold(&app, &maker, &id).await;
    assert_eq!(
        (&h["kinds"], &h["needs_staff"]),
        (&json!(["defender_missing"]), &json!(false))
    );
    assert_eq!(
        h["problems"][0]["you_can"],
        json!(["run_slop_check", "write_to_staff"])
    );
    assert!(
        h["problems"][0]["explained"]
            .as_str()
            .unwrap()
            .contains("This release goes on by itself as soon as the result arrives.")
    );
    assert_eq!(queue(&app, &staff).await["releases"], json!([]));
    assert!(notices(&app, staff_id).await.is_empty());
    let steps = body_json(
        app.send(req(Method::GET, "/v1/me/next-steps").cookie(&maker))
            .await,
    )
    .await;
    let step = steps["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["what"].as_str().unwrap().contains("1.0.0"))
        .unwrap()
        .clone();
    assert_eq!(step["who"], "you");
    assert_eq!(
        step["what"],
        "Game 1.0.0 is on hold: the Windows (.zip) build needs Microsoft Defender's check. The Releases page says what you can do."
    );

    // Writing to staff makes it theirs.
    assert_eq!(
        app.send(write(
            &maker,
            &id,
            "Slop Check can't run on our CI: can you look?"
        ))
        .await
        .status(),
        StatusCode::OK
    );
    let q = queue(&app, &staff).await;
    assert_eq!(q["releases"][0]["id"], id.as_str());
    let steps = body_json(
        app.send(req(Method::GET, "/v1/me/next-steps").cookie(&maker))
            .await,
    )
    .await;
    assert!(steps["steps"].as_array().unwrap().iter().any(|s| s["what"]
        == "SLAPorSLOP staff are looking at Game 1.0.0's hold."
        && s["who"] == "slaporslop"));
    // Staff answer: getting the check is the maker's again, so it leaves the queue.
    let answer = req(
        Method::POST,
        format!("/v1/admin/releases/{id}/hold/messages"),
    )
    .cookie(&staff)
    .json(json!({ "body": "Slop Check runs on GitHub's own runners: add check.yml." }));
    assert_eq!(app.send(answer).await.status(), StatusCode::NO_CONTENT);
    assert_eq!(hold(&app, &maker, &id).await["needs_staff"], false);
    assert_eq!(queue(&app, &staff).await["releases"], json!([]));
    // Its checks ending again don't put it back while staff answered last.
    slop_api::publish::finish_checks(
        &app.db,
        admin::ReviewRules::from_config(&app.config),
        id.parse().unwrap(),
        "slop",
    )
    .await
    .unwrap();
    assert_eq!(hold(&app, &maker, &id).await["needs_staff"], false);

    // Slop Check reports a clean result: it goes on by itself.
    let sha: String =
        sqlx::query_scalar("select sha256 from release_files where release_id = $1::uuid")
            .bind(&id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    let check_ref = RECIPE_REF.replace("build.yml", "check.yml");
    let res = app
        .send(
            req(Method::POST, "/v1/slop-build/checks")
                .header(
                    "authorization",
                    format!(
                        "Bearer {}",
                        github_token(&app, json!({ "job_workflow_ref": check_ref }))
                    ),
                )
                .json(json!({ "app": "game", "files": [{
                    "sha256": sha, "engine": "1.1.25090.3001", "signatures": "1.461.119.0",
                    "detections": [], "signed_by": null
                }] })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(release_status(&app, &id).await.0, "ready");
    assert_eq!(hold(&app, &maker, &id).await["status"], "passed");
    assert!(
        notices(&app, maker_id)
            .await
            .iter()
            .any(|(t, _)| t == "Game 1.0.0 passed SLAPorSLOP's checks")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Makers put their own drafts in the store (SPEC §7): a person signed in, not an agent; the
/// same checks as staff listing, said for the maker; staff keep appeal relists and going public.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn makers_put_their_own_drafts_in_the_store(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "beta").await;
    let (_, staff) = reviewer(&app, "Rita Reviewer").await;
    let (_, stranger) = app.user("Stranger", "US").await;
    let list = |cookie: &str| req(Method::POST, "/v1/apps/game/list").cookie(cookie);
    assert_eq!(
        app.send(list(&stranger)).await.status(),
        StatusCode::NOT_FOUND
    );
    let res = app.send(list(&maker)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(res).await["error"]["message"],
        "A Beta app needs a published release first: submit one on the Releases page and publish it once it passes the checks. A Coming soon page can go in without one."
    );
    // A ready release isn't enough: it has to be published.
    let id = submit_and_fetch(&app, &maker, "1.0.0").await;
    assert_eq!(app.send(list(&maker)).await.status(), StatusCode::CONFLICT);
    let steps = body_json(
        app.send(req(Method::GET, "/v1/me/next-steps").cookie(&maker))
            .await,
    )
    .await;
    assert!(
        !steps
            .to_string()
            .contains("in the store from its Manage page"),
        "{steps}"
    );
    approve_release(&app, lax(), id.parse().unwrap())
        .await
        .unwrap();
    // Staff don't list drafts from their queue any more, nor by hand (B-4).
    assert_eq!(queue(&app, &staff).await["ready_to_list"], json!([]));
    let res = app
        .send(req(Method::POST, "/v1/admin/apps/game/list").cookie(&staff))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("its maker puts a draft in the store themselves"),
    );
    assert!(
        admin::list_app(&app.db, "game", "Rita Reviewer")
            .await
            .is_err(),
        "nor from the command line"
    );
    let steps = body_json(
        app.send(req(Method::GET, "/v1/me/next-steps").cookie(&maker))
            .await,
    )
    .await;
    assert!(
        steps["steps"].as_array().unwrap().iter().any(|s| s["what"]
            == "Put Game in the store from its Manage page."
            && s["who"] == "you"),
        "{steps}"
    );
    assert_eq!(
        app.send(list(&maker)).await.status(),
        StatusCode::NO_CONTENT
    );
    let (status, actor, detail): (String, String, Value) = sqlx::query_as(
        "select a.status::text, l.actor, l.detail from apps a, audit_log l
         where a.slug = 'game' and l.action = 'app.list' and l.subject = 'game'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(status, "listed");
    assert_eq!(actor, maker_id.to_string());
    assert_eq!(detail, json!({ "from": "draft", "by": "maker" }));
    let res = app.send(list(&maker)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(res).await["error"]["message"],
        "It's already in the store."
    );
    // Taken out by staff: back only through an appeal.
    sqlx::query("update apps set status = 'delisted' where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    let res = app.send(list(&maker)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Moderation page")
    );
    // Reinstated on appeal: staff's to list, in their queue.
    sqlx::query("update apps set relist_after_appeal = true where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        queue(&app, &staff).await["ready_to_list"][0]["slug"],
        "game"
    );
    // Private: staff make it public after checking the repository.
    sqlx::query(
        "update apps set status = 'draft', relist_after_appeal = false, visibility = 'private' where slug = 'game'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    let res = app.send(list(&maker)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        body_json(res).await["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("It's private, so it can't go in the store.")
    );
}

/// Releases left waiting for a person by the old flow have their checks end at startup, and
/// pending requests to hold players at a published release are applied (SPEC §9), unless a newer
/// release was published after the request: then it's closed and the maker told (B-12).
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn releases_left_waiting_are_settled_at_startup(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "beta").await;
    let first = submit_and_fetch(&app, &maker, "1.0.0").await;
    approve_release(&app, lax(), first.parse().unwrap())
        .await
        .unwrap();
    let second = submit_and_fetch(&app, &maker, "1.1.0").await;
    approve_release(&app, lax(), second.parse().unwrap())
        .await
        .unwrap();
    // As the old flow left them: 1.2.0 waiting for a person with no hold, and a request to hold
    // players at 1.0.0 made before 1.1.0 was published.
    let waiting = submit_and_fetch(&app, &maker, "1.2.0").await;
    sqlx::query("update releases set status = 'awaiting_review' where id = $1::uuid")
        .bind(&waiting)
        .execute(&app.db)
        .await
        .unwrap();
    let pin = |created_ago: &'static str| {
        let (app, first) = (&app, &first);
        async move {
            let id = Uuid::now_v7();
            sqlx::query(
                "insert into app_changes (id, app_id, kind, release_id, requested_by, created_at)
                 select $1, a.id, 'release', $2::uuid, a.maker_id, now() - $3::interval
                 from apps a where a.slug = 'game'",
            )
            .bind(id)
            .bind(first)
            .bind(created_ago)
            .execute(&app.db)
            .await
            .unwrap();
            id
        }
    };
    let stale = pin("1 day").await;
    // Unread "passed the checks" notices for the app merge: count how many were sent.
    let told = || async {
        sqlx::query_scalar::<_, i32>(
            "select count from notifications where user_id = $1 and collapse_key = 'ready:game'",
        )
        .bind(maker_id)
        .fetch_one(&app.db)
        .await
        .unwrap()
    };
    let before = told().await;
    slop_api::publish::settle_after_upgrade(&app.state)
        .await
        .unwrap();
    assert_eq!(release_status(&app, &waiting).await.0, "ready");
    assert_eq!(told().await, before + 1, "its maker is told");
    // Holding players back isn't what the maker asked for once they published 1.1.0.
    assert_eq!(served_version(&app, &maker).await, "1.1.0");
    let (status, reason): (String, Option<String>) =
        sqlx::query_as("select status, status_reason from app_changes where id = $1")
            .bind(stale)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(status, "cancelled");
    assert_eq!(
        reason.as_deref(),
        Some("Version 1.1.0 was published after you asked, so players get it instead.")
    );
    let said = notices(&app, maker_id).await;
    assert!(
        said.iter().any(|(t, b)| t == "Game: players aren't held at 1.0.0"
            && b == "You asked to hold players at 1.0.0, then published 1.1.0, so players get 1.1.0. To hold them at an earlier version, ask again on the Manage page."),
        "{said:?}"
    );
    // One made after it is applied, as such requests now are at once. Again: nothing more.
    pin("0 seconds").await;
    slop_api::publish::settle_after_upgrade(&app.state)
        .await
        .unwrap();
    assert_eq!(served_version(&app, &maker).await, "1.0.0");
    slop_api::publish::settle_after_upgrade(&app.state)
        .await
        .unwrap();
    assert_eq!(release_status(&app, &waiting).await.0, "ready");
    assert_eq!(served_version(&app, &maker).await, "1.0.0");
}

/// The maker's conversations about their releases are in their export, and their own messages
/// are blanked when the account closes; staff answers and SLAPorSLOP's lines stay (ruling 8).
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn hold_conversations_are_exported_and_blanked_on_closing(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "beta").await;
    let (_, staff) = reviewer(&app, "Rita Reviewer").await;
    let id = submit_and_fetch(&app, &maker, "1.0.0").await;
    flag_release(&app, &id).await;
    assert_eq!(
        app.send(write(&maker, &id, "It's a false positive."))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        app.send(
            req(
                Method::POST,
                format!("/v1/admin/releases/{id}/hold/messages"),
            )
            .cookie(&staff)
            .json(json!({ "body": "Looking." }))
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    let export = body_json(
        app.send(req(Method::GET, "/v1/me/export").cookie(&maker))
            .await,
    )
    .await;
    let talk = &export["release_conversations"][0];
    assert_eq!(
        (&talk["app"], &talk["version"]),
        (&json!("game"), &json!("1.0.0"))
    );
    let froms: Vec<(&str, &str)> = talk["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| (m["from"].as_str().unwrap(), m["body"].as_str().unwrap()))
        .collect();
    assert_eq!(
        froms,
        [
            (
                "slaporslop",
                "On hold: a scanner flagged the Windows (.zip) build."
            ),
            ("you", "It's a false positive."),
            ("staff", "Looking."),
        ]
    );
    assert!(!export["release_conversations"].to_string().contains("Rita"));

    slop_api::account::close_account(&app.db, maker_id, "test")
        .await
        .unwrap();
    let rows: Vec<(Option<Uuid>, String)> = sqlx::query_as(
        "select author_id, body from release_hold_messages where release_id = $1::uuid order by created_at",
    )
    .bind(&id)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[1],
        (
            None,
            "(message deleted when the account closed)".to_string()
        )
    );
    assert_eq!(rows[2].1, "Looking.");
    assert!(rows[2].0.is_some());
}

/// A staff member with the owner role, who can make admin API tokens. Returns (ID, cookie).
async fn owner_staff(app: &TestApp, name: &str) -> (Uuid, String) {
    let (id, cookie) = app.user(name, "US").await;
    sqlx::query("update users set is_admin = true where id = $1")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("insert into staff_roles (user_id, role, granted_by) values ($1, 'owner', 'test')")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
    (id, cookie)
}

/// A review-level admin API token made by this owner, as an Authorization header value.
async fn review_token(app: &TestApp, cookie: &str) -> String {
    let res = app
        .send(
            req(Method::POST, "/v1/admin/tokens")
                .cookie(cookie)
                .json(json!({ "name": "Agent", "scope": "review", "days": 7 })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    format!("Bearer {}", body_json(res).await["token"].as_str().unwrap())
}

async fn error_message(res: Response) -> String {
    body_json(res).await["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// Microsoft Defender's report flagging release `id`'s Windows file, as Slop Check would attach it.
async fn defender_flags(app: &TestApp, id: &str) {
    sqlx::query(
        "update release_files set defender = jsonb_build_object('sha256', sha256, 'engine', '1.1',
                'signatures', '1.4', 'detections', jsonb_build_array(jsonb_build_object('file', 'game.exe', 'threat', 'Trojan:Win32/Test')))
         where release_id = $1::uuid",
    )
    .bind(id)
    .execute(&app.db)
    .await
    .unwrap();
}

/// Microsoft Defender is judged apart from the scanner (B-1): accepting or clearing a scanner
/// finding never covers a missing Defender check, and a Defender report that flags the file later
/// still holds it. Outside GitHub, where Slop Check can't run, a missing check is staff's from the
/// start, and a staff member signed in (never an agent's token) waives it after checking by hand.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn defender_is_judged_apart_from_the_scanner(db: PgPool) {
    let dir = std::env::temp_dir().join(format!("slop-holds-{}", Uuid::now_v7()));
    let (command, mode) = mode_scanner(&dir);
    let app = test_app_with(db, move |c| {
        c.releases.scanner = slop_api::scan::Scanner::from_setting("local", &command).unwrap();
    })
    .await;
    let (maker_id, maker) = maker_with_app(&app, "beta").await;
    let (staff_id, staff) = owner_staff(&app, "Olive Owner").await;
    let agent = review_token(&app, &staff).await;
    let id = submit_and_fetch(&app, &maker, "1.0.0").await;

    // The test's repository isn't on GitHub: Slop Check can't run, so staff look from the start.
    let h = hold(&app, &maker, &id).await;
    assert_eq!(h["kinds"], json!(["scan", "defender_missing"]));
    assert_eq!(h["needs_staff"], true);
    let missing = h["problems"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["kind"] == "defender_missing")
        .unwrap()
        .clone();
    assert_eq!(
        missing["what"],
        "The Windows build needs Microsoft Defender's check"
    );
    assert_eq!(
        missing["explained"],
        "Slop Check runs on GitHub Actions, so it can't run for a repository outside GitHub yet. SLAPorSLOP staff check the Windows build by hand; you can write to them below."
    );
    assert_eq!(missing["you_can"], json!(["write_to_staff", "new_release"]));
    let told = notices(&app, maker_id).await;
    let held = told
        .iter()
        .find(|(t, _)| t == "Game 1.0.0 is on hold")
        .unwrap();
    assert!(!held.1.contains("run Slop Check"), "{held:?}");
    assert!(
        held.1.contains("SLAPorSLOP staff check it by hand"),
        "{held:?}"
    );
    let mine = body_json(
        app.send(req(Method::GET, "/v1/apps/game/releases").cookie(&maker))
            .await,
    )
    .await;
    let summary = mine[0]["progress"]["summary"].as_str().unwrap();
    assert!(!summary.contains("Run Slop Check"), "{summary}");

    // Staff answering doesn't take it out of their queue: only they can resolve it.
    let answer = req(
        Method::POST,
        format!("/v1/admin/releases/{id}/hold/messages"),
    )
    .cookie(&staff)
    .json(json!({ "body": "Looking at the build now." }));
    assert_eq!(app.send(answer).await.status(), StatusCode::NO_CONTENT);
    assert_eq!(hold(&app, &maker, &id).await["needs_staff"], true);

    // Accepting clears the scanner's finding, never the missing check.
    let approve = |who: &str| {
        let r = req(Method::POST, format!("/v1/admin/releases/{id}/approve"));
        if who.starts_with("Bearer ") {
            r.header("authorization", who.to_string())
        } else {
            r.cookie(who)
        }
    };
    assert_eq!(
        app.send(approve(&staff)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(release_status(&app, &id).await.0, "awaiting_review");
    let h = hold(&app, &maker, &id).await;
    assert_eq!(h["kinds"], json!(["defender_missing"]));
    // Clearing has nothing to clear on it now.
    let clear = req(
        Method::POST,
        format!("/v1/admin/releases/{id}/files/windows/clear"),
    )
    .cookie(&staff)
    .json(json!({}));
    let res = app.send(clear).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        error_message(res)
            .await
            .contains("nothing on this file is flagged")
    );

    // Defender reports a detection after the scanner's finding was cleared: it still holds it.
    defender_flags(&app, &id).await;
    slop_api::publish::finish_checks(
        &app.db,
        admin::ReviewRules::from_config(&app.config),
        id.parse().unwrap(),
        "slop",
    )
    .await
    .unwrap();
    assert_eq!(hold(&app, &maker, &id).await["kinds"], json!(["defender"]));
    // Clearing it now covers Defender's detections, which were there to judge.
    let clear = req(
        Method::POST,
        format!("/v1/admin/releases/{id}/files/windows/clear"),
    )
    .cookie(&staff)
    .json(json!({}));
    assert_eq!(app.send(clear).await.status(), StatusCode::NO_CONTENT);
    assert_eq!(release_status(&app, &id).await.0, "ready");

    // 1.1.0 passes the scanner; only Defender's check is missing.
    std::fs::write(&mode, "clean").unwrap();
    let next = submit_and_fetch(&app, &maker, "1.1.0").await;
    assert_eq!(
        hold(&app, &maker, &next).await["kinds"],
        json!(["defender_missing"])
    );
    let waive = |who: &str, body: Value| {
        let r = req(
            Method::POST,
            format!("/v1/admin/releases/{next}/files/windows/waive-defender"),
        )
        .json(body);
        if who.starts_with("Bearer ") {
            r.header("authorization", who.to_string())
        } else {
            r.cookie(who)
        }
    };
    // An AI agent's token can't waive it, nor can the maker.
    let res = app.send(waive(&agent, json!({}))).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        error_message(res).await,
        "Waiving Microsoft Defender's check is for a SLAPorSLOP staff member signed in themselves, not an AI agent's token."
    );
    assert_eq!(
        app.send(waive(&maker, json!({}))).await.status(),
        StatusCode::NOT_FOUND
    );
    let res = app
        .send(waive(
            &staff,
            json!({ "reason": "Checked the build on a Windows machine with Defender." }),
        ))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(release_status(&app, &next).await.0, "ready");
    let h = hold(&app, &maker, &next).await;
    let bodies: Vec<&str> = h["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["body"].as_str().unwrap())
        .collect();
    assert!(
        bodies.contains(&"SLAPorSLOP staff checked the Windows (.zip) build by hand and waived Microsoft Defender's check."),
        "{bodies:?}"
    );
    assert!(bodies.contains(&"Checked the build on a Windows machine with Defender."));
    let waived: (Option<String>,) =
        sqlx::query_as("select defender_waived_by from release_files where release_id = $1::uuid")
            .bind(&next)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(waived.0.unwrap().contains("Olive Owner"));
    let mine = body_json(
        app.send(req(Method::GET, "/v1/apps/game/releases").cookie(&maker))
            .await,
    )
    .await;
    assert_eq!(mine[0]["files"][0]["defender_waived"], true);
    // Waived twice: nothing missing to waive.
    assert_eq!(
        app.send(waive(&staff, json!({}))).await.status(),
        StatusCode::CONFLICT
    );

    // A Defender report that flags the waived file later still holds it, at Publish too.
    defender_flags(&app, &next).await;
    sqlx::query(
        "update releases set repo_scan = '{\"status\": \"not_checked\"}' where id = $1::uuid",
    )
    .bind(&next)
    .execute(&app.db)
    .await
    .unwrap();
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/apps/game/releases/{next}/publish"),
            )
            .cookie(&maker),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(
        hold(&app, &maker, &next).await["kinds"],
        json!(["defender"])
    );
    assert!(
        notices(&app, staff_id)
            .await
            .iter()
            .any(|(t, _)| t == "Releases on hold")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A lowered rating is accepted only as staff saw it (B-2): by a person signed in, never an AI
/// agent's token; recorded with the change, so a worse one at Publish is held again; and a hold
/// that didn't show a lowered rating doesn't accept one.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_lowered_rating_is_accepted_by_a_person_as_they_saw_it(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "released").await;
    let (_, staff) = owner_staff(&app, "Olive Owner").await;
    let agent = review_token(&app, &staff).await;
    let submit = |version: &str, age: i16, labels: Value| {
        let mut body = release_body(&app, version, "game-windows.zip");
        body["rating"] = json!({ "min_age": age, "content_labels": labels });
        req(Method::POST, "/v1/apps/game/releases")
            .cookie(&maker)
            .json(body)
    };
    let sent =
        |res: Response| async move { body_json(res).await["id"].as_str().unwrap().to_string() };
    let first = sent(
        app.send(submit("1.0.0", 16, json!(["violence", "strong_language"])))
            .await,
    )
    .await;
    releases::fetch_next(&app.state).await.unwrap();
    approve_release(&app, lax(), first.parse().unwrap())
        .await
        .unwrap();
    let id = sent(app.send(submit("1.1.0", 12, json!([]))).await).await;
    releases::fetch_next(&app.state).await.unwrap();
    let h = hold(&app, &maker, &id).await;
    assert_eq!(h["kinds"], json!(["rating_lowered"]));
    // Labels as players read them.
    assert_eq!(
        h["problems"][0]["explained"],
        "Lowers the age rating from 16+ to 12+ (drops: violence, strong language). A SLAPorSLOP staff member checks a lowered rating before you can publish."
    );

    // An AI agent's token can't accept it; a person can.
    let approve = |who: &str| {
        let r = req(Method::POST, format!("/v1/admin/releases/{id}/approve"));
        if who.starts_with("Bearer ") {
            r.header("authorization", who.to_string())
        } else {
            r.cookie(who)
        }
    };
    let res = app.send(approve(&agent)).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        error_message(res).await,
        "A lowered age rating is accepted by a SLAPorSLOP staff member signed in themselves, not by an AI agent's token. Leave a note for staff instead."
    );
    // The agent's feed doesn't offer it either.
    let feed = body_json(
        app.send(
            req(Method::GET, "/v1/admin/review?kind=release")
                .header("authorization", agent.clone()),
        )
        .await,
    )
    .await;
    assert!(feed["items"][0].get("approve").is_none(), "{feed}");
    assert_eq!(
        app.send(approve(&staff)).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(release_status(&app, &id).await.0, "ready");
    let accepted: (Option<i16>, Option<i16>, Option<Vec<String>>) = sqlx::query_as(
        "select rating_accepted_from, rating_accepted_to, rating_accepted_labels from releases where id = $1::uuid",
    )
    .bind(&id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        accepted,
        (
            Some(16),
            Some(12),
            Some(vec!["violence".to_string(), "strong_language".to_string()])
        )
    );

    // The store page is raised to 18 meanwhile: publishing would lower more than staff accepted.
    let res = app
        .send(
            req(Method::PATCH, "/v1/apps/game")
                .cookie(&maker)
                .json(json!({ "min_age": 18 })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let res = app
        .send(req(Method::POST, format!("/v1/apps/game/releases/{id}/publish")).cookie(&maker))
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let h = hold(&app, &maker, &id).await;
    assert_eq!(h["kinds"], json!(["rating_lowered"]));
    assert_eq!(h["rating"]["from_age"], 18);

    // 1.2.0 keeps the rating but a scanner holds it; the store page rises before staff decide, so
    // the checks now also find a lowered rating the hold never showed. Accepting clears the
    // finding only: the rating is held again, for staff to see.
    let third = sent(
        app.send(submit("1.2.0", 18, json!(["violence", "strong_language"])))
            .await,
    )
    .await;
    releases::fetch_next(&app.state).await.unwrap();
    flag_release(&app, &third).await;
    assert_eq!(hold(&app, &maker, &third).await["kinds"], json!(["scan"]));
    let res = app
        .send(
            req(Method::PATCH, "/v1/apps/game")
                .cookie(&maker)
                .json(json!({ "content_labels": ["violence", "strong_language", "horror"] })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let outcome = admin::accept(
        &app.db,
        lax(),
        third.parse().unwrap(),
        "Olive Owner",
        None,
        None,
    )
    .await
    .unwrap();
    assert!(
        matches!(&outcome, slop_api::publish::Outcome::Held { kinds, .. } if kinds == &[slop_api::publish::HoldKind::RatingLowered]),
        "{outcome:?}"
    );
    let accepted: Option<OffsetDateTime> =
        sqlx::query_scalar("select rating_accepted_at from releases where id = $1::uuid")
            .bind(&third)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(accepted.is_none());
}

/// What players are shown is the baseline for a lowered rating (B-3): a rating SLAPorSLOP staff
/// set on the store page is a floor the maker can't go under and a lower release keeps, and
/// holding players at an older, higher-rated release raises the store page.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_store_rating_is_what_a_lowered_rating_is_judged_against(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "released").await;
    let (_, staff) = owner_staff(&app, "Olive Owner").await;
    let submit = |version: &str, age: i16| {
        let mut body = release_body(&app, version, "game-windows.zip");
        body["rating"] = json!({ "min_age": age, "content_labels": [] });
        req(Method::POST, "/v1/apps/game/releases")
            .cookie(&maker)
            .json(body)
    };
    let sent =
        |res: Response| async move { body_json(res).await["id"].as_str().unwrap().to_string() };
    let store_age = || async {
        sqlx::query_scalar::<_, i16>("select min_age from apps where slug = 'game'")
            .fetch_one(&app.db)
            .await
            .unwrap()
    };
    let first = sent(app.send(submit("1.0.0", 12)).await).await;
    releases::fetch_next(&app.state).await.unwrap();
    approve_release(&app, lax(), first.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(store_age().await, 12);

    // Staff raise the store page to 16: the maker can't lower it again.
    let res = app
        .send(
            req(Method::PATCH, "/v1/admin/apps/game/listing")
                .cookie(&staff)
                .json(json!({ "min_age": 16, "reason": "The game shows blood." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let res = app
        .send(
            req(Method::PATCH, "/v1/apps/game")
                .cookie(&maker)
                .json(json!({ "min_age": 12 })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        error_message(res)
            .await
            .contains("SLAPorSLOP staff set this rating")
    );
    // A release still rated 12 isn't held (the store keeps 16), and publishing it keeps 16.
    let second = sent(app.send(submit("1.1.0", 12)).await).await;
    releases::fetch_next(&app.state).await.unwrap();
    assert_eq!(release_status(&app, &second).await.0, "ready");
    approve_release(&app, lax(), second.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(store_age().await, 16);

    // 1.2.0 is rated 18 and published; 1.3.0 goes back to 16 (staff accept it) and is published.
    let third = sent(app.send(submit("1.2.0", 18)).await).await;
    releases::fetch_next(&app.state).await.unwrap();
    approve_release(&app, lax(), third.parse().unwrap())
        .await
        .unwrap();
    let fourth = sent(app.send(submit("1.3.0", 16)).await).await;
    releases::fetch_next(&app.state).await.unwrap();
    assert_eq!(release_status(&app, &fourth).await.0, "awaiting_review");
    admin::approve(&app.db, lax(), fourth.parse().unwrap(), "Olive Owner")
        .await
        .unwrap();
    approve_release(&app, lax(), fourth.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(store_age().await, 16);
    // Holding players at 1.2.0 shows its higher rating on the store page.
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/changes")
                .cookie(&maker)
                .json(json!({ "kind": "release", "release_id": third, "note": "1.3 crashes" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    assert_eq!(store_age().await, 18);
    // The staff record shows when each release was published.
    let record = body_json(
        app.send(req(Method::GET, "/v1/admin/apps/game/record").cookie(&staff))
            .await,
    )
    .await;
    assert!(
        record["releases"][0]["published_at"].is_string(),
        "{record}"
    );
}

/// An older release never ends ready, or stays on hold, once a newer one is out (B-5): Publish
/// withdraws older releases still being checked, and an older one whose checks end after a newer
/// one was published (left by the old flow, say) is replaced, with no "passed" notice. Questions
/// about older failed or turned-down releases close once a newer release passes (B-7), and a
/// maker can ask about a release staff turned down (B-15).
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn an_older_release_is_replaced_once_a_newer_one_is_out(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "beta").await;
    let (_, staff) = reviewer(&app, "Rita Reviewer").await;
    // 1.0.0 is being fetched (by another instance, say) when 1.1.0 passes and is published.
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases")
                .cookie(&maker)
                .json(release_body(&app, "1.0.0", "game-windows.zip")),
        )
        .await;
    let older = body_json(res).await["id"].as_str().unwrap().to_string();
    sqlx::query("update releases set fetch_started_at = now() where id = $1::uuid")
        .bind(&older)
        .execute(&app.db)
        .await
        .unwrap();
    let newer = submit_and_fetch(&app, &maker, "1.1.0").await;
    approve_release(&app, lax(), newer.parse().unwrap())
        .await
        .unwrap();
    let (status, reason, _) = release_status(&app, &older).await;
    assert_eq!(
        (status.as_str(), reason.as_deref()),
        (
            "withdrawn",
            Some("Replaced by 1.1.0 before it was published.")
        )
    );
    // The fetcher leaves it alone, even once its lease runs out.
    sqlx::query(
        "update releases set fetch_started_at = now() - interval '2 hours' where id = $1::uuid",
    )
    .bind(&older)
    .execute(&app.db)
    .await
    .unwrap();
    assert!(!releases::fetch_next(&app.state).await.unwrap());
    assert_eq!(release_status(&app, &older).await.0, "withdrawn");

    // As the old flow could leave it: on hold, with no hold row, older than the published 1.1.0.
    sqlx::query(
        "update releases set status = 'awaiting_review', status_reason = null where id = $1::uuid",
    )
    .bind(&older)
    .execute(&app.db)
    .await
    .unwrap();
    let outcome = slop_api::publish::finish_checks(&app.db, lax(), older.parse().unwrap(), "slop")
        .await
        .unwrap();
    assert_eq!(
        outcome,
        slop_api::publish::Outcome::Replaced("1.1.0".into())
    );
    let (status, reason, _) = release_status(&app, &older).await;
    assert_eq!(
        (status.as_str(), reason.as_deref()),
        (
            "withdrawn",
            Some("Replaced by 1.1.0 before it was published.")
        )
    );
    assert!(
        !notices(&app, maker_id)
            .await
            .iter()
            .any(|(t, _)| t == "Game 1.0.0 passed SLAPorSLOP's checks")
    );

    // 1.2.0 fails its checks and its maker asks staff; 1.3.0 passes, which closes the question.
    let mut body = release_body(&app, "1.2.0", "x.zip");
    body["files"][0]["url"] = json!(format!("{}octo/game/missing/x.zip", app.fake_base));
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases")
                .cookie(&maker)
                .json(body),
        )
        .await;
    let failed = body_json(res).await["id"].as_str().unwrap().to_string();
    releases::fetch_next(&app.state).await.unwrap();
    assert_eq!(release_status(&app, &failed).await.0, "failed");
    assert!(
        notices(&app, maker_id)
            .await
            .iter()
            .any(|(t, _)| t == "Game 1.2.0 didn't pass SLAPorSLOP's checks")
    );
    assert_eq!(
        app.send(write(&maker, &failed, "Was it a hiccup?"))
            .await
            .status(),
        StatusCode::OK
    );
    let fixed = submit_and_fetch(&app, &maker, "1.3.0").await;
    assert_eq!(release_status(&app, &fixed).await.0, "ready");
    let h = hold(&app, &maker, &failed).await;
    assert_eq!(h["status"], "replaced");
    assert_eq!(
        h["messages"].as_array().unwrap().last().unwrap()["body"],
        "Closed: version 1.3.0 replaced it."
    );
    assert!(admin::pending(&app.db).await.unwrap().is_empty());

    // Staff turn 1.3.0 down; its maker can ask about that too, and staff answer.
    admin::reject(
        &app.db,
        fixed.parse().unwrap(),
        "Rita Reviewer",
        "The store page misleads.",
    )
    .await
    .unwrap();
    assert!(
        notices(&app, maker_id)
            .await
            .iter()
            .any(|(t, _)| t == "Game 1.3.0 was turned down by SLAPorSLOP staff")
    );
    let h = hold(&app, &maker, &fixed).await;
    assert_eq!(h["problems"][0]["kind"], "rejected");
    assert_eq!(h["problems"][0]["explained"], "The store page misleads.");
    assert!(
        h["you_can"]
            .as_array()
            .unwrap()
            .contains(&json!("write_to_staff"))
    );
    assert_eq!(
        app.send(write(
            &maker,
            &fixed,
            "I changed the page: can you look again?"
        ))
        .await
        .status(),
        StatusCode::OK
    );
    let feed = body_json(
        app.send(req(Method::GET, "/v1/admin/review?kind=release").cookie(&staff))
            .await,
    )
    .await;
    assert!(
        feed["items"][0]["question"]
            .as_str()
            .unwrap()
            .starts_with("Answer the maker's question about version 1.3.0 of Game: SLAPorSLOP staff turned it down (The store page misleads)."),
        "{feed}"
    );
    let res = app
        .send(
            req(
                Method::POST,
                format!("/v1/admin/releases/{fixed}/hold/messages"),
            )
            .cookie(&staff)
            .json(json!({ "body": "Thanks: send it again as 1.3.1." })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(hold(&app, &maker, &fixed).await["status"], "answered");
}

/// A release that can't be published without its source check keeps being tried (B-6): after a
/// week of failures it's recorded as not checked, which lets its maker publish; a published one
/// is tried for a week after it was sent.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn an_unreadable_source_stops_blocking_publish_after_a_week(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, _) = maker_with_app(&app, "beta").await;
    // On GitHub, at a repository the stand-in doesn't have: reading it fails.
    sqlx::query("update apps set repo_url = 'https://github.com/octo/gone' where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    let release = |status: &str, version: &str, failing_days: i64| {
        let (status, version) = (status.to_string(), version.to_string());
        let app = &app;
        async move {
            let id = Uuid::now_v7();
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "insert into releases (id, app_id, version, commit_sha, status, submitted_by, created_at,
                                       repo_scan, repo_scanned_at)
                 select $1, a.id, $2, repeat('c', 40), '{status}', a.maker_id, now() - interval '10 days',
                        jsonb_build_object('status', 'failed', 'detail', 'GitHub answered 502',
                                           'failing_since', to_char((now() - make_interval(days => $3::int)) at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')),
                        now() - interval '2 hours'
                 from apps a where a.slug = 'game'"
            )))
            .bind(id)
            .bind(version)
            .bind(failing_days as i32)
            .execute(&app.db)
            .await
            .unwrap();
            id
        }
    };
    let published = release("approved", "0.9.0", 8).await;
    let week = release("ready", "1.0.0", 8).await;
    sqlx::query(
        "insert into release_files (release_id, platform, kind, url, file_name, size_bytes, sha256, scan_result)
         values ($1, 'windows', 'archive', 'https://github.com/octo/gone/releases/download/v1/g.zip', 'g.zip', 10,
                 repeat('a', 64), 'skipped')",
    )
    .bind(week)
    .execute(&app.db)
    .await
    .unwrap();
    let day = release("awaiting_review", "1.1.0", 1).await;
    let scanned = slop_api::source_scan::run_once(&app.state).await.unwrap();
    assert_eq!(scanned, 2, "not the published one sent 10 days ago");
    let scan = |id: Uuid| {
        let app = &app;
        async move {
            sqlx::query_scalar::<_, Value>("select repo_scan from releases where id = $1")
                .bind(id)
                .fetch_one(&app.db)
                .await
                .unwrap()
        }
    };
    let gave_up = scan(week).await;
    assert_eq!(gave_up["status"], "not_checked");
    assert!(
        gave_up["detail"]
            .as_str()
            .unwrap()
            .starts_with("SLAPorSLOP couldn't read this release's source for a week ("),
        "{gave_up}"
    );
    let trying = scan(day).await;
    assert_eq!(trying["status"], "failed");
    assert!(
        trying["failing_since"].is_string(),
        "the first failure is kept: {trying}"
    );
    assert_eq!(scan(published).await["detail"], "GitHub answered 502");
    // Not reading it no longer stops its maker publishing.
    slop_api::publish::publish(&app.db, Some(&app.signing_key), lax(), week, maker_id)
        .await
        .unwrap();
}

/// A release whose checks keep failing after its files were fetched stops after 3 tries, failed
/// with a plain reason, instead of being downloaded again forever (B-10).
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn checks_that_keep_failing_stop_after_three_tries(db: PgPool) {
    let app = test_app(db).await;
    let (maker_id, maker) = maker_with_app(&app, "beta").await;
    let res = app
        .send(
            req(Method::POST, "/v1/apps/game/releases")
                .cookie(&maker)
                .json(release_body(&app, "1.0.0", "game-windows.zip")),
        )
        .await;
    let id = body_json(res).await["id"].as_str().unwrap().to_string();
    sqlx::query("update releases set fetch_attempts = 3 where id = $1::uuid")
        .bind(&id)
        .execute(&app.db)
        .await
        .unwrap();
    assert!(releases::fetch_next(&app.state).await.unwrap());
    let (status, reason, _) = release_status(&app, &id).await;
    assert_eq!(status, "failed");
    assert_eq!(
        reason.as_deref(),
        Some(
            "SLAPorSLOP couldn't finish checking it after 3 tries. Send it again as a new version; if that fails too, write to SLAPorSLOP staff from the release."
        )
    );
    assert!(
        notices(&app, maker_id)
            .await
            .iter()
            .any(|(t, _)| t == "Game 1.0.0 didn't pass SLAPorSLOP's checks")
    );
    // Its maker can ask staff about it, as the reason says.
    assert_eq!(
        app.send(write(&maker, &id, "It keeps failing: why?"))
            .await
            .status(),
        StatusCode::OK
    );
}

/// The ops alert counts a hold waiting on staff from the maker's latest unanswered message; one
/// staff answered last waits for the maker, not staff (B-7). Staff notices name the app by its
/// title cleaned as staff text is, or else its address (B-14).
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_ops_alert_and_staff_notices_about_holds(db: PgPool) {
    let app = test_app(db).await;
    let (_, maker) = maker_with_app(&app, "beta").await;
    let (staff_id, staff) = reviewer(&app, "Rita Reviewer").await;
    sqlx::query("update apps set title = 'Game\u{202e}emaG' where slug = 'game'")
        .execute(&app.db)
        .await
        .unwrap();
    let id = submit_and_fetch(&app, &maker, "1.0.0").await;
    flag_release(&app, &id).await;
    assert_eq!(
        notices(&app, staff_id).await,
        [(
            "Releases on hold".to_string(),
            "Newest: game 1.0.0 (a scanner finding).".to_string()
        )]
    );
    let waiting = || async {
        slop_api::ops::collect(&app.state)
            .await
            .unwrap()
            .into_iter()
            .any(|a| a.key == "reviews-waiting")
    };
    // Opened two days ago, nobody wrote: waiting on staff.
    sqlx::query("update release_holds set opened_at = now() - interval '2 days' where release_id = $1::uuid")
        .bind(&id)
        .execute(&app.db)
        .await
        .unwrap();
    assert!(waiting().await);
    // Staff wrote last: it waits for the maker now.
    let answer = req(
        Method::POST,
        format!("/v1/admin/releases/{id}/hold/messages"),
    )
    .cookie(&staff)
    .json(json!({ "body": "Which packer does the build use?" }));
    assert_eq!(app.send(answer).await.status(), StatusCode::NO_CONTENT);
    assert!(!waiting().await);
    // The maker answers: an hour isn't long; two days is.
    assert_eq!(
        app.send(write(&maker, &id, "UPX, from the build script."))
            .await
            .status(),
        StatusCode::OK
    );
    assert!(!waiting().await);
    sqlx::query(
        "update release_hold_messages set created_at = now() - interval '2 days'
         where release_id = $1::uuid and not from_staff and kind = 'message'",
    )
    .bind(&id)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query(
        "update release_hold_messages set created_at = now() - interval '3 days'
         where release_id = $1::uuid and from_staff",
    )
    .bind(&id)
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query("update release_holds set maker_asked_at = now() - interval '2 days' where release_id = $1::uuid")
        .bind(&id)
        .execute(&app.db)
        .await
        .unwrap();
    assert!(waiting().await);
}
