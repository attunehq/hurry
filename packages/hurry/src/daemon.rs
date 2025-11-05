mod cargo;

pub use cargo::{
    CargoDaemonState, CargoUploadRequest, CargoUploadResponse, CargoUploadStatus,
    CargoUploadStatusRequest, CargoUploadStatusResponse, cargo_router,
};

use crate::{
    fs, mk_rel_file,
    path::{AbsFilePath, JoinWith as _},
};
use atomic_time::AtomicInstant;
use color_eyre::{
    Result,
    eyre::{Context as _, OptionExt as _},
};
use derive_more::Debug;
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
use tracing::debug;

/// How frequently to check if the daemon has exceeded its idle timeout.
///
/// This balances responsiveness (shorter intervals shut down faster after
/// becoming idle) against resource usage (frequent checks consume more CPU).
const IDLE_CHECK_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct DaemonContext {
    pub pid: u32,
    pub url: String,
    pub log_file_path: AbsFilePath,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct DaemonPaths {
    pub pid_file_path: AbsFilePath,
    pub context_path: AbsFilePath,
}

impl DaemonPaths {
    pub async fn initialize() -> Result<DaemonPaths> {
        let hurry_cache_dir = fs::user_global_cache_path().await?;
        let pid_file_path = hurry_cache_dir.join(mk_rel_file!("hurryd.pid"));
        let context_path = hurry_cache_dir.join(mk_rel_file!("hurryd.json"));
        Ok(DaemonPaths {
            pid_file_path,
            context_path,
        })
    }

    pub async fn daemon_running(&self) -> Result<Option<DaemonContext>> {
        let Some(context) = self.read_context().await? else {
            return Ok(None);
        };

        let client = reqwest::Client::new();
        let url = format!("http://{}/api/v0/health", context.url);

        match client.get(&url).send().await {
            Ok(response) if response.status().is_success() => Ok(Some(context)),
            Ok(_) => {
                debug!("daemon health check returned non-success status");
                Ok(None)
            }
            Err(err) => {
                debug!(?err, "daemon health check failed");
                Ok(None)
            }
        }
    }

    pub async fn read_context(&self) -> Result<Option<DaemonContext>> {
        if !self.context_path.exists().await {
            return Ok(None);
        }

        let context = fs::read_buffered_utf8(&self.context_path)
            .await
            .context("read daemon context file")?
            .ok_or_eyre("no daemon context file")?;

        let daemon_context =
            serde_json::from_str::<DaemonContext>(&context).context("parse daemon context")?;

        Ok(Some(daemon_context))
    }
}

/// Track the "idleness" of a resource using this structure.
#[derive(Clone, Debug)]
pub struct IdleState {
    #[debug("{:?}", last_activity.load(Ordering::Relaxed))]
    last_activity: Arc<AtomicInstant>,
    timeout: Duration,
}

impl IdleState {
    /// Create a new instance with the given timeout.
    pub fn new(timeout: Duration) -> Self {
        Self {
            last_activity: Arc::new(AtomicInstant::now()),
            timeout,
        }
    }

    /// Indicates activity, resetting the idle state.
    pub fn touch(&self) {
        self.last_activity.store(Instant::now(), Ordering::Relaxed);
    }

    /// Check if the state is idle.
    pub fn is_idle(&self) -> bool {
        let last = self.last_activity.load(Ordering::Relaxed);
        last.elapsed() > self.timeout
    }

    /// The configured timeout duration.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Complete the future when the state is idle.
    ///
    /// ## Cancellation
    ///
    /// This method is cancellation safe and intended to be used in
    /// `tokio::select!` or similar calls.
    pub async fn monitor(&self) {
        let mut interval = tokio::time::interval(IDLE_CHECK_INTERVAL);
        loop {
            interval.tick().await;
            debug!("checking idle state for server");
            if self.is_idle() {
                break;
            }
        }
    }
}
