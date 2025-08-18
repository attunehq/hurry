//! Builds Cargo projects using an optimized cache.
//!
//! Reference:
//! - `docs/DESIGN.md`
//! - `docs/development/cargo.md`

use std::time::Instant;

use clap::Args;
use color_eyre::{Result, eyre::Context};
use tracing::{info, instrument, warn};

use crate::{
    cargo::{
        invoke,
        workspace::{Cache, Locked, Workspace},
    },
    fs,
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
    let key = fs::hash_file_content(workspace.dir().join("Cargo.lock"))
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
    fs::copy_dir(cache.root(), workspace.target())
}

/// Cache the target directory to the cache.
#[instrument(skip_all)]
fn cache_target_from_workspace(workspace: &Workspace, cache: &Cache<Locked>) -> Result<()> {
    fs::copy_dir(workspace.target(), cache.root())
}
