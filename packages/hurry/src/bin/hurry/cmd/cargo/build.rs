//! Builds Cargo projects using an optimized cache.
//!
//! Reference:
//! - `docs/DESIGN.md`
//! - `docs/development/cargo.md`

use std::{collections::HashMap, ffi::OsStr, fmt::Debug, process::Stdio};

use cargo_metadata::{Artifact, PackageId};
use clap::Args;
use color_eyre::{Result, eyre::Context, owo_colors::OwoColorize};
use hurry::{
    Locked,
    cache::{FsCache, FsCas},
    cargo::{
        self, CargoCache, Handles, INVOCATION_ID_ENV_VAR, INVOCATION_LOG_DIR_ENV_VAR,
        Optimizations, Profile, RawRustcInvocation, Workspace, cache_target_from_workspace,
        invocation_log_dir, restore_target_from_cache,
    },
    fs,
    path::{AbsFilePath, TryJoinWith},
};
use tokio::io::AsyncBufReadExt;
use tokio_stream::{StreamExt, wrappers::ReadDirStream};
use tracing::{debug, error, info, instrument, warn};

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

    let workspace = Workspace::from_argv(&options.argv)
        .await
        .context("open workspace")?;
    let profile = options.profile();
    let cas = FsCas::open_default().await.context("opening CAS")?;
    let cache = CargoCache::open_default().await.context("opening cache")?;

    // TODO: Precompute which dependencies we expect to retrieve from cache, and
    // which we expect to provide to cache afterwards.

    // if !options.skip_restore {
    //     info!(?cache, "Restoring target directory from cache");
    //     let target = workspace
    //         .open_profile_locked(&profile)
    //         .await
    //         .context("open profile")?;

    //     let restore = restore_target_from_cache(cas, cache, &target, |key, dependency| {
    //         info!(
    //             name = %dependency.package_name,
    //             version = %dependency.version,
    //             target = %dependency.target,
    //             %key,
    //             "Restored dependency from cache",
    //         )
    //     });
    //     match restore.await {
    //         Ok(_) => info!("Restored cache"),
    //         Err(error) => warn!(?error, "Failed to restore cache"),
    //     }
    // }

    // After restoring the target directory from cache,
    // or if we never had a cache, we need to build it-
    // this is because we currently only cache based on lockfile hash;
    // if the first-party code has changed we'll need to rebuild.
    if !options.skip_build {
        // Ensure that the Hurry build cache within `target` is created for the
        // invocation, and that the build is run with the Hurry wrapper.
        let cargo_invocation_id = uuid::Uuid::new_v4().to_string();
        let cargo_invocation_log_dir = invocation_log_dir(&workspace.target);
        fs::create_dir_all(&cargo_invocation_log_dir)
            .await
            .context("create build-scoped Hurry cache")?;

        let mut build_args = options.argv;
        // TODO: Handle the case where this flag is already set.
        build_args.push(String::from(
            "--message-format=json-diagnostic-rendered-ansi",
        ));

        info!("Building target directory");
        // TODO: Handle the case where the user has already defined a
        // `RUSTC_WRAPPER` (e.g. if they're using `sccache`).
        //
        // TODO: Figure out how to properly distribute the wrapper. Maybe we'll
        // embed it into the binary, and write it out? See example[^1].
        //
        // [^1]: https://zameermanji.com/blog/2021/6/17/embedding-a-rust-binary-in-another-rust-binary/
        let mut child = cargo::invoke_with(
            "build",
            &build_args,
            [
                ("RUSTC_WRAPPER", "hurry-cargo-rustc-wrapper".as_ref()),
                (INVOCATION_ID_ENV_VAR, cargo_invocation_id.as_ref()),
                (
                    INVOCATION_LOG_DIR_ENV_VAR,
                    cargo_invocation_log_dir.as_os_str(),
                ),
            ],
            Handles {
                stdout: Stdio::piped(),
                stderr: Stdio::inherit(),
            },
        )
        .await?;

        // Read the compiler output messages so we know the features (for cache
        // keying reasons).
        let reader = tokio::io::BufReader::new(child.stdout.take().unwrap());
        let mut lines = reader.lines();
        let mut artifact_messages: HashMap<PackageId, Vec<Artifact>> = HashMap::new();
        while let Some(line) = lines.next_line().await? {
            let message = serde_json::from_str::<cargo_metadata::Message>(&line)?;
            match message {
                cargo_metadata::Message::CompilerArtifact(artifact) => {
                    // There can be multiple artifact messages with the same
                    // package ID. Why is this?
                    //
                    // 1. The compilation of the build script binary is itself a
                    //    rustc invocation that results in a compiler-artifact
                    //    message.
                    // 2. Library crates with the same name and version can be
                    //    compiled multiple times if they need to be compiled
                    //    with different features (e.g. if the project uses
                    //    `resolver = "2"` (or equivalently `edition = "2021"`),
                    //    or has an upstream dependency which does).
                    // 3. Library crates with the same name, version, and
                    //    features can be compiled multiple times if they need
                    //    to be linked against different upstream dependencies
                    //    (e.g. if the upstream dependencies themselves have
                    //    multiple compiled instances because of (2)).
                    //
                    // If we wanted to create a map that uniquely keys on all of
                    // these, we would need to key on (package_id, features,
                    // filenames). Note that we cannot key on upstream
                    // dependencies (and instead rely on the stable package hash
                    // in the filename to capture this) because the compiler
                    // message does not actually _include_ the upstream
                    // dependencies (we must get these from the rustc
                    // invocation). This is a pretty annoying map to work with,
                    // so we just key by package ID and eat the `.find` cost for
                    // now.
                    if artifact_messages.contains_key(&artifact.package_id) {
                        artifact_messages
                            .get_mut(&artifact.package_id)
                            .expect("artifacts map should contain checked package ID")
                            .push(artifact);
                    } else {
                        artifact_messages.insert(artifact.package_id.clone(), vec![artifact]);
                    }
                }
                cargo_metadata::Message::BuildScriptExecuted(_) => {
                    // TODO: Parse the build script output and key on it?
                }
                cargo_metadata::Message::BuildFinished(_) => {
                    // TODO: Handle the case where the build failed.
                    break;
                }
                // Ignore - these are compiler warnings and errors.
                cargo_metadata::Message::CompilerMessage(message) => {
                    // TODO: This doesn't actually quite do the right thing when
                    // the compiler emits error messages. In particular:
                    //
                    // 1. The stock `cargo` output synchronizes output log
                    //    messages with repainting the progress bar. This
                    //    current implementation does not repaint the progress
                    //    bar, so message output on STDERR will sometimes
                    //    incorrectly interleave with progress bar output. I
                    //    think the right way to fix this is to pipe STDERR in
                    //    addition to STDOUT and add a thread that manages
                    //    STDERR painting for us. There, we should repaint
                    //    STDERR's progress bar whenever we need to emit an
                    //    error message. It helps that the progress bar is
                    //    always the _last_ line in the current output, so we
                    //    should be able to just clear the last line, print the
                    //    compiler message, and then redraw the last line (i.e.
                    //    the progress bar).
                    // 2. There seem to be some `cargo` stock output messages
                    //    that are not emitted as JSON messages. In particular,
                    //    the final "warning: `package` generated N warnings"
                    //    message is not present.
                    //
                    // Note that these differences stem primarily from
                    // `--message-format=json`. To see for yourself how the
                    // messages are different, compare the outputs of `cargo
                    // build >/dev/null` and `cargo build --message-format=json
                    // >/dev/null`.
                    match message.message.rendered {
                        Some(rendered) => eprint!("{}", rendered),
                        None => warn!(?message, "unrenderable compiler message"),
                    }
                }
                // Ignore - these are unparseable lines.
                _ => (),
            }
        }

        // TODO: Handle the case where the build fails. Maybe bail here?
        let _ = child
            .wait()
            .await
            .context("Couldn't get cargo's exit status")?;

        // Backup is nested with build right now because we need rustc
        // invocations to disambiguate artifacts from dependencies with multiple
        // versions in the same build. _Technically_ we could do a partial
        // backup even without these invocations, but let's not add that
        // complexity until users start asking for "skip build but do backup".
        //
        // TODO: watch and cache the target directory as the build occurs rather
        // than having to copy it all at the end?
        if !options.skip_backup {
            info!("Caching built target directory");
            let target = workspace
                .open_profile_locked(&profile)
                .await
                .context("open profile")?;

            // TODO: Consider reading the unit graph from `RUSTC_BOOTSTRAP=1
            // cargo build --unit-graph -Z unstable-options`. This output gives
            // us the graph of units-as-in-targets, including handling cases
            // where a package is built multiple times with different
            // features[^1], although it still does not give us artifact names.
            //
            // [^1]: https://doc.rust-lang.org/cargo/reference/unstable.html#unit-graph

            // Read `rustc` invocations from newly built packages. These
            // packages should be cached, because they were rebuilt, which
            // implies that they were not originally cached.
            //
            // We need to read these invocations because they tell us what
            // dependencies are used in each library crate's build (via the
            // `--extern` flag) since the compiler output messages don't
            // actually tell us this information.
            let mut invocations = ReadDirStream::new(
                fs::read_dir(&cargo_invocation_log_dir.try_join_dir(&cargo_invocation_id)?)
                    .await
                    .context("reading rustc invocation dir")?,
            );
            while let Some(invocation) = invocations.next().await {
                let invocation = invocation.context("statting rustc invocation")?;
                let path = AbsFilePath::try_from(invocation.path())?;
                // debug!(?path, "rustc invocation path");
                let contents = fs::must_read_buffered_utf8(&path)
                    .await
                    .context("reading rustc invocation")?;
                let invocation: RawRustcInvocation =
                    serde_json::from_str(&contents).context("parsing rustc invocation")?;
                debug!(?invocation, "parsed invocation");
            }

            // workspace
            //     .dependencies
            //     .iter()
            //     .map(|dependency| {})
            //     .collect::<Vec<_>>();

            // let backup = cache_target_from_workspace(cas, cache, &target, |key, dependency| {
            //     info!(
            //         name = %dependency.package_name,
            //         version = %dependency.version,
            //         target = %dependency.target,
            //         %key,
            //         "Updated dependency in cache",
            //     )
            // });
            // match backup.await {
            //     Ok(_) => info!("Cached target directory"),
            //     Err(error) => warn!(?error, "Failed to cache target"),
            // }
        }
    }

    Ok(())
}
