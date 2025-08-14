use std::{
    fs::{self, File, FileTimes},
    path::Path,
};

use clap::Args;
use color_eyre::{
    Result,
    eyre::{Context, OptionExt},
};
use rayon::iter::{ParallelBridge, ParallelIterator};
use tracing::{info, instrument, trace, warn};
use walkdir::WalkDir;

use crate::cargo::{
    cache::{Locked, WorkspaceCache},
    invoke,
    workspace::Workspace,
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
    let workspace = Workspace::open().context("open workspace")?;

    // TODO: we need to separate various cargo flags in the cache
    // - Release vs debug builds
    // - Different sets of features
    // - Different targets (linux/x86_64 vs darwin/aarch64, etc)
    // - Probably more
    let workspace_cache = WorkspaceCache::new(&workspace.metadata.workspace_root)
        .context("create workspace cache")?;

    // TODO: only lock if we need to write to the cache.
    // Probably we need to move to a "staging area" and a "committed area"
    // for the cache.
    //
    // TODO: Only log that we're waiting on the lock if it takes longer than
    // a certain amount of time.
    info!("waiting on workspace cache lock");
    let workspace_cache = workspace_cache.lock().context("lock workspace cache")?;

    // This is split into an inner function so that we can reliably
    // release the lock if it fails.
    let result = exec_inner(options, workspace, &workspace_cache);
    if let Err(err) = workspace_cache.unlock() {
        // This shouldn't happen, but if it does, we should warn users.
        // TODO: figure out a way to recover.
        warn!("unable to release workspace cache lock: {err:?}");
    }

    result
}

fn exec_inner(
    options: Options,
    workspace: Workspace,
    cache: &WorkspaceCache<Locked>,
) -> Result<()> {
    // If we have a `target` directory,
    // we currently assume that we have already built this lockfile
    // and restore it from the cache unconditionally.
    let cache_exists = cache.target.exists();
    if cache_exists {
        info!("target directory exists, skipping build");
        info!("Restoring target directory from cache");
        restore_target_from_cache(&workspace, cache)
            .context("restore target directory from cache")?;
    }

    // After restoring the target directory from cache,
    // or if we never had a cache, we need to build it-
    // this is because we currently only cache based on lockfile hash;
    // if the first-party code has changed we'll need to rebuild.
    info!("Building target directory");
    invoke("build", &options.argv).context("build with cargo")?;

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
        cache_target_from_workspace(&workspace, cache)?;
    }

    Ok(())
}

/// Restore the target directory from the cache.
//
// TODO: Today we unconditionally copy the contents.
// Implement with copy-on-write when possible;
// otherwise fall back to a symlink.
#[instrument(skip_all)]
fn restore_target_from_cache(
    workspace: &Workspace,
    workspace_cache: &WorkspaceCache<Locked>,
) -> Result<()> {
    copy_dir(
        &workspace_cache.target,
        workspace.metadata.target_directory.as_std_path(),
    )
}

/// Cache the target directory to the cache.
#[instrument(skip_all)]
fn cache_target_from_workspace(
    workspace: &Workspace,
    workspace_cache: &WorkspaceCache<Locked>,
) -> Result<()> {
    copy_dir(
        workspace.metadata.target_directory.as_std_path(),
        &workspace_cache.target,
    )
}

/// Recursively copy a directory in parallel.
///
/// File times are preserved during the copy operation.
#[instrument]
fn copy_dir(source: &Path, destination: &Path) -> Result<()> {
    info!(?source, ?destination, "copying directory recursively");
    WalkDir::new(source)
        .into_iter()
        .par_bridge()
        .try_for_each(|entry| {
            let entry = entry.context("read entry")?;
            let src = entry.path();
            let rel = src.strip_prefix(source).context("strip prefix")?;
            let dst = destination.join(rel);

            // TODO: handle symlinks properly
            if src.is_symlink() {
                warn!(?src, "skipping symlink");
            } else if entry.path().is_dir() {
                // We do nothing here, because we already create parent directories
                // when copying files.
            } else {
                // TODO: only create parents that haven't already been created.
                let parent = dst.parent().ok_or_eyre("get parent directory")?;
                trace!(?src, ?dst, ?parent, "create parent directory");
                fs::create_dir_all(parent)
                    .with_context(|| format!("create parent directory {parent:?}"))?;

                // TODO: only copy if the file content has changed.
                trace!(?src, ?dst, "copy file");
                fs::copy(src, &dst).with_context(|| format!("copy {src:?} to {dst:?}"))?;

                trace!(?src, ?dst, "set metadata on destination");
                let src_meta = entry.path().metadata().context("get source metadata")?;
                let dst_meta = File::options().write(true).open(&dst)?;
                let times = FileTimes::new()
                    .set_accessed(src_meta.accessed()?)
                    .set_modified(src_meta.modified()?);
                dst_meta
                    .set_times(times)
                    .context("update destination file metadata")?;
                dst_meta
                    .sync_all()
                    .context("sync destination file metadata")?;
            }

            Ok(())
        })
}
