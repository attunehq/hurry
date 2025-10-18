use axum::{
    Router,
    routing::{get, head, post, put},
};

use crate::api::State;

pub mod bulk;
pub mod check;
pub mod read;
pub mod write;

pub fn router() -> Router<State> {
    Router::new()
        .route("/{key}", head(check::handle))
        .route("/{key}", get(read::handle))
        .route("/{key}", put(write::handle))
        .route("/bulk/read", post(bulk::read::handle))
        .route("/bulk/write", post(bulk::write::handle))
}
