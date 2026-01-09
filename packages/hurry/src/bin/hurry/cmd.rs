use std::time::Duration;

use clap::Args;
use color_eyre::{
    Result, Section as _, SectionExt as _,
    eyre::{Context, OptionExt as _, bail},
};
use derive_more::Debug;
use tracing::{instrument, trace};
use url::Url;
use uuid::Uuid;

use clients::Token;
use hurry::{
    daemon::{CargoUploadStatus, CargoUploadStatusRequest, CargoUploadStatusResponse, DaemonPaths},
    progress::TransferBar,
};

pub mod cache;
pub mod cargo;
pub mod cross;
pub mod daemon;
pub mod debug;

/// Shared options for `hurry cargo build` and `hurry cross build`.
///
/// These options are prefixed with `hurry-` to disambiguate from cargo/cross args.
#[derive(Clone, Args, Debug)]
#[command(disable_help_flag = true)]
pub struct HurryBuildOptions {
    /// Base URL for the Hurry API.
    #[arg(
        long = "hurry-api-url",
        env = "HURRY_API_URL",
        default_value = "https://app.hurry.build"
    )]
    #[debug("{api_url}")]
    pub api_url: Url,

    /// Authentication token for the Hurry API.
    #[arg(long = "hurry-api-token", env = "HURRY_API_TOKEN")]
    pub api_token: Option<Token>,

    /// Skip backing up the cache.
    #[arg(long = "hurry-skip-backup", default_value_t = false)]
    pub skip_backup: bool,

    /// Skip the build, only performing the cache actions.
    #[arg(long = "hurry-skip-build", default_value_t = false)]
    pub skip_build: bool,

    /// Skip restoring the cache.
    #[arg(long = "hurry-skip-restore", default_value_t = false)]
    pub skip_restore: bool,

    /// Upload artifacts asynchronously in the background instead of waiting.
    ///
    /// By default, hurry waits for uploads to complete before exiting.
    /// Use this flag to upload in the background and exit immediately after the
    /// build.
    #[arg(
        long = "hurry-async-upload",
        env = "HURRY_ASYNC_UPLOAD",
        default_value_t = false
    )]
    pub async_upload: bool,

    /// Show help for this hurry command.
    #[arg(long = "hurry-help", default_value_t = false)]
    pub help: bool,

    /// These arguments are passed directly to the underlying build command.
    #[arg(
        num_args = ..,
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "ARGS",
    )]
    pub argv: Vec<String>,
}

impl HurryBuildOptions {
    /// Check if help is requested in the arguments.
    pub fn is_help_request(&self) -> bool {
        self.argv
            .iter()
            .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
    }
}

/// Wait for an async upload to complete by polling the daemon.
#[instrument]
pub async fn wait_for_upload(request_id: Uuid, progress: &TransferBar) -> Result<()> {
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
