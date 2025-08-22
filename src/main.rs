//! The binary entrypoint for `hurry`, the ultra-fast build tool.

use cargo_metadata::camino::Utf8PathBuf;
use clap::{Parser, Subcommand};
use color_eyre::{Result, eyre::Context};
use tracing::{instrument, level_filters::LevelFilter};
use tracing_error::ErrorLayer;
use tracing_flame::FlameLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

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
