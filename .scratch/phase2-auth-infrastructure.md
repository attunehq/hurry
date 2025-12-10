# Phase 2: Core Types & Auth Infrastructure

Parent: `.scratch/self-service-signup.md`

## Overview

Set up foundational types and infrastructure for the new auth system:
- New auth types for sessions and roles
- GitHub OAuth configuration
- OAuth client module
- Crypto helpers for token generation
- Dependencies

## Configuration Changes

### ServeConfig in `main.rs`

Add optional GitHub OAuth config (optional to allow existing deployments to work):

```rust
/// GitHub App Client ID for OAuth authentication
#[arg(long, env = "GITHUB_CLIENT_ID")]
github_client_id: Option<String>,

/// GitHub App Client Secret for OAuth authentication
#[arg(long, env = "GITHUB_CLIENT_SECRET")]
#[debug(ignore)]
github_client_secret: Option<String>,

/// Comma-separated list of allowed redirect URIs for OAuth
#[arg(long, env = "GITHUB_REDIRECT_ALLOWLIST")]
github_redirect_allowlist: Option<String>,
```

## New Types in `auth.rs`

### SessionToken

Similar to `RawToken` but for sessions:

```rust
/// A session token for web UI authentication.
/// Separate from API keys - sessions are short-lived and user-facing.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Display, Deserialize, Serialize)]
#[debug("[session-redacted]")]
#[display("[session-redacted]")]
pub struct SessionToken(String);

impl SessionToken {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}
```

### OrgRole

Enum matching database `organization_role` table:

```rust
/// Organization role enum matching database values.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OrgRole {
    Member,
    Admin,
}

impl OrgRole {
    /// Convert to database role_id
    pub fn to_role_id(&self) -> i64 {
        match self {
            OrgRole::Member => 1,
            OrgRole::Admin => 2,
        }
    }

    /// Convert from database role_id
    pub fn from_role_id(id: i64) -> Option<Self> {
        match id {
            1 => Some(OrgRole::Member),
            2 => Some(OrgRole::Admin),
            _ => None,
        }
    }
}
```

### SessionContext

Authenticated session for web UI:

```rust
/// Authenticated session context from web UI.
/// Unlike AuthenticatedToken, sessions don't have implicit org context.
#[derive(Clone, Debug)]
pub struct SessionContext {
    pub account_id: AccountId,
    pub session_id: i64,
}
```

Note: `SessionContext` extractor implementation will be done in Phase 4 (OAuth Endpoints).

## OAuth Client Module (`oauth.rs`)

Create new module for GitHub OAuth:

```rust
//! GitHub OAuth client for authentication flow.

use color_eyre::{Result, eyre::Context};
use serde::Deserialize;
use url::Url;

/// GitHub OAuth client configuration and methods.
pub struct GitHubClient {
    client_id: String,
    client_secret: String,
    redirect_allowlist: Vec<String>,
    http_client: reqwest::Client,
}

impl GitHubClient {
    /// Create a new GitHub OAuth client.
    pub fn new(
        client_id: String,
        client_secret: String,
        redirect_allowlist: String,
    ) -> Self {
        Self {
            client_id,
            client_secret,
            redirect_allowlist: redirect_allowlist
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            http_client: reqwest::Client::new(),
        }
    }

    /// Check if a redirect URI is allowed.
    pub fn is_redirect_allowed(&self, uri: &str) -> bool {
        self.redirect_allowlist
            .iter()
            .any(|allowed| uri.starts_with(allowed))
    }

    /// Generate the GitHub authorization URL.
    pub fn authorization_url(
        &self,
        redirect_uri: &str,
        state: &str,
        code_challenge: &str,
    ) -> String {
        format!(
            "https://github.com/login/oauth/authorize?\
             client_id={}&\
             redirect_uri={}&\
             state={}&\
             code_challenge={}&\
             code_challenge_method=S256",
            self.client_id,
            urlencoding::encode(redirect_uri),
            state,
            code_challenge,
        )
    }

    /// Exchange authorization code for access token.
    pub async fn exchange_code(
        &self,
        code: &str,
        verifier: &str,
    ) -> Result<String> {
        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
        }

        let response = self.http_client
            .post("https://github.com/login/oauth/access_token")
            .header("Accept", "application/json")
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("code", code),
                ("code_verifier", verifier),
            ])
            .send()
            .await
            .context("send token exchange request")?
            .error_for_status()
            .context("token exchange failed")?
            .json::<TokenResponse>()
            .await
            .context("parse token response")?;

        Ok(response.access_token)
    }

    /// Fetch user profile from GitHub.
    pub async fn fetch_user(&self, access_token: &str) -> Result<GitHubUser> {
        self.http_client
            .get("https://api.github.com/user")
            .header("Authorization", format!("Bearer {}", access_token))
            .header("User-Agent", "Courier")
            .send()
            .await
            .context("fetch user profile")?
            .error_for_status()
            .context("user profile request failed")?
            .json()
            .await
            .context("parse user profile")
    }

    /// Fetch user's primary email from GitHub.
    pub async fn fetch_primary_email(&self, access_token: &str) -> Result<String> {
        #[derive(Deserialize)]
        struct Email {
            email: String,
            primary: bool,
        }

        let emails: Vec<Email> = self.http_client
            .get("https://api.github.com/user/emails")
            .header("Authorization", format!("Bearer {}", access_token))
            .header("User-Agent", "Courier")
            .send()
            .await
            .context("fetch user emails")?
            .error_for_status()
            .context("user emails request failed")?
            .json()
            .await
            .context("parse user emails")?;

        emails
            .into_iter()
            .find(|e| e.primary)
            .map(|e| e.email)
            .ok_or_else(|| color_eyre::eyre::eyre!("no primary email found"))
    }
}

/// GitHub user profile.
#[derive(Debug, Deserialize)]
pub struct GitHubUser {
    pub id: i64,
    pub login: String,
    pub name: Option<String>,
    pub email: Option<String>,
}
```

## Crypto Helpers in `crypto.rs`

Add token generation functions:

```rust
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use sha2::{Sha256, Digest};

/// Generate PKCE code verifier (43-128 URL-safe chars).
/// Uses 32 bytes of randomness = 256 bits entropy.
pub fn generate_pkce_verifier() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Generate PKCE code challenge from verifier using S256 method.
pub fn generate_pkce_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    URL_SAFE_NO_PAD.encode(hash)
}

/// Generate OAuth state token (128 bits entropy, 32 hex chars).
pub fn generate_oauth_state() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Generate session token (256 bits entropy, 64 hex chars).
pub fn generate_session_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Generate invitation token.
/// - Short (8 chars, ~47 bits) for ≤30 day expiry
/// - Long (12 chars, ~71 bits) for >30 day or never expiry
pub fn generate_invitation_token(long_lived: bool) -> String {
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let len = if long_lived { 12 } else { 8 };

    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| {
            let idx = (rng.next_u32() as usize) % CHARSET.len();
            CHARSET[idx] as char
        })
        .collect()
}
```

## Dependencies to Add

```bash
# Production dependencies
cargo add base64 --package courier
cargo add urlencoding --package courier
cargo add reqwest --features json --package courier

# Rate limiting (used later but add now)
cargo add tower-governor --package courier

# Dev dependencies for testing
cargo add --dev wiremock --package courier
```

## Update lib.rs

Export new module:

```rust
pub mod oauth;
```

## Checklist

- [ ] Add GitHub OAuth config to `ServeConfig` in main.rs
- [ ] Add `SessionToken` type to auth.rs
- [ ] Add `OrgRole` enum to auth.rs
- [ ] Add `SessionContext` struct to auth.rs
- [ ] Create `oauth.rs` module with `GitHubClient`
- [ ] Add PKCE helpers to crypto.rs
- [ ] Add `generate_session_token` to crypto.rs
- [ ] Add `generate_invitation_token` to crypto.rs
- [ ] Add `generate_oauth_state` to crypto.rs
- [ ] Add dependencies to Cargo.toml
- [ ] Export `oauth` module in lib.rs
- [ ] Unit tests for crypto functions
- [ ] Unit tests for `OrgRole` conversion
