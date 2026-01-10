//! Rename organization endpoint.

use aerosol::axum::Dep;
use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Deserialize;
use serde_json::json;
use tracing::{error, info, warn};

use crate::{auth::SessionTokenAdmin, db::Postgres};

#[derive(Debug, Deserialize)]
pub struct RenameOrganizationRequest {
    /// The new name for the organization.
    pub name: String,
}

/// Rename an organization. Only admins can perform this action.
///
/// This handler uses `SessionTokenAdmin` as an extractor, which means:
/// - The session token is validated before the handler runs
/// - Admin privileges are verified before the handler runs
/// - The handler is NEVER called if the user is not an admin
#[tracing::instrument(skip(db, admin))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    admin: SessionTokenAdmin,
    Json(request): Json<RenameOrganizationRequest>,
) -> Response {
    let org_id = admin.org_id();
    let account_id = admin.account_id();

    // Validate name is not empty
    let name = request.name.trim();
    if name.is_empty() {
        warn!(
            account_id = %account_id,
            org_id = %org_id,
            "organizations.rename.empty_name"
        );
        return Response::EmptyName;
    }

    match db.rename_organization(org_id, name).await {
        Ok(true) => {
            let _ = db
                .log_audit_event(
                    Some(account_id),
                    Some(org_id),
                    "organization.renamed",
                    Some(json!({
                        "new_name": name,
                    })),
                )
                .await;

            info!(
                org_id = %org_id,
                new_name = %name,
                "organizations.rename.success"
            );
            Response::Success
        }
        Ok(false) => Response::NotFound,
        Err(error) => {
            error!(?error, "organizations.rename.error");
            Response::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum Response {
    Success,
    EmptyName,
    NotFound,
    Error(String),
}

impl IntoResponse for Response {
    fn into_response(self) -> axum::response::Response {
        match self {
            Response::Success => StatusCode::NO_CONTENT.into_response(),
            Response::EmptyName => {
                (StatusCode::BAD_REQUEST, "Organization name cannot be empty").into_response()
            }
            Response::NotFound => (StatusCode::NOT_FOUND, "Organization not found").into_response(),
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
