//! List organization members endpoint.

use aerosol::axum::Dep;
use axum::{Json, extract::Path, http::StatusCode, response::IntoResponse};
use serde::Serialize;
use tap::Pipe;
use time::OffsetDateTime;
use tracing::{error, info};

use crate::{
    auth::{ApiError, OrgId, OrgRole, SessionContext},
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
) -> Result<Response, ApiError> {
    let org_id = OrgId::from_i64(org_id);

    // Verify membership using strongly typed role check
    let _member = session.try_member(&db, org_id).await?;

    match db.list_organization_members(org_id).await {
        Ok(members) => {
            info!(
                org_id = %org_id,
                count = members.len(),
                "organizations.list_members.success"
            );
            Ok(members
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
                .pipe(Response::Success))
        }
        Err(error) => {
            error!(?error, "organizations.list_members.error");
            Ok(Response::Error(error.to_string()))
        }
    }
}

#[derive(Debug)]
pub enum Response {
    Success(MemberListResponse),
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
