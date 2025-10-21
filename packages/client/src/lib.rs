//! Shared client library for API types and HTTP clients.
//!
//! This library provides type definitions and HTTP client implementations
//! for various APIs. Types are always available, while HTTP client code
//! is gated behind feature flags.

pub mod courier;

/// The latest Courier client version.
#[cfg(feature = "client")]
pub type Courier = courier::v1::Client;

/// Courier v1 client.
#[cfg(feature = "client")]
pub type CourierV1 = courier::v1::Client;

// Future:
// pub mod github;
// pub type GitHub = github::v1::Client;
