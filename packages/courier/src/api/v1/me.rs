//! Current user endpoints.

use aerosol::axum::Dep;
use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::get};
use serde::Serialize;
use tap::Pipe;
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

#[derive(Debug, Serialize)]
pub struct MeResponse {
    /// The account ID.
    pub id: i64,

    /// The account email.
    pub email: String,

    /// The account name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// The GitHub username, if linked.
    /// All accounts should have these, other than bot accounts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_username: Option<String>,

    /// The account creation timestamp.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// Get the current user's profile.
#[tracing::instrument(skip(db, session))]
pub async fn get_me(Dep(db): Dep<Postgres>, session: SessionContext) -> GetMeResponse {
    let account = match db.get_account(session.account_id).await {
        Ok(Some(account)) => account,
        Ok(None) => {
            error!(account_id = %session.account_id, "me.get.not_found");
            return GetMeResponse::NotFound;
        }
        Err(error) => {
            error!(?error, "me.get.error");
            return GetMeResponse::Error(error.to_string());
        }
    };

    let github_username = match db.get_github_identity(session.account_id).await {
        Ok(Some(identity)) => Some(identity.github_username),
        Ok(None) => None,
        Err(error) => {
            error!(?error, "me.get.github_identity_error");
            return GetMeResponse::Error(error.to_string());
        }
    };

    info!(account_id = %session.account_id, "me.get.success");
    GetMeResponse::Success(MeResponse {
        id: account.id.as_i64(),
        email: account.email,
        name: account.name,
        github_username,
        created_at: account.created_at,
    })
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
            orgs.into_iter()
                .map(|org| OrganizationEntry {
                    id: org.organization.id.as_i64(),
                    name: org.organization.name,
                    role: org.role,
                    created_at: org.organization.created_at,
                })
                .collect::<Vec<_>>()
                .pipe(|organizations| OrganizationListResponse { organizations })
                .pipe(ListOrganizationsResponse::Success)
        }
        Err(error) => {
            error!(?error, "me.organizations.error");
            ListOrganizationsResponse::Error(error.to_string())
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
            ListOrganizationsResponse::Success(list) => {
                (StatusCode::OK, Json(list)).into_response()
            }
            ListOrganizationsResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}
