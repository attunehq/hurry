use std::{fmt::Debug as StdDebug, iter::once};

use color_eyre::{
    Result,
    eyre::{Context, bail},
};
use futures::{StreamExt, TryStreamExt, stream};
use itertools::Itertools;
use tap::{Pipe, TapFallible};
use tracing::{debug, instrument, trace, warn};

use crate::{
    Locked,
    cache::{Cache, Cas, Kind},
    hash::Blake3,
};

mod dependency;
mod metadata;
mod profile;
mod workspace;

pub use dependency::*;
pub use metadata::*;
pub use profile::*;
pub use workspace::*;

/// Execute a Cargo subcommand with specified arguments.
#[instrument(skip_all)]
pub async fn invoke(
    subcommand: impl AsRef<str>,
    args: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<()> {
    let subcommand = subcommand.as_ref();
    let args = args.into_iter().collect::<Vec<_>>();
    let args = args.iter().map(|a| a.as_ref()).collect::<Vec<_>>();

    let mut cmd = tokio::process::Command::new("cargo");
    cmd.args(once(subcommand).chain(args.iter().copied()));
    let status = cmd
        .spawn()
        .context("could not spawn cargo")?
        .wait()
        .await
        .context("could complete cargo execution")?;
    if status.success() {
        trace!(?subcommand, ?args, "invoke cargo");
        Ok(())
    } else {
        bail!("cargo exited with status: {status}");
    }
}

/// Cache build artifacts from a workspace target directory.
///
/// Enumerates and stores all build artifacts for third-party dependencies
/// in the content-addressable storage (CAS) and updates the cache index.
///
/// ## Process
/// For each dependency in the workspace:
///   1. Enumerate its artifacts (`.rlib`, `.rmeta`, fingerprints, etc.)
///   2. Store each artifact file in the CAS
///   3. Create a cache record mapping dependency key to artifact hashes
///
/// ## Caching Strategy
/// The cache assumes the current `target/` directory is valid and trustworthy.
/// It stores sufficient metadata to recreate the dependency artifacts in a
/// fresh environment without relying on Cargo's internal state files.
///
#[instrument(skip(progress))]
pub async fn cache_target_from_workspace(
    cas: impl Cas + StdDebug + Clone,
    cache: impl Cache + StdDebug + Clone,
    target: &ProfileDir<'_, Locked>,
    progress: impl Fn(&Blake3, &Dependency) + Clone,
) -> Result<()> {
    // The concurrency limits below are currently just vibes;
    // we want to avoid opening too many file handles at a time
    // because that can have a negative effect on performance
    // but we obviously want to have enough running that we saturate the disk.
    //
    // TODO: this currently assumes that the entire `target/` folder
    // doesn't have any _outdated_ data; this may not be correct.
    stream::iter(&target.workspace.dependencies)
        .filter_map(|(key, dependency)| {
            let target = target.clone();
            async move {
                debug!(?key, ?dependency, "restoring dependency");
                target
                    .enumerate_cache_artifacts(dependency)
                    .await
                    .map(|artifacts| (key, dependency, artifacts))
                    .tap_err(|err| {
                        warn!(
                            ?err,
                            "Failed to enumerate cache artifacts for dependency: {dependency}"
                        )
                    })
                    .ok()
                    .map(Ok)
            }
        })
        .try_for_each_concurrent(Some(10), |(key, dependency, artifacts)| {
            let (cas, target, cache, progress) =
                (cas.clone(), target.clone(), cache.clone(), progress.clone());
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
                    .map(|_| progress(key, dependency))
            }
        })
        .await
}

/// Restore build artifacts from cache to the target directory.
///
/// Retrieves cached artifacts for all workspace dependencies and extracts
/// them to their proper locations in the target directory. Only restores
/// dependencies that have cached records available.
///
/// ## Process
/// For each dependency in the workspace:
///   1. Look up cached artifacts by dependency key
///   2. Extract each artifact from CAS to target location
///   3. Call progress callback when dependency is complete
#[instrument(skip(progress))]
pub async fn restore_target_from_cache(
    cas: impl Cas + StdDebug + Clone,
    cache: impl Cache + StdDebug + Clone,
    target: &ProfileDir<'_, Locked>,
    progress: impl Fn(&Blake3, &Dependency) + Clone,
) -> Result<()> {
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
    debug!(dependencies = ?target.workspace.dependencies, "restoring dependencies");
    stream::iter(&target.workspace.dependencies)
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
            let (cas, target, progress) = (cas.clone(), target.clone(), progress.clone());
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
                    .map(|_| progress(key, dependency))
            }
        })
        .await
}

/// Extract the value of a command line flag from argument vector.
///
/// Supports both space-separated (`--flag value`) and equals-separated
/// (`--flag=value`) flag formats. Returns the first matching value found.
///
/// ## Examples
/// ```not_rust
/// let args = vec!["--profile".to_string(), "release".to_string()];
/// assert_eq!(read_argv(&args, "--profile"), Some("release"));
///
/// let args = vec!["--profile=debug".to_string()];
/// assert_eq!(read_argv(&args, "--profile"), Some("debug"));
/// ```
#[instrument]
pub fn read_argv<'a>(argv: &'a [String], flag: &str) -> Option<&'a str> {
    debug_assert!(flag.starts_with("--"), "flag {flag:?} must start with `--`");
    argv.iter().tuple_windows().find_map(|(a, b)| {
        let (a, b) = (a.trim(), b.trim());

        // Handle the `--flag value` case, where the flag and its value
        // are distinct entries in `argv`.
        if a == flag {
            return Some(b);
        }

        // Handle the `--flag=value` case, where the flag and its value
        // are the same entry in `argv`.
        //
        // Due to how tuple windows work, this case could be in either
        // `a` or `b`. If `b` is the _last_ element in `argv`,
        // it won't be iterated over again as a future `a`,
        // so we have to check both.
        //
        // Unfortunately this leads to rework as all but the last `b`
        // will be checked again as a future `a`, but since `argv`
        // is relatively small this shouldn't be an issue in practice.
        //
        // Just in case I've thrown an `instrument` call on the function,
        // but this is extremely unlikely to ever be an issue.
        for v in [a, b] {
            if let Some((a, b)) = v.split_once('=')
                && a == flag
            {
                return Some(b);
            }
        }

        None
    })
}
