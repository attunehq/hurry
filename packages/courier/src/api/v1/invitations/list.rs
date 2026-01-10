//! List invitations endpoint.

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
pub struct InvitationListResponse {
    /// The list of invitations.
    pub invitations: Vec<InvitationEntry>,
}

#[derive(Debug, Serialize)]
pub struct InvitationEntry {
    /// The invitation ID.
    pub id: i64,

    /// The role to grant.
    pub role: OrgRole,

    /// The creation timestamp.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,

    /// The expiration timestamp. None means the invitation never expires.
    #[serde(with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,

    /// The maximum number of uses.
    pub max_uses: Option<i32>,

    /// The number of times the invitation has been used.
    pub use_count: i32,

    /// Whether the invitation has been revoked.
    pub revoked: bool,
}

/// List invitations for an organization.
#[tracing::instrument(skip(db, session))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
) -> Result<Response, ApiError> {
    let org_id = OrgId::from_i64(org_id);

    // Verify admin access using strongly typed role check
    let _admin = session.try_admin(&db, org_id).await?;

    match db.list_invitations(org_id).await {
        Ok(invitations) => {
            info!(
                org_id = %org_id,
                count = invitations.len(),
                "invitations.list.success"
            );
            Ok(invitations
                .into_iter()
                .map(|inv| InvitationEntry {
                    id: inv.id.as_i64(),
                    role: inv.role,
                    created_at: inv.created_at,
                    expires_at: inv.expires_at,
                    max_uses: inv.max_uses,
                    use_count: inv.use_count,
                    revoked: inv.revoked_at.is_some(),
                })
                .collect::<Vec<_>>()
                .pipe(|invitations| InvitationListResponse { invitations })
                .pipe(Response::Success))
        }
        Err(error) => {
            error!(?error, "invitations.list.error");
            Ok(Response::Error(error.to_string()))
        }
    }
}

#[derive(Debug)]
pub enum Response {
    Success(InvitationListResponse),
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
