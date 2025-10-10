use axum::{
    Router,
    routing::{get, post},
};

use crate::api::State;

pub mod stateless_mint;
pub mod stateless_validate;

pub fn router() -> Router<State> {
    Router::new()
        .route("/", post(stateless_mint::handle))
        .route("/", get(stateless_validate::handle))
}
