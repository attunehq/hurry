use clap::Subcommand;

pub mod build;
pub mod run;

/// Supported cargo subcommands.
#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    /// Fast `cargo` builds.
    Build(build::Options),

    /// Execute `cargo` commands.
    Run(run::Options),
}
