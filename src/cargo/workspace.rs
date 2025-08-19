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
use itertools::Itertools;
use rayon::iter::{ParallelBridge, ParallelIterator};
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument, trace};
use walkdir::WalkDir;

use crate::{fs, hash::Blake3};

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
    /// The root directory of the workspace.
    pub root: Utf8PathBuf,

    /// The root of the target directory in the workspace.
    pub target: Utf8PathBuf,

    /// The user's Cargo home directory.
    ///
    /// This is almost definitely not inside the current workspace,
    /// but we record it because we need it to make third party crate file paths
    /// relative so they can be portable across machines.
    pub cargo_home: Utf8PathBuf,
}

impl Workspace {
    /// Parse metadata about the current workspace.
    ///
    /// "Current workspace" is discovered by parsing the arguments passed
    /// to `hurry` and using `--manifest_path` if it is available;
    /// if not then it uses the current working directory.
    #[instrument]
    pub fn current() -> Result<Self> {
        // TODO: Should we even support this?
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
        let cargo_home = std::env::var("CARGO_HOME")
            .map(Utf8PathBuf::from)
            .context("get cargo home")?;
        Ok(Self {
            root: metadata.workspace_root,
            target: metadata.target_directory,
            cargo_home,
        })
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
        let root = workspace.root.join("target").join(&profile);

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
    pub fn enumerate_buildunits(&self) -> Result<Vec<BuildUnit>> {
        // TODO: parallelize this.
        // The critical part here is that we retain the order of input files.
        WalkDir::new(self.root.as_std_path()).into_iter().try_fold(
            Vec::new(),
            |mut acc, entry| -> Result<Vec<BuildUnit>> {
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
pub struct BuildUnit<'ws>(BuildUnitOutput<'ws>, Vec<BuildUnitInput<'ws>>);

impl<'ws> BuildUnit<'ws> {
    /// Parse a file to create the instance in the provided profile.
    #[instrument]
    pub fn parse(
        profile: &'ws ProfileDir<'ws, Locked>,
        content: impl AsRef<str> + std::fmt::Debug,
    ) -> Result<Vec<BuildUnit<'ws>>> {
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
            .map(|(output, inputs)| Self::new_from_strs(profile, output, &inputs))
            .collect()
    }

    /// Create an instance from the provided output and input files,
    /// where the file paths are strings.
    ///
    /// This isn't public because it's really only meant to be called from
    /// `parse` as a convenience/code organization function.
    fn new_from_strs(
        profile: &'ws ProfileDir<'ws, Locked>,
        output: &str,
        inputs: &[&str],
    ) -> Result<Self> {
        let output = Utf8PathBuf::from_str(output)
            .with_context(|| format!("create output path from: {output}"))?;
        let inputs = inputs
            .into_iter()
            .map(|input| {
                Utf8PathBuf::from_str(input)
                    .with_context(|| format!("create input path from: {input}"))
            })
            .collect::<Result<Vec<_>>>()?;
        Self::new(profile, output, inputs)
    }

    /// Create an instance from the provided output and input files.
    #[instrument]
    pub fn new(
        profile: &'ws ProfileDir<'ws, Locked>,
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

        Ok(Self(output, inputs))
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
pub struct BuildUnitOutput<'ws> {
    /// The profile to which this output belongs.
    profile: &'ws ProfileDir<'ws, Locked>,

    /// The path on disk for the output file.
    ///
    /// This is a relative path to [`ProfileDir::root`]
    /// in the associated `profile`.
    path: Utf8PathBuf,

    /// The Blake3 hash of the file's content on disk.
    hash: Blake3,
}

impl<'ws> BuildUnitOutput<'ws> {
    /// Create the output from the provided path on disk.
    pub fn read(profile: &'ws ProfileDir<'ws, Locked>, path: &Utf8Path) -> Result<Self> {
        let hash = Blake3::from_file(path).context("hash output file")?;
        Ok(Self {
            profile,
            path: path.to_owned(),
            hash,
        })
    }
}

/// An input for a build unit.
#[derive(Debug)]
pub struct BuildUnitInput<'ws> {
    /// The workspace to which this input belongs.
    workspace: &'ws Workspace,

    /// The path on disk for the input file.
    ///
    /// This path is relative to [`Workspace::cargo_home`]
    /// in the associated `workspace`.
    path: Utf8PathBuf,

    /// The Blake3 hash of the file's content on disk.
    hash: Blake3,
}

impl<'ws> BuildUnitInput<'ws> {
    /// Create the input from the provided path on disk.
    pub fn read(workspace: &'ws Workspace, path: &Utf8Path) -> Result<Self> {
        let hash = Blake3::from_file(path).with_context(|| format!("hash input file {path:?}"))?;

        // For now we report any case where this isn't valid as an error.
        // I suspect that it'll be possible to have input paths that are
        // relative to the current directory but right now the overall design
        // assumes that they are relative to the cargo home directory,
        // so if we find this to be the case we'll need to revisit the design.
        // And the only way to know if this happens is to have errors surfaced.
        let path = path.strip_prefix(&workspace.cargo_home).with_context(|| {
            format!(
                "make input {path:?} relative to cargo home {:?}",
                workspace.cargo_home
            )
        })?;

        Ok(Self {
            workspace,
            path: path.to_owned(),
            hash,
        })
    }
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
