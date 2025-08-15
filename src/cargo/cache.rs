use std::{
    fs::{self, File},
    io::BufReader,
    marker::PhantomData,
    path::{Path, PathBuf},
};

use color_eyre::{
    Result,
    eyre::{Context, OptionExt},
};
use fslock::LockFile;
use homedir::my_home;
use tap::Pipe;
use tracing::instrument;

/// The workspace cache is unlocked.
#[derive(Debug, Clone, Copy, Default)]
pub struct Unlocked;

/// The workspace cache is locked.
#[derive(Debug, Clone, Copy, Default)]
pub struct Locked;

/// Represents a workspace cache.
///
/// ## Invariant
///
/// An unlocked `WorkspaceCache` instance MUST be safe to use for
/// all instances of `hurry`.
///
/// Given this, you MUST lock the cache before using it.
#[derive(Debug)]
pub struct WorkspaceCache<State> {
    /// Prevents instantiating the struct directly
    /// outside of this module.
    private: PhantomData<State>,

    /// Locks the workspace cache.
    lock: LockFile,

    /// The root directory of the workspace cache.
    ///
    /// Validated to exist when `WorkspaceCache` is constructed.
    pub root: PathBuf,

    /// The `target` directory within the workspace cache.
    ///
    /// If this exists, it is a known-valid target directory
    /// for the state of the workspace hash.
    pub target: PathBuf,

    /// The hash of the workspace cache.
    pub hash: Vec<u8>,

    /// Content-addressable shared storage directory.
    ///
    /// This is a shared directory for all builds,
    /// but is stored in the cache just so that it doesn't have to be
    /// recomputed every time we want to reference this path.
    ///
    /// Validated to exist when `WorkspaceCache` is constructed.
    pub cas: PathBuf,
}

impl WorkspaceCache<Unlocked> {
    /// Construct a new cache instance for the given workspace path.
    #[instrument]
    pub fn new(workspace: impl AsRef<Path> + std::fmt::Debug) -> Result<Self> {
        let workspace = workspace.as_ref();

        // Ensure user cache directory exists.
        let cache_root = user_cache_path().context("get user cache path")?;
        fs::create_dir_all(&cache_root).context("ensure user hurry cache exists")?;

        // Ensure CAS directory exists.
        let cas = cache_root.join("cas");
        fs::create_dir_all(&cas).context("ensure CAS exists")?;

        // Ensure workspace cache directory exists.
        // We intentionally don't create the `target` directory if it doesn't exist;
        // it needs to only exist if it's known to be valid.
        let lockfile = workspace.join("Cargo.lock");
        let lockfile_hash = hash_file_content(&lockfile).context("hash workspace lockfile")?;
        let workspace_cache_root = cache_root.join("ws").join(hex::encode(&lockfile_hash));
        let workspace_cache_target = workspace_cache_root.join("target");
        fs::create_dir_all(&workspace_cache_root).context("ensure workspace cache exists")?;

        // Prevents concurrent access to the workspace cache
        // from other `hurry` instances.
        let lock = workspace_cache_root.join("lock");
        let lock = LockFile::open(&lock).context("open workspace lockfile")?;

        Ok(Self {
            private: PhantomData,
            root: workspace_cache_root,
            target: workspace_cache_target,
            hash: lockfile_hash,
            cas,
            lock,
        })
    }

    /// Lock the workspace cache.
    ///
    /// Make sure to call `unlock` when you're done,
    /// unless you're going to drop the `WorkspaceCache` instance entirely-
    /// the lock will be released in that case.
    ///
    /// ## Invariant
    ///
    /// An unlocked `WorkspaceCache` instance MUST be safe to use for
    /// all instances of `hurry`.
    //
    // TODO: make an intermediate type that we can just drop to unlock.
    pub fn lock(mut self) -> Result<WorkspaceCache<Locked>> {
        self.lock.lock().context("lock workspace cache")?;
        Ok(WorkspaceCache {
            private: PhantomData,
            root: self.root,
            target: self.target,
            cas: self.cas,
            lock: self.lock,
            hash: self.hash,
        })
    }
}

impl WorkspaceCache<Locked> {
    /// Unlock the workspace cache.
    ///
    /// ## Invariant
    ///
    /// An unlocked `WorkspaceCache` instance MUST be safe to use for
    /// all instances of `hurry`.
    pub fn unlock(mut self) -> Result<WorkspaceCache<Unlocked>> {
        self.lock.unlock().context("unlock workspace cache")?;
        Ok(WorkspaceCache {
            private: PhantomData,
            root: self.root,
            target: self.target,
            cas: self.cas,
            lock: self.lock,
            hash: self.hash,
        })
    }
}

fn hash_file_content(path: &PathBuf) -> Result<Vec<u8>> {
    let mut hasher = blake3::Hasher::new();

    let file = File::open(path).with_context(|| format!("open {path:?}"))?;
    let mut reader = BufReader::new(file);

    std::io::copy(&mut reader, &mut hasher)?;
    Ok(hasher.finalize().as_bytes().to_vec())
}

/// Determine the canonical cache path for the current user, if possible.
///
/// This can fail if the user has no home directory,
/// or if the home directory cannot be accessed.
fn user_cache_path() -> Result<PathBuf> {
    my_home()
        .context("get user home directory")?
        .ok_or_eyre("user has no home directory")?
        .join(".cache")
        .join("hurry")
        .join("v1")
        .join("cargo")
        .pipe(Ok)
}
