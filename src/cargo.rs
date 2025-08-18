use std::iter::once;

use color_eyre::{
    Result,
    eyre::{Context, bail},
};
use tracing::instrument;

mod cmd;
mod workspace;

pub use cmd::*;

use crate::cargo::workspace::{Unlocked, Workspace};

/// Invoke a cargo subcommand with the given arguments.
#[instrument(skip_all)]
pub fn invoke(
    workspace: &Workspace,
    subcommand: impl AsRef<str>,
    args: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<()> {
    let args = args.into_iter().collect::<Vec<_>>();
    let args = args.iter().map(|a| a.as_ref()).collect::<Vec<_>>();

    let mut cmd = std::process::Command::new("cargo");
    cmd.current_dir(workspace.dir());
    cmd.args(once(subcommand.as_ref()).chain(args));
    let status = cmd
        .spawn()
        .context("could not spawn cargo")?
        .wait()
        .context("could complete cargo execution")?;
    if status.success() {
        Ok(())
    } else {
        bail!("cargo exited with status: {status}");
    }
}
