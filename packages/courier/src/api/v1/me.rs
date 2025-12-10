//! Current user endpoints.
//!
//! These endpoints allow authenticated users to view their own profile
//! and organization memberships.

use aerosol::axum::Dep;
use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::get};
use serde::Serialize;
use time::OffsetDateTime;
use tracing::{error, info};

use crate::{
    api::State,
    auth::{OrgRole, SessionContext},
    db::Postgres,
};

pub fn router() -> Router<State> {
    Router::new()
        .route("/", get(get_me))
        .route("/organizations", get(list_organizations))
}

/// Response for GET /me endpoint.
#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub id: i64,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// Get the current user's profile.
///
/// Returns the authenticated user's account information.
///
/// ## Endpoint
/// ```
/// GET /api/v1/me
/// Authorization: Bearer <session_token>
/// ```
///
/// ## Responses
/// - 200: User profile
/// - 401: Not authenticated
/// - 404: Account not found (shouldn't happen for valid sessions)
#[tracing::instrument(skip(db, session))]
pub async fn get_me(Dep(db): Dep<Postgres>, session: SessionContext) -> GetMeResponse {
    match db.get_account(session.account_id).await {
        Ok(Some(account)) => {
            info!(account_id = %session.account_id, "me.get.success");
            GetMeResponse::Success(MeResponse {
                id: account.id.as_i64(),
                email: account.email,
                name: account.name,
                created_at: account.created_at,
            })
        }
        Ok(None) => {
            error!(account_id = %session.account_id, "me.get.not_found");
            GetMeResponse::NotFound
        }
        Err(err) => {
            error!(?err, "me.get.error");
            GetMeResponse::Error(err.to_string())
        }
    }
}

#[derive(Debug)]
pub enum GetMeResponse {
    Success(MeResponse),
    NotFound,
    Error(String),
}

impl IntoResponse for GetMeResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            GetMeResponse::Success(me) => (StatusCode::OK, Json(me)).into_response(),
            GetMeResponse::NotFound => (
                StatusCode::NOT_FOUND,
                "Account not found. This may indicate a database inconsistency.",
            )
                .into_response(),
            GetMeResponse::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}

/// Response for GET /me/organizations endpoint.
#[derive(Debug, Serialize)]
pub struct OrganizationListResponse {
    pub organizations: Vec<OrganizationEntry>,
}

/// A single organization entry in the list response.
#[derive(Debug, Serialize)]
pub struct OrganizationEntry {
    pub id: i64,
    pub name: String,
    pub role: OrgRole,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// List the current user's organizations.
///
/// Returns all organizations the authenticated user is a member of,
/// along with their role in each organization.
///
/// ## Endpoint
/// ```
/// GET /api/v1/me/organizations
/// Authorization: Bearer <session_token>
/// ```
///
/// ## Responses
/// - 200: List of organizations with roles
/// - 401: Not authenticated
#[tracing::instrument(skip(db, session))]
pub async fn list_organizations(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
) -> ListOrganizationsResponse {
    match db.list_organizations_for_account(session.account_id).await {
        Ok(orgs) => {
            info!(
                account_id = %session.account_id,
                count = orgs.len(),
                "me.organizations.success"
            );
            let organizations = orgs
                .into_iter()
                .map(|org| OrganizationEntry {
                    id: org.organization.id.as_i64(),
                    name: org.organization.name,
                    role: org.role,
                    created_at: org.organization.created_at,
                })
                .collect();
            ListOrganizationsResponse::Success(OrganizationListResponse { organizations })
        }
        Err(err) => {
            error!(?err, "me.organizations.error");
            ListOrganizationsResponse::Error(err.to_string())
        }
    }
}

#[derive(Debug)]
pub enum ListOrganizationsResponse {
    Success(OrganizationListResponse),
    Error(String),
}

impl IntoResponse for ListOrganizationsResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            ListOrganizationsResponse::Success(list) => (StatusCode::OK, Json(list)).into_response(),
            ListOrganizationsResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}
