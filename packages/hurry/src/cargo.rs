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

mod profile;
mod workspace;
mod dependency;
mod metadata;

pub use profile::*;
pub use workspace::*;
pub use dependency::*;
pub use metadata::*;

/// Invoke a cargo subcommand with the given arguments.
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

/// Restore the target directory from the cache.
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

/// Parse the value of an argument flag from `argv`.
///
/// Handles cases like:
/// - `--flag value`
/// - `--flag=value`
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