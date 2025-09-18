//! End-to-end tests for the Hurry project.
//!
//! The intention with this package is that:
//! - We use `hurry` as a CLI tool rather than as a library; just like a user.
//! - We clone or otherwise reproduce test cases with real-world projects.
//! - We use local tools on the system to do testing so that we can keep this as
//! close to a real-world usage as possible.
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

use std::sync::LazyLock;

use color_eyre::{Result, eyre::Context};
use tempfile::TempDir;
use tracing::instrument;

pub mod build;
pub mod command;
pub mod container;
pub mod ext;

pub use build::*;
pub use command::*;
pub use container::*;

static GITHUB_TOKEN: LazyLock<Option<String>> =
    LazyLock::new(|| std::env::var("GITHUB_TOKEN").ok());

#[instrument]
pub fn temporary_directory() -> Result<TempDir> {
    TempDir::new().context("create temporary directory")
}
