use std::ffi::OsString;

use clap::{Args, CommandFactory, Parser};
use color_eyre::{Result, eyre::Context};
use hurry::cargo;
use tracing::debug;

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

    // If this is Windows, just pass through to `cargo` unconditionally.
    //
    // Note: we don't use `#[cfg]` on this whole function because we want to have a
    // bunch of conditional modules/functions/etc. We also want to make sure
    // that we're at least _compiling_ properly for Windows moving forward; we're
    // doing this though because we're not sure that we're _working_ properly
    // for Windows yet. For more context, see issue #153.
    if cfg!(target_os = "windows") {
        debug!("windows currently unconditionally passes through all commands");
        return cargo::invoke(command, options).await;
    }

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
            let opts: CommandOptions<build::Options> = CommandOptions::parse(&arguments)?;
            if opts.opts.help {
                // Help flag handling happens here because `build --help` passes
                // through to `cargo build --help`, and we need the `Command`
                // struct in order to print the generated help text.
                let mut cmd = CommandOptions::<build::Options>::command();
                cmd = cmd.about("Run `cargo build` with Hurry build acceleration");
                cmd.print_help()?;
                return Ok(());
            }
            build::exec(opts.into_inner()).await
        }
        _ => cargo::invoke(command, options).await,
    }
}
