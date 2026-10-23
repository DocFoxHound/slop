// SPDX-License-Identifier: AGPL-3.0-or-later
//! A throwaway Slop for the browser tests (web/e2e). Never for real use.
//!
//! Makes a new database (`slop_e2e_<random>_sim`) on the test Postgres, migrates and seeds it
//! (a maker, a player, an admin, a few games), starts a fake GitHub that serves release files,
//! and runs the API with the release fetcher on a free port, serving the built website
//! (`web/dist`). Prints one line of JSON (`url`, `cookie_name`, `cookies`, `fake`) once it's
//! ready, and drops the database when stopped (Ctrl-C, SIGTERM, or stdin closing).
//!
//! `cargo run -p slop-api --example e2e_server` with `DATABASE_URL` pointing at the test server.
//! `SLOP_E2E_SIMULATE=<apps>,<users>` fills the store with the simulation first (the load test).

use std::{io::Write as _, net::SocketAddr, path::PathBuf};

use anyhow::Context;
use axum::{Router, body::Bytes, extract::Path, routing::get};
use slop_api::{
    AppState, MIGRATOR,
    auth::session,
    config::{CatalogRules, Config, GithubConfig, ReleaseConfig},
    router,
};
use sqlx::{Connection, Executor, PgConnection, PgPool};
use uuid::Uuid;

/// A tiny Windows build: a `.zip` with the program to start.
fn release_zip() -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(&mut buf);
    let opts: zip::write::SimpleFileOptions = Default::default();
    zip.start_file("pong.exe", opts).unwrap();
    zip.write_all(b"MZ e2e test build").unwrap();
    zip.finish().unwrap();
    buf.into_inner()
}

/// Stands in for GitHub: release files under any `/<owner>/<repo>/releases/download/...`.
async fn fake_github() -> anyhow::Result<String> {
    let zip = Bytes::from(release_zip());
    let app = Router::new().route(
        "/{owner}/{repo}/releases/download/{tag}/{file}",
        get(
            move |Path((_, _, _, file)): Path<(String, String, String, String)>| {
                let zip = zip.clone();
                async move {
                    if file.ends_with(".zip") {
                        Ok(zip)
                    } else {
                        Err(axum::http::StatusCode::NOT_FOUND)
                    }
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(format!("http://{addr}/"))
}

async fn user(
    db: &PgPool,
    config: &Config,
    name: &str,
    staff: bool,
) -> anyhow::Result<(Uuid, String)> {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into users (id, display_name, country_code, age_16_confirmed_at) values ($1, $2, 'US', now())",
    )
    .bind(id)
    .bind(name)
    .execute(db)
    .await?;
    sqlx::query(
        "insert into legal_acceptances (user_id, kind, version)
         select $1, kind, version from legal_documents where kind in ('terms', 'publisher', 'people')",
    )
    .bind(id)
    .execute(db)
    .await?;
    if staff {
        sqlx::query("update users set is_admin = true where id = $1")
            .bind(id)
            .execute(db)
            .await?;
        sqlx::query("insert into staff_roles (user_id, role, granted_by) values ($1, 'owner', $1)")
            .bind(id)
            .execute(db)
            .await?;
    }
    let cookie = session::create(db, config, id, Some("e2e")).await?;
    Ok((id, cookie.value().to_string()))
}

/// A listed game by `maker`, with an approved release.
async fn listed(
    db: &PgPool,
    maker: Uuid,
    slug: &str,
    title: &str,
    extra: &str,
) -> anyhow::Result<()> {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into apps (id, slug, title, summary, description_md, kind, stage, status, maker_id, repo_url,
                           license_spdx, platforms, last_commit_at)
         values ($1, $2, $3, $4, $5, 'game', 'released', 'listed', $6, $7, 'MIT', '{windows}', now())",
    )
    .bind(id)
    .bind(slug)
    .bind(title)
    .bind(format!("{title}: a game for the browser tests."))
    .bind(format!("# {title}\n\nMade for Slop's browser tests."))
    .bind(maker)
    .bind(format!("https://github.com/e2e/{slug}"))
    .execute(db)
    .await?;
    if !extra.is_empty() {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "update apps set {extra} where id = $1"
        )))
        .bind(id)
        .execute(db)
        .await?;
    }
    sqlx::query(
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by, reviewed_at, commit_verified)
         values ($1, $2, '1.0.0', repeat('a', 40), 'approved', $3, now(), true)",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(maker)
    .execute(db)
    .await?;
    Ok(())
}

/// A thread an account's AI agent started with every admin, with a token made 12 days before.
async fn agent_message(
    db: &PgPool,
    sender: Uuid,
    category: &str,
    subject: &str,
    body: &str,
) -> anyhow::Result<()> {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into admin_threads (id, sender_id, via, category, subject) values ($1, $2, 'maker_agent', $3, $4)",
    )
    .bind(id)
    .bind(sender)
    .bind(category)
    .bind(subject)
    .execute(db)
    .await?;
    sqlx::query(
        "insert into admin_thread_messages (id, thread_id, author_id, from_staff, by_agent, body,
                token_kind, token_id, token_name, token_scope, token_created_at)
         values ($1, $2, $3, false, true, $4, 'maker', $5, 'Claude Code', 'read', now() - interval '12 days 1 hour')",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(sender)
    .bind(body)
    .bind(Uuid::now_v7())
    .execute(db)
    .await?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "slop_api=warn,e2e_server=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let server = std::env::var("DATABASE_URL").context("DATABASE_URL: the test Postgres")?;
    let name = format!("slop_e2e_{:08x}_sim", rand::random::<u32>());
    let mut admin_conn = PgConnection::connect(&server).await?;
    admin_conn
        .execute(sqlx::AssertSqlSafe(format!("create database {name}")))
        .await?;
    let mut url = url::Url::parse(&server)?;
    url.set_path(&name);
    let db = PgPool::connect(url.as_str()).await?;
    let result = run(&db).await;
    db.close().await;
    admin_conn
        .execute(sqlx::AssertSqlSafe(format!(
            "drop database {name} with (force)"
        )))
        .await?;
    result
}

async fn run(db: &PgPool) -> anyhow::Result<()> {
    MIGRATOR.run(db).await?;
    slop_api::legal::sync(db).await?;
    let fake = fake_github().await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr: SocketAddr = listener.local_addr()?;
    let web_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/dist");
    anyhow::ensure!(
        web_dir.join("index.html").exists(),
        "build the website first: cd web && npm run build"
    );
    let config = Config {
        bind: addr,
        database_url: String::new(),
        migrate_database_url: None,
        bad_link_feeds: vec![],
        public_url: format!("http://{addr}").parse()?,
        github: Some(GithubConfig {
            client_id: "e2e".into(),
            client_secret: "e2e".into(),
            oauth_base: fake.parse()?,
            api_base: fake.parse()?,
        }),
        google: None,
        gitlab: None,
        bitbucket: None,
        microsoft: None,
        discord: None,
        trusted_origins: vec![],
        country_header: None,
        catalog: CatalogRules::default(),
        releases: ReleaseConfig {
            signing_key: Some(slop_manifest::ed25519_dalek::SigningKey::from_bytes(
                &[7; 32],
            )),
            max_file_bytes: 10 * 1024 * 1024,
            github_api_base: fake.parse()?,
            allow_http_downloads: true,
            require_maker_signature: false,
            scanner: slop_api::scan::Scanner::Off,
            slop_build_workflow: None,
            slop_build_version: "v1".into(),
            github_oidc_issuer: fake.trim_end_matches('/').to_string(),
            github_token: None,
            github_app: None,
            require_repo_owner: false,
            repo_token_key: Some([9; 32]),
            osv_api_base: fake.parse()?,
            source_hosts: vec![],
        },
        // Browser tests, and the load test's pretend players, all come from one address.
        rate_limits: slop_api::ratelimit::Limits {
            auth_per_minute: 1_000_000,
            write_per_minute: 1_000_000,
            read_per_minute: 1_000_000,
        },
        desktop: Default::default(),
        demos: Default::default(),
        media: slop_api::config::MediaConfig {
            github_raw_base: format!("{fake}raw/").parse()?,
        },
        // Simulated, as on the simulation sandbox: the browser tests pledge to bounties.
        payments: slop_api::config::PaymentsConfig {
            simulated: true,
            ..Default::default()
        },
        email: None,
        push: None,
        doh_url: format!("{fake}dns-query").parse()?,
        web_dir: Some(web_dir),
        client_ip_header: None,
        simulation: false,
        ops: Default::default(),
        approvals_required: 1,
        field_key: Some([11; 32]),
        security_contact: None,
        source_url: None,
    };

    // The load test wants a full store: the simulation's games, players, votes and history.
    if let Ok(size) = std::env::var("SLOP_E2E_SIMULATE") {
        let (apps, users) = size
            .split_once(',')
            .and_then(|(a, u)| Some((a.parse().ok()?, u.parse().ok()?)))
            .context("SLOP_E2E_SIMULATE is <apps>,<users>")?;
        slop_api::sim::generate(
            db,
            slop_api::sim::Settings {
                apps,
                users,
                ..Default::default()
            },
        )
        .await?;
    }
    let (maker, maker_cookie) = user(db, &config, "Mia Maker", false).await?;
    let (_, player_cookie) = user(db, &config, "Pat Player", false).await?;
    let (_, admin_cookie) = user(db, &config, "Ada Admin", true).await?;
    let (other, _) = user(db, &config, "Sam Seed", false).await?;
    // A publisher past their passkey deadline, with no passkey (passkeys.rs).
    let (overdue, overdue_cookie) = user(db, &config, "Olly Overdue", false).await?;
    sqlx::query("update users set passkey_due_at = now() - interval '1 day' where id = $1")
        .bind(overdue)
        .execute(db)
        .await?;
    listed(db, other, "castle-rock", "Castle Rock", "").await?;
    listed(
        db,
        other,
        "neon-drift",
        "Neon Drift",
        "online = '{player_servers}'",
    )
    .await?;
    listed(db, maker, "mias-first", "Mia's First Game", "").await?;
    // Messages Mia's AI agent wrote to the admins (admin_messages.rs), for the AI messages tab.
    agent_message(
        db,
        maker,
        "report",
        "Release upload times out",
        "Submitting 1.2.0 timed out after 30 s.\nLog: https://example.com/log <b>not bold</b>",
    )
    .await?;
    agent_message(
        db,
        maker,
        "suggestion",
        "Show scan progress",
        "A percentage while files are scanned would help.",
    )
    .await?;

    let state = AppState::new(db.clone(), config.clone())?;
    tokio::spawn(slop_api::releases::run_fetcher(state.clone()));
    tokio::spawn(slop_api::bounties::run_charger(state.clone()));
    let app = router(state)?;
    // As the real server does (main.rs).
    let listener = axum::serve::ListenerExt::tap_io(listener, |tcp| {
        let _ = tcp.set_nodelay(true);
    });
    tokio::spawn(async move { axum::serve(listener, app).await });

    let cookie_name = session::cookie_name(&config);
    println!(
        "{}",
        serde_json::json!({
            "url": format!("http://{addr}"),
            "fake": fake,
            "cookie_name": cookie_name,
            "cookies": { "maker": maker_cookie, "player": player_cookie, "admin": admin_cookie, "overdue": overdue_cookie },
        })
    );
    let _ = maker;

    // Until stopped, or until whoever started it goes away.
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let stdin_closed = tokio::task::spawn_blocking(|| {
        let mut sink = String::new();
        while std::io::stdin().read_line(&mut sink).is_ok_and(|n| n > 0) {}
    });
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
        _ = stdin_closed => {}
    }
    Ok(())
}
