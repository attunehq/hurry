//! Filesystem operations tailored to `hurry`.
//!
//! Inside this module, we refer to `std::fs` by its fully qualified path to
//! make it maximally clear what we are using.

#![allow(clippy::disallowed_methods)]

use std::{path::Path, time::SystemTime};

use cargo_metadata::camino::Utf8PathBuf;
use color_eyre::{
    Result,
    eyre::{Context, OptionExt},
};
use filetime::{FileTime, set_file_handle_times};
use tap::{Pipe, Tap, TapFallible, TryConv};
use tracing::{instrument, trace, warn};

/// Determine the canonical cache path for the current user, if possible.
///
/// This can fail if the user has no home directory,
/// or if the home directory cannot be accessed.
#[instrument]
pub fn user_global_cache_path() -> Result<Utf8PathBuf> {
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
pub fn create_dir_all(dir: impl AsRef<Path> + std::fmt::Debug) -> Result<()> {
    let dir = dir.as_ref();
    std::fs::create_dir_all(dir)
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
pub fn copy_file(
    src: impl AsRef<Path> + std::fmt::Debug,
    dst: impl AsRef<Path> + std::fmt::Debug,
) -> Result<()> {
    // Manually opening the source file allows us to access the stat info directly,
    // without an additional syscall to stat directly.
    let mut src = std::fs::File::open(src).context("open source file")?;
    let src_meta = src.metadata().context("get source metadata")?;

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

    // Manually opening the destination file allows us to set the metadata directly,
    // without the additional syscall to touch the file metadata.
    //
    // We don't currently care about any other metadata (e.g. permission bits, read only, etc)
    // since the rust compiler is the ultimate arbiter of this data and will reject/rebuild
    // anything that is out of sync.
    //
    // If we find that we have excessive rebuilds we can revisit this.
    let mut dst = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(dst)
        .context("open destination file")?;
    let bytes = std::io::copy(&mut src, &mut dst).context("copy file contents")?;

    // Using the `filetime` crate here instead of the stdlib because it's cross platform.
    let mtime = FileTime::from_system_time(src_mtime);
    let atime = FileTime::from_system_time(src_atime);
    set_file_handle_times(&dst, Some(atime), Some(mtime)).context("set destination file times")?;

    // And finally, we have to sync the file to disk so that we are sure it's actually finished copying
    // before we move on. Technically we could leave this up to the FS, but this is safer.
    dst.sync_all().context("sync destination file")?;

    trace!(?src, ?dst, ?mtime, ?atime, ?bytes, "copy file");
    Ok(())
}

/// Buffer the file content from disk.
#[instrument]
pub fn read_buffered(path: impl AsRef<Path> + std::fmt::Debug) -> Result<Vec<u8>> {
    let path = path.as_ref();
    std::fs::read(path)
        .with_context(|| format!("read file: {path:?}"))
        .tap_ok(|buf| trace!(?path, bytes = buf.len(), "read file"))
}

/// Buffer the file content from disk and parse it as UTF8.
#[instrument]
pub fn read_buffered_utf8(path: impl AsRef<Path> + std::fmt::Debug) -> Result<String> {
    let path = path.as_ref();
    std::fs::read_to_string(path)
        .with_context(|| format!("read file: {path:?}"))
        .tap_ok(|buf| trace!(?path, bytes = buf.len(), "read file as string"))
}

/// Write the provided file content to disk.
#[instrument(skip(content))]
pub fn write(path: impl AsRef<Path> + std::fmt::Debug, content: impl AsRef<[u8]>) -> Result<()> {
    let (path, content) = (path.as_ref(), content.as_ref());
    std::fs::write(path, content)
        .with_context(|| format!("write file: {path:?}"))
        .tap_ok(|_| trace!(?path, bytes = content.len(), "write file"))
}

/// Open a file for reading.
#[instrument]
pub fn open_file(path: impl AsRef<Path> + std::fmt::Debug) -> Result<std::fs::File> {
    let path = path.as_ref();
    std::fs::File::open(path)
        .with_context(|| format!("open file: {path:?}"))
        .tap_ok(|_| trace!(?path, "open file"))
}

/// Read directory entries.
#[instrument]
pub fn read_dir(path: impl AsRef<Path> + std::fmt::Debug) -> Result<std::fs::ReadDir> {
    let path = path.as_ref();
    std::fs::read_dir(path)
        .with_context(|| format!("read directory: {path:?}"))
        .tap_ok(|_| trace!(?path, "read directory"))
}
