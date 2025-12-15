use clap::Args;
use color_eyre::{Result, eyre::Context as _};
use colored::Colorize as _;
use derive_more::Debug;
use inquire::Confirm;
use tracing::instrument;
use url::Url;

use clients::{Courier, Token};

#[derive(Clone, Args, Debug)]
pub struct Options {
    /// Skip all confirmation prompts.
    #[arg(short, long)]
    yes: bool,

    /// Base URL for the Hurry API.
    #[arg(
        long = "api-url",
        env = "HURRY_API_URL",
        default_value = "https://courier.staging.corp.attunehq.com"
    )]
    #[debug("{api_url}")]
    api_url: Url,

    /// Authentication token for the Hurry API.
    #[arg(long = "api-token", env = "HURRY_API_TOKEN")]
    api_token: Token,

    /// Delete remote cache.
    // TODO: Once we have a tiered local cache, add a `--local` option.
    //
    // TODO: Once we support multiple languages, maybe this should migrate to
    // `hurry cache reset cargo`?
    #[arg(long)]
    remote: bool,
}

#[instrument]
pub async fn exec(options: Options) -> Result<()> {
    if !options.remote {
        println!("You must specify which caches to delete with `--remote`");
        return Ok(());
    }

    if !options.yes {
        println!(
            "{}",
            "WARNING: This will delete all cached data across your entire organization".on_red()
        );
        let confirmed = Confirm::new("Are you sure you want to proceed?")
            .with_default(false)
            .prompt()?;
        if !confirmed {
            return Ok(());
        }
    }
    if options.remote {
        let courier = Courier::new(options.api_url, options.api_token)?;
        courier.ping().await.context("ping Hurry API")?;

        println!("Resetting remote cache...");
        courier.cache_reset().await.context("reset remote cache")?;
    }

    println!("Done!");
    Ok(())
}
