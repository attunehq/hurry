use clap::Args;
use color_eyre::Result;
use derive_more::Debug;
use url::Url;

#[derive(Clone, Args, Debug)]
pub struct Options {
    /// Base URL for the Courier instance.
    #[arg(
        long = "courier-url",
        env = "HURRY_COURIER_URL",
        default_value = "https://courier.staging.corp.attunehq.com"
    )]
    #[debug("{courier_url}")]
    courier_url: Url,

    /// Name of the package to display.
    #[arg(long)]
    name: String,

    // TODO: Add more flags like --version and --library-unit-hash.
}

pub async fn exec(opts: Options) -> Result<()> {
    // TODO: Load cache information for the package, and show what artifact
    // files are cached for the package. If some flags are missing, infer the
    // desired package using information in the current build artifact plan
    // (e.g. if we only have name, use the version of the package in the
    // workspace).
    Ok(())
}
