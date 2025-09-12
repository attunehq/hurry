use clap::Args;
use color_eyre::{Result, eyre::Context as _};
use colored::Colorize as _;
use hurry::fs::{self, user_global_cache_path};
use inquire::Confirm;
use tracing::{instrument, warn};

#[derive(Clone, Args, Debug)]
pub struct Options {
    /// Skip confirmation prompt.
    #[arg(short, long)]
    yes: bool,
}

#[instrument]
pub async fn exec(options: Options) -> Result<()> {
    if !options.yes {
        println!(
            "{}",
            "WARNING: This will delete all cached data across all Hurry projects".on_red()
        );
        let confirmed = Confirm::new("Are you sure you want to proceed?")
            .with_default(false)
            .prompt()?;
        if !confirmed {
            return Ok(());
        }
    }

    let cache_path = user_global_cache_path()
        .await
        .context("get user global cache path")?;
    println!("Clearing cache directory at {cache_path}");
    match fs::metadata(cache_path.as_std_path()).await {
        Ok(Some(metadata)) => {
            if !metadata.is_dir() {
                warn!("Cache directory is not a directory: {metadata:?}");
            }
        }
        // If the directory already doesn't exist, then we're done. We
        // short-circuit here because `remove_dir_all` will fail if the
        // directory doesn't exist.
        Ok(None) => {
            println!("Done!");
            return Ok(());
        }
        Err(err) => {
            warn!("Failed to stat cache directory: {err}");
        }
    }
    fs::remove_dir_all(&cache_path)
        .await
        .with_context(|| format!("remove cache directory: {cache_path}"))?;
    println!("Done!");
    Ok(())
}
