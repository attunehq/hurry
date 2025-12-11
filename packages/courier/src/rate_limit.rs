//! Rate limiting configuration for the Courier API.
//!
//! Uses tower-governor to implement rate limiting based on client IP address.
//! Different configurations are provided for different sensitivity levels.

use std::sync::Arc;

use tower_governor::{
    GovernorLayer,
    governor::GovernorConfigBuilder,
    key_extractor::SmartIpKeyExtractor,
};

/// Create a rate limiter layer for sensitive endpoints.
///
/// This configuration is used for endpoints that should be protected against
/// abuse, such as invitation acceptance and API key creation.
///
/// **Configuration:**
/// - 10 requests per minute per IP address
/// - Uses SmartIpKeyExtractor which checks x-forwarded-for, x-real-ip, and
///   forwarded headers before falling back to peer IP
///
/// ## Usage
///
/// ```ignore
/// use tower::ServiceBuilder;
///
/// Router::new()
///     .route("/sensitive", post(handler))
///     .layer(sensitive_rate_limit())
/// ```
pub fn sensitive() -> GovernorLayer<SmartIpKeyExtractor, governor::middleware::NoOpMiddleware<governor::clock::QuantaInstant>, axum::body::Body> {
    let config = GovernorConfigBuilder::default()
        .per_second(6) // ~10 per minute: replenish 1 every 6 seconds
        .burst_size(10) // Allow burst up to 10
        .key_extractor(SmartIpKeyExtractor)
        .finish()
        .expect("valid governor config");

    GovernorLayer::new(Arc::new(config))
}

/// Create a rate limiter layer for less sensitive but still protected endpoints.
///
/// This configuration is used for endpoints that need some protection but
/// can tolerate more traffic.
///
/// **Configuration:**
/// - 60 requests per minute per IP address (1/second)
/// - Uses SmartIpKeyExtractor
pub fn standard() -> GovernorLayer<SmartIpKeyExtractor, governor::middleware::NoOpMiddleware<governor::clock::QuantaInstant>, axum::body::Body> {
    let config = GovernorConfigBuilder::default()
        .per_second(1) // 60 per minute: replenish 1 every second
        .burst_size(10) // Allow small bursts
        .key_extractor(SmartIpKeyExtractor)
        .finish()
        .expect("valid governor config");

    GovernorLayer::new(Arc::new(config))
}

/// Create a very permissive rate limiter for read-heavy endpoints.
///
/// **Configuration:**
/// - 600 requests per minute per IP address (10/second)
/// - Uses SmartIpKeyExtractor
pub fn permissive() -> GovernorLayer<SmartIpKeyExtractor, governor::middleware::NoOpMiddleware<governor::clock::QuantaInstant>, axum::body::Body> {
    let config = GovernorConfigBuilder::default()
        .per_millisecond(100) // 10 per second = 600 per minute
        .burst_size(20) // Allow larger bursts
        .key_extractor(SmartIpKeyExtractor)
        .finish()
        .expect("valid governor config");

    GovernorLayer::new(Arc::new(config))
}
