// SPDX-License-Identifier: AGPL-3.0-or-later
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use slop_api::{AppState, MIGRATOR, admin, catalog::AppStage, config::Config, releases, router};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

/// The SLAPorSLOP API server, plus the staff commands for reviewing releases.
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the API server and the release fetcher (the default).
    Serve,
    /// Run database migrations as the tables' owner (SLOP_MIGRATE_DATABASE_URL), then give the
    /// API's role (SLOP_APP_ROLE, default slop_app) its rights. The deployment runs this as its own
    /// step (slop-migrate.service), so the API never holds the owner's password.
    Migrate,
    /// Signing keys.
    Keys {
        #[command(subcommand)]
        command: KeysCommand,
    },
    /// Review releases and listings.
    Review {
        #[command(subcommand)]
        command: ReviewCommand,
    },
    /// Desktop app builds made on this server.
    Desktop {
        #[command(subcommand)]
        command: DesktopCommand,
    },
    /// Give or take away admin rights (the website's admin panel).
    Admin {
        #[command(subcommand)]
        command: AdminCommand,
    },
    /// The simulation sandbox (only on a database whose name ends in _sim).
    Simulate {
        #[command(subcommand)]
        command: SimulateCommand,
    },
    /// Revoke an approved release: it stops launching on every client (SECURITY §5.5).
    Revoke {
        release_id: Uuid,
        #[arg(long)]
        reviewer: String,
        #[arg(long)]
        reason: String,
    },
}

#[derive(Subcommand)]
enum SimulateCommand {
    /// Wipe the sandbox and make a new simulated store: apps, players, libraries, votes and
    /// player counts over the past days.
    Generate {
        #[arg(long, default_value_t = 400)]
        apps: usize,
        #[arg(long, default_value_t = 1500)]
        users: usize,
        #[arg(long, default_value_t = 120)]
        days: i64,
        #[arg(long, default_value_t = 42)]
        seed: u64,
    },
    /// Score recommendations, ratings and player counts against the simulated truth. Prints JSON.
    Report {
        #[arg(long, default_value_t = 400)]
        sample: usize,
        #[arg(long, default_value_t = 7)]
        seed: u64,
    },
    /// Keep "playing now" alive: refresh the simulated live players every minute.
    Pulse,
}

#[derive(Subcommand)]
enum DesktopCommand {
    /// Sign and publish a build for the app's auto-updater.
    Publish {
        /// windows, mac, linux-deb or linux-rpm.
        #[arg(long)]
        platform: String,
        #[arg(long)]
        version: String,
        /// The full commit SHA it was built from.
        #[arg(long)]
        commit: String,
        file: std::path::PathBuf,
    },
}

#[derive(Subcommand)]
enum AdminCommand {
    /// Make an account an admin (owner role). WHO is its account ID (from its profile link).
    Grant {
        who: String,
        #[arg(long)]
        reviewer: String,
    },
    /// Take admin rights away.
    Revoke {
        who: String,
        #[arg(long)]
        reviewer: String,
    },
}

#[derive(Subcommand)]
enum KeysCommand {
    /// Make a new key for signing desktop app updates. Prints the secret (for SLOP_UPDATER_KEY)
    /// and the public key (for plugins.updater.pubkey in crates/slop-desktop/tauri.conf.json).
    GenerateUpdater,
    /// Make a new Ed25519 signing key. Prints the secret (for SLOP_SIGNING_KEY) and the public key
    /// (for SLOP_TRUSTED_KEYS when building the desktop app).
    Generate,
    /// Make a new VAPID key for phone and browser notifications (SLOP_VAPID_PRIVATE_KEY).
    GenerateVapid,
}

#[derive(Subcommand)]
enum ReviewCommand {
    /// Releases waiting for review.
    Pending,
    /// Approve a release and sign its manifest.
    Approve {
        release_id: Uuid,
        #[arg(long)]
        reviewer: String,
    },
    /// Reject a release. The reason is shown to the maker.
    Reject {
        release_id: Uuid,
        #[arg(long)]
        reviewer: String,
        #[arg(long)]
        reason: String,
    },
    /// Put a reviewed app in the store.
    List {
        slug: String,
        #[arg(long)]
        reviewer: String,
    },
    /// Take an app out of the store. The maker gets a statement of reasons and can appeal.
    Delist {
        slug: String,
        #[arg(long)]
        reviewer: String,
        /// The Content Policy rule it breaks (moderation.rs RULES), e.g. malware.
        #[arg(long)]
        rule: String,
        /// The facts, as the maker will read them.
        #[arg(long)]
        reason: String,
    },
    /// Move an app to coming-soon, beta or released.
    Stage {
        slug: String,
        #[arg(value_parser = parse_stage)]
        stage: AppStage,
        #[arg(long)]
        reviewer: String,
    },
}

fn parse_stage(s: &str) -> Result<AppStage, String> {
    match s {
        "coming-soon" | "coming_soon" => Ok(AppStage::ComingSoon),
        "beta" => Ok(AppStage::Beta),
        "released" => Ok(AppStage::Released),
        _ => Err("use coming-soon, beta or released".into()),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "slop_api=info,tower_http=info".into()),
        )
        .init();

    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => serve().await,
        Command::Migrate => migrate().await,
        Command::Keys {
            command: KeysCommand::GenerateUpdater,
        } => {
            let (secret, public) = slop_api::desktop_updates::generate_updater_key()?;
            println!("SLOP_UPDATER_KEY={secret}");
            println!("updater pubkey: {public}");
            eprintln!("Keep SLOP_UPDATER_KEY secret: it signs every desktop update.");
            Ok(())
        }
        Command::Keys {
            command: KeysCommand::GenerateVapid,
        } => {
            let (private, public) = slop_api::push::generate();
            println!("SLOP_VAPID_PRIVATE_KEY={private}");
            println!("public key: {public}");
            eprintln!(
                "Keep SLOP_VAPID_PRIVATE_KEY secret. Changing it means everyone turns phone notifications on again."
            );
            Ok(())
        }
        Command::Keys {
            command: KeysCommand::Generate,
        } => {
            let key =
                slop_manifest::ed25519_dalek::SigningKey::from_bytes(&rand::random::<[u8; 32]>());
            println!(
                "SLOP_SIGNING_KEY={}",
                slop_manifest::signing_key_to_base64(&key)
            );
            println!(
                "SLOP_TRUSTED_KEYS={}",
                slop_manifest::verifying_key_to_base64(&key.verifying_key())
            );
            println!("key id: {}", slop_manifest::key_id(&key.verifying_key()));
            eprintln!(
                "Keep SLOP_SIGNING_KEY secret. Production keys belong in a KMS (docs/SECURITY.md §5.3)."
            );
            Ok(())
        }
        Command::Simulate { command } => {
            let (config, db) = connect().await?;
            match command {
                SimulateCommand::Generate {
                    apps,
                    users,
                    days,
                    seed,
                } => {
                    let made = slop_api::sim::generate(
                        &db,
                        slop_api::sim::Settings {
                            apps,
                            users,
                            days,
                            seed,
                        },
                    )
                    .await?;
                    println!("{}", serde_json::to_string_pretty(&made)?);
                }
                SimulateCommand::Report { sample, seed } => {
                    let report = slop_api::sim::report(&db, &config, sample, seed).await?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                }
                SimulateCommand::Pulse => {
                    let mut rng = slop_api::sim::Rng::new(rand::random());
                    loop {
                        match slop_api::sim::pulse_tick(&db, &mut rng).await {
                            Ok(live) => tracing::info!(
                                players = live.values().sum::<i64>(),
                                apps = live.len(),
                                "simulated players"
                            ),
                            Err(err) => tracing::error!(error = ?err, "simulated players"),
                        }
                        tokio::time::sleep(Duration::from_secs(60)).await;
                    }
                }
            }
            Ok(())
        }
        Command::Review { command } => {
            let (config, db) = connect().await?;
            match command {
                ReviewCommand::Pending => {
                    let pending = admin::pending(&db).await?;
                    if pending.is_empty() {
                        println!("Nothing waiting for review.");
                    }
                    if !config.releases.scanner.is_on() {
                        println!("Note: scanning is off on this server; files are NOT scanned.");
                    }
                    for r in pending {
                        println!(
                            "{}  {} {}  commit {}{}  {}",
                            r.id,
                            r.app_slug,
                            r.version,
                            &r.commit_sha[..12],
                            if r.commit_verified {
                                " (found in repo)"
                            } else {
                                " (NOT checked)"
                            },
                            r.repo_url
                        );
                    }
                }
                ReviewCommand::Approve {
                    release_id,
                    reviewer,
                } => {
                    admin::approve(
                        &db,
                        config.releases.signing_key.as_ref(),
                        admin::ReviewRules::from_config(&config),
                        release_id,
                        &reviewer,
                    )
                    .await?;
                    println!("Approved and signed {release_id}.");
                }
                ReviewCommand::Reject {
                    release_id,
                    reviewer,
                    reason,
                } => {
                    admin::reject(&db, release_id, &reviewer, &reason).await?;
                    println!("Rejected {release_id}.");
                }
                ReviewCommand::List { slug, reviewer } => {
                    admin::list_app(&db, &slug, &reviewer).await?;
                    println!("{slug} is in the store.");
                }
                ReviewCommand::Delist {
                    slug,
                    reviewer,
                    rule,
                    reason,
                } => {
                    admin::delist_app(&db, &slug, &reviewer, &rule, &reason).await?;
                    println!("{slug} is out of the store.");
                }
                ReviewCommand::Stage {
                    slug,
                    stage,
                    reviewer,
                } => {
                    admin::set_stage(&db, &slug, stage, &reviewer).await?;
                    println!("{slug} moved to {stage:?}.");
                }
            }
            Ok(())
        }
        Command::Desktop {
            command:
                DesktopCommand::Publish {
                    platform,
                    version,
                    commit,
                    file,
                },
        } => {
            let (config, db) = connect().await?;
            let (Some(dir), Some(key)) = (&config.desktop.dir, &config.desktop.updater_key) else {
                anyhow::bail!("set SLOP_DESKTOP_DIR and SLOP_UPDATER_KEY first");
            };
            let version = semver::Version::parse(&version)?;
            let published = slop_api::desktop_updates::publish(
                &db, dir, key, &platform, &version, &commit, &file,
            )
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            println!(
                "Published {} {} ({})",
                platform, published.version, published.file_name
            );
            Ok(())
        }
        Command::Admin { command } => {
            let (_, db) = connect().await?;
            match command {
                AdminCommand::Grant { who, reviewer } => {
                    let name = admin::set_admin(&db, &who, true, &reviewer).await?;
                    println!("{name} is now an admin.");
                }
                AdminCommand::Revoke { who, reviewer } => {
                    let name = admin::set_admin(&db, &who, false, &reviewer).await?;
                    println!("{name} is no longer an admin.");
                }
            }
            Ok(())
        }
        Command::Revoke {
            release_id,
            reviewer,
            reason,
        } => {
            let (_, db) = connect().await?;
            admin::revoke(&db, release_id, &reviewer, &reason).await?;
            println!(
                "Revoked {release_id}. Clients stop launching it at their next revocation check."
            );
            Ok(())
        }
    }
}

async fn connect() -> anyhow::Result<(Config, PgPool)> {
    let config = Config::from_env()?;
    let db = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&config.database_url)
        .await
        .context("connecting to Postgres")?;
    match &config.migrate_database_url {
        // Migrations as the tables' owner; the API keeps its own role, with the ledgers
        // insert-only (migration 0050).
        Some(owner_url) => {
            let owner = PgPoolOptions::new()
                .max_connections(1)
                .connect(owner_url)
                .await
                .context("connecting to Postgres as the owner (SLOP_MIGRATE_DATABASE_URL)")?;
            MIGRATOR.run(&owner).await.context("running migrations")?;
            let role: String = sqlx::query_scalar("select current_user::text")
                .fetch_one(&db)
                .await?;
            sqlx::query("select slop_grant_app_role($1)")
                .bind(&role)
                .execute(&owner)
                .await
                .context("granting the API's role")?;
            owner.close().await;
        }
        // Migrations ran as their own step (`slop-api migrate`): only check they did.
        None if std::env::var("SLOP_MIGRATIONS").as_deref() == Ok("external") => {
            let applied: i64 = sqlx::query_scalar(
                "select coalesce(max(version), 0) from _sqlx_migrations where success",
            )
            .fetch_one(&db)
            .await
            .context("reading which migrations ran")?;
            let needed = MIGRATOR.iter().map(|m| m.version).max().unwrap_or(0);
            anyhow::ensure!(
                applied >= needed,
                "the database is at migration {applied} but this build needs {needed}: run `slop-api migrate`"
            );
        }
        None => MIGRATOR.run(&db).await.context("running migrations")?,
    }
    let owns: i64 = sqlx::query_scalar(
        "select count(*) from pg_tables where schemaname = 'public' and tableowner = current_user",
    )
    .fetch_one(&db)
    .await?;
    if owns > 0 {
        tracing::warn!(
            "the API owns its tables, so payments and votes are guarded by triggers only: \
             see deploy/README.md, \"Database roles\""
        );
    }
    slop_api::legal::sync(&db)
        .await
        .context("publishing legal documents")?;
    Ok((config, db))
}

async fn migrate() -> anyhow::Result<()> {
    let owner_url = std::env::var("SLOP_MIGRATE_DATABASE_URL")
        .context("SLOP_MIGRATE_DATABASE_URL (the tables' owner) isn't set")?;
    let role = std::env::var("SLOP_APP_ROLE").unwrap_or_else(|_| "slop_app".into());
    let owner = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(30))
        .connect(&owner_url)
        .await
        .context("connecting to Postgres as the owner")?;
    MIGRATOR.run(&owner).await.context("running migrations")?;
    let exists: bool =
        sqlx::query_scalar("select exists (select 1 from pg_roles where rolname = $1)")
            .bind(&role)
            .fetch_one(&owner)
            .await?;
    if exists {
        sqlx::query("select slop_grant_app_role($1)")
            .bind(&role)
            .execute(&owner)
            .await
            .context("granting the API's role")?;
    } else {
        tracing::warn!(%role, "the API's role doesn't exist yet: see deploy/README.md, \"Database roles\"");
    }
    tracing::info!("migrations are up to date");
    Ok(())
}

async fn serve() -> anyhow::Result<()> {
    let (config, db) = connect().await?;
    if config.github.is_none() {
        tracing::warn!("GITHUB_CLIENT_ID / GITHUB_CLIENT_SECRET not set: sign-in is off");
    }
    if config.releases.signing_key.is_none() {
        tracing::warn!(
            "SLOP_SIGNING_KEY not set: releases can't be approved and there's no revocation list"
        );
    }

    if config.simulation {
        let name = slop_api::sim::ensure_sandbox(&db)
            .await
            .context("SLOP_SIMULATION is only allowed on a sandbox database")?;
        tracing::warn!(database = %name, "simulation sandbox: anyone can view as a simulated user");
    }
    // Simulated People payments only on the sandbox or a plain-http development server.
    if config.payments.simulated && config.payments.stripe.is_none() && !config.simulation {
        anyhow::ensure!(
            !config.secure_cookies(),
            "SLOP_PAYMENTS=simulated is only for development servers (http), never a public site"
        );
        tracing::warn!("People payments are simulated: no money moves");
    }
    if config.payments.stripe.is_none() && !config.payments.simulated {
        tracing::warn!(
            "STRIPE_SECRET_KEY / STRIPE_WEBHOOK_SECRET not set: People payments are off"
        );
    }
    for a in slop_api::ops::config_alerts(&config) {
        tracing::warn!(problem = %a.title, "{}", a.detail);
    }
    let bind = config.bind;
    let state = AppState::new(db, config)?;
    tokio::spawn(releases::run_fetcher(state.clone()));
    tokio::spawn(slop_api::bounties::run_charger(state.clone()));
    tokio::spawn(slop_api::repo_identity::run_fill(state.clone()));
    tokio::spawn(slop_api::updates::run_watcher(state.clone()));
    tokio::spawn(slop_api::cleanup::run(state.clone()));
    tokio::spawn(slop_api::pulse::run_sampler(state.clone()));
    tokio::spawn(slop_api::notify::run_email(state.clone()));
    tokio::spawn(slop_api::ops::run(state.clone()));
    match slop_api::fields::seal_existing(&state).await {
        Ok(0) => {}
        Ok(n) => tracing::info!(n, "encrypted stored VAT numbers"),
        Err(err) => tracing::error!(error = %err, "encrypting stored fields"),
    }
    tokio::spawn(slop_api::rescan::run(state.clone()));
    tokio::spawn(slop_api::abuse::run(state.clone()));
    tokio::spawn(slop_api::source_scan::run(state.clone()));
    tokio::spawn(slop_api::dep_watch::run(state.clone()));
    tokio::spawn(slop_api::push::run(state.clone()));
    tokio::spawn(slop_api::donations::run(state.clone()));
    tokio::spawn(slop_api::teams::run(state.clone()));
    // The sandbox's repositories are invented, so checking them would freeze every app.
    if !state.config.simulation {
        tokio::spawn(slop_api::frozen::run(state.clone()));
    }
    let app = router(state)?;
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    tracing::info!(%bind, "SLAPorSLOP API listening");
    // Send each response as soon as it's written: without this, small responses streamed in
    // pieces (the website's files) wait ~40 ms for the client's delayed acknowledgement.
    let listener = axum::serve::ListenerExt::tap_io(listener, |tcp| {
        let _ = tcp.set_nodelay(true);
    });
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown())
    .await?;
    Ok(())
}

async fn shutdown() {
    let ctrl_c = async { tokio::signal::ctrl_c().await.ok() };
    #[cfg(unix)]
    let term = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler")
            .recv()
            .await
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}
