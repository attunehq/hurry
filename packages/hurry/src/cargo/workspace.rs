use std::{fmt::Debug as StdDebug, marker::PhantomData, path::PathBuf, sync::Arc};

use ahash::{HashMap, HashSet};
use cargo_metadata::camino::Utf8PathBuf;
use color_eyre::{
    Result,
    eyre::{Context, OptionExt},
};
use derive_more::{Debug, Display};
use itertools::Itertools;
use location_macros::workspace_dir;
use relative_path::{PathExt, RelativePathBuf};
use tap::{Pipe, Tap, TapFallible};
use tokio::task::spawn_blocking;
use tracing::{debug, instrument, trace};

use crate::{
    Locked, Unlocked,
    cache::Artifact,
    fs::{self, Index, LockFile},
    hash::Blake3,
};

use super::{
    Profile,
    dependency::Dependency,
    metadata::{Dotd, RustcMetadata},
    read_argv,
};

/// Represents a Cargo workspace with caching metadata.
///
/// A workspace is the root container for a Rust project, containing
/// the `Cargo.toml`, `Cargo.lock`, and `target/` directory. This struct
/// holds parsed metadata needed for intelligent caching of build artifacts.
///
/// Note: For hurry's purposes, workspace and non-workspace projects
/// are treated identically.
#[derive(Debug, Display)]
#[display("{root}")]
pub struct Workspace {
    /// The root directory of the workspace.
    pub root: Utf8PathBuf,

    /// The target directory in the workspace.
    #[debug(skip)]
    pub target: Utf8PathBuf,

    /// Parsed `rustc` metadata relating to the current workspace.
    #[debug(skip)]
    pub rustc: RustcMetadata,

    /// Dependencies in the workspace, keyed by [`Dependency::key`].
    #[debug(skip)]
    pub dependencies: HashMap<Blake3, Dependency>,
}

impl Workspace {
    /// Create a workspace by parsing metadata from the given directory.
    ///
    /// Loads and parses `Cargo.toml`, `Cargo.lock`, and rustc metadata
    /// to build a complete picture of the workspace for caching purposes.
    /// Only includes third-party dependencies from the default registry
    /// in the dependencies map.
    //
    // TODO: A few of these setup steps could be parallelized...
    // I'm not certain they're worth the thread spawn cost
    // but this can be mitigated by using the rayon thread pool.
    #[instrument(name = "Workspace::from_argv_in_dir")]
    pub async fn from_argv_in_dir(
        path: impl Into<PathBuf> + StdDebug,
        argv: &[String],
    ) -> Result<Self> {
        let path = path.into();

        // TODO: Maybe we should just replicate this logic and perform it
        // statically using filesystem operations instead of shelling out? This
        // costs something on the order of 200ms, which is not _terrible_ but
        // feels much slower than if we just did our own filesystem reads.
        let manifest_path = read_argv(argv, "--manifest-path").map(String::from);
        let metadata = spawn_blocking(move || -> Result<_> {
            cargo_metadata::MetadataCommand::new()
                .tap_mut(|cmd| {
                    if let Some(p) = manifest_path {
                        cmd.manifest_path(p);
                    }
                })
                .current_dir(&path)
                .exec()
                .context("could not read cargo metadata")
        })
        .await
        .context("join task")?
        .tap_ok(|metadata| debug!(?metadata, "cargo metadata"))
        .context("read cargo metadata")?;

        // TODO: This currently blows up if we have no lockfile.
        let cargo_lock = metadata.workspace_root.join("Cargo.lock");
        let lockfile = spawn_blocking(move || -> Result<_> {
            cargo_lock::Lockfile::load(cargo_lock).context("load cargo lockfile")
        })
        .await
        .context("join task")?
        .tap_ok(|lockfile| debug!(?lockfile, "cargo lockfile"))
        .context("read cargo lockfile")?;

        let rustc_meta = RustcMetadata::from_argv(&metadata.workspace_root, argv)
            .await
            .tap_ok(|rustc_meta| debug!(?rustc_meta, "rustc metadata"))
            .context("read rustc metadata")?;

        // We only care about third party packages for now.
        //
        // From observation, first party packages seem to have
        // no `source` or `checksum` while third party packages do,
        // so we just filter anything that doesn't have these.
        //
        // In addition, to keep things simple, we filter to only
        // packages that are in the default registry.
        //
        // Only dependencies reported here are actually cached;
        // anything we exclude here is ignored by the caching system.
        //
        // TODO: Support caching packages not in the default registry.
        // TODO: Support caching first party packages.
        // TODO: Support caching git etc packages.
        // TODO: How can we properly report `target` for cross compiled deps?
        let dependencies = lockfile
            .packages
            .into_iter()
            .filter_map(|package| match (&package.source, &package.checksum) {
                (Some(source), Some(checksum)) if source.is_default_registry() => {
                    Dependency::builder()
                        .checksum(checksum.to_string())
                        .name(package.name.to_string())
                        .version(package.version.to_string())
                        .target(&rustc_meta.llvm_target)
                        .build()
                        .pipe(Some)
                }
                _ => {
                    trace!(?package, "skipped indexing package for cache");
                    None
                }
            })
            .map(|dependency| (dependency.key(), dependency))
            .inspect(|(key, dependency)| trace!(?key, ?dependency, "indexed dependency"))
            .collect::<HashMap<_, _>>();

        Ok(Self {
            root: metadata.workspace_root,
            target: metadata.target_directory,
            rustc: rustc_meta,
            dependencies,
        })
    }

    /// Create a workspace from the current working directory.
    ///
    /// Convenience method that calls `from_argv_in_dir`
    /// using the current working directory as the workspace root.
    #[instrument(name = "Workspace::from_argv")]
    pub async fn from_argv(argv: &[String]) -> Result<Self> {
        let pwd = std::env::current_dir().context("get working directory")?;
        Self::from_argv_in_dir(pwd, argv).await
    }

    /// Initialize the target directory structure for a build profile.
    ///
    /// Creates the profile subdirectory under `target/` and writes a
    /// `CACHEDIR.TAG` file to mark it as a cache directory.
    ///
    /// We don't currently have this as a distinct state transition for
    /// workspace as it's unclear whether this is strictly required.
    //
    // TODO: Is this required?
    #[instrument(name = "Workspace::init_target")]
    pub async fn init_target(&self, profile: &Profile) -> Result<()> {
        const CACHEDIR_TAG_NAME: &str = "CACHEDIR.TAG";
        const CACHEDIR_TAG_CONTENT: &[u8] =
            include_bytes!(concat!(workspace_dir!(), "/static/cargo/CACHEDIR.TAG"));

        fs::create_dir_all(self.target.join(profile.as_str()))
            .await
            .context("create target directory")?;
        fs::write(self.target.join(CACHEDIR_TAG_NAME), CACHEDIR_TAG_CONTENT)
            .await
            .context("write CACHEDIR.TAG")
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
/// Represents a specific profile subdirectory
/// (e.g., `target/debug/`, `target/release/`)
/// within a workspace's target directory.
/// Provides controlled access to the directory
/// contents with proper locking to prevent conflicts
/// with concurrent Cargo builds.
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

    /// The index of files inside the profile directory.
    /// Paths in this index are relative to [`ProfileDir::root`].
    ///
    /// This index is built when the profile directory is locked.
    /// Currently there's no explicit unlock mechanism for profiles since
    /// they're just dropped, but if we ever add one that's where we'd clear
    /// this and set it to `None`.
    ///
    /// This is in an `Arc` so that we don't have to clone the whole index
    /// when we clone the `ProfileDir`.
    index: Option<Arc<Index>>,

    /// The root of the directory,
    /// relative to [`workspace.target`](Workspace::target).
    ///
    /// Note: this is intentionally not `pub` because we only want to give
    /// callers access to the directory when the cache is locked;
    /// reference the `root` method in the locked implementation block.
    /// The intention here is to minimize the chance of callers mutating or
    /// referencing the contents of the cache while it is locked.
    root: RelativePathBuf,

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
        workspace
            .init_target(profile)
            .await
            .context("init workspace target")?;

        let root = workspace.target.join(profile.as_str());
        let lock = root.join(".cargo-lock");
        let lock = LockFile::open(lock).await.context("open lockfile")?;
        let root = root
            .as_std_path()
            .relative_to(&workspace.target)
            .context("make root relative")?;

        Ok(Self {
            state: PhantomData,
            index: None,
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
        let root = self.root.to_path(&self.workspace.target);
        let index = Index::recursive(&root)
            .await
            .map(Arc::new)
            .map(Some)
            .context("index target folder")?;
        Ok(ProfileDir {
            state: PhantomData,
            profile: self.profile,
            root: self.root,
            workspace: self.workspace,
            lock,
            index,
        })
    }
}

impl<'ws> ProfileDir<'ws, Locked> {
    /// Find all cache artifacts for a specific dependency.
    ///
    /// Scans the profile directory's file index to locate artifacts that belong
    /// to the given dependency.
    ///
    /// Artifacts are identified by location and naming.
    ///
    /// ## Artifact Types
    /// - **Fingerprint/Build**: Files in `.fingerprint/` or `build/`
    ///   subdirectories where the subdirectory name starts with
    ///   the dependency name.
    /// - **Dependencies**: Files in `deps/` directory, discovered through
    ///   `.d` files that list the actual outputs (`.rlib`, `.rmeta`, etc.)
    ///
    /// ## Contract
    /// - Returns artifacts with paths relative to profile root
    /// - Filters to dependency-specific files to avoid over-caching
    ///
    /// TODO: Evaluate more precise filtering to reduce cache invalidations
    #[instrument(name = "ProfileDir::enumerate_cache_artifacts")]
    pub async fn enumerate_cache_artifacts(
        &self,
        dependency: &Dependency,
    ) -> Result<Vec<Artifact>> {
        let index = self.index.as_ref().ok_or_eyre("files not indexed")?;

        // Fingerprint artifacts are straightforward:
        // if they're inside the `.fingerprint` directory,
        // and the subdirectory of `.fingerprint` starts with the name of
        // the dependency, then they should be backed up.
        //
        // Builds are the same as fingerprints, just with a different root:
        // instead of `.fingerprint`, they're looking for `build`.
        let standard = index
            .files
            .iter()
            .filter(|(path, _)| {
                path.components()
                    .tuple_windows()
                    .next()
                    .is_some_and(|(parent, child)| {
                        child.as_str().starts_with(&dependency.name)
                            && (parent.as_str() == ".fingerprint" || parent.as_str() == "build")
                    })
            })
            .collect_vec();

        // Dependencies are totally different from the two above.
        // This directory is flat; inside we're looking for one primary file:
        // a `.d` file whose name starts with the name of the dependency.
        //
        // This file then lists other files (e.g. `*.rlib` and `*.rmeta`)
        // that this dependency built; these are _often_ (but not always)
        // named with a different prefix (often, but not always, "lib").
        //
        // Along the way we also grab any other random file in here that is
        // prefixed with the name of the dependency; so far this has been
        // `.rcgu.o` files which appear to be compiled codegen.
        //
        // Not all dependencies create `.d` files or indeed anything else
        // in the `deps` folder- from observation, it seems that this is the
        // case for purely proc-macro crates. This is honestly mostly ok,
        // because we want those to run anyway for now (until we figure out
        // a way to cache proc-macro invocations).
        let dependencies = index
            .files
            .iter()
            .filter(|(path, _)| {
                path.components()
                    .next()
                    .is_some_and(|part| part.as_str() == "deps")
            })
            .collect_vec();
        let dotd = dependencies.iter().find(|(path, _)| {
            path.components().nth(1).is_some_and(|part| {
                part.as_str().ends_with(".d") && part.as_str().starts_with(&dependency.name)
            })
        });
        let dependencies = if let Some((path, _)) = dotd {
            let outputs = Dotd::from_file(self, path)
                .await
                .context("parse .d file")?
                .outputs
                .into_iter()
                .collect::<HashSet<_>>();
            dependencies
                .into_iter()
                .filter(|(path, _)| {
                    outputs.contains(*path)
                        || path
                            .file_name()
                            .is_some_and(|name| name.starts_with(&dependency.name))
                })
                .collect_vec()
        } else {
            Vec::new()
        };

        // Now that we have our three sources of files,
        // we actually treat them all the same way!
        standard
            .into_iter()
            .chain(dependencies)
            .map(|(path, entry)| Artifact::builder().target(path).hash(&entry.hash).build())
            .inspect(|artifact| trace!(?artifact, "enumerated artifact"))
            .collect::<Vec<_>>()
            .pipe(Ok)
    }

    /// Get the absolute path to the profile directory root.
    ///
    /// Converts the internal relative path to an absolute path
    /// based on the workspace's target directory.
    pub fn root(&self) -> PathBuf {
        self.root.to_path(&self.workspace.target)
    }
}
