use std::{collections::BTreeMap, time::SystemTime};

use serde::{Deserialize, Serialize};

use crate::path::{AbsDirPath, TryJoinWith};

/// Records the raw `rustc` invocation information.
#[derive(Debug, Serialize, Deserialize)]
pub struct RawRustcInvocation {
    pub timestamp: SystemTime,
    pub invocation: Vec<String>,
    // Use BTreeMap instead of HashMap so the JSON is sorted.
    pub env: BTreeMap<String, String>,
    pub cwd: String,
}

// TODO: Just set the log_dir fully rendered, no need to set an ID.
pub const INVOCATION_ID_ENV_VAR: &str = "HURRY_CARGO_INVOCATION_ID";
pub const INVOCATION_LOG_DIR_ENV_VAR: &str = "HURRY_CARGO_INVOCATION_LOG_DIR";

pub fn invocation_log_dir(workspace_target_dir: &AbsDirPath) -> AbsDirPath {
    workspace_target_dir
        .try_join_dirs(["hurry", "rustc"])
        .expect("rustc invocation log dir should be valid")
}

pub struct RustcInvocation {}
