use color_eyre::Result;
use hurry::cross;
use tracing::debug;

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
        // TODO: Add special handling for build subcommand once implemented
        // "build" => build::exec(opts.into_inner()).await,
        _ => cross::invoke(command, options).await,
    }
}
