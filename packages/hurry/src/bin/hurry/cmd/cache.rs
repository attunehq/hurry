use clap::Subcommand;

pub mod reset;

#[derive(Clone, Subcommand)]
pub enum Command {
    /// Reset the cache.
    #[command(hide = true)]
    Reset(reset::Options),
}
