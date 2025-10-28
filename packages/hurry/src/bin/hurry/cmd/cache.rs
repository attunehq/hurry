use clap::Subcommand;

pub mod reset;
pub mod show;

#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    /// Reset the cache.
    Reset(reset::Options),

    /// Print the cache directory for the user.
    Show,
}
