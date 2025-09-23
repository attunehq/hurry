use clap::Args;
use color_eyre::{Result, eyre::Context};
use hurry::cargo::{self, Workspace};
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
    let workspace = Workspace::from_argv(&options.argv)
        .await
        .context("open workspace")?;

    cargo::invoke(&workspace, "run", options.argv).await
}
