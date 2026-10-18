use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use slop_api::{AppState, MIGRATOR, admin, catalog::AppStage, config::Config, releases, router};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

/// The Slop API server, plus the staff commands for reviewing releases.
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
    /// Make an account an admin. WHO is its ID, email or exact display name.
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
    /// Take an app out of the store.
    Delist {
        slug: String,
        #[arg(long)]
        reviewer: String,
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
                    reason,
                } => {
                    admin::delist_app(&db, &slug, &reviewer, &reason).await?;
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
    MIGRATOR.run(&db).await.context("running migrations")?;
    Ok((config, db))
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

    let bind = config.bind;
    let state = AppState::new(db, config)?;
    tokio::spawn(releases::run_fetcher(state.clone()));
    tokio::spawn(slop_api::cleanup::run(state.clone()));
    let app = router(state)?;
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    tracing::info!(%bind, "Slop API listening");
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
