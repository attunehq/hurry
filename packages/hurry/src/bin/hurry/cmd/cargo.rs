use std::ffi::OsString;

use clap::{Args, Parser};
use color_eyre::{Result, eyre::Context};
use hurry::cargo;

pub mod build;

/// Helper type for parsing options with `clap`.
#[derive(Parser)]
struct CommandOptions<T: Args> {
    #[clap(flatten)]
    opts: T,
}

impl<T: Args> CommandOptions<T> {
    fn parse(args: impl IntoIterator<Item = impl Into<OsString> + Clone>) -> Result<Self> {
        Self::try_parse_from(args).context("parse options")
    }

    fn into_inner(self) -> T {
        self.opts
    }
}

/// Execute a cargo command by dispatching based on the first argument.
pub async fn exec(arguments: Vec<String>) -> Result<()> {
    let Some((command, options)) = arguments.split_first() else {
        return cargo::invoke("", Vec::<String>::new()).await;
    };

    // The first argument being a flag means we're running against `cargo` directly.
    if command.starts_with('-') {
        return cargo::invoke(command, options).await;
    }

    // Otherwise, we're running a subcommand.
    //
    // We do it this way instead of constructing subcommands "the clap way" because
    // we want to passthrough things like `help` and `version` to cargo instead of
    // having clap intercept them.
    //
    // As we add special cased handling for more subcommands we'll extend this match
    // statement with other functions similar to the one we use for `build`.
    match command.as_str() {
        "build" => {
            let opts = CommandOptions::parse(&arguments)?;
            build::exec(opts.into_inner()).await
        }
        _ => cargo::invoke(command, options).await,
    }
}
