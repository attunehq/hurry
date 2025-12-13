//! Current user endpoints.

use axum::{Router, routing::get};

use crate::api::State;

pub mod get;
pub mod organizations;

pub fn router() -> Router<State> {
    Router::new()
        .route("/", get(get::handle))
        .route("/organizations", get(organizations::handle))
}
