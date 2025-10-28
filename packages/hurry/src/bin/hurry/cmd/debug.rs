use clap::Subcommand;

pub mod copy;
pub mod metadata;

/// Supported debug subcommands.
#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    /// Recursively enumerate all files in the directory and emit the paths
    /// along with the metadata `hurry` tracks for these files.
    Metadata(metadata::Options),

    /// Recursively copy the contents of the source directory to destination.
    Copy(copy::Options),
}
