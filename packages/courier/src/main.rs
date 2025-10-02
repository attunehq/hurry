use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use atomic_time::AtomicInstant;
use axum::{routing::get, Router};
use clap::Parser;
use color_eyre::Result;
use tap::Pipe;
use tower::ServiceBuilder;
use tower_http::{limit::RequestBodyLimitLayer, timeout::TimeoutLayer, trace::TraceLayer};
use tracing::level_filters::LevelFilter;
use tracing_error::ErrorLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tracing_tree::time::FormatTime;

mod api;
mod auth;
mod cache;
mod db;
mod storage;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Config {
    /// Database URL (Postgres)
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// Port to listen on
    #[arg(long, env = "PORT", default_value = "3000")]
    port: u16,

    /// Host to bind to
    #[arg(long, env = "HOST", default_value = "0.0.0.0")]
    host: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::parse();
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

    // API health limits from RFC:
    // - Max request deadline: 15 seconds ✓
    // - Max requests in flight: 1,000 (TODO: add via load balancer or global semaphore)
    // - Pending queue: 100 (TODO: add via load balancer or global semaphore)
    // - Max body size: 100MiB ✓
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
    const MAX_BODY_SIZE: usize = 100 * 1024 * 1024; // 100 MiB

    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .nest("/api/v1", api::routes())
        .layer(
            ServiceBuilder::new()
                // HTTP request tracing
                .layer(TraceLayer::new_for_http())
                // Body size limit: reject request bodies larger than 100MiB
                .layer(RequestBodyLimitLayer::new(MAX_BODY_SIZE))
                // Timeout: reject requests taking longer than 15 seconds
                .layer(TimeoutLayer::new(REQUEST_TIMEOUT)),
        );

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;

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
