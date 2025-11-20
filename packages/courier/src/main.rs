use std::path::PathBuf;

use aerosol::Aero;
use clap::Parser;
use color_eyre::{Result, eyre::Context};
use derive_more::Debug;
use tracing::level_filters::LevelFilter;
use tracing_error::ErrorLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::db::Postgres;

mod api;
mod auth;
mod crypto;
mod db;
mod storage;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Start the Courier API server
    Serve(ServeConfig),

    /// Apply database migrations
    Migrate(MigrateConfig),
}

#[derive(Parser, Debug)]
struct ServeConfig {
    /// Database URL
    #[arg(long, env = "COURIER_DATABASE_URL")]
    #[debug(ignore)]
    database_url: String,

    /// Port to listen on
    #[arg(long, env = "PORT", default_value = "3000")]
    port: u16,

    /// Host to bind to
    #[arg(long, env = "HOST", default_value = "0.0.0.0")]
    host: String,

    /// Root path to store CAS blobs
    #[arg(long, env = "CAS_ROOT")]
    cas_root: PathBuf,
}

#[derive(Parser, Debug)]
struct MigrateConfig {
    /// Database URL
    #[arg(long, env = "COURIER_DATABASE_URL")]
    #[debug(ignore)]
    database_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    color_eyre::install()?;

    tracing_subscriber::registry()
        .with(ErrorLayer::default())
        .with(
            tracing_subscriber::fmt::layer()
                .with_level(true)
                .with_file(true)
                .with_line_number(true)
                .with_target(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .pretty(),
        )
        .with(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .init();

    match cli.command {
        Command::Serve(config) => serve(config).await,
        Command::Migrate(config) => migrate(config).await,
    }
}

async fn serve(config: ServeConfig) -> Result<()> {
    tracing::info!("constructing application router...");
    let storage = storage::Disk::new(&config.cas_root);
    let db = Postgres::connect(&config.database_url)
        .await
        .context("connect to database")?;
    let router = api::router(Aero::new().with(storage).with(db));

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on {}", listener.local_addr()?);

    // Graceful shutdown: wait for SIGTERM or SIGINT, then allow in-flight
    // requests to complete with a grace period.
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("server shutdown complete");
    Ok(())
}

/// Wait for a shutdown signal (SIGTERM or SIGINT).
async fn shutdown_signal() {
    use tokio::signal;

    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("received SIGINT (Ctrl+C), starting graceful shutdown");
        },
        _ = terminate => {
            tracing::info!("received SIGTERM, starting graceful shutdown");
        },
    }
}

async fn migrate(config: MigrateConfig) -> Result<()> {
    tracing::info!("applying migrations...");

    let pool = Postgres::connect(&config.database_url)
        .await
        .context("connect to database")?;

    Postgres::MIGRATOR
        .run(pool.as_ref())
        .await
        .context("apply migrations")?;

    tracing::info!("migrations applied successfully");
    Ok(())
}
