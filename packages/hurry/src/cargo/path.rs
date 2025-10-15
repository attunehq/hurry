use std::{
    fmt::Debug,
    path::{Path, PathBuf},
};

use color_eyre::{Result, eyre::bail};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::workspace::ProfileDir;
use crate::{
    Locked, fs,
    path::{AbsDirPath, AbsFilePath, JoinWith as _, RelFilePath, RelativeTo as _},
};

/// A "qualified" path inside a Cargo project.
///
/// Paths in some files, such as "dep-info" files or build script outputs, are
/// sometimes written using absolute paths. However `hurry` wants to know what
/// these paths are relative to so that it can back up and restore paths in
/// different workspaces and machines. This type supports `hurry` being able to
/// determine what kind of path is being referenced.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize)]
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
}

impl QualifiedPath {
    #[instrument(name = "QualifiedPath::parse_string")]
    pub async fn parse_string(profile: &ProfileDir<'_, Locked>, path: &str) -> Result<Self> {
        Ok(if let Ok(rel) = RelFilePath::try_from(path) {
            if fs::exists(profile.root().join(&rel).as_std_path()).await {
                Self::RelativeTargetProfile(rel)
            } else if fs::exists(profile.workspace.cargo_home.join(&rel).as_std_path()).await {
                Self::RelativeCargoHome(rel)
            } else {
                Self::Rootless(rel)
            }
        } else if let Ok(abs) = AbsFilePath::try_from(path) {
            if let Ok(rel) = abs.relative_to(profile.root()) {
                Self::RelativeTargetProfile(rel)
            } else if let Ok(rel) = abs.relative_to(&profile.workspace.cargo_home) {
                Self::RelativeCargoHome(rel)
            } else {
                bail!("unknown root for absolute path: {abs:?}");
            }
        } else {
            bail!("unknown kind of path: {path:?}")
        })
    }

    #[instrument(name = "QualifiedPath::parse")]
    pub async fn parse(profile: &ProfileDir<'_, Locked>, path: &Path) -> Result<Self> {
        Ok(if let Ok(rel) = RelFilePath::try_from(path) {
            if fs::exists(profile.root().join(&rel).as_std_path()).await {
                Self::RelativeTargetProfile(rel)
            } else if fs::exists(profile.workspace.cargo_home.join(&rel).as_std_path()).await {
                Self::RelativeCargoHome(rel)
            } else {
                Self::Rootless(rel)
            }
        } else if let Ok(abs) = AbsFilePath::try_from(path) {
            if let Ok(rel) = abs.relative_to(profile.root()) {
                Self::RelativeTargetProfile(rel)
            } else if let Ok(rel) = abs.relative_to(&profile.workspace.cargo_home) {
                Self::RelativeCargoHome(rel)
            } else {
                bail!("unknown root for absolute path: {abs:?}");
            }
        } else {
            bail!("unknown kind of path: {path:?}")
        })
    }

    #[instrument(name = "QualifiedPath::reconstruct_string")]
    pub fn reconstruct_string(&self, profile: &ProfileDir<'_, Locked>) -> String {
        match self {
            QualifiedPath::Rootless(rel) => rel.to_string(),
            QualifiedPath::RelativeTargetProfile(rel) => profile.root().join(rel).to_string(),
            QualifiedPath::RelativeCargoHome(rel) => {
                profile.workspace.cargo_home.join(rel).to_string()
            }
        }
    }

    #[instrument(name = "QualifiedPath::reconstruct")]
    pub fn reconstruct(&self, profile: &ProfileDir<'_, Locked>) -> PathBuf {
        match self {
            QualifiedPath::Rootless(rel) => rel.into(),
            QualifiedPath::RelativeTargetProfile(rel) => profile.root().join(rel).into(),
            QualifiedPath::RelativeCargoHome(rel) => profile.workspace.cargo_home.join(rel).into(),
        }
    }

    #[instrument(name = "QualifiedPath::reconstruct_raw")]
    pub fn reconstruct_raw(&self, profile_root: &AbsDirPath, cargo_home: &AbsDirPath) -> PathBuf {
        match self {
            QualifiedPath::Rootless(rel) => rel.into(),
            QualifiedPath::RelativeTargetProfile(rel) => profile_root.join(rel).into(),
            QualifiedPath::RelativeCargoHome(rel) => cargo_home.join(rel).into(),
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
        }
    }
}
