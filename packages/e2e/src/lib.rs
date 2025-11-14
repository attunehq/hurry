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
//! ## Test Infrastructure
//!
//! Tests use testcontainers-rs with Docker Compose to manage test environments.
//! The [`TestEnv`] type provides a managed test environment with:
//! - PostgreSQL database
//! - Courier API service
//! - Multiple hurry containers for cross-container testing
//! - Automatic cleanup on drop
//!
//! Use [`Command::run_compose()`] to execute commands in compose containers.
//!
//! ## Tracing
//!
//! Remember that the tracing system is only emitted in test logs; as such you
//! probably want to "up-level" your tracing call levels. For example, things
//! that are `info!` will still only be emitted in test logs since this library
//! is only used in tests.

use std::sync::LazyLock;

pub mod build;
pub mod command;
pub mod container;
pub mod env;
pub mod ext;

pub use build::*;
pub use command::*;
pub use env::*;

static GITHUB_TOKEN: LazyLock<Option<String>> =
    LazyLock::new(|| std::env::var("GITHUB_TOKEN").ok());
