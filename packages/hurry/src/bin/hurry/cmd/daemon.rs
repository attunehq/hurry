use clap::Subcommand;

pub mod start;
pub mod stop;

#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    /// Start the Hurry daemon.
    Start(start::Options),

    /// Stop the daemon.
    ///
    /// The daemon does finish serving any requests to it, but any uploads that
    /// are in-flight or enqueued are interrupted by this shutdown.
    Stop(stop::Options),
}
