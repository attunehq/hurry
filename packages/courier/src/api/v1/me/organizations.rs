//! List current user's organizations endpoint.

use aerosol::axum::Dep;
use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Serialize;
use tap::Pipe;
use time::OffsetDateTime;
use tracing::{error, info};

use crate::{
    auth::{OrgRole, SessionContext},
    db::Postgres,
};

#[derive(Debug, Serialize)]
pub struct OrganizationListResponse {
    /// The list of organizations.
    pub organizations: Vec<OrganizationEntry>,
}

#[derive(Debug, Serialize)]
pub struct OrganizationEntry {
    /// The organization ID.
    pub id: i64,

    /// The organization name.
    pub name: String,

    /// The user's role in the organization.
    pub role: OrgRole,

    /// The organization creation timestamp.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// List the current user's organizations.
#[tracing::instrument(skip(db, session))]
pub async fn handle(Dep(db): Dep<Postgres>, session: SessionContext) -> Response {
    match db.list_organizations_for_account(session.account_id).await {
        Ok(orgs) => {
            info!(
                account_id = %session.account_id,
                count = orgs.len(),
                "me.organizations.success"
            );
            orgs.into_iter()
                .map(|org| OrganizationEntry {
                    id: org.organization.id.as_i64(),
                    name: org.organization.name,
                    role: org.role,
                    created_at: org.organization.created_at,
                })
                .collect::<Vec<_>>()
                .pipe(|organizations| OrganizationListResponse { organizations })
                .pipe(Response::Success)
        }
        Err(error) => {
            error!(?error, "me.organizations.error");
            Response::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum Response {
    Success(OrganizationListResponse),
    Error(String),
}

impl IntoResponse for Response {
    fn into_response(self) -> axum::response::Response {
        match self {
            Response::Success(list) => (StatusCode::OK, Json(list)).into_response(),
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
