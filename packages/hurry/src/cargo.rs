use std::{
    ffi::OsStr,
    fmt,
    iter::once,
    process::{Output, Stdio},
};

use color_eyre::{
    Result,
    eyre::{Context, bail},
};
use itertools::Itertools;
use tokio::process::Child;
use tracing::{instrument, trace};

mod build_args;
mod build_plan;
mod build_script;
mod cache;
mod dep_info;
mod dependency;
mod path;
mod profile;
mod rustc;
mod unit_graph;
mod workspace;

pub use build_args::{CargoBuildArgument, CargoBuildArguments, ColorWhen, MessageFormat};
pub use build_plan::BuildPlan;
pub use build_script::{BuildScriptOutput, RootOutput};
pub use cache::{CargoCache, Restored, SaveProgress, save_artifacts};
pub use dep_info::{DepInfo, DepInfoLine};
pub use dependency::{Dependency, DependencyBuild, Optimizations};
pub use path::QualifiedPath;
pub use profile::Profile;
pub use rustc::{RustcArgument, RustcArguments, RustcMetadata};
pub use unit_graph::{
    CargoCompileMode, UnitGraph, UnitGraphDependency, UnitGraphProfile,
    UnitGraphProfilePanicStrategy, UnitGraphUnit,
};
pub use workspace::{ArtifactKey, ArtifactPlan, BuildScriptDirs, BuiltArtifact, Workspace};

/// Execute Cargo without a subcommand with specified arguments.
#[instrument]
pub async fn invoke_plain(
    args: impl IntoIterator<Item = impl AsRef<str>> + fmt::Debug,
) -> Result<()> {
    let args = args.into_iter().collect::<Vec<_>>();
    let args = args.iter().map(|a| a.as_ref()).collect::<Vec<_>>();

    trace!(?args, "invoke cargo");
    let mut cmd = tokio::process::Command::new("cargo");
    cmd.args(args.iter().copied());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());

    let status = cmd
        .spawn()
        .context("could not spawn cargo")?
        .wait()
        .await
        .context("could not complete cargo execution")?;
    if status.success() {
        Ok(())
    } else {
        bail!("cargo exited with status: {}", status);
    }
}

/// Execute a Cargo subcommand with specified arguments.
#[instrument]
pub async fn invoke(
    subcommand: impl AsRef<str> + fmt::Debug,
    args: impl IntoIterator<Item = impl AsRef<str>> + fmt::Debug,
) -> Result<()> {
    let status = invoke_with(
        subcommand,
        args,
        [] as [(&OsStr, &OsStr); 0],
        Handles {
            stdout: Stdio::inherit(),
            stderr: Stdio::inherit(),
        },
    )
    .await?
    .wait()
    .await
    .context("could complete cargo execution")?;
    if status.success() {
        Ok(())
    } else {
        bail!("cargo exited with status: {}", status);
    }
}

/// Execute a Cargo subcommand with specified arguments and environment
/// variables, capturing and returning the output.
pub async fn invoke_output(
    subcommand: impl AsRef<str> + fmt::Debug,
    args: impl IntoIterator<Item = impl AsRef<str>> + fmt::Debug,
    env: impl IntoIterator<Item = (impl AsRef<OsStr>, impl AsRef<OsStr>)> + fmt::Debug,
) -> Result<Output> {
    let child = invoke_with(
        subcommand,
        args,
        env,
        Handles {
            stdout: Stdio::piped(),
            stderr: Stdio::piped(),
        },
    )
    .await?;
    let output = child.wait_with_output().await?;
    if output.status.success() {
        Ok(output)
    } else {
        bail!("cargo exited with status: {}", output.status);
    }
}

#[derive(Debug)]
pub struct Handles {
    pub stdout: Stdio,
    pub stderr: Stdio,
}

/// Execute a Cargo subcommand with specified arguments and environment
/// variables.
#[instrument]
pub async fn invoke_with(
    subcommand: impl AsRef<str> + fmt::Debug,
    args: impl IntoIterator<Item = impl AsRef<str>> + fmt::Debug,
    env: impl IntoIterator<Item = (impl AsRef<OsStr>, impl AsRef<OsStr>)> + fmt::Debug,
    handles: Handles,
) -> Result<Child> {
    let subcommand = subcommand.as_ref();
    let args = args.into_iter().collect::<Vec<_>>();
    let args = args.iter().map(|a| a.as_ref()).collect::<Vec<_>>();

    trace!(?subcommand, ?args, "invoke cargo");
    let mut cmd = tokio::process::Command::new("cargo");
    cmd.args(once(subcommand).chain(args.iter().copied()));
    cmd.envs(env);
    cmd.stdout(handles.stdout);
    cmd.stderr(handles.stderr);

    cmd.spawn().context("could not spawn cargo")
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
