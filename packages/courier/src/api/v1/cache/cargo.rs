use axum::{Router, routing::post};

use crate::api::State;

pub mod bulk_restore;
pub mod reset;
pub mod restore;
pub mod save;

pub fn router() -> Router<State> {
    Router::new()
        .route("/save", post(save::handle))
        .route("/restore", post(restore::handle))
        .route("/bulk/restore", post(bulk_restore::handle))
        .route("/reset", post(reset::handle))
}
