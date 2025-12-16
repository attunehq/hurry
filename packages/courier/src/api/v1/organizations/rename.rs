//! Rename organization endpoint.

use aerosol::axum::Dep;
use axum::{Json, extract::Path, http::StatusCode, response::IntoResponse};
use serde::Deserialize;
use serde_json::json;
use tracing::{error, info, warn};

use crate::{
    auth::{OrgId, SessionContext},
    db::Postgres,
};

#[derive(Debug, Deserialize)]
pub struct RenameOrganizationRequest {
    /// The new name for the organization.
    pub name: String,
}

/// Rename an organization. Only admins can perform this action.
#[tracing::instrument(skip(db, session))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
    Json(request): Json<RenameOrganizationRequest>,
) -> Response {
    let org_id = OrgId::from_i64(org_id);

    // Validate name is not empty
    let name = request.name.trim();
    if name.is_empty() {
        warn!(
            account_id = %session.account_id,
            org_id = %org_id,
            "organizations.rename.empty_name"
        );
        return Response::EmptyName;
    }

    // Check that the user is an admin of the organization
    match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(role)) if role.is_admin() => {}
        Ok(Some(_)) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.rename.not_admin"
            );
            return Response::Forbidden;
        }
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.rename.not_member"
            );
            return Response::Forbidden;
        }
        Err(error) => {
            error!(?error, "organizations.rename.role_check_error");
            return Response::Error(error.to_string());
        }
    }

    match db.rename_organization(org_id, name).await {
        Ok(true) => {
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
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
    Forbidden,
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
            Response::Forbidden => (
                StatusCode::FORBIDDEN,
                "Only admins can rename the organization",
            )
                .into_response(),
            Response::NotFound => (StatusCode::NOT_FOUND, "Organization not found").into_response(),
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
