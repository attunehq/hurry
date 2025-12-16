use std::{ffi::OsStr, io::Cursor, process::Stdio};

use cargo_metadata::Message;
use clap::Args;
use color_eyre::{
    Result,
    eyre::{Context, bail},
};
use derive_more::Debug;
use tracing::{debug, info, instrument, trace, warn};
use url::Url;

use clients::Token;
use hurry::{
    cargo::{self, CargoBuildArguments, CargoCache, Handles, Workspace},
    progress::TransferBar,
};

#[derive(Clone, Args, Debug)]
pub struct Options {
    /// Base URL for the Hurry API.
    #[arg(
        long = "hurry-api-url",
        env = "HURRY_API_URL",
        default_value = "https://app.hurry.build"
    )]
    #[debug("{api_url}")]
    api_url: Url,

    /// Authentication token for the Hurry API.
    #[arg(long = "hurry-api-token", env = "HURRY_API_TOKEN")]
    api_token: Token,

    /// These arguments are passed directly to `cargo build` as provided.
    #[arg(
        num_args = ..,
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "ARGS",
    )]
    argv: Vec<String>,
}

#[instrument]
pub async fn exec(options: Options) -> Result<()> {
    // Parse and validate cargo build arguments.
    let args = CargoBuildArguments::from_iter(&options.argv);
    debug!(?args, "parsed cargo build arguments");

    // Open workspace.
    let workspace = Workspace::from_argv(&args)
        .await
        .context("opening workspace")?;

    // Compute unit plan, which provides expected units. Note that
    // because we are not actually running build scripts, these "expected
    // units" do not contain fully unambiguous cache key information.
    let units = workspace
        .units(&args)
        .await
        .context("calculating expected units")?;
    info!(target = ?workspace.target_arch, "restoring using target");

    // Initialize cache.
    let cache = CargoCache::open(options.api_url, options.api_token, workspace)
        .await
        .context("opening cache")?;

    // Restore units.
    let unit_count = units.len() as u64;
    let progress = TransferBar::new(unit_count, "Restoring cache");
    cache.restore(&units, &progress).await?;
    drop(progress);

    // Run build with `--message-format=json` for freshness indicators and
    // `--verbose` for debugging information.
    let mut argv = options.argv;
    if !argv.contains(&String::from("--message-format=json")) {
        argv.push(String::from("--message-format=json"));
    }
    if !argv.contains(&String::from("--verbose")) {
        argv.push(String::from("--verbose"));
    }
    let handles = Handles {
        stdout: Stdio::piped(),
        stderr: Stdio::inherit(),
    };
    let child = cargo::invoke_with("build", &argv, [] as [(&OsStr, &OsStr); 0], handles)
        .await
        .context("build with cargo")?;
    let output = child.wait_with_output().await?;
    trace!(?output, "cargo output");
    let output = Cursor::new(output.stdout);
    let mut ok = true;
    for message in Message::parse_stream(output) {
        debug!(?message, "cargo message");
        let message = message?;
        if let Message::CompilerArtifact(msg) = message
            && !msg.fresh
            && msg
                .package_id
                .repr
                .starts_with("registry+https://github.com/rust-lang/crates.io-index#")
        {
            // TODO: Only warn if _restored_ units are not fresh.
            warn!("unit {:?} is not fresh", msg.package_id);
            ok = false;
        }
    }

    if ok {
        info!("OK");
        Ok(())
    } else {
        bail!("not all units were fresh")
    }
}
