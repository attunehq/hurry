use clap::Args;
use color_eyre::{Result, eyre::Context as _};
use colored::Colorize as _;
use hurry::client::Courier;
use inquire::Confirm;
use tracing::instrument;
use url::Url;

#[derive(Clone, Args, Debug)]
pub struct Options {
    /// Skip confirmation prompt.
    #[arg(short, long)]
    yes: bool,

    /// Base URL for the Courier instance.
    #[arg(long = "hurry-courier-url", env = "HURRY_COURIER_URL")]
    courier_url: Url,
}

#[instrument]
pub async fn exec(options: Options) -> Result<()> {
    if !options.yes {
        println!(
            "{}",
            "WARNING: This will delete all cached data for your entire organization".on_red()
        );
        let confirmed = Confirm::new("Are you sure you want to proceed?")
            .with_default(false)
            .prompt()?;
        if !confirmed {
            return Ok(());
        }
    }

    let courier = Courier::new(options.courier_url);
    courier.ping().await.context("ping courier service")?;

    println!("Resetting Courier cache...");
    courier.cache_reset().await.context("reset cache")?;
    println!("Done!");
    Ok(())
}
