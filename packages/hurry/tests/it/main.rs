use std::path::PathBuf;

use hurry::path::{AbsDirPath, TryJoinWith};
use location_macros::workspace_dir;
use tempfile::TempDir;

pub mod cargo;
pub mod fs;

#[track_caller]
pub fn current_workspace() -> AbsDirPath {
    let ws = workspace_dir!();
    AbsDirPath::new(PathBuf::from(ws))
        .unwrap_or_else(|err| panic!("parse {ws:?} as abs dir: {err:?}"))
}

#[track_caller]
fn current_target() -> AbsDirPath {
    current_workspace()
        .try_join_dir("target")
        .expect("parse target directory")
}

#[track_caller]
pub fn temporary_directory() -> (TempDir, AbsDirPath) {
    let dir = TempDir::new().expect("create temporary directory");
    let path = AbsDirPath::try_from(dir.path()).expect("read temp dir as abs dir");
    (dir, path)
}
