use axum::{Router, routing::get};

use crate::api::State;

pub mod cache;
pub mod cas;
pub mod health;

pub fn router() -> Router<State> {
    Router::new()
        .nest("/cache", cache::router())
        .nest("/cas", cas::router())
        .route("/health", get(health::handle))
}
