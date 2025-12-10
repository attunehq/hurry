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
use serde::Deserialize;
use tokio::process::Child;
use tracing::{instrument, trace};

mod build_args;
mod build_plan;
mod build_script;
mod cache;
mod dep_info;
mod fingerprint;
mod glibc;
mod path;
mod profile;
mod rustc;
mod unit_graph;
mod units;
mod workspace;

pub use build_args::{CargoBuildArgument, CargoBuildArguments, ColorWhen, MessageFormat};
pub use build_plan::BuildPlan;
pub use build_script::BuildScriptOutput;
pub use cache::{CargoCache, Restored, SaveProgress, SavedFile, save_units};
pub use dep_info::{DepInfo, DepInfoLine};
pub use fingerprint::Fingerprint;
pub use glibc::host_glibc_version;
pub use path::QualifiedPath;
pub use profile::Profile;
pub use rustc::{RustcArgument, RustcArguments, RustcTarget, RustcTargetPlatform};
pub use unit_graph::{
    UnitGraph, UnitGraphDependency, UnitGraphProfile, UnitGraphProfilePanicStrategy, UnitGraphUnit,
};
pub use units::{
    BuildScriptCompilationUnitPlan, BuildScriptCompiledFiles, BuildScriptExecutionUnitPlan,
    BuildScriptOutputFiles, LibraryCrateUnitPlan, LibraryFiles,
};
pub use workspace::{UnitHash, UnitPlan, UnitPlanInfo, Workspace};

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CargoCompileMode {
    Test,
    Build,
    Check,
    Doc,
    Doctest,
    Docscrape,
    RunCustomBuild,
}

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
