//! Progress bar utilities for interactive and CI environments.

use std::borrow::Cow;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use derive_more::Deref;
use indicatif::{HumanDuration, ProgressBar, ProgressStyle};

/// A progress bar wrapper that emits periodic updates.
///
/// - In interactive terminals, displays a normal progress bar.
/// - In non-interactive environments emits log lines every 5 seconds.
#[derive(Deref)]
pub struct TransferBar {
    #[deref]
    progress: ProgressBar,
    start: Instant,
    handle: Option<JoinHandle<()>>,
    signal: Option<Arc<StopSignal>>,
}

impl TransferBar {
    /// Creates a new transfer progress tracker.
    pub fn new(items: u64, message: impl Into<Cow<'static, str>>) -> Self {
        let progress = ProgressBar::new(items);
        let style = ProgressStyle::default_bar()
            .template("[{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .expect("invalid progress bar template")
            .progress_chars("=> ");
        progress.set_style(style);
        progress.set_message(message);

        let start = Instant::now();
        if is_interactive() {
            Self {
                progress,
                start,
                handle: None,
                signal: None,
            }
        } else {
            let signal = StopSignal::new();
            let handle = thread::spawn({
                let progress = progress.clone();
                let signal = signal.clone();
                move || {
                    // Log immediately on start
                    let elapsed = HumanDuration(start.elapsed());
                    let pos = progress.position();
                    let len = progress.length().unwrap_or(0);
                    let msg = progress.message();
                    progress.suspend(|| {
                        println!("[{elapsed}] [{pos}/{len}] {msg}");
                    });

                    // Log every interval.
                    let interval = Duration::from_secs(5);
                    loop {
                        if signal.wait_timeout(interval) {
                            break;
                        }

                        if progress.is_finished() {
                            break;
                        }

                        let elapsed = HumanDuration(start.elapsed());
                        let pos = progress.position();
                        let len = progress.length().unwrap_or(0);
                        let msg = progress.message();
                        progress.suspend(|| {
                            println!("[{elapsed}] [{pos}/{len}] {msg}");
                        });
                    }
                }
            });
            Self {
                progress,
                start,
                handle: Some(handle),
                signal: Some(signal),
            }
        }
    }
}

impl Drop for TransferBar {
    fn drop(&mut self) {
        // Signal the logging thread to stop and wake it up
        if let Some(signal) = &self.signal {
            signal.stop();
        }

        // Wait for the logging thread to complete if it exists
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }

        // In non-interactive mode, log the final state immediately
        if !is_interactive() {
            let elapsed = HumanDuration(self.start.elapsed());
            let pos = self.progress.position();
            let len = self.progress.length().unwrap_or(0);
            let msg = self.progress.message();
            self.progress.suspend(|| {
                println!("[{elapsed}] [{pos}/{len}] {msg}");
            });
        }
    }
}

/// A simple signal for stopping a thread using a condition variable.
struct StopSignal {
    stopped: Mutex<bool>,
    condvar: Condvar,
}

impl StopSignal {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            stopped: Mutex::new(false),
            condvar: Condvar::new(),
        })
    }

    /// Wait for the signal or timeout. Returns true if signaled to stop.
    fn wait_timeout(&self, timeout: Duration) -> bool {
        let stopped = self.stopped.lock().expect("mutex is poisoned");
        let (stop, _) = self
            .condvar
            .wait_timeout(stopped, timeout)
            .expect("mutex is poisoned");
        *stop
    }

    /// Signal the thread to stop.
    fn stop(&self) {
        let mut stopped = self.stopped.lock().unwrap();
        *stopped = true;
        self.condvar.notify_one();
    }
}

/// Detects if running in an interactive terminal environment.
fn is_interactive() -> bool {
    console::Term::stderr().is_term()
}

/// Formats the transfer amount as a string like "10 MB".
pub fn format_size(bytes: u64) -> String {
    humansize::format_size(bytes, humansize::DECIMAL)
}

/// Formats the transfer rate as a string like "10 MB/s".
///
/// Returns "0 MB/s" if:
/// - Elapsed time is zero.
/// - Transferred bytes are zero.
pub fn format_transfer_rate(bytes: u64, start_time: Instant) -> String {
    let elapsed = start_time.elapsed().as_secs_f64();
    let size = if elapsed > 0.0 && bytes > 0 {
        format_size((bytes as f64 / elapsed) as u64)
    } else {
        String::from("0 MB")
    };
    format!("{size}/s")
}
