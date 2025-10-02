use axum::Router;

use crate::api::State;

pub mod auth;
pub mod cas;

pub fn router() -> Router<State> {
    Router::new()
        .nest("/auth", auth::router())
        .nest("/cas", cas::router())
}
