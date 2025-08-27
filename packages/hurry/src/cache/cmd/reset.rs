use std::fs;

use clap::Args;
use color_eyre::{Result, eyre::Context as _};
use colored::Colorize as _;
use inquire::Confirm;
use tracing::{instrument, warn};

use crate::fs::user_global_cache_path;

#[derive(Clone, Args, Debug)]
pub struct Options {}

#[instrument]
pub fn exec(options: Options) -> Result<()> {
    println!(
        "{}",
        "WARNING: This will delete all cached data across all Hurry projects".on_red()
    );
    let ok = Confirm::new("Are you sure you want to proceed?")
        .with_default(false)
        .prompt()?;
    if !ok {
        return Ok(());
    }

    let cache_path = user_global_cache_path().context("get user global cache path")?;
    println!("Clearing cache directory at {cache_path:?}");
    match fs::metadata(&cache_path) {
        Ok(metadata) => {
            if !metadata.is_dir() {
                warn!("Cache directory is not a directory: {metadata:?}");
            }
        }
        Err(err) => {
            // If the directory already doesn't exist, then we're done. We
            // short-circuit here because `remove_dir_all` will fail if the
            // directory doesn't exist.
            if err.kind() == std::io::ErrorKind::NotFound {
                println!("Done!");
                return Ok(());
            }
            warn!("Failed to stat cache directory: {err:?}");
        }
    }
    fs::remove_dir_all(&cache_path).context(format!("remove cache directory: {cache_path:?}"))?;
    println!("Done!");
    Ok(())
}
