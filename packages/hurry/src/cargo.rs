use std::{
    ffi::{OsStr, OsString},
    fmt,
    iter::once,
    sync::LazyLock,
};

use color_eyre::{
    Result,
    eyre::{Context, OptionExt, bail},
};
use futures::{StreamExt, TryFutureExt, TryStreamExt, stream};
use itertools::Itertools;
use tap::Pipe;
use tracing::{debug, instrument, trace, warn};

use crate::{
    Locked,
    cache::{FsCache, FsCas, RecordArtifact, RecordKind},
    fs::{DEFAULT_CONCURRENCY, Metadata},
    hash::Blake3,
    path::AbsDirPath,
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
#[instrument]
pub async fn invoke(
    workspace: &Workspace,
    subcommand: impl AsRef<str> + fmt::Debug,
    args: impl IntoIterator<Item = impl AsRef<str>> + fmt::Debug,
) -> Result<()> {
    invoke_env(
        workspace,
        subcommand,
        args,
        std::iter::empty::<(&OsStr, &OsStr)>(),
    )
    .await
}

/// Execute a Cargo subcommand with specified arguments and environment
/// variables.
#[instrument]
pub async fn invoke_env(
    workspace: &Workspace,
    subcommand: impl AsRef<str> + fmt::Debug,
    args: impl IntoIterator<Item = impl AsRef<str>> + fmt::Debug,
    env: impl IntoIterator<Item = (impl AsRef<OsStr>, impl AsRef<OsStr>)> + fmt::Debug,
) -> Result<()> {
    let subcommand = subcommand.as_ref();
    let args = args.into_iter().collect::<Vec<_>>();
    let args = args.iter().map(|a| a.as_ref()).collect::<Vec<_>>();

    trace!(?subcommand, ?args, "invoke cargo");
    let mut cmd = tokio::process::Command::new("cargo");
    cmd.args(once(subcommand).chain(args.iter().copied()));
    cmd.envs(env);

    // Using `CARGO_ENCODED_RUSTFLAGS` allows us to include arguments that might
    // have spaces, such as paths; reference:
    // https://doc.rust-lang.org/cargo/reference/environment-variables.html
    //
    // TODO: This obviously makes it more difficult for users to debug
    // the program; we should probably give users an option to do this
    // or not.
    cmd.env(
        "CARGO_ENCODED_RUSTFLAGS",
        remap_path_prefixes(workspace)
            .into_iter()
            .chain(["-C", "strip=debuginfo"].map(OsString::from))
            .collect_vec()
            .join(OsStr::new("\x1f")),
    );

    // We currently use `-Z remap-cwd-prefix=.` in `remap_path_prefixes`, so
    // need to enable nightly features on stable rust.
    // It's unclear if this is actually what we should be doing longer term.
    cmd.env("RUSTC_BOOTSTRAP", "1");

    let status = cmd
        .spawn()
        .context("could not spawn cargo")?
        .wait()
        .await
        .context("could complete cargo execution")?;
    if status.success() {
        Ok(())
    } else {
        bail!("cargo exited with status: {status}");
    }
}

/// Construct the remap path prefixes for the given workspace.
///
/// Reference: https://doc.rust-lang.org/rustc/command-line-arguments.html#--remap-path-prefix-remap-source-names-in-output
//
// TODO: If we stick with this or something like it, we should probably compute
// this once per workspace instead of over and over.
fn remap_path_prefixes(workspace: &Workspace) -> Vec<OsString> {
    static REMAP_PATH_PREFIX: LazyLock<OsString> =
        LazyLock::new(|| OsString::from("--remap-path-prefix"));
    static REMAP_CWD_PREFIX: LazyLock<OsString> =
        LazyLock::new(|| OsString::from("remap-cwd-prefix=."));
    static UNSTABLE: LazyLock<OsString> = LazyLock::new(|| OsString::from("-Z"));
    fn remap(from: &AbsDirPath, to: impl AsRef<OsStr>) -> OsString {
        [from.as_os_str(), to.as_ref()].join(OsStr::new("="))
    }

    // We put the more general remappings first; from the docs:
    // > When multiple remappings are given and several of them match, the last
    // > matching one is applied.
    let mut remappings = Vec::with_capacity(16);
    remappings.push(UNSTABLE.clone());
    remappings.push(REMAP_CWD_PREFIX.clone());

    if let Some(user_home) = &workspace.user_home {
        remappings.push(REMAP_PATH_PREFIX.clone());
        remappings.push(remap(&user_home, "USER_HOME"));
    }
    if let Some(rustup_home) = &workspace.rustup_home {
        remappings.push(REMAP_PATH_PREFIX.clone());
        remappings.push(remap(&rustup_home, "RUSTUP_HOME"));
    }
    remappings.push(REMAP_PATH_PREFIX.clone());
    remappings.push(remap(&workspace.cargo_home, "CARGO_HOME"));
    remappings.push(REMAP_PATH_PREFIX.clone());
    remappings.push(remap(&workspace.root, "WORKSPACE_ROOT"));
    remappings
}

/// Cache build artifacts from a workspace target directory.
///
/// Enumerates and stores all build artifacts for third-party dependencies in
/// the content-addressable storage (CAS) and updates the cache index.
///
/// For each dependency in the workspace:
/// 1. Enumerate its artifacts (`.rlib`, `.rmeta`, fingerprints, etc.).
/// 2. Store each artifact file in the CAS.
/// 3. Create a cache record mapping dependency key to artifact hashes.
#[instrument(skip(progress))]
pub async fn cache_target_from_workspace(
    cas: &FsCas,
    cache: &FsCache<Locked>,
    target: &ProfileDir<'_, Locked>,
    progress: impl Fn(&Blake3, &Dependency),
) -> Result<()> {
    // The concurrency limits below are currently just vibes from staring at
    // benchmarks; we want to avoid opening too many file handles at a time
    // because that can have a negative effect on performance but we obviously
    // want to have enough running that we saturate the disk.
    //
    // TODO: this currently assumes that the entire `target/` folder doesn't
    // have any _outdated_ data; this may not be correct.
    for (key, dependency) in &target.workspace.dependencies {
        debug!(?key, ?dependency, "caching dependency");
        let artifacts = match target.enumerate_cache_artifacts(dependency).await {
            Ok(artifacts) => artifacts,
            Err(error) => {
                warn!(
                    ?error,
                    "Failed to enumerate cache artifacts for dependency: {dependency}"
                );
                continue;
            }
        };

        if artifacts.is_empty() {
            debug!(
                ?key,
                ?dependency,
                "skipped cache: no artifacts for dependency"
            );
            continue;
        }

        debug!(?key, ?dependency, ?artifacts, "caching artifacts");
        let artifacts = stream::iter(artifacts)
            .map(|artifact| async move {
                let (key, file) = target.store_cas(cas, artifact).await?;
                trace!(?key, ?dependency, ?artifact, "stored artifact");

                let meta = Metadata::from_file(&file)
                    .await
                    .context("read metadata")?
                    .ok_or_eyre("file does not exist")?;
                RecordArtifact::builder()
                    .cas_key(key)
                    .metadata(meta)
                    .target(artifact)
                    .build()
                    .pipe(Result::<_>::Ok)
            })
            .buffer_unordered(DEFAULT_CONCURRENCY)
            .try_collect::<Vec<_>>()
            .await
            .context("cache artifacts")?;

        cache
            .store(RecordKind::Cargo, key, &artifacts)
            .await
            .context("store cache record")?;
        debug!(?key, ?dependency, ?artifacts, "stored cache record");
        progress(key, dependency);
    }

    Ok(())
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
    cas: &FsCas,
    cache: &FsCache<Locked>,
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
    for (key, dependency) in &target.workspace.dependencies {
        debug!(?key, ?dependency, "restoring dependency");
        let record = match cache.get(RecordKind::Cargo, key).await {
            Ok(Some(record)) => record,
            Ok(None) => {
                debug!(?key, ?dependency, "no record found for key");
                continue;
            }
            Err(error) => {
                warn!(
                    ?error,
                    "Failed to read cache record for dependency: {dependency}"
                );
                continue;
            }
        };

        if record.artifacts.is_empty() {
            debug!(
                ?key,
                ?dependency,
                "skipped restore: no artifacts for dependency"
            );
            continue;
        }

        debug!(?key, ?dependency, artifacts = ?record.artifacts, "restoring artifacts");
        stream::iter(&record.artifacts)
            .for_each_concurrent(Some(DEFAULT_CONCURRENCY), |artifact| async move {
                let key = &artifact.cas_key;
                let restore = target
                    .restore_cas(cas, key, &artifact.target)
                    .and_then(|path| async move { artifact.metadata.set_file(&path).await });
                match restore.await {
                    Ok(_) => debug!(?key, ?artifact, ?dependency, "restored file"),
                    Err(error) => {
                        warn!(
                            ?error,
                            ?key,
                            "Failed to restore artifact for dependency: {dependency}"
                        );
                    }
                }
            })
            .await;

        debug!(?key, ?dependency, ?record, "restored cache record");
        progress(key, dependency);
    }

    Ok(())
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
