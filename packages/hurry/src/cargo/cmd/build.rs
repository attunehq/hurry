//! Builds Cargo projects using an optimized cache.
//!
//! Reference:
//! - `docs/DESIGN.md`
//! - `docs/development/cargo.md`

use std::fmt::Debug;

use clap::Args;
use color_eyre::{Result, eyre::Context};
use futures::{StreamExt, TryStreamExt, stream};
use tap::{Pipe, TapFallible};
use tracing::{debug, error, info, instrument, trace, warn};

use crate::{
    cache::{Cache, Cas, FsCache, FsCas, Kind},
    cargo::{Profile, invoke, workspace::Workspace},
};

/// Options for `cargo build`.
//
// Hurry options are prefixed with `hurry-` to disambiguate from `cargo` args.
#[derive(Clone, Args, Debug)]
pub struct Options {
    /// Skip backing up the cache.
    #[arg(long = "hurry-skip-backup", default_value_t = false)]
    skip_backup: bool,

    /// Skip the Cargo build, only performing the cache actions.
    #[arg(long = "hurry-skip-build", default_value_t = false)]
    skip_build: bool,

    /// Skip restoring the cache.
    #[arg(long = "hurry-skip-restore", default_value_t = false)]
    skip_restore: bool,

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
    #[instrument(name = "Options::profile")]
    pub fn profile(&self) -> Profile {
        Profile::from_argv(&self.argv)
    }
}

#[instrument]
pub async fn exec(options: Options) -> Result<()> {
    info!("Starting");

    let cas = FsCas::open_default().await.context("open CAS")?;
    let workspace = Workspace::from_argv(&options.argv)
        .await
        .context("open workspace")?;

    let cache = FsCache::open_default(&workspace.root)
        .await
        .context("open cache")?;
    let cache = cache.lock().await.context("lock cache")?;

    // This is split into an inner function so that we can reliably
    // release the lock if it fails.
    let result = exec_inner(options, &cas, &workspace, &cache).await;
    if let Err(err) = cache.unlock().await {
        // This shouldn't happen, but if it does, we should warn users.
        // TODO: figure out a way to recover.
        warn!("unable to release workspace cache lock: {err:?}");
    }

    result
        .inspect(|_| info!("finished"))
        .inspect_err(|error| error!(?error, "failed: {error:#?}"))
}

#[instrument]
async fn exec_inner(
    options: Options,
    cas: impl Cas + Debug + Copy,
    workspace: &Workspace,
    cache: impl Cache + Debug + Copy,
) -> Result<()> {
    let profile = options.profile();

    if !options.skip_restore {
        info!(?cache, "Restoring target directory from cache");
        match restore_target_from_cache(cas, workspace, cache, &profile).await {
            Ok(_) => info!("Restored cache"),
            Err(error) => warn!(?error, "Failed to restore cache"),
        }
    }

    // After restoring the target directory from cache,
    // or if we never had a cache, we need to build it-
    // this is because we currently only cache based on lockfile hash;
    // if the first-party code has changed we'll need to rebuild.
    if !options.skip_build {
        info!("Building target directory");
        invoke("build", &options.argv)
            .await
            .context("build with cargo")?;
    }

    // If we didn't have a cache, we cache the target directory
    // after the build finishes.
    //
    // We don't _always_ cache because since we don't currently
    // cache based on first-party code changes so this would lead to
    // lots of unnecessary copies.
    //
    // TODO: watch and cache the target directory _as the build occurs_
    // rather than having to copy it all at the end.
    if !options.skip_backup {
        info!("Caching built target directory");
        match cache_target_from_workspace(cas, workspace, cache, &profile).await {
            Ok(_) => info!("Cached target directory"),
            Err(error) => warn!(?error, "Failed to cache target"),
        }
    }

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
async fn cache_target_from_workspace(
    cas: impl Cas + Debug + Clone,
    workspace: &Workspace,
    cache: impl Cache + Debug + Clone,
    profile: &Profile,
) -> Result<()> {
    info!("Indexing target folder");
    let target = workspace
        .open_profile_locked(profile)
        .await
        .context("open profile")?;

    // TODO: this currently assumes that the entire `target/` folder
    // doesn't have any _outdated_ data; this may not be correct.
    stream::iter(&workspace.dependencies)
        .then(|(key, dependency)| {
            let target = target.clone();
            async move {
                debug!(?key, ?dependency, "restoring dependency");
                target
                    .enumerate_cache_artifacts(dependency)
                    .await
                    .map(|artifacts| (key, dependency, artifacts))
                    .with_context(|| {
                        format!("enumerate cache artifacts for dependency: {dependency}")
                    })
            }
        })
        .try_for_each_concurrent(Some(10), |(key, dependency, artifacts)| {
            let (cas, target, cache) = (cas.clone(), target.clone(), cache.clone());
            async move {
                debug!(?key, ?dependency, ?artifacts, "caching artifacts");
                stream::iter(&artifacts)
                    .map(|artifact| Ok(artifact))
                    .try_for_each_concurrent(Some(100), |artifact| {
                        let (cas, target) = (cas.clone(), target.clone());
                        async move {
                            let dst = artifact.target.to_path(target.root());
                            cas.store_file(Kind::Cargo, &dst)
                                .await
                                .with_context(|| format!("backup output file: {dst:?}"))
                                .tap_ok(|key| {
                                    trace!(?key, ?dependency, ?artifact, "restored artifact")
                                })
                                .map(drop)
                        }
                    })
                    .await
                    .pipe(|_| {
                        let cache = cache.clone();
                        async move {
                            cache
                                .store(Kind::Cargo, key, &artifacts)
                                .await
                                .context("store cache record")
                                .tap_ok(|_| {
                                    debug!(?key, ?dependency, ?artifacts, "stored cache record")
                                })
                        }
                    })
                    .await
                    .map(|_| {
                        info!(
                            name = %dependency.name,
                            version = %dependency.version,
                            target = %dependency.target,
                            %key,
                            "Updated dependency in cache",
                        )
                    })
            }
        })
        .await
    // for (key, dependency) in &workspace.dependencies {
    //     // Each dependency has several entries we need to back up
    //     // inside the profile directory.
    //     let artifacts = target
    //         .enumerate_cache_artifacts(dependency)
    //         .await
    //         .with_context(|| format!("enumerate cache artifacts for dependency: {dependency}"))?;

    //     for artifact in &artifacts {
    //         let output_file = artifact.target.to_path(target.root());
    //         cas.store_file(Kind::Cargo, &output_file)
    //             .await
    //             .with_context(|| format!("backup output file: {output_file:?}"))?;
    //         trace!(?key, ?dependency, ?artifact, "stored artifact");
    //     }

    //     cache
    //         .store(Kind::Cargo, key, &artifacts)
    //         .await
    //         .context("store cache record")?;
    //     debug!(?key, ?dependency, ?artifacts, "stored cache record");
    //     info!(
    //         name = %dependency.name,
    //         version = %dependency.version,
    //         target = %dependency.target,
    //         %key,
    //         "Updated dependency in cache",
    //     );
    // }

    // Ok(())
}

/// Restore the target directory from the cache.
//
// TODO: Today we unconditionally copy files.
// Implement with copy-on-write when possible;
// otherwise fall back to a symlink.
#[instrument]
async fn restore_target_from_cache(
    cas: impl Cas + Debug + Clone,
    workspace: &Workspace,
    cache: impl Cache + Debug + Clone,
    profile: &Profile,
) -> Result<()> {
    info!("Indexing target folder");
    let target = workspace
        .open_profile_locked(profile)
        .await
        .context("open profile")?;

    // When backing up a `target/` directory, we enumerate
    // the build units before backing up dependencies.
    // But when we restore, we don't have a target directory
    // (or don't trust it), so we can't do that here.
    // Instead, we just enumerate dependencies
    // and try to find some to restore.
    //
    // The concurrency limits below are currently just vibes;
    // we want to avoid opening too many file handles at a time
    // because that can have a negative effect on performance
    // but we obviously want to have enough running that we saturate the disk.
    //
    // TODO: ideally we'd have some kind of dynamic semaphore that sets
    // a budget based on task throughput so that we can ramp up or down
    // concurrency based on the capability and contention of the hardware.
    //
    // TODO: benchmark different approaches and compare to a standard `cp`.
    debug!(dependencies = ?workspace.dependencies, "restoring dependencies");
    stream::iter(&workspace.dependencies)
        .filter_map(|(key, dependency)| {
            let cache = cache.clone();
            async move {
                debug!(?key, ?dependency, "restoring dependency");
                cache
                    .get(Kind::Cargo, key)
                    .await
                    .with_context(|| format!("retrieve cache record for dependency: {dependency}"))
                    .map(|lookup| lookup.map(|record| (key, dependency, record)))
                    .transpose()
            }
        })
        .try_for_each_concurrent(Some(10), |(key, dependency, record)| {
            let (cas, target) = (cas.clone(), target.clone());
            async move {
                debug!(?key, ?dependency, artifacts = ?record.artifacts, "restoring artifacts");
                stream::iter(record.artifacts)
                    .map(|artifact| Ok(artifact))
                    .try_for_each_concurrent(Some(100), |artifact| {
                        let (cas, target) = (cas.clone(), target.clone());
                        async move {
                            let dst = artifact.target.to_path(target.root());
                            cas.get_file(Kind::Cargo, &artifact.hash, &dst)
                                .await
                                .context("extract crate")
                                .tap_ok(|_| {
                                    trace!(?key, ?dependency, ?artifact, "restored artifact")
                                })
                        }
                    })
                    .await
                    .map(|_| {
                        info!(
                            name = %dependency.name,
                            version = %dependency.version,
                            target = %dependency.target,
                            %key,
                            "Restored dependency from cache",
                        )
                    })
            }
        })
        .await
    // for (key, dependency) in &workspace.dependencies {
    //     debug!(?key, ?dependency, "restoring dependency");
    //     let Some(record) = cache
    //         .get(Kind::Cargo, key)
    //         .await
    //         .with_context(|| format!("retrieve cache record for dependency: {dependency}"))?
    //     else {
    //         trace!(?key, ?dependency, "no cache record for dependency");
    //         continue;
    //     };

    //     debug!(?key, ?dependency, artifacts = ?record.artifacts, "restoring artifacts");
    //     for artifact in record.artifacts {
    //         let dst = artifact.target.to_path(target.root());
    //         cas.get_file(Kind::Cargo, &artifact.hash, &dst)
    //             .await
    //             .context("extract backed up crate from cas")?;
    //         if artifact.executable {
    //             fs::set_executable(&dst).await.context("set executable")?;
    //         }
    //         trace!(?key, ?dependency, ?artifact, ?dst, "restored artifact");
    //     }

    //     info!(
    //         name = %dependency.name,
    //         version = %dependency.version,
    //         target = %dependency.target,
    //         %key,
    //         "Restored dependency from cache",
    //     );
    // }

    // Ok(())
}
