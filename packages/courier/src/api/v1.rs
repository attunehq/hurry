use axum::{Router, routing::get};

use crate::api::State;

pub mod cache;
pub mod cas;
pub mod health;
pub mod oauth;

pub fn router() -> Router<State> {
    Router::new()
        .nest("/cache", cache::router())
        .nest("/cas", cas::router())
        .nest("/oauth", oauth::router())
        .route("/health", get(health::handle))
}
