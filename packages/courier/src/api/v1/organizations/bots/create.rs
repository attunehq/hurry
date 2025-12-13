//! Create organization bot endpoint.

use aerosol::axum::Dep;
use axum::{Json, extract::Path, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{error, info, warn};

use crate::{
    auth::{OrgId, SessionContext},
    db::Postgres,
};

#[derive(Debug, Deserialize)]
pub struct CreateBotRequest {
    /// The bot name.
    pub name: String,

    /// The email of the person/team responsible for this bot.
    pub responsible_email: String,
}

#[derive(Debug, Serialize)]
pub struct CreateBotResponse {
    /// The bot account ID.
    pub account_id: i64,

    /// The bot name.
    pub name: String,

    /// The API key token. Only returned once at creation.
    pub api_key: String,
}

/// Create a bot account for an organization.
#[tracing::instrument(skip(db, session))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
    Json(request): Json<CreateBotRequest>,
) -> Response {
    let org_id = OrgId::from_i64(org_id);

    match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(role)) if role.is_admin() => {}
        Ok(Some(_)) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.bots.create.not_admin"
            );
            return Response::Forbidden;
        }
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.bots.create.not_member"
            );
            return Response::Forbidden;
        }
        Err(error) => {
            error!(?error, "organizations.bots.create.role_check_error");
            return Response::Error(error.to_string());
        }
    }

    let name = request.name.trim();
    if name.is_empty() {
        return Response::EmptyName;
    }

    let email = request.responsible_email.trim();
    if email.is_empty() {
        return Response::EmptyEmail;
    }

    match db.create_bot_account(org_id, name, email).await {
        Ok((account_id, token)) => {
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    Some(org_id),
                    "bot.created",
                    Some(json!({
                        "bot_account_id": account_id.as_i64(),
                        "name": name,
                        "responsible_email": email,
                    })),
                )
                .await;

            info!(
                account_id = %session.account_id,
                org_id = %org_id,
                bot_account_id = %account_id,
                "organizations.bots.create.success"
            );

            Response::Created(CreateBotResponse {
                account_id: account_id.as_i64(),
                name: name.to_string(),
                api_key: token.expose().to_string(),
            })
        }
        Err(error) => {
            error!(?error, "organizations.bots.create.error");
            Response::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum Response {
    Created(CreateBotResponse),
    EmptyName,
    EmptyEmail,
    Forbidden,
    Error(String),
}

impl IntoResponse for Response {
    fn into_response(self) -> axum::response::Response {
        match self {
            Response::Created(bot) => (StatusCode::CREATED, Json(bot)).into_response(),
            Response::EmptyName => {
                (StatusCode::BAD_REQUEST, "Bot name cannot be empty").into_response()
            }
            Response::EmptyEmail => {
                (StatusCode::BAD_REQUEST, "Responsible email cannot be empty").into_response()
            }
            Response::Forbidden => {
                (StatusCode::FORBIDDEN, "Only admins can create bot accounts").into_response()
            }
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
