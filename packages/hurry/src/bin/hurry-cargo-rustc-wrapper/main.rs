use std::{
    collections::BTreeMap,
    process::{ExitCode, ExitStatus},
    time::SystemTime,
};

use color_eyre::eyre::{Context, OptionExt as _};
use tap::Pipe;
use tracing::{debug, instrument, level_filters::LevelFilter, warn};
use tracing_error::ErrorLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tracing_tree::time::Uptime;

use hurry::{
    cargo::{INVOCATION_ID_ENV_VAR, INVOCATION_LOG_DIR_ENV_VAR, RawRustcInvocation},
    fs,
    path::{AbsDirPath, JoinWith as _, RelFilePath, TryJoinWith as _},
};

#[instrument]
#[tokio::main]
pub async fn main() -> ExitCode {
    ExitCode::from(match run().await {
        Ok(status) => {
            if status.success() {
                0
            } else {
                u8::try_from(status.code().unwrap_or(1)).unwrap_or(1)
            }
        }
        Err(e) => {
            eprintln!("{}", e);
            1
        }
    })
}

#[instrument]
pub async fn run() -> color_eyre::Result<ExitStatus> {
    color_eyre::install()?;
    tracing_subscriber::registry()
        .with(ErrorLayer::default())
        .with(
            tracing_tree::HierarchicalLayer::default()
                .with_indent_lines(true)
                .with_indent_amount(2)
                .with_thread_ids(false)
                .with_thread_names(false)
                .with_verbose_exit(false)
                .with_verbose_entry(false)
                .with_deferred_spans(false)
                .with_bracketed_fields(true)
                .with_span_retrace(true)
                .with_timer(Uptime::default())
                .with_targets(false),
        )
        .with(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(LevelFilter::ERROR.into())
                .from_env_lossy(),
        )
        .init();

    let argv = std::env::args().collect::<Vec<_>>();
    debug!(?argv, "invoked with args");

    // Read invocation ID from environment variable.
    let cargo_invocation_id = std::env::var(INVOCATION_ID_ENV_VAR)
        .context(format!("{} must be set", INVOCATION_ID_ENV_VAR))?;
    // TODO: Is there a way to set up the directory layout such that intercepted
    // invocations from previous runs can be reused? For example:
    //
    // ```
    // ./target/hurry/rustc/<hurry_cargo_invocation_timestamp>
    //    invocation.json
    //    rustc_invocations/
    //      <rustc_invocation_hash>.json
    // ```
    //
    // where `<rustc_invocation_hash>` is the hash of the rustc invocation argv
    // and the cargo environment variables and `invocation.json` contains the
    // arguments and environment variables set for `hurry cargo build`. This
    // way:
    //
    // 1. We can see when previous Hurry invocations have the same invocation
    //    arguments.
    // 2. We can see which previous Hurry invocation is the most recent.
    // 3. We can quickly determine whether a previously recorded rustc
    //    invocation is the same as a later one or not.
    // 4. No two different rustc invocations can ever overwrite each other
    //    within a single invocation.
    //
    // QUESTION: In this configuration, how do we tell when we have a complete
    // graph? Do we need to cross-reference against the unit graph? Or against
    // `cargo metadata`?
    //
    // Well, we only ever actually care about the _invocations_, not the
    // _outputs_, so maybe we can just store all rustc invocations and traverse
    // from the root?
    let cargo_invocation_log_dir = std::env::var(INVOCATION_LOG_DIR_ENV_VAR)
        .context(format!("{} must be set", INVOCATION_LOG_DIR_ENV_VAR))?;
    debug!(
        ?cargo_invocation_id,
        ?cargo_invocation_log_dir,
        "Hurry environment variables"
    );

    // Note that we cannot use `Workspace::from_argv` here to get the `target`
    // directory location, because it invokes `cargo metadata`. This causes an
    // infinite co-recursive loop, where running the wrapper calls `cargo
    // metadata`, which calls the wrapper (to call `rustc -vV`), which calls
    // `cargo metadata`, etc.
    let invocation_cache = AbsDirPath::try_from(cargo_invocation_log_dir)
        .context("invalid cargo invocation log dir")?
        .try_join_dir(&cargo_invocation_id)
        .context("invalid cargo invocation cache dirname")?;
    let cargo_envs = std::env::vars()
        .filter(|(key, _)| key == "OUT_DIR" || key.starts_with("CARGO_"))
        .collect::<BTreeMap<_, _>>();
    let invocation_name =
        RelFilePath::try_from(format!("{}.json", uuid::Uuid::new_v4().to_string()))
            .expect("UUID should be a valid filename");
    fs::write(
        &invocation_cache.join(invocation_name),
        serde_json::to_string_pretty(&RawRustcInvocation {
            timestamp: SystemTime::now(),
            invocation: argv.clone(),
            env: cargo_envs,
            cwd: std::env::current_dir()
                .context("getting current directory")?
                .to_string_lossy()
                .to_string(),
        })
        .context("serializing rustc invocation")?,
    )
    .await
    .context("writing RUSTC_WRAPPER invocation")?;

    // Invoke `rustc`.
    let mut argv = argv.into_iter();
    let wrapper = argv
        .next()
        .ok_or_eyre("expected RUSTC_WRAPPER as argv[0]")?;
    if wrapper != "hurry-cargo-rustc-wrapper" {
        warn!(
            "RUSTC_WRAPPER is not `hurry-cargo-rustc-wrapper`: {:?}",
            wrapper
        );
    }
    let rustc = argv.next().ok_or_eyre("expected rustc as argv[1]")?;
    debug!(?rustc, ?argv, "invoking rustc");
    let mut cmd = tokio::process::Command::new(rustc);
    cmd.args(argv);
    // TODO: Handle the case where the user has intentionally set a
    // RUSTC_WRAPPER, which we then need to pass on to `rustc`.
    cmd.spawn()
        .context("could not spawn rustc")?
        .wait()
        .await
        .context("could not complete rustc execution")?
        .pipe(Ok)
}
