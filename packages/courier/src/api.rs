//! API endpoint handlers for the service.
//!
//! ## Dependency injection
//!
//! We use [`aerosol`][^1] to manage dependencies and inject them into handlers.
//! Reference [`State`] for the list of dependencies; note that when providing
//! dependencies that are in this required list you need to provide them in
//! reverse order of the list.
//!
//! Items that are in the list can be extracted in handlers using the
//! [`Dep`](aerosol::axum::Dep) extractor.
//!
//! [^1]: https://docs.rs/aerosol
//!
//! ## Response types
//!
//! Most handlers return a response type that implements
//! [`IntoResponse`](axum::response::IntoResponse)[^2]. This is a trait that
//! allows handlers to return a response without having to manually implement
//! the response type.
//!
//! We do it this way instead of just returning a more generic response type
//! because it supports better documentation and makes it easier to realize if
//! you're writing backwards-incompatible changes to the API.
//!
//! For documentation, we can in the future add `utoipa` and then use it to
//! annotate the response type with documentation which is then automatically
//! rendered for the user in the OpenAPI spec.
//!
//! [^2]: https://docs.rs/axum/latest/axum/response/trait.IntoResponse.html

use std::time::{Duration, Instant};

use aerosol::Aero;
use axum::{
    Router, extract::DefaultBodyLimit, extract::Request, http::HeaderValue, middleware::Next,
    response::Response,
};
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer, decompression::RequestDecompressionLayer,
    limit::RequestBodyLimitLayer, timeout::TimeoutLayer,
};
use tracing::Instrument;
use uuid::Uuid;

pub mod v1;

/// Request timeout is set to accommodate bulk operations transferring large
/// amounts of data. 30 minutes allows for 10GB transfers over slower
/// connections (~50 Mbps) while still protecting against indefinitely hanging
/// connections.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(1800);

/// Body size limit for streaming operations (CAS uploads). Single artifacts
/// can be large (e.g., libaws_sdk_s3 produces 125MB rlibs) and bulk operations
/// may transfer many artifacts in one request.
const MAX_BODY_SIZE: usize = 10 * 1024 * 1024 * 1024; // 10GB

/// Body size limit for JSON deserialization. Set to accommodate bulk metadata
/// operations like bulk restore requests.
const MAX_JSON_BODY_SIZE: usize = 100 * 1024 * 1024; // 100MB

pub type State = Aero![crate::db::Postgres, crate::storage::Disk,];

pub fn router(state: State) -> Router {
    let middleware = ServiceBuilder::new()
        .layer(RequestDecompressionLayer::new())
        .layer(CompressionLayer::new())
        .layer(RequestBodyLimitLayer::new(MAX_BODY_SIZE))
        .layer(TimeoutLayer::new(REQUEST_TIMEOUT));

    Router::new()
        .nest("/api/v1", v1::router())
        .layer(DefaultBodyLimit::max(MAX_JSON_BODY_SIZE))
        .layer(middleware)
        .layer(axum::middleware::from_fn(trace_request))
        .with_state(state)
}

async fn trace_request(request: Request, next: Next) -> Response {
    const REQUEST_ID_HEADER: &str = "x-request-id";
    let id = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|id| id.to_str().map(|id| id.to_string()).ok())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let start = Instant::now();
    let url = request.uri().to_string();
    let method = request.method().to_string();

    let span = tracing::info_span!("http.request", %id, %url, %method);
    async move {
        let mut response = next.run(request).await;
        let status = response.status();
        let duration = start.elapsed();
        tracing::info!(%id, %url, %method, %status, ?duration, "http.request.response");

        if let Ok(id) = HeaderValue::from_str(&id) {
            response.headers_mut().insert(REQUEST_ID_HEADER, id);
        }
        response
    }
    .instrument(span)
    .await
}
