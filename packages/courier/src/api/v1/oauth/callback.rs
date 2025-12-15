//! GitHub OAuth callback endpoint.

use aerosol::axum::Dep;
use axum::{
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use oauth2::PkceCodeVerifier;
use serde::Deserialize;
use serde_json::json;
use time::OffsetDateTime;
use tracing::{error, info, warn};

use crate::{
    db::Postgres,
    oauth::{self, GitHub},
};

use super::EXCHANGE_CODE_DURATION;

#[derive(Debug, Deserialize)]
pub struct CallbackParams {
    /// The authorization code from GitHub.
    code: String,

    /// The state token (must match what we stored).
    state: String,
}

/// Handle the GitHub OAuth callback.
#[tracing::instrument(skip(db, github, params), fields(state = %params.state))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    Dep(github): Dep<Option<GitHub>>,
    Query(params): Query<CallbackParams>,
) -> CallbackResponse {
    let Some(github) = github.as_ref() else {
        warn!("oauth.callback.not_configured");
        return CallbackResponse::NotConfigured;
    };

    let oauth_state = match db.consume_oauth_state(&params.state).await {
        Ok(Some(state)) => state,
        Ok(None) => {
            warn!("oauth.callback.invalid_state");
            return CallbackResponse::InvalidState;
        }
        Err(error) => {
            error!(?error, "oauth.callback.state_error");
            return CallbackResponse::Error(format!("Failed to validate OAuth state: {error}"));
        }
    };

    let redirect_uri = match oauth2::url::Url::parse(&oauth_state.redirect_uri) {
        Ok(uri) => uri,
        Err(error) => {
            error!(?error, "oauth.callback.invalid_stored_redirect_uri");
            return CallbackResponse::Error(String::from("Invalid stored redirect URI"));
        }
    };

    let pkce_verifier = PkceCodeVerifier::new(oauth_state.pkce_verifier);
    // Exchange the authorization code for an access token. The redirect_uri used
    // here is courier's callback URL (stored in the GitHub client), which must
    // match what was sent in the authorization request.
    let access_token = match github.exchange_code(params.code, pkce_verifier).await {
        Ok(token) => token,
        Err(error) => {
            warn!(?error, "oauth.callback.token_exchange_error");
            let _ = db
                .log_audit_event(
                    None,
                    None,
                    "oauth.failure",
                    Some(json!({ "error": error.to_string() })),
                )
                .await;
            return CallbackResponse::TokenExchangeFailed;
        }
    };

    let github_user = match oauth::fetch_user(&access_token).await {
        Ok(user) => user,
        Err(error) => {
            error!(?error, "oauth.callback.fetch_user_error");
            let _ = db
                .log_audit_event(
                    None,
                    None,
                    "oauth.failure",
                    Some(json!({ "error": error.to_string() })),
                )
                .await;
            return CallbackResponse::Error(format!("Failed to fetch GitHub user: {error}"));
        }
    };

    let emails = match oauth::fetch_emails(&access_token).await {
        Ok(emails) => emails,
        Err(error) => {
            error!(?error, "oauth.callback.fetch_emails_error");
            let _ = db
                .log_audit_event(
                    None,
                    None,
                    "oauth.failure",
                    Some(json!({ "error": error.to_string() })),
                )
                .await;
            return CallbackResponse::Error(format!("Failed to fetch GitHub emails: {error}"));
        }
    };

    let email = oauth::primary_email(&emails)
        .or(github_user.email.as_deref())
        .unwrap_or_default();

    if email.is_empty() {
        warn!(github_user_id = github_user.id, "oauth.callback.no_email");
        return CallbackResponse::NoEmail;
    }

    let (account_id, new_user) = match db.get_account_by_github_id(github_user.id).await {
        Ok(Some(account)) => {
            if account.email != email
                && let Err(error) = db.update_account_email(account.id, email).await
            {
                error!(?error, "oauth.callback.update_email_error");
            }
            if let Err(error) = db
                .update_github_username(account.id, &github_user.login)
                .await
            {
                error!(?error, "oauth.callback.update_username_error");
            }

            if account.disabled_at.is_some() {
                warn!(
                    account_id = %account.id,
                    "oauth.callback.account_disabled"
                );
                return CallbackResponse::AccountDisabled;
            }

            info!(
                account_id = %account.id,
                github_user_id = github_user.id,
                "oauth.callback.existing_user"
            );
            (account.id, false)
        }
        Ok(None) => {
            let org_name = format!("{}'s Org", github_user.login);
            let signup_result = match db
                .signup_with_github(
                    email,
                    github_user.name.as_deref(),
                    github_user.id,
                    &github_user.login,
                    &org_name,
                )
                .await
            {
                Ok(result) => result,
                Err(error) => {
                    error!(?error, "oauth.callback.signup_error");
                    return CallbackResponse::Error(format!("Failed to create account: {error}"));
                }
            };

            let _ = db
                .log_audit_event(
                    Some(signup_result.account_id),
                    Some(signup_result.org_id),
                    "account.created",
                    Some(json!({
                        "github_user_id": github_user.id,
                        "github_username": github_user.login,
                    })),
                )
                .await;

            info!(
                account_id = %signup_result.account_id,
                github_user_id = github_user.id,
                "oauth.callback.new_user"
            );
            (signup_result.account_id, true)
        }
        Err(error) => {
            error!(?error, "oauth.callback.lookup_error");
            return CallbackResponse::Error(format!("Failed to lookup account: {error}"));
        }
    };

    let expires_at = OffsetDateTime::now_utc() + EXCHANGE_CODE_DURATION;

    let auth_code = db
        .create_exchange_code(
            account_id,
            oauth_state.redirect_uri.as_str(),
            new_user,
            expires_at,
        )
        .await;
    let auth_code = match auth_code {
        Ok(code) => code,
        Err(error) => {
            error!(?error, "oauth.callback.create_exchange_code_error");
            return CallbackResponse::Error(format!("Failed to create exchange code: {error}"));
        }
    };

    let _ = db
        .log_audit_event(
            Some(account_id),
            None,
            "oauth.success",
            Some(json!({
                "github_user_id": github_user.id,
                "github_username": github_user.login,
                "new_user": new_user,
            })),
        )
        .await;

    let db_cleanup = db.clone();
    tokio::spawn(async move {
        if let Err(error) = db_cleanup.cleanup_expired_oauth_state().await {
            error!(?error, "oauth.cleanup.state_error");
        }
        if let Err(error) = db_cleanup.cleanup_expired_exchange_codes().await {
            error!(?error, "oauth.cleanup.exchange_code_error");
        }
    });

    let mut final_redirect = redirect_uri;
    final_redirect
        .query_pairs_mut()
        .append_pair("auth_code", auth_code.expose())
        .append_pair("new_user", if new_user { "true" } else { "false" });

    info!("oauth.callback.success");
    CallbackResponse::Success(final_redirect.to_string())
}

#[derive(Debug)]
pub enum CallbackResponse {
    Success(String),
    InvalidState,
    TokenExchangeFailed,
    NoEmail,
    AccountDisabled,
    NotConfigured,
    Error(String),
}

impl IntoResponse for CallbackResponse {
    fn into_response(self) -> Response {
        match self {
            CallbackResponse::Success(url) => Redirect::temporary(&url).into_response(),
            CallbackResponse::InvalidState => (
                StatusCode::BAD_REQUEST,
                "Invalid or expired OAuth state. Please try again.",
            )
                .into_response(),
            CallbackResponse::TokenExchangeFailed => (
                StatusCode::BAD_REQUEST,
                "Failed to exchange authorization code. Please try again.",
            )
                .into_response(),
            CallbackResponse::NoEmail => (
                StatusCode::BAD_REQUEST,
                "No verified email found on your GitHub account. Please verify an email address on GitHub and try again.",
            )
                .into_response(),
            CallbackResponse::AccountDisabled => (
                StatusCode::FORBIDDEN,
                "Your account has been disabled. Please contact support.",
            )
                .into_response(),
            CallbackResponse::NotConfigured => (
                StatusCode::SERVICE_UNAVAILABLE,
                "OAuth is not configured on this server",
            )
                .into_response(),
            CallbackResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}
