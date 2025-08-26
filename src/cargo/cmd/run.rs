use clap::Args;
use color_eyre::Result;
use tracing::instrument;

use crate::cargo::invoke;

/// Options for `cargo run`
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
pub fn exec(options: Options) -> Result<()> {
    invoke("run", options.argv)
}
