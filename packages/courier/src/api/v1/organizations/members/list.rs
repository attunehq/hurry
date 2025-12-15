//! List organization members endpoint.

use aerosol::axum::Dep;
use axum::{Json, extract::Path, http::StatusCode, response::IntoResponse};
use serde::Serialize;
use tap::Pipe;
use time::OffsetDateTime;
use tracing::{error, info, warn};

use crate::{
    auth::{OrgId, OrgRole, SessionContext},
    db::Postgres,
};

#[derive(Debug, Serialize)]
pub struct MemberListResponse {
    /// The list of members.
    pub members: Vec<MemberEntry>,
}

#[derive(Debug, Serialize)]
pub struct MemberEntry {
    /// The account ID.
    pub account_id: i64,

    /// The account email.
    pub email: String,

    /// The account name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// The member's role in the organization.
    pub role: OrgRole,

    /// The date the member joined the organization.
    #[serde(with = "time::serde::rfc3339")]
    pub joined_at: OffsetDateTime,

    /// Whether the account is a bot (i.e., does not have a GitHub identity).
    pub bot: bool,
}

/// List members of an organization.
#[tracing::instrument(skip(db, session))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
) -> Response {
    let org_id = OrgId::from_i64(org_id);

    match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.list_members.not_member"
            );
            return Response::Forbidden;
        }
        Err(error) => {
            error!(?error, "organizations.list_members.role_check_error");
            return Response::Error(error.to_string());
        }
    }

    match db.list_organization_members(org_id).await {
        Ok(members) => {
            info!(
                org_id = %org_id,
                count = members.len(),
                "organizations.list_members.success"
            );
            members
                .into_iter()
                .map(|m| MemberEntry {
                    account_id: m.account_id.as_i64(),
                    email: m.email,
                    name: m.name,
                    role: m.role,
                    joined_at: m.created_at,
                    bot: !m.has_github_identity,
                })
                .collect::<Vec<_>>()
                .pipe(|members| MemberListResponse { members })
                .pipe(Response::Success)
        }
        Err(error) => {
            error!(?error, "organizations.list_members.error");
            Response::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum Response {
    Success(MemberListResponse),
    Forbidden,
    Error(String),
}

impl IntoResponse for Response {
    fn into_response(self) -> axum::response::Response {
        match self {
            Response::Success(list) => (StatusCode::OK, Json(list)).into_response(),
            Response::Forbidden => (
                StatusCode::FORBIDDEN,
                "You must be a member of this organization to view members",
            )
                .into_response(),
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
