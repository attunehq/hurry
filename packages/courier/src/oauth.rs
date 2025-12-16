//! GitHub OAuth client for user authentication.

use std::collections::HashSet;

use color_eyre::{
    Result,
    eyre::{bail, eyre},
};
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointNotSet, EndpointSet,
    PkceCodeChallenge, PkceCodeVerifier, TokenResponse, TokenUrl, basic::BasicClient,
    reqwest as oauth_reqwest, url::Url,
};

/// Configured OAuth client type with auth and token URLs set.
type ConfiguredClient =
    BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>;

/// Configuration for the GitHub OAuth client.
#[derive(Clone, Debug)]
pub struct GitHubConfig {
    /// GitHub OAuth Client ID.
    pub client_id: String,
    /// GitHub OAuth Client Secret.
    pub client_secret: String,
    /// Allowed redirect URIs (validated before initiating OAuth).
    pub redirect_allowlist: HashSet<String>,
}

/// GitHub OAuth client.
///
/// Handles the OAuth flow with GitHub for user authentication.
/// This client is optional - if not configured, OAuth endpoints are disabled.
#[derive(Clone)]
pub struct GitHub {
    client: ConfiguredClient,
    http_client: oauth_reqwest::Client,
    redirect_allowlist: HashSet<String>,
}

impl std::fmt::Debug for GitHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHub")
            .field("redirect_allowlist", &self.redirect_allowlist)
            .finish_non_exhaustive()
    }
}

impl GitHub {
    /// GitHub OAuth authorize URL.
    const AUTH_URL: &'static str = "https://github.com/login/oauth/authorize";
    /// GitHub OAuth token URL.
    const TOKEN_URL: &'static str = "https://github.com/login/oauth/access_token";
    /// GitHub API URL for fetching user info.
    pub const USER_API_URL: &'static str = "https://api.github.com/user";
    /// GitHub API URL for fetching user emails.
    pub const EMAILS_API_URL: &'static str = "https://api.github.com/user/emails";

    /// Create a new GitHub OAuth client from configuration.
    ///
    /// Returns `None` if the configuration is incomplete (missing client_id or
    /// client_secret).
    pub fn new(config: GitHubConfig) -> Option<Self> {
        if config.client_id.is_empty() || config.client_secret.is_empty() {
            return None;
        }

        let client = BasicClient::new(ClientId::new(config.client_id))
            .set_client_secret(ClientSecret::new(config.client_secret))
            .set_auth_uri(AuthUrl::new(Self::AUTH_URL.to_string()).expect("valid auth URL"))
            .set_token_uri(TokenUrl::new(Self::TOKEN_URL.to_string()).expect("valid token URL"));

        // Build HTTP client that doesn't follow redirects (security requirement)
        let http_client = oauth_reqwest::ClientBuilder::new()
            .redirect(oauth_reqwest::redirect::Policy::none())
            .build()
            .expect("Client should build");

        Some(Self {
            client,
            http_client,
            redirect_allowlist: config.redirect_allowlist,
        })
    }

    /// Validate that a redirect URI is in the allowlist.
    pub fn validate_redirect_uri(&self, uri: &str) -> Result<Url> {
        let parsed = Url::parse(uri).map_err(|e| eyre!("invalid redirect URI: {e}"))?;

        // Normalize: check origin (scheme + host + port)
        let origin = parsed.origin().ascii_serialization();

        // Check if any allowlisted URL has the same origin
        let allowed = self.redirect_allowlist.iter().any(|allowed| {
            Url::parse(allowed)
                .map(|u| u.origin().ascii_serialization() == origin)
                .unwrap_or(false)
        });

        if !allowed {
            return Err(eyre!("redirect URI not in allowlist: {uri}"));
        }

        Ok(parsed)
    }

    /// Generate the authorization URL for starting the OAuth flow.
    ///
    /// Returns the URL to redirect the user to, along with the PKCE verifier
    /// and CSRF state token that must be stored server-side.
    ///
    /// No redirect_uri is sent to GitHub - it uses the callback URL configured
    /// in the GitHub App settings. The client's redirect_uri is stored in
    /// oauth_state and used after the callback to redirect the user back.
    pub fn authorization_url(&self) -> (Url, PkceCodeVerifier, CsrfToken) {
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let (auth_url, csrf_token) = self
            .client
            .authorize_url(CsrfToken::new_random)
            .set_pkce_challenge(pkce_challenge)
            .url();

        (auth_url, pkce_verifier, csrf_token)
    }

    /// Exchange an authorization code for an access token.
    ///
    /// This should be called after the user is redirected back from GitHub
    /// with an authorization code.
    pub async fn exchange_code(
        &self,
        code: String,
        pkce_verifier: PkceCodeVerifier,
    ) -> Result<String> {
        let token_result = self
            .client
            .exchange_code(AuthorizationCode::new(code))
            .set_pkce_verifier(pkce_verifier)
            .request_async(&self.http_client)
            .await
            .map_err(|e| eyre!("token exchange failed: {e}"))?;

        Ok(token_result.access_token().secret().clone())
    }
}

/// User information from GitHub.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct GitHubUser {
    /// GitHub user ID (stable identifier).
    pub id: i64,
    /// GitHub username (can change).
    pub login: String,
    /// User's display name (optional).
    pub name: Option<String>,
    /// User's email (may be null if private).
    pub email: Option<String>,
}

/// Email information from GitHub.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct GitHubEmail {
    /// Email address.
    pub email: String,
    /// Whether this is the primary email.
    pub primary: bool,
    /// Whether this email is verified.
    pub verified: bool,
}

/// Fetch the authenticated user's profile from GitHub.
pub async fn fetch_user(access_token: &str) -> Result<GitHubUser> {
    let client = ::reqwest::Client::new();
    let response = client
        .get(GitHub::USER_API_URL)
        .bearer_auth(access_token)
        .header("User-Agent", "Courier")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| eyre!("failed to fetch user: {}", e))?;

    if !response.status().is_success() {
        bail!(
            "GitHub API error: {} {}",
            response.status(),
            response.text().await.unwrap_or_default()
        );
    }

    response
        .json()
        .await
        .map_err(|e| eyre!("failed to parse user response: {}", e))
}

/// Fetch the authenticated user's emails from GitHub.
pub async fn fetch_emails(access_token: &str) -> Result<Vec<GitHubEmail>> {
    let client = ::reqwest::Client::new();
    let response = client
        .get(GitHub::EMAILS_API_URL)
        .bearer_auth(access_token)
        .header("User-Agent", "Courier")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| eyre!("failed to fetch emails: {}", e))?;

    if !response.status().is_success() {
        bail!(
            "GitHub API error: {} {}",
            response.status(),
            response.text().await.unwrap_or_default()
        );
    }

    response
        .json()
        .await
        .map_err(|e| eyre!("failed to parse emails response: {}", e))
}

/// Get the primary verified email from a list of GitHub emails.
pub fn primary_email(emails: &[GitHubEmail]) -> Option<&str> {
    emails
        .iter()
        .find(|e| e.primary && e.verified)
        .map(|e| e.email.as_str())
}
