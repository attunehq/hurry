//! Invocation helpers for the `cross` command.
//!
//! This module provides utilities for spawning and interacting with the `cross`
//! binary, which is a drop-in replacement for Cargo that handles
//! cross-compilation.

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
use tokio::process::Child;
use tracing::{instrument, trace};

mod config;
mod workspace;

pub use config::CrossConfig;
pub use workspace::extract_host_arch;

#[derive(Debug)]
pub struct Handles {
    pub stdout: Stdio,
    pub stderr: Stdio,
}

/// Execute cross without a subcommand with specified arguments.
#[instrument]
pub async fn invoke_plain(
    args: impl IntoIterator<Item = impl AsRef<str>> + fmt::Debug,
) -> Result<()> {
    let args = args.into_iter().collect::<Vec<_>>();
    let args = args.iter().map(|a| a.as_ref()).collect::<Vec<_>>();

    trace!(?args, "invoke cross");
    let mut cmd = tokio::process::Command::new("cross");
    cmd.args(args.iter().copied());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());

    let status = cmd
        .spawn()
        .with_context(|| {
            "could not spawn cross\n\n\
             cross is not installed or not in your PATH.\n\
             To install cross, run:\n\n\
             \tcargo install cross\n\n\
             For more information, see: https://github.com/cross-rs/cross"
        })?
        .wait()
        .await
        .context("could not complete cross execution")?;
    if status.success() {
        Ok(())
    } else {
        bail!("cross exited with status: {}", status);
    }
}

/// Execute a cross subcommand with specified arguments.
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
    .context("could not complete cross execution")?;
    if status.success() {
        Ok(())
    } else {
        bail!("cross exited with status: {}", status);
    }
}

/// Execute a cross subcommand with specified arguments and environment
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
        bail!("cross exited with status: {}", output.status);
    }
}

/// Execute a cross subcommand with specified arguments and environment
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

    trace!(?subcommand, ?args, "invoke cross");
    let mut cmd = tokio::process::Command::new("cross");
    cmd.args(once(subcommand).chain(args.iter().copied()));
    cmd.envs(env);
    cmd.stdout(handles.stdout);
    cmd.stderr(handles.stderr);

    cmd.spawn().with_context(|| {
        "could not spawn cross\n\n\
         cross is not installed or not in your PATH.\n\
         To install cross, run:\n\n\
         \tcargo install cross\n\n\
         For more information, see: https://github.com/cross-rs/cross"
    })
}
