use clap::Subcommand;
use color_eyre::Result;
use tracing::instrument;

mod cargo;

/// Display various cache information.
#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    /// Display information about cached Cargo packages.
    Cargo(cargo::Options),
}

#[instrument]
pub async fn exec(cmd: Command) -> Result<()> {
    match cmd {
        Command::Cargo(opts) => cargo::exec(opts).await,
    }
}
