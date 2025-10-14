use axum::{
    Router,
    routing::{get, head, put},
};

use crate::api::State;

pub mod check;
pub mod read;
pub mod write;

pub fn router() -> Router<State> {
    Router::new()
        .route("/{key}", head(check::handle))
        .route("/{key}", get(read::handle))
        .route("/{key}", put(write::handle))
}
