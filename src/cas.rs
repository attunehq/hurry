use cargo_metadata::camino::Utf8PathBuf;
use fslock::LockFile;

use color_eyre::{Result, eyre::Context};

use crate::fs;

/// The content-addressed storage area shared by all `hurry` cache instances.
#[derive(Debug)]
pub struct Cas {
    /// The root directory of the CAS.
    ///
    /// The CAS is a flat directory of files where each file is named for
    /// the hex encoded representation of the Blake3 hash of the file content.
    ///
    /// No path details are exposed from the CAS on purpose: instead, users must
    /// use the methods on this struct to interact with files inside the CAS.
    /// This is done so that the CAS instance can properly manage lockfiles
    /// (so that multiple instances of `hurry` correctly interact)
    /// and so that we can swap out the implementation for another one
    /// in the future if we desire (for example, a remote object store).
    ///
    /// Internally, the CAS holds a lockfile with the same name as each
    /// file it is accessing, suffixed with `.lock`, for the duration
    /// of the file's access.
    root: Utf8PathBuf,
}

impl Cas {
    /// Open an instance in the default location for the user.
    pub fn open_default() -> Result<Self> {
        let root = fs::user_global_cache_path()
            .context("find user cache path")?
            .join("cas");

        std::fs::create_dir_all(&root).context("ensure directory exists")?;
        Ok(Self { root })
    }
}

/// Holds the lock of the file being interacted with, preventing other instances
/// of `hurry` from mutating or reading the file at the same time.
///
/// Once this guard is dropped, the lock is released.
#[derive(Debug)]
pub struct CasLockGuard(LockFile);
