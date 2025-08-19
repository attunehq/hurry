use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    marker::PhantomData,
    path::{Path, PathBuf},
    str::FromStr,
};

use cargo_metadata::{
    Metadata,
    camino::{Utf8Path, Utf8PathBuf},
};
use color_eyre::{Result, eyre::Context};
use derive_more::Display;
use fslock::LockFile;
use rayon::iter::{ParallelBridge, ParallelIterator};
use tracing::{debug, instrument, trace};
use walkdir::WalkDir;

use crate::fs;

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
#[derive(Debug)]
pub struct Workspace {
    metadata: Metadata,
}

impl Workspace {
    /// Parse metadata about the current workspace.
    ///
    /// "Current workspace" is discovered by parsing the arguments passed
    /// to `hurry` and using `--manifest_path` if it is available;
    /// if not then it uses the current working directory.
    #[instrument]
    pub fn current() -> Result<Self> {
        // TODO: Should these be parsed higher up and passed in?
        let mut args = std::env::args().skip_while(|val| !val.starts_with("--manifest-path"));

        // TODO: Maybe we should just replicate this logic and perform it
        // statically using filesystem operations instead of shelling out? This
        // costs something on the order of 200ms, which is not _terrible_ but
        // feels much slower than if we just did our own filesystem reads.
        let mut cmd = cargo_metadata::MetadataCommand::new();
        match args.next() {
            Some(ref p) if p == "--manifest-path" => {
                cmd.manifest_path(args.next().expect("--manifest-path requires a value"));
            }
            Some(p) => {
                cmd.manifest_path(p.trim_start_matches("--manifest-path="));
            }
            None => {}
        }

        let metadata = cmd.exec().context("could not read cargo metadata")?;
        trace!(?metadata, "cargo metadata");
        Ok(Self { metadata })
    }

    /// The working directory for the workspace on disk.
    pub fn dir(&self) -> &Utf8Path {
        &self.metadata.workspace_root
    }

    /// The target directory.
    pub fn target(&self) -> &Path {
        self.metadata.target_directory.as_std_path()
    }

    /// Open the given named profile directory in the workspace.
    pub fn open_profile(
        &self,
        profile: impl Into<String> + std::fmt::Debug,
    ) -> Result<ProfileDir<'_, Unlocked>> {
        ProfileDir::open(self, profile)
    }

    /// Open the `hurry` cache for the given key.
    pub fn open_cache(
        &self,
        key: impl AsRef<Utf8Path> + std::fmt::Debug,
    ) -> Result<Cache<'_, Unlocked>> {
        Cache::open_default(self, key)
    }

    // Note that this iterator may contain the same module (i.e. file) multiple
    // times if it is included from multiple target root directories (e.g. if a
    // module is contained in both a `library` and a `bin` target).
    #[instrument(skip(self))]
    pub fn source_files(&self) -> impl Iterator<Item = Result<walkdir::DirEntry, walkdir::Error>> {
        let packages = self.metadata.workspace_packages();
        trace!(?packages, "workspace packages");
        packages.into_iter().flat_map(|package| {
            trace!(?package, "getting source files of package");
            // TODO: The technically correct way to calculate the source files
            // of a target is to shell out to `rustc --emit=dep-info=-` with the
            // correct `rustc` flags (e.g. the `--extern` flags, which are
            // required to import macros defined in dependency crates which
            // might be used to add more source files to the target) and the
            // module root.
            //
            // Unfortunately, this is quite annoying:
            // - We need to get the correct flags for `rustc`. I'm not totally
            //   sure how to do this - I think we should be able to by parsing
            //   the output messages from `cargo build`? Or maybe we should be
            //   able to reconstruct them from `cargo metadata`? Or maybe we can
            //   use `cargo rustc`?
            // - Running `rustc` takes quite a long time. Maybe we can improve
            //   this by using some background daemon / file change notification
            //   trickery? Maybe we can run things in parallel?
            //
            // Instead, we approximate the source files in a module by taking
            // all the files in the folder of the crate root source file. This
            // is also the approximation that Cargo uses to determine "relevant"
            // files.
            //
            // TODO: We have not yet implemented Cargo's approximation logic
            // that handles things like `.gitignore`, `package.include`,
            // `package.exclude`, etc.
            //
            // See also:
            // - `dep-info` files:
            //   https://doc.rust-lang.org/cargo/reference/build-cache.html#dep-info-files
            // - `cargo build` output messages:
            //   https://doc.rust-lang.org/cargo/reference/external-tools.html#json-messages
            // - Cargo's source file discovery logic:
            //   https://docs.rs/cargo/latest/cargo/sources/path/struct.PathSource.html#method.list_files
            package.targets.iter().flat_map(|target| {
                let target_root = target.src_path.clone();
                let target_root_folder = target_root
                    .parent()
                    .expect("module root should be a file in a folder");
                debug!(
                    ?target_root,
                    ?target_root_folder,
                    "adding target root to walk"
                );
                WalkDir::new(target_root_folder).into_iter()
            })
        })
    }
}

/// A profile directory inside a [`Workspace`].
#[derive(Debug)]
pub struct ProfileDir<'ws, State> {
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
    lock: LockFile,

    /// The workspace in which this build profile is located.
    pub workspace: &'ws Workspace,

    /// The root of the directory.
    ///
    /// For example, if the workspace is at `/home/me/projects/foo`,
    /// and the value of `profile` is `release`,
    /// the value of `root` would be `/home/me/projects/foo/target/release`.
    ///
    /// Users should not rely on this though: use the actual value in this field.
    pub root: Utf8PathBuf,

    /// The profile to which this directory refers.
    ///
    /// By default, profiles are `release`, `debug`, `test`, and `bench`
    /// although users can also define custom profiles, which is why
    /// this value is an opaque string:
    /// https://doc.rust-lang.org/cargo/reference/profiles.html#custom-profiles
    pub profile: String,
}

impl<'ws> ProfileDir<'ws, Unlocked> {
    /// Instantiate a new instance for the provided profile in the workspace.
    #[instrument]
    pub fn open(
        workspace: &'ws Workspace,
        profile: impl Into<String> + std::fmt::Debug,
    ) -> Result<Self> {
        let profile = profile.into();
        let root = workspace.dir().join("target").join(&profile);

        let lock = root.join(".cargo-lock");
        let lock = LockFile::open(lock.as_std_path()).context("open lockfile")?;

        Ok(Self {
            state: PhantomData,
            profile,
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

    /// Enumerate build units in the target.
    pub fn enumerate_buildunits(&self) -> Result<Vec<BuildUnit<'_>>> {
        // TODO: parallelize this.
        // The critical part here is that we retain the order of input files.
        WalkDir::new(self.root.as_std_path()).into_iter().try_fold(
            Vec::new(),
            |mut acc, entry| -> Result<Vec<BuildUnit<'_>>> {
                let entry = entry.context("walk file")?;

                // Only `*.d` files are valid build units.
                if !entry.path().ends_with(".d") {
                    return Ok(acc);
                }

                let content = fs::read_buffered_utf8(entry.path()).context("read file")?;
                let unit = BuildUnit::parse(self, content).context("parse build unit")?;
                acc.extend(unit);
                Ok(acc)
            },
        )
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
#[derive(Debug)]
pub struct BuildUnit(BuildUnitOutput, Vec<BuildUnitInput>);

impl BuildUnit {
    /// Parse a file to create the instance in the provided profile.
    #[instrument]
    pub fn parse(
        profile: &ProfileDir<'_, Locked>,
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
            .filter_map(|line| {
                let (output, inputs) = line.split_once(':')?;
                let (output, inputs) = (output.trim(), inputs.trim());
                let inputs = inputs.split_whitespace().collect::<Vec<_>>();
                Some((output, inputs))
            })
            // Build units with empty inputs seem to be practice source files,
            // which are not copied into the `deps/` directory anyway
            // and therefore do not need to be considered.
            .filter(|(_, inputs)| !inputs.is_empty())
            .map(|(output, inputs)| {
                let output = {
                    let path = Utf8PathBuf::from_str(output).context("create output path")?;
                    let hash = fs::hash_file_content(&path).context("hash output file")?;
                    BuildUnitOutput { path, hash }
                };
                let inputs = inputs
                    .into_iter()
                    .map(|input| {
                        let path = Utf8PathBuf::from_str(input)
                            .with_context(|| format!("create input path from: {input}"))?;
                        let hash = fs::hash_file_content(&path)
                            .with_context(|| format!("hash input file: {path}"))?;
                        Ok(BuildUnitInput(hash))
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(BuildUnit(output, inputs))
            })
            .collect()
    }

    /// The output of the build unit.
    pub fn output(&self) -> &BuildUnitOutput {
        &self.0
    }

    /// The inputs of the build unit.
    pub fn inputs(&self) -> &[BuildUnitInput] {
        &self.1
    }
}

/// An output file for the build unit.
#[derive(Debug)]
pub struct BuildUnitOutput {
    /// The path on disk for the artifact.
    /// This is a relative path to [`ProfileDir::root`].
    path: Utf8PathBuf,

    /// The Blake3 hash of the file's content on disk.
    hash: Vec<u8>,
}

/// The Blake3 hash of the input file.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct BuildUnitInput {
    /// The path on disk for the artifact.
    ///
    /// TODO: We need a stable way to refer to this path, but these can
    /// exist in the user's cargo cache path so we need to gather
    /// this information.
    path: Utf8PathBuf,

    /// The Blake3 hash of the file's content on disk.
    hash: Vec<u8>,
}

/// The `hurry` cache corresponding to a given [`Workspace`].
#[derive(Debug)]
pub struct Cache<'ws, State> {
    state: PhantomData<State>,

    /// Locks the workspace cache.
    ///
    /// The intention of this lock is to prevent multiple `hurry` instances
    /// from mutating the state of the cache directory at the same time,
    /// or from mutating it at the same time as another instance
    /// is reading it.
    lock: LockFile,

    /// The root directory of the workspace cache.
    root: Utf8PathBuf,

    /// The workspace in the context of which this cache is referenced.
    pub workspace: &'ws Workspace,
}

impl<'ws> Cache<'ws, Unlocked> {
    /// Open the cache for the given workspace for the given cache key
    /// in the default location for the user.
    #[instrument]
    pub fn open_default(
        workspace: &'ws Workspace,
        key: impl AsRef<Utf8Path> + std::fmt::Debug,
    ) -> Result<Self> {
        let root = fs::user_global_cache_path()
            .context("find user cache path")?
            .join("cargo")
            .join("ws")
            .join(key);

        std::fs::create_dir_all(&root).context("ensure directory exists")?;
        let lock = root.join(".hurry-lock");
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
    ///
    /// Users can only get this value if the cache is locked;
    /// the intention here is to reduce the likelihood of mutating the content
    /// of the cache without having the cache locked.
    pub fn root(&self) -> &Utf8Path {
        &self.root
    }

    /// Check whether the cache is empty (other than the lockfile).
    pub fn is_empty(&self) -> Result<bool> {
        for entry in std::fs::read_dir(&self.root).context("read cache directory")? {
            let entry = entry.context("read entry")?;
            if !entry.path().ends_with(".hurry-lock") {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
