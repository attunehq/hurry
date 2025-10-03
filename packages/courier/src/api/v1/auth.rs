use axum::{
    Router,
    routing::{delete, get, post},
};

use crate::api::State;

pub mod stateless_mint;
pub mod stateless_revoke;
pub mod stateless_validate;

pub fn router() -> Router<State> {
    Router::new()
        .route("/", post(stateless_mint::handle))
        .route("/", delete(stateless_revoke::handle))
        .route("/", get(stateless_validate::handle))
}
