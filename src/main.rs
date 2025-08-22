//! The binary entrypoint for `hurry`, the ultra-fast build tool.

use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use atomic_time::AtomicInstant;
use cargo_metadata::camino::Utf8PathBuf;
use clap::{Parser, Subcommand};
use color_eyre::{Result, eyre::Context};
use tap::Pipe;
use tracing::{instrument, level_filters::LevelFilter};
use tracing_error::ErrorLayer;
use tracing_flame::FlameLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tracing_tree::time::FormatTime;

// Since this is a binary crate, we need to ensure these modules aren't pub
// so that they can correctly warn about dead code:
// https://github.com/rust-lang/rust/issues/74970
//
// Relatedly, in this file specifically nothing should be `pub`.
mod cargo;
mod cas;
mod fs;
mod hash;

#[derive(Parser)]
#[command(name = "hurry", about = "Really, really fast builds", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Emit flamegraph profiling data
    #[arg(short, long, hide(true))]
    profile: Option<Utf8PathBuf>,
}

#[derive(Clone, Subcommand)]
enum Command {
    /// Fast `cargo` builds
    #[clap(subcommand)]
    Cargo(cargo::Command),
    // TODO: /// Manage remote authentication
    // Auth,

    // TODO: Manage user cache, including busting it when it gets into a corrupt or weird state.
    // Cache,
}

#[instrument]
fn main() -> Result<()> {
    let cli = Cli::parse();
    color_eyre::install()?;

    let (flame_layer, flame_guard) = if let Some(profile) = cli.profile {
        FlameLayer::with_file(&profile)
            .with_context(|| format!("set up profiling to {profile:?}"))
            .map(|(layer, guard)| (Some(layer), Some(guard)))?
    } else {
        (None, None)
    };

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
        .with(flame_layer)
        .init();

    let result = match cli.command {
        Command::Cargo(cmd) => match cmd {
            cargo::Command::Build(opts) => cargo::build::exec(opts),
            cargo::Command::Run(opts) => cargo::run::exec(opts),
        },
    };

    // TODO: Unsure if we need to keep this,
    // the guard _should_ flush on drop.
    if let Some(flame_guard) = flame_guard {
        flame_guard.flush().context("flush flame_guard")?;
    }

    result
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
        // We don't want to make users jump around to read messages, so
        // we pad the decimal part of the second.
        // Seconds going from single to double digits, then to triples,
        // will cause the overall message to shift but this isn't the same
        // as "jumping around" so it's fine.
        write!(w, "{seconds:.03}s")
    }

    // Elapsed here is the total time _in this span_,
    // but we want "the time since the last message was printed"
    // so we use `self.prior`.
    fn style_timestamp(
        &self,
        _ansi: bool,
        _elapsed: std::time::Duration,
        w: &mut impl std::fmt::Write,
    ) -> std::fmt::Result {
        // We expect the vast majority of events to be less than 999ms apart,
        // but expect a fair amount of variance between 1 and 3 digits.
        // We don't want to make users jump around to read messages, so
        // we pad with spaces to 3 characters.
        //
        // If events actually do take >999ms, we want those to stand out,
        // so it's OK that they're a bit longer.
        let elapsed = self.elapsed_since_prior().as_millis();
        write!(w, "{elapsed: >3}ms")
    }
}
