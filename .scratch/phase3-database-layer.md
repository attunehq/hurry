# Phase 3: Database Layer

Parent: `.scratch/self-service-signup.md`

## Overview

Add database operations to `db.rs` for all new functionality. Operations grouped by domain. Each method follows existing patterns: `#[tracing::instrument]`, returns `Result`, uses `sqlx::query!`.

## Account Operations

```rust
/// Create a new account (used during OAuth signup).
#[tracing::instrument(name = "Postgres::create_account")]
pub async fn create_account(&self, email: &str, name: Option<&str>) -> Result<AccountId>

/// Get account by ID.
#[tracing::instrument(name = "Postgres::get_account")]
pub async fn get_account(&self, id: AccountId) -> Result<Option<Account>>

/// Get account by GitHub user ID.
#[tracing::instrument(name = "Postgres::get_account_by_github_id")]
pub async fn get_account_by_github_id(&self, github_user_id: i64) -> Result<Option<AccountId>>

/// Update account email (called on each OAuth login).
#[tracing::instrument(name = "Postgres::update_account_email")]
pub async fn update_account_email(&self, id: AccountId, email: &str) -> Result<()>

/// Update account name.
#[tracing::instrument(name = "Postgres::update_account_name")]
pub async fn update_account_name(&self, id: AccountId, name: Option<&str>) -> Result<()>

/// Disable account (sets disabled_at, invalidates sessions).
#[tracing::instrument(name = "Postgres::disable_account")]
pub async fn disable_account(&self, id: AccountId) -> Result<()>
```

## GitHub Identity Operations

```rust
/// Link GitHub identity to account.
#[tracing::instrument(name = "Postgres::link_github_identity")]
pub async fn link_github_identity(
    &self,
    account_id: AccountId,
    github_user_id: i64,
    github_username: &str,
) -> Result<()>

/// Get GitHub identity for account.
#[tracing::instrument(name = "Postgres::get_github_identity")]
pub async fn get_github_identity(&self, account_id: AccountId) -> Result<Option<GitHubIdentity>>

/// Update GitHub username (on each OAuth login).
#[tracing::instrument(name = "Postgres::update_github_username")]
pub async fn update_github_username(
    &self,
    github_user_id: i64,
    username: &str,
) -> Result<()>
```

## Session Operations

```rust
/// Create a new session for account. Returns session ID and token.
#[tracing::instrument(name = "Postgres::create_session", skip(self))]
pub async fn create_session(&self, account_id: AccountId) -> Result<(i64, SessionToken)>

/// Validate session token. Returns account_id and session_id if valid.
/// Also extends expiry (sliding window) on success.
#[tracing::instrument(name = "Postgres::validate_session", skip(token))]
pub async fn validate_session(&self, token: &SessionToken) -> Result<Option<SessionContext>>

/// Revoke a specific session.
#[tracing::instrument(name = "Postgres::revoke_session")]
pub async fn revoke_session(&self, session_id: i64) -> Result<()>

/// Revoke all sessions for account.
#[tracing::instrument(name = "Postgres::revoke_all_sessions")]
pub async fn revoke_all_sessions(&self, account_id: AccountId) -> Result<()>

/// Cleanup expired sessions. Returns count deleted. Called lazily.
#[tracing::instrument(name = "Postgres::cleanup_expired_sessions")]
pub async fn cleanup_expired_sessions(&self) -> Result<u64>
```

## OAuth State Operations

```rust
/// Store OAuth state for PKCE flow. Expires in 10 minutes.
#[tracing::instrument(name = "Postgres::store_oauth_state", skip(pkce_verifier))]
pub async fn store_oauth_state(
    &self,
    state_token: &str,
    pkce_verifier: &str,
    redirect_uri: &str,
) -> Result<()>

/// Consume OAuth state (one-time use). Returns verifier and redirect_uri.
#[tracing::instrument(name = "Postgres::consume_oauth_state")]
pub async fn consume_oauth_state(&self, state_token: &str) -> Result<Option<OAuthState>>

/// Cleanup expired OAuth states. Returns count deleted. Called lazily.
#[tracing::instrument(name = "Postgres::cleanup_expired_oauth_states")]
pub async fn cleanup_expired_oauth_states(&self) -> Result<u64>
```

## Organization Operations

```rust
/// Create organization with creator as admin.
#[tracing::instrument(name = "Postgres::create_organization")]
pub async fn create_organization(&self, name: &str, creator: AccountId) -> Result<OrgId>

/// Get organization by ID.
#[tracing::instrument(name = "Postgres::get_organization")]
pub async fn get_organization(&self, id: OrgId) -> Result<Option<Organization>>

/// List organizations for account with their roles.
#[tracing::instrument(name = "Postgres::list_account_organizations")]
pub async fn list_account_organizations(&self, account_id: AccountId) -> Result<Vec<OrgMembership>>
```

## Membership Operations

```rust
/// Add member to organization with role.
#[tracing::instrument(name = "Postgres::add_org_member")]
pub async fn add_org_member(
    &self,
    org_id: OrgId,
    account_id: AccountId,
    role: OrgRole,
) -> Result<()>

/// Get member's role in organization. None if not a member.
#[tracing::instrument(name = "Postgres::get_org_member_role")]
pub async fn get_org_member_role(
    &self,
    org_id: OrgId,
    account_id: AccountId,
) -> Result<Option<OrgRole>>

/// Update member's role.
#[tracing::instrument(name = "Postgres::update_org_member_role")]
pub async fn update_org_member_role(
    &self,
    org_id: OrgId,
    account_id: AccountId,
    role: OrgRole,
) -> Result<()>

/// Remove member from organization.
#[tracing::instrument(name = "Postgres::remove_org_member")]
pub async fn remove_org_member(&self, org_id: OrgId, account_id: AccountId) -> Result<()>

/// List all members of organization with their info.
#[tracing::instrument(name = "Postgres::list_org_members")]
pub async fn list_org_members(&self, org_id: OrgId) -> Result<Vec<OrgMember>>

/// Count admins in organization (for "last admin" check).
#[tracing::instrument(name = "Postgres::count_org_admins")]
pub async fn count_org_admins(&self, org_id: OrgId) -> Result<i64>

/// Check if account is member of organization.
#[tracing::instrument(name = "Postgres::is_org_member")]
pub async fn is_org_member(&self, org_id: OrgId, account_id: AccountId) -> Result<bool>

/// Check if account is admin of organization.
#[tracing::instrument(name = "Postgres::is_org_admin")]
pub async fn is_org_admin(&self, org_id: OrgId, account_id: AccountId) -> Result<bool>
```

## Invitation Operations

```rust
/// Create invitation. Returns invitation ID and token.
#[tracing::instrument(name = "Postgres::create_invitation")]
pub async fn create_invitation(
    &self,
    org_id: OrgId,
    role: OrgRole,
    created_by: AccountId,
    expires_at: DateTime<Utc>,
    max_uses: Option<i32>,
) -> Result<(i64, String)>

/// Get invitation by token.
#[tracing::instrument(name = "Postgres::get_invitation")]
pub async fn get_invitation(&self, token: &str) -> Result<Option<Invitation>>

/// Get invitation info for public preview (org name, role, validity).
#[tracing::instrument(name = "Postgres::get_invitation_preview")]
pub async fn get_invitation_preview(&self, token: &str) -> Result<Option<InvitationPreview>>

/// Accept invitation. Adds membership, records redemption, increments use_count.
#[tracing::instrument(name = "Postgres::accept_invitation")]
pub async fn accept_invitation(
    &self,
    token: &str,
    account_id: AccountId,
) -> Result<InvitationAcceptResult>

/// Revoke invitation.
#[tracing::instrument(name = "Postgres::revoke_invitation")]
pub async fn revoke_invitation(&self, invitation_id: i64) -> Result<()>

/// List invitations for organization.
#[tracing::instrument(name = "Postgres::list_org_invitations")]
pub async fn list_org_invitations(&self, org_id: OrgId) -> Result<Vec<Invitation>>
```

## API Key Operations (Updated)

```rust
/// Create API key with optional org scope.
/// If org_id is None, creates a personal API key.
#[tracing::instrument(name = "Postgres::create_api_key_with_org")]
pub async fn create_api_key_with_org(
    &self,
    account_id: AccountId,
    name: &str,
    org_id: Option<OrgId>,
) -> Result<(i64, RawToken)>

/// List personal API keys for account (org_id IS NULL).
#[tracing::instrument(name = "Postgres::list_personal_api_keys")]
pub async fn list_personal_api_keys(&self, account_id: AccountId) -> Result<Vec<ApiKeyInfo>>

/// List org-scoped API keys for account.
#[tracing::instrument(name = "Postgres::list_org_api_keys")]
pub async fn list_org_api_keys(
    &self,
    account_id: AccountId,
    org_id: OrgId,
) -> Result<Vec<ApiKeyInfo>>

/// Revoke API key by ID. Only revokes if owned by account.
#[tracing::instrument(name = "Postgres::revoke_api_key_by_id")]
pub async fn revoke_api_key_by_id(&self, key_id: i64, account_id: AccountId) -> Result<bool>
```

## Audit Log Operations

```rust
/// Log an audit event.
#[tracing::instrument(name = "Postgres::audit_log")]
pub async fn audit_log(
    &self,
    account_id: Option<AccountId>,
    org_id: Option<OrgId>,
    action: &str,
    details: Option<serde_json::Value>,
) -> Result<()>
```

### Audit Action Constants

```rust
pub mod audit_actions {
    pub const OAUTH_SUCCESS: &str = "oauth.success";
    pub const OAUTH_FAILURE: &str = "oauth.failure";
    pub const ACCOUNT_CREATED: &str = "account.created";
    pub const ACCOUNT_DISABLED: &str = "account.disabled";
    pub const ORGANIZATION_CREATED: &str = "organization.created";
    pub const INVITATION_CREATED: &str = "invitation.created";
    pub const INVITATION_ACCEPTED: &str = "invitation.accepted";
    pub const INVITATION_REVOKED: &str = "invitation.revoked";
    pub const MEMBER_ADDED: &str = "member.added";
    pub const MEMBER_REMOVED: &str = "member.removed";
    pub const MEMBER_ROLE_CHANGED: &str = "member.role_changed";
    pub const API_KEY_CREATED: &str = "api_key.created";
    pub const API_KEY_REVOKED: &str = "api_key.revoked";
    pub const SESSION_CREATED: &str = "session.created";
    pub const SESSION_REVOKED: &str = "session.revoked";
}
```

## Data Types

Add to `db.rs` or a new `db/types.rs`:

```rust
use chrono::{DateTime, Utc};
use crate::auth::{AccountId, OrgId, OrgRole};

#[derive(Debug, Clone)]
pub struct Account {
    pub id: AccountId,
    pub email: String,
    pub name: Option<String>,
    pub disabled_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct GitHubIdentity {
    pub id: i64,
    pub account_id: AccountId,
    pub github_user_id: i64,
    pub github_username: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Organization {
    pub id: OrgId,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct OrgMembership {
    pub organization: Organization,
    pub role: OrgRole,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct OrgMember {
    pub account_id: AccountId,
    pub email: String,
    pub name: Option<String>,
    pub role: OrgRole,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Invitation {
    pub id: i64,
    pub organization_id: OrgId,
    pub token: String,
    pub role: OrgRole,
    pub created_by: AccountId,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub max_uses: Option<i32>,
    pub use_count: i32,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct InvitationPreview {
    pub organization_name: String,
    pub role: OrgRole,
    pub expires_at: DateTime<Utc>,
    pub valid: bool,
}

#[derive(Debug, Clone)]
pub struct OAuthState {
    pub pkce_verifier: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone)]
pub struct ApiKeyInfo {
    pub id: i64,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub enum InvitationAcceptResult {
    Success { org_id: OrgId, org_name: String, role: OrgRole },
    Expired,
    Revoked,
    MaxUsesReached,
    AlreadyMember,
    NotFound,
}
```

## Update validate() Method

Modify existing `validate()` to check both `api_key` and `user_session`:

```rust
/// Validate a token (API key or session) against the database.
/// Returns AuthenticatedToken for API keys, or returns an error for sessions
/// (sessions should use validate_session instead).
#[tracing::instrument(name = "Postgres::validate", skip(token))]
pub async fn validate(&self, token: impl Into<RawToken>) -> Result<Option<AuthenticatedToken>> {
    let token = token.into();

    // Try API key first (existing behavior)
    if let Some((account_id, org_id)) = self.token_lookup(&token).await? {
        return Ok(Some(AuthenticatedToken {
            account_id,
            org_id,
            plaintext: token,
        }));
    }

    // Not found
    Ok(None)
}
```

Note: Session validation is separate via `validate_session()`. The auth extractor will try both.

## Checklist

- [x] Add `Account` struct and related types
- [x] Implement account CRUD operations
- [x] Implement GitHub identity operations
- [x] Implement session operations
- [x] Implement OAuth state operations
- [x] Implement organization operations
- [x] Implement membership operations
- [x] Implement invitation operations
- [x] Implement audit log operation
- [x] Update API key operations for org scope
- [x] Add audit action constants
- [x] Integration tests for account operations (6 tests)
- [x] Integration tests for session operations (8 tests)
- [x] Integration tests for organization operations (4 tests)
- [x] Integration tests for membership operations (8 tests)
- [x] Integration tests for invitation operations (10 tests)
- [x] Integration tests for GitHub identity operations (5 tests)
- [x] Integration tests for OAuth state operations (4 tests)
- [x] Integration tests for API key operations (6 tests)
- [x] Account model migration complete (org_id removed)
