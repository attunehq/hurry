use std::{
    borrow::Cow,
    ffi::{OsStr, OsString},
    os::unix::process::ExitStatusExt,
    path::PathBuf,
    process::{ExitStatus, Output},
};

use bollard::{
    Docker,
    container::LogOutput,
    exec::{CreateExecOptions, StartExecResults},
};
use bon::{Builder, bon};
use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context, OptionExt, bail, eyre},
};
use futures::StreamExt;
use tokio::io::{stderr, stdout};
use tracing::instrument;

use crate::GITHUB_TOKEN;

/// Construct a command to run.
///
/// This type provides an abstracted interface for running a command in
/// testcontainers compose environments.
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

    /// Run the command inside a testcontainers compose container.
    ///
    /// This method integrates with Docker exec API, allowing commands
    /// to be run in containers managed by Docker Compose via testcontainers.
    ///
    /// The command's pwd and environment variables are handled by wrapping the
    /// command in a shell invocation.
    ///
    /// # Example
    /// ```ignore
    /// let env = TestEnv::new().await?;
    /// Command::new()
    ///     .name("hurry")
    ///     .arg("--version")
    ///     .pwd("/workspace")
    ///     .finish()
    ///     .run_compose(&env.hurry_container_id())
    ///     .await?;
    /// ```
    #[instrument(skip(self, container_id), fields(name = ?self.name, pwd = ?self.pwd))]
    pub async fn run_compose(self, container_id: &str) -> Result<()> {
        use tokio::io::AsyncWriteExt;

        fn try_as_unicode(s: impl AsRef<OsStr>) -> Result<String> {
            let s = s.as_ref();
            s.to_str()
                .map(String::from)
                .ok_or_eyre("invalid unicode")
                .with_context(|| format!("parse as unicode: {s:?}"))
        }

        let name = try_as_unicode(&self.name).context("convert process name")?;
        let args = self
            .args
            .iter()
            .map(try_as_unicode)
            .collect::<Result<Vec<_>>>()
            .context("convert args")?;

        let mut cmd_parts = vec![name];
        cmd_parts.extend(args);

        let shell_cmd = if !self.envs.is_empty() {
            let env_exports = self
                .envs
                .iter()
                .map(|(k, v)| -> Result<String> {
                    let k = try_as_unicode(k).context("convert env key")?;
                    let v = try_as_unicode(v).context("convert env value")?;
                    Ok(format!("export {k}={v:?}"))
                })
                .collect::<Result<Vec<_>>>()
                .context("convert envs")?
                .join(" && ");

            let pwd = try_as_unicode(&self.pwd).context("convert pwd")?;
            let cmd_str = cmd_parts.join(" ");
            format!("{env_exports} && cd {pwd:?} && {cmd_str}")
        } else {
            let pwd = try_as_unicode(&self.pwd).context("convert pwd")?;
            let cmd_str = cmd_parts.join(" ");
            format!("cd {pwd:?} && {cmd_str}")
        };

        // Create Docker client
        let docker = Docker::connect_with_local_defaults().context("connect to Docker")?;

        // Create exec instance
        let exec_config = CreateExecOptions {
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            cmd: Some(vec!["sh", "-c", &shell_cmd]),
            ..Default::default()
        };

        let exec_id = docker
            .create_exec(container_id, exec_config)
            .await
            .context("create exec")?
            .id;

        // Start the exec and stream output
        let mut stdout = stdout();
        let mut stderr = stderr();

        match docker.start_exec(&exec_id, None).await {
            Ok(StartExecResults::Attached { mut output, .. }) => {
                while let Some(line) = output.next().await {
                    match line.context("read line")? {
                        LogOutput::StdIn { .. } => {}
                        LogOutput::Console { .. } => {}
                        LogOutput::StdErr { message } => {
                            stderr.write_all(&message).await.context("write stderr")?;
                        }
                        LogOutput::StdOut { message } => {
                            stdout.write_all(&message).await.context("write stdout")?;
                        }
                    }
                }
            }
            Ok(StartExecResults::Detached) => unreachable!("we don't use a detached API"),
            Err(err) => bail!("run command: {err:?}"),
        }

        // Check exit code
        let info = docker
            .inspect_exec(&exec_id)
            .await
            .context("inspect exec")?;
        let code = info.exit_code.map(|code| code as i32).unwrap_or_default();

        ParsedOutput::parse_status(ExitStatus::from_raw(code)).map(drop)
    }

    /// Run the command inside a testcontainers compose container, capturing
    /// output.
    ///
    /// Similar to `run_compose()` but also captures and returns stdout/stderr.
    #[instrument(skip(self, container_id), fields(name = ?self.name, pwd = ?self.pwd))]
    pub async fn run_compose_with_output(self, container_id: &str) -> Result<ParsedOutput> {
        use tokio::io::AsyncWriteExt;

        fn try_as_unicode(s: impl AsRef<OsStr>) -> Result<String> {
            let s = s.as_ref();
            s.to_str()
                .map(String::from)
                .ok_or_eyre("invalid unicode")
                .with_context(|| format!("parse as unicode: {s:?}"))
        }

        let name = try_as_unicode(&self.name).context("convert process name")?;
        let args = self
            .args
            .iter()
            .map(try_as_unicode)
            .collect::<Result<Vec<_>>>()
            .context("convert args")?;

        let mut cmd_parts = vec![name];
        cmd_parts.extend(args);

        let shell_cmd = if !self.envs.is_empty() {
            let env_exports = self
                .envs
                .iter()
                .map(|(k, v)| -> Result<String> {
                    let k = try_as_unicode(k).context("convert env key")?;
                    let v = try_as_unicode(v).context("convert env value")?;
                    Ok(format!("export {k}={v:?}"))
                })
                .collect::<Result<Vec<_>>>()
                .context("convert envs")?
                .join(" && ");

            let pwd = try_as_unicode(&self.pwd).context("convert pwd")?;
            let cmd_str = cmd_parts.join(" ");
            format!("{env_exports} && cd {pwd:?} && {cmd_str}")
        } else {
            let pwd = try_as_unicode(&self.pwd).context("convert pwd")?;
            let cmd_str = cmd_parts.join(" ");
            format!("cd {pwd:?} && {cmd_str}")
        };

        // Create Docker client
        let docker = Docker::connect_with_local_defaults().context("connect to Docker")?;

        // Create exec instance
        let exec_config = CreateExecOptions {
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            cmd: Some(vec!["sh", "-c", &shell_cmd]),
            ..Default::default()
        };

        let exec_id = docker
            .create_exec(container_id, exec_config)
            .await
            .context("create exec")?
            .id;

        // Start the exec and collect output
        let mut stdout = stdout();
        let mut stderr = stderr();
        let mut stdout_buf = Vec::<u8>::new();
        let mut stderr_buf = Vec::<u8>::new();

        match docker.start_exec(&exec_id, None).await {
            Ok(StartExecResults::Attached { mut output, .. }) => {
                while let Some(line) = output.next().await {
                    match line.context("read line")? {
                        LogOutput::StdIn { .. } => {}
                        LogOutput::Console { .. } => {}
                        LogOutput::StdErr { message } => {
                            stderr_buf.extend_from_slice(&message);
                            stderr.write_all(&message).await.context("write stderr")?;
                        }
                        LogOutput::StdOut { message } => {
                            stdout_buf.extend_from_slice(&message);
                            stdout.write_all(&message).await.context("write stdout")?;
                        }
                    }
                }
            }
            Ok(StartExecResults::Detached) => unreachable!("we don't use a detached API"),
            Err(err) => bail!("run command: {err:?}"),
        }

        // Check exit code
        let info = docker
            .inspect_exec(&exec_id)
            .await
            .context("inspect exec")?;
        let code = info.exit_code.map(|code| code as i32).unwrap_or_default();

        ParsedOutput::parse(Output {
            status: ExitStatus::from_raw(code),
            stdout: stdout_buf,
            stderr: stderr_buf,
        })
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
