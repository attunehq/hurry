//! Organization management endpoints.

use axum::{
    Router,
    routing::{delete, get, patch, post},
};

use crate::{api::State, api::v1::invitations, rate_limit};

pub mod api_keys;
pub mod bots;
pub mod create;
pub mod leave;
pub mod members;
pub mod rename;

pub fn router() -> Router<State> {
    let sensitive = Router::new()
        .route("/{org_id}/api-keys", post(api_keys::create::handle))
        .route("/{org_id}/bots", post(bots::create::handle))
        .layer(rate_limit::sensitive());

    Router::new()
        .route("/", post(create::handle))
        .route("/{org_id}", patch(rename::handle))
        .route("/{org_id}/members", get(members::list::handle))
        .route(
            "/{org_id}/members/{account_id}",
            patch(members::update::handle),
        )
        .route(
            "/{org_id}/members/{account_id}",
            delete(members::remove::handle),
        )
        .route("/{org_id}/leave", post(leave::handle))
        .route("/{org_id}/api-keys", get(api_keys::list::handle))
        .route(
            "/{org_id}/api-keys/{key_id}",
            delete(api_keys::delete::handle),
        )
        .route("/{org_id}/bots", get(bots::list::handle))
        .merge(invitations::organization_router())
        .merge(sensitive)
}
