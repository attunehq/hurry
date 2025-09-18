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
    ffi::{OsStr, OsString},
    fmt::Debug,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    sync::LazyLock,
};

use bon::{Builder, builder};
use cargo_metadata::Message;
use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context, ContextCompat, OptionExt, bail},
};
use tempfile::TempDir;
use tracing::instrument;

pub mod ext;

static GITHUB_TOKEN: LazyLock<Option<String>> =
    LazyLock::new(|| std::env::var("GITHUB_TOKEN").ok());

/// Construct a command for building a package with Cargo.
///
/// This type provides an abstracted interface for running the build locally or
/// in a docker context.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Builder)]
#[builder(start_fn = new, finish_fn = finish)]
pub struct Build {
    /// Additional arguments to set when running the build.
    ///
    /// The [`Build::DEFAULT_ARGS`] are always set; arguments provided to this
    /// function are set afterwards.
    /// Arguments for the command.
    #[builder(field)]
    additional_args: Vec<OsString>,

    /// Environment variable pairs to set when running the build.
    /// Each pair is in the form of `("VAR", "VALUE")`.
    #[builder(field)]
    envs: Vec<(OsString, OsString)>,

    /// The working directory in which to run the build.
    /// This should generally be the root of the workspace.
    #[builder(into)]
    pwd: PathBuf,

    /// The binary to build.
    #[builder(into)]
    bin: Option<String>,

    /// The package to build.
    #[builder(into)]
    package: Option<String>,

    /// Whether to build in release mode.
    #[builder(default)]
    release: bool,
}

impl Build {
    /// The name of the `hurry` package and executable.
    pub const HURRY_NAME: &str = "hurry";

    /// The default set of arguments that are always provided to build commands.
    pub const DEFAULT_ARGS: [&str; 3] = ["build", "-v", "--message-format=json-render-diagnostics"];

    /// Construct an instance for building `hurry` in the current directory with
    /// default settings.
    #[instrument]
    pub fn hurry(pwd: impl Into<PathBuf> + Debug) -> Build {
        Build::new()
            .pwd(pwd)
            .bin(Build::HURRY_NAME)
            .package(Build::HURRY_NAME)
            .release(true)
            .finish()
    }

    /// Run the build locally through `hurry`.
    ///
    /// This method builds `hurry`, then uses `hurry cargo build` to run the
    /// build locally.
    #[instrument]
    pub fn hurry_local(&self, hurry: &Build) -> Result<Vec<Message>> {
        let hurry = hurry.run_local().context("build hurry")?;
        let hurry_path = hurry
            .iter()
            .find_map(|m| match m {
                Message::CompilerArtifact(artifact) => artifact
                    .executable
                    .as_ref()
                    .map(|p| p.as_std_path().to_path_buf()),
                _ => None,
            })
            .ok_or_eyre("unable to locate hurry executable in output")
            .with_section(|| {
                hurry
                    .iter()
                    .map(|msg| format!("{msg:?}"))
                    .collect::<Vec<_>>()
                    .join("\n")
                    .header("Compiler messages:")
            })?;

        Self::capture_local(self.as_wrapped_command(&hurry_path))
            .with_context(|| {
                format!(
                    "'{hurry_path:?} cargo build' {:?}/{:?} in {:?}",
                    self.package, self.bin, self.pwd
                )
            })
            .with_section(|| {
                hurry
                    .iter()
                    .map(|msg| format!("{msg:?}"))
                    .collect::<Vec<_>>()
                    .join("\n")
                    .header("Hurry compiler messages:")
            })
    }

    /// Run the build locally.
    #[instrument]
    pub fn run_local(&self) -> Result<Vec<Message>> {
        Self::capture_local(self.as_command()).with_context(|| {
            format!(
                "'cargo build' {:?}/{:?} in {:?}",
                self.package, self.bin, self.pwd
            )
        })
    }

    fn as_command(&self) -> Command {
        Command::new()
            .name("cargo")
            .args(Self::DEFAULT_ARGS)
            .arg_maybe("--bin", self.bin.as_ref())
            .arg_maybe("--package", self.package.as_ref())
            .arg_if(self.release, "--release")
            .args(&self.additional_args)
            .envs(self.envs.iter().map(|(k, v)| (k, v)))
            .pwd(&self.pwd)
            .finish()
    }

    fn as_wrapped_command(&self, wrapper: impl AsRef<OsStr>) -> Command {
        Command::new()
            .name(wrapper.as_ref())
            .arg("cargo")
            .args(Self::DEFAULT_ARGS)
            .arg_maybe("--bin", self.bin.as_ref())
            .arg_maybe("--package", self.package.as_ref())
            .arg_if(self.release, "--release")
            .args(&self.additional_args)
            .envs(self.envs.iter().map(|(k, v)| (k, v)))
            .pwd(&self.pwd)
            .finish()
    }

    fn capture_local(cmd: Command) -> Result<Vec<Message>> {
        let mut handle = cmd
            .as_std()
            .stdout(Stdio::piped())
            .spawn()
            .context("run build command")?;

        let stdout = handle.stdout.take().context("get stdout")?;
        let reader = std::io::BufReader::new(stdout);
        let messages = Message::parse_stream(reader)
            .map(|m| m.context("parse message"))
            .collect::<Result<Vec<_>>>()
            .context("parse messages")?;

        handle
            .wait()
            .context("read build command")
            .and_then(eyre_from_status)
            .map(|_| messages)
    }
}

impl<S: build_builder::State> BuildBuilder<S> {
    /// Add a single additional argument to pass to the program.
    ///
    /// The [`Build::DEFAULT_ARGS`] are always set, and then are followed by the
    /// arguments set by options to this type; "additional" arguments are set
    /// afterwards.
    pub fn additional_arg(mut self, arg: impl Into<OsString>) -> Self {
        self.additional_args.push(arg.into());
        self
    }

    /// Add multiple additional arguments to pass to the program.
    ///
    /// The [`Build::DEFAULT_ARGS`] are always set, and then are followed by the
    /// arguments set by options to this type; "additional" arguments are set
    /// afterwards.
    pub fn additional_args(mut self, args: impl IntoIterator<Item = impl Into<OsString>>) -> Self {
        self.additional_args
            .extend(args.into_iter().map(Into::into));
        self
    }

    /// Add an environment variable pair to use when running the build.
    pub fn env(mut self, var: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.envs.push((var.into(), value.into()));
        self
    }

    /// Add multiple environment variable pairs to use when running the build.
    /// Each pair is in the form of `("VAR", "VALUE")`.
    pub fn envs(
        mut self,
        envs: impl IntoIterator<Item = (impl Into<OsString>, impl Into<OsString>)>,
    ) -> Self {
        self.envs
            .extend(envs.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }
}

/// Construct a command to run.
///
/// This type provides an abstracted interface for running a command locally or
/// in a docker context.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Builder)]
#[builder(start_fn = new, finish_fn = finish)]
pub struct Command {
    /// Arguments for the command.
    #[builder(field)]
    args: Vec<OsString>,

    /// Environment variable pairs for the command.
    /// Each pair is in the form of `("VAR", "VALUE")`.
    #[builder(field)]
    envs: Vec<(OsString, OsString)>,

    /// The working directory in which to execute the command.
    #[builder(into)]
    pwd: PathBuf,

    /// The binary name (if in `$PATH`) or path to execute.
    #[builder(into)]
    name: OsString,
}

impl Command {
    /// Clean the `cargo` workspace in the provided working directory.
    pub fn cargo_clean(pwd: impl Into<PathBuf>) -> Self {
        Self::new().pwd(pwd).name("cargo").arg("clean").finish()
    }

    /// Clone a github repository.
    pub fn clone_github(user: &str, repo: &str, path: &Path, branch: &str) -> Self {
        let url = match GITHUB_TOKEN.as_ref() {
            Some(token) => format!("https://oauth2:{token}@github.com/{user}/{repo}"),
            None => format!("https://github.com/{user}/{repo}"),
        };
        Command::new()
            .pwd(path)
            .name("git")
            .arg("clone")
            .arg("--recurse-submodules")
            .arg("--depth=1")
            .arg("--branch")
            .arg(branch)
            .arg(&url)
            .arg(".")
            .finish()
    }

    /// Run the command locally.
    #[instrument]
    pub fn run_local(self) -> Result<()> {
        self.as_std()
            .status()
            .with_context(|| format!("exec: `{:?} {:?}` in {:?}", self.name, self.args, self.pwd))
            .and_then(eyre_from_status)
    }

    fn as_std(&self) -> std::process::Command {
        let mut cmd = std::process::Command::new(&self.name);
        cmd.args(&self.args)
            .current_dir(&self.pwd)
            .envs(self.envs.iter().map(|(k, v)| (k, v)));
        cmd
    }
}

impl<S: command_builder::State> CommandBuilder<S> {
    /// Adds a single argument to pass to the program if the predicate is true.
    pub fn arg_if(mut self, predicate: bool, arg: impl Into<OsString>) -> Self {
        if predicate {
            self.args.push(arg.into());
        }
        self
    }

    /// Adds a single argument to pass to the program.
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Adds the argument pair if the value is `Some`.
    pub fn arg_maybe(
        mut self,
        flag: impl Into<OsString>,
        value: Option<impl Into<OsString>>,
    ) -> Self {
        if let Some(v) = value {
            self.args.push(flag.into());
            self.args.push(v.into());
        }
        self
    }

    /// Adds multiple arguments to pass to the program.
    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<OsString>>) -> Self {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Add an environment variable pair to use when running the build.
    pub fn env(mut self, var: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.envs.push((var.into(), value.into()));
        self
    }

    /// Add multiple environment variable pairs to use when running the build.
    /// Each pair is in the form of `("VAR", "VALUE")`.
    pub fn envs(
        mut self,
        envs: impl IntoIterator<Item = (impl Into<OsString>, impl Into<OsString>)>,
    ) -> Self {
        self.envs
            .extend(envs.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }
}

#[instrument]
pub fn temporary_directory() -> Result<TempDir> {
    TempDir::new().context("create temporary directory")
}

#[instrument]
fn eyre_from_status(status: ExitStatus) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        bail!("failed with status: {status}");
    }
}
