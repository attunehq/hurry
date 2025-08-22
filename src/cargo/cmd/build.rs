//! Builds Cargo projects using an optimized cache.
//!
//! Reference:
//! - `docs/DESIGN.md`
//! - `docs/development/cargo.md`

use clap::Args;
use color_eyre::{Result, eyre::Context};
use tracing::{debug, error, info, instrument, trace, warn};

use crate::{
    cargo::{
        CacheRecord, Profile, invoke,
        workspace::{Cache, Locked, Workspace},
    },
    cas::Cas,
};

/// Options for `cargo build`
#[derive(Clone, Args, Debug)]
pub struct Options {
    /// Skip backing up the cache.
    #[arg(long, default_value_t = false)]
    skip_backup: bool,

    /// Skip the Cargo build, only performing the cache actions.
    #[arg(long, default_value_t = false)]
    skip_build: bool,

    /// Skip restoring the cache.
    #[arg(long, default_value_t = false)]
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
    pub fn profile(&self) -> Profile {
        Profile::from_argv(&self.argv)
    }
}

#[instrument(name = "cargo_build")]
pub fn exec(options: Options) -> Result<()> {
    info!("Starting");

    let cas = Cas::open_default().context("open cas")?;
    let workspace = Workspace::from_argv(&options.argv).context("open workspace")?;

    let cache = workspace.open_cache().context("open cache")?;
    let cache = cache.lock().context("lock cache")?;

    // This is split into an inner function so that we can reliably
    // release the lock if it fails.
    let result = exec_inner(options, &cas, &workspace, &cache);
    if let Err(err) = cache.unlock() {
        // This shouldn't happen, but if it does, we should warn users.
        // TODO: figure out a way to recover.
        warn!("unable to release workspace cache lock: {err:?}");
    }

    result
        .inspect(|_| info!("finished"))
        .inspect_err(|error| error!(?error, "failed: {error:#?}"))
}

fn exec_inner(
    options: Options,
    cas: &Cas,
    workspace: &Workspace,
    cache: &Cache<'_, Locked>,
) -> Result<()> {
    let profile = options.profile();

    if !options.skip_restore {
        info!(?cache, "Restoring target directory from cache");
        match restore_target_from_cache(cas, &workspace, &cache, &profile) {
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
        invoke(&workspace, "build", &options.argv).context("build with cargo")?;
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
        match cache_target_from_workspace(cas, &workspace, &cache, &profile) {
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
#[instrument(skip_all, fields(%cas, %workspace, %cache, %profile))]
fn cache_target_from_workspace(
    cas: &Cas,
    workspace: &Workspace,
    cache: &Cache<Locked>,
    profile: &Profile,
) -> Result<()> {
    let target = workspace
        .open_profile(profile)
        .context("open profile")
        .and_then(|target| target.lock().context("lock profile"))?;

    // TODO: this currently assumes that the entire `target/` folder
    // doesn't have any _outdated_ data; this is _probably_ not correct.
    for (key, dependency) in &workspace.dependencies {
        // Each dependency has several entries we need to back up
        // inside the profile directory.
        let artifacts = target
            .enumerate_cache_artifacts(dependency)
            .with_context(|| format!("enumerate cache artifacts for dependency: {dependency}"))?;

        for artifact in &artifacts {
            let output_file = target.root().join(&artifact.target);
            cas.copy_from(&output_file, &artifact.hash)
                .with_context(|| format!("backup output file: {output_file:?}"))?;
            trace!(?key, ?dependency, ?artifact, "stored artifact");
        }

        let record = CacheRecord::builder()
            .artifacts(artifacts)
            .dependency_key(key)
            .build();
        cache.store(&record).context("store cache record")?;
        debug!(?key, ?dependency, ?record, "stored cache record");
        info!(
            name = %dependency.name,
            version = %dependency.version,
            target = %dependency.target,
            %key,
            "Updated dependency in cache",
        );
    }

    Ok(())
}

/// Restore the target directory from the cache.
//
// TODO: Today we unconditionally copy files.
// Implement with copy-on-write when possible;
// otherwise fall back to a symlink.
#[instrument(skip_all, fields(%cas, %workspace, %cache, %profile))]
fn restore_target_from_cache(
    cas: &Cas,
    workspace: &Workspace,
    cache: &Cache<Locked>,
    profile: &Profile,
) -> Result<()> {
    let target = workspace
        .open_profile(profile)
        .context("open profile")
        .and_then(|target| target.lock().context("lock profile"))?;

    // When backing up a `target/` directory, we enumerate
    // the build units before backing up dependencies.
    // But when we restore, we don't have a target directory
    // (or don't trust it), so we can't do that here.
    // Instead, we just enumerate dependencies
    // and try to find some to restore.
    for (key, dependency) in &workspace.dependencies {
        let Some(record) = cache
            .retrieve(key)
            .with_context(|| format!("retrieve cache record for dependency: {dependency}"))?
        else {
            trace!(?key, ?dependency, "no cache record for dependency");
            continue;
        };

        for artifact in record.artifacts {
            let dst = target.root().join(&artifact.target);
            cas.extract_to(&artifact.hash, &dst)
                .context("extract backed up crate from cas")?;
            trace!(?key, ?dependency, ?artifact, ?dst, "restored artifact");
        }

        info!(
            name = %dependency.name,
            version = %dependency.version,
            target = %dependency.target,
            %key,
            "Restored dependency from cache",
        );
    }

    Ok(())
}
