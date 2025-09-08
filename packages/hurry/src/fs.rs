//! Filesystem operations tailored to `hurry`.
//!
//! Inside this module, we refer to `std::fs` or `tokio::fs` by its fully
//! qualified path to make it maximally clear what we are using.
//!
//! ## Other IO implementations
//!
//! We may find that we want to swap to a different IO backend than tokio:
//! - https://docs.rs/compio/latest/compio/
//! - https://docs.rs/nuclei/latest/nuclei/
//! - https://docs.rs/monoio/latest/monoio/
//! - https://docs.rs/rio/latest/rio/
//!
//! Alternatively, we may want to swap to different library implementations:
//! - https://docs.rs/io-uring/latest/io_uring/
//! - https://docs.rs/reflink-copy/latest/reflink_copy/
//!
//! I've held off on this for now until/unless we can prove that
//! tokio and its default way of interfacing with the file system is
//! actually the bottleneck for us.

#![allow(
    clippy::disallowed_methods,
    reason = "The methods are disallowed elsewhere, but we need them here!"
)]

use std::{
    fmt::Debug as StdDebug,
    marker::PhantomData,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use ahash::AHashMap;
use async_walkdir::{DirEntry, WalkDir};
use cargo_metadata::camino::Utf8PathBuf;
use color_eyre::{
    Result,
    eyre::{Context, OptionExt, eyre},
};
use derive_more::{Debug, Display};
use filetime::FileTime;
use fslock::LockFile as FsLockFile;
use futures::{Stream, TryStreamExt};
use jiff::Timestamp;
use rayon::iter::{ParallelBridge, ParallelIterator};
use relative_path::RelativePathBuf;
use serde::{Deserialize, Serialize};
use tap::{Pipe, Tap, TapFallible, TryConv};
use tokio::{
    fs::{File, ReadDir},
    runtime::Handle,
    sync::Mutex,
    task::spawn_blocking,
};
use tracing::{debug, instrument, trace};

use crate::{Locked, Unlocked, ext::then_context, hash::Blake3};

/// The default level of concurrency used in hurry `fs` operations.
///
/// This number was chosen using the results of the `copytarget`
/// benchmark in the hurry repository tested across machines on the team.
pub const DEFAULT_CONCURRENCY: usize = 10;

/// Shared lock file on the file system.
///
/// Lock the file with [`LockFile::lock`]. Unlock it with [`LockFile::unlock`],
/// or by dropping the locked instance.
#[derive(Debug, Clone, Display)]
#[display("{}", path.display())]
pub struct LockFile<State> {
    state: PhantomData<State>,
    path: PathBuf,
    inner: Arc<Mutex<FsLockFile>>,
}

impl LockFile<Unlocked> {
    /// Create a new instance at the provided path.
    pub async fn open(path: impl AsRef<Path> + StdDebug) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let (file, path) = spawn_blocking(move || FsLockFile::open(&path).map(|file| (file, path)))
            .await
            .context("join task")?
            .context("open lock file")?;
        Ok(Self {
            state: PhantomData,
            inner: Arc::new(Mutex::new(file)),
            path,
        })
    }

    /// Lock the lockfile.
    #[instrument(skip_all, fields(%self))]
    pub async fn lock(self) -> Result<LockFile<Locked>> {
        spawn_blocking(move || {
            {
                // fslock::LockFile can panic if the handle is already locked,
                // but we've set it up (using typestate) such that it's not
                // possible to lock an already locked handle.
                let mut inner = self.inner.blocking_lock();
                inner.lock().context("lock file")?;
            }
            Ok(LockFile {
                state: PhantomData,
                inner: self.inner,
                path: self.path,
            })
        })
        .await
        .context("join task")?
        .tap_ok(|f| trace!(path = ?f.path, "locked file"))
    }
}

impl LockFile<Locked> {
    /// Unlock the lockfile.
    #[instrument(skip_all, fields(%self))]
    pub async fn unlock(self) -> Result<LockFile<Unlocked>> {
        spawn_blocking(move || -> Result<_> {
            {
                // fslock::LockFile can panic if the handle is not locked,
                // but we've set it up (using typestate) such that it's not
                // possible to unlock a non-locked handle.
                let mut inner = self.inner.blocking_lock();
                inner.unlock().context("unlock file")?;
            }

            Ok(LockFile {
                state: PhantomData,
                inner: self.inner,
                path: self.path,
            })
        })
        .await
        .context("join task")?
        .tap_ok(|f| trace!(path = ?f.path, "unlocked file"))
    }
}

/// File index of a directory.
#[derive(Clone, Debug)]
pub struct Index {
    /// The root directory of the index.
    pub root: Utf8PathBuf,

    /// Stores the index.
    /// Keys relative to `root`.
    //
    // TODO: May want to make this a trie or something.
    // https://docs.rs/fs-tree/0.2.2/fs_tree/ looked like it might work,
    // but the API was sketchy so I didn't use it for now.
    #[debug("{}", files.len())]
    pub files: AHashMap<RelativePathBuf, IndexEntry>,
}

impl Index {
    /// Index the provided path recursively.
    //
    // TODO: move this to use async natively.
    #[instrument(name = "Index::recursive")]
    pub async fn recursive(root: impl AsRef<Path> + StdDebug) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        spawn_blocking(move || Self::recursive_sync(root))
            .await
            .context("join task")?
    }

    /// Index the provided path recursively, blocking the current thread.
    #[instrument(name = "Index::recursive_sync")]
    fn recursive_sync(root: impl AsRef<Path> + StdDebug) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let root = Utf8PathBuf::try_from(root).context("path as utf8")?;

        // The `rayon` instance runs in its own threadpool, but its overall
        // operation is still blocking, so we run it in a background thread that
        // just waits for rayon to complete.
        let (tx, rx) = flume::bounded::<(RelativePathBuf, IndexEntry)>(0);
        let runtime = Handle::current();
        let walker = std::thread::spawn({
            let root = root.clone();
            let runtime = runtime.clone();
            move || {
                walkdir::WalkDir::new(&root)
                    .into_iter()
                    .par_bridge()
                    .try_for_each(move |entry| -> Result<()> {
                        let _guard = runtime.enter();
                        let entry = entry.context("walk files")?;
                        let path = entry.path();
                        if !entry.file_type().is_file() {
                            trace!(?path, "skipped entry: not a file");
                            return Ok(());
                        }

                        trace!(?path, "walked entry");
                        let path = path
                            .strip_prefix(&root)
                            .with_context(|| format!("make {path:?} relative to {root:?}"))?
                            .to_path_buf()
                            .pipe(RelativePathBuf::from_path)
                            .context("read path as utf8")?;
                        let entry = runtime
                            .block_on(IndexEntry::from_file(entry.path()))
                            .context("index entry")?;

                        // Only errors if the channel receivers have been dropped,
                        // which should never happen but we'll handle it
                        // just in case.
                        tx.send((path, entry)).context("send entry to main thread")
                    })
            }
        });

        // When the directory walk finishes, the senders all drop.
        // This causes the receiver channel to close, terminating the iterator.
        let files = rx
            .into_iter()
            .inspect(|(path, entry)| trace!(?path, ?entry, "indexed file"))
            .collect();

        // Joining a fallible operation from a background thread (as we do here)
        // has two levels of errors:
        // - The thread could have panicked
        // - The operation could have completed fallibly
        //
        // The `expect` call here is for the former case: if the thread panicks,
        // the only really safe thing to do is also panic since panic implies
        // a broken invariant or partially corrupt state.
        //
        // Then the `context` call wraps the result of the actual fallible
        // operation that we were doing inside the thread (walking the files).
        walker
            .join()
            .expect("join thread")
            .context("walk directory")?;

        debug!("indexed directory");
        Ok(Self { root, files })
    }
}

/// An entry for a file that was indexed in [`Index`].
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct IndexEntry {
    /// The hash of the file's contents.
    pub hash: Blake3,

    /// The metadata of the file.
    pub metadata: Metadata,
}

impl IndexEntry {
    /// Construct the entry from the provided file on disk.
    #[instrument(name = "IndexEntry::from_file")]
    pub async fn from_file(path: impl AsRef<Path> + StdDebug) -> Result<Self> {
        let path = path.as_ref();
        let hash = Blake3::from_file(path).then_context("hash file").await?;
        let metadata = Metadata::from_file(path)
            .then_context("get metadata")
            .await?
            .ok_or_eyre(format!("file {path:?} should exist"))?;
        Ok(Self { hash, metadata })
    }
}

/// Determine the canonical cache path for the current user, if possible.
///
/// This can fail if the user has no home directory,
/// or if the home directory cannot be accessed.
#[instrument]
pub async fn user_global_cache_path() -> Result<Utf8PathBuf> {
    homedir::my_home()
        .context("get user home directory")?
        .ok_or_eyre("user has no home directory")?
        .try_conv::<Utf8PathBuf>()
        .context("user home directory is not utf8")?
        .join(".cache")
        .join("hurry")
        .join("v2")
        .tap(|dir| trace!(?dir, "read user global cache path"))
        .pipe(Ok)
}

/// Create the directory and all its parents, if they don't already exist.
#[instrument]
pub async fn create_dir_all(dir: impl AsRef<Path> + StdDebug) -> Result<()> {
    let dir = dir.as_ref();
    tokio::fs::create_dir_all(dir)
        .await
        .with_context(|| format!("create dir: {dir:?}"))
        .tap_ok(|_| trace!(?dir, "create directory"))
}

/// Recursively copy the contents of `src` to `dst`.
///
/// Preserves metadata that cargo/rustc cares about during the copy.
/// Returns the total number of bytes copied across all files.
///
/// Equivalent to [`copy_dir_with_concurrency`] with [`DEFAULT_CONCURRENCY`].
#[instrument]
pub async fn copy_dir(
    src: impl AsRef<Path> + StdDebug,
    dst: impl AsRef<Path> + StdDebug,
) -> Result<u64> {
    copy_dir_with_concurrency(DEFAULT_CONCURRENCY, src, dst).await
}

/// Walk files in a directory recursively.
///
/// Only emits regular files; symbolic links
/// and directories are not emitted in the stream.
#[instrument]
pub fn walk_files(
    root: impl AsRef<Path> + StdDebug,
) -> impl Stream<Item = Result<DirEntry>> + Unpin {
    let root = root.as_ref().to_path_buf();
    WalkDir::new(&root)
        .map_err(move |err| eyre!(err).wrap_err(format!("walk files in {root:?}")))
        .try_filter_map(|entry| async move {
            let src_file = entry.path();
            let ft = entry
                .file_type()
                .await
                .with_context(|| format!("get type of: {src_file:?}"))?;
            if ft.is_file() {
                Ok(Some(entry))
            } else {
                Ok(None)
            }
        })
        .pipe(Box::pin)
}

/// Report whether the provided directory is empty.
/// For the purpose of this function, the directory is empty
/// if it has no regular files.
#[instrument]
pub async fn is_dir_empty(path: impl AsRef<Path> + StdDebug) -> Result<bool> {
    let path = path.as_ref();
    walk_files(path)
        .try_any(|_| async { true })
        .await
        .map(|found| !found)
}

/// Recursively copy the contents of `src` to `dst` with specified concurrency.
///
/// Preserves metadata that cargo/rustc cares about during the copy.
/// Returns the total number of bytes copied across all files.
#[instrument]
pub async fn copy_dir_with_concurrency(
    concurrency: usize,
    src: impl AsRef<Path> + StdDebug,
    dst: impl AsRef<Path> + StdDebug,
) -> Result<u64> {
    let (src, dst) = (src.as_ref(), dst.as_ref());
    walk_files(&src)
        .map_ok(|entry| async move {
            let src_file = entry.path();
            let rel = src_file
                .strip_prefix(&src)
                .with_context(|| format!("make {src_file:?} relative to {src:?}"))?;

            let dst_file = dst.join(rel);
            copy_file(&src_file, &dst_file)
                .await
                .with_context(|| format!("copy {src_file:?} to {dst_file:?}"))
        })
        .try_buffer_unordered(concurrency)
        .try_fold(0u64, |total, copied| async move { Ok(total + copied) })
        .await
}

/// Copy the file from `src` to `dst`.
///
/// Preserves metadata that cargo/rustc cares about during the copy.
/// Returns the number of bytes copied.
#[instrument]
pub async fn copy_file(
    src: impl AsRef<Path> + StdDebug,
    dst: impl AsRef<Path> + StdDebug,
) -> Result<u64> {
    let (src, dst) = (src.as_ref(), dst.as_ref());

    if let Some(parent) = dst.parent() {
        create_dir_all(parent)
            .await
            .context("create parent directory")?;
    }
    let bytes = tokio::fs::copy(src, dst).await.context("copy file")?;
    trace!(?src, ?dst, ?bytes, "copy file");

    Ok(bytes)
}

/// Buffer the file content from disk.
#[instrument]
pub async fn read_buffered(path: impl AsRef<Path> + StdDebug) -> Result<Option<Vec<u8>>> {
    let path = path.as_ref();
    match tokio::fs::read(path).await {
        Ok(buf) => {
            trace!(?path, bytes = buf.len(), "read file");
            Ok(Some(buf))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).context(format!("read file: {path:?}")),
    }
}

/// Buffer the file content from disk and parse it as UTF8.
#[instrument]
pub async fn read_buffered_utf8(path: impl AsRef<Path> + StdDebug) -> Result<Option<String>> {
    let path = path.as_ref();
    match tokio::fs::read_to_string(path).await {
        Ok(buf) => {
            trace!(?path, bytes = buf.len(), "read file as string");
            Ok(Some(buf))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).context(format!("read file: {path:?}")),
    }
}

/// Write the provided file content to disk.
#[instrument(skip(content))]
pub async fn write(path: impl AsRef<Path> + StdDebug, content: impl AsRef<[u8]>) -> Result<()> {
    let (path, content) = (path.as_ref(), content.as_ref());
    if let Some(parent) = path.parent() {
        create_dir_all(parent)
            .await
            .context("create parent directory")?;
    }
    tokio::fs::write(path, content)
        .await
        .with_context(|| format!("write file: {path:?}"))
        .tap_ok(|_| trace!(?path, bytes = content.len(), "write file"))
}

/// Open a file for reading.
#[instrument]
pub async fn open_file(path: impl AsRef<Path> + StdDebug) -> Result<File> {
    let path = path.as_ref();
    File::open(path)
        .await
        .with_context(|| format!("open file: {path:?}"))
        .tap_ok(|_| trace!(?path, "open file"))
}

/// Read directory entries.
#[instrument]
pub async fn read_dir(path: impl AsRef<Path> + StdDebug) -> Result<ReadDir> {
    let path = path.as_ref();
    tokio::fs::read_dir(path)
        .await
        .with_context(|| format!("read directory: {path:?}"))
        .tap_ok(|_| trace!(?path, "read directory"))
}

/// The set of metadata that hurry cares about.
///
/// This has a few goals compared to the standard set of metadata:
/// - Track only the fields hurry believes cargo/rustc care about.
/// - Be comparable with other instances for testing/diffing.
/// - Be cross platform (namely, on Windows).
///
/// We will probably need to add more fields as we find things that cargo/rustc
/// care about that we overlooked; don't treat this as gospel if you think
/// something is missing.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub struct Metadata {
    /// The last time the file was modified.
    ///
    /// If the mtime is not available on the file, defaults to the unix epoch.
    /// The intention here is that cargo/rustc use "is the mtime of the source
    /// file newer than the mtime of the artifact in target" to determine if
    /// the artifact needs to be rebuilt; since we want to have the system
    /// "fail open" (meahing: we prefer to rebuild more if there is a question
    /// instead of produce bad builds) this is an acceptable fallback.
    #[debug("{}", Timestamp::try_from(mtime.clone()).map(|t| t.to_string()).unwrap_or_else(|_| format!("{mtime:?}")))]
    pub mtime: SystemTime,

    /// Whether the file is executable.
    ///
    /// On unix, this is set according to the executable bit.
    /// On windows, this is set according to file extension.
    pub executable: bool,
}

impl Metadata {
    /// Read the metadata from the provided file.
    #[instrument]
    #[cfg(not(target_os = "windows"))]
    pub async fn from_file(path: impl AsRef<Path> + StdDebug) -> Result<Option<Self>> {
        use std::os::unix::fs::PermissionsExt;
        let path = path.as_ref();

        let metadata = match metadata(path).await? {
            Some(metadata) => metadata,
            None => return Ok(None),
        };
        let mtime = metadata
            .modified()
            .with_context(|| format!("read file {path:?} mtime"))?;
        let executable = metadata.permissions().mode() & 0o111 != 0;
        Ok(Some(Self { mtime, executable }))
    }

    /// Set the metadata on the provided file.
    #[instrument]
    #[cfg(not(target_os = "windows"))]
    pub async fn set_file(&self, path: impl AsRef<Path> + StdDebug) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let path = path.as_ref();

        // We read the current metadata for the file so that we don't
        // accidentally clobber other fields (although it's not clear
        // that this is necessary- we mostly do this out of an abundance
        // of caution as we want to avoid breaking things).
        // If this ends up being too much of a performance hit we should
        // revisit.
        if self.executable {
            let metadata = tokio::fs::metadata(path).await.context("get metadata")?;
            let mut permissions = metadata.permissions();
            permissions.set_mode(permissions.mode() | 0o111);
            tokio::fs::set_permissions(path, permissions.clone())
                .await
                .context("set permissions")
                .tap_ok(|_| trace!(?path, ?permissions, "set permissions"))?;
        }

        // Make sure to set the file times last so that other modifications to
        // the metadata don't mess with these.
        let mtime = FileTime::from_system_time(self.mtime);
        let path = path.to_path_buf();
        spawn_blocking(move || {
            filetime::set_file_mtime(&path, mtime).tap_ok(|_| trace!(?path, ?mtime, "update mtime"))
        })
        .await
        .context("join thread")?
        .context("update handle")
    }
}

/// Remove the directory and all its contents.
pub async fn remove_dir_all(path: impl AsRef<Path> + StdDebug) -> Result<()> {
    let path = path.as_ref();
    match tokio::fs::remove_dir_all(path).await {
        Ok(()) => {
            trace!(?path, "removed directory");
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            trace!(?path, "removed directory (already removed)");
            Ok(())
        }
        Err(err) => Err(err).context(format!("remove directory: {path:?}")),
    }
}

/// Get the standard metadata for the file.
///
/// Note: you probably want [`Metadata::from_file`] instead,
/// although this function exists in case you need the standard metadata shape
/// for some reason.
#[instrument]
pub async fn metadata(path: impl AsRef<Path> + StdDebug) -> Result<Option<std::fs::Metadata>> {
    let path = path.as_ref();
    match tokio::fs::metadata(path).await {
        Ok(metadata) => {
            trace!(?path, ?metadata, "stat metadata");
            Ok(Some(metadata))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).context(format!("stat metadata: {path:?}")),
    }
}
