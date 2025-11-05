mod cargo;

pub use cargo::{
    CargoDaemonState, CargoUploadRequest, CargoUploadResponse, CargoUploadStatus,
    CargoUploadStatusRequest, CargoUploadStatusResponse, cargo_router,
};

use crate::{
    fs, mk_rel_file,
    path::{AbsFilePath, JoinWith as _},
};
use color_eyre::{
    Result,
    eyre::{Context as _, OptionExt as _, bail},
};
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};
use tap::Pipe as _;

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
        if self.pid_file_path.exists().await {
            let pid = fs::must_read_buffered_utf8(&self.pid_file_path).await?;
            match pid.trim().parse::<u32>() {
                Ok(pid) => {
                    let system = System::new_with_specifics(
                        RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing()),
                    );
                    let process = system.process(Pid::from_u32(pid));
                    match process {
                        Some(_) => self.read_context().await?,
                        None => None,
                    }
                }
                Err(err) => {
                    bail!("could not parse pid-file: {err}")
                }
            }
        } else {
            None
        }
        .pipe(Ok)
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
