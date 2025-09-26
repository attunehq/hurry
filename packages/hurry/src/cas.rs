use std::fmt::Debug;

use color_eyre::{Result, eyre::Context};
use derive_more::Display;
use tap::Pipe;
use tracing::{instrument, trace};

use crate::{
    fs,
    hash::Blake3,
    mk_rel_dir,
    path::{AbsDirPath, JoinWith as _, TryJoinWith as _},
};

/// The content-addressed storage area shared by all `hurry` cache instances.
///
/// The intention of the CAS is that it should be as "stupid" as possible:
/// - Globally stored.
/// - Purely concerned with storing/retrieving bytes, keyed by their hash.
/// - Does not contain implementation details for specific build systems.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display)]
#[display("{root}")]
pub struct FsCas {
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
    root: AbsDirPath,
}

impl FsCas {
    /// Open an instance in the default location for the user.
    #[instrument(name = "FsCas::open_default")]
    pub async fn open_default() -> Result<Self> {
        fs::user_global_cache_path()
            .await
            .context("find user cache path")?
            .join(mk_rel_dir!("cas"))
            .pipe_ref(Self::open_dir)
            .await
    }

    /// Open an instance in the provided directory.
    /// If the directory does not already exist, it is created.
    #[instrument(name = "FsCas::open_dir")]
    pub async fn open_dir(root: &AbsDirPath) -> Result<Self> {
        let root = root.clone();
        fs::create_dir_all(&root).await?;
        trace!(?root, "open cas");
        Ok(Self { root })
    }

    /// Report whether there are items in the CAS.
    #[instrument(name = "FsCas::is_empty")]
    pub async fn is_empty(&self) -> Result<bool> {
        fs::is_dir_empty(&self.root).await
    }

    /// Store the entry in the CAS.
    #[instrument(name = "FsCas::store", skip(content))]
    pub async fn store(&self, content: &[u8]) -> Result<Blake3> {
        let key = Blake3::from_buffer(content);
        let dst = self.root.try_join_file(key.as_str())?;
        fs::write(&dst, content).await?;
        trace!(?key, bytes = ?content.len(), "stored content");
        Ok(key)
    }

    /// Get the entry out of the CAS.
    #[instrument(name = "FsCas::get")]
    pub async fn get(&self, key: &Blake3) -> Result<Option<Vec<u8>>> {
        let src = self.root.try_join_file(key.as_str())?;
        fs::read_buffered(&src).await
    }

    /// Get the entry out of the CAS.
    /// Errors if the entry is not available.
    #[instrument(name = "FsCas::get")]
    pub async fn must_get(&self, key: &Blake3) -> Result<Vec<u8>> {
        let src = self.root.try_join_file(key.as_str())?;
        fs::must_read_buffered(&src).await
    }
}
