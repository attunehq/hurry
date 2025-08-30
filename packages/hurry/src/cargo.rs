use std::{
    fmt::Debug as StdDebug, iter::once, marker::PhantomData, path::PathBuf, str::FromStr, sync::Arc,
};

use ahash::{HashMap, HashSet};
use bon::{Builder, bon};
use cargo_metadata::camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context, OptionExt, bail, eyre},
};
use derive_more::{Debug, Display};
use enum_assoc::Assoc;
use futures::{StreamExt, TryStreamExt, stream};
use itertools::Itertools;
use location_macros::workspace_dir;
use relative_path::{PathExt, RelativePath, RelativePathBuf};
use serde::Deserialize;
use strum::{EnumIter, IntoEnumIterator};
use subenum::subenum;
use tap::{Pipe, Tap, TapFallible};
use tokio::task::spawn_blocking;
use tracing::{debug, instrument, trace, warn};

use crate::{
    Locked, Unlocked,
    cache::{Artifact, Cache, Cas, Kind},
    fs::{self, Index, LockFile},
    hash::Blake3,
};

/// Invoke a cargo subcommand with the given arguments.
#[instrument(skip_all)]
pub async fn invoke(
    subcommand: impl AsRef<str>,
    args: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<()> {
    let subcommand = subcommand.as_ref();
    let args = args.into_iter().collect::<Vec<_>>();
    let args = args.iter().map(|a| a.as_ref()).collect::<Vec<_>>();

    let mut cmd = tokio::process::Command::new("cargo");
    cmd.args(once(subcommand).chain(args.iter().copied()));
    let status = cmd
        .spawn()
        .context("could not spawn cargo")?
        .wait()
        .await
        .context("could complete cargo execution")?;
    if status.success() {
        trace!(?subcommand, ?args, "invoke cargo");
        Ok(())
    } else {
        bail!("cargo exited with status: {status}");
    }
}

/// Cache the target directory to the cache.
///
/// When **restoring** `target/` in the future, we need to be able to restore
/// from scratch without an existing `target/` directory. This is for two
/// reasons: first, the project may actually be fresh, with no `target/`
/// at all. Second, the `target/` may be outdated.
/// This means that we can't rely on the functionality that `cargo`
/// would typically provide for us inside `target/`, such as `.fingerprint`
/// or `.d` files to find dependencies or hashes.
///
/// Of course, when **caching** `target/`, we can (and indeed must) assume
/// that the contents of `target/` are correct and trustworthy. But we must
/// copy all the data necessary to recreate the important parts of `target/`
/// in a future fresh start environment.
///
/// ## Third party crates
///
/// The backup process enumerates dependencies (third party crates)
/// in the project. For each discovered dependency, it:
/// - Finds the built `.rlib` and `.rmeta` files
/// - Finds tertiary files like `.fingerprint` etc
/// - Stores the files in the CAS in such a way that they can be found
///   using only data inside `Cargo.lock` in the future.
#[instrument(skip(progress))]
pub async fn cache_target_from_workspace(
    cas: impl Cas + StdDebug + Clone,
    cache: impl Cache + StdDebug + Clone,
    target: &ProfileDir<'_, Locked>,
    progress: impl Fn(&Blake3, &Dependency) + Clone,
) -> Result<()> {
    // The concurrency limits below are currently just vibes;
    // we want to avoid opening too many file handles at a time
    // because that can have a negative effect on performance
    // but we obviously want to have enough running that we saturate the disk.
    //
    // TODO: this currently assumes that the entire `target/` folder
    // doesn't have any _outdated_ data; this may not be correct.
    stream::iter(&target.workspace.dependencies)
        .filter_map(|(key, dependency)| {
            let target = target.clone();
            async move {
                debug!(?key, ?dependency, "restoring dependency");
                target
                    .enumerate_cache_artifacts(dependency)
                    .await
                    .map(|artifacts| (key, dependency, artifacts))
                    .tap_err(|err| {
                        warn!(
                            ?err,
                            "Failed to enumerate cache artifacts for dependency: {dependency}"
                        )
                    })
                    .ok()
                    .map(Ok)
            }
        })
        .try_for_each_concurrent(Some(10), |(key, dependency, artifacts)| {
            let (cas, target, cache, progress) =
                (cas.clone(), target.clone(), cache.clone(), progress.clone());
            async move {
                debug!(?key, ?dependency, ?artifacts, "caching artifacts");
                stream::iter(&artifacts)
                    .map(|artifact| Ok(artifact))
                    .try_for_each_concurrent(Some(100), |artifact| {
                        let (cas, target) = (cas.clone(), target.clone());
                        async move {
                            let dst = artifact.target.to_path(target.root());
                            cas.store_file(Kind::Cargo, &dst)
                                .await
                                .with_context(|| format!("backup output file: {dst:?}"))
                                .tap_ok(|key| {
                                    trace!(?key, ?dependency, ?artifact, "restored artifact")
                                })
                                .map(drop)
                        }
                    })
                    .await
                    .pipe(|_| {
                        let cache = cache.clone();
                        async move {
                            cache
                                .store(Kind::Cargo, key, &artifacts)
                                .await
                                .context("store cache record")
                                .tap_ok(|_| {
                                    debug!(?key, ?dependency, ?artifacts, "stored cache record")
                                })
                        }
                    })
                    .await
                    .map(|_| progress(key, dependency))
            }
        })
        .await
}

/// Restore the target directory from the cache.
#[instrument(skip(progress))]
pub async fn restore_target_from_cache(
    cas: impl Cas + StdDebug + Clone,
    cache: impl Cache + StdDebug + Clone,
    target: &ProfileDir<'_, Locked>,
    progress: impl Fn(&Blake3, &Dependency) + Clone,
) -> Result<()> {
    // When backing up a `target/` directory, we enumerate
    // the build units before backing up dependencies.
    // But when we restore, we don't have a target directory
    // (or don't trust it), so we can't do that here.
    // Instead, we just enumerate dependencies
    // and try to find some to restore.
    //
    // The concurrency limits below are currently just vibes;
    // we want to avoid opening too many file handles at a time
    // because that can have a negative effect on performance
    // but we obviously want to have enough running that we saturate the disk.
    debug!(dependencies = ?target.workspace.dependencies, "restoring dependencies");
    stream::iter(&target.workspace.dependencies)
        .filter_map(|(key, dependency)| {
            let cache = cache.clone();
            async move {
                debug!(?key, ?dependency, "restoring dependency");
                cache
                    .get(Kind::Cargo, key)
                    .await
                    .with_context(|| format!("retrieve cache record for dependency: {dependency}"))
                    .map(|lookup| lookup.map(|record| (key, dependency, record)))
                    .transpose()
            }
        })
        .try_for_each_concurrent(Some(10), |(key, dependency, record)| {
            let (cas, target, progress) = (cas.clone(), target.clone(), progress.clone());
            async move {
                debug!(?key, ?dependency, artifacts = ?record.artifacts, "restoring artifacts");
                stream::iter(record.artifacts)
                    .map(|artifact| Ok(artifact))
                    .try_for_each_concurrent(Some(100), |artifact| {
                        let (cas, target) = (cas.clone(), target.clone());
                        async move {
                            let dst = artifact.target.to_path(target.root());
                            cas.get_file(Kind::Cargo, &artifact.hash, &dst)
                                .await
                                .context("extract crate")
                                .tap_ok(|_| {
                                    trace!(?key, ?dependency, ?artifact, "restored artifact")
                                })
                        }
                    })
                    .await
                    .map(|_| progress(key, dependency))
            }
        })
        .await
}

/// The profile for the build.
///
/// Reference: https://doc.rust-lang.org/cargo/reference/profiles.html
//
// Note: We define `ProfileBuiltin` and only derive `EnumIter` on it
// because `EnumIter` over `Custom` does so over the default string value,
// which is an empty string; this is meaningless from an application logic
// perspective and can only ever result in bugs and wasted allocations.
#[subenum(ProfileBuiltin(derive(EnumIter)))]
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Default, Assoc)]
#[func(pub const fn as_str(&self) -> &str)]
pub enum Profile {
    /// The `debug` profile.
    #[default]
    #[assoc(as_str = "debug")]
    #[subenum(ProfileBuiltin)]
    Debug,

    /// The `bench` profile.
    #[assoc(as_str = "bench")]
    #[subenum(ProfileBuiltin)]
    Bench,

    /// The `test` profile.
    #[assoc(as_str = "test")]
    #[subenum(ProfileBuiltin)]
    Test,

    /// The `release` profile.
    #[assoc(as_str = "release")]
    #[subenum(ProfileBuiltin)]
    Release,

    /// A custom user-specified profile.
    #[assoc(as_str = _0.as_str())]
    Custom(String),
}

impl Profile {
    /// Get the profile specified by the user.
    ///
    /// If the user didn't specify, defaults to [`Profile::Debug`].
    #[instrument(name = "Profile::from_argv")]
    pub fn from_argv(argv: &[String]) -> Profile {
        if let Some(profile) = read_argv(argv, "--profile") {
            return Profile::from(profile);
        }

        // TODO: today this will never result in `bench` or `test` profiles;
        // how should we detect and handle these?
        argv.iter()
            .tuple_windows()
            .find_map(|(a, b)| {
                if a == "--release" || b == "--release" {
                    Some(Profile::Release)
                } else {
                    None
                }
            })
            .unwrap_or(Profile::Debug)
    }
}

impl std::fmt::Display for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<String> for Profile {
    fn from(value: String) -> Self {
        for variant in ProfileBuiltin::iter() {
            if variant.as_str() == value {
                return variant.into();
            }
        }
        Profile::Custom(value)
    }
}

impl From<&str> for Profile {
    fn from(value: &str) -> Self {
        for variant in ProfileBuiltin::iter() {
            if variant.as_str() == value {
                return variant.into();
            }
        }
        Profile::Custom(value.to_string())
    }
}

impl From<&String> for Profile {
    fn from(value: &String) -> Self {
        value.as_str().into()
    }
}

/// A Cargo workspace.
///
/// Note that in Cargo, "workspace" projects are slightly different than
/// standard projects; however for `hurry` they are not.
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
    /// Parse metadata about the workspace, indicated by the provided path.
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

    /// Parse metadata about the current workspace.
    /// "Current workspace" is defined by the process working directory.
    //
    // TODO: A few of these setup steps could be parallelized...
    // I'm not certain they're worth the thread spawn cost
    // but this can be mitigated by using the rayon thread pool.
    #[instrument(name = "Workspace::from_argv")]
    pub async fn from_argv(argv: &[String]) -> Result<Self> {
        let pwd = std::env::current_dir().context("get working directory")?;
        Self::from_argv_in_dir(pwd, argv).await
    }

    /// Ensure that the workspace `target/` directory
    /// is created and well formed with the provided
    /// profile directory created.
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

    /// Open the given named profile directory in the workspace.
    pub async fn open_profile(&self, profile: &Profile) -> Result<ProfileDir<'_, Unlocked>> {
        ProfileDir::open(self, profile).await
    }

    /// Open the given named profile directory in the workspace locked.
    pub async fn open_profile_locked(&self, profile: &Profile) -> Result<ProfileDir<'_, Locked>> {
        self.open_profile(profile)
            .await
            .context("open profile")?
            .pipe(|target| target.lock())
            .await
            .context("lock profile")
    }
}

/// A Cargo dependency.
///
/// This isn't the full set of information about a dependency, but it's enough
/// to identify it uniquely within a workspace for the purposes of caching.
///
/// Each piece of data in this struct is used to build the "cache key"
/// for the dependency; the intention is that each dependency is cached
/// independently and restored in other projects based on a matching
/// cache key derived from other instances of `hurry` reading the
/// `Cargo.lock` and other workspace/compiler/platform metadata.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display, Builder)]
#[display("{name}@{version}")]
pub struct Dependency {
    /// The name of the dependency.
    #[builder(into)]
    pub name: String,

    /// The version of the dependency.
    #[builder(into)]
    pub version: String,

    /// The checksum of the dependency.
    #[builder(into)]
    pub checksum: String,

    /// The target triple for which the dependency
    /// is being or has been built.
    ///
    /// Examples:
    /// ```not_rust
    /// aarch64-apple-darwin
    /// x86_64-unknown-linux-gnu
    /// ```
    #[builder(into)]
    pub target: String,
}

impl Dependency {
    /// Hash key for the dependency.
    pub fn key(&self) -> Blake3 {
        Self::key_for()
            .checksum(&self.checksum)
            .name(&self.name)
            .target(&self.target)
            .version(&self.version)
            .call()
    }
}

#[bon]
impl Dependency {
    /// Produce a hash key for all the fields of a dependency
    /// without having to actually make a dependency instance
    /// (which may involve cloning).
    #[builder]
    pub fn key_for(
        name: impl AsRef<[u8]>,
        version: impl AsRef<[u8]>,
        checksum: impl AsRef<[u8]>,
        target: impl AsRef<[u8]>,
    ) -> Blake3 {
        let name = name.as_ref();
        let version = version.as_ref();
        let checksum = checksum.as_ref();
        let target = target.as_ref();
        Blake3::from_fields([name, version, checksum, target])
    }
}

/// A profile directory inside a [`Workspace`].
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
    /// Instantiate a new instance for the provided profile in the workspace.
    /// If the directory doesn't already exist, it is created.
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

    /// Lock the directory.
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
    /// Enumerate cache artifacts in the target directory for the dependency.
    ///
    /// For now in this context, a "cache artifact" is _any file_ inside the
    /// profile directory that is inside the `.fingerprint`, `build`, or `deps`
    /// directories, where the immediate subdirectory of that parent is prefixed
    /// by the name of the dependency.
    ///
    /// TODO: the above is probably overly broad for a cache; evaluate
    /// what filtering mechanism to apply to reduce invalidations and rework.
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

    /// The root of the profile directory.
    pub fn root(&self) -> PathBuf {
        self.root.to_path(&self.workspace.target)
    }
}

/// Rust's compiler options for the current platform.
///
/// This isn't the _full_ set of options,
/// just what we need for caching.
//
// TODO: Support users cross compiling; probably need to parse argv?
// TODO: Determine minimum compiler version.
// TODO: Is there a better way to get this?
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize)]
pub struct RustcMetadata {
    /// The LLVM target triple.
    #[serde(rename = "llvm-target")]
    llvm_target: String,
}

impl RustcMetadata {
    /// Get platform metadata from the current compiler.
    #[instrument(name = "RustcMetadata::from_argv")]
    pub async fn from_argv(workspace_root: &Utf8Path, _argv: &[String]) -> Result<Self> {
        let mut cmd = tokio::process::Command::new("rustc");

        // Bypasses the check that disallows using unstable commands on stable.
        cmd.env("RUSTC_BOOTSTRAP", "1");
        cmd.args(["-Z", "unstable-options", "--print", "target-spec-json"]);
        cmd.current_dir(workspace_root);
        let output = cmd.output().await.context("run rustc")?;
        if !output.status.success() {
            return Err(eyre!("invoke rustc"))
                .with_section(|| {
                    String::from_utf8_lossy(&output.stdout)
                        .to_string()
                        .header("Stdout:")
                })
                .with_section(|| {
                    String::from_utf8_lossy(&output.stderr)
                        .to_string()
                        .header("Stderr:")
                });
        }

        serde_json::from_slice::<RustcMetadata>(&output.stdout)
            .context("parse rustc output")
            .with_section(|| {
                String::from_utf8_lossy(&output.stdout)
                    .to_string()
                    .header("Rustc Output:")
            })
    }
}

/// A parsed Cargo .d file.
///
/// `.d` files are structured a little like makefiles, where each output
/// is on its own line followed by a colon followed by the inputs.
#[derive(Debug)]
pub struct Dotd {
    /// Recorded output paths, relative to the profile root.
    pub outputs: Vec<RelativePathBuf>,
}

impl Dotd {
    /// Construct an instance by parsing the file.
    #[instrument(name = "Dotd::from_file")]
    pub async fn from_file(
        profile: &ProfileDir<'_, Locked>,
        target: &RelativePath,
    ) -> Result<Self> {
        const DEP_EXTS: [&str; 3] = [".d", ".rlib", ".rmeta"];
        let profile_root = profile.root();
        let outputs = fs::read_buffered_utf8(target.to_path(&profile_root))
            .await
            .with_context(|| format!("read .d file: {target:?}"))?
            .ok_or_eyre("file does not exist")?
            .lines()
            .filter_map(|line| {
                let (output, _) = line.split_once(':')?;
                if DEP_EXTS.iter().any(|ext| output.ends_with(ext)) {
                    trace!(?line, ?output, "read .d line");
                    Utf8PathBuf::from_str(output)
                        .tap_err(|error| trace!(?line, ?output, ?error, "not a valid path"))
                        .ok()
                } else {
                    trace!(?line, "skipped .d line");
                    None
                }
            })
            .map(|output| -> Result<RelativePathBuf> {
                output
                    .strip_prefix(&profile_root)
                    .with_context(|| format!("make {output:?} relative to {profile_root:?}"))
                    .and_then(|p| RelativePathBuf::from_path(p).context("read path as utf8"))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { outputs })
    }
}

/// Parse the value of an argument flag from `argv`.
///
/// Handles cases like:
/// - `--flag value`
/// - `--flag=value`
#[instrument]
pub fn read_argv<'a>(argv: &'a [String], flag: &str) -> Option<&'a str> {
    debug_assert!(flag.starts_with("--"), "flag {flag:?} must start with `--`");
    argv.iter().tuple_windows().find_map(|(a, b)| {
        let (a, b) = (a.trim(), b.trim());

        // Handle the `--flag value` case, where the flag and its value
        // are distinct entries in `argv`.
        if a == flag {
            return Some(b);
        }

        // Handle the `--flag=value` case, where the flag and its value
        // are the same entry in `argv`.
        //
        // Due to how tuple windows work, this case could be in either
        // `a` or `b`. If `b` is the _last_ element in `argv`,
        // it won't be iterated over again as a future `a`,
        // so we have to check both.
        //
        // Unfortunately this leads to rework as all but the last `b`
        // will be checked again as a future `a`, but since `argv`
        // is relatively small this shouldn't be an issue in practice.
        //
        // Just in case I've thrown an `instrument` call on the function,
        // but this is extremely unlikely to ever be an issue.
        for v in [a, b] {
            if let Some((a, b)) = v.split_once('=')
                && a == flag
            {
                return Some(b);
            }
        }

        None
    })
}
