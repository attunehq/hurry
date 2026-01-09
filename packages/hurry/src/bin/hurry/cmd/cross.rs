use std::ffi::OsString;

use clap::{Args, CommandFactory, Parser};
use color_eyre::{Result, eyre::Context};
use hurry::cross;
use tracing::debug;

mod build;

/// Helper type for parsing options with `clap`.
#[derive(Parser)]
struct CommandOptions<T: Args> {
    #[clap(flatten)]
    inner: T,
}

impl<T: Args> CommandOptions<T> {
    fn parse(args: impl IntoIterator<Item = impl Into<OsString> + Clone>) -> Result<Self> {
        Self::try_parse_from(args).context("parse options")
    }

    fn into_inner(self) -> T {
        self.inner
    }
}

/// Execute a cross command by dispatching based on the first argument.
pub async fn exec(arguments: Vec<String>) -> Result<()> {
    let Some((command, options)) = arguments.split_first() else {
        return cross::invoke_plain(Vec::<String>::new()).await;
    };

    // If this is Windows, just pass through to `cross` unconditionally.
    //
    // We passthrough on Windows for the same reasons as cargo: we're not sure
    // that cross acceleration is working properly for Windows yet. For more
    // context, see issue #153.
    if cfg!(target_os = "windows") {
        debug!("windows currently unconditionally passes through all cross commands");
        return cross::invoke(command, options).await;
    }

    // The first argument being a flag means we're running against `cross` directly.
    //
    // FIXME: This isn't always true - e.g. `cross --locked build` would have
    // `--locked` as the first argument but should still accelerate `build`.
    // See: https://github.com/attunehq/hurry/issues/266
    if command.starts_with('-') {
        return cross::invoke(command, options).await;
    }

    // Otherwise, we're running a subcommand.
    //
    // We do it this way instead of constructing subcommands "the clap way" because
    // we want to passthrough things like `help` and `version` to cross instead of
    // having clap intercept them.
    //
    // As we add special cased handling for more subcommands we'll extend this match
    // statement with other functions similar to the one we use for `build`.
    match command.as_str() {
        "build" | "b" => {
            let opts = CommandOptions::<build::Options>::parse(&arguments)?;
            if opts.inner.hurry.help {
                // Help flag handling happens here because `build --help` passes
                // through to `cross build --help`, and we need the `Command`
                // struct in order to print the generated help text.
                let mut cmd = CommandOptions::<build::Options>::command();
                cmd = cmd.about("Run `cross build` with Hurry build acceleration");
                cmd.print_help()?;
                return Ok(());
            }
            build::exec(opts.into_inner()).await
        }
        _ => cross::invoke(command, options).await,
    }
}
