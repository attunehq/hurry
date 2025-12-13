//! Rate limiting configuration for the Courier API.
//!
//! Uses tower-governor to implement rate limiting based on:
//! - Authorization header (for authenticated requests)
//! - Invitation token prefix (for invitation acceptance)
//!
//! IP-based rate limiting is intentionally avoided as it's ineffective
//! against distributed attacks and penalizes users behind shared IPs.

use std::sync::Arc;

use axum::body::Body;
use governor::{clock::QuantaInstant, middleware::NoOpMiddleware};
use http::{Request, header::AUTHORIZATION};
use tap::Pipe;
use tower_governor::{
    GovernorLayer, errors::GovernorError, governor::GovernorConfigBuilder,
    key_extractor::KeyExtractor,
};
use tracing::error;

/// Key extractor that uses the Authorization header value.
///
/// Falls back to a constant "anonymous" key if no Authorization header is
/// present, which creates a shared rate limit bucket for all unauthenticated
/// requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthHeaderKeyExtractor;

impl KeyExtractor for AuthHeaderKeyExtractor {
    type Key = String;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        req.headers()
            .get(AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .map(String::from)
            .unwrap_or_else(|| String::from("anonymous"))
            .pipe(Ok)
    }
}

/// Key extractor based on the hash of the route.
///
/// The route is hashed, and then the first `prefix` characters are selected
/// from the hex-encoded representation of the hash; these are then used to
/// bucket requests for rate limiting.
///
/// This extractor is intended to be used when the route itself contains data
/// that is usable for rate limiting (e.g. invitation tokens).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteHashExtractor {
    /// Number of characters to use from the hash prefix for bucketing.
    prefix_length: usize,
}

impl RouteHashExtractor {
    /// Create a new extractor with the specified prefix length.
    pub const fn new(prefix_length: usize) -> Self {
        Self { prefix_length }
    }
}

impl KeyExtractor for RouteHashExtractor {
    type Key = String;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        let path = req.uri().path();
        let hash = blake3::hash(path.as_bytes());
        let encoded = hex::encode(hash.as_bytes());
        let prefix_length = self.prefix_length;

        if encoded.len() < prefix_length {
            error!(?prefix_length, ?encoded, "route hash prefix too short");
            return Err(GovernorError::UnableToExtractKey);
        }

        let prefix = &encoded[..prefix_length];
        Ok(format!("inv:{prefix}"))
    }
}

/// Create a rate limiter layer for sensitive endpoints.
///
/// This configuration is used for endpoints that should be protected against
/// abuse, such as API key creation.
///
/// **Configuration:**
/// - 10 requests per minute per Authorization header value
/// - Unauthenticated requests share a single "anonymous" bucket
///
/// ## Usage
///
/// ```ignore
/// Router::new()
///     .route("/sensitive", post(handler))
///     .layer(rate_limit::sensitive())
/// ```
pub fn sensitive() -> GovernorLayer<AuthHeaderKeyExtractor, NoOpMiddleware<QuantaInstant>, Body> {
    GovernorConfigBuilder::default()
        .per_second(6)
        .burst_size(10)
        .key_extractor(AuthHeaderKeyExtractor)
        .finish()
        .expect("valid governor config")
        .pipe(Arc::new)
        .pipe(GovernorLayer::new)
}

/// Create a rate limiter layer for invitation acceptance.
///
/// This configuration buckets requests by the hash of the route, and then
/// uses the first few characters of the hex-encoded hash to bucket requests.
///
/// **Configuration:**
/// - 10 requests per minute per route hash prefix bucket
/// - Burst of 5 to allow some legitimate retries
pub fn invitation() -> GovernorLayer<RouteHashExtractor, NoOpMiddleware<QuantaInstant>, Body> {
    GovernorConfigBuilder::default()
        .per_second(6) // ~10 per minute
        .burst_size(5) // Allow small burst for retries
        .key_extractor(RouteHashExtractor::new(8))
        .finish()
        .expect("valid governor config")
        .pipe(Arc::new)
        .pipe(GovernorLayer::new)
}

/// The default rate limiter layer is really just for protecting the API from
/// outright abuse or DOS attacks.
///
/// **Configuration:**
/// - 6,000 requests per minute per Authorization header value
/// - Unauthenticated requests share a single "anonymous" bucket
pub fn standard() -> GovernorLayer<AuthHeaderKeyExtractor, NoOpMiddleware<QuantaInstant>, Body> {
    GovernorConfigBuilder::default()
        .per_second(6000)
        .burst_size(200)
        .key_extractor(AuthHeaderKeyExtractor)
        .finish()
        .expect("valid governor config")
        .pipe(Arc::new)
        .pipe(GovernorLayer::new)
}

/// The caching rate limiter layer is really just for protecting the API from
/// abuse or DOS attacks on the caching endpoints. This is higher than the
/// standard rate limiter because the caching endpoints are likely to be hit
/// extremely often.
///
/// **Configuration:**
/// - 60,000 requests per minute per Authorization header value
/// - Unauthenticated requests share a single "anonymous" bucket
pub fn caching() -> GovernorLayer<AuthHeaderKeyExtractor, NoOpMiddleware<QuantaInstant>, Body> {
    GovernorConfigBuilder::default()
        .per_second(60_000)
        .burst_size(20_000)
        .key_extractor(AuthHeaderKeyExtractor)
        .finish()
        .expect("valid governor config")
        .pipe(Arc::new)
        .pipe(GovernorLayer::new)
}
