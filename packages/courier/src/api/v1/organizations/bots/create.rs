//! Create organization bot endpoint.

use aerosol::axum::Dep;
use axum::{Json, extract::Path, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{error, info};

use crate::{
    auth::{ApiError, OrgId, SessionContext},
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
) -> Result<Response, ApiError> {
    let org_id = OrgId::from_i64(org_id);

    // Verify admin access using strongly typed role check
    let admin = session.try_admin(&db, org_id).await?;

    let name = request.name.trim();
    if name.is_empty() {
        return Ok(Response::EmptyName);
    }

    let email = request.responsible_email.trim();
    if email.is_empty() {
        return Ok(Response::EmptyEmail);
    }

    match db.create_bot_account(org_id, name, email).await {
        Ok((account_id, token)) => {
            let _ = db
                .log_audit_event(
                    Some(admin.account_id),
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
                account_id = %admin.account_id,
                org_id = %org_id,
                bot_account_id = %account_id,
                "organizations.bots.create.success"
            );

            Ok(Response::Created(CreateBotResponse {
                account_id: account_id.as_i64(),
                name: name.to_string(),
                api_key: token.expose().to_string(),
            }))
        }
        Err(error) => {
            error!(?error, "organizations.bots.create.error");
            Ok(Response::Error(error.to_string()))
        }
    }
}

#[derive(Debug)]
pub enum Response {
    Created(CreateBotResponse),
    EmptyName,
    EmptyEmail,
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
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
