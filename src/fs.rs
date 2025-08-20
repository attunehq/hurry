//! Filesystem operations tailored to `hurry`.
//!
//! Inside this module, we refer to `std::fs` by its fully qualified path to
//! make it maximally clear what we are using.

use std::{
    path::{Path, PathBuf},
    str::FromStr,
    time::SystemTime,
};

use bon::Builder;
use cargo_metadata::camino::Utf8PathBuf;
use color_eyre::{
    Result,
    eyre::{Context, OptionExt},
};
use dashmap::DashSet;
use filetime::{FileTime, set_file_handle_times};
use rayon::iter::{ParallelBridge, ParallelIterator};
use serde::{Deserialize, Serialize};
use tap::{Pipe, TryConv};
use tracing::{debug, instrument, trace, warn};
use walkdir::WalkDir;

use crate::hash::Blake3;

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
        .pipe(Ok)
}

/// Convert the provided string into a path, if valid.
pub fn into_path(path: impl AsRef<str>) -> Result<Utf8PathBuf> {
    let path = path.as_ref();
    Utf8PathBuf::from_str(path).with_context(|| format!("convert to path: {path}"))
}

/// Recursively copy a directory in parallel.
///
/// - Preserves file `mtime` and `atime`.
/// - Uses `mtime` and file size (similar to rsync) to determine if the file
///   needs to be synced.
//
// TODO: what does the rust compiler/cargo do here? we should be at least as
// correct as it is. maybe there's some work we can reuse? References:
// - `target/{debug|release}/incremental/.../.../*.bin`
// - `target/{debug|release}/fingerprint/...`
// - `fingerprint`: https://github.com/rust-lang/cargo/blob/bc89bffa5987d4af8f71011c7557119b39e44a65/src/cargo/core/compiler/fingerprint/mod.rs#L539-L613
// - `cargo vendor`: https://doc.rust-lang.org/cargo/commands/cargo-vendor.html
#[instrument]
pub fn copy_dir(
    src: impl AsRef<Path> + std::fmt::Debug,
    dst: impl AsRef<Path> + std::fmt::Debug,
) -> Result<()> {
    let src = src.as_ref();
    let dst = dst.as_ref();
    debug!(?src, ?dst, "copying directory recursively");

    // We use this to ensure that we actually only create any given parent
    // once per overall copy operation.
    let created_parents = DashSet::<PathBuf, _>::with_hasher(ahash::RandomState::default());

    WalkDir::new(src)
        .into_iter()
        .par_bridge()
        .try_for_each(|entry| {
            let entry = entry.context("read entry")?;
            let src = entry.path();

            // TODO: handle symlinks properly
            // TODO: does the rust compiler even ever put symlinks in the target directory?
            if !src.is_file() {
                if src.is_symlink() {
                    warn!(?src, "skipping symlink");
                }
                return Ok(());
            }

            // We delay these vars until here so that we can skip allocating/possible error path
            // for things that we don't actually care about.
            let rel = src.strip_prefix(src).context("strip prefix")?;
            let dst = dst.join(rel);

            // We only need to copy the file if it has actually changed
            // (or if it doesn't exist) in the destination.
            //
            // We use the tri-state enum here so that we can avoid creating parent directories
            // if the destination file already exists and is merely outdated.
            match compare_file_sync(src, &dst).context("check if file should be copied")? {
                FileComparison::SourceNewer => {
                    debug!(?src, ?dst, "file needs copy: source newer");
                    copy_file(src, &dst).context("copy file")?;
                }
                FileComparison::DestinationInSync => {
                    debug!(?src, ?dst, "file does not need copy: destination in sync");
                    return Ok(());
                }
                FileComparison::DestinationMissing => {
                    debug!(?src, ?dst, "file needs copy: destination missing");
                    create_parents_of(&created_parents, &dst, rel)
                        .with_context(|| format!("create parents of {rel:?}"))?;
                    copy_file(src, &dst).context("copy file")?;
                }
            }

            debug!(?src, ?dst, "file synchronized");
            Ok(())
        })
}

/// The result of comparing a `src` and `dst` file and their metadata
/// for the purpose of copying `src` to `dst` only if needed.
///
/// The intention of having this enum here is that it allows calling code
/// (currently, the closure inside `copy_dir`) to only create directory parents
/// if the file doesn't already exist.
#[derive(Debug)]
pub enum FileComparison {
    /// The source file is newer; a copy is needed.
    SourceNewer,

    /// The destination file is in sync; no copy is needed.
    DestinationInSync,

    /// The destination file is missing; a copy is needed.
    DestinationMissing,
}

/// Determine whether the file needs to be copied.
///
/// Uses similar heuristics to rsync for fast decisions without having to read the whole file:
/// - if `dst` doesn't exist, copy
/// - if `src` is newer than `dst`, copy
/// - if `src` and `dst` are different sizes, copy
/// - otherwise, assume we don't need to copy.
///
/// In the absence of a clear signal, we assume that the destination is already up to date.
/// We do this because it allows us to skip a fair amount of IO, and if we were wrong
/// then the rust compiler will simply fix our mistake.
///
/// We obviously want to minimize this so that we keep rust compiler rework to a minimum,
/// but this is the safest default.
//
// TODO: This is _fine_ but the best possible way to know if a file has changed is to actually compare
// the content, e.g. comparing using a hash function. We should benchmark and test
// to determine whether this is needed.
#[instrument]
pub fn compare_file_sync(
    src: impl AsRef<Path> + std::fmt::Debug,
    dst: impl AsRef<Path> + std::fmt::Debug,
) -> Result<FileComparison> {
    let src = src.as_ref();
    let dst = dst.as_ref();

    // Statting destination first allows us to skip an exists check.
    match std::fs::metadata(dst) {
        Ok(dst_meta) => {
            let src_meta = std::fs::metadata(src).context("get source metadata")?;

            // If we can't read the actual times from the stat, default to unix epoch
            // so that we don't break the build system.
            //
            // We could promote this to an actual error, but since the rust compiler is ultimately
            // what's going to read this, this is simpler: it'll just transparently rebuild anything
            // that we had to set like this (since the source file will obviously be newer).
            //
            // In other words, this forms a safe "fail closed" system since
            // the rust compiler is the ultimate authority here.
            let dst_mtime = dst_meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let src_mtime = src_meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let dst_size = dst_meta.len();
            let src_size = src_meta.len();
            trace!(
                ?src,
                ?dst,
                ?src_mtime,
                ?dst_mtime,
                ?src_size,
                ?dst_size,
                "compare src to dst"
            );
            if src_size != dst_size || src_mtime > dst_mtime {
                return Ok(FileComparison::SourceNewer);
            }
        }
        Err(err) => {
            trace!(?src, ?dst, ?err, "could not stat destination file");
            return Ok(FileComparison::DestinationMissing);
        }
    }

    // In the absence of a clear signal, we assume that the destination is already up to date.
    // We do this because it allows us to skip a fair amount of IO, and if we were wrong
    // then the rust compiler will simply fix our mistake.
    //
    // We obviously want to minimize this, but this is the safest default.
    trace!(?src, ?dst, "destination file is up to date");
    Ok(FileComparison::DestinationInSync)
}

/// Copy the file from `src` to the root of `dir` with the provided `name`.
#[instrument]
pub fn copy_file_into(
    src: impl AsRef<Path> + std::fmt::Debug,
    dir: impl AsRef<Path> + std::fmt::Debug,
    name: impl AsRef<str> + std::fmt::Debug,
) -> Result<()> {
    let dst = dir.as_ref().join(name.as_ref());
    copy_file(src, &dst)
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
    debug!(?src, ?dst, "copy file");

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
    std::io::copy(&mut src, &mut dst).context("copy file contents")?;

    // Using the `filetime` crate here instead of the stdlib because it's cross platform.
    let mtime = FileTime::from_system_time(src_mtime);
    let atime = FileTime::from_system_time(src_atime);
    set_file_handle_times(&dst, Some(atime), Some(mtime)).context("set destination file times")?;

    // And finally, we have to sync the file to disk so that we are sure it's actually finished copying
    // before we move on. Technically we could leave this up to the FS, but this is safer.
    dst.sync_all().context("sync destination file")?;

    Ok(())
}

/// Create all parents of the provided file inside the provided root.
#[instrument(skip(created_parents))]
pub fn create_parents_of(
    created_parents: &DashSet<PathBuf, ahash::RandomState>,
    destination_root: impl AsRef<Path> + std::fmt::Debug,
    file_rel: impl AsRef<Path> + std::fmt::Debug,
) -> Result<()> {
    let destination_root = destination_root.as_ref();
    let file_rel = file_rel.as_ref();
    let parent_rel = file_rel.parent().ok_or_eyre("get parent directory")?;
    if created_parents.contains(parent_rel) {
        trace!(?parent_rel, "parent directory already exists");
        return Ok(());
    }

    let parent = destination_root.join(parent_rel);
    debug!(?parent, ?parent_rel, "create parent directory");
    std::fs::create_dir_all(&parent)
        .with_context(|| format!("create parent directory {parent:?}"))?;

    // Since we're doing a `create_dir_all`, we know all the parent segments
    // exist after creating this directory.
    for segment in parent_rel.ancestors() {
        trace!(?segment, "marking parent directory as created");
        created_parents.insert(segment.to_path_buf());
    }
    Ok(())
}

/// Buffer the file content from disk.
#[instrument]
pub fn read_buffered(path: impl AsRef<Path> + std::fmt::Debug) -> Result<Vec<u8>> {
    let path = path.as_ref();
    std::fs::read(path).with_context(|| format!("read file: {path:?}"))
}

/// Buffer the file content from disk and parse it as UTF8.
#[instrument]
pub fn read_buffered_utf8(path: impl AsRef<Path> + std::fmt::Debug) -> Result<String> {
    let path = path.as_ref();
    std::fs::read_to_string(path).with_context(|| format!("read file: {path:?}"))
}

/// A file on disk and the hash of its contents.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize, Builder)]
pub struct HashedFile {
    /// The path on disk for the file.
    #[builder(into)]
    pub path: Utf8PathBuf,

    /// The Blake3 hash of the file's content on disk.
    #[builder(into)]
    pub hash: Blake3,
}

impl HashedFile {
    /// Create the output from the provided path on disk.
    #[instrument]
    pub fn read(path: impl Into<Utf8PathBuf> + std::fmt::Debug) -> Result<Self> {
        let path = path.into();
        let hash = Blake3::from_file(&path).with_context(|| format!("hash {path:?}"))?;
        Ok(Self { path, hash })
    }
}
