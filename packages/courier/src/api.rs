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
//! Most handlers return a response type that implements [`IntoResponse`](axum::response::IntoResponse)[^2].
//! This is a trait that allows handlers to return a response without having to
//! manually implement the response type.
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

use std::time::Duration;

use aerosol::Aero;
use axum::Router;
use tower::ServiceBuilder;
use tower_http::{limit::RequestBodyLimitLayer, timeout::TimeoutLayer, trace::TraceLayer};

pub mod v1;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_BODY_SIZE: usize = 100 * 1024 * 1024;

pub type State = Aero![
    crate::db::Postgres,
    crate::storage::Disk,
    crate::auth::KeySets,
];

pub fn router(state: State) -> Router {
    let middleware = ServiceBuilder::new()
        .layer(TraceLayer::new_for_http())
        .layer(RequestBodyLimitLayer::new(MAX_BODY_SIZE))
        .layer(TimeoutLayer::new(REQUEST_TIMEOUT));

    Router::new()
        .nest("/api/v1", v1::router())
        .layer(middleware)
        .with_state(state)
}
