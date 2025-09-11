use clap::Args;
use color_eyre::{Result, eyre::Context};
use hurry::{fs, path::SomeDirPath};
use tracing::instrument;

/// Options for `debug copy`
#[derive(Clone, Args, Debug)]
pub struct Options {
    /// The source directory.
    source: SomeDirPath,

    /// The destination directory.
    destination: SomeDirPath,
}

#[instrument]
pub async fn exec(options: Options) -> Result<()> {
    let src = options
        .source
        .try_as_abs_dir_using_cwd()
        .context("make source absolute")?;
    let dst = options
        .destination
        .try_as_abs_dir_using_cwd()
        .context("make destination absolute")?;
    let bytes = fs::copy_dir(&src, &dst).await?;
    println!("copied {bytes} bytes");
    Ok(())
}
