//! Local file system implementation of cache and CAS traits.

use std::{fmt::Debug as StdDebug, marker::PhantomData, path::Path};

use cargo_metadata::camino::Utf8PathBuf;
use color_eyre::{Result, Section, SectionExt, eyre::Context};
use derive_more::{Debug, Display};
use fslock::LockFile;
use itertools::Itertools;
use tap::Pipe;
use tracing::{instrument, trace};

use crate::{
    Locked, Unlocked,
    cache::{Artifact, Kind, Record},
    fs,
    hash::Blake3,
};

/// The local file system implementation of a cache.
#[derive(Debug, Display)]
#[display("{root}")]
pub struct FsCache<State> {
    #[debug(skip)]
    state: PhantomData<State>,

    /// The root directory of the workspace cache.
    ///
    /// Note: this is intentionally not `pub` because we only want to give
    /// callers access to the directory when the cache is locked;
    /// reference the `root` method in the locked implementation block.
    ///
    /// The intention here is to minimize the chance of callers mutating or
    /// referencing the contents of the cache while it is locked.
    root: Utf8PathBuf,

    /// Locks the workspace cache.
    ///
    /// The intention of this lock is to prevent multiple `hurry` instances
    /// from mutating the state of the cache directory at the same time,
    /// or from mutating it at the same time as another instance
    /// is reading it.
    #[debug(skip)]
    lock: LockFile,

    /// The cache root of the workspace in the context
    /// in which this cache exists.
    /// For most kinds of projects, this is expected to be
    /// the root of the project.
    pub workspace: Utf8PathBuf,
}

/// Implementation for all valid lock states.
impl<L> FsCache<L> {
    /// The filename of the lockfile.
    const LOCKFILE_NAME: &'static str = ".hurry-lock";
}

/// Implementation for all lifetimes and the unlocked state only.
impl FsCache<Unlocked> {
    /// Open the cache in the default location for the user.
    #[instrument(name = "FsCache::open_default")]
    pub fn open_default(workspace: impl Into<Utf8PathBuf> + StdDebug) -> Result<Self> {
        let root = fs::user_global_cache_path()
            .context("find user cache path")?
            .join("ws");

        fs::create_dir_all(&root)?;
        let lock = root.join(Self::LOCKFILE_NAME);
        let lock = LockFile::open(lock.as_std_path()).context("open lockfile")?;

        Ok(Self {
            state: PhantomData,
            workspace: workspace.into(),
            root,
            lock,
        })
    }

    /// Lock the cache.
    #[instrument(name = "FsCache::lock")]
    pub fn lock(mut self) -> Result<FsCache<Locked>> {
        self.lock.lock().context("lock cache")?;
        Ok(FsCache {
            state: PhantomData,
            root: self.root,
            lock: self.lock,
            workspace: self.workspace,
        })
    }
}

impl FsCache<Locked> {
    /// Unlock the cache.
    #[instrument(name = "FsCache::unlock")]
    pub fn unlock(mut self) -> Result<FsCache<Unlocked>> {
        self.lock.unlock().context("unlock cache")?;
        Ok(FsCache {
            state: PhantomData,
            root: self.root,
            lock: self.lock,
            workspace: self.workspace,
        })
    }
}

impl super::Cache for &FsCache<Locked> {
    #[instrument(name = "FsCache::store")]
    async fn store(
        &self,
        kind: Kind,
        key: impl AsRef<Blake3> + StdDebug + Send,
        artifacts: impl IntoIterator<Item = impl Into<Artifact>> + StdDebug + Send,
    ) -> Result<()> {
        let key = key.as_ref();
        let artifacts = artifacts.into_iter().map(Into::into).collect_vec();
        let name = self.root.join(kind.as_str()).join(key.as_str());
        let content = Record::builder()
            .key(key)
            .artifacts(artifacts)
            .kind(kind)
            .build()
            .pipe_ref(serde_json::to_string_pretty)
            .context("encode record")?;
        fs::write(name, content).context("store record")
    }

    #[instrument(name = "FsCache::get")]
    async fn get(&self, kind: Kind, key: impl AsRef<Blake3> + StdDebug) -> Result<Option<Record>> {
        let key = key.as_ref();
        let name = self.root.join(kind.as_str()).join(key.as_str());
        Ok(match fs::read_buffered_utf8(name).context("read file")? {
            Some(content) => serde_json::from_str(&content)
                .context("decode record")
                .with_section(|| content.header("Content:"))?,
            None => None,
        })
    }
}

/// The content-addressed storage area shared by all `hurry` cache instances.
#[derive(Debug, Display)]
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
    root: Utf8PathBuf,
}

impl FsCas {
    /// Open an instance in the default location for the user.
    #[instrument(name = "FsCas::open_default")]
    pub fn open_default() -> Result<Self> {
        let root = fs::user_global_cache_path()
            .context("find user cache path")?
            .join("cas");

        fs::create_dir_all(&root)?;
        trace!(?root, "open cas");
        Ok(Self { root })
    }
}

impl super::Cas for &FsCas {
    #[instrument(name = "FsCas::store")]
    async fn store(
        &self,
        kind: Kind,
        content: impl AsRef<[u8]> + StdDebug + Send,
    ) -> Result<Blake3> {
        let content = content.as_ref();
        let key = Blake3::from_buffer(content);
        let dst = self.root.join(kind.as_str()).join(key.as_str());
        fs::write(dst, content)?;
        Ok(key)
    }

    #[instrument(name = "FsCas::store_file")]
    async fn store_file(
        &self,
        kind: Kind,
        src: impl AsRef<Path> + StdDebug + Send,
    ) -> Result<Blake3> {
        let src = src.as_ref();
        let key = Blake3::from_file(src).context("hash file")?;
        let dst = self.root.join(kind.as_str()).join(key.as_str());
        fs::copy_file(src, dst)?;
        Ok(key)
    }

    #[instrument(name = "FsCas::get")]
    async fn get(
        &self,
        kind: Kind,
        key: impl AsRef<Blake3> + StdDebug + Send,
    ) -> Result<Option<Vec<u8>>> {
        let src = self.root.join(kind.as_str()).join(key.as_ref().as_str());
        fs::read_buffered(src)
    }

    #[instrument(name = "FsCas::get_file")]
    async fn get_file(
        &self,
        kind: Kind,
        key: impl AsRef<Blake3> + StdDebug + Send,
        destination: impl AsRef<Path> + StdDebug + Send,
    ) -> Result<()> {
        let src = self.root.join(kind.as_str()).join(key.as_ref().as_str());
        fs::copy_file(src, destination.as_ref())
    }
}
