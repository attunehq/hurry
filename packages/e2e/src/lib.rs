//! End-to-end tests for the Hurry project.
//!
//! The intention with this package is that:
//! - We use `hurry` as a CLI tool rather than as a library; just like a user.
//! - We clone or otherwise reproduce test cases with real-world projects.
//! - Prioritize real-world usage as much as possible.
//! - This also serves as backwards compatibility checks for users.
//!
//! All tests are implemented as integration tests in the `tests/` directory;
//! this library crate for the `e2e` package provides shared functionality and
//! utilities for the tests.
//!
//! ## Tracing
//!
//! Remember that the tracing system is only emitted in test logs; as such you
//! probably want to "up-level" your tracing call levels. For example, things
//! that are `info!` will still only be emitted in test logs since this library
//! is only used in tests.

use std::{fmt::Debug, path::Path, sync::LazyLock, time::SystemTime};

use color_eyre::{
    Result,
    eyre::{Context, bail},
};
use filetime::FileTime;
use tempfile::TempDir;
use tracing::instrument;

pub mod build;
pub mod command;
pub mod container;
pub mod env;
pub mod ext;

pub use build::*;
pub use command::*;
pub use container::*;
pub use env::*;
use walkdir::WalkDir;

static GITHUB_TOKEN: LazyLock<Option<String>> =
    LazyLock::new(|| std::env::var("GITHUB_TOKEN").ok());

/// Create a temporary directory.
#[instrument]
pub fn temporary_directory() -> Result<TempDir> {
    TempDir::new().context("create temporary directory")
}

/// Set the mtime of all files in `dir` to the current time, recursively.
#[instrument]
pub fn set_mtime(dir: impl AsRef<Path> + Debug, mtime: SystemTime) -> Result<()> {
    let dir = dir.as_ref();
    let mtime = FileTime::from_system_time(mtime);
    WalkDir::new(dir)
        .into_iter()
        .try_for_each(|entry| -> Result<()> {
            let entry = entry.context("walk directory")?;
            let path = entry.path();
            filetime::set_file_mtime(path, mtime).with_context(|| format!("set mtime for {path:?}"))
        })
}

/// Copy the contents of `src` to `dst`.
/// Returns the number of files copied and the number of bytes copied.
///
/// ```ignore
/// let (file_count, byte_count) = copy_dir(src, dst)?;
/// ```
#[instrument]
pub fn copy_dir(
    src: impl AsRef<Path> + Debug,
    dst: impl AsRef<Path> + Debug,
) -> Result<(u64, u64)> {
    let src_root = src.as_ref();
    let dst_root = dst.as_ref();
    let mut file_count = 0u64;
    let mut byte_count = 0u64;
    WalkDir::new(src_root)
        .into_iter()
        .try_for_each(|entry| -> Result<()> {
            let entry = entry.context("walk directory")?;

            let src = entry.path();
            let rel = src
                .strip_prefix(src_root)
                .with_context(|| format!("strip prefix {src_root:?} from {src:?}"))?;
            let dst = dst_root.join(rel);

            let kind = entry.file_type();
            if kind.is_dir() {
                std::fs::create_dir_all(&dst)
                    .with_context(|| format!("create parent directory {dst:?}"))
            } else if kind.is_file() {
                let bytes = std::fs::copy(src, &dst)
                    .with_context(|| format!("copy file {src:?} to {dst:?}"))?;
                byte_count += bytes;
                file_count += 1;
                Ok(())
            } else {
                bail!("unexpected file type for {src:?}: {kind:?}");
            }
        })?;
    Ok((file_count, byte_count))
}
