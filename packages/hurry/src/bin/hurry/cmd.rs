use std::time::Duration;

use color_eyre::{
    Result, Section as _, SectionExt as _,
    eyre::{Context, OptionExt as _, bail, eyre},
};
use derive_more::Debug;
use tracing::{trace, instrument};
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

/// Common Hurry options shared between cargo and cross build commands.
///
/// These options control Hurry's caching behavior and are prefixed with
/// `hurry-` to disambiguate from cargo/cross arguments.
#[derive(Clone, clap::Args, Debug)]
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
    ///
    /// Note: this field is not _actually_ optional for `hurry` to operate; we're just telling clap
    /// that it is so that if the user runs with the `-h` or `--help` arguments we can not require
    /// the token in that case.
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

    /// Show help for this Hurry command.
    #[arg(long = "hurry-help", default_value_t = false)]
    pub help: bool,
}

impl HurryBuildOptions {
    /// Returns the API token, or an error with suggestions if not provided.
    pub fn require_api_token(&self) -> Result<&Token> {
        self.api_token.as_ref().ok_or_else(|| {
            eyre!("Hurry API authentication token is required")
                .suggestion("Set the `HURRY_API_TOKEN` environment variable")
                .suggestion("Provide it with the `--hurry-api-token` argument")
        })
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
