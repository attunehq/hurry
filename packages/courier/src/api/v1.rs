use axum::{Router, routing::get};

use crate::api::State;

pub mod auth;
pub mod cas;
pub mod health;

pub fn router() -> Router<State> {
    Router::new()
        .nest("/auth", auth::router())
        .nest("/cas", cas::router())
        .route("/health", get(health::handle))
}
