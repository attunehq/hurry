//! Start GitHub OAuth flow endpoint.

use aerosol::axum::Dep;
use axum::{
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use time::OffsetDateTime;
use tracing::{error, info, warn};

use crate::{db::Postgres, oauth::GitHub};

use super::OAUTH_STATE_DURATION;

#[derive(Debug, Deserialize)]
pub struct StartParams {
    /// The URL to redirect to after authentication.
    redirect_uri: String,
}

/// Start the GitHub OAuth flow.
#[tracing::instrument(skip(db, github))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    Dep(github): Dep<Option<GitHub>>,
    Query(params): Query<StartParams>,
) -> StartResponse {
    let Some(github) = github.as_ref() else {
        warn!("oauth.start.not_configured");
        return StartResponse::NotConfigured;
    };

    let redirect_uri = match github.validate_redirect_uri(&params.redirect_uri) {
        Ok(uri) => uri,
        Err(error) => {
            warn!(?error, "oauth.start.invalid_redirect_uri");
            return StartResponse::InvalidRedirectUri(error.to_string());
        }
    };

    // Generate authorization URL using courier's callback URL (not the client's
    // redirect_uri). The client's redirect_uri is stored in oauth_state and used
    // after the callback to redirect the user back to the client.
    let (auth_url, pkce_verifier, csrf_token) = github.authorization_url();
    let expires_at = OffsetDateTime::now_utc() + OAUTH_STATE_DURATION;
    if let Err(error) = db
        .store_oauth_state(
            csrf_token.secret(),
            pkce_verifier.secret(),
            redirect_uri.as_str(),
            expires_at,
        )
        .await
    {
        error!(?error, "oauth.start.store_state_error");
        return StartResponse::Error(format!("Failed to store OAuth state: {error}"));
    }

    info!("oauth.start.redirecting");
    StartResponse::Redirect(auth_url.to_string())
}

#[derive(Debug)]
pub enum StartResponse {
    Redirect(String),
    InvalidRedirectUri(String),
    NotConfigured,
    Error(String),
}

impl IntoResponse for StartResponse {
    fn into_response(self) -> Response {
        match self {
            StartResponse::Redirect(url) => Redirect::temporary(&url).into_response(),
            StartResponse::InvalidRedirectUri(msg) => (
                StatusCode::BAD_REQUEST,
                format!("Invalid redirect URI: {msg}"),
            )
                .into_response(),
            StartResponse::NotConfigured => (
                StatusCode::SERVICE_UNAVAILABLE,
                "OAuth is not configured on this server",
            )
                .into_response(),
            StartResponse::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
