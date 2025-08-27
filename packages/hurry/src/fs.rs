//! Filesystem operations tailored to `hurry`.
//!
//! Inside this module, we refer to `std::fs` or `tokio::fs` by its fully
//! qualified path to make it maximally clear what we are using.

#![allow(
    clippy::disallowed_methods,
    reason = "The methods are disallowed elsewhere, but we need them here!"
)]

use std::{
    fmt::Debug as StdDebug,
    fs::Metadata,
    marker::PhantomData,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use ahash::AHashMap;
use cargo_metadata::camino::Utf8PathBuf;
use color_eyre::{
    Result,
    eyre::{Context, OptionExt},
};
use derive_more::{Debug, Display};
use filetime::FileTime;
use fslock::LockFile as FsLockFile;
use rayon::iter::{ParallelBridge, ParallelIterator};
use relative_path::RelativePathBuf;
use tap::{Pipe, Tap, TapFallible, TryConv};
use tokio::{
    fs::{File, ReadDir},
    runtime::Handle,
    sync::Mutex,
    task::spawn_blocking,
};
use tracing::{debug, instrument, trace};
use walkdir::WalkDir;

use crate::{Locked, Unlocked, ext::then_context, hash::Blake3};

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
    #[allow(dead_code)]
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
                WalkDir::new(&root).into_iter().par_bridge().try_for_each(
                    move |entry| -> Result<()> {
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
                    },
                )
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

    /// Whether the file is executable.
    pub executable: bool,
}

impl IndexEntry {
    /// Construct the entry from the provided file on disk.
    #[instrument(name = "IndexEntry::from_file")]
    pub async fn from_file(path: impl AsRef<Path> + StdDebug) -> Result<Self> {
        let path = path.as_ref();
        let (hash, executable) = tokio::try_join!(
            Blake3::from_file(path).then_context("hash file"),
            is_executable(path).then_context("check executable"),
        )?;
        Ok(Self { hash, executable })
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

/// Copy the file from `src` to `dst`.
///
/// Preserves some metadata from `src`:
/// - `mtime`: used for `should_copy_file` and the rust compiler.
/// - `atime`: used by the rust compiler(?)
//
// TODO: should we hold on to the `fs::metadata` result from `should_copy_file`
// and reuse it here instead of statting again?
//
// TODO: use a reflink/fclonefileat/clonefile first, fall back to actual copy if that fails;
// this action will only be supported on filesystems with copy-on-write support.
//
// TODO: optionally use `rustix::fs::copy_file_range` or similar to do linux copies
// fully in kernel instead of passing through userspace(?)
#[instrument]
pub async fn copy_file(
    src: impl AsRef<Path> + StdDebug,
    dst: impl AsRef<Path> + StdDebug,
) -> Result<()> {
    // Manually opening the source file allows us to access the stat info directly,
    // without an additional syscall to stat directly.
    let mut src = tokio::fs::File::open(src)
        .await
        .context("open source file")?;
    let src_meta = src.metadata().await.context("get source metadata")?;

    // If we can't read the actual times from the stat, default to unix epoch
    // so that we don't break the build system.
    //
    // We could promote this to an actual error, but since the rust compiler is ultimately
    // what's going to read this, this is simpler: it'll just transparently rebuild anything
    // that we had to set like this (since the source file will obviously be newer).
    //
    // In other words, this forms a safe "fail closed" system since
    // the rust compiler is the ultimate authority here.
    let src_mtime = src_meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let src_atime = src_meta.accessed().unwrap_or(SystemTime::UNIX_EPOCH);
    if let Some(parent) = dst.as_ref().parent() {
        create_dir_all(parent)
            .await
            .context("create parent directory")?;
    }

    // Manually opening the destination file allows us to set the metadata directly,
    // without the additional syscall to touch the file metadata.
    //
    // We don't currently care about any other metadata (e.g. permission bits, read only, etc)
    // since the rust compiler is the ultimate arbiter of this data and will reject/rebuild
    // anything that is out of sync.
    //
    // If we find that we have excessive rebuilds we can revisit this.
    let mut dst = tokio::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(dst)
        .await
        .context("open destination file")?;
    let bytes = tokio::io::copy(&mut src, &mut dst)
        .await
        .context("copy file contents")?;

    // Using the `filetime` crate here instead of the stdlib because it's cross platform.
    let mtime = FileTime::from_system_time(src_mtime);
    let atime = FileTime::from_system_time(src_atime);
    trace!(?src, ?dst, ?mtime, ?atime, ?bytes, "copy file");

    // We need to get the raw handle for filetime operations
    let dst = set_file_handle_times(dst, Some(atime), Some(mtime))
        .await
        .context("set destination file times")?;

    // And finally, we have to sync the file to disk so that we are sure it's actually finished copying
    // before we move on. Technically we could leave this up to the FS, but this is safer.
    dst.sync_all().await.context("sync destination file")
}

/// Update the `atime` and `mtime` of a file handle.
/// Returns the same file handle after the update.
#[instrument]
pub async fn set_file_handle_times(
    file: File,
    mtime: Option<FileTime>,
    atime: Option<FileTime>,
) -> Result<File> {
    match (mtime, atime) {
        (None, None) => Ok(file),
        (mtime, atime) => {
            let file = file.into_std().await;
            spawn_blocking(move || {
                filetime::set_file_handle_times(&file, atime, mtime).map(|_| file)
            })
            .await
            .context("join thread")?
            .context("update handle")
            .map(File::from_std)
        }
    }
}

/// Buffer the file content from disk.
#[instrument]
#[allow(dead_code)]
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
#[allow(dead_code)]
pub async fn read_dir(path: impl AsRef<Path> + StdDebug) -> Result<ReadDir> {
    let path = path.as_ref();
    tokio::fs::read_dir(path)
        .await
        .with_context(|| format!("read directory: {path:?}"))
        .tap_ok(|_| trace!(?path, "read directory"))
}

/// Report whether the file is executable.
#[instrument]
#[cfg(not(target_os = "windows"))]
pub async fn is_executable(path: impl AsRef<Path> + StdDebug) -> Result<bool> {
    use std::os::unix::fs::PermissionsExt;
    let path = path.as_ref();
    let metadata = tokio::fs::metadata(path).await.context("get metadata")?;
    let is_executable = metadata.permissions().mode() & 0o111 != 0;
    trace!(?is_executable, "is executable");
    Ok(is_executable)
}

/// Set the file as executable.
#[instrument]
#[cfg(not(target_os = "windows"))]
pub async fn set_executable(path: impl AsRef<Path> + StdDebug) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let path = path.as_ref();
    let metadata = tokio::fs::metadata(path).await.context("get metadata")?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(permissions.mode() | 0o111);
    tokio::fs::set_permissions(path, permissions)
        .await
        .context("set permissions")
        .tap_ok(|_| trace!("set executable"))
}

/// Report whether the file is executable.
/// On Windows, this is a simple executable check.
#[instrument]
#[cfg(target_os = "windows")]
pub async fn is_executable(path: impl AsRef<Path> + StdDebug) -> Result<bool> {
    path.as_ref()
        .extension()
        .is_some_and(|ext| ext == "exe")
        .pipe(Ok)
}

/// Set the file as executable.
/// On Windows this is a no-op.
#[instrument]
#[cfg(target_os = "windows")]
pub async fn set_executable(path: impl AsRef<Path> + StdDebug) -> Result<()> {
    Ok(())
}

/// Get the metadata for a file.
pub async fn metadata(path: impl AsRef<Path> + StdDebug) -> Result<Option<Metadata>> {
    let path = path.as_ref();
    match tokio::fs::metadata(path).await {
        Ok(metadata) => {
            trace!(?path, ?metadata, "read metadata");
            Ok(Some(metadata))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).context(format!("remove directory: {path:?}")),
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
