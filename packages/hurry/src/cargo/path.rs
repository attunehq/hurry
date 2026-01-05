use std::fmt::Debug;

use color_eyre::{Result, eyre::bail};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::{
    cargo::{RustcTarget, UnitPlanInfo, Workspace},
    fs,
    path::{AbsFilePath, GenericPath, JoinWith as _, RelFilePath, RelativeTo as _},
};

/// A "qualified" path inside a Cargo project.
///
/// Semantically relative paths in some files (e.g. dep-info files, build script
/// outputs, etc.) are sometimes written as resolved absolute paths. However,
/// `hurry` needs to recognize that these paths are relative so it can rewrite
/// them when restoring artifacts to different machines with different paths.
/// This type implements path parsing and rewriting.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Deserialize, Serialize)]
#[serde(tag = "t", content = "c")]
pub enum QualifiedPath {
    /// The path is originally written as relative. Such paths are backed up and
    /// restored "as-is".
    Rootless(RelFilePath),

    /// The absolute path is relative to the workspace target profile directory.
    RelativeTargetProfile(RelFilePath),

    /// The absolute path is relative to `$CARGO_HOME` for the user.
    RelativeCargoHome(RelFilePath),

    /// The absolute path is not relative to any known root.
    ///
    /// In practice, these are paths to SDK headers, system libraries, etc.
    /// items that are at known paths on machines. Crates semantically should
    /// not be referencing absolute paths without also emitting Cargo directives
    /// to invalidate builds when the files at those paths change (e.g. see how
    /// the openssl build script discovers the system SSL library[^1]).
    ///
    /// We handle these paths by handling build script output directives.
    ///
    /// In the future, we'll enumerate more roots (e.g. macOS SDK, Homebrew) and
    /// add specific handling if needed.
    ///
    /// [^1]: https://github.com/rust-openssl/rust-openssl/blob/09b90d036ec5341deefb7fce86748e176379d01a/openssl-sys/build/find_normal.rs#L72
    Absolute(AbsFilePath),
}

impl QualifiedPath {
    pub async fn parse_string(ws: &Workspace, target: &RustcTarget, path: &str) -> Result<Self> {
        Self::parse(ws, target, &GenericPath::try_from(path)?).await
    }

    #[instrument(name = "QualifiedPath::parse")]
    pub async fn parse(
        ws: &Workspace,
        // TODO: This should be UnitPlanInfo so we can use
        // ws.unit_profile_dir(), but we can't migrate over until all call-sites
        // are ready (because we can easily construct a default RustcTarget but
        // less so a default UnitPlanInfo).
        target: &RustcTarget,
        path: &GenericPath,
    ) -> Result<Self> {
        // TODO: Do we see repeated paths a lot? Should we cache the
        // `fs::exists` calls?
        let profile_dir = ws.arch_profile_dir(target);
        Ok(if let Ok(rel) = RelFilePath::try_from(path) {
            if fs::exists(profile_dir.join(&rel).as_std_path()).await {
                Self::RelativeTargetProfile(rel)
            } else if fs::exists(ws.cargo_home.join(&rel).as_std_path()).await {
                Self::RelativeCargoHome(rel)
            } else {
                Self::Rootless(rel)
            }
        } else if let Ok(abs) = AbsFilePath::try_from(path) {
            if let Ok(rel) = abs.relative_to(&profile_dir) {
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

    /// Parse absolute paths into qualified paths.
    ///
    /// This is separate from `QualifiedPath::parse` because absolute paths do
    /// not need to be parsed asynchronously or fallibly.
    #[instrument(name = "QualifiedPath::parse_abs")]
    pub fn parse_abs(ws: &Workspace, target: &RustcTarget, path: &AbsFilePath) -> Self {
        let profile_dir = ws.arch_profile_dir(target);
        if let Ok(rel) = path.relative_to(&profile_dir) {
            Self::RelativeTargetProfile(rel)
        } else if let Ok(rel) = path.relative_to(&ws.cargo_home) {
            Self::RelativeCargoHome(rel)
        } else {
            Self::Absolute(path.clone())
        }
    }

    #[instrument(name = "QualifiedPath::reconstruct_string")]
    pub fn reconstruct_string(self, ws: &Workspace, target: &RustcTarget) -> String {
        self.reconstruct_inner(ws, target).to_string()
    }

    #[instrument(name = "QualifiedPath::reconstruct")]
    pub fn reconstruct(self, ws: &Workspace, unit_info: &UnitPlanInfo) -> GenericPath {
        self.reconstruct_inner(ws, &unit_info.target_arch)
    }

    fn reconstruct_inner(self, ws: &Workspace, target: &RustcTarget) -> GenericPath {
        let profile_dir = ws.arch_profile_dir(target);
        match self {
            QualifiedPath::Rootless(rel) => rel.into(),
            QualifiedPath::RelativeTargetProfile(rel) => profile_dir.join(rel).into(),
            QualifiedPath::RelativeCargoHome(rel) => ws.cargo_home.join(rel).into(),
            QualifiedPath::Absolute(abs) => abs.into(),
        }
    }
}
