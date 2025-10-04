//! Builds Cargo projects using an optimized cache.
//!
//! Reference:
//! - `docs/DESIGN.md`
//! - `docs/development/cargo.md`

use std::{
    collections::HashMap,
    ffi::OsStr,
    fmt::Debug,
    process::Stdio,
    time::{SystemTime, UNIX_EPOCH},
};

use cargo_metadata::{Artifact, PackageId};
use clap::Args;
use color_eyre::{Result, eyre::Context};
use hurry::{
    cargo::{
        self, CargoCache, Handles, INVOCATION_LOG_DIR_ENV_VAR, Profile, Workspace,
        invocation_log_dir,
    },
    cas::FsCas,
    fs,
    path::TryJoinWith as _,
};
use tokio::io::AsyncBufReadExt;
use tracing::{error, info, instrument, warn};

/// Options for `cargo build`.
//
// Hurry options are prefixed with `hurry-` to disambiguate from `cargo` args.
#[derive(Clone, Args, Debug)]
pub struct Options {
    /// Skip backing up the cache.
    #[arg(long = "hurry-skip-backup", default_value_t = false)]
    skip_backup: bool,

    /// Skip the Cargo build, only performing the cache actions.
    #[arg(long = "hurry-skip-build", default_value_t = false)]
    skip_build: bool,

    /// Skip restoring the cache.
    #[arg(long = "hurry-skip-restore", default_value_t = false)]
    skip_restore: bool,

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
    /// Get the profile specified by the user.
    #[instrument(name = "Options::profile")]
    pub fn profile(&self) -> Profile {
        Profile::from_argv(&self.argv)
    }
}

#[instrument]
pub async fn exec(options: Options) -> Result<()> {
    info!("Starting");

    // Open workspace.
    let workspace = Workspace::from_argv(&options.argv)
        .await
        .context("opening workspace")?;
    let profile = options.profile();

    // Open backing storage services.
    let cas = FsCas::open_default().await.context("opening CAS")?;
    let cache = CargoCache::open_default(workspace)
        .await
        .context("opening cache")?;

    // Compute expected artifacts.
    let artifacts = cache
        .artifacts(&profile)
        .await
        .context("calculating expected artifacts")?;

    // TODO: Restore artifacts.

    // Run the build.
    if !options.skip_build {
        info!("Building target directory");

        // Record `rustc` invocations. We need these invocations in order to
        // properly key the cache against `rustc` flags.
        //
        // These flags cannot be read out of the build plan or unit graph
        // because they are not known until build scripts are executed, and they
        // can't be read out of the build JSON messages because the message
        // format does not include this information.
        //
        // We record these invocations by using a `rustc` wrapper that writes
        // the invocation and immediately delegates to the real `rustc`. The
        // invocation logging directory format is:
        //
        // ```
        // ./target/<profile>/hurry/rustc/<hurry_invocation_timestamp>/<rustc_unit_hash>.json
        // ```
        //
        // This format allows us to quickly find the `rustc` invocation for a
        // particular unit hash, and allows us to quickly search _previous_
        // `hurry` invocations to see whether a recorded `rustc` invocation is
        // available for a particular unit. This "quick historical search"
        // capability is important because `rustc` is not invoked for crates
        // that have _already been compiled_, and because cargo does not record
        // the `rustc` invocation anywhere else (in particular, cargo _does_
        // replay old build JSON messages, but these messages do not contain
        // information about `rustc` flags).
        //
        // TODO: Is this really necessary? Can we just reconstruct the
        // invocation by parsing the build script output for each unit and
        // simulating the `rustc` invocation construction?
        let cargo_invocation_log_dir = {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("current time should be after Unix epoch");
            cache
                .ws
                .open_profile_locked(&profile)
                .await
                .context("opening target directory")?
                .root()
                .try_join_dirs(["hurry", "rustc", &timestamp.as_nanos().to_string()])
                .expect("rustc invocation log dir should be valid")
        };
        fs::create_dir_all(&cargo_invocation_log_dir)
            .await
            .context("create build-scoped Hurry cache")?;

        let mut child = cargo::invoke_with(
            "build",
            &options.argv,
            [
                ("RUSTC_WRAPPER", "hurry-cargo-rustc-wrapper".as_ref()),
                (
                    INVOCATION_LOG_DIR_ENV_VAR,
                    cargo_invocation_log_dir.as_os_str(),
                ),
            ],
            Handles {
                stdout: Stdio::inherit(),
                stderr: Stdio::inherit(),
            },
        )
        .await
        .context("build with cargo")?;

        // TODO: Handle the case where the build fails. Maybe bail here?
        let result = child
            .wait()
            .await
            .context("Couldn't get cargo's exit status")?;

        // TODO: Read the build script output from the build folders, and parse the output for directives.
    }

    // Cache the built artifacts.
    if !options.skip_backup {
        info!("Caching built artifacts");
    }
    todo!()
}
