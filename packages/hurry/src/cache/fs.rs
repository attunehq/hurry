//! Local file system implementation of cache and CAS traits.

use std::{fmt::Debug as StdDebug, marker::PhantomData};

use color_eyre::{Result, Section, SectionExt, eyre::Context};
use derive_more::{Debug, Display};
use itertools::Itertools;
use tap::Pipe;
use tracing::{instrument, trace};

use crate::{
    Locked, Unlocked,
    cache::{Artifact, Kind, Record},
    fs::{self, LockFile},
    hash::Blake3,
    mk_rel_dir, mk_rel_file,
    path::{Abs, Dir, File, JoinWith, Rel, TypedPath},
};

/// The local file system implementation of a cache.
#[derive(Clone, Debug, Display)]
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
    root: TypedPath<Abs, Dir>,

    /// Locks the workspace cache.
    ///
    /// The intention of this lock is to prevent multiple `hurry` instances
    /// from mutating the state of the cache directory at the same time,
    /// or from mutating it at the same time as another instance
    /// is reading it.
    #[debug(skip)]
    lock: LockFile<State>,
}

/// Implementation for all valid lock states.
impl<L> FsCache<L> {
    /// The name of the lockfile.
    fn lockfile() -> TypedPath<Rel, File> {
        mk_rel_file!(".hurry-lock")
    }
}

/// Implementation for all lifetimes and the unlocked state only.
impl FsCache<Unlocked> {
    /// Open the cache in the default location for the user.
    #[instrument(name = "FsCache::open_default")]
    pub async fn open_default() -> Result<Self> {
        fs::user_global_cache_path()
            .await
            .context("find user cache path")?
            .join(mk_rel_dir!("ws"))
            .pipe(Self::open_dir)
            .await
    }

    /// Open the cache in the provided directory.
    #[instrument(name = "FsCache::open_dir")]
    pub async fn open_dir(root: impl Into<TypedPath<Abs, Dir>> + StdDebug) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)
            .await
            .context("create cache directory")?;

        let lock = root.join(Self::lockfile());
        let lock = LockFile::open(lock).await.context("open lockfile")?;
        Ok(Self {
            state: PhantomData,
            root,
            lock,
        })
    }

    /// Open the cache in the provided directory.
    #[instrument(name = "FsCache::open_dir_std")]
    pub async fn open_dir_std(root: impl Into<std::path::PathBuf> + StdDebug) -> Result<Self> {
        let root = root.into();
        TypedPath::new_abs_dir(root)
            .await
            .context("parse as abs dir")?
            .pipe(Self::open_dir)
            .await
    }

    /// Lock the cache.
    #[instrument(name = "FsCache::lock")]
    pub async fn lock(self) -> Result<FsCache<Locked>> {
        let lock = self.lock.lock().await.context("lock cache")?;
        Ok(FsCache {
            state: PhantomData,
            root: self.root,
            lock,
        })
    }
}

impl FsCache<Locked> {
    /// Unlock the cache.
    #[instrument(name = "FsCache::unlock")]
    pub async fn unlock(self) -> Result<FsCache<Unlocked>> {
        let lock = self.lock.unlock().await.context("unlock cache")?;
        Ok(FsCache {
            state: PhantomData,
            root: self.root,
            lock,
        })
    }

    /// Report whether there are items in the cache.
    #[instrument(name = "FsCache::is_empty")]
    pub async fn is_empty(&self) -> Result<bool> {
        fs::is_dir_empty(&self.root).await
    }
}

impl super::Cache for FsCache<Locked> {
    #[instrument(name = "FsCache::store")]
    async fn store(
        &self,
        kind: Kind,
        key: &Blake3,
        artifacts: impl IntoIterator<Item = impl Into<Artifact>> + StdDebug + Send,
    ) -> Result<()> {
        let artifacts = artifacts.into_iter().map(Into::into).collect_vec();
        let name = self.root.join(kind.as_rel_dir()).join(key.as_rel_file()?);
        let content = Record::builder()
            .key(key)
            .artifacts(artifacts)
            .kind(kind)
            .build()
            .pipe_ref(serde_json::to_string_pretty)
            .context("encode record")?;
        fs::write(&name, content).await.context("store record")
    }

    #[instrument(name = "FsCache::get")]
    async fn get(&self, kind: Kind, key: &Blake3) -> Result<Option<Record>> {
        let name = self.root.join(kind.as_rel_dir()).join(key.as_rel_file()?);
        Ok(
            match fs::read_buffered_utf8(&name).await.context("read file")? {
                Some(content) => serde_json::from_str(&content)
                    .context("decode record")
                    .with_section(|| content.header("Content:"))?,
                None => None,
            },
        )
    }
}

/// The content-addressed storage area shared by all `hurry` cache instances.
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
    root: TypedPath<Abs, Dir>,
}

impl FsCas {
    /// Open an instance in the default location for the user.
    #[instrument(name = "FsCas::open_default")]
    pub async fn open_default() -> Result<Self> {
        fs::user_global_cache_path()
            .await
            .context("find user cache path")?
            .join(mk_rel_dir!("cas"))
            .pipe(Self::open_dir)
            .await
    }

    /// Open an instance in the provided directory.
    #[instrument(name = "FsCas::open_dir")]
    pub async fn open_dir(root: impl Into<TypedPath<Abs, Dir>> + StdDebug) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root).await?;
        trace!(?root, "open cas");
        Ok(Self { root })
    }

    /// Open an instance in the provided directory.
    #[instrument(name = "FsCas::open_dir_std")]
    pub async fn open_dir_std(root: impl Into<std::path::PathBuf> + StdDebug) -> Result<Self> {
        let root = root.into();
        TypedPath::new_abs_dir(root)
            .await
            .context("parse path as abs dir")?
            .pipe(Self::open_dir)
            .await
    }

    /// Report whether there are items in the CAS.
    #[instrument(name = "FsCas::is_empty")]
    pub async fn is_empty(&self) -> Result<bool> {
        fs::is_dir_empty(&self.root).await
    }
}

impl super::Cas for FsCas {
    #[instrument(name = "FsCas::store_file")]
    async fn store_file(&self, kind: Kind, src: &TypedPath<Abs, File>) -> Result<Blake3> {
        let src = src.as_ref();
        let key = Blake3::from_file(src).await.context("hash file")?;
        let dst = self.root.join(kind.as_rel_dir()).join(key.as_rel_file()?);
        fs::copy_file(src, &dst).await?;
        Ok(key)
    }

    #[instrument(name = "FsCas::get_file")]
    async fn get_file(
        &self,
        kind: Kind,
        key: &Blake3,
        destination: &TypedPath<Abs, File>,
    ) -> Result<()> {
        let src = self.root.join(kind.as_rel_dir()).join(key.as_rel_file()?);
        fs::copy_file(&src, destination).await.map(drop)
    }
}
