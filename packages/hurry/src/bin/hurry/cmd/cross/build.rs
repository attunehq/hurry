//! Builds cross-compiled projects using an optimized cache.
//!
//! This is similar to `cargo build` but uses `cross` for cross-compilation.
//! The caching logic is identical - we just run build plans through the
//! cross container and convert container paths to host paths before caching.

use clap::Args;
use color_eyre::{
    Result, Section as _, SectionExt as _,
    eyre::{Context, eyre},
};
use derive_more::Debug;
use tracing::{debug, info, instrument, warn};

use hurry::{
    cargo::{CargoCache, Workspace},
    cross,
    progress::TransferBar,
};

use super::super::{BuildOptions, wait_for_upload};

/// Options for `cross build`.
#[derive(Clone, Args, Debug)]
#[command(disable_help_flag = true)]
pub struct Options {
    /// Shared build options.
    #[clap(flatten)]
    pub build: BuildOptions,
}

#[instrument]
pub async fn exec(options: Options) -> Result<()> {
    // If help is requested, passthrough directly to cross to show cross's help
    if options.build.is_help_request() {
        return cross::invoke("build", &options.build.argv).await;
    }

    // We make the API token required here; if we make it required in the actual
    // clap state then we aren't able to support e.g. `cross build -h` passthrough.
    let Some(token) = &options.build.hurry.api_token else {
        return Err(eyre!("Hurry API authentication token is required"))
            .suggestion("Set the `HURRY_API_TOKEN` environment variable")
            .suggestion("Provide it with the `--hurry-api-token` argument");
    };

    info!("Starting");

    // Parse and validate cargo build arguments.
    let args = options.build.parsed_args();
    debug!(?args, "parsed cross build arguments");

    // Open workspace.
    let workspace = Workspace::from_argv(&args)
        .await
        .context("opening workspace")?;
    debug!(?workspace, "opened workspace");

    // Compute expected unit plans using cross build plan.
    // If this fails (unsupported target, etc.), fall back to passthrough.
    println!("[hurry] Computing build plan inside Cross context");
    let units = match workspace.cross_units(&args).await {
        Ok(units) => units,
        Err(error) => {
            warn!(
                ?error,
                "Cross acceleration not available for this configuration, \
                 falling back to passthrough build"
            );

            println!("[hurry] Running cross build without caching");
            return cross::invoke("build", &options.build.argv)
                .await
                .context("passthrough build with cross")
                .with_warning(|| format!("{error:?}").header("Cross acceleration error:"));
        }
    };

    // Initialize cache.
    let cache = CargoCache::open(
        options.build.hurry.api_url.clone(),
        token.clone(),
        workspace,
    )
    .await
    .context("opening cache")?;

    // Restore artifacts.
    let unit_count = units.len() as u64;
    let restored = if !options.build.hurry.skip_restore {
        let progress = TransferBar::new(unit_count, "Restoring cache");
        cache.restore(&units, &progress).await?
    } else {
        Default::default()
    };

    // Run the cross build.
    if !options.build.hurry.skip_build {
        println!("[hurry] Building with Cross");

        cross::invoke("build", &options.build.argv)
            .await
            .context("build with cross")?;
    }

    // Cache the built artifacts.
    if !options.build.hurry.skip_backup {
        let upload_id = cache.save(units, restored).await?;
        if !options.build.hurry.async_upload {
            let progress = TransferBar::new(unit_count, "Uploading cache");
            wait_for_upload(upload_id, &progress).await?;
        }
    }

    Ok(())
}
