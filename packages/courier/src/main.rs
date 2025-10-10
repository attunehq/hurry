use std::{
    path::PathBuf,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use aerosol::Aero;
use atomic_time::AtomicInstant;
use clap::Parser;
use color_eyre::{Result, eyre::Context};
use derive_more::Debug;
use tap::Pipe;
use tracing::level_filters::LevelFilter;
use tracing_error::ErrorLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tracing_tree::time::FormatTime;

use crate::{auth::KeySets, db::Postgres};

mod api;
mod auth;
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
            tracing_tree::HierarchicalLayer::default()
                .with_indent_lines(true)
                .with_indent_amount(2)
                .with_thread_ids(false)
                .with_thread_names(false)
                .with_verbose_exit(false)
                .with_verbose_entry(false)
                .with_deferred_spans(true)
                .with_bracketed_fields(true)
                .with_span_retrace(true)
                .with_timer(Uptime::default())
                .with_targets(false),
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
    let key_cache = KeySets::new();
    let router = api::router(Aero::new().with(key_cache).with(storage).with(db));

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

/// Prints the overall latency and latency between tracing events.
struct Uptime {
    start: Instant,
    prior: AtomicInstant,
}

impl Uptime {
    /// Get the [`Duration`] since the last time this function was called.
    /// Uses relaxed atomic ordering; this isn't meant to be super precise-
    /// just fast to run and good enough for humans to eyeball.
    ///
    /// If the function hasn't yet been called, it returns the time
    /// since the overall [`Uptime`] struct was created.
    fn elapsed_since_prior(&self) -> Duration {
        const RELAXED: Ordering = Ordering::Relaxed;
        self.prior
            .fetch_update(RELAXED, RELAXED, |_| Some(Instant::now()))
            .unwrap_or_else(|_| Instant::now())
            .pipe(|prior| prior.elapsed())
    }
}

impl Default for Uptime {
    fn default() -> Self {
        Self {
            start: Instant::now(),
            prior: AtomicInstant::now(),
        }
    }
}

impl FormatTime for Uptime {
    // Prints the total runtime for the program.
    fn format_time(&self, w: &mut impl std::fmt::Write) -> std::fmt::Result {
        let elapsed = self.start.elapsed();
        let seconds = elapsed.as_secs_f64();
        write!(w, "{seconds:.03}s")
    }

    // Elapsed here is the total time _in this span_,
    // but we want "the time since the last message was printed"
    // so we use `self.prior`.
    fn style_timestamp(
        &self,
        _ansi: bool,
        _elapsed: Duration,
        w: &mut impl std::fmt::Write,
    ) -> std::fmt::Result {
        let elapsed = self.elapsed_since_prior().as_millis();
        write!(w, "{elapsed: >3}ms")
    }
}
