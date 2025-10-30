use clap::Subcommand;

pub mod start;
pub mod stop;

#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    /// Start the Hurry daemon.
    Start(start::Options),

    /// Stop the daemon.
    Stop(stop::Options),
}
