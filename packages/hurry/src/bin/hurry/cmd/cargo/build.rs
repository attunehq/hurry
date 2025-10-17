//! Builds Cargo projects using an optimized cache.
//!
//! Reference:
//! - `docs/DESIGN.md`
//! - `docs/development/cargo.md`

use clap::Args;
use color_eyre::{Result, eyre::Context};
use derive_more::Debug;
use humansize::{DECIMAL, format_size};
use indicatif::{ProgressBar, ProgressStyle};
use tap::Tap;
use tracing::{debug, info, instrument, warn};

use hurry::{
    cargo::{self, CargoBuildArguments, CargoCache, Profile, Workspace},
    client::Courier,
};
use url::Url;

/// Options for `cargo build`.
//
// Hurry options are prefixed with `hurry-` to disambiguate from `cargo` args.
#[derive(Clone, Args, Debug)]
pub struct Options {
    /// Base URL for the Courier instance.
    #[arg(long = "hurry-courier-url", env = "HURRY_COURIER_URL")]
    #[debug("{courier_url}")]
    courier_url: Url,

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
    /// Parse the cargo build arguments.
    #[instrument(name = "Options::parsed_args")]
    pub fn parsed_args(&self) -> CargoBuildArguments {
        CargoBuildArguments::from_iter(&self.argv)
    }
}

#[instrument]
pub async fn exec(options: Options) -> Result<()> {
    info!("Starting");

    // Parse and validate cargo build arguments.
    let args = options.parsed_args();
    debug!(?args, "parsed cargo build arguments");

    // Open workspace.
    let workspace = Workspace::from_argv(&args)
        .await
        .context("opening workspace")?;
    let profile = args.profile().map(Profile::from).unwrap_or(Profile::Debug);

    let courier = Courier::new(options.courier_url);
    courier.ping().await.context("ping courier service")?;

    // Open cache.
    let cache = CargoCache::open(courier, workspace)
        .await
        .context("opening cache")?;

    // Compute artifact plan, which provides expected artifacts. Note that
    // because we are not actually running build scripts, these "expected
    // artifacts" do not contain fully unambiguous cache key information.
    let artifact_plan = cache
        .artifact_plan(&profile, &args)
        .await
        .context("calculating expected artifacts")?;

    let progress_style = ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}")
        .context("configure progress bar")?
        .progress_chars("=> ");

    // Restore artifacts.
    let restored = if !options.skip_restore {
        let count = artifact_plan.artifacts.len() as u64;
        let progress = ProgressBar::new(count);
        progress.set_style(progress_style.clone());
        progress.set_message("Restoring cache");

        cache
            .restore(&artifact_plan, &progress)
            .await?
            .tap(|restored| {
                progress.finish_with_message(format!(
                    "Cache restored ({} files, {} transferred)",
                    restored.stats.files,
                    format_size(restored.stats.bytes, DECIMAL)
                ))
            })
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
        let count = artifact_plan.artifacts.len() as u64;
        let progress = ProgressBar::new(count);
        progress.set_style(progress_style);
        progress.set_message("Backing up cache");

        let stats = cache.save(artifact_plan, &progress, &restored).await?;
        progress.finish_with_message(format!(
            "Cache backed up ({} files, {} transferred)",
            stats.files,
            format_size(stats.bytes, DECIMAL)
        ));
    }

    Ok(())
}
