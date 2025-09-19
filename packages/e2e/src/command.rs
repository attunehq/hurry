use std::{
    borrow::Cow,
    ffi::{OsStr, OsString},
    io::{Read, Write},
    iter::once,
    os::unix::process::ExitStatusExt,
    path::PathBuf,
    process::{ExitStatus, Output, Stdio},
};

use bollard::{container::LogOutput, exec::StartExecResults, secret::ExecConfig};
use bon::{Builder, bon};
use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context, OptionExt, bail, eyre},
};
use futures::StreamExt;
use tracing::instrument;

use crate::{Container, GITHUB_TOKEN};

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

#[bon]
impl Command {
    /// Clean the `cargo` workspace in the provided working directory.
    pub fn cargo_clean(pwd: impl Into<PathBuf>) -> Self {
        Self::new().pwd(pwd).name("cargo").arg("clean").finish()
    }

    /// Clone the `github.com/attunehq/hurry` repository.
    pub fn clone_hurry(pwd: impl Into<PathBuf>) -> Self {
        Self::clone_github()
            .pwd(pwd)
            .user("attunehq")
            .repo("hurry")
            .branch("main")
            .finish()
    }

    /// Install the `hurry` binaries in the provided working directory.
    pub fn install_hurry(pwd: impl Into<PathBuf>) -> Self {
        Self::cargo_install()
            .pwd(pwd)
            .args(["--path", "packages/hurry"])
            .finish()
    }

    /// Run `cargo install` in the provided working directory.
    #[builder(finish_fn = finish)]
    pub fn cargo_install(
        /// Arguments for the command.
        #[builder(field)]
        args: Vec<OsString>,

        /// The working directory in which to perform the install.
        #[builder(into)]
        pwd: PathBuf,
    ) -> Self {
        Self::new()
            .pwd(pwd)
            .name("cargo")
            .arg("install")
            .args(args)
            .finish()
    }

    /// Clone a github repository.
    #[builder(finish_fn = finish)]
    pub fn clone_github(
        /// The working directory in which to perform the clone.
        ///
        /// Note that the clone will create a new subdirectory in this working
        /// directory with the name of the repository.
        #[builder(into)]
        pwd: PathBuf,

        /// The user in GitHub that owns the repository.
        #[builder(into)]
        user: String,

        /// The repository to clone.
        #[builder(into)]
        repo: String,

        /// The branch to clone.
        #[builder(into)]
        branch: String,

        /// The directory to clone into. If not provided, defaults to the name
        /// of the repository.
        #[builder(into)]
        dir: Option<OsString>,
    ) -> Self {
        let url = match GITHUB_TOKEN.as_ref() {
            Some(token) => format!("https://oauth2:{token}@github.com/{user}/{repo}"),
            None => format!("https://github.com/{user}/{repo}"),
        };
        Command::new()
            .pwd(pwd)
            .name("git")
            .arg("clone")
            .arg("--recurse-submodules")
            .arg("--depth=1")
            .arg("--branch")
            .arg(branch)
            .arg(&url)
            .arg(dir.unwrap_or_else(|| repo.into()))
            .finish()
    }

    /// Run the command locally.
    ///
    /// The command stdio pipes are inherited from the parent (meaning, they
    /// interact with the stdio pipes of the current process).
    #[instrument]
    pub fn run_local(self) -> Result<()> {
        self.as_std()
            .status()
            .with_context(|| format!("exec: `{:?} {:?}` in {:?}", self.name, self.args, self.pwd))
            .and_then(ParsedOutput::parse_status)
            .map(drop)
    }

    /// Run the command locally, capturing the output.
    ///
    /// The command stdout and stderr are written to the corresponding pipes of
    /// the current process, and are also buffered into the returned value.
    #[instrument]
    pub fn run_local_with_output(self) -> Result<ParsedOutput> {
        let mut child = self
            .as_std()
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| {
                format!("exec: `{:?} {:?}` in {:?}", self.name, self.args, self.pwd)
            })?;

        let mut read_stdout = child.stdout.take().ok_or_eyre("take stdout")?;
        let mut read_stderr = child.stderr.take().ok_or_eyre("take stderr")?;
        let mut write_stdout = std::io::stdout();
        let mut write_stderr = std::io::stderr();
        let mut buf_stdout = Vec::<u8>::new();
        let mut buf_stderr = Vec::<u8>::new();

        // We do this manually instead of using e.g. `std::io::copy` with
        // `io_tee` so that we can drive both pipes concurrently without needing
        // threads.
        loop {
            let mut buf = [0; 1024];
            let stdout_read = read_stdout.read(&mut buf).context("read stdout")?;
            if stdout_read > 0 {
                write_stdout
                    .write_all(&buf[..stdout_read])
                    .context("write stdout")?;
                buf_stdout.extend_from_slice(&buf[..stdout_read]);
            }
            let stderr_read = read_stderr.read(&mut buf).context("read stderr")?;
            if stderr_read > 0 {
                write_stderr
                    .write_all(&buf[..stderr_read])
                    .context("write stderr")?;
                buf_stderr.extend_from_slice(&buf[..stderr_read]);
            }
            if stdout_read == 0 && stderr_read == 0 {
                break;
            }
        }

        let status = child
            .wait()
            .with_context(|| format!("run: `{:?} {:?}` in {:?}", self.name, self.args, self.pwd))
            .with_section(|| {
                String::from_utf8_lossy(&buf_stderr)
                    .into_owned()
                    .header("Stderr:")
            })
            .with_section(|| {
                String::from_utf8_lossy(&buf_stdout)
                    .into_owned()
                    .header("Stdout:")
            })?;

        ParsedOutput::parse(Output {
            status,
            stdout: buf_stdout,
            stderr: buf_stderr,
        })
    }

    /// Run the command inside the container.
    ///
    /// Simulates stdout and stderr pipe inheritance from the parent: output to
    /// each pipe by the command is emitted to the equivalent pipe for the
    /// current process.
    ///
    /// Note: The `pwd` and other paths/binaries/etc specified in the command
    /// are all inside the _container_ context, not the host machine; this
    /// command does nothing to e.g. move the working directory to the container
    /// or anything similar.
    #[instrument]
    pub async fn run_docker(self, container: &Container) -> Result<()> {
        let config = self
            .as_container_exec()
            .context("build docker exec context")?;
        let exec = container
            .docker()
            .create_exec(container.id(), config)
            .await
            .context("create exec")?
            .id;
        match container.docker().start_exec(&exec, None).await {
            Ok(StartExecResults::Attached { mut output, .. }) => {
                let mut stdout = std::io::stdout();
                let mut stderr = std::io::stderr();
                while let Some(line) = output.next().await {
                    match line.context("read line")? {
                        LogOutput::StdIn { .. } => {}
                        LogOutput::Console { .. } => {}
                        LogOutput::StdErr { message } => {
                            stderr.write_all(&message).context("write stderr")?;
                        }
                        LogOutput::StdOut { message } => {
                            stdout.write_all(&message).context("write stdout")?;
                        }
                    }
                }
            }
            Ok(StartExecResults::Detached) => unreachable!("we don't use a detached API"),
            Err(err) => bail!("run command: {err:?}"),
        }

        let info = container
            .docker()
            .inspect_exec(&exec)
            .await
            .context("inspect exec")?;
        let code = info.exit_code.map(|code| code as i32).unwrap_or_default();
        ParsedOutput::parse_status(ExitStatus::from_raw(code)).map(drop)
    }

    /// Run the command inside the container, capturing the output.
    ///
    /// The command stdout and stderr are written to the corresponding pipes of
    /// the current process, and are also buffered into the returned value.
    ///
    /// Note: The `pwd` and other paths/binaries/etc specified in the command
    /// are all inside the _container_ context, not the host machine; this
    /// command does nothing to e.g. move the working directory to the container
    /// or anything similar.
    #[instrument]
    pub async fn run_docker_with_output(self, container: &Container) -> Result<ParsedOutput> {
        let config = self
            .as_container_exec()
            .context("build docker exec context")?;
        let exec = container
            .docker()
            .create_exec(container.id(), config)
            .await
            .context("create exec")?
            .id;

        let mut stdout_buf = Vec::<u8>::new();
        let mut stderr_buf = Vec::<u8>::new();
        match container.docker().start_exec(&exec, None).await {
            Ok(StartExecResults::Attached { mut output, .. }) => {
                let mut stdout = std::io::stdout();
                let mut stderr = std::io::stderr();
                while let Some(line) = output.next().await {
                    match line.context("read line")? {
                        LogOutput::StdIn { .. } => {}
                        LogOutput::Console { .. } => {}
                        LogOutput::StdErr { message } => {
                            stderr_buf.write_all(&message).context("buffer stderr")?;
                            stderr.write_all(&message).context("write stderr")?;
                        }
                        LogOutput::StdOut { message } => {
                            stdout_buf.write_all(&message).context("buffer stdout")?;
                            stdout.write_all(&message).context("write stdout")?;
                        }
                    }
                }
            }
            Ok(StartExecResults::Detached) => unreachable!("we don't use a detached API"),
            Err(err) => bail!("run command: {err:?}"),
        }

        let info = container
            .docker()
            .inspect_exec(&exec)
            .await
            .context("inspect exec")?;
        let code = info.exit_code.map(|code| code as i32).unwrap_or_default();
        ParsedOutput::parse(Output {
            status: ExitStatus::from_raw(code),
            stdout: stdout_buf,
            stderr: stderr_buf,
        })
    }

    pub(super) fn as_container_exec(&self) -> Result<ExecConfig> {
        fn try_as_unicode(s: impl AsRef<OsStr>) -> Result<String> {
            let s = s.as_ref();
            s.to_str()
                .map(String::from)
                .ok_or_eyre("invalid unicode")
                .with_context(|| format!("parse as unicode: {s:?}"))
        }

        let pwd = try_as_unicode(&self.pwd).context("convert pwd")?;
        let envs = self
            .envs
            .iter()
            .map(|(k, v)| -> Result<String> {
                let k = try_as_unicode(k).context("convert env key")?;
                let v = try_as_unicode(v).context("convert env value")?;
                Ok(format!("{k}={v}"))
            })
            .collect::<Result<Vec<_>>>()
            .context("convert envs")?;

        let name = try_as_unicode(&self.name).context("convert process name")?;
        let args = self
            .args
            .iter()
            .map(try_as_unicode)
            .collect::<Result<Vec<_>>>()
            .context("convert args")?;

        Ok(ExecConfig {
            attach_stderr: Some(true),
            attach_stdout: Some(true),
            working_dir: Some(pwd),
            env: Some(envs),
            cmd: Some(once(name).chain(args).collect()),
            ..Default::default()
        })
    }

    pub(super) fn as_std(&self) -> std::process::Command {
        let mut cmd = std::process::Command::new(&self.name);
        cmd.args(&self.args)
            .current_dir(&self.pwd)
            .envs(self.envs.iter().map(|(k, v)| (k, v)));
        cmd
    }
}

impl<S: command_cargo_install_builder::State> CommandCargoInstallBuilder<S> {
    /// Adds a single argument to pass to the program.
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Adds multiple arguments to pass to the program.
    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<OsString>>) -> Self {
        self.args.extend(args.into_iter().map(Into::into));
        self
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

    /// Adds a single argument to pass to the program.
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
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

impl From<&Command> for Command {
    fn from(command: &Command) -> Self {
        command.clone()
    }
}

impl AsRef<Command> for Command {
    fn as_ref(&self) -> &Command {
        self
    }
}

/// The parsed output of a command.
#[derive(Clone, Debug, Builder)]
pub struct ParsedOutput {
    /// The stderr of the command.
    #[builder(into, default)]
    pub stderr: Vec<u8>,

    /// The stdout of the command.
    #[builder(into, default)]
    pub stdout: Vec<u8>,

    /// The status of the command.
    ///
    /// Note that if you use [`ParsedOutput::parse`] to construct this type, the
    /// command is guaranteed to be successful as otherwise that method would
    /// have generated an error.
    ///
    /// The field exists so that if you want to actually run fallible commands
    /// and read their output regardless of status code, in which case you can
    /// use [`ParsedOutput::from`] to construct this type.
    #[builder(with = |status: i32| ExitStatus::from_raw(status))]
    pub status: ExitStatus,
}

impl ParsedOutput {
    /// Parse the output of a command.
    ///
    /// If the command status code indicates failure, this method returns an
    /// error with the status code and the contents of stdout/stderr.
    ///
    /// Note: if you want to actually run fallible commands and read their
    /// output regardless of status code, in which case you can use
    /// [`ParsedOutput::from`] to construct this type.
    #[instrument]
    pub fn parse(output: Output) -> Result<Self> {
        let output = Self::from(output);
        if output.status.success() {
            Ok(output)
        } else {
            Err(eyre!("command failed with status: {}", output.status))
                .section(output.stdout_lossy_string().header("Stdout:"))
                .section(output.stderr_lossy_string().header("Stderr:"))
        }
    }
    /// Parse the status of a command.
    ///
    /// [`ParsedOutput::stderr`] and [`ParsedOutput::stdout`] are empty when
    /// this method is used to construct the type, since they are not available.
    ///
    /// If the command status code indicates failure, this method returns an
    /// error with the status code; since stdout/stderr are not available they
    /// are not included in the error.
    #[instrument]
    pub fn parse_status(status: ExitStatus) -> Result<Self> {
        if status.success() {
            Ok(Self {
                status,
                stderr: Vec::new(),
                stdout: Vec::new(),
            })
        } else {
            Err(eyre!("command failed with status: {status}"))
        }
    }
    /// View [`ParsedOutput::stderr`] as a lossily-converted string.
    pub fn stderr_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stderr)
    }

    /// View [`ParsedOutput::stderr`] as a lossily-converted owned string.
    pub fn stderr_lossy_string(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    /// View [`ParsedOutput::stdout`] as a lossily-converted string.
    pub fn stdout_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stdout)
    }

    /// View [`ParsedOutput::stdout`] as a lossily-converted owned string.
    pub fn stdout_lossy_string(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

impl From<Output> for ParsedOutput {
    fn from(output: Output) -> Self {
        Self {
            stderr: output.stderr,
            stdout: output.stdout,
            status: output.status,
        }
    }
}
