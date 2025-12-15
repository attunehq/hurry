//! Builds Cargo projects using an optimized cache.
//!
//! Reference:
//! - `docs/DESIGN.md`
//! - `docs/development/cargo.md`

use std::time::Duration;

use clap::Args;
use color_eyre::{
    Result, Section as _, SectionExt as _,
    eyre::{Context, OptionExt as _, bail, eyre},
};
use derive_more::Debug;
use tracing::{debug, info, instrument, trace, warn};
use url::Url;
use uuid::Uuid;

use clients::Token;
use hurry::{
    cargo::{self, CargoBuildArguments, CargoCache, Workspace},
    ci::is_ci,
    daemon::{CargoUploadStatus, CargoUploadStatusRequest, CargoUploadStatusResponse, DaemonPaths},
    progress::TransferBar,
};

/// Options for `cargo build`.
//
// Hurry options are prefixed with `hurry-` to disambiguate from `cargo` args.
//
// TODO: When we implemented passthrough support for subcommands, we hid the `hurry` help
// documentation in favor of showing users Cargo's documentation; this was done in order to make it
// easier to use `hurry cargo` as an alias for `cargo` in onboarding teams. However, this might be
// confusing as teams onboard or as our set of options grows. We probably want to implement a custom
// help function that extracts the current help output from `cargo` and then blends it with `hurry`
// specific help so that users get both sets of options.
#[derive(Clone, Args, Debug)]
#[command(disable_help_flag = true)]
pub struct Options {
    /// Base URL for the Hurry API.
    #[arg(
        long = "hurry-api-url",
        env = "HURRY_API_URL",
        default_value = "https://courier.staging.corp.attunehq.com"
    )]
    #[debug("{api_url}")]
    api_url: Url,

    /// Authentication token for the Hurry API.
    // Note: this field is not _actually_ optional for `hurry` to operate; we're just telling clap
    // that it is so that if the user runs with the `-h` or `--help` arguments we can not require
    // the token in that case.
    #[arg(long = "hurry-api-token", env = "HURRY_API_TOKEN")]
    api_token: Option<Token>,

    /// Skip backing up the cache.
    #[arg(long = "hurry-skip-backup", default_value_t = false)]
    skip_backup: bool,

    /// Skip the Cargo build, only performing the cache actions.
    #[arg(long = "hurry-skip-build", default_value_t = false)]
    skip_build: bool,

    /// Skip restoring the cache.
    #[arg(long = "hurry-skip-restore", default_value_t = false)]
    skip_restore: bool,

    /// Wait for all new artifacts to upload to cache to finish before exiting.
    ///
    /// When not provided, automatically decides based on environment:
    /// - In CI, defaults to waiting.
    /// - In local development, defaults to async upload.
    /// - If desired, override CI behavior using `=false`.
    //
    // This grossly byzantine way of parsing is required to support:
    // --hurry-wait-for-upload (no arg) -> true
    // --hurry-wait-for-upload=true -> true
    // --hurry-wait-for-upload=false -> false
    //
    // Sadly this breaks if we set `require_equals` to false: clap then eagerly parses the next
    // argument and chokes if it's not present.
    #[arg(
        long = "hurry-wait-for-upload",
        env = "HURRY_WAIT_FOR_UPLOAD",
        num_args = 0..=1,
        default_value = None,
        default_missing_value = "true",
        require_equals = true,
    )]
    wait_for_upload: Option<bool>,

    /// Show help for `hurry cargo build`.
    #[arg(long = "hurry-help", default_value_t = false)]
    pub help: bool,

    /// These arguments are passed directly to `cargo build` as provided.
    #[arg(
        num_args = ..,
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "ARGS",
    )]
    argv: Vec<String>,
}

impl Options {
    /// Parse the cargo build arguments.
    #[instrument(name = "Options::parsed_args")]
    pub fn parsed_args(&self) -> CargoBuildArguments {
        CargoBuildArguments::from_iter(&self.argv)
    }

    /// Check if help is requested in the arguments.
    pub fn is_help_request(&self) -> bool {
        self.argv
            .iter()
            .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
    }
}

#[instrument]
pub async fn exec(options: Options) -> Result<()> {
    // If help is requested, passthrough directly to cargo to show cargo's help
    if options.is_help_request() {
        return cargo::invoke("build", &options.argv).await;
    }

    // We make the API token required here; if we make it required in the actual
    // clap state then we aren't able to support e.g. `cargo build -h` passthrough.
    let Some(token) = &options.api_token else {
        return Err(eyre!("Hurry API authentication token is required"))
            .suggestion("Set the `HURRY_API_TOKEN` environment variable")
            .suggestion("Provide it with the `--hurry-api-token` argument");
    };

    info!("Starting");

    // Parse and validate cargo build arguments.
    let args = options.parsed_args();
    debug!(?args, "parsed cargo build arguments");

    // Open workspace.
    let workspace = Workspace::from_argv(&args)
        .await
        .context("opening workspace")?;
    debug!(?workspace, "opened workspace");

    // Compute expected unit plans. Note that because we are not actually
    // running build scripts, these "unit plans" do not contain fully
    // unambiguous cache key information (e.g. they do not provide build script
    // outputs).
    let units = workspace
        .units(&args)
        .await
        .context("calculating expected units")?;

    // Initialize cache.
    let cache = CargoCache::open(options.api_url, token.clone(), workspace)
        .await
        .context("opening cache")?;

    // Restore artifacts.
    let unit_count = units.len() as u64;
    let restored = if !options.skip_restore {
        let progress = TransferBar::new(unit_count, "Restoring cache");
        cache.restore(&units, &progress).await?
    } else {
        Default::default()
    };

    // Run the build.
    if !options.skip_build {
        info!("Building target directory");

        // There are two integration points here that we specifically do _not_
        // use.
        //
        // # 1: Using `RUSTC_WRAPPER` to intercept `rustc` invocations
        //
        // We could intercept `rustc` invocations using `RUSTC_WRAPPER`. We
        // prototyped this, and the problem with this approach is that the
        // wrapper is only invoked when rustc itself is invoked. In particular,
        // this means that the wrapper is never invoked for crates that have
        // already been built previously. This means we can't rely on rustc
        // invocation recording to capture the rustc invocations of _all_
        // crates. We could structure the recording logs such that we can
        // quickly access recordings from previous `hurry` invocations (the
        // original log directory format was
        // `./target/hurry/rustc/<hurry_invocation_timestamp>/<unit_hash>.json`,
        // which meant we could quickly look up invocations per unit hash _and_
        // quickly see if that unit hash was present in previous `hurry` runs),
        // but this still means that we must at some point clean build every
        // crate to get its recorded invocation (i.e. users would be forced to
        // `cargo clean` the very first time they ran `hurry` in a project).
        //
        // Instead, we reconstruct the `rustc` invocation from a combination of:
        // 1. The base static invocation for a package from the build plan.
        // 2. The parsed build script outputs for a package.
        //
        // Theoretically, there is no stable interface that guarantees that this
        // will fully reconstruct the `rustc` invocation. In practice, we expect
        // this to work because we stared at the Cargo source code for building
        // `rustc` flags[^1] for a long time, and hopefully Cargo won't make big
        // changes any time soon.
        //
        // [^1]: https://github.com/rust-lang/cargo/blob/c24e1064277fe51ab72011e2612e556ac56addf7/src/cargo/core/compiler/mod.rs#L360-L375
        //
        // # 2: Reading `cargo build --message-format=json`
        //
        // `cargo build` has a flag that emits JSON messages about the build,
        // whose format is stable and documented[^2]. These messages are emitted
        // on STDOUT, so we can read them while still forwarding interactive
        // user messages that are emitted on STDERR.
        //
        // We don't use this integration point for two reasons:
        // 1. These messages don't actually give us anything that we don't already get
        //    from the build plan and build script output.
        // 2. Enabling this flag actually _changes_ the interactive user messages on
        //    STDERR. In particular, certain warnings and progress messages are
        //    different (because they are now emitted on STDOUT as JSON messages e.g.
        //    compiler errors and warnings), and we now need to add logic to manually
        //    repaint the progress bar when messages are emitted.
        //
        // It's just a whole lot of effort for no incremental value. Instead, we
        // reconstruct information from these messages using the build plan and
        // the target directory's build script outputs.
        //
        // [^2]: https://doc.rust-lang.org/cargo/reference/external-tools.html#json-messages

        // TODO: Add `RUSTC_WRAPPER` wrapper that records invocations in "debug
        // mode", so we can assert that our invocation reconstruction works
        // properly. Maybe that should be added to a test harness?

        // TODO: Maybe we can also use `strace`/`dtrace` to trace child
        // processes, and use that to determine invocation and OUT_DIR from argv
        // and environment variables?

        cargo::invoke("build", &options.argv)
            .await
            .context("build with cargo")?;

        // TODO: One thing that _would_ be interesting would be to `epoll` the
        // target directory while the build is running. Maybe information about
        // file changes in this directory tree could tell us interesting things
        // about what changed and needs to be cached.
    }

    // Cache the built artifacts.
    if !options.skip_backup {
        let upload_id = cache.save(units, restored).await?;
        if WaitForUpload::from(options.wait_for_upload).should_wait() {
            let progress = TransferBar::new(unit_count, "Uploading cache");
            wait_for_upload(upload_id, &progress).await?;
        }
    }

    Ok(())
}

#[instrument]
async fn wait_for_upload(request_id: Uuid, progress: &TransferBar) -> Result<()> {
    let paths = DaemonPaths::initialize().await?;
    let Some(daemon) = paths.daemon_running().await? else {
        bail!("daemon is not running");
    };

    let client = reqwest::Client::default();
    let endpoint = format!("http://{}/api/v0/cargo/status", daemon.url);
    let request = CargoUploadStatusRequest { request_id };
    let mut interval = tokio::time::interval(Duration::from_secs(1));

    let mut last_uploaded_artifacts = 0u64;
    let mut last_uploaded_files = 0u64;
    let mut last_uploaded_bytes = 0u64;
    let mut last_total_artifacts = 0u64;
    loop {
        interval.tick().await;
        trace!(?request, "submitting upload status request");
        let response = client
            .post(&endpoint)
            .json(&request)
            .send()
            .await
            .with_context(|| format!("send upload status request to daemon at: {endpoint}"))
            .with_section(|| format!("{daemon:?}").header("Daemon context:"))?;
        trace!(?response, "got upload status response");
        let response = response.json::<CargoUploadStatusResponse>().await?;
        trace!(?response, "parsed upload status response");
        let status = response.status.ok_or_eyre("no upload status")?;
        match status {
            CargoUploadStatus::Complete => break,
            CargoUploadStatus::InProgress(save_progress) => {
                progress.add_bytes(
                    save_progress
                        .uploaded_bytes
                        .saturating_sub(last_uploaded_bytes),
                );
                last_uploaded_bytes = save_progress.uploaded_bytes;
                progress.add_files(
                    save_progress
                        .uploaded_files
                        .saturating_sub(last_uploaded_files),
                );
                last_uploaded_files = save_progress.uploaded_files;
                progress.inc(
                    save_progress
                        .uploaded_units
                        .saturating_sub(last_uploaded_artifacts),
                );
                last_uploaded_artifacts = save_progress.uploaded_units;
                progress.dec_length(last_total_artifacts.saturating_sub(save_progress.total_units));
                last_total_artifacts = save_progress.total_units;
            }
        }
    }

    Ok(())
}

/// Control whether to wait for artifact uploads to complete.
#[derive(Clone, Copy, Debug)]
enum WaitForUpload {
    /// Automatically decide based on environment
    Auto,

    /// Wait for uploads
    ExplicitWait,

    /// Don't wait for uploads
    ExplicitAsync,
}

impl WaitForUpload {
    fn should_wait(self) -> bool {
        match self {
            Self::ExplicitWait => true,
            Self::ExplicitAsync => false,
            Self::Auto => is_ci(),
        }
    }
}

impl From<Option<bool>> for WaitForUpload {
    fn from(value: Option<bool>) -> Self {
        match value {
            None => Self::Auto,
            Some(true) => Self::ExplicitWait,
            Some(false) => Self::ExplicitAsync,
        }
    }
}
