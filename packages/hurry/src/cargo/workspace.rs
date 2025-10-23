use std::fmt::Debug;

use color_eyre::{Result, eyre::Context};
use derive_more::{Debug as DebugExt, Display};
use serde::{Deserialize, Serialize};
use tap::{Pipe as _, Tap as _, TapFallible as _};
use tokio::task::spawn_blocking;
use tracing::{debug, instrument};

use crate::path::{AbsDirPath, TryJoinWith as _};

use super::{CargoBuildArguments, Profile};

/// The Cargo workspace of a build.
///
/// Workspaces contain all the paths needed to unambiguously specify the files
/// in a build. Note that workspaces are constructed with a specific profile in
/// mind, which we parse from the build command's arguments.
#[derive(Clone, Eq, PartialEq, Hash, DebugExt, Display, Serialize, Deserialize)]
#[display("{root}")]
pub struct Workspace {
    /// The root directory of the workspace.
    pub root: AbsDirPath,

    /// The target directory in the workspace.
    #[debug(skip)]
    pub target: AbsDirPath,

    /// The $CARGO_HOME value.
    #[debug(skip)]
    pub cargo_home: AbsDirPath,

    /// The build profile of this workspace invocation.
    pub profile: Profile,

    /// The build profile target directory.
    #[debug(skip)]
    pub profile_dir: AbsDirPath,
}

impl Workspace {
    /// Create a workspace by parsing `cargo metadata` from the given directory.
    #[instrument(name = "Workspace::from_argv_in_dir")]
    pub async fn from_argv_in_dir(
        path: &AbsDirPath,
        args: impl AsRef<CargoBuildArguments> + Debug,
    ) -> Result<Self> {
        let args = args.as_ref();

        let (workspace_root, workspace_target) = {
            // TODO: Maybe we should just replicate this logic and perform it
            // statically using filesystem operations instead of shelling out?
            // This costs something on the order of 200ms, which is not
            // _terrible_ but feels much slower than if we just did our own
            // filesystem reads, especially since we don't actually use any of
            // the logic except the paths.
            let manifest_path = args.manifest_path().map(String::from);
            let cmd_current_dir = path.as_std_path().to_path_buf();
            let metadata = spawn_blocking(move || -> Result<_> {
                cargo_metadata::MetadataCommand::new()
                    .tap_mut(|cmd| {
                        if let Some(p) = manifest_path {
                            cmd.manifest_path(p);
                        }
                    })
                    .current_dir(cmd_current_dir)
                    .exec()
                    .context("exec and parse cargo metadata")
            })
            .await
            .context("join task")?
            .tap_ok(|metadata| debug!(?metadata, "cargo metadata"))
            .context("get cargo metadata")?;
            (
                AbsDirPath::try_from(&metadata.workspace_root)
                    .context("parse workspace root as absolute directory")?,
                AbsDirPath::try_from(&metadata.target_directory)
                    .context("parse workspace target as absolute directory")?,
            )
        };

        let cargo_home = home::cargo_home_with_cwd(workspace_root.as_std_path())?
            .pipe(AbsDirPath::try_from)
            .context("parse path as utf8")?;

        let profile = args.profile().map(Profile::from).unwrap_or(Profile::Debug);
        let profile_dir = workspace_target.try_join_dir(profile.as_str())?;

        Ok(Self {
            root: workspace_root,
            target: workspace_target,
            cargo_home,
            profile,
            profile_dir,
        })
    }

    /// Create a workspace from the current working directory.
    ///
    /// Convenience method that calls `from_argv_in_dir`
    /// using the current working directory as the workspace root.
    #[instrument(name = "Workspace::from_argv")]
    pub async fn from_argv(args: impl AsRef<CargoBuildArguments> + Debug) -> Result<Self> {
        let pwd = AbsDirPath::current().context("get working directory")?;
        Self::from_argv_in_dir(&pwd, args).await
    }
}
