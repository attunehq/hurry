//! List organization audit log endpoint.

use aerosol::axum::Dep;
use axum::{Json, extract::Path, extract::Query, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};
use tap::Pipe;
use time::OffsetDateTime;
use tracing::{error, info};

use crate::{
    auth::{ApiError, OrgId, SessionContext},
    db::{Postgres, audit::AuditLogCursor},
};

#[derive(Debug, Deserialize)]
pub struct ListParams {
    /// Maximum number of entries to return. Defaults to 25.
    #[serde(default = "default_limit")]
    pub limit: i64,

    /// Cursor for pagination: the created_at timestamp of the last entry seen.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub cursor_time: Option<OffsetDateTime>,

    /// Cursor for pagination: the ID of the last entry seen.
    #[serde(default)]
    pub cursor_id: Option<i64>,
}

fn default_limit() -> i64 {
    25
}

#[derive(Debug, Serialize)]
pub struct AuditLogListResponse {
    /// The list of audit log entries.
    pub entries: Vec<AuditLogEntry>,

    /// Whether there are more entries after these (for "Next" button).
    pub has_more: bool,
}

#[derive(Debug, Serialize)]
pub struct AuditLogEntry {
    /// The audit log entry ID.
    pub id: i64,

    /// The account ID that performed the action (if known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<i64>,

    /// The email of the account that performed the action (if known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_email: Option<String>,

    /// The name of the account that performed the action (if known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_name: Option<String>,

    /// The action that was performed.
    pub action: String,

    /// Additional details about the action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,

    /// When the action was performed.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// List audit log entries for an organization.
///
/// Only admins can view the audit log.
#[tracing::instrument(skip(db, session))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
    Query(params): Query<ListParams>,
) -> Result<Response, ApiError> {
    let org_id = OrgId::from_i64(org_id);

    // Verify admin access using strongly typed role check
    let _admin = session.try_admin(&db, org_id).await?;

    // Clamp limit to reasonable range, fetch one extra to check has_more
    let limit = params.limit.clamp(1, 100);
    let fetch_limit = limit + 1;

    let cursor = match (params.cursor_time, params.cursor_id) {
        (Some(created_at), Some(id)) => Some(AuditLogCursor { created_at, id }),
        _ => None,
    };

    let entries = match db.list_audit_log(org_id, fetch_limit, cursor).await {
        Ok(entries) => entries,
        Err(error) => {
            error!(?error, "organizations.audit_log.list.error");
            return Ok(Response::Error(error.to_string()));
        }
    };

    let has_more = entries.len() as i64 > limit;
    let entries = entries.into_iter().take(limit as usize).collect::<Vec<_>>();

    info!(
        org_id = %org_id,
        count = entries.len(),
        has_more = has_more,
        "organizations.audit_log.list.success"
    );

    Ok(entries
        .into_iter()
        .map(|entry| AuditLogEntry {
            id: entry.id,
            account_id: entry.account_id.map(|id| id.as_i64()),
            account_email: entry.account_email,
            account_name: entry.account_name,
            action: entry.action,
            details: entry.details,
            created_at: entry.created_at,
        })
        .collect::<Vec<_>>()
        .pipe(|entries| AuditLogListResponse { entries, has_more })
        .pipe(Response::Success))
}

#[derive(Debug)]
pub enum Response {
    Success(AuditLogListResponse),
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
