//! Get invitation preview endpoint.

use aerosol::axum::Dep;
use axum::{Json, extract::Path, http::StatusCode, response::IntoResponse};
use serde::Serialize;
use time::OffsetDateTime;
use tracing::{error, info, warn};

use crate::{auth::OrgRole, db::Postgres};

#[derive(Debug, Serialize)]
pub struct InvitationPreviewResponse {
    /// The organization name.
    pub organization_name: String,

    /// The role to grant.
    pub role: OrgRole,

    /// The expiration timestamp. None means the invitation never expires.
    #[serde(with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,

    /// Whether the invitation is valid.
    pub valid: bool,
}

/// Get a preview of an invitation (no authentication required).
///
/// This allows potential members to see what organization they're joining
/// before signing in.
#[tracing::instrument(skip(db))]
pub async fn handle(Dep(db): Dep<Postgres>, Path(token): Path<String>) -> Response {
    match db.get_invitation_preview(&token).await {
        Ok(Some(preview)) => {
            info!("invitations.preview.success");
            Response::Success(InvitationPreviewResponse {
                organization_name: preview.organization_name,
                role: preview.role,
                expires_at: preview.expires_at,
                valid: preview.valid,
            })
        }
        Ok(None) => {
            warn!("invitations.preview.not_found");
            Response::NotFound
        }
        Err(err) => {
            error!(?err, "invitations.preview.error");
            Response::Error(err.to_string())
        }
    }
}

#[derive(Debug)]
pub enum Response {
    Success(InvitationPreviewResponse),
    NotFound,
    Error(String),
}

impl IntoResponse for Response {
    fn into_response(self) -> axum::response::Response {
        match self {
            Response::Success(preview) => (StatusCode::OK, Json(preview)).into_response(),
            Response::NotFound => (StatusCode::NOT_FOUND, "Invitation not found").into_response(),
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
