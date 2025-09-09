use std::path::PathBuf;

use clap::Args;
use color_eyre::{Result, eyre::Context};
use hurry::{fs, path::AbsDirPath};
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
    let src = AbsDirPath::new(options.source).context("parse source dir")?;
    let dst = AbsDirPath::new(options.destination).context("parse destination dir")?;
    let bytes = fs::copy_dir(&src, &dst).await?;
    println!("copied {bytes} bytes");
    Ok(())
}
