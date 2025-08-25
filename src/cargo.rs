use std::iter::once;

use color_eyre::{
    Result,
    eyre::{Context, bail},
};
use tracing::{instrument, trace};

mod cmd;
mod workspace;

pub use cmd::*;

use crate::cargo::workspace::Workspace;

/// Invoke a cargo subcommand with the given arguments.
#[instrument(skip_all, name = "cargo::invoke")]
pub fn invoke(
    workspace: &Workspace,
    subcommand: impl AsRef<str>,
    args: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<()> {
    let subcommand = subcommand.as_ref();
    let args = args.into_iter().collect::<Vec<_>>();
    let args = args.iter().map(|a| a.as_ref()).collect::<Vec<_>>();

    let mut cmd = std::process::Command::new("cargo");
    cmd.current_dir(&workspace.root);
    cmd.args(once(subcommand).chain(args.iter().copied()));
    let status = cmd
        .spawn()
        .context("could not spawn cargo")?
        .wait()
        .context("could complete cargo execution")?;
    if status.success() {
        trace!(?workspace, ?subcommand, ?args, "invoke cargo");
        Ok(())
    } else {
        bail!("cargo exited with status: {status}");
    }
}
