//! Builds cross-compiled projects using an optimized cache.
//!
//! This is similar to `cargo build` but uses `cross` for cross-compilation.
//! The caching logic is identical - we just run build plans through the
//! cross container and convert container paths to host paths before caching.

use std::time::Duration;

use color_eyre::{
    Result, Section as _, SectionExt as _,
    eyre::{Context, OptionExt as _, bail, eyre},
};
use derive_more::Debug;
use tracing::{debug, info, instrument, trace, warn};
use url::Url;
use uuid::Uuid;

use clients::Token;
use hurry::{
    cargo::{CargoBuildArguments, CargoCache, Workspace},
    ci::is_ci,
    cross,
    daemon::{CargoUploadStatus, CargoUploadStatusRequest, CargoUploadStatusResponse, DaemonPaths},
    progress::TransferBar,
};

/// Options for `cross build`.
#[derive(Clone, clap::Args, Debug)]
#[command(disable_help_flag = true)]
pub struct Options {
    /// Base URL for the Hurry API.
    #[arg(
        long = "hurry-api-url",
        env = "HURRY_API_URL",
        default_value = "https://courier.staging.corp.attunehq.com"
    )]
    #[debug("{api_url}")]
    api_url: Url,

    /// Authentication token for the Hurry API.
    #[arg(long = "hurry-api-token", env = "HURRY_API_TOKEN")]
    api_token: Option<Token>,

    /// Skip backing up the cache.
    #[arg(long = "hurry-skip-backup", default_value_t = false)]
    skip_backup: bool,

    /// Skip the cross build, only performing the cache actions.
    #[arg(long = "hurry-skip-build", default_value_t = false)]
    skip_build: bool,

    /// Skip restoring the cache.
    #[arg(long = "hurry-skip-restore", default_value_t = false)]
    skip_restore: bool,

    /// Wait for all new artifacts to upload to cache to finish before exiting.
    ///
    /// When not provided, automatically decides based on environment:
    /// - In CI, defaults to waiting.
    /// - In local development, defaults to async upload.
    /// - If desired, override CI behavior using `=false`.
    #[arg(
        long = "hurry-wait-for-upload",
        env = "HURRY_WAIT_FOR_UPLOAD",
        num_args = 0..=1,
        default_value = None,
        default_missing_value = "true",
        require_equals = true,
    )]
    wait_for_upload: Option<bool>,

    /// Show help for `hurry cross build`.
    #[arg(long = "hurry-help", default_value_t = false)]
    pub help: bool,

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
    let Some(token) = &options.api_token else {
        return Err(eyre!("Hurry API authentication token is required"))
            .suggestion("Set the `HURRY_API_TOKEN` environment variable")
            .suggestion("Provide it with the `--hurry-api-token` argument");
    };

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
    let cache = CargoCache::open(options.api_url, token.clone(), workspace)
        .await
        .context("opening cache")?;

    // Restore artifacts.
    let unit_count = units.len() as u64;
    let restored = if !options.skip_restore {
        let progress = TransferBar::new(unit_count, "Restoring cache");
        cache.restore(&units, &progress).await?
    } else {
        Default::default()
    };

    // Run the cross build.
    if !options.skip_build {
        println!("[hurry] Building with Cross");

        cross::invoke("build", &options.argv)
            .await
            .context("build with cross")?;
    }

    // Cache the built artifacts.
    if !options.skip_backup {
        let upload_id = cache.save(units, restored).await?;
        if WaitForUpload::from(options.wait_for_upload).should_wait() {
            let progress = TransferBar::new(unit_count, "Uploading cache");
            wait_for_upload(upload_id, &progress).await?;
        }
    }

    Ok(())
}

#[instrument]
async fn wait_for_upload(request_id: Uuid, progress: &TransferBar) -> Result<()> {
    let paths = DaemonPaths::initialize().await?;
    let Some(daemon) = paths.daemon_running().await? else {
        bail!("daemon is not running");
    };

    let client = reqwest::Client::default();
    let endpoint = format!("http://{}/api/v0/cargo/status", daemon.url);
    let request = CargoUploadStatusRequest { request_id };
    let mut interval = tokio::time::interval(Duration::from_secs(1));

    let mut last_uploaded_artifacts = 0u64;
    let mut last_uploaded_files = 0u64;
    let mut last_uploaded_bytes = 0u64;
    let mut last_total_artifacts = 0u64;
    loop {
        interval.tick().await;
        trace!(?request, "submitting upload status request");
        let response = client
            .post(&endpoint)
            .json(&request)
            .send()
            .await
            .with_context(|| format!("send upload status request to daemon at: {endpoint}"))
            .with_section(|| format!("{daemon:?}").header("Daemon context:"))?;
        trace!(?response, "got upload status response");
        let response = response.json::<CargoUploadStatusResponse>().await?;
        trace!(?response, "parsed upload status response");
        let status = response.status.ok_or_eyre("no upload status")?;
        match status {
            CargoUploadStatus::Complete => break,
            CargoUploadStatus::InProgress(save_progress) => {
                progress.add_bytes(
                    save_progress
                        .uploaded_bytes
                        .saturating_sub(last_uploaded_bytes),
                );
                last_uploaded_bytes = save_progress.uploaded_bytes;
                progress.add_files(
                    save_progress
                        .uploaded_files
                        .saturating_sub(last_uploaded_files),
                );
                last_uploaded_files = save_progress.uploaded_files;
                progress.inc(
                    save_progress
                        .uploaded_units
                        .saturating_sub(last_uploaded_artifacts),
                );
                last_uploaded_artifacts = save_progress.uploaded_units;
                progress.dec_length(last_total_artifacts.saturating_sub(save_progress.total_units));
                last_total_artifacts = save_progress.total_units;
            }
        }
    }

    Ok(())
}

/// Control whether to wait for artifact uploads to complete.
#[derive(Clone, Copy, Debug)]
enum WaitForUpload {
    /// Automatically decide based on environment
    Auto,

    /// Wait for uploads
    ExplicitWait,

    /// Don't wait for uploads
    ExplicitAsync,
}

impl WaitForUpload {
    fn should_wait(self) -> bool {
        match self {
            Self::ExplicitWait => true,
            Self::ExplicitAsync => false,
            Self::Auto => is_ci(),
        }
    }
}

impl From<Option<bool>> for WaitForUpload {
    fn from(value: Option<bool>) -> Self {
        match value {
            None => Self::Auto,
            Some(true) => Self::ExplicitWait,
            Some(false) => Self::ExplicitAsync,
        }
    }
}
