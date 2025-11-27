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
    convert::identity, fmt::Debug as StdDebug, marker::PhantomData, sync::Arc, time::SystemTime,
};

use bon::Builder;
use color_eyre::{
    Result,
    eyre::{Context, OptionExt, bail},
};
use derive_more::{Debug, Display};
use filetime::FileTime;
use fslock::LockFile as FsLockFile;
use futures::{Stream, TryStreamExt};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tap::{Pipe, TapFallible, TryConv as _};
use tokio::{fs::ReadDir, io::AsyncReadExt, sync::Mutex, task::spawn_blocking};
use tracing::{debug, error, instrument, trace, warn};

use clients::courier::v1::Key;
use uuid::Uuid;

use crate::path::{
    Abs, AbsDirPath, AbsFilePath, JoinWith as _, RelativeTo as _, SomeType, TryJoinWith as _,
    TypedPath,
};

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
#[display("{path}")]
pub struct LockFile<State> {
    state: PhantomData<State>,
    path: AbsFilePath,
    inner: Arc<Mutex<FsLockFile>>,
}

/// The associated type's state is unlocked.
/// Used for the typestate pattern.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Display, Default)]
pub struct Unlocked;

/// The associated type's state is locked.
/// Used for the typestate pattern.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Display, Default)]
pub struct Locked;

impl LockFile<Unlocked> {
    /// Create a new instance at the provided path.
    pub async fn open(path: impl Into<AbsFilePath> + StdDebug) -> Result<Self> {
        let path = path.into();
        let (file, path) =
            spawn_blocking(move || FsLockFile::open(path.as_std_path()).map(|file| (file, path)))
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

/// Determine the canonical cache path for the current user, if possible.
///
/// ## Strategy
///
/// Attempts to put the cache directory in the correct place depending on the
/// conventions of the operating system in which hurry is running.
///
/// - Linux: `$XDG_CACHE_HOME/hurry/v2`
/// - macOS: `$HOME/Library/Caches/com.attunehq.hurry/v2`
/// - Windows: `%LOCALAPPDATA%\hurry\v2`
///
/// If unable to find those directories, falls back to:
/// - Linux/macOS: `$HOME/.cache/hurry/v2`
/// - Windows: `%USERPROFILE%\.cache\hurry\v2`
///
/// ## Errors
///
/// This can fail if the user has no home directory or if it cannot be accessed.
#[instrument]
pub async fn user_global_cache_path() -> Result<AbsDirPath> {
    let dirs = spawn_blocking(|| directories::ProjectDirs::from("com", "attunehq", "hurry"))
        .await
        .expect("join task");

    let base = if let Some(dirs) = dirs {
        dirs.cache_dir().to_path_buf()
    } else {
        homedir::my_home()
            .context("get user home directory")?
            .ok_or_eyre("user has no home directory")?
            .join(".cache")
            .join("hurry")
    };

    base.join("v2")
        .try_conv::<AbsDirPath>()
        .tap_ok(|dir| debug!(?dir, "user global cache path"))
}

/// Create the directory and all its parents, if they don't already exist.
#[instrument]
pub async fn create_dir_all(dir: &AbsDirPath) -> Result<()> {
    tokio::fs::create_dir_all(dir.as_std_path())
        .await
        .with_context(|| format!("create dir: {dir:?}"))
        .tap_ok(|_| trace!(?dir, "create directory"))
}

/// Recursively copy the contents of `src` to `dst`.
///
/// Returns the total number of bytes copied across all files.
/// Equivalent to [`copy_dir_with_concurrency`] with [`DEFAULT_CONCURRENCY`].
#[instrument]
pub async fn copy_dir(src: &AbsDirPath, dst: &AbsDirPath) -> Result<u64> {
    copy_dir_with_concurrency(DEFAULT_CONCURRENCY, src, dst).await
}

/// Walk files in a directory recursively.
///
/// Only emits regular files; symbolic links
/// and directories are not emitted in the stream.
#[instrument]
pub fn walk_files(root: &AbsDirPath) -> impl Stream<Item = Result<AbsFilePath>> + Unpin {
    let (tx, rx) = flume::bounded::<Result<AbsFilePath>>(0);
    let root = root.clone();

    spawn_blocking(move || {
        for entry in jwalk::WalkDir::new(root.as_std_path()).skip_hidden(false) {
            let entry = match entry.with_context(|| format!("walk files in {root:?}")) {
                Ok(entry) => entry,
                Err(err) => {
                    if let Err(send) = tx.send(Err(err)) {
                        let err = send.into_inner();
                        error!(error = ?err, "unable to walk files");
                        return;
                    }
                    continue;
                }
            };

            if !entry.file_type().is_file() {
                continue;
            }

            let path = match AbsFilePath::try_from(entry.path()) {
                Ok(path) => path,
                Err(err) => {
                    if let Err(send) = tx.send(Err(err)) {
                        let err = send.into_inner();
                        error!(error = ?err, "unable to walk files");
                        return;
                    }
                    continue;
                }
            };

            if let Err(send) = tx.send(Ok(path)) {
                let err = send.into_inner();
                error!(error = ?err, "unable to walk files");
                return;
            }
        }
    });

    rx.into_stream().pipe(Box::pin)
}

/// Report whether the provided directory is empty.
/// For the purpose of this function, the directory is empty
/// if it has no regular files.
#[instrument]
pub async fn is_dir_empty(path: &AbsDirPath) -> Result<bool> {
    walk_files(path)
        .try_any(|_| async { true })
        .await
        .map(|found| !found)
}

/// Recursively copy the contents of `src` to `dst` with specified concurrency.
///
/// Returns the total number of bytes copied across all files.
#[instrument]
pub async fn copy_dir_with_concurrency(
    concurrency: usize,
    src: &AbsDirPath,
    dst: &AbsDirPath,
) -> Result<u64> {
    walk_files(src)
        .map_ok(|src_file| async move {
            let rel = src_file.relative_to(src).context("make relative")?;

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
/// Returns the total number of bytes copied.
#[instrument]
pub async fn copy_file(src: &AbsFilePath, dst: &AbsFilePath) -> Result<u64> {
    if let Some(parent) = dst.parent() {
        create_dir_all(&parent)
            .await
            .context("create parent directory")?;
    }
    let bytes = tokio::fs::copy(src.as_std_path(), dst.as_std_path())
        .await
        .context("copy file")?;
    trace!(?src, ?dst, ?bytes, "copy file");

    Ok(bytes)
}

/// Buffer the file content from disk.
#[instrument]
pub async fn read_buffered(path: &AbsFilePath) -> Result<Option<Vec<u8>>> {
    match tokio::fs::read(path.as_std_path()).await {
        Ok(buf) => {
            trace!(?path, bytes = buf.len(), "read file");
            Ok(Some(buf))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).context(format!("read file: {path:?}")),
    }
}

/// Buffer the file content from disk.
/// Unlike [`read_buffered`], this function returns an error if the file
/// doesn't exist.
#[instrument]
pub async fn must_read_buffered(path: &AbsFilePath) -> Result<Vec<u8>> {
    tokio::fs::read(path.as_std_path())
        .await
        .with_context(|| format!("read file: {path:?}"))
}

/// Buffer the file content from disk and parse it as UTF8.
#[instrument]
pub async fn read_buffered_utf8(path: &AbsFilePath) -> Result<Option<String>> {
    match tokio::fs::read_to_string(path.as_std_path()).await {
        Ok(buf) => {
            trace!(?path, bytes = buf.len(), "read file as string");
            Ok(Some(buf))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).context(format!("read file: {path:?}")),
    }
}

/// Buffer the file content from disk and parse it as UTF8.
/// Unlike [`read_buffered_utf8`], this function returns an error if the file
/// doesn't exist.
#[instrument]
pub async fn must_read_buffered_utf8(path: &AbsFilePath) -> Result<String> {
    tokio::fs::read_to_string(path.as_std_path())
        .await
        .with_context(|| format!("read file: {path:?}"))
}

/// Write the provided file content to disk.
#[instrument(skip(content))]
pub async fn write(path: &AbsFilePath, content: impl AsRef<[u8]>) -> Result<()> {
    let content = content.as_ref();
    if let Some(parent) = path.parent() {
        create_dir_all(&parent)
            .await
            .context("create parent directory")?;
    }
    tokio::fs::write(path.as_std_path(), content)
        .await
        .with_context(|| format!("write file: {path:?}"))
        .tap_ok(|_| trace!(?path, bytes = content.len(), "write file"))
}

/// Open a file for reading.
#[instrument]
pub async fn open_file(path: &AbsFilePath) -> Result<tokio::fs::File> {
    tokio::fs::File::open(path.as_std_path())
        .await
        .with_context(|| format!("open file: {path:?}"))
        .tap_ok(|_| trace!(?path, "open file"))
}

/// Open a file for writing.
#[instrument]
pub async fn create_file(path: &AbsFilePath) -> Result<tokio::fs::File> {
    tokio::fs::File::create(path.as_std_path())
        .await
        .with_context(|| format!("create file: {path:?}"))
        .tap_ok(|_| trace!(?path, "create file"))
}

/// Remove a file.
#[instrument]
pub async fn remove_file(path: &AbsFilePath) -> Result<()> {
    tokio::fs::remove_file(path.as_std_path())
        .await
        .with_context(|| format!("remove file: {path:?}"))
        .tap_ok(|_| trace!(?path, "remove file"))
}

/// Rename a file or folder, overwriting the destination if it already exists.
#[instrument]
pub async fn rename<T>(from: &TypedPath<Abs, T>, to: &TypedPath<Abs, T>) -> Result<()> {
    tokio::fs::rename(from.as_std_path(), to.as_std_path())
        .await
        .with_context(|| format!("rename {from:?} to {to:?}"))
        .tap_ok(|_| trace!(?from, ?to, "rename"))
}

/// Rename a file or directory, returning a guard that restores it to its
/// original location when dropped.
///
/// This function attempts to create the temporary path "close to the original":
/// - If the path has a name, the temporary file uses it as a prefix.
/// - If the path has a parent, the temporary file is stored there too.
/// - If neither are true, this function errors.
///
/// The intention for using this function is mainly for temporary backups; the
/// rename destination is a "sibling" of the original path so that:
/// 1. We can rename without worrying about crossing file systems.
/// 2. If the process dies part of the way through for some reason and cleanup
///    isn't possible, it's hopefully obvious to the user how to recover.
///
/// ## Example
///
/// If your original path looks like this:
/// ```not_rust
/// /some/path/to/entity.ext
/// ```
///
/// The renamed version should look like this:
/// ```not_rust
/// /some/path/to/entity.ext.2f5ba9bf-0672-47eb-8542-710b0967330c
/// ```
#[instrument]
pub async fn rename_temporary(from: &TypedPath<Abs, SomeType>) -> Result<RenameGuard> {
    let Some(parent) = from.parent() else {
        bail!("path does not have a parent: {from:?}");
    };
    let Some(name) = from.file_name() else {
        bail!("path does not have a name: {from:?}");
    };
    // HACK: We just use try_join_file here because there's no try_join for
    // arbitrary T and the validators don't do anything yet anyway for Dir/File.
    let to: TypedPath<Abs, SomeType> = parent
        .try_join_file(format!(
            "{}.{}",
            String::from_utf8_lossy(name.as_encoded_bytes()),
            Uuid::new_v4()
        ))?
        .into();

    rename(&from, &to)
        .await
        .map(|_| RenameGuard::new(from.clone(), to.clone()))
}

/// Restore a file or directory to its original location when dropped.
///
/// ## Leaking
///
/// Restoring the rename can fail if:
/// - A new file exists in the `original` location.
/// - The program crashes before the `Drop` implementation can run.
/// - The instance of `RenameGuard` is prevented from dropping.
#[derive(Debug)]
pub struct RenameGuard {
    restored: bool,
    original: TypedPath<Abs, SomeType>,
    renamed: TypedPath<Abs, SomeType>,
}

impl RenameGuard {
    /// Create a new instance with the provided paths.
    fn new(original: TypedPath<Abs, SomeType>, renamed: TypedPath<Abs, SomeType>) -> Self {
        trace!(?original, ?renamed, "temporarily renamed path");
        Self {
            restored: false,
            original,
            renamed,
        }
    }

    /// Restore the file or directory to its original location.
    ///
    /// This is also performed on drop, so you only need to do this if you want
    /// to ensure the synchronous drop IO doesn't get performed or if you want
    /// to handle errors explicitly.
    #[instrument]
    pub async fn restore(&mut self) -> Result<()> {
        trace!(
            original = ?self.original,
            renamed = ?self.renamed,
            "restored temporary renamed path"
        );

        rename(&self.renamed, &self.original).await?;
        self.restored = true;
        Ok(())
    }
}

#[allow(clippy::disallowed_methods, reason = "cannot use async in drop")]
impl Drop for RenameGuard {
    fn drop(&mut self) {
        if !self.restored {
            match std::fs::rename(&self.renamed, &self.original) {
                Ok(_) => trace!(
                    original = ?self.original,
                    renamed = ?self.renamed,
                    "restored temporary renamed path"
                ),
                Err(error) => warn!(
                    original = ?self.original,
                    renamed = ?self.renamed,
                    ?error,
                    "failed to restore temporarily renamed path"
                ),
            }
        }
    }
}

/// Read directory entries.
#[instrument]
pub async fn read_dir(path: &AbsDirPath) -> Result<ReadDir> {
    tokio::fs::read_dir(path.as_std_path())
        .await
        .tap_ok(|_| trace!(?path, "read directory"))
        .map_err(|err| err.into())
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
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize, Builder)]
pub struct Metadata {
    /// The last time the file was modified.
    ///
    /// If the mtime is not available on the file, defaults to the unix epoch.
    /// The intention here is that cargo/rustc use "is the mtime of the source
    /// file newer than the mtime of the artifact in target" to determine if
    /// the artifact needs to be rebuilt; since we want to have the system
    /// "fail open" (meahing: we prefer to rebuild more if there is a question
    /// instead of produce bad builds) this is an acceptable fallback.
    #[debug("{}", Timestamp::try_from(*mtime).map(|t| t.to_string()).unwrap_or_else(|_| format!("{mtime:?}")))]
    pub mtime: SystemTime,

    /// Whether the file is executable.
    pub executable: bool,

    /// The size of the file in bytes.
    pub len: u64,
}

impl Metadata {
    /// Read the metadata from the provided file.
    #[instrument(name = "Metadata::from_file")]
    pub async fn from_file(path: &AbsFilePath) -> Result<Option<Self>> {
        let path = path.as_std_path();
        let (executable, metadata) = tokio::join!(is_executable(path), metadata(path));
        let metadata = match metadata? {
            Some(metadata) => metadata,
            None => return Ok(None),
        };
        let mtime = metadata
            .modified()
            .with_context(|| format!("read file {path:?} mtime"))?;
        Ok(Some(Self {
            mtime,
            executable,
            len: metadata.len(),
        }))
    }

    /// Set the metadata on the provided file.
    ///
    /// ## Windows
    ///
    /// This function does not attempt to set whether a file is executable on
    /// Windows: in Windows files do not have "executable bits" and
    /// therefore whether they are executable is an intrinsic property of either
    /// the path extension or the file itself.
    #[instrument(name = "Metadata::set_file")]
    pub async fn set_file(&self, path: &AbsFilePath) -> Result<()> {
        set_executable(path, self.executable).await?;

        // Make sure to set the file times last so that other modifications to
        // the metadata don't mess with these.
        set_mtime(path, self.mtime).await?;

        Ok(())
    }
}

/// Remove the directory and all its contents.
pub async fn remove_dir_all(path: &AbsDirPath) -> Result<()> {
    match tokio::fs::remove_dir_all(path.as_std_path()).await {
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
pub async fn metadata(
    path: impl AsRef<std::path::Path> + StdDebug,
) -> Result<Option<std::fs::Metadata>> {
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

/// Check whether the file exists.
///
/// Returns `false` if there is an error checking whether the path exists.
/// Note that this sort of check is prone to race conditions - if you plan
/// to do anything with the file after checking, you should probably
/// just try to do the operation and handle the case of the file not existing.
#[instrument]
pub async fn exists(path: impl AsRef<std::path::Path> + StdDebug) -> bool {
    tokio::fs::try_exists(path).await.is_ok_and(identity)
}

/// Check whether the file is executable.
///
/// Returns false if there is an error checking whether the file is executable.
/// Note that this sort of check is prone to race conditions- if you plan
/// to do anything with the file after checking, you should probably
/// just try to do the operation and handle the case of the file not existing.
#[instrument]
pub async fn is_executable(path: impl AsRef<std::path::Path> + StdDebug) -> bool {
    let path = path.as_ref().to_path_buf();
    spawn_blocking(move || is_executable::is_executable(path))
        .await
        .expect("join task")
}

/// Set the mtime of the file.
#[instrument]
pub async fn set_mtime(path: &AbsFilePath, mtime: SystemTime) -> Result<()> {
    let mtime = FileTime::from_system_time(mtime);
    let path = path.as_std_path().to_path_buf();
    spawn_blocking(move || {
        filetime::set_file_mtime(&path, mtime).tap_ok(|_| trace!(?path, ?mtime, "update mtime"))
    })
    .await
    .context("join thread")?
    .context("update handle")
}

/// Set the file to be executable.
///
/// ## Windows
///
/// This function does not attempt to set whether a file is executable on
/// Windows: in Windows files do not have "executable bits" and
/// therefore whether they are executable is an intrinsic property of either
/// the path extension or the file itself.
#[instrument]
pub async fn set_executable(path: &AbsFilePath, executable: bool) -> Result<()> {
    // We read the current metadata for the file so that we don't accidentally
    // clobber other fields (although it's not clear that this is necessary - we
    // mostly do this out of an abundance of caution as we want to avoid
    // breaking things). If this ends up being too much of a performance hit we
    // should revisit.
    #[cfg(not(target_os = "windows"))]
    if executable {
        use std::os::unix::fs::PermissionsExt as _;

        let metadata = tokio::fs::metadata(path.as_std_path())
            .await
            .context("get metadata")?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(permissions.mode() | 0o111);
        tokio::fs::set_permissions(path.as_std_path(), permissions.clone())
            .await
            .context("set permissions")
            .tap_ok(|_| trace!(?path, ?permissions, "set permissions"))?;
    }
    Ok(())
}

/// Create a hard link to the file.
#[instrument]
pub async fn hard_link(original: &AbsFilePath, link: &AbsFilePath) -> Result<()> {
    if exists(link).await {
        remove_file(link)
            .await
            .context("remove linked destination")?;
    }

    tokio::fs::hard_link(original.as_std_path(), link.as_std_path())
        .await
        .context(format!("hard link {original:?} -> {link:?}"))
}

/// Return whether the path represents a directory.
///
/// Returns `false` if the directory doesn't exist
/// or if there is an error checking the metadata;
/// to differentiate this case use [`metadata`].
#[instrument]
pub async fn is_dir(path: impl AsRef<std::path::Path> + StdDebug) -> bool {
    metadata(path)
        .await
        .is_ok_and(|m| m.is_some_and(|m| m.is_dir()))
}

/// Return whether the path represents a normal file.
///
/// Returns `false` if the file doesn't exist;
/// or if there is an error checking the metadata;
/// to differentiate this case use [`metadata`].
#[instrument]
pub async fn is_file(path: impl AsRef<std::path::Path> + StdDebug) -> bool {
    metadata(path)
        .await
        .is_ok_and(|m| m.is_some_and(|m| m.is_file()))
}

/// Synchronously hash the contents of the file at the specified path.
#[instrument]
pub fn hash_file_sync(path: &AbsFilePath) -> Result<Key> {
    let mut file =
        std::fs::File::open(path.as_std_path()).with_context(|| format!("open file: {path}"))?;
    let mut hasher = blake3::Hasher::new();
    let bytes = std::io::copy(&mut file, &mut hasher).context("hash file")?;
    let hash = hasher.finalize();
    let key = Key::from_blake3(hash);
    trace!(?path, hash = %key, ?bytes, "hash file");
    Ok(key)
}

/// Hash the contents of the file at the specified path.
#[instrument]
pub async fn hash_file(path: &AbsFilePath) -> Result<Key> {
    let mut file = open_file(path).await.context("open file")?;
    let mut hasher = blake3::Hasher::new();
    let mut data = vec![0; 64 * 1024];
    let mut bytes = 0;
    loop {
        let len = file.read(&mut data).await.context("read chunk")?;
        if len == 0 {
            break;
        }
        hasher.update(&data[..len]);
        bytes += len;
    }
    let hash = hasher.finalize();
    let key = Key::from_blake3(hash);
    trace!(?path, hash = %key, ?bytes, "hash file");
    Ok(key)
}
