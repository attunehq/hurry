use axum::{Router, routing::get};

use crate::api::State;

pub mod cache;
pub mod cas;
pub mod health;
pub mod invitations;
pub mod me;
pub mod oauth;
pub mod organizations;

pub fn router() -> Router<State> {
    Router::new()
        .nest("/cache", cache::router())
        .nest("/cas", cas::router())
        .nest("/me", me::router())
        .nest("/oauth", oauth::router())
        .nest("/organizations", organizations::router())
        .merge(invitations::router())
        .route("/health", get(health::handle))
}
