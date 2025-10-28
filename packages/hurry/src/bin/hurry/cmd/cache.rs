use clap::Subcommand;

pub mod reset;
pub mod show;

#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    /// Reset the cache.
    Reset(reset::Options),

    /// Print the location of the local cache directory for the user.
    Show,
}
