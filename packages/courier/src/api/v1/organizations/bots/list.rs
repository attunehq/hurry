//! List organization bots endpoint.

use aerosol::axum::Dep;
use axum::{Json, extract::Path, http::StatusCode, response::IntoResponse};
use serde::Serialize;
use tap::Pipe;
use time::OffsetDateTime;
use tracing::{error, info, warn};

use crate::{
    auth::{OrgId, SessionContext},
    db::Postgres,
};

#[derive(Debug, Serialize)]
pub struct BotListResponse {
    /// The list of bot accounts.
    pub bots: Vec<BotEntry>,
}

#[derive(Debug, Serialize)]
pub struct BotEntry {
    /// The bot account ID.
    pub account_id: i64,

    /// The bot name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// The email of the person/team responsible for this bot.
    pub responsible_email: String,

    /// The creation timestamp.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// List bot accounts for an organization.
#[tracing::instrument(skip(db, session))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
) -> Response {
    let org_id = OrgId::from_i64(org_id);

    match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(role)) if role.is_admin() => {}
        Ok(Some(_)) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.bots.list.not_admin"
            );
            return Response::Forbidden;
        }
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.bots.list.not_member"
            );
            return Response::Forbidden;
        }
        Err(error) => {
            error!(?error, "organizations.bots.list.role_check_error");
            return Response::Error(error.to_string());
        }
    }

    match db.list_bot_accounts(org_id).await {
        Ok(bots) => {
            info!(
                org_id = %org_id,
                count = bots.len(),
                "organizations.bots.list.success"
            );
            bots.into_iter()
                .map(|bot| BotEntry {
                    account_id: bot.id.as_i64(),
                    name: bot.name,
                    responsible_email: bot.email,
                    created_at: bot.created_at,
                })
                .collect::<Vec<_>>()
                .pipe(|bots| BotListResponse { bots })
                .pipe(Response::Success)
        }
        Err(error) => {
            error!(?error, "organizations.bots.list.error");
            Response::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum Response {
    Success(BotListResponse),
    Forbidden,
    Error(String),
}

impl IntoResponse for Response {
    fn into_response(self) -> axum::response::Response {
        match self {
            Response::Success(list) => (StatusCode::OK, Json(list)).into_response(),
            Response::Forbidden => {
                (StatusCode::FORBIDDEN, "Only admins can view bot accounts").into_response()
            }
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
