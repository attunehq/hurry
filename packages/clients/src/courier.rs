//! Courier API client types and HTTP client.

pub mod v1;

/// An authentication token for Courier API access.
///
/// Re-exported from the root module for backwards compatibility.
/// See [`crate::Token`] for documentation.
pub use crate::Token;
