//! Builds cross-compiled projects using an optimized cache.
//!
//! This is similar to `cargo build` but uses `cross` for cross-compilation.
//! The caching logic is identical - we just run build plans through the
//! cross container and convert container paths to host paths before caching.

use color_eyre::{Result, Section as _, SectionExt as _, eyre::Context};
use derive_more::Debug;
use tracing::{debug, info, instrument, warn};

use hurry::{
    cargo::{CargoBuildArguments, CargoCache, Workspace},
    cross,
    progress::TransferBar,
};

use crate::cmd::{HurryBuildOptions, wait_for_upload};

/// Options for `cross build`.
#[derive(Clone, clap::Args, Debug)]
#[command(disable_help_flag = true)]
pub struct Options {
    /// Hurry-specific options for caching and API configuration.
    #[clap(flatten)]
    pub hurry: HurryBuildOptions,

    /// These arguments are passed directly to `cross build` as provided.
    #[arg(
        num_args = ..,
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "ARGS",
    )]
    pub argv: Vec<String>,
}

impl Options {
    /// Parse the cargo build arguments.
    ///
    /// Note: cross uses the same argument format as cargo, so we can reuse
    /// CargoBuildArguments for parsing.
    #[instrument(name = "Options::parsed_args")]
    pub fn parsed_args(&self) -> CargoBuildArguments {
        CargoBuildArguments::from_iter(&self.argv)
    }

    /// Check if help is requested in the arguments.
    pub fn is_help_request(&self) -> bool {
        self.argv
            .iter()
            .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
    }
}

#[instrument]
pub async fn exec(options: Options) -> Result<()> {
    // If help is requested, passthrough directly to cross to show cross's help
    if options.is_help_request() {
        return cross::invoke("build", &options.argv).await;
    }

    // We make the API token required here; if we make it required in the actual
    // clap state then we aren't able to support e.g. `cross build -h` passthrough.
    let token = options.hurry.require_api_token()?;

    info!("Starting");

    // Parse and validate cargo build arguments.
    let args = options.parsed_args();
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
            return cross::invoke("build", &options.argv)
                .await
                .context("passthrough build with cross")
                .with_warning(|| format!("{error:?}").header("Cross acceleration error:"));
        }
    };

    // Initialize cache.
    let cache = CargoCache::open(options.hurry.api_url.clone(), token.clone(), workspace)
        .await
        .context("opening cache")?;

    // Restore artifacts.
    let unit_count = units.len() as u64;
    let restored = if !options.hurry.skip_restore {
        let progress = TransferBar::new(unit_count, "Restoring cache");
        cache.restore(&units, &progress).await?
    } else {
        Default::default()
    };

    // Run the cross build.
    if !options.hurry.skip_build {
        println!("[hurry] Building with Cross");

        cross::invoke("build", &options.argv)
            .await
            .context("build with cross")?;
    }

    // Cache the built artifacts.
    if !options.hurry.skip_backup {
        let upload_id = cache.save(units, restored).await?;
        if !options.hurry.async_upload {
            let progress = TransferBar::new(unit_count, "Uploading cache");
            wait_for_upload(upload_id, &progress).await?;
        }
    }

    Ok(())
}
