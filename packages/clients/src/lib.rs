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

use derive_more::Display;
use enum_assoc::Assoc;
use http::header::{self, HeaderName, HeaderValue};

pub mod courier;

/// The default buffer size used by the client and server.
///
/// We're sending relatively large chunks over the network, so we think this is
/// a good buffer size to use, but haven't done a lot of testing with different
/// sizes. Note that if you're piping content between tasks or threads (e.g.
/// using `piper::pipe`) you probably want to use this value over
/// [`LOCAL_BUFFER_SIZE`]; this seems to make a significant difference in
/// benchmarks.
pub const NETWORK_BUFFER_SIZE: usize = 1024 * 1024;

/// The default buffer size for static local buffers, e.g. when hashing files.
/// The goal with this is to allow things like SIMD operations but not be so
/// large that the buffer is unwieldy or too expensive.
///
/// We think this is a good buffer size to use, but haven't done a lot of
/// testing with different sizes.
pub const LOCAL_BUFFER_SIZE: usize = 16 * 1024;

/// The latest Courier client version.
#[cfg(feature = "client")]
pub type Courier = courier::v1::Client;

/// Courier v1 client.
#[cfg(feature = "client")]
pub type CourierV1 = courier::v1::Client;

/// Content types used by the library.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Display, Assoc)]
#[func(pub const fn value(&self) -> HeaderValue)]
#[func(pub const fn to_str(&self) -> &'static str)]
#[display("{}", self.to_str())]
pub enum ContentType {
    #[assoc(to_str = "application/x-tar")]
    #[assoc(value = HeaderValue::from_static(self.to_str()))]
    Tar,

    #[assoc(to_str = "application/x-tar+zstd")]
    #[assoc(value = HeaderValue::from_static(self.to_str()))]
    TarZstd,

    #[assoc(to_str = "application/octet-stream")]
    #[assoc(value = HeaderValue::from_static(self.to_str()))]
    Bytes,

    #[assoc(to_str = "application/octet-stream+zstd")]
    #[assoc(value = HeaderValue::from_static(self.to_str()))]
    BytesZstd,

    #[assoc(to_str = "application/json")]
    #[assoc(value = HeaderValue::from_static(self.to_str()))]
    Json,
}

impl ContentType {
    pub const HEADER: HeaderName = header::CONTENT_TYPE;
    pub const ACCEPT: HeaderName = header::ACCEPT;
}

impl PartialEq<ContentType> for HeaderValue {
    fn eq(&self, other: &ContentType) -> bool {
        self == other.value()
    }
}

impl PartialEq<ContentType> for &HeaderValue {
    fn eq(&self, other: &ContentType) -> bool {
        *self == other.value()
    }
}

impl PartialEq<HeaderValue> for ContentType {
    fn eq(&self, other: &HeaderValue) -> bool {
        self.value() == other
    }
}

impl PartialEq<&HeaderValue> for ContentType {
    fn eq(&self, other: &&HeaderValue) -> bool {
        self.value() == other
    }
}
