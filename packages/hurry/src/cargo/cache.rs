use std::{process::Stdio, time::Duration};

use color_eyre::{Result, Section, SectionExt, eyre::Context as _};
use derive_more::Debug;
use serde::{Deserialize, Serialize};
use tracing::{instrument, trace};
use url::Url;
use uuid::Uuid;

use crate::{
    cargo::{QualifiedPath, UnitPlan, Workspace},
    cas::CourierCas,
    daemon::{CargoUploadRequest, DaemonPaths},
    progress::TransferBar,
};
use clients::{Courier, Token};

mod restore;
mod save;

pub use restore::{Restored, restore_units};
pub use save::{SaveProgress, save_units};

#[derive(Debug, Clone)]
pub struct CargoCache {
    #[debug("{:?}", courier_url.as_str())]
    courier_url: Url,
    courier_token: Token,
    courier: Courier,
    cas: CourierCas,
    ws: Workspace,
}

impl CargoCache {
    #[instrument(name = "CargoCache::open", skip(courier_token))]
    pub async fn open(courier_url: Url, courier_token: Token, ws: Workspace) -> Result<Self> {
        let courier = Courier::new(courier_url.clone(), courier_token.clone())?;
        courier.ping().await.context("ping courier service")?;
        let cas = CourierCas::new(courier.clone());
        Ok(Self {
            courier_url,
            courier_token,
            courier,
            cas,
            ws,
        })
    }

    #[instrument(name = "CargoCache::save", skip_all)]
    pub async fn save(&self, units: Vec<UnitPlan>, restored: Restored) -> Result<Uuid> {
        let paths = DaemonPaths::initialize().await?;

        // Start daemon if it's not already running. If it is, try to read its
        // context file to get its url, which we need to know in order to
        // communicate with it.
        let daemon = if let Some(daemon) = paths.daemon_running().await? {
            daemon
        } else {
            // TODO: Ideally we'd replace this with proper double-fork
            // daemonization to avoid the security and compatibility concerns
            // here: someone could replace the binary at this path in the time
            // between when this binary launches and when it re-launches itself
            // as a daemon.
            let hurry_binary = std::env::current_exe().context("read current binary path")?;

            // Spawn self as a child and wait for the ready message on STDOUT.
            let mut cmd = tokio::process::Command::new(hurry_binary);
            cmd.arg("daemon")
                .arg("start")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            cmd.spawn()?;

            // This value was chosen arbitrarily. Adjust as needed.
            const DAEMON_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
            tokio::time::timeout(DAEMON_STARTUP_TIMEOUT, async {
                let mut interval = tokio::time::interval(Duration::from_secs(1));
                loop {
                    interval.tick().await;
                    if let Some(daemon) = paths.daemon_running().await? {
                        break Result::<_>::Ok(daemon);
                    }
                }
            })
            .await
            .context("wait for daemon to start")??
        };

        // Connect to daemon HTTP server.
        let client = reqwest::Client::default();
        let endpoint = format!("http://{}/api/v0/cargo/upload", daemon.url);

        // Send upload request.
        let request_id = Uuid::new_v4();
        let request = CargoUploadRequest {
            request_id,
            courier_url: self.courier_url.clone(),
            courier_token: self.courier_token.clone(),
            ws: self.ws.clone(),
            units,
            skip: restored,
        };
        trace!(?request, "submitting upload request");
        let response = client
            .post(&endpoint)
            .json(&request)
            .send()
            .await
            .with_context(|| format!("send upload request to daemon at: {endpoint}"))
            .with_section(|| format!("{daemon:?}").header("Daemon context:"))?;
        trace!(?response, "got upload response");

        Ok(request_id)
    }

    #[instrument(name = "CargoCache::restore", skip_all)]
    pub async fn restore(&self, units: &Vec<UnitPlan>, progress: &TransferBar) -> Result<Restored> {
        restore_units(&self.courier, &self.cas, &self.ws, units, progress).await
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SavedFile {
    pub path: QualifiedPath,
    pub contents: Vec<u8>,
    pub executable: bool,
}
