mod cargo;

pub use cargo::{CargoUploadRequest, CargoUploadResponse};

use crate::{
    fs, mk_rel_file,
    path::{AbsFilePath, JoinWith as _},
};
use color_eyre::Result;
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};
use tap::Pipe as _;
use tracing::warn;

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct DaemonReadyMessage {
    pub pid: u32,
    pub socket_path: AbsFilePath,
    pub log_file_path: AbsFilePath,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct DaemonPaths {
    pub pid_file_path: AbsFilePath,
    pub socket_path: AbsFilePath,
}

pub async fn daemon_paths() -> Result<DaemonPaths> {
    let hurry_cache_dir = fs::user_global_cache_path().await?;
    let pid_file_path = hurry_cache_dir.join(mk_rel_file!("hurryd.pid"));
    let socket_path = hurry_cache_dir.join(mk_rel_file!("hurryd.sock"));
    Ok(DaemonPaths {
        pid_file_path,
        socket_path,
    })
}

pub async fn daemon_is_running(pid_file_path: &AbsFilePath) -> Result<bool> {
    if pid_file_path.exists().await {
        let pid = fs::must_read_buffered_utf8(pid_file_path).await?;
        match pid.trim().parse::<u32>() {
            Ok(pid) => {
                let system = System::new_with_specifics(
                    RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing()),
                );
                let process = system.process(Pid::from_u32(pid));
                process.is_some()
            }
            Err(err) => {
                warn!(?err, "could not parse pid-file");
                false
            }
        }
    } else {
        false
    }
    .pipe(Ok)
}
