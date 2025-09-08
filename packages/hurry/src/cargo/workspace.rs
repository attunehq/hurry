use std::{fmt::Debug as StdDebug, marker::PhantomData, path::PathBuf, sync::Arc};

use ahash::{HashMap, HashSet};
use cargo_metadata::{TargetKind, camino::Utf8PathBuf};
use color_eyre::{
    Result,
    eyre::{Context, OptionExt},
};
use derive_more::{Debug, Display};
use itertools::{Itertools, process_results};
use location_macros::workspace_dir;
use regex::Regex;
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

        // We need to know which packages are first-party workspace members for
        // two reasons:
        //
        // 1. We cache them differently in order to correctly handle incremental
        //    compilation. (Right now, we don't cache them at all.)
        // 2. Workspace members can be binary-only crates if they're
        //    entrypoints. This means our normal library crate name resolution
        //    won't (and _shouldn't_) work for them, so we need to handle them
        //    as a special case. Note that not all workspace members are
        //    binary-only crates! Some may have library crates, and some may be
        //    used as dependencies.
        let workspace_package_names = metadata
            .workspace_members
            .iter()
            .map(|id| {
                metadata
                    .packages
                    .iter()
                    .find(|package| package.id == *id)
                    .ok_or_eyre("workspace member not found")
                    .map(|package| package.name.clone())
            })
            .collect::<Result<HashSet<_>>>()?;

        // Construct a mapping of _package names_ to _library crate names_.
        let package_to_lib = metadata
            .packages
            .iter()
            .filter_map(|package| {
                let libs = package
                    .targets
                    .iter()
                    .filter_map(|target| {
                        if target.kind.contains(&TargetKind::Lib)
                            || target.kind.contains(&TargetKind::ProcMacro)
                            || target.kind.contains(&TargetKind::RLib)
                        {
                            Some(target.name.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                let len = libs.len();
                // Workspace members might be binary-only crates.
                if len < 1 && workspace_package_names.contains(&package.name) {
                    return None;
                }
                libs.into_iter()
                    .exactly_one()
                    .with_context(|| format!(
                        "package {} has {} library targets but expected 1: {:?}",
                        package.name, len, package.targets
                    ))
                    .map(|lib| (package.name.as_str(), lib))
                    .pipe(Some)
            })
            .collect::<Result<HashMap<_, _>>>()?;

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
        let dependencies = lockfile.packages.into_iter().filter_map(|package| {
            match (&package.source, &package.checksum) {
                (Some(source), Some(checksum)) if source.is_default_registry() => {
                    // Ignore workspace members.
                    if workspace_package_names.contains(package.name.as_str()) {
                        return None;
                    }

                    package_to_lib
                        .get(package.name.as_str())
                        .ok_or_eyre(format!(
                            "package {:?} has unknown library target",
                            package.name.as_str()
                        ))
                        .map(|lib_name| {
                            Dependency::builder()
                                .checksum(checksum.to_string())
                                .package_name(package.name.to_string())
                                .lib_name(lib_name)
                                .version(package.version.to_string())
                                .target(&rustc_meta.llvm_target)
                                .build()
                        })
                        .pipe(Some)
                }
                _ => {
                    trace!(?package, "skipped indexing package for cache");
                    None
                }
            }
        });
        let dependencies = process_results(dependencies, |iter| {
            iter.map(|dependency| (dependency.key(), dependency))
                .inspect(|(key, dependency)| trace!(?key, ?dependency, "indexed dependency"))
                .collect::<HashMap<_, _>>()
        })?;

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
    /// # Dependencies
    ///
    /// Each dependency is a single package, and the first-party project depends
    /// on its library crate. When we talk about "_the_ crate" of a dependency,
    /// we mean its library crate (as opposed to its binary crates or test
    /// crates). Below, we notate the package name as `PACKAGE_NAME` and the
    /// crate name as `CRATE_NAME` (the difference being that the crate name
    /// must be a valid Rust identifier and therefore has hyphens replaced with
    /// underscores).
    ///
    /// These dependency library crates can come in two configurations: either
    /// they _do_ have a build script or _do not_ have a build script.
    ///
    /// # Expected artifacts
    ///
    /// For each package, here are the artifacts we expect:
    ///
    /// - Fingerprints: these are stored in `$PROFILE/.fingerprint/`.
    ///   - For packages with build scripts, there are 3 fingerprint folders:
    ///     - `{PACKAGE_NAME}-{HASH1}`, which contains the fingerprint for the
    ///       library crate itself.
    ///       - `dep-lib-{CRATE_NAME}` (TODO: what's this used for? It's some
    ///         fixed 14-byte marker)
    ///       - `invoked.timestamp`, whose mtime marks the start of the
    ///         package's build
    ///       - `lib-{CRATE_NAME}`, containing a hash (TODO: for what? It's not
    ///         the PackageId or the package checksum)
    ///       - `lib-{CRATE_NAME}.json`, containing unit information like the
    ///         rustc version, features, dependencies, etc.
    ///     - `{PACKAGE_NAME}-{HASH2}`, which contains the fingerprint for
    ///       compiling the build script crate.
    ///       - `dep-build-script-build-script-build`
    ///       - `invoked.timestamp`
    ///       - `build-script-build-script-build`
    ///       - `build-script-build-script-build.json`
    ///     - `{PACKAGE_NAME}-{HASH3}`, which contains the fingerprint for
    ///       running the build script.
    ///       - `run-build-script-build-script-build`
    ///       - `run-build-script-build-script-build.json`, which among other
    ///         things contains the conditions for re-running the build script
    ///   - For packages without build scripts, only `{PACKAGE_NAME}-{HASH1}` is
    ///     present.
    /// - Build scripts: these are stored in `$PROFILE/build/`. For packages
    ///   without build scripts, there are no entries in this folder. For
    ///   packages with build scripts, there are 2 folders:
    ///   - `{PACKAGE_NAME}-{HASH2}`, which contains the compiled build script.
    ///     - `build-script-build`, an executable ELF.
    ///     - `build_script_build-{HASH2}`, the exact same ELF.
    ///     - `build_script_build-{HASH2}.d`, a `.d` file listing the input
    ///       files.
    ///   - `{PACKAGE_NAME}-{HASH3}`, which contains the outputs of running the
    ///     build script.
    ///     - `invoked.timestamp`, whose mtime marks when the build script
    ///       compilation was started (I think?)
    ///     - `out`, the build script output folder.
    ///     - `output`, the STDOUT of the build script, containing e.g. printed
    ///       `cargo:` directives.
    ///     - `root-output`, a file containing the absolute path to the output
    ///       folder of the build script.
    ///     - `stderr`, the STDERR of the build script.
    /// - Deps: these are stored in `$PROFILE/deps/`. See also:
    ///   https://rustc-dev-guide.rust-lang.org/backend/libs-and-metadata.html
    ///   - `{CRATE_NAME}-{HASH1}.d`, a `.d` file listing the inputs.
    ///   - `lib{CRATE_NAME}-{HASH1}.rlib`
    ///   - `lib{CRATE_NAME}-{HASH1}.rmeta`
    ///
    /// # Known weirdness
    ///
    /// ## Extra fingerprint and dep files
    ///
    /// When you examine your target folder after a build, you may see
    /// additional instances of fingerprint and dep files? Why is this? It's
    /// very likely because you have your IDE open, and rust-analyzer is running
    /// `cargo check` which is checking the units in a different feature/profile
    /// configuration than a regular `cargo build`, and therefore is creating
    /// units with different `PackageId`s. See if these extra files still appear
    /// after closing your IDE, killing all rust-analyzer processes, and running
    /// `cargo clean` and then `cargo build`.
    ///
    /// ## Missing artifacts
    ///
    /// Some dependencies listed in the lockfile don't generate artifacts
    /// because they aren't actually used. To check whether the artifact is
    /// actually used, see if you can find it in the output of `cargo tree`. See
    /// also: https://github.com/rust-lang/cargo/issues/10801
    ///
    /// FIXME: The right thing to do here is to read the dependency source from
    /// `cargo tree` or whatever underlying mechanism it's using, rather than
    /// from the lockfile or `cargo metadata`.
    #[instrument(name = "ProfileDir::enumerate_cache_artifacts")]
    pub async fn enumerate_cache_artifacts(
        &self,
        dependency: &Dependency,
    ) -> Result<Vec<Artifact>> {
        // TODO: We could make these lookups faster. For example, we could build
        // dedicated indexes for the fingerprint, deps, and build folders. We
        // could also use a data structure that natively supports fast "look up
        // by package name (i.e. file prefix)" rather than doing
        // iter-then-filter.
        let index = self.index.as_ref().ok_or_eyre("files not indexed")?;

        // TODO: Figure out how to directly reconstruct the expected package
        // hashes.
        let package_regex = Regex::new(&format!("^{}-[0-9a-f]{{16}}$", dependency.package_name))?;
        let dotd_regex = Regex::new(&format!("^{}-[0-9a-f]{{16}}\\.d$", dependency.lib_name))?;

        // Save fingerprints.
        let fingerprints = index
            .files
            .iter()
            .filter(|(path, _)| {
                path.components()
                    .tuple_windows()
                    .next()
                    .is_some_and(|(parent, child)| {
                        parent.as_str() == ".fingerprint" && package_regex.is_match(child.as_str())
                    })
            })
            .collect_vec();

        // Save build scripts.
        let build_scripts = index
            .files
            .iter()
            .filter(|(path, _)| {
                path.components()
                    .tuple_windows()
                    .next()
                    .is_some_and(|(parent, child)| {
                        parent.as_str() == "build" && package_regex.is_match(child.as_str())
                    })
            })
            .collect_vec();

        // We find dependencies by looking for a `.d` file in the `deps` folder
        // whose name starts with the name of the dependency and parsing it.
        // This will include the `.rlib` and `.rmeta`.
        let deps = index
            .files
            .iter()
            .filter(|(path, _)| {
                path.components()
                    .next()
                    .is_some_and(|part| part.as_str() == "deps")
            })
            .collect_vec();
        // We collect dependencies by finding the `.d` file and reading it.
        //
        // FIXME: If we can't reconstruct the expected hash, we can't actually
        // find the _one_ `.d` file because we can't tell which file is for
        // which package version in scenarios where our project has multiple
        // versions of a dependency.
        let dotds = deps.iter().filter(|(path, _)| {
            path.components()
                .nth(1)
                .is_some_and(|part| dotd_regex.is_match(part.as_str()))
        });

        let mut dependencies = Vec::new();
        for (dotd, _) in dotds {
            let outputs = Dotd::from_file(self, dotd)
                .await
                .context("parse .d file")?
                .outputs
                .into_iter()
                .collect::<HashSet<_>>();
            dependencies.extend(deps.iter().filter(|(path, _)| outputs.contains(*path)));
        }

        // Now that we have our three sources of files,
        // we actually treat them all the same way!
        fingerprints
            .into_iter()
            .chain(build_scripts)
            .chain(dependencies)
            .map(|(path, entry)| {
                Artifact::builder()
                    .target(path)
                    .hash(&entry.hash)
                    .metadata(entry.metadata.clone())
                    .build()
            })
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
