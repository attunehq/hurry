use std::path::PathBuf;

use clap::Args;
use color_eyre::Result;
use hurry::fs;
use tracing::instrument;

/// Options for `debug copy`
#[derive(Clone, Args, Debug)]
pub struct Options {
    /// The source directory.
    source: PathBuf,

    /// The destination directory.
    destination: PathBuf,
}

#[instrument]
pub async fn exec(options: Options) -> Result<()> {
    let bytes = fs::copy_dir(options.source, options.destination).await?;
    println!("copied {bytes} bytes");
    Ok(())
}
