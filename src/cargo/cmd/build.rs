//! Builds Cargo projects using an optimized cache.
//!
//! Reference:
//! - `docs/DESIGN.md`
//! - `docs/development/cargo.md`

use std::time::Instant;

use clap::Args;
use color_eyre::{Result, eyre::Context};
use tracing::{debug, info, instrument, warn};

use crate::{
    cargo::{
        Profile, invoke,
        workspace::{Cache, CacheRecord, Locked, Workspace},
    },
    cas::Cas,
    fs,
};

/// Options for `cargo build`
#[derive(Clone, Args, Debug)]
pub struct Options {
    /// Force updating the cache even if it already exists.
    #[arg(long, default_value_t = false)]
    force_cache_update: bool,

    /// These arguments are passed directly to `cargo build` as provided.
    #[arg(
        num_args = ..,
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "ARGS",
    )]
    argv: Vec<String>,
}

impl Options {
    /// Get the profile specified by the user.
    pub fn profile(&self) -> Profile {
        Profile::from_argv(&self.argv)
    }
}

#[instrument]
pub fn exec(options: Options) -> Result<()> {
    let start = Instant::now();
    let cas = Cas::open_default().context("open cas")?;
    let workspace = Workspace::from_argv(&options.argv).context("open workspace")?;

    let cache = workspace.open_cache().context("open cache")?;
    let cache = cache.lock().context("lock cache")?;

    // This is split into an inner function so that we can reliably
    // release the lock if it fails.
    let result = exec_inner(start, options, &cas, &workspace, &cache);
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
    cas: &Cas,
    workspace: &Workspace,
    cache: &Cache<'_, Locked>,
) -> Result<()> {
    let profile = options.profile();

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
    if !cache_exists || options.force_cache_update {
        info!("Caching built target directory");
        match cache_target_from_workspace(cas, &workspace, &cache, &profile) {
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
    warn!("restoring cache is currently a no-op");
    Ok(())
}

/// Cache the target directory to the cache.
///
/// When **restoring** `target/` in the future, we need to be able to restore
/// from scratch without an existing `target/` directory. This is for two
/// reasons: first, the project may actually be fresh, with no `target/`
/// at all. Second, the `target/` may be outdated.
/// This means that we can't rely on the functionality that `cargo`
/// would typically provide for us inside `target/`, such as `.fingerprint`
/// or `.d` files to find dependencies or hashes.
///
/// Of course, when **caching** `target/`, we can (and indeed must) assume
/// that the contents of `target/` are correct and trustworthy. But we must
/// copy all the data necessary to recreate the important parts of `target/`
/// in a future fresh start environment.
///
/// ## Third party crates
///
/// The backup process enumerates dependencies (third party crates)
/// in the project. For each discovered dependency, it:
/// - Finds the built `.rlib` and `.rmeta` files
/// - Finds tertiary files like `.fingerprint` etc
/// - Stores the files in the CAS in such a way that they can be found
///   using only data inside `Cargo.lock` in the future.
#[instrument]
fn cache_target_from_workspace(
    cas: &Cas,
    workspace: &Workspace,
    cache: &Cache<Locked>,
    profile: &Profile,
) -> Result<()> {
    let target = workspace
        .open_profile(profile)
        .with_context(|| format!("open profile: {profile:?}"))
        .and_then(|target| target.lock().context("lock profile: {profile:?}"))?;

    let units = target
        .enumerate_buildunits()
        .context("enumerate build units")?;
    debug!(?units, "enumerated build units");
    for unit in units {
        if let Some(key) = &unit.dependency_key {
            let output_file = unit.output.path(&target);
            cas.copy_from(&output_file, unit.output.hash())
                .context("backup output file")?;

            let record = CacheRecord::builder()
                .dependency_key(key)
                .hash(unit.output.hash())
                .target(unit.output.path_rel())
                .build();
            cache.store(&record).context("store cache record")?;
            debug!(?unit, ?record, "stored cache record");
        } else {
            debug!(?unit, "skipped unit: no dependency");
        }
    }

    Ok(())
}
