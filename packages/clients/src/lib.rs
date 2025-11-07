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

use std::{fmt, str::FromStr};

use color_eyre::eyre::bail;
use derive_more::Display;
use enum_assoc::Assoc;
use http::header::{self, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use tap::Pipe;

pub mod courier;

/// An authentication token for API access.
///
/// This type wraps a token string and ensures it is never accidentally leaked
/// in logs or debug output. To access the actual token value, use the
/// `expose()` method.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Token(String);

impl Token {
    /// Expose the raw token value.
    ///
    /// This method must be called explicitly to access the token string,
    /// preventing accidental exposure in logs or debug output.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

impl FromStr for Token {
    type Err = color_eyre::Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            bail!("token cannot be empty");
        }
        String::from(s).pipe(Self).pipe(Ok)
    }
}

impl<S: Into<String>> From<S> for Token {
    fn from(s: S) -> Self {
        Self(s.into())
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_redaction() {
        let token = Token::from("super-secret-token-12345");

        // Verify redaction in debug and display
        assert_eq!(format!("{:?}", token), "[redacted]");
        assert_eq!(format!("{}", token), "[redacted]");

        // Verify expose() returns the actual value
        assert_eq!(token.expose(), "super-secret-token-12345");
    }

    #[test]
    fn token_from_str() {
        let token = "test-token".parse::<Token>().unwrap();
        assert_eq!(token.expose(), "test-token");

        // Empty string should fail
        assert!("".parse::<Token>().is_err());
    }

    #[test]
    fn token_serialization() {
        let token = Token::from("test-token-12345");

        // Serialize
        let json = serde_json::to_string(&token).unwrap();
        assert_eq!(json, r#""test-token-12345""#);

        // Deserialize
        let deserialized = serde_json::from_str::<Token>(&json).unwrap();
        assert_eq!(deserialized.expose(), "test-token-12345");
    }
}
