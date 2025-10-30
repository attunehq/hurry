use clap::Subcommand;

pub mod copy;
pub mod daemon;
pub mod metadata;

/// Supported debug subcommands.
#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    /// Recursively enumerate all files in the directory and emit the paths
    /// along with the metadata `hurry` tracks for these files.
    Metadata(metadata::Options),

    /// Recursively copy the contents of the source directory to destination.
    Copy(copy::Options),

    /// Daemon-related debugging commands.
    #[clap(subcommand)]
    Daemon(DaemonCommand),
}

/// Daemon debugging subcommands.
#[derive(Clone, Debug, Subcommand)]
pub enum DaemonCommand {
    /// Print or follow the daemon log file.
    Log(daemon::log::Options),

    /// Show the daemon context file or a specific field.
    Context(daemon::context::Options),

    /// Report whether the daemon is running or stopped.
    State(daemon::state::Options),
}
