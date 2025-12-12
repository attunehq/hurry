//! OAuth authentication endpoints.
//!
//! These endpoints handle the GitHub OAuth flow for user authentication.
//! See RFC docs/rfc/0003-self-service-signup.md for the full flow.

use aerosol::axum::Dep;
use axum::{
    Router,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use oauth2::PkceCodeVerifier;
use serde::Deserialize;
use time::{Duration, OffsetDateTime};
use tracing::{error, info, warn};

use crate::{
    api::State,
    auth::SessionContext,
    crypto::generate_session_token,
    db::Postgres,
    oauth::{self, GitHub},
};

/// Session duration: 24 hours.
const SESSION_DURATION: Duration = Duration::hours(24);

/// OAuth state expiration: 10 minutes.
const OAUTH_STATE_DURATION: Duration = Duration::minutes(10);

pub fn router() -> Router<State> {
    Router::new()
        .route("/github/start", get(start))
        .route("/github/callback", get(callback))
        .route("/logout", post(logout))
}

/// Query parameters for the OAuth start endpoint.
#[derive(Debug, Deserialize)]
pub struct StartParams {
    /// The URL to redirect to after authentication.
    redirect_uri: String,
}

/// Start the GitHub OAuth flow.
///
/// Validates the redirect URI, generates PKCE challenge and state token,
/// stores them in the database, and redirects to GitHub's authorization URL.
///
/// ## Endpoint
/// ```
/// GET /api/v1/oauth/github/start?redirect_uri=https://site.example.com/callback
/// ```
///
/// ## Responses
/// - 302: Redirect to GitHub authorization URL
/// - 400: Invalid redirect URI
/// - 503: OAuth not configured
#[tracing::instrument(skip(db, github))]
pub async fn start(
    Dep(db): Dep<Postgres>,
    Dep(github): Dep<Option<GitHub>>,
    Query(params): Query<StartParams>,
) -> StartResponse {
    let Some(github) = github.as_ref() else {
        warn!("oauth.start.not_configured");
        return StartResponse::NotConfigured;
    };

    // Validate redirect URI against allowlist
    let redirect_uri = match github.validate_redirect_uri(&params.redirect_uri) {
        Ok(uri) => uri,
        Err(err) => {
            warn!(?err, "oauth.start.invalid_redirect_uri");
            return StartResponse::InvalidRedirectUri(err.to_string());
        }
    };

    // Generate authorization URL with PKCE
    let (auth_url, pkce_verifier, csrf_token) = github.authorization_url(redirect_uri.clone());

    // Store state in database
    let expires_at = OffsetDateTime::now_utc() + OAUTH_STATE_DURATION;
    if let Err(err) = db
        .store_oauth_state(
            csrf_token.secret(),
            pkce_verifier.secret(),
            redirect_uri.as_str(),
            expires_at,
        )
        .await
    {
        error!(?err, "oauth.start.store_state_error");
        return StartResponse::Error(format!("Failed to store OAuth state: {}", err));
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

/// Query parameters for the OAuth callback endpoint.
#[derive(Debug, Deserialize)]
pub struct CallbackParams {
    /// The authorization code from GitHub.
    code: String,
    /// The state token (must match what we stored).
    state: String,
}

/// Handle the GitHub OAuth callback.
///
/// Validates the state token, exchanges the authorization code for an access
/// token, fetches the user profile from GitHub, creates or updates the account,
/// creates a session, and redirects to the original redirect URI with the
/// session token.
///
/// ## Endpoint
/// ```
/// GET /api/v1/oauth/github/callback?code=...&state=...
/// ```
///
/// ## Responses
/// - 302: Redirect to original redirect_uri with
///   `?session=...&new_user=true|false`
/// - 400: Invalid state or code
/// - 503: OAuth not configured
#[tracing::instrument(skip(db, github, params), fields(state = %params.state))]
pub async fn callback(
    Dep(db): Dep<Postgres>,
    Dep(github): Dep<Option<GitHub>>,
    Query(params): Query<CallbackParams>,
) -> CallbackResponse {
    let Some(github) = github.as_ref() else {
        warn!("oauth.callback.not_configured");
        return CallbackResponse::NotConfigured;
    };

    // Consume OAuth state (validates and deletes atomically)
    let oauth_state = match db.consume_oauth_state(&params.state).await {
        Ok(Some(state)) => state,
        Ok(None) => {
            warn!("oauth.callback.invalid_state");
            return CallbackResponse::InvalidState;
        }
        Err(err) => {
            error!(?err, "oauth.callback.state_error");
            return CallbackResponse::Error(format!("Failed to validate OAuth state: {}", err));
        }
    };

    // Parse redirect URI
    let redirect_uri = match oauth2::url::Url::parse(&oauth_state.redirect_uri) {
        Ok(uri) => uri,
        Err(err) => {
            error!(?err, "oauth.callback.invalid_stored_redirect_uri");
            return CallbackResponse::Error(String::from("Invalid stored redirect URI"));
        }
    };

    // Exchange code for access token
    let pkce_verifier = PkceCodeVerifier::new(oauth_state.pkce_verifier);
    let access_token = match github
        .exchange_code(params.code, redirect_uri.clone(), pkce_verifier)
        .await
    {
        Ok(token) => token,
        Err(err) => {
            warn!(?err, "oauth.callback.token_exchange_error");
            // Log audit event for failed OAuth
            let _ = db
                .log_audit_event(
                    None,
                    None,
                    "oauth.failure",
                    Some(serde_json::json!({ "error": err.to_string() })),
                )
                .await;
            return CallbackResponse::TokenExchangeFailed;
        }
    };

    // Fetch user profile from GitHub
    let github_user = match oauth::fetch_user(&access_token).await {
        Ok(user) => user,
        Err(err) => {
            error!(?err, "oauth.callback.fetch_user_error");
            let _ = db
                .log_audit_event(
                    None,
                    None,
                    "oauth.failure",
                    Some(serde_json::json!({ "error": err.to_string() })),
                )
                .await;
            return CallbackResponse::Error(format!("Failed to fetch GitHub user: {}", err));
        }
    };

    // Fetch user emails from GitHub
    let emails = match oauth::fetch_emails(&access_token).await {
        Ok(emails) => emails,
        Err(err) => {
            error!(?err, "oauth.callback.fetch_emails_error");
            let _ = db
                .log_audit_event(
                    None,
                    None,
                    "oauth.failure",
                    Some(serde_json::json!({ "error": err.to_string() })),
                )
                .await;
            return CallbackResponse::Error(format!("Failed to fetch GitHub emails: {}", err));
        }
    };

    // Get primary verified email
    let email = oauth::primary_email(&emails)
        .or(github_user.email.as_deref())
        .unwrap_or_default();

    if email.is_empty() {
        warn!(github_user_id = github_user.id, "oauth.callback.no_email");
        return CallbackResponse::NoEmail;
    }

    // Check if user already exists
    let (account_id, new_user) = match db.get_account_by_github_id(github_user.id).await {
        Ok(Some(account)) => {
            // Existing user - update email and username if changed
            if account.email != email
                && let Err(err) = db.update_account_email(account.id, email).await
            {
                error!(?err, "oauth.callback.update_email_error");
            }
            if let Err(err) = db
                .update_github_username(account.id, &github_user.login)
                .await
            {
                error!(?err, "oauth.callback.update_username_error");
            }

            // Check if account is disabled
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
            // New user - create account and default organization
            // Create the account first (accounts can exist without orgs now)
            let account_id = match db.create_account(email, github_user.name.as_deref()).await {
                Ok(id) => id,
                Err(err) => {
                    error!(?err, "oauth.callback.create_account_error");
                    return CallbackResponse::Error(format!("Failed to create account: {}", err));
                }
            };

            // Link GitHub identity
            if let Err(err) = db
                .link_github_identity(account_id, github_user.id, &github_user.login)
                .await
            {
                error!(?err, "oauth.callback.link_identity_error");
                return CallbackResponse::Error(format!("Failed to link GitHub identity: {}", err));
            }

            // Create a default organization for the user
            let org_id = match db
                .create_organization(&format!("{}'s Org", github_user.login))
                .await
            {
                Ok(org_id) => org_id,
                Err(err) => {
                    error!(?err, "oauth.callback.create_org_error");
                    return CallbackResponse::Error(format!(
                        "Failed to create organization: {}",
                        err
                    ));
                }
            };

            // Add user as admin of their org
            if let Err(err) = db
                .add_organization_member(org_id, account_id, crate::auth::OrgRole::Admin)
                .await
            {
                error!(?err, "oauth.callback.add_member_error");
                // Non-fatal, continue
            }

            // Log account creation
            let _ = db
                .log_audit_event(
                    Some(account_id),
                    Some(org_id),
                    "account.created",
                    Some(serde_json::json!({
                        "github_user_id": github_user.id,
                        "github_username": github_user.login,
                    })),
                )
                .await;

            info!(
                account_id = %account_id,
                github_user_id = github_user.id,
                "oauth.callback.new_user"
            );
            (account_id, true)
        }
        Err(err) => {
            error!(?err, "oauth.callback.lookup_error");
            return CallbackResponse::Error(format!("Failed to lookup account: {}", err));
        }
    };

    // Create session
    let session_token = generate_session_token();
    let expires_at = OffsetDateTime::now_utc() + SESSION_DURATION;

    if let Err(err) = db
        .create_session(account_id, &session_token, expires_at)
        .await
    {
        error!(?err, "oauth.callback.create_session_error");
        return CallbackResponse::Error(format!("Failed to create session: {}", err));
    }

    // Log successful OAuth
    let _ = db
        .log_audit_event(
            Some(account_id),
            None,
            "oauth.success",
            Some(serde_json::json!({
                "github_user_id": github_user.id,
                "github_username": github_user.login,
                "new_user": new_user,
            })),
        )
        .await;

    // Log session creation
    let _ = db
        .log_audit_event(Some(account_id), None, "session.created", None)
        .await;

    // Clean up expired OAuth states lazily (don't block on it)
    let db_cleanup = db.clone();
    tokio::spawn(async move {
        if let Err(err) = db_cleanup.cleanup_expired_oauth_state().await {
            error!(?err, "oauth.cleanup.error");
        }
    });

    // Redirect to original redirect URI with session token
    let mut final_redirect = redirect_uri;
    final_redirect
        .query_pairs_mut()
        .append_pair("session", session_token.expose())
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

/// Log out the current session.
///
/// Revokes the session token, invalidating it for future requests.
///
/// ## Endpoint
/// ```
/// POST /api/v1/oauth/logout
/// Authorization: Bearer <session_token>
/// ```
///
/// ## Responses
/// - 204: Session revoked
/// - 401: Not authenticated
#[tracing::instrument(skip(db, session))]
pub async fn logout(Dep(db): Dep<Postgres>, session: SessionContext) -> LogoutResponse {
    match db.revoke_session(&session.session_token).await {
        Ok(true) => {
            // Log session revocation
            let _ = db
                .log_audit_event(Some(session.account_id), None, "session.revoked", None)
                .await;
            info!(account_id = %session.account_id, "oauth.logout.success");
            LogoutResponse::Success
        }
        Ok(false) => {
            warn!(account_id = %session.account_id, "oauth.logout.session_not_found");
            LogoutResponse::Success // Still return success - session is gone either way
        }
        Err(err) => {
            error!(?err, "oauth.logout.error");
            LogoutResponse::Error(err.to_string())
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
