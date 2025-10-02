use std::time::Duration;

use aerosol::Aero;
use axum::{Router, routing::get};
use tower::ServiceBuilder;
use tower_http::{limit::RequestBodyLimitLayer, timeout::TimeoutLayer, trace::TraceLayer};

pub mod v1;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_BODY_SIZE: usize = 100 * 1024 * 1024;

pub type State = Aero![crate::storage::Disk];

pub fn router(state: State) -> Router {
    let middleware = ServiceBuilder::new()
        .layer(TraceLayer::new_for_http())
        .layer(RequestBodyLimitLayer::new(MAX_BODY_SIZE))
        .layer(TimeoutLayer::new(REQUEST_TIMEOUT));

    Router::new()
        .route("/health", get(|| async { "ok" }))
        .nest("/api/v1", v1::router())
        .layer(middleware)
        .with_state(state)
}
