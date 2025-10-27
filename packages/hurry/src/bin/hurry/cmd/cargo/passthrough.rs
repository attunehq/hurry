use clap::Args;
use color_eyre::Result;
use hurry::cargo;
use tracing::instrument;

/// Options for passthrough cargo commands.
#[derive(Clone, Args, Debug)]
#[command(disable_help_flag = true)]
pub struct Options {
    /// Arguments passed directly to cargo.
    #[arg(
        num_args = ..,
        trailing_var_arg = true,
        allow_hyphen_values = true,
    )]
    argv: Vec<String>,
}

impl Options {
    pub fn new(argv: Vec<String>) -> Self {
        Self { argv }
    }
}

#[instrument]
pub async fn exec(subcommand: &str, options: Options) -> Result<()> {
    cargo::invoke(subcommand, options.argv).await
}
