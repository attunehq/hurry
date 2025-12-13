//! Invitation management endpoints.
//!
//! These endpoints allow organization admins to create and manage invitations,
//! and users to view and accept invitations.

use axum::{
    Router,
    routing::{delete, get, post},
};
use time::Duration;

use crate::{api::State, rate_limit};

pub mod accept;
pub mod create;
pub mod list;
pub mod preview;
pub mod revoke;

/// Invitations that live longer than this threshold are considered long-lived
/// and use more entropy in their tokens.
pub const LONG_LIVED_THRESHOLD: Duration = Duration::days(7);

pub fn organization_router() -> Router<State> {
    Router::new()
        .route("/{org_id}/invitations", post(create::handle))
        .route("/{org_id}/invitations", get(list::handle))
        .route(
            "/{org_id}/invitations/{invitation_id}",
            delete(revoke::handle),
        )
}

pub fn router() -> Router<State> {
    let invitation = Router::new()
        .route("/{token}/accept", post(accept::handle))
        .layer(rate_limit::invitation());

    Router::new()
        .route("/{token}", get(preview::handle))
        .merge(invitation)
}
