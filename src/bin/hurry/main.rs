use std::path::PathBuf;

use clap::{Parser, Subcommand};
use color_eyre::{Result, eyre::Context};
use tracing::{instrument, level_filters::LevelFilter};
use tracing_flame::FlameLayer;
use tracing_subscriber::{
    Layer as _, fmt::format::FmtSpan, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

mod cargo;

#[derive(Parser)]
#[command(name = "hurry", about = "Really, really fast builds", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Emit flamegraph profiling data
    #[arg(short, long, hide(true))]
    profile: Option<PathBuf>,
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

    let (flame_layer, flame_guard) = if let Some(profile) = cli.profile {
        FlameLayer::with_file(&profile)
            .with_context(|| format!("set up profiling to {profile:?}"))
            .map(|(layer, guard)| (Some(layer), Some(guard)))?
    } else {
        (None, None)
    };

    let filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(LevelFilter::WARN.into())
        .from_env_lossy();
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
                .with_file(true)
                .with_line_number(true)
                .with_target(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .with_writer(std::io::stderr)
                .pretty()
                .with_filter(filter),
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
