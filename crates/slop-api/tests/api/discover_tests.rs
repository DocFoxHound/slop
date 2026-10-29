// SPDX-License-Identifier: AGPL-3.0-or-later
//! Discover (discover.rs): the personal reels and why they're there, what never shows, the
//! general reels for visitors and people with recommendations off, and the daily order.

use crate::*;

use std::collections::{HashMap, HashSet};

use slop_api::discover;

// ---------------------------------------------------------------------------------------------
// Helpers for this area

async fn exec(app: &TestApp, sql: &str, user: Uuid, slug: &str) {
    sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
        .bind(user)
        .bind(slug)
        .execute(&app.db)
        .await
        .unwrap();
}

async fn library(app: &TestApp, user: Uuid, slug: &str) {
    exec(
        app,
        "insert into library_entries (user_id, app_id) select $1, id from apps where slug = $2",
        user,
        slug,
    )
    .await;
}

async fn vote(app: &TestApp, user: Uuid, slug: &str, vote: i16) {
    sqlx::query(
        "insert into votes (user_id, app_id, vote, weight) select $1, id, $3, 1 from apps where slug = $2",
    )
    .bind(user)
    .bind(slug)
    .bind(vote)
    .execute(&app.db)
    .await
    .unwrap();
}

async fn played(app: &TestApp, user: Uuid, slug: &str, seconds: i64, days: i32) {
    sqlx::query(
        "insert into user_app_usage (user_id, app_id, playtime_seconds, days_launched)
         select $1, id, $3, $4 from apps where slug = $2",
    )
    .bind(user)
    .bind(slug)
    .bind(seconds)
    .bind(days)
    .execute(&app.db)
    .await
    .unwrap();
}

/// A paid one-off donation (or one in another state).
async fn donated(app: &TestApp, user: Uuid, slug: &str, status: &str) {
    sqlx::query(
        "insert into donations (id, kind, app_id, donor_id, stripe_account_id, currency, donation_minor,
                                slop_fee_minor, processing_minor, total_minor, donor_covers_fees, status, paid_at)
         select $1, 'one_off', id, $2, 'acct_test', 'usd', 500, 25, 45, 570, true, $4,
                case when $4 = 'paid' then now() end
         from apps where slug = $3",
    )
    .bind(Uuid::now_v7())
    .bind(user)
    .bind(slug)
    .bind(status)
    .execute(&app.db)
    .await
    .unwrap();
}

async fn page(app: &TestApp, cookie: Option<&str>, country: &str) -> Value {
    let mut r = req(Method::GET, "/v1/discover").header("x-country", country);
    if let Some(c) = cookie {
        r = r.cookie(c);
    }
    let res = app.send(r).await;
    assert_eq!(res.status(), StatusCode::OK);
    body_json(res).await
}

fn reel_keys(page: &Value) -> Vec<String> {
    page["reels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["key"].as_str().unwrap().to_string())
        .collect()
}

fn reel<'a>(page: &'a Value, key: &str) -> &'a Value {
    page["reels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["key"] == key)
        .unwrap_or_else(|| panic!("no {key} reel: {:?}", reel_keys(page)))
}

fn apps_in(reel: &Value) -> Vec<String> {
    reel["apps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["slug"].as_str().unwrap().to_string())
        .collect()
}

/// Every app on the page, with how many reels it's in.
fn shown(page: &Value) -> HashMap<String, usize> {
    let mut n = HashMap::new();
    for r in page["reels"].as_array().unwrap() {
        for slug in apps_in(r) {
            *n.entry(slug).or_default() += 1;
        }
    }
    n
}

const GENERAL: [&str; 4] = ["all_time", "rising", "week", "month"];

/// A store with five apps in each of four genres, plus what Pat did: played a roguelike for two
/// hours, 👍 a cozy game, kept a puzzle game, donated to a racing game, 👎 a horror game.
async fn store_with_pat(app: &TestApp) -> (Uuid, String) {
    let (maker, _) = app.user("Maker", "US").await;
    let (pat_id, pat) = app.user("Pat", "US").await;
    for (genre, tag) in [
        ("rogue", "roguelike"),
        ("cozy", "cozy"),
        ("puzzle", "puzzle"),
        ("racer", "racing"),
        ("horror", "horror"),
    ] {
        for i in 1..=5 {
            let slug = format!("{genre}-{i}");
            listed_with(app, maker, &slug, "released", "").await;
            tag_app(app, &slug, &[tag]).await;
        }
    }
    for (slug, tag, by, set) in [
        ("played-rogue", "roguelike", maker, ""),
        ("liked-cozy", "cozy", maker, ""),
        ("kept-puzzle", "puzzle", maker, ""),
        ("gave-racing", "racing", maker, ""),
        ("hated-horror", "horror", maker, ""),
        ("pats-rogue", "roguelike", pat_id, ""),
        (
            "dead-rogue",
            "roguelike",
            maker,
            "stage = 'beta', last_commit_at = now() - interval '200 days'",
        ),
        ("grown-rogue", "roguelike", maker, "min_age = 18"),
        ("soon-cozy", "cozy", maker, "stage = 'coming_soon'"),
        ("classic", "strategy", maker, ""),
    ] {
        let stage = if set.contains("beta") {
            "beta"
        } else {
            "released"
        };
        listed_with(app, by, slug, stage, set).await;
        tag_app(app, slug, &[tag]).await;
    }
    played(app, pat_id, "played-rogue", 7200, 5).await;
    vote(app, pat_id, "liked-cozy", 1).await;
    library(app, pat_id, "kept-puzzle").await;
    donated(app, pat_id, "gave-racing", "paid").await;
    donated(app, pat_id, "horror-1", "expired").await;
    vote(app, pat_id, "hated-horror", -1).await;

    // Everyone's numbers: Classic is loved (6 👍), racer-1 is played this week, cozy-1 is added a lot.
    for i in 0..6 {
        let (fan, _) = app.user(&format!("Fan {i}"), "US").await;
        vote(app, fan, "classic", 1).await;
        library(app, fan, "cozy-1").await;
    }
    sqlx::query(
        "insert into pulse_counts (app_id, bucket, players) select id, now() - interval '1 hour', 9 from apps where slug = 'racer-1'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    (pat_id, pat)
}

// ---------------------------------------------------------------------------------------------
// Tests

/// Signed in with recommendations on: For you first, then a reel for each of the genres Pat
/// plays, likes, keeps and supports, each saying why, mixed with the general reels. Nothing Pat
/// has, made, voted on or donated to shows, nor dead projects, nor a genre Pat voted down, and
/// no app is in more than two reels.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn discover_has_personal_reels_that_say_why(db: PgPool) {
    let app = test_app(db).await;
    let (_, pat) = store_with_pat(&app).await;
    let p = page(&app, Some(&pat), "US").await;
    assert_eq!(p["personalized"], true);
    assert!(p["off"].is_null(), "{p}");
    let keys = reel_keys(&p);
    assert_eq!(keys[0], "for_you");
    let first = reel(&p, "for_you");
    assert_eq!(first["title"], "Recommended for you");
    assert!(
        first["why"]
            .as_str()
            .unwrap()
            .starts_with("From your library, Interested list, playtime and votes, leaning on "),
        "{first}"
    );
    assert!(first["apps"][0]["reason"].is_string());
    assert!(keys[1].starts_with("genre:"), "a genre reel next: {keys:?}");

    for (key, title, why, tag) in [
        (
            "genre:play:roguelike",
            "Because you play a lot of Roguelike",
            "Roguelike is what you've spent the most time in",
            "rogue",
        ),
        (
            "genre:like:cozy",
            "Because you like Cozy",
            "From your 👍 Slaps votes and Interested list",
            "cozy-",
        ),
        (
            "genre:library:puzzle",
            "More Puzzle for your library",
            "Puzzle is the tag your library has most",
            "puzzle-",
        ),
        (
            "genre:donated:racing",
            "You've supported Racing",
            "You've donated to Racing projects",
            "racer-",
        ),
    ] {
        let r = reel(&p, key);
        assert_eq!(r["title"], title);
        assert!(r["why"].as_str().unwrap().starts_with(why), "{r}");
        assert_eq!(
            r["see_all"],
            format!("/tag/{}?sort=rating", &key[key.rfind(':').unwrap() + 1..])
        );
        let apps = apps_in(r);
        assert!(apps.len() >= 2, "{key}: {apps:?}");
        assert!(apps.iter().all(|a| a.contains(tag)), "{key}: {apps:?}");
    }
    assert!(!keys.iter().any(|k| k.ends_with(":horror")), "{keys:?}");
    // Every reel says why; the general ones are mixed in and link to more.
    for r in p["reels"].as_array().unwrap() {
        assert!(!r["why"].as_str().unwrap().is_empty(), "{r}");
    }
    assert_eq!(apps_in(reel(&p, "all_time")), ["classic"]);
    assert_eq!(
        apps_in(reel(&p, "upcoming_for_you")),
        ["soon-cozy"],
        "upcoming, in a genre Pat likes"
    );

    let counts = shown(&p);
    for never in [
        "played-rogue",
        "liked-cozy",
        "kept-puzzle",
        "gave-racing",
        "hated-horror",
        "pats-rogue",
        "dead-rogue",
    ] {
        assert!(!counts.contains_key(never), "{never} shown: {keys:?}");
    }
    assert!(counts.values().all(|n| *n <= 2), "{counts:?}");
    // An expired donation isn't a donation.
    assert!(!keys.iter().any(|k| k == "genre:donated:horror"));
}

/// Signed out, switched off, or in the UK under-18 view by default: no personal reels, and the
/// page says why; switched off is the same page as a visitor's that day (bar their own apps).
/// Turned on in the under-18 view, the genre reels still hide what that view hides.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn discover_without_personal_recommendations_is_the_general_reels(db: PgPool) {
    let app = test_app(db).await;
    let (pat_id, pat) = store_with_pat(&app).await;

    let visitor = page(&app, None, "US").await;
    assert_eq!(visitor["personalized"], false);
    assert_eq!(visitor["off"], "signed_out");
    let keys = reel_keys(&visitor);
    assert!(!keys.is_empty());
    assert!(
        keys.iter().all(|k| GENERAL.contains(&k.as_str())),
        "{keys:?}"
    );
    assert!(!shown(&visitor).contains_key("dead-rogue"));
    assert!(visitor["reels"][0]["apps"][0]["reason"].is_null());
    let all_time = reel(&visitor, "all_time");
    assert_eq!(apps_in(all_time), ["classic"]);
    assert_eq!(all_time["title"], "All-time favourites");
    assert_eq!(all_time["see_all"], "/search?sort=rating");
    // Only three apps have players or library adds, so with at most two reels per app the
    // later of week, month and up-and-coming get what's left, in their own order.
    for (key, title, full) in [
        // Most players this week, then library adds this week.
        (
            "week",
            "Popular this week",
            ["racer-1", "cozy-1", "kept-puzzle"],
        ),
        // Most library adds this month, then players.
        (
            "month",
            "Popular this month",
            ["cozy-1", "kept-puzzle", "racer-1"],
        ),
        // Released lately, most added and played.
        (
            "rising",
            "Up-and-coming",
            ["racer-1", "cozy-1", "kept-puzzle"],
        ),
    ] {
        if !keys.contains(&key.to_string()) {
            continue;
        }
        let r = reel(&visitor, key);
        assert_eq!(r["title"], title);
        let apps = apps_in(r);
        let mut rest = full.iter();
        assert!(
            !apps.is_empty() && apps.iter().all(|a| rest.any(|f| f == a)),
            "{key}: {apps:?}"
        );
    }
    assert!(keys.len() >= 3, "{keys:?}");
    assert!(shown(&visitor).values().all(|n| *n <= 2));

    app.send(
        req(Method::PATCH, "/v1/me")
            .cookie(&pat)
            .json(json!({ "personalize": false })),
    )
    .await;
    let off = page(&app, Some(&pat), "US").await;
    assert_eq!(off["personalized"], false);
    assert_eq!(off["off"], "switched_off");
    assert_eq!(reel_keys(&off), keys, "the same reels, in the same order");
    for r in off["reels"].as_array().unwrap() {
        assert!(!apps_in(r).contains(&"pats-rogue".to_string()));
    }

    // The UK under-18 view: off by default, and never 18+ apps.
    let (uk_id, uk) = app.user("Uki", "GB").await;
    library(&app, uk_id, "played-rogue").await;
    let p = page(&app, Some(&uk), "GB").await;
    assert_eq!(p["off"], "switched_off");
    assert!(reel_keys(&p).iter().all(|k| GENERAL.contains(&k.as_str())));
    sqlx::query("update users set personalize = true where id = $1")
        .bind(uk_id)
        .execute(&app.db)
        .await
        .unwrap();
    let p = page(&app, Some(&uk), "GB").await;
    assert_eq!(p["personalized"], true);
    let rogue = apps_in(reel(&p, "genre:library:roguelike"));
    assert!(!rogue.is_empty());
    assert!(!shown(&p).contains_key("grown-rogue"), "{p}");
    // Pat (US) sees it.
    sqlx::query("update users set personalize = null where id = $1")
        .bind(pat_id)
        .execute(&app.db)
        .await
        .unwrap();
    let p = page(&app, Some(&pat), "US").await;
    assert!(shown(&p).contains_key("grown-rogue"), "{p}");
}

/// The page is the same all day and reshuffles from one day to the next; the general reels take
/// turns with the personal ones after For you.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn discover_order_holds_for_a_day(db: PgPool) {
    let app = test_app(db).await;
    let (pat_id, pat) = store_with_pat(&app).await;
    let a = page(&app, Some(&pat), "US").await;
    let b = page(&app, Some(&pat), "US").await;
    assert_eq!(a, b, "the same page twice in a day");
    assert_eq!(page(&app, None, "US").await, page(&app, None, "US").await);

    let day = |d: u8| time::Date::from_calendar_date(2026, time::Month::November, d).unwrap();
    let order = |p: &discover::Discover| p.reels.iter().map(|r| r.key.clone()).collect::<Vec<_>>();
    let on = |d: u8| {
        let app = &app;
        async move {
            discover::page(
                &app.db,
                &app.config,
                Some(pat_id),
                slop_api::policy::ViewerPolicy::FULL_VIEW,
                day(d),
            )
            .await
            .unwrap()
        }
    };
    let first = order(&on(1).await);
    assert_eq!(first, order(&on(1).await));
    let mut changed = false;
    for d in 2..=10 {
        changed |= order(&on(d).await) != first;
    }
    assert!(changed, "the order moves from day to day: {first:?}");

    // Taking turns: personal, general, personal, general…, the rest after.
    let personal: Vec<String> = ["p1", "p2", "p3"].map(String::from).to_vec();
    let general: Vec<String> = GENERAL.map(String::from).to_vec();
    for seed in [1, 2, 3, 99] {
        let mixed = discover::mix(&personal, &general, seed);
        assert_eq!(mixed.len(), 7);
        for (i, k) in mixed.iter().enumerate() {
            let is_personal = k.starts_with('p');
            assert_eq!(is_personal, i < 6 && i % 2 == 0, "{mixed:?}");
        }
        assert_eq!(mixed, discover::mix(&personal, &general, seed));
    }
    let orders: HashSet<Vec<String>> = (0..20)
        .map(|seed| discover::mix(&[], &general, seed))
        .collect();
    assert!(
        orders.len() > 1,
        "the general reels' order depends on the seed"
    );
}

/// On the simulation's store (many players, votes, play and donations), for every player: never
/// what they have, did, made or donated to, never a dead project, at most two reels per app.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn discover_keeps_its_rules_on_the_simulated_store(db: PgPool) {
    let app = test_app(db).await;
    slop_api::sim::generate(
        &app.db,
        slop_api::sim::Settings {
            apps: 120,
            users: 120,
            days: 40,
            seed: 7,
        },
    )
    .await
    .unwrap();
    let users: Vec<Uuid> = sqlx::query_scalar(
        "select id from users where exists (select 1 from library_entries l where l.user_id = users.id)
         order by id limit 40",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert!(users.len() >= 20);
    let dead: HashSet<String> = sqlx::query_scalar(
        "select slug from apps where stage <> 'released' and last_commit_at < now() - interval '90 days'",
    )
    .fetch_all(&app.db)
    .await
    .unwrap()
    .into_iter()
    .collect();
    let (mut genre_reels, mut donated_reels) = (0, 0);
    for user in users {
        let p = discover::page(
            &app.db,
            &app.config,
            Some(user),
            slop_api::policy::ViewerPolicy::FULL_VIEW,
            time::Date::from_calendar_date(2026, time::Month::November, 3).unwrap(),
        )
        .await
        .unwrap();
        if !p.personalized {
            continue;
        }
        let theirs: HashSet<String> = sqlx::query_scalar(
            "select a.slug from apps a where a.maker_id = $1
                or exists (select 1 from library_entries l where l.app_id = a.id and l.user_id = $1)
                or exists (select 1 from interests i where i.app_id = a.id and i.user_id = $1)
                or exists (select 1 from user_app_usage u where u.app_id = a.id and u.user_id = $1)
                or exists (select 1 from votes v where v.app_id = a.id and v.user_id = $1)
                or exists (select 1 from donations d where d.app_id = a.id and d.donor_id = $1
                           and d.status in ('paid', 'partially_refunded'))",
        )
        .bind(user)
        .fetch_all(&app.db)
        .await
        .unwrap()
        .into_iter()
        .collect();
        assert_eq!(p.reels[0].key, "for_you");
        let mut count: HashMap<&str, usize> = HashMap::new();
        for r in &p.reels {
            assert!(!r.why.is_empty() && !r.title.is_empty());
            assert!(r.apps.len() <= discover::REEL);
            genre_reels += usize::from(r.key.starts_with("genre:"));
            donated_reels += usize::from(r.key.starts_with("genre:donated:"));
            for t in &r.apps {
                let slug = t.app.slug.as_str();
                assert!(!theirs.contains(slug), "{} shows {slug}", r.key);
                assert!(!dead.contains(slug), "{} shows dead {slug}", r.key);
                assert!(!t.app.is_dead);
                *count.entry(slug).or_default() += 1;
            }
        }
        assert!(count.values().all(|n| *n <= 2), "{count:?}");
    }
    assert!(genre_reels > 20, "{genre_reels}");
    assert!(donated_reels > 0, "the simulation's donors get one");
}
