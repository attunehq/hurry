use std::{marker::PhantomData, path::Path};

use cargo_metadata::{
    Metadata,
    camino::{Utf8Path, Utf8PathBuf},
};
use color_eyre::{Result, eyre::Context};
use fslock::LockFile;
use tracing::{debug, instrument, trace};
use walkdir::WalkDir;

use crate::user_global_cache_path;

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
    ) -> Result<ProfileDir<Unlocked>> {
        ProfileDir::open(self, profile)
    }

    /// Open the `hurry` cache for the given key.
    pub fn open_cache(
        &self,
        key: impl AsRef<Utf8Path> + std::fmt::Debug,
    ) -> Result<Cache<Unlocked>> {
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
        let root = user_global_cache_path()
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
