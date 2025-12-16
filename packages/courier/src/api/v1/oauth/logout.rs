//! Logout endpoint.

use aerosol::axum::Dep;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use tracing::{error, info, warn};

use crate::{auth::SessionContext, db::Postgres};

/// Log out the current session.
#[tracing::instrument(skip(db, session))]
pub async fn handle(Dep(db): Dep<Postgres>, session: SessionContext) -> LogoutResponse {
    match db.revoke_session(&session.session_token).await {
        Ok(true) => {
            let _ = db
                .log_audit_event(Some(session.account_id), None, "session.revoked", None)
                .await;
            info!(account_id = %session.account_id, "oauth.logout.success");
            LogoutResponse::Success
        }
        Ok(false) => {
            // Still return success - session is gone either way
            warn!(account_id = %session.account_id, "oauth.logout.session_not_found");
            LogoutResponse::Success
        }
        Err(error) => {
            error!(?error, "oauth.logout.error");
            LogoutResponse::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum LogoutResponse {
    Success,
    Error(String),
}

impl IntoResponse for LogoutResponse {
    fn into_response(self) -> Response {
        match self {
            LogoutResponse::Success => StatusCode::NO_CONTENT.into_response(),
            LogoutResponse::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
