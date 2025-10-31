use std::time::Instant;
use std::{io::BufWriter, path::Path};

use clap::ValueEnum;
use color_eyre::{Result, eyre::Context as _};
use tracing_error::ErrorLayer;
use tracing_flame::{FlameLayer, FlushGuard};
use tracing_subscriber::fmt::format::{FmtSpan, Writer};
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::{Layer as _, fmt::MakeWriter, layer::SubscriberExt as _};

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum WhenColor {
    Always,
    Never,
    Auto,
}

pub fn make_logger<W>(
    writer: W,
    profile: Option<impl AsRef<Path>>,
    color: WhenColor,
) -> Result<(
    impl tracing::Subscriber,
    Option<FlushGuard<BufWriter<std::fs::File>>>,
)>
where
    W: for<'writer> MakeWriter<'writer> + 'static,
{
    let (flame_layer, flame_guard) = if let Some(profile) = profile {
        let profile = profile.as_ref();
        FlameLayer::with_file(profile)
            .with_context(|| format!("set up profiling to {profile:?}"))
            .map(|(layer, guard)| (Some(layer), Some(guard)))?
    } else {
        (None, None)
    };

    let logger = tracing_subscriber::registry()
        .with(ErrorLayer::default())
        .with({
            let layer = tracing_subscriber::fmt::layer()
                .with_level(true)
                .with_file(true)
                .with_line_number(true)
                .with_target(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
                .with_timer(Uptime::default())
                .with_writer(writer)
                .pretty();
            match color {
                WhenColor::Always => layer.with_ansi(true),
                WhenColor::Never => layer.with_ansi(false),
                WhenColor::Auto => layer,
            }
            .with_filter(
                tracing_subscriber::EnvFilter::builder()
                    .with_env_var("HURRY_LOG")
                    .from_env_lossy(),
            )
        })
        .with(flame_layer);

    Ok((logger, flame_guard))
}

struct Uptime {
    start: Instant,
}

impl Default for Uptime {
    fn default() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl FormatTime for Uptime {
    // Prints the total runtime for the program.
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        let elapsed = self.start.elapsed();
        let seconds = elapsed.as_secs_f64();
        // We don't want to make users jump around to read messages, so
        // we pad the decimal part of the second.
        // Seconds going from single to double digits, then to triples,
        // will cause the overall message to shift but this isn't the same
        // as "jumping around" so it's fine.
        write!(w, "{seconds:.03}s")
    }
}
