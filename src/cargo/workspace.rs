use std::{
    collections::{HashMap, HashSet},
    marker::PhantomData,
    str::FromStr,
};

use bon::{Builder, bon};
use cargo_metadata::camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context, OptionExt, eyre},
};
use derive_more::{Debug, Display};
use fslock::LockFile;
use itertools::Itertools;
use serde::Deserialize;
use tap::{Pipe, Tap, TapFallible};
use tracing::{debug, instrument, trace};

use crate::{
    cargo::{CacheRecord, CacheRecordArtifact, Profile, read_argv},
    fs::{self, Index},
    hash::Blake3,
};

/// The associated type's state is unlocked.
/// Used for the typestate pattern.
#[derive(Debug, Clone, Copy, Default)]
pub struct Unlocked;

/// The associated type's state is locked.
/// Used for the typestate pattern.
#[derive(Debug, Clone, Copy, Default)]
pub struct Locked;

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
    /// Parse metadata about the current workspace.
    ///
    /// "Current workspace" is discovered by parsing the arguments passed
    /// to `hurry` and using `--manifest_path` if it is available;
    /// if not then it uses the current working directory.
    //
    // TODO: A few of these setup steps could be parallelized...
    // I'm not certain they're worth the thread spawn cost
    // but this can be mitigated by using the rayon thread pool.
    #[instrument(name = "Workspace::from_argv")]
    pub fn from_argv(argv: &[String]) -> Result<Self> {
        // TODO: Maybe we should just replicate this logic and perform it
        // statically using filesystem operations instead of shelling out? This
        // costs something on the order of 200ms, which is not _terrible_ but
        // feels much slower than if we just did our own filesystem reads.
        let mut cmd = cargo_metadata::MetadataCommand::new();
        if let Some(p) = read_argv(argv, "--manifest-path") {
            cmd.manifest_path(p);
        }
        let metadata = cmd.exec().context("could not read cargo metadata")?;
        debug!(?metadata, "cargo metadata");

        // TODO: This currently blows up if we have no lockfile.
        let lockfile = cargo_lock::Lockfile::load(metadata.workspace_root.join("Cargo.lock"))
            .context("load cargo lockfile")?;
        debug!(?lockfile, "cargo lockfile");

        let rustc_meta = RustcMetadata::from_argv(&metadata.workspace_root, argv)
            .context("read rustc metadata")?;
        debug!(?rustc_meta, "rustc metadata");

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

    /// Ensure that the workspace `target/` directory
    /// is created and well formed with the provided
    /// profile directory created.
    #[instrument(name = "Workspace::init_target")]
    pub fn init_target(&self, profile: &Profile) -> Result<()> {
        const CACHEDIR_TAG_NAME: &str = "CACHEDIR.TAG";
        const CACHEDIR_TAG_CONTENT: &[u8] = include_bytes!("../../static/cargo/CACHEDIR.TAG");

        // TODO: do we need to create `.rustc_info.json` to get cargo
        // to recognize the target folder as valid when restoring caches?
        fs::create_dir_all(self.target.join(profile.as_str()))
            .context("create target directory")?;
        fs::write(self.target.join(CACHEDIR_TAG_NAME), CACHEDIR_TAG_CONTENT)
            .context("write CACHEDIR.TAG")
    }

    /// Open the given named profile directory in the workspace.
    pub fn open_profile(&self, profile: &Profile) -> Result<ProfileDir<'_, Unlocked>> {
        ProfileDir::open(self, profile)
    }

    /// Open the `hurry` cache in the default location for the user.
    pub fn open_cache(&self) -> Result<Cache<'_, Unlocked>> {
        Cache::open_default(self)
    }

    /// Find a dependency with the specified name and version
    /// in the workspace, if it exists.
    #[instrument(name = "Workspace::find_dependency")]
    fn find_dependency(
        &self,
        name: impl AsRef<str> + std::fmt::Debug,
        version: impl AsRef<str> + std::fmt::Debug,
    ) -> Option<&Dependency> {
        // TODO: we may want to index this instead of iterating each time,
        // or at minimum cache it (ref: https://docs.rs/cached/latest/cached/)
        let (name, version) = (name.as_ref(), version.as_ref());
        self.dependencies
            .values()
            .find(|d| d.name == name && d.version == version)
            .tap(|dependency| trace!(?dependency, "search result"))
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
#[derive(Debug)]
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
    lock: LockFile,

    /// The workspace in which this build profile is located.
    pub workspace: &'ws Workspace,

    /// The index of files inside the profile directory.
    /// Paths in this index are relative to [`ProfileDir::root`].
    ///
    /// This index is built when the profile directory is locked.
    /// Currently there's no explicit unlock mechanism for profiles since
    /// they're just dropped, but if we ever add one that's where we'd clear
    /// this and set it to `None`.
    index: Option<Index>,

    /// The root of the directory.
    ///
    /// For example, if the workspace is at `/home/me/projects/foo`,
    /// and the value of `profile` is `release`,
    /// the value of `root` would be `/home/me/projects/foo/target/release`.
    ///
    /// Users should not rely on this though:
    /// use the actual value in this field.
    ///
    /// Note: this is intentionally not `pub` because we only want to give
    /// callers access to the directory when the cache is locked;
    /// reference the `root` method in the locked implementation block.
    /// The intention here is to minimize the chance of callers mutating or
    /// referencing the contents of the cache while it is locked.
    root: Utf8PathBuf,

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
    pub fn open(workspace: &'ws Workspace, profile: &Profile) -> Result<Self> {
        workspace
            .init_target(profile)
            .context("init workspace target")?;

        let root = workspace.root.join("target").join(profile.as_str());
        let lock = root.join(".cargo-lock");
        let lock = LockFile::open(lock.as_std_path()).context("open lockfile")?;

        Ok(Self {
            state: PhantomData,
            index: None,
            profile: profile.clone(),
            lock,
            root,
            workspace,
        })
    }

    /// Lock the directory.
    #[instrument(name = "ProfileDir::lock")]
    pub fn lock(mut self) -> Result<ProfileDir<'ws, Locked>> {
        self.lock.lock().context("lock profile")?;
        let index = Index::recursive(&self.root)
            .map(Some)
            .context("index target folder")?;
        Ok(ProfileDir {
            state: PhantomData,
            profile: self.profile,
            lock: self.lock,
            root: self.root,
            workspace: self.workspace,
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
    pub fn enumerate_cache_artifacts(
        &self,
        dependency: &Dependency,
    ) -> Result<Vec<CacheRecordArtifact>> {
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
            path.components().skip(1).next().is_some_and(|part| {
                part.as_str().ends_with(".d") && part.as_str().starts_with(&dependency.name)
            })
        });
        let dependencies = if let Some((path, _)) = dotd {
            let outputs = Dotd::from_file(self, path)
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
            .map(|(path, entry)| {
                CacheRecordArtifact::builder()
                    .hash(&entry.hash)
                    .target(path)
                    .build()
            })
            .inspect(|artifact| trace!(?artifact, "enumerated artifact"))
            .collect::<Vec<_>>()
            .pipe(Ok)
    }

    /// The root of the profile directory.
    pub fn root(&self) -> &Utf8Path {
        &self.root
    }
}

/// The `hurry` cache corresponding to a given [`Workspace`].
#[derive(Debug, Display)]
#[display("{root}")]
pub struct Cache<'ws, State> {
    #[debug(skip)]
    state: PhantomData<State>,

    /// Locks the workspace cache.
    ///
    /// The intention of this lock is to prevent multiple `hurry` instances
    /// from mutating the state of the cache directory at the same time,
    /// or from mutating it at the same time as another instance
    /// is reading it.
    #[debug(skip)]
    lock: LockFile,

    /// The root directory of the workspace cache.
    ///
    /// Note: this is intentionally not `pub` because we only want to give
    /// callers access to the directory when the cache is locked;
    /// reference the `root` method in the locked implementation block.
    ///
    /// The intention here is to minimize the chance of callers mutating or
    /// referencing the contents of the cache while it is locked.
    root: Utf8PathBuf,

    /// The workspace in the context of which this cache is referenced.
    pub workspace: &'ws Workspace,
}

/// Implementation for all valid lifetimes and lock states.
impl<'ws, L> Cache<'ws, L> {
    /// The filename of the lockfile.
    const LOCKFILE_NAME: &'static str = ".hurry-lock";
}

/// Implementation for all lifetimes and the unlocked state only.
impl<'ws> Cache<'ws, Unlocked> {
    /// Open the cache in the default location for the user.
    #[instrument(name = "Cache::open_default")]
    pub fn open_default(workspace: &'ws Workspace) -> Result<Self> {
        let root = fs::user_global_cache_path()
            .context("find user cache path")?
            .join("cargo")
            .join("ws");

        fs::create_dir_all(&root)?;
        let lock = root.join(Self::LOCKFILE_NAME);
        let lock = LockFile::open(lock.as_std_path()).context("open lockfile")?;

        Ok(Self {
            state: PhantomData,
            root,
            workspace,
            lock,
        })
    }

    /// Lock the cache.
    #[instrument(name = "Cache::lock")]
    pub fn lock(mut self) -> Result<Cache<'ws, Locked>> {
        self.lock.lock().context("lock workspace cache")?;
        Ok(Cache {
            state: PhantomData,
            root: self.root,
            lock: self.lock,
            workspace: self.workspace,
        })
    }
}

/// Implementation for all lifetimes and the locked state only.
impl<'ws> Cache<'ws, Locked> {
    /// Unlock the cache.
    #[instrument(name = "Cache::unlock")]
    pub fn unlock(mut self) -> Result<Cache<'ws, Unlocked>> {
        self.lock.unlock().context("unlock workspace cache")?;
        Ok(Cache {
            state: PhantomData,
            root: self.root,
            lock: self.lock,
            workspace: self.workspace,
        })
    }

    /// Store the provided record in the cache.
    #[instrument(name = "Cache::store")]
    pub fn store(&self, record: &CacheRecord) -> Result<()> {
        let name = self.root.join(record.dependency_key.as_str());
        let content = serde_json::to_string_pretty(record).context("encode record")?;
        fs::write(name, content).context("store cache record")
    }

    /// Retrieve the record from the cache for the given dependency key.
    #[instrument(name = "Cache::retrieve")]
    pub fn retrieve(
        &self,
        key: impl AsRef<Blake3> + std::fmt::Debug,
    ) -> Result<Option<CacheRecord>> {
        let name = self.root.join(key.as_ref().as_str());
        match fs::read_buffered_utf8(name) {
            Ok(content) => Ok(Some(
                serde_json::from_str(&content).context("decode record")?,
            )),
            Err(err)
                if err
                    .downcast_ref::<std::io::Error>()
                    .map(|e| e.kind() == std::io::ErrorKind::NotFound)
                    .unwrap_or(false) =>
            {
                Ok(None)
            }
            Err(err) => Err(err).context("read cache record"),
        }
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
    pub fn from_argv(workspace_root: &Utf8Path, _argv: &[String]) -> Result<Self> {
        let mut cmd = std::process::Command::new("rustc");

        // Bypasses the check that disallows using unstable commands on stable.
        cmd.env("RUSTC_BOOTSTRAP", "1");
        cmd.args(["-Z", "unstable-options", "--print", "target-spec-json"]);
        cmd.current_dir(workspace_root);
        let output = cmd.output().context("run rustc")?;
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
pub struct Dotd<'ws> {
    #[debug(skip)]
    #[allow(dead_code)]
    profile: &'ws ProfileDir<'ws, Locked>,

    /// Recorded output paths, relative to the profile root.
    pub outputs: Vec<Utf8PathBuf>,
}

impl<'ws> Dotd<'ws> {
    /// Construct an instance by parsing the file.
    #[instrument(name = "Dotd::from_file")]
    pub fn from_file(profile: &'ws ProfileDir<'ws, Locked>, path: &Utf8Path) -> Result<Self> {
        const DEP_EXTS: [&str; 3] = [".d", ".rlib", ".rmeta"];
        let outputs = fs::read_buffered_utf8(profile.root.join(path))
            .with_context(|| format!("read .d file: {path:?}"))?
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
            .map(|output| -> Result<Utf8PathBuf> {
                output
                    .strip_prefix(&profile.root)
                    .with_context(|| format!("make {output:?} relative to {:?}", profile.root))
                    .map(|p| p.to_path_buf())
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { profile, outputs })
    }
}
