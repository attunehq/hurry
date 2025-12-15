use std::{ffi::OsString, fmt::Debug, io::Cursor, path::PathBuf};

use bon::Builder;
use cargo_metadata::Message;
use color_eyre::{Result, Section, SectionExt, eyre::Context};
use tracing::instrument;

use crate::Command;

/// Construct a command for building a package with Cargo.
///
/// This type provides an abstracted interface for running the build in
/// testcontainers compose environments.
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

    /// Features to enable for the build.
    #[builder(field)]
    features: Vec<String>,

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

    /// The wrapper binary to use for `cargo`.
    ///
    /// For example, normally builds run e.g. `cargo build`, but if you provide
    /// a wrapper (e.g. `hurry`) then the build runs with the wrapper like
    /// `hurry cargo build`.
    #[builder(into)]
    wrapper: Option<OsString>,

    /// Whether to build in release mode.
    #[builder(default)]
    release: bool,

    /// The Hurry API URL for distributed caching.
    ///
    /// If provided, this is passed to hurry via the `HURRY_API_URL`
    /// environment variable.
    #[builder(into)]
    api_url: Option<String>,

    /// The Hurry API token for authentication.
    ///
    /// If provided, this is passed to hurry via the `HURRY_API_TOKEN`
    /// environment variable.
    #[builder(into)]
    api_token: Option<String>,
}

impl Build {
    /// The name of the `hurry` package and executable.
    pub const HURRY_NAME: &str = "hurry";

    /// The default set of arguments that are always provided to build commands.
    pub const DEFAULT_ARGS: [&str; 3] = ["build", "-v", "--message-format=json-render-diagnostics"];

    /// Run the build inside a compose container.
    ///
    /// Uses the Docker container ID from a Docker Compose stack managed by
    /// testcontainers.
    ///
    /// Note: The `pwd` and other paths/binaries/etc specified in the command
    /// are all inside the _container_ context, not the host machine.
    #[instrument(skip(self, container_id), fields(package = ?self.package, bin = ?self.bin, pwd = ?self.pwd))]
    pub async fn run_compose(&self, container_id: impl AsRef<str> + Debug) -> Result<Vec<Message>> {
        Self::capture_compose(self.as_command(), container_id.as_ref())
            .await
            .with_context(|| {
                format!(
                    "'cargo build' {:?}/{:?} in {:?}",
                    self.package, self.bin, self.pwd
                )
            })
    }

    fn as_command(&self) -> Command {
        let mut cmd = match &self.wrapper {
            Some(wrapper) => Command::new().name(wrapper).arg("cargo"),
            None => Command::new().name("cargo"),
        };

        cmd = cmd
            .args(Self::DEFAULT_ARGS)
            .arg_maybe("--bin", self.bin.as_ref())
            .arg_maybe("--package", self.package.as_ref())
            .arg_if(self.release, "--release")
            .arg_if(
                !self.features.is_empty(),
                format!("--features={}", self.features.join(",")),
            )
            .args(&self.additional_args)
            .envs(self.envs.iter().map(|(k, v)| (k, v)));

        // We pass these as environment variables so that we can avoid the annoyance
        // around argument ordering; see https://github.com/attunehq/hurry/issues/170
        // This also lets us not have to worry about whether we're using a wrapper or
        // not, since non-hurry binaries will just ignore these.
        if let Some(url) = &self.api_url {
            cmd = cmd.env("HURRY_API_URL", url);
        }
        if let Some(token) = &self.api_token {
            cmd = cmd.env("HURRY_API_TOKEN", token);
        }

        // Always wait for uploads in tests to ensure artifacts are available for
        // subsequent builds.
        cmd = cmd.env("HURRY_WAIT_FOR_UPLOAD", "true");

        cmd.pwd(&self.pwd).finish()
    }

    #[instrument(skip_all)]
    async fn capture_compose(cmd: Command, container_id: &str) -> Result<Vec<Message>> {
        let output = cmd
            .run_compose_with_output(container_id)
            .await
            .context("run command in compose container")?;
        let reader = Cursor::new(&output.stdout);
        Message::parse_stream(reader)
            .map(|m| m.context("parse message"))
            .collect::<Result<Vec<_>>>()
            .context("parse messages")
            .with_section(|| output.stdout_lossy_string().header("Stdout:"))
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

    /// Add a feature to enable for the build.
    pub fn feature(mut self, feature: impl Into<String>) -> Self {
        self.features.push(feature.into());
        self
    }

    /// Add multiple features to enable for the build.
    pub fn features(mut self, features: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.features.extend(features.into_iter().map(Into::into));
        self
    }
}
