use clap::Parser;
use tracing::instrument;
use tracing_flame::FlameLayer;
use tracing_subscriber::{
    Layer, fmt::format::FmtSpan, layer::SubscriberExt, util::SubscriberInitExt,
};

#[derive(Parser)]
#[command(name = "hurryd")]
#[command(about = "Background daemon for `hurry`")]
struct HurryDaemonArgs {
    /// Emit flamegraph profiling data
    #[arg(short, long, hide(true))]
    profile: Option<String>,
}

#[tokio::main]
#[instrument(level = "debug")]
pub async fn main() {
    let cli = HurryDaemonArgs::parse();

    // Configure logging.
    let (flame_layer, flame_guard) = if let Some(profile) = cli.profile {
        let (flame_layer, _flame_guard) = FlameLayer::with_file(profile).unwrap();
        (Some(flame_layer), Some(_flame_guard))
    } else {
        (None, None)
    };
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
                .with_file(true)
                .with_line_number(true)
                .with_target(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .with_writer(std::io::stderr)
                .pretty()
                .with_filter(tracing_subscriber::EnvFilter::from_default_env()),
        )
        .with(flame_layer)
        .init();

    println!("Hello from hurryd!")
}
