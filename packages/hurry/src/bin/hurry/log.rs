use std::{io::BufWriter, path::Path};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use atomic_time::AtomicInstant;
use clap::ValueEnum;
use color_eyre::{Result, eyre::Context as _};
use tap::Pipe;
use tracing_error::ErrorLayer;
use tracing_flame::{FlameLayer, FlushGuard};
use tracing_subscriber::{Layer as _, fmt::MakeWriter, layer::SubscriberExt as _};
use tracing_tree::time::FormatTime;

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
            let layer = tracing_tree::HierarchicalLayer::default()
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
                .with_writer(writer)
                .with_targets(false);
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

/// Prints the overall latency and latency between tracing events.
struct Uptime {
    start: Instant,
    prior: AtomicInstant,
}

impl Uptime {
    /// Get the [`Duration`] since the last time this function was called.
    /// Uses relaxed atomic ordering; this isn't meant to be super precise-
    /// just fast to run and good enough for humans to eyeball.
    ///
    /// If the function hasn't yet been called, it returns the time
    /// since the overall [`Uptime`] struct was created.
    fn elapsed_since_prior(&self) -> Duration {
        const RELAXED: Ordering = Ordering::Relaxed;
        self.prior
            .fetch_update(RELAXED, RELAXED, |_| Some(Instant::now()))
            .unwrap_or_else(|_| Instant::now())
            .pipe(|prior| prior.elapsed())
    }
}

impl Default for Uptime {
    fn default() -> Self {
        Self {
            start: Instant::now(),
            prior: AtomicInstant::now(),
        }
    }
}

impl FormatTime for Uptime {
    // Prints the total runtime for the program.
    fn format_time(&self, w: &mut impl std::fmt::Write) -> std::fmt::Result {
        let elapsed = self.start.elapsed();
        let seconds = elapsed.as_secs_f64();
        // We don't want to make users jump around to read messages, so
        // we pad the decimal part of the second.
        // Seconds going from single to double digits, then to triples,
        // will cause the overall message to shift but this isn't the same
        // as "jumping around" so it's fine.
        write!(w, "{seconds:.03}s")
    }

    // Elapsed here is the total time _in this span_,
    // but we want "the time since the last message was printed"
    // so we use `self.prior`.
    fn style_timestamp(
        &self,
        _ansi: bool,
        _elapsed: std::time::Duration,
        w: &mut impl std::fmt::Write,
    ) -> std::fmt::Result {
        // We expect the vast majority of events to be less than 999ms apart,
        // but expect a fair amount of variance between 1 and 3 digits.
        // We don't want to make users jump around to read messages, so
        // we pad with spaces to 3 characters.
        //
        // If events actually do take >999ms, we want those to stand out,
        // so it's OK that they're a bit longer.
        let elapsed = self.elapsed_since_prior().as_millis();
        write!(w, "{elapsed: >3}ms")
    }
}
