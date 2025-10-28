use clap::Subcommand;

pub mod start;

#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    /// Start the Hurry daemon.
    Start(start::Options),
}
