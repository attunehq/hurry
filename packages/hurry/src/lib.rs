//! Library for `hurry`.
//!
//! This library is not intended to be used directly and is unsupported in
//! that configuration. It's only a library to enable sharing code in `hurry`
//! with benchmarks and integration tests in the `hurry` repository.

use std::time::Instant;

use derive_more::Display;
use humansize::{DECIMAL, format_size};

pub mod cargo;
pub mod cas;
pub mod client;
pub mod ext;
pub mod fs;
pub mod hash;
pub mod path;

/// The associated type's state is unlocked.
/// Used for the typestate pattern.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display, Default)]
pub struct Unlocked;

/// The associated type's state is locked.
/// Used for the typestate pattern.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display, Default)]
pub struct Locked;

/// Formats the transfer rate as a string like "10 MB/s".
///
/// Returns "0 MB/s" if:
/// - Elapsed time is zero.
/// - Transferred bytes are zero.
pub fn format_transfer_rate(bytes: u64, start_time: Instant) -> String {
    let elapsed = start_time.elapsed().as_secs_f64();
    let size = if elapsed > 0.0 && bytes > 0 {
        format_size((bytes as f64 / elapsed) as u64, DECIMAL)
    } else {
        String::from("0 MB")
    };
    format!("{size}/s")
}
