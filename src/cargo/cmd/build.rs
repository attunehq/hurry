//! Builds Cargo projects using an optimized cache.
//!
//! Reference:
//! - `docs/DESIGN.md`
//! - `docs/development/cargo.md`

use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    time::{Instant, SystemTime},
};

use clap::Args;
use color_eyre::{
    Result,
    eyre::{Context, OptionExt},
};
use dashmap::DashSet;
use filetime::{FileTime, set_file_handle_times};
use rayon::iter::{ParallelBridge, ParallelIterator};
use tracing::{debug, info, instrument, trace, warn};
use walkdir::WalkDir;

use crate::{
    cargo::{
        invoke,
        workspace::{Cache, Locked, Workspace},
    },
    hash_file_content,
};

/// Options for `cargo build`
#[derive(Clone, Args, Debug)]
pub struct Options {
    /// These arguments are passed directly to `cargo build` as provided.
    #[arg(
        num_args = ..,
        trailing_var_arg = true,
        allow_hyphen_values = true,
    )]
    argv: Vec<String>,
}

#[instrument]
pub fn exec(options: Options) -> Result<()> {
    let start = Instant::now();
    let workspace = Workspace::current().context("open workspace")?;

    // TODO: we need to separate various cargo flags in the cache
    // - Release vs debug builds
    // - Different sets of features
    // - Different targets (linux/x86_64 vs darwin/aarch64, etc)
    // - Probably more
    //
    // TODO: we currently assume one cache key is good enough for the whole
    // workspace, but this is definitely not correct. We'll need to heavily use
    // the CAS to cache individual items from different workspaces. In reality,
    // the "cache" as a concept may go away in favor of pure CAS
    // (maybe separated by build tool).
    let key = hash_file_content(workspace.dir().join("Cargo.lock"))
        .with_context(|| format!("hash `Cargo.lock` inside {}", workspace.dir()))
        .map(hex::encode)?;
    let cache = workspace
        .open_cache(&key)
        .with_context(|| format!("open cache for key {key}"))?;
    let cache = cache
        .lock()
        .with_context(|| format!("lock cache for key {key}"))?;

    // This is split into an inner function so that we can reliably
    // release the lock if it fails.
    let result = exec_inner(start, options, &workspace, &cache);
    if let Err(err) = cache.unlock() {
        // This shouldn't happen, but if it does, we should warn users.
        // TODO: figure out a way to recover.
        warn!("unable to release workspace cache lock: {err:?}");
    }

    let elapsed = start.elapsed();
    result
        .inspect(|_| info!(?elapsed, "cargo build completed successfully"))
        .inspect_err(|_| warn!(?elapsed, "cargo build failed"))
}

fn exec_inner(
    start: Instant,
    options: Options,
    workspace: &Workspace,
    cache: &Cache<'_, Locked>,
) -> Result<()> {
    let cache_exists = !cache.is_empty().context("check if cache is empty")?;
    if cache_exists {
        info!(?cache, "Restoring target directory from cache");
        match restore_target_from_cache(&workspace, &cache) {
            Ok(_) => info!(elapsed = ?start.elapsed(), "restored cache"),
            Err(err) => warn!(elapsed = ?start.elapsed(), ?err, "failed to restore cache"),
        }
    }

    // After restoring the target directory from cache,
    // or if we never had a cache, we need to build it-
    // this is because we currently only cache based on lockfile hash;
    // if the first-party code has changed we'll need to rebuild.
    info!("Building target directory");
    invoke(&workspace, "build", &options.argv).context("build with cargo")?;

    // If we didn't have a cache, we cache the target directory
    // after the build finishes.
    //
    // We don't _always_ cache because since we don't currently
    // cache based on first-party code changes so this would lead to
    // lots of unnecessary copies.
    //
    // TODO: watch and cache the target directory _as the build occurs_
    // rather than having to copy it all at the end.
    if !cache_exists {
        info!("Caching built target directory");
        match cache_target_from_workspace(&workspace, &cache) {
            Ok(_) => info!(elapsed = ?start.elapsed(), "cached target directory"),
            Err(err) => warn!(elapsed = ?start.elapsed(), ?err, "failed to cache target directory"),
        }
    }

    Ok(())
}

/// Restore the target directory from the cache.
//
// TODO: Today we unconditionally copy the contents.
// Implement with copy-on-write when possible;
// otherwise fall back to a symlink.
#[instrument(skip_all)]
fn restore_target_from_cache(workspace: &Workspace, cache: &Cache<Locked>) -> Result<()> {
    copy_dir(cache.root(), workspace.target())
}

/// Cache the target directory to the cache.
#[instrument(skip_all)]
fn cache_target_from_workspace(workspace: &Workspace, cache: &Cache<Locked>) -> Result<()> {
    copy_dir(workspace.target(), cache.root())
}

/// Recursively copy a directory in parallel.
///
/// - Preserves file `mtime` and `atime`.
/// - Uses `mtime` and file size (similar to rsync) to determine if the file needs to be synced.
//
// TODO: what does the rust compiler/cargo do here? we should be at least as correct as it is.
// maybe there's some work we can reuse? References:
// - `target/{debug|release}/incremental/.../.../*.bin`
// - `target/{debug|release}/fingerprint/...`
// - `fingerprint`: https://github.com/rust-lang/cargo/blob/bc89bffa5987d4af8f71011c7557119b39e44a65/src/cargo/core/compiler/fingerprint/mod.rs#L539-L613
// - `cargo vendor`: https://doc.rust-lang.org/cargo/commands/cargo-vendor.html
#[instrument]
fn copy_dir(
    source: impl AsRef<Path> + std::fmt::Debug,
    destination: impl AsRef<Path> + std::fmt::Debug,
) -> Result<()> {
    let source = source.as_ref();
    let destination = destination.as_ref();
    debug!(?source, ?destination, "copying directory recursively");

    // We use this to ensure that we actually only create any given parent
    // once per overall copy operation.
    let created_parents = DashSet::<PathBuf, _>::with_hasher(ahash::RandomState::default());

    WalkDir::new(source)
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
            let rel = src.strip_prefix(source).context("strip prefix")?;
            let dst = destination.join(rel);

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
                    create_parents_of(&created_parents, destination, rel)
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
enum FileComparison {
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
fn compare_file_sync(src: &Path, dst: &Path) -> Result<FileComparison> {
    // Statting destination first allows us to skip an exists check.
    match fs::metadata(dst) {
        Ok(dst_meta) => {
            let src_meta = fs::metadata(src).context("get source metadata")?;

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
fn copy_file(src: &Path, dst: &Path) -> Result<()> {
    debug!(?src, ?dst, "copy file");

    // Manually opening the source file allows us to access the stat info directly,
    // without an additional syscall to stat directly.
    let mut src = File::open(src).context("open source file")?;
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
    let mut dst = fs::OpenOptions::new()
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

#[instrument(skip(created_parents))]
fn create_parents_of(
    created_parents: &DashSet<PathBuf, ahash::RandomState>,
    destination_root: &Path,
    file_rel: &Path,
) -> Result<()> {
    let parent_rel = file_rel.parent().ok_or_eyre("get parent directory")?;
    if created_parents.contains(parent_rel) {
        trace!(?parent_rel, "parent directory already exists");
        return Ok(());
    }

    let parent = destination_root.join(parent_rel);
    debug!(?parent, ?parent_rel, "create parent directory");
    fs::create_dir_all(&parent).with_context(|| format!("create parent directory {parent:?}"))?;

    // Since we're doing a `create_dir_all`, we know all the parent segments
    // exist after creating this directory.
    for segment in parent_rel.ancestors() {
        trace!(?segment, "marking parent directory as created");
        created_parents.insert(segment.to_path_buf());
    }
    Ok(())
}
