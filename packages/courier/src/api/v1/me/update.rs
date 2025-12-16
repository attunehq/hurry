//! Update current user profile endpoint.

use aerosol::axum::Dep;
use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Deserialize;
use serde_json::json;
use tracing::{error, info, warn};

use crate::{auth::SessionContext, db::Postgres};

#[derive(Debug, Deserialize)]
pub struct UpdateMeRequest {
    /// The new name for the account.
    pub name: Option<String>,
}

/// Update the current user's profile.
#[tracing::instrument(skip(db, session))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Json(request): Json<UpdateMeRequest>,
) -> Response {
    // Validate name is not empty if provided
    if let Some(ref name) = request.name
        && name.trim().is_empty()
    {
        warn!(account_id = %session.account_id, "me.update.empty_name");
        return Response::EmptyName;
    }

    // Get the trimmed name (or None if clearing)
    let name = request.name.as_deref().map(str::trim);

    match db.update_account_name(session.account_id, name).await {
        Ok(()) => {
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    None,
                    "account.name_updated",
                    Some(json!({
                        "new_name": name,
                    })),
                )
                .await;

            info!(account_id = %session.account_id, "me.update.success");
            Response::Success
        }
        Err(error) => {
            error!(?error, "me.update.error");
            Response::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum Response {
    Success,
    EmptyName,
    Error(String),
}

impl IntoResponse for Response {
    fn into_response(self) -> axum::response::Response {
        match self {
            Response::Success => StatusCode::NO_CONTENT.into_response(),
            Response::EmptyName => {
                (StatusCode::BAD_REQUEST, "Name cannot be empty").into_response()
            }
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
