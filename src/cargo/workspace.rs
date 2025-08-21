use std::{
    collections::{HashMap, HashSet},
    iter::repeat,
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
use lockfile::Lockfile;
use rayon::iter::ParallelBridge;
use serde::{Deserialize, Serialize};
use tap::{Pipe, Tap, TryConv};
use tracing::{debug, instrument, trace};
use walkdir::{DirEntry, WalkDir};

use crate::{
    cargo::{CacheRecord, CacheRecordArtifact, Profile, read_argv},
    fs::{self, HashedFile},
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

    /// The root of the target directory in the workspace.
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
    #[instrument]
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
        trace!(?metadata, "cargo metadata");

        // TODO: This currently blows up if we have no lockfile.
        let lockfile = cargo_lock::Lockfile::load(metadata.workspace_root.join("Cargo.lock"))
            .context("load cargo lockfile")?;
        trace!(?lockfile, "cargo lockfile");

        let rustc_meta = RustcMetadata::from_argv(&metadata.workspace_root, argv)
            .context("read rustc metadata")?;
        trace!(?rustc_meta, "rustc metadata");

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
    pub fn init_target(&self, profile: &Profile) -> Result<()> {
        const CACHEDIR_TAG_NAME: &str = "CACHEDIR.TAG";
        const CACHEDIR_TAG_CONTENT: &[u8] = include_bytes!("../../static/cargo/CACHEDIR.TAG");

        // TODO: do we need to create `.rustc_info.json` to get cargo
        // to recognize the target folder as valid when restoring caches?
        std::fs::create_dir_all(self.target.join(profile.as_str()))
            .context("create target directory")?;
        std::fs::write(self.target.join(CACHEDIR_TAG_NAME), CACHEDIR_TAG_CONTENT)
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
    #[instrument]
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
    /// without having to actually make a proper dependency instance
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
    #[instrument]
    pub fn open(workspace: &'ws Workspace, profile: &Profile) -> Result<Self> {
        let root = workspace.root.join("target").join(profile.as_str());
        workspace
            .init_target(profile)
            .context("init workspace target")?;

        let lock = root.join(".cargo-lock");
        let lock = LockFile::open(lock.as_std_path()).context("open lockfile")?;

        Ok(Self {
            state: PhantomData,
            profile: profile.clone(),
            lock,
            root,
            workspace,
        })
    }

    /// Lock the directory.
    pub fn lock(mut self) -> Result<ProfileDir<'ws, Locked>> {
        self.lock.lock().context("lock profile")?;
        Ok(ProfileDir {
            state: PhantomData,
            profile: self.profile,
            lock: self.lock,
            root: self.root,
            workspace: self.workspace,
        })
    }
}

impl<'ws> ProfileDir<'ws, Locked> {
    /// Unlock the directory.
    pub fn unlock(mut self) -> Result<ProfileDir<'ws, Unlocked>> {
        self.lock.unlock().context("unlock profile")?;
        Ok(ProfileDir {
            state: PhantomData,
            profile: self.profile,
            lock: self.lock,
            root: self.root,
            workspace: self.workspace,
        })
    }

    /// Enumerate cache artifacts in the target directory for the dependency.
    ///
    /// For now in this context, a "cache artifact" is _any file_ inside the
    /// profile directory that is inside the `.fingerprint`, `build`, or `deps`
    /// directories, where the immediate subdirectory of that parent is prefixed
    /// by the name of the dependency.
    ///
    /// TODO: the above is probably overly broad for a cache; evaluate
    /// what filtering mechanism to apply to reduce invalidations and rework.
    /// TODO: This requires us to walk the target directory for every dep;
    /// should we index instead? I haven't pre-emptively done this
    /// because we may find a way to pare down what we need to walk
    /// (or even avoid walking and compute keys directly)
    /// by solving the todo above this one.
    #[instrument]
    pub fn enumerate_cache_artifacts(
        &self,
        dependency: &Dependency,
    ) -> Result<Vec<CacheRecordArtifact>> {
        let root = &self.root;

        // Fingerprint artifacts are straightforward:
        // if they're inside the `.fingerprint` directory,
        // and the subdirectory of `.fingerprint` starts with the name of
        // the dependency, then they should be backed up.
        //
        // Builds are the same as fingerprints, just with a different root:
        // instead of `.fingerprint`, they're looking for `build`.
        let standard = WalkDir::new(root).into_iter().filter_entry(|entry| {
            let path = entry.path();
            if path == root {
                return true;
            }
            let Ok(subdir) = path.strip_prefix(root) else {
                return false;
            };

            for sdname in [".fingerprint", "build"] {
                if subdir.starts_with(sdname) {
                    let subsubdir = subdir.components().skip(1).next();
                    return subsubdir.is_none_or(|ssd| {
                        ssd.as_os_str()
                            .to_string_lossy()
                            .starts_with(&dependency.name)
                    });
                }
            }

            false
        });

        // Dependencies are totally different from the two above.
        // This directory is flat; inside we're looking for one primary file:
        // a `.d` file whose name starts with the name of the dependency.
        //
        // This file then lists other files (namely, `.rlib` and `.rmeta`)
        // that this dependency built; these are _often_ (but not always)
        // named with a different prefix (often, but not always, "lib").
        //
        // Along the way we also grab any other random file in here that is
        // prefixed with the same name as the `.d` file; from observation
        // so far this has been `.rcgu.o` files which appear to be compiled
        // codegen.
        //
        // Since we have to read this directory potentially several times in
        // effectively random order we just read the whole list into memory.
        // TODO: cache this?
        let dependencies_root = root.join("deps");
        let all_dependencies = std::fs::read_dir(&dependencies_root)
            .context("read dependencies")?
            .into_iter()
            .map(|entry| -> Result<String> {
                let entry = entry.context("walk files")?;
                Ok(entry.file_name().to_string_lossy().to_string())
            })
            .collect::<Result<Vec<_>>>()
            .with_context(|| format!("enumerate contents of {dependencies_root:?}"))?;

        // `.d` files are structured a little like makefiles, where each output
        // is on its own line followed by a colon followed by the inputs.
        //
        // Currently, we only care about the outputs, and only the outputs
        // that end with `.d`, `.rlib`, or `.rmeta`.
        //
        // Also, not all crates create `.d` files, or even entries in
        // the `deps` folder at all!
        let dotd = all_dependencies
            .iter()
            .find(|name| name.starts_with(&dependency.name) && name.ends_with(".d"));
        let dependencies = if let Some(dotd) = dotd {
            let dotd_path = dependencies_root.join(dotd);
            let dependency_outputs = fs::read_buffered_utf8(&dotd_path)
                .with_context(|| format!("read .d file: {dotd_path:?}"))?
                .lines()
                .filter_map(|line| {
                    let (output, _) = line.split_once(':')?;
                    const DEP_EXTS: [&str; 3] = [".d", ".rlib", ".rmeta"];
                    if DEP_EXTS.iter().any(|ext| output.ends_with(ext)) {
                        trace!(?dotd, ?line, ?output, "read .d line");
                        Utf8PathBuf::from_str(output).ok()
                    } else {
                        trace!(?dotd, ?line, "skipped .d line");
                        None
                    }
                })
                .filter_map(|path| {
                    path.file_name()
                        .map(|n| n.to_string())
                        .tap(|stripped| trace!(?path, ?stripped, "stripped .d path"))
                })
                .collect::<HashSet<_>>();
            all_dependencies
                .into_iter()
                .filter_map(|name| {
                    if name.starts_with(&dependency.name) || dependency_outputs.contains(&name) {
                        Some(dependencies_root.join(name))
                    } else {
                        None
                    }
                })
                .collect_vec()
        } else {
            Vec::new()
        };

        // Now that we have our three sources of files,
        // we actually treat them all the same way!
        standard
            .map(|entry| -> Result<Option<Utf8PathBuf>> {
                let entry = entry.context("walk files")?;
                if entry.file_type().is_file() {
                    entry.into_path().try_into().context("parse path").map(Some)
                } else {
                    Ok(None)
                }
            })
            .filter_map(Result::transpose)
            .chain(dependencies.into_iter().map(Ok))
            // TODO: parallelize with rayon
            .map(|entry| -> Result<CacheRecordArtifact> {
                let entry = entry?;
                CacheRecordArtifact::builder()
                    .hash(
                        Blake3::from_file(&entry)
                            .with_context(|| format!("hash file: {entry:?}"))?,
                    )
                    .target(
                        entry
                            .strip_prefix(root)
                            .with_context(|| format!("make {entry:?} relative to {root:?}"))?,
                    )
                    .build()
                    .tap(|artifact| trace!(?artifact, "enumerated artifact"))
                    .pipe(Ok)
            })
            .collect()
    }

    /// Enumerate build units in the target.
    pub fn enumerate_buildunits(&self) -> Result<Vec<BuildUnit>> {
        // TODO: parallelize this.
        // The critical part here is that we retain the order of input files.
        WalkDir::new(self.root.as_std_path()).into_iter().try_fold(
            Vec::new(),
            |mut acc, entry| -> Result<Vec<BuildUnit>> {
                let entry = entry.context("walk file")?;
                let entry = entry.path();

                // Only `*.d` files are valid build units.
                if !entry.extension().is_some_and(|ext| ext == "d") {
                    trace!(?entry, "skip file: not a build unit");
                    return Ok(acc);
                }

                let content = fs::read_buffered_utf8(entry)
                    .with_context(|| format!("read build unit: {entry:?}"))?;
                let unit = BuildUnit::parse(self, content)
                    .with_context(|| format!("parse build unit: {entry:?}"))?;
                acc.extend(unit);
                Ok(acc)
            },
        )
    }

    /// The root of the profile directory.
    pub fn root(&self) -> &Utf8Path {
        &self.root
    }
}

/// A build unit inside a workspace.
///
/// This is a `hurry`-specific term for a `.d` file inside the `target`
/// directory; these files are a psuedo-makefile syntax that lists:
/// - Output files (compiled artifacts)
/// - Their input files (source code)
///
/// ## Example
///
/// For example here you can see that `libahash-d548a2253ff6e8a0.rlib`
/// depends on several files, e.g. `ahash-0.8.12/src/lib.rs`;
/// then later in the file you can see that `ahash-0.8.12/src/lib.rs`
/// doesn't depend on anything else.
///
/// ```not_rust
/// /Users/jess/projects/attune/target/release/deps/ahash-d548a2253ff6e8a0.d: /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/lib.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/convert.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/fallback_hash.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/operations.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/random_state.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/specialize.rs
///
/// /Users/jess/projects/attune/target/release/deps/libahash-d548a2253ff6e8a0.rlib: /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/lib.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/convert.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/fallback_hash.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/operations.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/random_state.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/specialize.rs
///
/// /Users/jess/projects/attune/target/release/deps/libahash-d548a2253ff6e8a0.rmeta: /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/lib.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/convert.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/fallback_hash.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/operations.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/random_state.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/specialize.rs
///
/// /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/lib.rs:
/// /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/convert.rs:
/// /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/fallback_hash.rs:
/// /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/operations.rs:
/// /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/random_state.rs:
/// /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ahash-0.8.12/src/specialize.rs:
/// ```
#[derive(Debug, Serialize, Deserialize)]
pub struct BuildUnit {
    /// The output of this build unit.
    pub output: BuildUnitOutput,

    /// The inputs of this build unit.
    pub inputs: Vec<BuildUnitInput>,

    /// The key of the dependency which built this unit.
    ///
    /// Currently, only build units that came from third party dependencies
    /// are supported for caching.
    pub dependency_key: Option<Blake3>,
}

impl BuildUnit {
    /// Parse a file to create the instance in the provided profile.
    #[instrument]
    pub fn parse(
        profile: &ProfileDir<Locked>,
        content: impl AsRef<str> + std::fmt::Debug,
    ) -> Result<Vec<BuildUnit>> {
        let content = content.as_ref();

        // TODO: Parallelize this.
        // The most important thing when it come to parallelization
        // is ensuring that we emit each `Vec<BuildUnitInput>` in the same order
        // as what is actually written in the file.
        content
            .lines()
            .into_iter()
            .inspect(|line| trace!(?line, "parse build unit .d file"))
            .filter_map(|line| {
                let (output, inputs) = line.split_once(':')?;
                let (output, inputs) = (output.trim(), inputs.trim());
                let inputs = inputs.split_whitespace().collect::<Vec<_>>();
                Some((output, inputs))
            })
            // Build units with empty inputs seem to be practice source files,
            // which are not copied into the `deps/` directory anyway
            // and therefore do not need to be considered.
            .filter(|(output, inputs)| {
                if inputs.is_empty() {
                    trace!(?output, "skipped build unit: empty inputs");
                    false
                } else {
                    true
                }
            })
            .filter_map(
                |(output, inputs)| match Self::new_from_strs(profile, output, &inputs) {
                    Ok(parsed) => {
                        trace!(?output, ?inputs, "parsed build unit");
                        Some(parsed)
                    }
                    Err(error) => {
                        trace!(?output, ?inputs, ?error, "failed to parse build unit");
                        None
                    }
                },
            )
            .collect::<Vec<_>>()
            .pipe(Ok)
    }

    /// Create an instance from the provided output and input files,
    /// where the file paths are strings.
    ///
    /// This isn't public because it's really only meant to be called from
    /// `parse` as a convenience/code organization function.
    fn new_from_strs(profile: &ProfileDir<Locked>, output: &str, inputs: &[&str]) -> Result<Self> {
        let output = fs::into_path(output).context("create output path")?;
        let inputs = inputs
            .into_iter()
            .map(|input| fs::into_path(input).context("create input path"))
            .collect::<Result<Vec<_>>>()?;
        Self::new(profile, output, inputs)
    }

    /// Create an instance from the provided output and input files.
    #[instrument]
    pub fn new(
        profile: &ProfileDir<Locked>,
        output: impl AsRef<Utf8Path> + std::fmt::Debug,
        inputs: impl AsRef<[Utf8PathBuf]> + std::fmt::Debug,
    ) -> Result<Self> {
        let output = output.as_ref();
        let inputs = inputs.as_ref();
        let output = BuildUnitOutput::read(profile, output)
            .with_context(|| format!("read output file: {output}"))?;
        let inputs = inputs
            .into_iter()
            .map(|input| {
                BuildUnitInput::read(profile.workspace, input)
                    .with_context(|| format!("read input file: {input}"))
            })
            .collect::<Result<Vec<_>>>()?;

        // TODO: we don't currently prioritize certain inputs,
        // or do anything to handle (or indeed check) when inputs
        // don't agree on a source. I don't actually know if this ever
        // happens, but we should figure it out.
        let dependency_key = inputs.iter().find_map(|input| {
            // Input paths are in the form:
            // `.cargo/registry/src/index.crates.io-$HASH/$NAME-$VERSION/src/lib.rs`.
            // We are after the `$NAME` and `$VERSION` here.
            //
            // TODO: this breaks on anything other than crates
            // in the public registry. Currently this is OK because we
            // similarly filter dependencies when listing them in the workspace,
            // but we should fix this eventually. Maybe we could read
            // the actual `Cargo.toml` at the directory that is the shared
            // common root of all inputs for this information?
            input
                .path_rel()
                .components()
                .tuple_windows()
                .find_map(|(parent, child)| {
                    if parent.as_str().contains("index.crates.io") {
                        let (name, version) = child.as_str().split_once('-')?;
                        profile
                            .workspace
                            .find_dependency(name, version)
                            .map(Dependency::key)
                    } else {
                        None
                    }
                })
        });

        Ok(Self {
            output,
            inputs,
            dependency_key,
        })
    }
}

/// An output file for the build unit.
#[derive(Debug, Serialize, Deserialize)]
pub struct BuildUnitOutput(HashedFile);

impl BuildUnitOutput {
    /// Create the output from the provided path on disk.
    pub fn read(profile: &ProfileDir<Locked>, path: &Utf8Path) -> Result<Self> {
        let file = HashedFile::read(path).context("hash file")?;

        // Need to make the file path relative to the profile directory
        // so that this can be cross platform and cross project.
        //
        // For now we report any case where this isn't valid as an error.
        // This may not be the right decision forever, but for now we need this
        // because the current system makes this assumption.
        let path = file
            .path
            .strip_prefix(profile.root())
            .with_context(|| format!("make {path:?} relative to {:?}", profile.root()))?;

        HashedFile::builder()
            .path(path)
            .hash(file.hash)
            .build()
            .pipe(Self)
            .pipe(Ok)
    }

    /// The hash of the file content.
    pub fn hash(&self) -> &Blake3 {
        &self.0.hash
    }

    /// The path relative to [`ProfileDir::root()`] for the file.
    pub fn path_rel(&self) -> &Utf8Path {
        &self.0.path
    }

    /// Compute the full path using the given profile.
    pub fn path(&self, profile: &ProfileDir<Locked>) -> Utf8PathBuf {
        profile.root().join(&self.0.path)
    }
}

/// An input for a build unit.
#[derive(Debug, Serialize, Deserialize)]
pub struct BuildUnitInput(HashedFile);

impl BuildUnitInput {
    /// Create the input from the provided path on disk.
    #[instrument]
    pub fn read(_workspace: &Workspace, path: &Utf8Path) -> Result<Self> {
        let file = HashedFile::read(path).context("hash file")?;

        // Need to make the file path relative to the cargo home directory
        // so that this can be cross platform and cross machine.
        //
        // For now we report any case where this isn't valid as an error.
        // This may not be the right decision forever, but for now we need this
        // because the current system makes this assumption.
        // let path = file
        //     .path
        //     .strip_prefix(&workspace.cargo_home)
        //     .with_context(|| format!("make {path:?} relative to {:?}", workspace.cargo_home))?
        //     .to_owned();

        HashedFile::builder()
            .path(file.path)
            .hash(file.hash)
            .build()
            .pipe(Self)
            .pipe(Ok)
    }

    /// The hash of the file content.
    pub fn hash(&self) -> &Blake3 {
        &self.0.hash
    }

    /// The path relative to [`Workspace::cargo_home`] for the file.
    pub fn path_rel(&self) -> &Utf8Path {
        &self.0.path
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
    #[instrument]
    pub fn open_default(workspace: &'ws Workspace) -> Result<Self> {
        let root = fs::user_global_cache_path()
            .context("find user cache path")?
            .join("cargo")
            .join("ws");

        std::fs::create_dir_all(&root).context("ensure directory exists")?;
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
    pub fn unlock(mut self) -> Result<Cache<'ws, Unlocked>> {
        self.lock.unlock().context("unlock workspace cache")?;
        Ok(Cache {
            state: PhantomData,
            root: self.root,
            lock: self.lock,
            workspace: self.workspace,
        })
    }

    /// The root path of the cache.
    pub fn root(&self) -> &Utf8Path {
        &self.root
    }

    /// Store the provided record in the cache.
    #[instrument]
    pub fn store(&self, record: &CacheRecord) -> Result<()> {
        let name = self.root.join(record.dependency_key.as_str());
        let content = serde_json::to_string_pretty(record).context("encode record")?;
        std::fs::write(name, content).context("store cache record")
    }

    /// Retrieve the record from the cache for the given dependency key.
    #[instrument]
    pub fn retrieve(
        &self,
        key: impl AsRef<Blake3> + std::fmt::Debug,
    ) -> Result<Option<CacheRecord>> {
        let name = self.root.join(key.as_ref().as_str());
        match std::fs::read_to_string(name) {
            Ok(content) => Ok(Some(
                serde_json::from_str(&content).context("decode record")?,
            )),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err).context("read cache record"),
        }
    }

    /// Check whether the cache is empty (other than the lockfile).
    pub fn is_empty(&self) -> Result<bool> {
        for entry in std::fs::read_dir(&self.root).context("read cache directory")? {
            let entry = entry.context("read entry")?;
            if !entry.path().ends_with(Self::LOCKFILE_NAME) {
                return Ok(false);
            }
        }
        Ok(true)
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
    #[instrument]
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
