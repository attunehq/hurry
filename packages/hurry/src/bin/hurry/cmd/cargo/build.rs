//! Builds Cargo projects using an optimized cache.
//!
//! Reference:
//! - `docs/DESIGN.md`
//! - `docs/development/cargo.md`

use color_eyre::{
    Result, Section as _,
    eyre::{Context, eyre},
};
use tracing::{debug, info, instrument};

use crate::cmd::{HurryBuildOptions, wait_for_upload};
use hurry::{
    cargo::{self, CargoCache, Workspace},
    progress::TransferBar,
};

/// Options for `cargo build`.
//
// TODO: When we implemented passthrough support for subcommands, we hid the
// `hurry` help documentation in favor of showing users Cargo's documentation;
// this was done in order to make it easier to use `hurry cargo` as an alias for
// `cargo` in onboarding teams. However, this might be confusing as teams
// onboard or as our set of options grows. We probably want to implement a
// custom help function that extracts the current help output from `cargo` and
// then blends it with `hurry` specific help so that users get both sets of
// options.
pub type Options = HurryBuildOptions;

#[instrument]
pub async fn exec(opts: Options) -> Result<()> {
    // If help is requested, passthrough directly to cargo to show cargo's help
    if opts.is_help_request() {
        return cargo::invoke("build", &opts.argv).await;
    }

    // We make the API token required here; if we make it required in the actual
    // clap state then we aren't able to support e.g. `cargo build -h` passthrough.
    let Some(token) = &opts.api_token else {
        return Err(eyre!("Hurry API authentication token is required"))
            .suggestion("Set the `HURRY_API_TOKEN` environment variable")
            .suggestion("Provide it with the `--hurry-api-token` argument");
    };

    info!("Starting");

    // Parse and validate cargo build arguments.
    let args = opts.parsed_args();
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
    let cache = CargoCache::open(opts.api_url.clone(), token.clone(), workspace)
        .await
        .context("opening cache")?;

    // Restore artifacts.
    let unit_count = units.len() as u64;
    let restored = if !opts.skip_restore {
        let progress = TransferBar::new(unit_count, "Restoring cache");
        cache.restore(&units, &progress).await?
    } else {
        Default::default()
    };

    // Run the build.
    if !opts.skip_build {
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

        cargo::invoke("build", &opts.argv)
            .await
            .context("build with cargo")?;

        // TODO: One thing that _would_ be interesting would be to `epoll` the
        // target directory while the build is running. Maybe information about
        // file changes in this directory tree could tell us interesting things
        // about what changed and needs to be cached.
    }

    // Cache the built artifacts.
    if !opts.skip_backup {
        let upload_id = cache.save(units, restored).await?;
        if !opts.async_upload {
            let progress = TransferBar::new(unit_count, "Uploading cache");
            wait_for_upload(upload_id, &progress).await?;
        }
    }

    Ok(())
}
