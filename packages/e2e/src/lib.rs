//! End-to-end tests for the Hurry project.
//!
//! The intention with this package is that:
//! - We use `hurry` as a CLI tool rather than as a library; just like a user.
//! - We clone or otherwise reproduce test cases with real-world projects.
//! - We use local tools on the system to do testing so that we can keep this as
//! close to a real-world usage as possible.
//! - This also serves as backwards compatibility checks for users.
//!
//! All tests are implemented as integration tests in the `tests/` directory;
//! this library crate for the `e2e` package provides shared functionality and
//! utilities for the tests.
//!
//! ## Tracing
//!
//! Remember that the tracing system is only emitted in test logs; as such you
//! probably want to "up-level" your tracing call levels. For example, things
//! that are `info!` will still only be emitted in test logs since this library
//! is only used in tests.

use std::{
    fs::create_dir_all,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    sync::LazyLock,
};

use bon::builder;
use cargo_metadata::Message;
use color_eyre::{
    Result,
    eyre::{Context, ContextCompat, bail},
};
use escargot::CargoBuild;
use tempfile::TempDir;
use tracing::instrument;

pub mod ext;

static PWD: LazyLock<PathBuf> =
    LazyLock::new(|| std::env::current_dir().expect("current directory"));
static GITHUB_TOKEN: LazyLock<Option<String>> =
    LazyLock::new(|| std::env::var("GITHUB_TOKEN").ok());

#[instrument]
pub fn temporary_directory() -> Result<TempDir> {
    TempDir::new().context("create temporary directory")
}

#[instrument]
pub fn clone_github(username: &str, repo: &str, path: &Path, branch: &str) -> Result<()> {
    eprintln!("clone_github: {username}/{repo}@{branch} to {path:?}");
    create_dir_all(path).with_context(|| format!("create directory: {path:?}"))?;
    let url = match GITHUB_TOKEN.as_ref() {
        Some(token) => format!("https://oauth2:{token}@github.com/{username}/{repo}"),
        None => format!("https://github.com/{username}/{repo}"),
    };
    cmd()
        .pwd(path)
        .bin("git")
        .args(&[
            "clone",
            "--recurse-submodules",
            "--depth=1",
            "--branch",
            branch,
            &url,
            ".",
        ])
        .run()
        .with_context(|| format!("clone repository: 'github.com/{username}/{repo}' to {path:?}"))
}

#[instrument]
pub fn cargo_clean(path: &Path) -> Result<()> {
    cmd()
        .pwd(path)
        .bin("cargo")
        .args(&["clean"])
        .run()
        .with_context(|| format!("clean workspace: {path:?}"))
}

#[instrument]
#[builder(finish_fn(name = "run"))]
pub fn hurry_cargo_build(
    /// The working directory in which to execute the command.
    #[builder(default = &PWD)]
    pwd: &Path,

    /// The `HOME` directory for the invocation.
    home: &Path,

    /// Environment variable pairs for the command.
    /// Each `(&str, &str)` pair is in the form of `("VAR", "VALUE")`.
    #[builder(default = &[])]
    envs: &[(&str, &str)],
) -> Result<Vec<Message>> {
    eprintln!("build with hurry: {pwd:?}");

    let mut cmd = CargoBuild::new()
        .bin("hurry")
        .package("hurry")
        .release()
        .run()
        .context("build hurry")?
        .command()
        .current_dir(pwd)
        .envs(envs.into_iter().copied())
        .env("HOME", home.as_os_str())
        .arg("cargo")
        .arg("build")
        .arg("-v")
        .arg("--message-format=json-render-diagnostics")
        .stdout(Stdio::piped())
        .spawn()
        .context("spawn hurry")?;

    let stdout = cmd.stdout.take().context("get stdout handle")?;
    let reader = std::io::BufReader::new(stdout);
    let messages = Message::parse_stream(reader)
        .map(|m| m.context("parse message"))
        .collect::<Result<Vec<_>>>()
        .context("parse messages")?;

    cmd.wait()
        .context("wait for hurry")
        .and_then(eyre_from_status)
        .map(|_| messages)
}

#[instrument]
#[builder(finish_fn(name = "run"))]
pub fn cmd(
    /// The working directory in which to execute the command.
    #[builder(default = &PWD)]
    pwd: &Path,

    /// Environment variable pairs for the command.
    /// Each `(&str, &str)` pair is in the form of `("VAR", "VALUE")`.
    #[builder(default = &[])]
    envs: &[(&str, &str)],

    /// The binary to execute.
    bin: &str,

    /// The arguments for the command.
    #[builder(default = &[])]
    args: &[&str],
) -> Result<()> {
    std::process::Command::new(bin)
        .args(args)
        .current_dir(pwd)
        .envs(envs.into_iter().copied())
        .status()
        .with_context(|| format!("exec: `{bin:?} {args:?}` in {pwd:?}"))
        .and_then(eyre_from_status)
}

#[instrument]
fn eyre_from_status(status: ExitStatus) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        bail!("failed with status: {status}");
    }
}
