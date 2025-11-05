//! The binary entrypoint for `hurry`, the ultra-fast build tool.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
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

// Version is sourced directly from the git tag at build time.
// In development, this will show the commit hash; in releases, it will show the
// tag (e.g., "v0.3.0").
#[derive(Clone, Debug, Parser)]
#[command(
    name = "hurry",
    about = "Really, really fast builds",
    version = git_version!(),
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
    #[command(disable_help_flag = true, disable_version_flag = true)]
    Cargo {
        // We do it this way instead of constructing subcommands "the clap way" because
        // we want to passthrough things like `help` and `version` to cargo instead of
        // having clap intercept them.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Passthrough to `cross` for cross-compilation
    #[command(disable_help_flag = true, disable_version_flag = true)]
    Cross {
        // We do it this way instead of constructing subcommands "the clap way" because
        // we want to passthrough things like `help` and `version` to cross instead of
        // having clap intercept them.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

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
        Command::Cache(cmd) => {
            logger.init();
            cmd::cache::exec(cmd).await
        }
        Command::Cargo { args } => {
            logger.init();
            cmd::cargo::exec(args).await
        }
        Command::Cross { args } => {
            logger.init();
            cmd::cross::exec(args).await
        }
        Command::Debug(cmd) => {
            logger.init();
            cmd::debug::exec(cmd).await
        }
        Command::Daemon(cmd) => match cmd {
            cmd::daemon::Command::Start(opts) => {
                // Note that in daemon mode we do not initialize the logger!
                // Instead, we pass in the STDERR logger, because our daemon
                // logger is actually the file logger.
                cmd::daemon::start::exec(t, logger, opts).await
            }
            cmd::daemon::Command::Stop(opts) => {
                logger.init();
                cmd::daemon::stop::exec(opts).await
            }
        },
    };

    // TODO: Unsure if we need to keep this, the guard _should_ flush on drop.
    if let Some(flame_guard) = flame_guard {
        flame_guard.flush().context("flush flame_guard")?;
    }

    result
}
