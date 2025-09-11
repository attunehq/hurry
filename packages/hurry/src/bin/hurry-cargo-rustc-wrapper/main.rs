use std::{collections::HashMap, time::SystemTime};

use color_eyre::{
    Result,
    eyre::{Context, OptionExt as _, bail},
};
use serde::Serialize;
use tracing::{debug, instrument, level_filters::LevelFilter, warn};
use tracing_error::ErrorLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tracing_tree::time::Uptime;

use hurry::{
    fs,
    path::{AbsDirPath, JoinWith as _, RelFilePath, TryJoinWith as _},
};

#[derive(Serialize)]
struct RustcInvocation {
    timestamp: SystemTime,
    invocation: Vec<String>,
    env: HashMap<String, String>,
    cwd: String,
}

#[instrument]
#[tokio::main]
pub async fn main() -> Result<()> {
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
                .with_deferred_spans(true)
                .with_bracketed_fields(true)
                .with_span_retrace(true)
                .with_timer(Uptime::default())
                .with_targets(false),
        )
        .with(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .init();

    let argv = std::env::args().collect::<Vec<_>>();
    debug!(?argv, "invoked with args");

    // Read invocation ID from environment variable.
    let cargo_invocation_id = std::env::var("HURRY_CARGO_INVOCATION_ID")
        .context("HURRY_CARGO_INVOCATION_ID must be set")?;
    let cargo_invocation_root = std::env::var("HURRY_CARGO_INVOCATION_ROOT")
        .context("HURRY_CARGO_INVOCATION_ROOT must be set")?;
    debug!(
        ?cargo_invocation_id,
        ?cargo_invocation_root,
        "input environment variables"
    );

    // Write `rustc` invocation.
    let rustc_invocation_id = uuid::Uuid::new_v4();
    // Note that we cannot use `Workspace::from_argv` here because it invokes
    // `cargo metadata`. This causes an infinite co-recursive loop, where
    // running the wrapper calls `cargo metadata`, which calls the wrapper (to
    // use `rustc`), which calls `cargo metadata`, etc.
    let invocation_cache = AbsDirPath::try_from(cargo_invocation_root)
        .context("invalid cargo invocation root")?
        .try_join_dirs(vec!["target", "hurry", "invocations", &cargo_invocation_id])
        .context("invalid cargo invocation cache dirname")?;
    fs::write(
        &invocation_cache.join(
            RelFilePath::try_from(format!(
                "{}-{}-{}.json",
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .expect("current time is after Unix epoch")
                    .as_secs(),
                std::process::id(),
                rustc_invocation_id,
            ))
            .expect("rustc invocation filename should be a valid filename"),
        ),
        serde_json::to_string_pretty(&RustcInvocation {
            timestamp: SystemTime::now(),
            invocation: argv.clone(),
            env: std::env::vars().collect(),
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
    let status = cmd
        .spawn()
        .context("could not spawn rustc")?
        .wait()
        .await
        .context("could not complete rustc execution")?;
    if status.success() {
        Ok(())
    } else {
        bail!("rustc exited with status: {status}");
    }
}
