//! Rate limiting configuration for the Courier API.
//!
//! Uses tower-governor to implement rate limiting based on:
//! - Authorization header (for authenticated requests)
//! - Invitation token prefix (for invitation acceptance)
//!
//! IP-based rate limiting is intentionally avoided as it's ineffective
//! against distributed attacks and penalizes users behind shared IPs.

use std::sync::Arc;

use http::{Request, header::AUTHORIZATION};
use tower_governor::{
    GovernorLayer,
    errors::GovernorError,
    governor::GovernorConfigBuilder,
    key_extractor::KeyExtractor,
};

/// Key extractor that uses the Authorization header value.
///
/// Falls back to a constant "anonymous" key if no Authorization header is present,
/// which creates a shared rate limit bucket for all unauthenticated requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthHeaderKeyExtractor;

impl KeyExtractor for AuthHeaderKeyExtractor {
    type Key = String;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        let key = req
            .headers()
            .get(AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .map(String::from)
            .unwrap_or_else(|| String::from("anonymous"));
        Ok(key)
    }
}

/// Key extractor for invitation token endpoints.
///
/// Extracts the invitation token from the URL path and buckets by the first
/// few characters. This provides rate limiting that:
/// - Prevents brute-force attacks on specific token prefixes
/// - Doesn't penalize legitimate users trying different invitations
/// - Groups similar tokens together to catch enumeration attempts
///
/// The token is expected at path segment index 3: `/api/v1/invitations/{token}/accept`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvitationTokenPrefixExtractor {
    /// Number of characters to use from the token prefix for bucketing.
    prefix_len: usize,
}

impl InvitationTokenPrefixExtractor {
    /// Create a new extractor with the specified prefix length.
    ///
    /// A prefix length of 4-6 characters is recommended:
    /// - Too short: legitimate users may collide
    /// - Too long: attackers can easily avoid rate limits
    pub const fn new(prefix_len: usize) -> Self {
        Self { prefix_len }
    }
}

impl KeyExtractor for InvitationTokenPrefixExtractor {
    type Key = String;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        let path = req.uri().path();

        // Path format: /api/v1/invitations/{token}/accept
        // We want the token segment (index 4 when split by '/')
        let token = path
            .split('/')
            .nth(4) // ["", "api", "v1", "invitations", "{token}", "accept"]
            .unwrap_or("unknown");

        // Take prefix for bucketing
        let prefix = if token.len() >= self.prefix_len {
            &token[..self.prefix_len]
        } else {
            token
        };

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
pub fn sensitive() -> GovernorLayer<
    AuthHeaderKeyExtractor,
    governor::middleware::NoOpMiddleware<governor::clock::QuantaInstant>,
    axum::body::Body,
> {
    let config = GovernorConfigBuilder::default()
        .per_second(6) // ~10 per minute: replenish 1 every 6 seconds
        .burst_size(10) // Allow burst up to 10
        .key_extractor(AuthHeaderKeyExtractor)
        .finish()
        .expect("valid governor config");

    GovernorLayer::new(Arc::new(config))
}

/// Create a rate limiter layer for invitation acceptance.
///
/// This configuration buckets requests by the first few characters of the
/// invitation token, which:
/// - Prevents brute-force enumeration of tokens with similar prefixes
/// - Doesn't penalize legitimate users with different tokens
///
/// **Configuration:**
/// - 10 requests per minute per token prefix bucket (4 characters)
/// - Burst of 5 to allow some legitimate retries
pub fn invitation_accept() -> GovernorLayer<
    InvitationTokenPrefixExtractor,
    governor::middleware::NoOpMiddleware<governor::clock::QuantaInstant>,
    axum::body::Body,
> {
    let config = GovernorConfigBuilder::default()
        .per_second(6) // ~10 per minute
        .burst_size(5) // Allow small burst for retries
        .key_extractor(InvitationTokenPrefixExtractor::new(4))
        .finish()
        .expect("valid governor config");

    GovernorLayer::new(Arc::new(config))
}

/// Create a rate limiter layer for less sensitive but still protected
/// endpoints.
///
/// This configuration is used for endpoints that need some protection but
/// can tolerate more traffic.
///
/// **Configuration:**
/// - 60 requests per minute per Authorization header value
/// - Unauthenticated requests share a single "anonymous" bucket
pub fn standard() -> GovernorLayer<
    AuthHeaderKeyExtractor,
    governor::middleware::NoOpMiddleware<governor::clock::QuantaInstant>,
    axum::body::Body,
> {
    let config = GovernorConfigBuilder::default()
        .per_second(1) // 60 per minute: replenish 1 every second
        .burst_size(10) // Allow small bursts
        .key_extractor(AuthHeaderKeyExtractor)
        .finish()
        .expect("valid governor config");

    GovernorLayer::new(Arc::new(config))
}

/// Create a very permissive rate limiter for read-heavy endpoints.
///
/// **Configuration:**
/// - 600 requests per minute per Authorization header value
/// - Unauthenticated requests share a single "anonymous" bucket
pub fn permissive() -> GovernorLayer<
    AuthHeaderKeyExtractor,
    governor::middleware::NoOpMiddleware<governor::clock::QuantaInstant>,
    axum::body::Body,
> {
    let config = GovernorConfigBuilder::default()
        .per_millisecond(100) // 10 per second = 600 per minute
        .burst_size(20) // Allow larger bursts
        .key_extractor(AuthHeaderKeyExtractor)
        .finish()
        .expect("valid governor config");

    GovernorLayer::new(Arc::new(config))
}
