//! The binary entrypoint for `hurry`, the ultra-fast build tool.

use std::path::PathBuf;

use clap::{Parser, Subcommand, crate_version};
use color_eyre::{Result, eyre::Context};
use git_version::git_version;
use tracing::instrument;
use tracing_subscriber::util::SubscriberInitExt;

// Since this is a binary crate, we need to ensure these modules aren't pub
// so that they can correctly warn about dead code:
// https://github.com/rust-lang/rust/issues/74970
//
// Relatedly, in this file specifically nothing should be `pub`.
mod cmd;
mod log;

// We use `cargo set-version` in CI to update the version in `Cargo.toml` to
// match the tag provided at release time; this means officially built releases
// are always "dirty" so we modify the `git_version!` macro to account for that.
#[derive(Clone, Debug, Parser)]
#[command(
    name = "hurry",
    about = "Really, really fast builds",
    version = format!("v{} commit {}", crate_version!(), git_version!(args = ["--always"])),
)]
struct TopLevelFlags {
    #[command(subcommand)]
    command: Command,

    /// Emit flamegraph profiling data
    #[arg(short, long, hide(true))]
    profile: Option<PathBuf>,

    /// When to colorize output
    #[arg(long, value_enum, default_value_t = log::WhenColor::Auto)]
    color: log::WhenColor,
}

#[derive(Clone, Debug, Subcommand)]
enum Command {
    /// Fast `cargo` builds
    #[clap(subcommand)]
    Cargo(cmd::cargo::Command),

    // TODO: /// Manage remote authentication
    // Auth,
    /// Manage user cache
    #[clap(subcommand)]
    Cache(cmd::cache::Command),

    /// Debug information
    #[clap(subcommand, hide(true))]
    Debug(cmd::debug::Command),

    /// Manage Hurry daemon
    ///
    /// This is an internal command used for managing hurryd, and end users
    /// generally shouldn't need to use it.
    #[clap(subcommand, hide(true))]
    Daemon(cmd::daemon::Command),
}

#[instrument]
#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let top = TopLevelFlags::parse();
    let t = top.clone();

    let (logger, flame_guard) = log::make_logger(std::io::stderr, top.profile.clone(), top.color)?;
    let result = match top.command {
        Command::Cache(cmd) => match cmd {
            cmd::cache::Command::Reset(opts) => {
                logger.init();
                cmd::cache::reset::exec(opts).await
            }
        },
        Command::Cargo(cmd) => match cmd {
            cmd::cargo::Command::Build(opts) => {
                logger.init();
                cmd::cargo::build::exec(opts).await
            }
            cmd::cargo::Command::Run(opts) => {
                logger.init();
                cmd::cargo::run::exec(opts).await
            }
        },
        Command::Debug(cmd) => match cmd {
            cmd::debug::Command::Metadata(opts) => {
                logger.init();
                cmd::debug::metadata::exec(opts).await
            }
            cmd::debug::Command::Copy(opts) => {
                logger.init();
                cmd::debug::copy::exec(opts).await
            }
        },
        Command::Daemon(cmd) => match cmd {
            cmd::daemon::Command::Start(opts) => {
                // Note that in daemon mode we do not initialize the logger!
                // Instead, we pass in the STDERR logger, because our daemon
                // logger is actually the file logger.
                cmd::daemon::start::exec(t, logger, opts).await
            }
        },
    };

    // TODO: Unsure if we need to keep this, the guard _should_ flush on drop.
    if let Some(flame_guard) = flame_guard {
        flame_guard.flush().context("flush flame_guard")?;
    }

    result
}
