//! Builds Cargo projects using an optimized cache.
//!
//! Reference:
//! - `docs/DESIGN.md`
//! - `docs/development/cargo.md`

use std::fmt::Debug;

use clap::Args;
use color_eyre::{Result, eyre::Context};
use hurry::{
    Locked,
    cache::{FsCache, FsCas},
    cargo::{self, Profile, Workspace, cache_target_from_workspace, restore_target_from_cache},
    fs,
    path::TryJoinWith,
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
async fn exec_inner(
    options: Options,
    cas: &FsCas,
    workspace: &Workspace,
    cache: &FsCache<Locked>,
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
                name = %dependency.package_name,
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
        // Ensure that the Hurry build cache within `target` is created for the
        // invocation, and that the build is run with the Hurry wrapper.
        let cargo_invocation_id = uuid::Uuid::new_v4();
        fs::create_dir_all(
            &workspace
                .target
                .try_join_dirs(["hurry", "invocations", &cargo_invocation_id.to_string()])
                .context("invalid cargo invocation cache dirname")?,
        )
        .await
        .context("create build-scoped Hurry cache")?;
        let cwd = std::env::current_dir().context("load build root")?;

        info!("Building target directory");
        // TODO: Handle the case where the user has already defined a
        // `RUSTC_WRAPPER` (e.g. if they're using `sccache`).
        //
        // TODO: Figure out how to properly distribute the wrapper. Maybe we'll
        // embed it into the binary, and write it out? See example[^1].
        //
        // [^1]: https://zameermanji.com/blog/2021/6/17/embedding-a-rust-binary-in-another-rust-binary/
        cargo::invoke_env(
            workspace,
            "build",
            &options.argv,
            [
                ("RUSTC_WRAPPER", "hurry-cargo-rustc-wrapper"),
                (
                    "HURRY_CARGO_INVOCATION_ID",
                    &cargo_invocation_id.to_string(),
                ),
                ("HURRY_CARGO_INVOCATION_ROOT", &cwd.to_string_lossy()),
            ],
        )
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
                name = %dependency.package_name,
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
