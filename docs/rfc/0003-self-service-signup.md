# RFC 0003: Self-Service Signup

## Overview

This RFC describes self-service signup for Courier, enabling users and organizations to onboard without manual intervention. Users authenticate via a GitHub App using OAuth web flow, and Courier provisions accounts and organizations based on their GitHub identity and org membership.

The goal is to allow a user to go from "heard about Hurry" to "running builds with caching" in a single session, with organization membership automatically managed based on their GitHub org.

## Design Principles

### GitHub as the source of truth

GitHub organizations and their membership are the authoritative source for Courier's organization structure. When a user authenticates, we query GitHub for their org memberships and sync that state to Courier. When membership changes in GitHub, Courier reflects those changes.

### Minimal friction

The signup flow should require as few steps as possible. A user clicks "sign up", authenticates with GitHub, selects which org context they want, and they're done. No email verification, no separate password, no waiting for approval (unless the org has access restrictions).

Users can always set up their own personal GitHub account as well, they just obviously won't be able to share their cache with teammates. Personal accounts in GitHub are modeled as orgs inside Courier, where the user is the admin of that org. This allows Courier to not have to worry about the difference and supports personal users adding e.g. bot tokens to their accounts.

### Security by default

API keys are hashed at rest and can only be read once (at creation time). GitHub user access tokens are stored encrypted and expire after 8 hours (with refresh tokens for renewal). Account access can be revoked instantly when org membership changes.

## Why GitHub App over OAuth App

We use a GitHub App with user access tokens rather than a traditional OAuth App:

| Aspect | OAuth App | GitHub App |
|--------|-----------|------------|
| Permissions | Broad scopes (`read:org`) | Fine-grained (`organization:members:read`) |
| Token expiry | Never expires | 8 hours (+ refresh token) |
| Webhooks | Manual per-org setup | Automatic if app installed on org |
| Identity | Acts as authorizing user | Can act as app or user |
| Rate limits | Fixed per user | Scales with installations |

GitHub Apps are the recommended approach for new integrations. The same OAuth web flow is used for authentication, but we get better security (token expiry) and more granular permissions.

References:
- [Differences between GitHub Apps and OAuth Apps](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/differences-between-github-apps-and-oauth-apps)
- [About authentication with a GitHub App](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/about-authentication-with-a-github-app)
- [Building a "Login with GitHub" button with a GitHub App](https://docs.github.com/en/apps/creating-github-apps/writing-code-for-a-github-app/building-a-login-with-github-button-with-a-github-app)

## GitHub App Behavior

During design, several behaviors were discovered that impact the architecture:

### No organization selection during OAuth

GitHub's OAuth flow does **not** prompt users to select which organization they're authenticating with. The flow authenticates the *user*, not an org membership. After OAuth completes, we receive a token scoped to the user, then must query GitHub's API to enumerate their org memberships.

Impact: Courier must implement its own organization selection UI after OAuth callback. The flow becomes: OAuth -> callback -> show org picker -> complete registration.

### Webhooks require per-org configuration

GitHub organization webhooks for membership events (`member_added`, `member_removed`) require configuration at the organization level by an org owner. We cannot register these webhooks automatically just because a user from that org signed up.

Impact: Webhook-based membership sync is opt-in and requires org owner action. Polling is the reliable baseline for all orgs.

### No webhook for role changes

GitHub provides webhooks for member added/removed, but **not** for role changes (member to admin, or vice versa). The `member_updated` event doesn't exist.

Impact: Admin status must always be determined via polling, even for orgs with webhooks configured.

### Personal accounts are not organizations

GitHub users who aren't members of any organization still have a personal account, but this is not an "organization" in GitHub's API. A user's personal repositories are owned by their username, not an org.

Impact: "Personal orgs" in Courier won't map to a GitHub org ID. We'll use a synthetic identifier based on the user's GitHub user ID.

### Required permissions

GitHub App permissions are fine-grained. We request:

- Organization permissions:
  - `members`: Read-only (to read org membership and roles)
- Account permissions:
  - `email_addresses`: Read-only (to get user's email)

> [!IMPORTANT]
> We request only read permissions. Courier never modifies anything in GitHub.

### PKCE required

GitHub requires PKCE (Proof Key for Code Exchange) for the OAuth web flow. We use the `S256` code challenge method.

## Authentication Flow

### Initial signup

```
┌─────────┐     ┌─────────┐     ┌─────────┐     ┌─────────┐
│  User   │     │  Site   │     │ Courier │     │ GitHub  │
└────┬────┘     └────┬────┘     └────┬────┘     └────┬────┘
     │               │               │               │
     │ Click signup  │               │               │
     ├──────────────>│               │               │
     │               │ Redirect to   │               │
     │               │ /oauth/start  │               │
     │               ├──────────────>│               │
     │               │               │ Redirect to   │
     │               │               │ GitHub OAuth  │
     │               │               ├──────────────>│
     │               │               │               │
     │<──────────────┼───────────────┼───────────────┤
     │               │ (User authorizes app)         │
     │               │               │               │
     ├───────────────┼───────────────┼──────────────>│
     │               │               │               │
     │               │               │<──────────────┤
     │               │               │ Code callback │
     │               │               │               │
     │               │               │ Exchange code │
     │               │               │ for token     │
     │               │               ├──────────────>│
     │               │               │<──────────────┤
     │               │               │               │
     │               │               │ Fetch user    │
     │               │               │ + orgs        │
     │               │               ├──────────────>│
     │               │               │<──────────────┤
     │               │               │               │
     │               │<──────────────┤               │
     │               │ Redirect with │               │
     │               │ pending token │               │
     │<──────────────┤               │               │
     │               │               │               │
     │ Select org    │               │               │
     ├──────────────>│               │               │
     │               │ Complete      │               │
     │               │ registration  │               │
     │               ├──────────────>│               │
     │               │<──────────────┤               │
     │<──────────────┤ API key       │               │
     │               │               │               │
```

### Flow details

1. OAuth initiation: User clicks signup, site redirects to `GET /api/v1/oauth/github/start?redirect_uri=...`
2. GitHub redirect: Courier generates PKCE challenge, stores state, redirects to GitHub's OAuth authorize URL
3. GitHub callback: User authorizes, GitHub redirects to `GET /api/v1/oauth/github/callback?code=...&state=...`
4. Token exchange: Courier validates state, exchanges code for user access token + refresh token using PKCE verifier
5. Identity fetch: Courier queries GitHub for user profile and org memberships with roles
6. Pending session: Courier creates a pending OAuth session and redirects back to the site with a session token
7. Org selection: Site displays org picker, user selects which org context to use (or "personal")
8. Registration complete: Site calls `POST /api/v1/oauth/github/complete` with session token and selected org
9. Account provisioning: Courier creates/updates organization and account, returns initial API key

User access tokens expire after 8 hours. Courier stores the refresh token (valid for 6 months) and automatically refreshes access tokens as needed for membership sync operations.

### Returning users

When an existing user signs in via OAuth:

1. Match GitHub user ID to existing accounts
2. Update org membership and admin status from fresh GitHub data
3. If the user selects an org they already have an account in, sign them in
4. If the user selects a new org, create a new account in that org
5. No new API key is generated automatically (user can create one via API)

## Database Schema

### Schema changes to existing tables

Remove email uniqueness on `account`:

```sql
-- Remove UNIQUE constraint from email
ALTER TABLE account DROP CONSTRAINT account_email_key;
```

Email is no longer unique at any level. A single email can appear multiple times even within the same org—for example, a user might have both a personal account and a bot account with the same responsible email.

Add disabled timestamp to `account`:

```sql
ALTER TABLE account ADD COLUMN disabled_at TIMESTAMPTZ;
```

When set, the account is disabled and all API requests are rejected. API keys are not automatically revoked (preserved for re-enablement).

### New tables

GitHub identity linking:

```sql
-- Links GitHub users to Courier accounts
CREATE TABLE github_identity (
  id BIGSERIAL PRIMARY KEY,
  account_id BIGINT NOT NULL REFERENCES account(id),
  github_user_id BIGINT NOT NULL,
  github_username TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE (account_id),
  UNIQUE (github_user_id, account_id)
);

CREATE INDEX idx_github_identity_user ON github_identity(github_user_id);
```

> [!NOTE]
> A GitHub user can have multiple Courier accounts (one per org), so `github_user_id` alone is not unique. The unique constraint is on `(github_user_id, account_id)` to prevent duplicate links.

GitHub user access tokens:

```sql
-- Stores GitHub user access tokens for API access
CREATE TABLE github_user_token (
  id BIGSERIAL PRIMARY KEY,
  github_user_id BIGINT NOT NULL UNIQUE,
  access_token_encrypted BYTEA NOT NULL,
  refresh_token_encrypted BYTEA NOT NULL,
  access_token_expires_at TIMESTAMPTZ NOT NULL,
  refresh_token_expires_at TIMESTAMPTZ NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

Tokens are stored per GitHub user (not per account) since the same token works across all orgs. Both tokens are encrypted at rest using AES-256-GCM with a server-managed key.

Access tokens expire after 8 hours; refresh tokens expire after 6 months. When performing membership sync, Courier checks `access_token_expires_at` and refreshes if needed. If the refresh token is also expired, the user must re-authenticate via OAuth.

Pending OAuth sessions:

```sql
-- Temporary storage for OAuth flow state
CREATE TABLE oauth_pending_session (
  id BIGSERIAL PRIMARY KEY,
  session_token TEXT NOT NULL UNIQUE,
  github_user_id BIGINT NOT NULL,
  github_username TEXT NOT NULL,
  email TEXT NOT NULL,
  available_orgs JSONB NOT NULL,  -- [{id, name, role}, ...]
  pkce_verifier TEXT NOT NULL,
  redirect_uri TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_oauth_pending_expires ON oauth_pending_session(expires_at);
```

Pending sessions expire after 10 minutes. A background job cleans up expired sessions.

Organization GitHub linking:

```sql
-- Links GitHub orgs to Courier organizations
CREATE TABLE github_organization (
  id BIGSERIAL PRIMARY KEY,
  organization_id BIGINT NOT NULL REFERENCES organization(id) UNIQUE,
  github_org_id BIGINT,  -- NULL for personal orgs
  github_org_name TEXT,  -- NULL for personal orgs
  webhook_secret TEXT,   -- NULL if webhooks not configured
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE (github_org_id)
);
```

For personal orgs, `github_org_id` and `github_org_name` are NULL. The `organization.name` will be set to the user's GitHub username with a "(personal)" suffix.

Organization administrators:

```sql
-- Tracks which accounts are org admins
CREATE TABLE organization_admin (
  organization_id BIGINT NOT NULL REFERENCES organization(id),
  account_id BIGINT NOT NULL REFERENCES account(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (organization_id, account_id)
);
```

### Membership sync state

```sql
-- Tracks when we last synced membership for each org
CREATE TABLE github_sync_state (
  id BIGSERIAL PRIMARY KEY,
  organization_id BIGINT NOT NULL REFERENCES organization(id) UNIQUE,
  last_sync_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  next_sync_at TIMESTAMPTZ NOT NULL,
  sync_failures INT NOT NULL DEFAULT 0
);
```

## Membership Synchronization

### Sync triggers

Membership is synchronized:

1. On OAuth: Every time a user completes OAuth, their membership and role are refreshed
2. On webhook (if configured): `organization.member_added`, `organization.member_removed` events
3. On poll: Background job polls orgs periodically

### Polling strategy

```
Base interval: 1 hour
Backoff on failure: 2^failures hours (max 24 hours)
Reset on success: back to 1 hour
```

For each org during sync:

1. Fetch all members via `GET /orgs/{org}/members?role=all`
2. Fetch membership details for each to determine admin status
3. Compare to current Courier state
4. Disable accounts for removed members
5. Update admin status for changed roles
6. Log changes for audit

> [!TIP]
> We don't automatically re-enable accounts that were disabled. If a user is re-added to the org, they must re-authenticate via OAuth to re-enable their account. This prevents stale accounts from becoming active without user action.

### Handling removed members

When sync detects a user is no longer in the GitHub org:

1. Set `account.disabled_at` to now
2. Do not revoke API keys (they're already unusable with a disabled account)
3. Log the event for audit

When a disabled user re-authenticates:

1. Verify they're back in the GitHub org
2. Clear `account.disabled_at`
3. Existing API keys become functional again

## API Endpoints

### OAuth flow

Start OAuth:
```
GET /api/v1/oauth/github/start
  ?redirect_uri=https://site.example.com/callback

Response: 302 redirect to GitHub
```

OAuth callback (called by GitHub):
```
GET /api/v1/oauth/github/callback
  ?code=...
  &state=...

Response: 302 redirect to redirect_uri with ?session=...
```

Complete registration:
```
POST /api/v1/oauth/github/complete
Content-Type: application/json

{
  "session_token": "...",
  "organization": {
    "type": "github_org",
    "github_org_id": 12345
  }
  // OR
  "organization": {
    "type": "personal"
  }
}

Response:
{
  "account_id": 1,
  "organization_id": 1,
  "api_key": "hur_..."  // Only returned on first registration
}
```

### User and org management

List org members (any org member):
```
GET /api/v1/organizations/{org_id}/members
Authorization: Bearer <api_key>

Response:
{
  "members": [
    {
      "account_id": 1,
      "email": "user@example.com",
      "is_admin": true,
      "disabled_at": null,
      "created_at": "2025-01-01T00:00:00Z"
    }
  ]
}
```

Disable account (org admin only):
```
POST /api/v1/organizations/{org_id}/members/{account_id}/disable
Authorization: Bearer <api_key>

Response: 204 No Content
```

Enable account (org admin only):
```
POST /api/v1/organizations/{org_id}/members/{account_id}/enable
Authorization: Bearer <api_key>

Response: 204 No Content
```

### API key management

List API keys (self or org admin):
```
GET /api/v1/accounts/{account_id}/api-keys
Authorization: Bearer <api_key>

Response:
{
  "api_keys": [
    {
      "id": 1,
      "name": "CI Bot",
      "created_at": "2025-01-01T00:00:00Z",
      "accessed_at": "2025-01-15T00:00:00Z",
      "revoked_at": null
    }
  ]
}
```

> [!NOTE]
> The actual key content is never returned. It's hashed at rest and can only be read once at creation time.

Create API key (self or org admin):
```
POST /api/v1/accounts/{account_id}/api-keys
Authorization: Bearer <api_key>
Content-Type: application/json

{
  "name": "CI Bot"
}

Response:
{
  "id": 2,
  "name": "CI Bot",
  "api_key": "hur_..."  // Only time this is returned
}
```

Revoke API key (self or org admin):
```
DELETE /api/v1/accounts/{account_id}/api-keys/{key_id}
Authorization: Bearer <api_key>

Response: 204 No Content
```

### Bot accounts

Create bot account (org admin only):
```
POST /api/v1/organizations/{org_id}/accounts
Authorization: Bearer <api_key>
Content-Type: application/json

{
  "email": "responsible-human@example.com",
  "name": "CI Bot"
}

Response:
{
  "account_id": 5,
  "api_key": "hur_..."
}
```

Bot accounts:
- Are not linked to a GitHub identity
- Cannot authenticate via OAuth
- Have the same capabilities as regular accounts
- The email is for the responsible human, not the bot itself
- Multiple bot accounts can share the same responsible email

## Authorization Model

### Permission levels

| Action | Any member | Self | Org admin |
|--------|------------|------|-----------|
| List org members | Yes | - | - |
| View own API keys | - | Yes | - |
| View other's API keys | - | - | Yes |
| Create own API key | - | Yes | - |
| Create other's API key | - | - | Yes |
| Revoke own API key | - | Yes | - |
| Revoke other's API key | - | - | Yes |
| Disable account | - | - | Yes |
| Enable account | - | - | Yes |
| Create bot account | - | - | Yes |

### Admin determination

A user is an org admin in Courier if:

1. Their GitHub role in that org is `admin` (owner), OR
2. They're the creator of a personal org

Admin status is refreshed:
- Every OAuth authentication
- Every membership sync (poll or webhook)

## Webhook Integration (Optional)

Organizations can optionally configure webhooks for real-time membership updates. There are two paths:

### Path A: Manual webhook configuration

For orgs that don't want to install the GitHub App:

1. Org admin visits Courier's org settings
2. Courier displays webhook URL and secret
3. Admin configures webhook in GitHub org settings
4. Courier validates webhook with ping event

### Path B: GitHub App installation (future)

If an org installs the Hurry GitHub App on their org (not just users authenticating), webhooks are configured automatically. This is noted in Future Work.

### Webhook endpoint

```
POST /api/v1/webhooks/github/organization
X-Hub-Signature-256: sha256=...
Content-Type: application/json

{
  "action": "member_removed",
  "membership": {
    "user": { "id": 123, "login": "octocat" }
  },
  "organization": { "id": 456, "login": "acme" }
}
```

### Supported events

- `organization.member_added`: Enable account if previously disabled, or note for next OAuth
- `organization.member_removed`: Disable account immediately
- `organization.member_invited`: No action (wait for accepted)

> [!IMPORTANT]
> Webhooks do not trigger for role changes. Admin status is only updated via polling or OAuth.

## Security Considerations

### Token storage

GitHub user access tokens and refresh tokens are encrypted at rest using AES-256-GCM with a server-managed key. The key is loaded from environment configuration and never stored in the database. Access tokens automatically expire after 8 hours, limiting exposure if encrypted tokens are compromised.

### API key generation

API keys are generated using a CSPRNG with the format `hur_{base62(32 bytes)}`. The key is returned exactly once at creation time, then only a SHA-256 hash is stored. Since keys are system-generated with 256 bits of entropy, slow password hashing (like Argon2) is unnecessary—brute-forcing is already computationally infeasible.

### PKCE

All OAuth flows use PKCE with the S256 challenge method. The verifier is stored in the pending session and validated during token exchange.

### Rate limiting

OAuth endpoints are rate limited per IP:
- `/oauth/github/start`: 10/minute
- `/oauth/github/callback`: 20/minute
- `/oauth/github/complete`: 10/minute

### Audit logging

All authorization-related actions are logged:
- OAuth authentication (success/failure)
- Account enable/disable
- API key create/revoke
- Admin status changes
- Membership sync results

## Out of Scope

The following are explicitly out of scope for this RFC:

- Web interface: The signup site and management dashboard are separate
- Email notifications: No emails are sent by Courier
- SSO/SAML: Only GitHub App authentication is supported
- Multiple identity providers: GitHub only for now
- Organization billing: No payment integration
- Invitations: Users self-service via GitHub org membership

## Migration

For existing deployments with manually-created orgs and accounts:

1. Existing accounts without `github_identity` continue to work
2. Existing orgs without `github_organization` are treated as "legacy" orgs
3. Legacy orgs cannot use OAuth signup (accounts must be created manually)
4. A migration tool can link existing accounts to GitHub identities if desired

## Future Work

- Team-based access: Scope access based on GitHub team membership, not just org
- GitHub App installation: Allow orgs to install the app for automatic webhooks (vs manual webhook setup)
- Multiple identity providers: GitLab, Bitbucket
- Organization federation: Link multiple GitHub orgs to one Courier org
