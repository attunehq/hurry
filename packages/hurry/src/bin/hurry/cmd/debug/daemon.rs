use clap::Subcommand;
use color_eyre::Result;

pub mod context;
pub mod log;
pub mod status;
pub mod stop;

/// Daemon debugging subcommands.
#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    /// Print or follow the daemon log file.
    Log(log::Options),

    /// Show the daemon context file or a specific field.
    Context(context::Options),

    /// Report whether the daemon is running or stopped.
    Status(status::Options),

    /// Stop the daemon.
    Stop(stop::Options),
}

pub async fn exec(cmd: Command) -> Result<()> {
    match cmd {
        Command::Log(opts) => log::exec(opts).await,
        Command::Context(opts) => context::exec(opts).await,
        Command::Status(opts) => status::exec(opts).await,
        Command::Stop(opts) => stop::exec(opts).await,
    }
}
