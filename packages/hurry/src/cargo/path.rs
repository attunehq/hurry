use std::{
    fmt::Debug,
    path::{Path, PathBuf},
};

use color_eyre::{Result, eyre::bail};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::{
    cargo::Workspace,
    fs,
    path::{AbsDirPath, AbsFilePath, JoinWith as _, RelFilePath, RelativeTo as _},
};

/// A "qualified" path inside a Cargo project.
///
/// Paths in some files, such as "dep-info" files or build script outputs, are
/// sometimes written using absolute paths. However `hurry` wants to know what
/// these paths are relative to so that it can back up and restore paths in
/// different workspaces and machines. This type supports `hurry` being able to
/// determine what kind of path is being referenced.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Deserialize, Serialize)]
#[serde(tag = "t", content = "c")]
pub enum QualifiedPath {
    /// The path is "natively" relative without a root prior to `hurry` making
    /// it relative. Such paths are backed up and restored "as-is".
    ///
    /// Note: Since these paths are natively written as relative paths,
    /// it's not necessarily clear to what file these are referring without more
    /// context (such as the kind of file that contained the path and its
    /// location). If this is ever a problem, we'll probably need to change how
    /// we represent this type- maybe e.g. provide a "computed" path relative to
    /// a known root along with the "native" version of the path.
    Rootless(RelFilePath),

    /// The path is relative to the workspace target profile directory.
    RelativeTargetProfile(RelFilePath),

    /// The path is relative to `$CARGO_HOME` for the user.
    RelativeCargoHome(RelFilePath),

    /// The path is an absolute path.
    ///
    /// In practice, we think this means it's SDK headers, system libraries,
    /// etc; items that are _assumed_ to be available on any machine where the
    /// build is restored.
    ///
    /// The reason we consider this safe is because we cache artifacts by Rust
    /// target triple along with other values; these should make it such that
    /// different operating systems don't end up having different system paths.
    ///
    /// As a future optimization we may want to enumerate and add various system
    /// path roots (e.g. the macOS SDK root, etc) that go through more specific
    /// handling before ultimately falling back to this option.
    Absolute(AbsFilePath),
}

impl QualifiedPath {
    #[instrument(name = "QualifiedPath::parse_string")]
    pub async fn parse_string(ws: &Workspace, path: &str) -> Result<Self> {
        Ok(if let Ok(rel) = RelFilePath::try_from(path) {
            if fs::exists(ws.profile_dir.join(&rel).as_std_path()).await {
                Self::RelativeTargetProfile(rel)
            } else if fs::exists(ws.cargo_home.join(&rel).as_std_path()).await {
                Self::RelativeCargoHome(rel)
            } else {
                Self::Rootless(rel)
            }
        } else if let Ok(abs) = AbsFilePath::try_from(path) {
            if let Ok(rel) = abs.relative_to(&ws.profile_dir) {
                Self::RelativeTargetProfile(rel)
            } else if let Ok(rel) = abs.relative_to(&ws.cargo_home) {
                Self::RelativeCargoHome(rel)
            } else {
                Self::Absolute(abs)
            }
        } else {
            bail!("unknown kind of path: {path:?}")
        })
    }

    #[instrument(name = "QualifiedPath::parse")]
    pub async fn parse(ws: &Workspace, path: &Path) -> Result<Self> {
        Ok(if let Ok(rel) = RelFilePath::try_from(path) {
            if fs::exists(ws.profile_dir.join(&rel).as_std_path()).await {
                Self::RelativeTargetProfile(rel)
            } else if fs::exists(ws.cargo_home.join(&rel).as_std_path()).await {
                Self::RelativeCargoHome(rel)
            } else {
                Self::Rootless(rel)
            }
        } else if let Ok(abs) = AbsFilePath::try_from(path) {
            if let Ok(rel) = abs.relative_to(&ws.profile_dir) {
                Self::RelativeTargetProfile(rel)
            } else if let Ok(rel) = abs.relative_to(&ws.cargo_home) {
                Self::RelativeCargoHome(rel)
            } else {
                Self::Absolute(abs)
            }
        } else {
            bail!("unknown kind of path: {path:?}")
        })
    }

    #[instrument(name = "QualifiedPath::reconstruct_string")]
    pub fn reconstruct_string(&self, ws: &Workspace) -> String {
        match self {
            QualifiedPath::Rootless(rel) => rel.to_string(),
            QualifiedPath::RelativeTargetProfile(rel) => ws.profile_dir.join(rel).to_string(),
            QualifiedPath::RelativeCargoHome(rel) => ws.cargo_home.join(rel).to_string(),
            QualifiedPath::Absolute(abs) => abs.to_string(),
        }
    }

    #[instrument(name = "QualifiedPath::reconstruct")]
    pub fn reconstruct(&self, ws: &Workspace) -> PathBuf {
        match self {
            QualifiedPath::Rootless(rel) => rel.into(),
            QualifiedPath::RelativeTargetProfile(rel) => ws.profile_dir.join(rel).into(),
            QualifiedPath::RelativeCargoHome(rel) => ws.cargo_home.join(rel).into(),
            QualifiedPath::Absolute(abs) => abs.as_std_path().into(),
        }
    }

    #[instrument(name = "QualifiedPath::reconstruct_raw")]
    pub fn reconstruct_raw(&self, profile_root: &AbsDirPath, cargo_home: &AbsDirPath) -> PathBuf {
        match self {
            QualifiedPath::Rootless(rel) => rel.into(),
            QualifiedPath::RelativeTargetProfile(rel) => profile_root.join(rel).into(),
            QualifiedPath::RelativeCargoHome(rel) => cargo_home.join(rel).into(),
            QualifiedPath::Absolute(abs) => abs.as_std_path().into(),
        }
    }

    #[instrument(name = "QualifiedPath::reconstruct_raw_string")]
    pub fn reconstruct_raw_string(
        &self,
        profile_root: &AbsDirPath,
        cargo_home: &AbsDirPath,
    ) -> String {
        match self {
            QualifiedPath::Rootless(rel) => rel.to_string(),
            QualifiedPath::RelativeTargetProfile(rel) => profile_root.join(rel).to_string(),
            QualifiedPath::RelativeCargoHome(rel) => cargo_home.join(rel).to_string(),
            QualifiedPath::Absolute(abs) => abs.to_string(),
        }
    }
}
