use clap::Subcommand;
use color_eyre::Result;

pub mod reset;
pub mod show;

#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    /// Reset the cache.
    Reset(reset::Options),

    /// Print the location of the local cache directory for the user.
    #[clap(subcommand)]
    Show(show::Command),
}

pub async fn exec(cmd: Command) -> Result<()> {
    match cmd {
        Command::Reset(opts) => reset::exec(opts).await,
        Command::Show(cmd) => show::exec(cmd).await,
    }
}
