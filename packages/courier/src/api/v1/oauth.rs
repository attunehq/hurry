//! OAuth authentication endpoints.

use axum::{
    Router,
    routing::{get, post},
};
use time::Duration;

use crate::api::State;

pub mod callback;
pub mod exchange;
pub mod logout;
pub mod start;

pub const SESSION_DURATION: Duration = Duration::hours(24);
pub const OAUTH_STATE_DURATION: Duration = Duration::minutes(10);
pub const EXCHANGE_CODE_DURATION: Duration = Duration::seconds(60);

pub fn router() -> Router<State> {
    Router::new()
        .route("/github/start", get(start::handle))
        .route("/github/callback", get(callback::handle))
        .route("/exchange", post(exchange::handle))
        .route("/logout", post(logout::handle))
}
