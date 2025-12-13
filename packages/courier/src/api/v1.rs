use axum::{Router, routing::get};

use crate::{api::State, rate_limit};

pub mod cache;
pub mod cas;
pub mod health;
pub mod invitations;
pub mod me;
pub mod oauth;
pub mod organizations;

pub fn router() -> Router<State> {
    let standard = Router::new()
        .nest("/me", me::router())
        .nest("/oauth", oauth::router())
        .nest("/organizations", organizations::router())
        .nest("/invitations", invitations::router())
        .route("/health", get(health::handle))
        .layer(rate_limit::standard());

    let caching = Router::new()
        .nest("/cache", cache::router())
        .nest("/cas", cas::router())
        .layer(rate_limit::caching());

    Router::new().merge(standard).merge(caching)
}
