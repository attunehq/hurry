use axum::Router;

use crate::api::State;

pub mod cargo;

pub fn router() -> Router<State> {
    Router::new().nest("/cargo", cargo::router())
}
