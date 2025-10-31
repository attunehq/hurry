use clap::Subcommand;
use color_eyre::Result;

pub mod check;
pub mod copy;
pub mod daemon;
pub mod metadata;

/// Supported debug subcommands.
#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    /// Check cached artifact integrity.
    Check(check::Options),

    /// Recursively enumerate all files in the directory and emit the paths
    /// along with the metadata `hurry` tracks for these files.
    Metadata(metadata::Options),

    /// Recursively copy the contents of the source directory to destination.
    Copy(copy::Options),

    /// Daemon-related debugging commands.
    #[clap(subcommand)]
    Daemon(daemon::Command),
}

pub async fn exec(cmd: Command) -> Result<()> {
    match cmd {
        Command::Check(opts) => check::exec(opts).await,
        Command::Metadata(opts) => metadata::exec(opts).await,
        Command::Copy(opts) => copy::exec(opts).await,
        Command::Daemon(subcmd) => daemon::exec(subcmd).await,
    }
}
