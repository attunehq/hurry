use cargo_metadata::camino::{Utf8Path, Utf8PathBuf};
use derive_more::Display;

use color_eyre::{Result, eyre::Context};
use tracing::{instrument, trace};

use crate::fs;

/// The content-addressed storage area shared by all `hurry` cache instances.
#[derive(Debug, Display)]
#[display("{root}")]
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
    #[instrument(name = "Cas::open_default")]
    pub fn open_default() -> Result<Self> {
        let root = fs::user_global_cache_path()
            .context("find user cache path")?
            .join("cas");

        fs::create_dir_all(&root)?;
        trace!(?root, "open cas");
        Ok(Self { root })
    }

    /// Copy the file at the provided path into the CAS using the provided key.
    #[instrument(name = "Cas::copy_from")]
    pub fn copy_from(&self, src: &Utf8Path, key: impl AsRef<str> + std::fmt::Debug) -> Result<()> {
        let dst = self.root.join(key.as_ref());
        fs::copy_file(src, &dst)
    }

    /// Extract the file with the referenced key to the destination path.
    /// If the destination's parent directory doesn't exist, it is created.
    #[instrument(name = "Cas::extract_to")]
    pub fn extract_to(&self, key: impl AsRef<str> + std::fmt::Debug, dst: &Utf8Path) -> Result<()> {
        let src = self.root.join(key.as_ref());
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy_file(src, dst)
    }
}
