//! Shared client library for API types and HTTP clients.
//!
//! This library provides type definitions and HTTP client implementations
//! for various APIs. Types are always available, while HTTP client code
//! is gated behind feature flags.
//!
//! ## Use of `#[non_exhaustive]`
//!
//! We use `#[non_exhaustive]` on structs and enums to prevent users manually
//! constructing the types while still allowing their fields to be `pub` for
//! reading. The intention here is that users must generally construct the types
//! either by:
//! - Using constructors on the types
//! - Using builder methods
//! - Using deserialization
//!
//! We do this because some types in this module may contain invariants that
//! need to be upheld, and it's easier to ensure that all types follow these
//! guidelines in the module than do it piecemeal.

pub mod courier;

/// The latest Courier client version.
#[cfg(feature = "client")]
pub type Courier = courier::v1::Client;

/// Courier v1 client.
#[cfg(feature = "client")]
pub type CourierV1 = courier::v1::Client;
