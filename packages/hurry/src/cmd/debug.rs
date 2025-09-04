use clap::Subcommand;

pub mod metadata;

/// Supported debug subcommands.
#[derive(Clone, Subcommand)]
pub enum Command {
    /// Recursively enumerate all files in the directory and emit the paths
    /// along with the metadata `hurry` tracks for these files.
    Metadata(metadata::Options),
}
