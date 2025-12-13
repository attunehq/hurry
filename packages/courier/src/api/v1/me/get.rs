//! Get current user profile endpoint.

use aerosol::axum::Dep;
use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Serialize;
use time::OffsetDateTime;
use tracing::{error, info};

use crate::{auth::SessionContext, db::Postgres};

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
pub async fn handle(Dep(db): Dep<Postgres>, session: SessionContext) -> Response {
    let account = match db.get_account(session.account_id).await {
        Ok(Some(account)) => account,
        Ok(None) => {
            error!(account_id = %session.account_id, "me.get.not_found");
            return Response::NotFound;
        }
        Err(error) => {
            error!(?error, "me.get.error");
            return Response::Error(error.to_string());
        }
    };

    let github_username = match db.get_github_identity(session.account_id).await {
        Ok(Some(identity)) => Some(identity.github_username),
        Ok(None) => None,
        Err(error) => {
            error!(?error, "me.get.github_identity_error");
            return Response::Error(error.to_string());
        }
    };

    info!(account_id = %session.account_id, "me.get.success");
    Response::Success(MeResponse {
        id: account.id.as_i64(),
        email: account.email,
        name: account.name,
        github_username,
        created_at: account.created_at,
    })
}

#[derive(Debug)]
pub enum Response {
    Success(MeResponse),
    NotFound,
    Error(String),
}

impl IntoResponse for Response {
    fn into_response(self) -> axum::response::Response {
        match self {
            Response::Success(me) => (StatusCode::OK, Json(me)).into_response(),
            Response::NotFound => (
                StatusCode::NOT_FOUND,
                "Account not found. This may indicate a database inconsistency.",
            )
                .into_response(),
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
