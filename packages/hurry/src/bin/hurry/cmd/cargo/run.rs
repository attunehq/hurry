use clap::Args;
use color_eyre::Result;
use hurry::cargo::invoke;
use tracing::instrument;

/// Options for `cargo run`
//
// Hurry options are prefixed with `hurry-` to disambiguate from `cargo` args.
#[derive(Clone, Args, Debug)]
pub struct Options {
    /// These arguments are passed directly to `cargo run` as provided.
    #[arg(
        num_args = ..,
        trailing_var_arg = true,
        allow_hyphen_values = true,
    )]
    argv: Vec<String>,
}

#[instrument]
pub async fn exec(options: Options) -> Result<()> {
    invoke("run", options.argv).await
}
