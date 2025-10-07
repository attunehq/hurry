use std::marker::PhantomData;

use color_eyre::{Result, eyre::Context};
use derive_more::{Debug, Display};
use location_macros::workspace_dir;
use tap::{Pipe as _, Tap as _, TapFallible as _};
use tokio::task::spawn_blocking;
use tracing::{debug, instrument, trace, warn};

use crate::{
    Locked, Unlocked,
    fs::{self, LockFile},
    mk_rel_file,
    path::{AbsDirPath, JoinWith as _, TryJoinWith as _},
};

use super::{Profile, read_argv};

/// Represents a Cargo workspace with caching metadata.
///
/// A workspace is the root container for a Rust project, containing
/// the `Cargo.toml`, `Cargo.lock`, and `target/` directory. This struct
/// holds parsed metadata needed for intelligent caching of build artifacts.
///
/// Note: For hurry's purposes, workspace and non-workspace projects
/// are treated identically.
#[derive(Clone, Eq, PartialEq, Debug, Display)]
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
}

impl Workspace {
    /// Create a workspace by parsing `cargo metadata` from the given directory.
    #[instrument(name = "Workspace::from_argv_in_dir")]
    pub async fn from_argv_in_dir(path: &AbsDirPath, argv: &[String]) -> Result<Self> {
        // TODO: Maybe we should just replicate this logic and perform it
        // statically using filesystem operations instead of shelling out? This
        // costs something on the order of 200ms, which is not _terrible_ but
        // feels much slower than if we just did our own filesystem reads,
        // especially since we don't actually use any of the logic except the
        // paths.
        let manifest_path = read_argv(argv, "--manifest-path").map(String::from);
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

        let workspace_root = AbsDirPath::try_from(&metadata.workspace_root)
            .context("parse workspace root as absolute directory")?;
        let workspace_target = AbsDirPath::try_from(&metadata.target_directory)
            .context("parse workspace target as absolute directory")?;

        let cargo_home = spawn_blocking({
            let workspace_root = workspace_root.clone();
            move || home::cargo_home_with_cwd(workspace_root.as_std_path())
        })
        .await
        .context("join background task")?
        .context("get $CARGO_HOME")?
        .pipe(AbsDirPath::try_from)
        .context("parse path as utf8")?;

        Ok(Self {
            root: workspace_root,
            target: workspace_target,
            cargo_home,
        })
    }

    /// Create a workspace from the current working directory.
    ///
    /// Convenience method that calls `from_argv_in_dir`
    /// using the current working directory as the workspace root.
    #[instrument(name = "Workspace::from_argv")]
    pub async fn from_argv(argv: &[String]) -> Result<Self> {
        let pwd = AbsDirPath::current().context("get working directory")?;
        Self::from_argv_in_dir(&pwd, argv).await
    }

    /// Initialize the target directory structure for a build profile.
    ///
    /// Creates the profile subdirectory under `target/` and writes a
    /// `CACHEDIR.TAG` file to mark it as a cache directory,
    /// then returns the path to the profile directory that was created.
    ///
    /// We don't currently have this as a distinct state transition for
    /// workspace as it's unclear whether this is strictly required.
    //
    // TODO: Is this required?
    #[instrument(name = "Workspace::init_target")]
    pub async fn init_target(&self, profile: &Profile) -> Result<AbsDirPath> {
        const CACHEDIR_TAG_CONTENT: &[u8] =
            include_bytes!(concat!(workspace_dir!(), "/static/cargo/CACHEDIR.TAG"));

        // We don't think the profile directory exists yet.
        let profile = self.target.try_join_dir(profile.as_str())?;
        fs::create_dir_all(&profile)
            .await
            .context("create target directory")?;
        fs::write(
            &self.target.join(&mk_rel_file!("CACHEDIR.TAG")),
            CACHEDIR_TAG_CONTENT,
        )
        .await
        .context("write CACHEDIR.TAG")?;
        Ok(profile)
    }

    /// Open a profile directory for reading.
    pub async fn open_profile(&self, profile: &Profile) -> Result<ProfileDir<'_, Unlocked>> {
        ProfileDir::open(self, profile).await
    }

    /// Open a profile directory with exclusive write access.
    ///
    /// Acquires a file lock and builds a file index of the profile directory.
    /// Required for cache operations that modify the target directory.
    pub async fn open_profile_locked(&self, profile: &Profile) -> Result<ProfileDir<'_, Locked>> {
        self.open_profile(profile)
            .await
            .context("open profile")?
            .pipe(|target| target.lock())
            .await
            .context("lock profile")
    }
}

/// A build profile directory within a Cargo workspace.
///
/// Represents a specific profile subdirectory (e.g., `target/debug/`,
/// `target/release/`) within a workspace's target directory. Provides
/// controlled access to the directory contents with proper locking to prevent
/// conflicts with concurrent Cargo builds and with rust-analyzer, which will
/// attempt to concurrently run `cargo check` if it is present (e.g. if a user
/// has their IDE open).
///
/// ## State Management
/// - `Unlocked`: No file operations allowed
/// - `Locked`: Exclusive access with file index and directory
/// - Locking is compatible with Cargo's own locking mechanism
#[derive(Debug, Clone)]
pub struct ProfileDir<'ws, State> {
    #[debug(skip)]
    state: PhantomData<State>,

    /// The lockfile for the directory.
    ///
    /// The intention of this lock is to prevent multiple `hurry` _or `cargo`_
    /// instances from mutating the state of the directory at the same time,
    /// or from mutating it at the same time as another instance
    /// is reading it.
    ///
    /// This lockfile uses the same name and implementation as `cargo` uses,
    /// so a locked `ProfileDir` in `hurry` will block `cargo` and vice versa.
    #[debug(skip)]
    lock: LockFile<State>,

    /// The workspace in which this build profile is located.
    pub workspace: &'ws Workspace,

    /// The root of the profile directory inside [`Workspace::target`].
    ///
    /// Note: this is intentionally not `pub` because we only want to give
    /// callers access to the directory when the cache is locked;
    /// reference the `root` method in the locked implementation block.
    /// The intention here is to minimize the chance of callers mutating or
    /// referencing the contents of the cache while it is locked.
    root: AbsDirPath,

    /// The profile to which this directory refers.
    ///
    /// By default, profiles are `release`, `debug`, `test`, and `bench`
    /// although users can also define custom profiles, which is why
    /// this value is an opaque string:
    /// https://doc.rust-lang.org/cargo/reference/profiles.html#custom-profiles
    pub profile: Profile,
}

impl<'ws> ProfileDir<'ws, Unlocked> {
    /// Open a profile directory in unlocked mode.
    #[instrument(name = "ProfileDir::open")]
    pub async fn open(workspace: &'ws Workspace, profile: &Profile) -> Result<Self> {
        let root = workspace
            .init_target(profile)
            .await
            .context("init workspace target")?;
        let lock = LockFile::open(root.join(mk_rel_file!(".cargo-lock")))
            .await
            .context("open lockfile")?;

        Ok(Self {
            state: PhantomData,
            profile: profile.clone(),
            root,
            lock,
            workspace,
        })
    }

    /// Acquire exclusive lock and build file index.
    #[instrument(name = "ProfileDir::lock")]
    pub async fn lock(self) -> Result<ProfileDir<'ws, Locked>> {
        let lock = self.lock.lock().await.context("lock profile")?;
        Ok(ProfileDir {
            state: PhantomData,
            profile: self.profile,
            root: self.root,
            lock,
            workspace: self.workspace,
        })
    }
}

impl<'ws> ProfileDir<'ws, Locked> {
    /// Get the absolute path to the profile directory root.
    ///
    /// Converts the internal relative path to an absolute path
    /// based on the workspace's target directory.
    pub fn root(&self) -> &AbsDirPath {
        &self.root
    }
}
