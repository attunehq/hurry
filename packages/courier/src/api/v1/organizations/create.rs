//! Create organization endpoint.

use aerosol::axum::Dep;
use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{error, info};

use crate::{auth::SessionContext, db::Postgres};

#[derive(Debug, Deserialize)]
pub struct CreateOrganizationRequest {
    /// The organization name.
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct CreateOrganizationResponse {
    /// The organization ID.
    pub id: i64,

    /// The organization name.
    pub name: String,
}

/// Create a new organization.
#[tracing::instrument(skip(db, session))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Json(request): Json<CreateOrganizationRequest>,
) -> Response {
    if request.name.trim().is_empty() {
        return Response::EmptyName;
    }

    let org_id = match db
        .create_organization_with_admin(&request.name, session.account_id)
        .await
    {
        Ok(id) => id,
        Err(error) => {
            error!(?error, "organizations.create.error");
            return Response::Error(error.to_string());
        }
    };

    let _ = db
        .log_audit_event(
            Some(session.account_id),
            Some(org_id),
            "organization.created",
            Some(json!({ "name": request.name })),
        )
        .await;

    info!(
        account_id = %session.account_id,
        org_id = %org_id,
        "organizations.create.success"
    );

    Response::Created(CreateOrganizationResponse {
        id: org_id.as_i64(),
        name: request.name,
    })
}

#[derive(Debug)]
pub enum Response {
    Created(CreateOrganizationResponse),
    EmptyName,
    Error(String),
}

impl IntoResponse for Response {
    fn into_response(self) -> axum::response::Response {
        match self {
            Response::Created(org) => (StatusCode::CREATED, Json(org)).into_response(),
            Response::EmptyName => {
                (StatusCode::BAD_REQUEST, "Organization name cannot be empty").into_response()
            }
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
