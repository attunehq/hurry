//! Builds Cargo projects using an optimized cache.
//!
//! Reference:
//! - `docs/DESIGN.md`
//! - `docs/development/cargo.md`

use std::fmt::Debug;

use clap::Args;
use color_eyre::{Result, eyre::Context};
use hurry::{
    cache::{Cache, Cas, FsCache, FsCas},
    cargo::{Profile, Workspace, cache_target_from_workspace, invoke, restore_target_from_cache},
    fs,
    hash::Blake3,
};
use tracing::{error, info, instrument, warn};

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

    /// Backup and restore the `target/` folder by copying the entire
    /// contents and storing it by hash of your `Cargo.lock` file
    /// instead of doing dependency-based restoration.
    #[arg(long = "hurry-simple-caching", default_value_t = false)]
    simple_caching: bool,

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

    let workspace = Workspace::from_argv(&options.argv)
        .await
        .context("open workspace")?;

    if options.simple_caching {
        warn!("Simple caching enabled; simple caches have less reuse across builds.");
        return exec_simple(options, &workspace).await;
    }

    let cas = FsCas::open_default().await.context("open CAS")?;
    let cache = FsCache::open_default().await.context("open cache")?;
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
async fn exec_simple(options: Options, workspace: &Workspace) -> Result<()> {
    let profile = options.profile();
    let cache_root = fs::user_global_cache_path()
        .await
        .context("open cache")?
        .join("simple");

    if !options.skip_restore {
        info!("Restoring target directory from cache");
        let key = Blake3::from_file(workspace.root.join("Cargo.lock")).await?;
        let cache_dir = cache_root.join(key.as_str()).join(profile.as_str());
        let target = workspace.target.join(profile.as_str());

        let has_cache = fs::metadata(&cache_dir)
            .await
            .is_ok_and(|meta| meta.is_some());
        if has_cache {
            let bytes = fs::copy_dir(&cache_dir, &target).await?;
            info!(?bytes, ?key, ?cache_dir, "Restored cache");
        } else {
            info!(?key, "No existing cache found");
        }
    }

    if !options.skip_build {
        info!("Building target directory");
        invoke("build", &options.argv)
            .await
            .context("build with cargo")?;
    }

    if !options.skip_backup {
        info!("Caching target directory");

        let key = Blake3::from_file(workspace.root.join("Cargo.lock")).await?;
        let cache_dir = cache_root.join(key.as_str()).join(profile.as_str());
        let target = workspace.target.join(profile.as_str());
        fs::remove_dir_all(&cache_dir)
            .await
            .context("remove old cache")?;

        // Only back up directories used in third party builds.
        let mut bytes = 0u64;
        for subdir in [".fingerprint", "build", "deps"] {
            bytes += fs::copy_dir(target.join(subdir), cache_dir.join(subdir))
                .await
                .with_context(|| format!("back up {subdir:?}"))?;
        }

        info!(?bytes, ?key, ?cache_dir, "Cached target directory");
    }

    Ok(())
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
        let target = workspace
            .open_profile_locked(&profile)
            .await
            .context("open profile")?;

        let restore = restore_target_from_cache(cas, cache, &target, |key, dependency| {
            info!(
                name = %dependency.name,
                version = %dependency.version,
                target = %dependency.target,
                %key,
                "Restored dependency from cache",
            )
        });
        match restore.await {
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
        let target = workspace
            .open_profile_locked(&profile)
            .await
            .context("open profile")?;

        let backup = cache_target_from_workspace(cas, cache, &target, |key, dependency| {
            info!(
                name = %dependency.name,
                version = %dependency.version,
                target = %dependency.target,
                %key,
                "Updated dependency in cache",
            )
        });
        match backup.await {
            Ok(_) => info!("Cached target directory"),
            Err(error) => warn!(?error, "Failed to cache target"),
        }
    }

    Ok(())
}
