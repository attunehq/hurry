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
use axum::{Router, extract::Request, http::HeaderValue, middleware::Next, response::Response};
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer, decompression::RequestDecompressionLayer,
    limit::RequestBodyLimitLayer, timeout::TimeoutLayer,
};
use tracing::Instrument;
use uuid::Uuid;

pub mod v1;

/// Not chosen for a specific reason, just seems reasonable.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// This was defaulted to 100MB but `libaws_sdk_s3` produces 125MB rlibs.
const MAX_BODY_SIZE: usize = 500 * 1024 * 1024;

pub type State = Aero![crate::db::Postgres, crate::storage::Disk,];

pub fn router(state: State) -> Router {
    let middleware = ServiceBuilder::new()
        .layer(RequestDecompressionLayer::new())
        .layer(CompressionLayer::new())
        .layer(RequestBodyLimitLayer::new(MAX_BODY_SIZE))
        .layer(TimeoutLayer::new(REQUEST_TIMEOUT));

    Router::new()
        .nest("/api/v1", v1::router())
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
        tracing::info!(%status, ?duration, "http.request.response");

        if let Ok(id) = HeaderValue::from_str(&id) {
            response.headers_mut().insert(REQUEST_ID_HEADER, id);
        }
        response
    }
    .instrument(span)
    .await
}

/// Create an isolated test server with the given database pool:
/// - The database pool is intended to come from the [`sqlx::test`] macro
/// - Creates a new [`Disk`](crate::storage::Disk) instance in a temp directory
/// - Creates a new empty [`KeySets`](crate::auth::KeySets) instance
#[cfg(test)]
pub async fn test_server(
    pool: sqlx::PgPool,
) -> color_eyre::Result<(axum_test::TestServer, async_tempfile::TempDir)> {
    use color_eyre::eyre::Context;

    let db = crate::db::Postgres { pool };
    let (storage, temp) = crate::storage::Disk::new_temp()
        .await
        .context("create temp storage")?;
    let state = Aero::new().with(storage).with(db);
    let router = crate::api::router(state);
    axum_test::TestServerConfig::default()
        .build(router)
        .map_err(|e| color_eyre::eyre::eyre!("create test server: {e}"))
        .map(|server| (server, temp))
}

#[cfg(test)]
pub(crate) mod test_helpers {
    use axum::body::Bytes;
    use axum::http::StatusCode;
    use axum_test::TestServer;
    use color_eyre::Result;

    use crate::storage::Key;

    /// Generate test content and compute its key.
    pub fn test_blob(content: &[u8]) -> (Vec<u8>, Key) {
        let hash = blake3::hash(content);
        (content.to_vec(), Key::from(hash))
    }

    /// Write a blob to CAS and return the key.
    pub async fn write_cas(server: &TestServer, content: &[u8]) -> Result<Key> {
        let (_, key) = test_blob(content);
        let response = server
            .put(&format!("/api/v1/cas/{key}"))
            .bytes(Bytes::copy_from_slice(content))
            .await;

        response.assert_status(StatusCode::CREATED);
        Ok(key)
    }
}
