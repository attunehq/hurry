//! Exchange auth code for session token endpoint.

use aerosol::axum::Dep;
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tracing::{error, info, warn};

use crate::{
    auth::{AuthCode, SessionToken},
    db::{Postgres, RedeemExchangeCodeError},
};

use super::SESSION_DURATION;

#[derive(Debug, Deserialize)]
pub struct ExchangeRequest {
    /// The auth code received from the OAuth callback.
    auth_code: String,
}

#[derive(Debug, Serialize)]
pub struct ExchangeResponseBody {
    /// The session token to use for subsequent requests.
    session_token: String,
}

/// Exchange an auth code for a session token.
#[tracing::instrument(skip(db, request))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    Json(request): Json<ExchangeRequest>,
) -> ExchangeResponse {
    let auth_code = AuthCode::new(&request.auth_code);

    match db.redeem_exchange_code(&auth_code).await {
        Ok(Ok(redemption)) => {
            let session_token = SessionToken::generate();
            let expires_at = OffsetDateTime::now_utc() + SESSION_DURATION;

            if let Err(error) = db
                .create_session(redemption.account_id, &session_token, expires_at)
                .await
            {
                error!(?error, "oauth.exchange.create_session_error");
                return ExchangeResponse::Error(format!("Failed to create session: {error}"));
            }

            let _ = db
                .log_audit_event(Some(redemption.account_id), None, "session.created", None)
                .await;

            info!(
                account_id = %redemption.account_id,
                new_user = redemption.new_user,
                "oauth.exchange.success"
            );
            ExchangeResponse::Success(ExchangeResponseBody {
                session_token: session_token.expose().to_string(),
            })
        }
        Ok(Err(RedeemExchangeCodeError::NotFound)) => {
            warn!("oauth.exchange.not_found");
            ExchangeResponse::NotFound
        }
        Ok(Err(RedeemExchangeCodeError::Expired)) => {
            warn!("oauth.exchange.expired");
            ExchangeResponse::Expired
        }
        Ok(Err(RedeemExchangeCodeError::AlreadyRedeemed)) => {
            warn!("oauth.exchange.already_redeemed");
            ExchangeResponse::AlreadyRedeemed
        }
        Err(error) => {
            error!(?error, "oauth.exchange.error");
            ExchangeResponse::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum ExchangeResponse {
    Success(ExchangeResponseBody),
    NotFound,
    Expired,
    AlreadyRedeemed,
    Error(String),
}

impl IntoResponse for ExchangeResponse {
    fn into_response(self) -> Response {
        match self {
            ExchangeResponse::Success(body) => (StatusCode::OK, Json(body)).into_response(),
            ExchangeResponse::NotFound => (
                StatusCode::BAD_REQUEST,
                "Invalid auth code. Please try signing in again.",
            )
                .into_response(),
            ExchangeResponse::Expired => (
                StatusCode::BAD_REQUEST,
                "Auth code has expired. Please try signing in again.",
            )
                .into_response(),
            ExchangeResponse::AlreadyRedeemed => (
                StatusCode::BAD_REQUEST,
                "Auth code has already been used. Please try signing in again.",
            )
                .into_response(),
            ExchangeResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}
