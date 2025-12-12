# RFC 0003: Self-Service Signup

## Overview

This RFC describes self-service signup for Courier, enabling users to onboard without manual intervention. Users authenticate via GitHub OAuth to establish their identity, and Courier provisions their account. Organization membership is managed entirely within Courier through an invitation system.

The goal is to allow a user to go from "heard about Hurry" to "running builds with caching" in a single session.

## Design Principles

### GitHub for identity only

GitHub is used solely for authentication—we get the user's identity and email, nothing more. Courier does not query or sync GitHub organization membership. This keeps the integration simple and avoids tight coupling to GitHub's org model.

### One account, multiple contexts

A user has one Courier account linked to their GitHub identity. That account can:
- Use Hurry in a personal organization
- Be a member of one or more organizations
- Be an admin of one or more organizations

This mirrors how developers actually work—the same person might use Hurry personally, contribute to their company's org, and help maintain an open source project's org.

### Invitation-based org membership

Organizations are created and managed entirely within Courier. Org admins invite members via shareable links. This gives organizations full control over who has access without depending on external systems.

### Minimal friction

The signup flow requires as few steps as possible: click "sign up", authenticate with GitHub, done. Users can immediately use Hurry for personal builds. Joining an organization requires accepting an invitation link.

## GitHub OAuth

We use GitHub as an OAuth provider to authenticate users and obtain their email address.

### Why GitHub App over OAuth App

We use a GitHub App rather than a traditional OAuth App:

| Aspect       | OAuth App      | GitHub App                 |
|--------------|----------------|----------------------------|
| Token expiry | Never expires  | 8 hours (+ refresh token)  |
| Permissions  | Broad scopes   | Fine-grained               |
| Rate limits  | Fixed per user | Scales with installations  |

GitHub Apps are the recommended approach for new integrations. The same OAuth web flow is used for authentication, but we get better security through token expiry.

References:
- [Differences between GitHub Apps and OAuth Apps](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/differences-between-github-apps-and-oauth-apps)
- [Building a "Login with GitHub" button with a GitHub App](https://docs.github.com/en/apps/creating-github-apps/writing-code-for-a-github-app/building-a-login-with-github-button-with-a-github-app)

### Required permissions

We request minimal permissions:

- Account permissions:
  - `email_addresses`: Read-only (to get user's email)

> [!IMPORTANT]
> We request no organization permissions. Courier never reads or modifies anything in GitHub beyond the user's basic profile.

### PKCE

GitHub requires PKCE (Proof Key for Code Exchange) for the OAuth web flow. We use the `S256` code challenge method.

## Authentication Flow

### Signup / Sign-in

The same flow handles both new and returning users:

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
     │               │               │ profile       │
     │               │               ├──────────────>│
     │               │               │<──────────────┤
     │               │               │               │
     │               │               │ Create/update │
     │               │               │ account       │
     │               │               │               │
     │               │<──────────────┤               │
     │<──────────────┤ Redirect with │               │
     │               │ auth_code     │               │
```

### Flow details

1. OAuth initiation: User clicks signup/login, site redirects to `GET /api/v1/oauth/github/start?redirect_uri=...`
2. State storage: Courier generates PKCE challenge and state token, stores them, redirects to GitHub's OAuth authorize URL
3. GitHub callback: User authorizes, GitHub redirects to `GET /api/v1/oauth/github/callback?code=...&state=...`
4. Token exchange: Courier validates state, exchanges code for access token using PKCE verifier
5. Identity fetch: Courier queries GitHub for user profile and email
6. Account provisioning: Courier creates account (if new) or updates existing account
7. Auth code creation: Courier creates a short-lived, single-use auth code (e.g., 60 seconds) and redirects to the site with that code
8. Session creation: The dashboard backend exchanges the auth code for a session token (server-to-server)

### New vs returning users

New users get an account created with their GitHub identity linked. A personal organization is created for personal use (it is a normal organization; it just starts as the only org the user belongs to). By default, it is named after the user. No API key is generated automatically—users create keys via the dashboard or API when needed.

Returning users are matched by GitHub user ID. Their email is updated if changed.

## Database Schema

### Core tables

Account table (modifications to existing):

```sql
-- Add disabled timestamp
ALTER TABLE account ADD COLUMN disabled_at TIMESTAMPTZ;

-- Add name field for display
ALTER TABLE account ADD COLUMN name TEXT;
```

When `disabled_at` is set, the account is disabled and all API requests are rejected. API keys are preserved for potential re-enablement.

GitHub identity linking:

```sql
-- Links a GitHub user to their Courier account (1:1)
CREATE TABLE github_identity (
  id BIGSERIAL PRIMARY KEY,
  account_id BIGINT NOT NULL REFERENCES account(id) UNIQUE,
  github_user_id BIGINT NOT NULL UNIQUE,
  github_username TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

Each GitHub user maps to exactly one Courier account, and each account has at most one GitHub identity (bot accounts have none).

The `account.email` field stores the user's current GitHub primary email and is updated on each OAuth authentication. It is not used for identity matching—`github_identity.github_user_id` is the stable identifier. This means users can change their GitHub email without losing access to their Courier account or organization memberships.

### Organization membership

Role definitions:

```sql
-- Defines valid roles for organization membership
CREATE TABLE organization_role (
  id BIGSERIAL PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  description TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Seed initial roles
INSERT INTO organization_role (name, description) VALUES
  ('member', 'Regular organization member'),
  ('admin', 'Organization administrator with full permissions');
```

Using a table instead of a PostgreSQL enum makes adding new roles simpler—just an INSERT rather than dropping and recreating an enum type. The application represents these as a Rust enum for type safety.

Organization member table:

```sql
-- Tracks which accounts belong to which organizations
CREATE TABLE organization_member (
  organization_id BIGINT NOT NULL REFERENCES organization(id),
  account_id BIGINT NOT NULL REFERENCES account(id),
  role_id BIGINT NOT NULL REFERENCES organization_role(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (organization_id, account_id)
);

CREATE INDEX idx_org_member_account ON organization_member(account_id);
CREATE INDEX idx_org_member_role ON organization_member(role_id);
```

An account can be a member of multiple organizations. The `role_id` determines privileges within that org. The creator of an organization is automatically added with the `admin` role.

### Invitation system

Organization invitations:

```sql
-- Invitations for users to join organizations
CREATE TABLE organization_invitation (
  id BIGSERIAL PRIMARY KEY,
  organization_id BIGINT NOT NULL REFERENCES organization(id),
  token TEXT NOT NULL UNIQUE,
  role_id BIGINT NOT NULL REFERENCES organization_role(id),
  created_by BIGINT NOT NULL REFERENCES account(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  expires_at TIMESTAMPTZ,
  max_uses INT,                          -- NULL = unlimited
  use_count INT NOT NULL DEFAULT 0,
  revoked_at TIMESTAMPTZ
);

CREATE INDEX idx_invitation_org ON organization_invitation(organization_id);
CREATE INDEX idx_invitation_expires ON organization_invitation(expires_at);
```

Invitations are link-based tokens that can be shared via any channel (Slack, email, etc.). Features:
- Optional expiration (`expires_at`)
- Optional use limit (`max_uses`)
- Can be revoked by admin (`revoked_at`)
- Track usage count for auditing

Invitation redemption log:

```sql
-- Tracks who used which invitation
CREATE TABLE invitation_redemption (
  id BIGSERIAL PRIMARY KEY,
  invitation_id BIGINT NOT NULL REFERENCES organization_invitation(id),
  account_id BIGINT NOT NULL REFERENCES account(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE (invitation_id, account_id)
);
```

### OAuth state

OAuth state storage (for PKCE flow):

```sql
-- Temporary storage for OAuth flow state
CREATE TABLE oauth_state (
  id BIGSERIAL PRIMARY KEY,
  state_token TEXT NOT NULL UNIQUE,
  pkce_verifier TEXT NOT NULL,
  redirect_uri TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_oauth_state_expires ON oauth_state(expires_at);
```

OAuth state records expire after 10 minutes. A background job cleans up expired records.

OAuth exchange codes:

After a successful OAuth callback, Courier issues a short-lived, single-use exchange code. This avoids returning session tokens in URLs.

```sql
-- Short-lived, single-use auth codes issued after OAuth callback.
CREATE TABLE oauth_exchange_code (
  id BIGSERIAL PRIMARY KEY,
  -- Store only a hash of the exchange code (like API keys/sessions), so DB
  -- leaks don't allow redeeming live auth codes.
  code_hash BYTEA NOT NULL UNIQUE,
  account_id BIGINT NOT NULL REFERENCES account(id),
  redirect_uri TEXT NOT NULL,
  -- Stored server-side; never trusted from the client.
  new_user BOOLEAN NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  expires_at TIMESTAMPTZ NOT NULL,
  redeemed_at TIMESTAMPTZ
);

CREATE INDEX idx_oauth_exchange_code_expires ON oauth_exchange_code(expires_at);
```

### Sessions

User sessions:

```sql
-- Active user sessions
CREATE TABLE user_session (
  id BIGSERIAL PRIMARY KEY,
  account_id BIGINT NOT NULL REFERENCES account(id),
  session_token TEXT NOT NULL UNIQUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  expires_at TIMESTAMPTZ NOT NULL,
  last_accessed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_session_account ON user_session(account_id);
CREATE INDEX idx_session_expires ON user_session(expires_at);
```

Sessions are used for web UI authentication. They're separate from API keys, which are used for CLI/CI authentication.

Session tokens are intended to be used by the dashboard backend when calling Courier. The browser typically authenticates to the dashboard backend using the dashboard's own session mechanism (often cookies). This keeps Courier's session tokens out of browser URLs and reduces exposure to XSS.

## API Endpoints

### OAuth flow

Start OAuth:
```
GET /api/v1/oauth/github/start
  ?redirect_uri=https://site.example.com/callback

Response: 302 redirect to GitHub
```

The `redirect_uri` must be on an allowlisted domain. Courier validates this before redirecting.

OAuth callback (called by GitHub):
```
GET /api/v1/oauth/github/callback
  ?code=...
  &state=...

Response: 302 redirect to redirect_uri with ?auth_code=...&new_user=true|false
```

The `new_user` parameter indicates whether this was a first-time signup, so the site can show appropriate onboarding.

We avoid returning a session token directly in the redirect URL because URLs are commonly persisted and leaked in ways that are hard to control:
- Browser history and “recently visited” UI
- Application/server access logs (both for Courier and the dashboard)
- Analytics/monitoring tooling that records full URLs
- `Referer` headers on subsequent navigations

Exchange auth code (called by dashboard backend):
```
POST /api/v1/oauth/exchange
Content-Type: application/json

{
  "auth_code": "..."
}

Response:
{
  "session_token": "..."
}
```

Auth codes are high-entropy, short-lived, and single-use. A second attempt to redeem the same auth code fails.

### Session management

Get current user:
```
GET /api/v1/me
Authorization: Bearer <session_token>

Response:
{
  "id": 1,
  "email": "user@example.com",
  "name": "Alice",
  "github_username": "alice",
  "created_at": "2025-01-01T00:00:00Z"
}
```

Organization memberships are fetched separately via `GET /api/v1/me/organizations`.

Sign out:
```
POST /api/v1/oauth/logout
Authorization: Bearer <session_token>

Response: 204 No Content
```

### Organization management

Create organization (any authenticated user):
```
POST /api/v1/organizations
Authorization: Bearer <session_token>
Content-Type: application/json

{
  "name": "Acme Corp"
}

Response:
{
  "organization_id": 1,
  "name": "Acme Corp"
}
```

The creating user is automatically added as an admin.

List user's organizations:
```
GET /api/v1/me/organizations
Authorization: Bearer <session_token>

Response:
{
  "organizations": [
    {
      "organization_id": 1,
      "name": "Acme Corp",
      "role": "admin"
    }
  ]
}
```

### Invitation management

Create invitation (org admin only):
```
POST /api/v1/organizations/{org_id}/invitations
Authorization: Bearer <session_token>
Content-Type: application/json

{
  "role": "member",           // or "admin"
  "expires_at": "2025-01-08T00:00:00Z", // optional, default now + 7 days; null = never
  "max_uses": 10              // optional, default unlimited
}

Response:
{
  "invitation_id": 1,
  "token": "hXkR4pN8",
  "url": "https://hurry.build/invite/hXkR4pN8",
  "expires_at": "2025-01-08T00:00:00Z",
  "max_uses": 10
}
```

If `expires_at` is null, the invitation never expires.

List invitations (org admin only):
```
GET /api/v1/organizations/{org_id}/invitations
Authorization: Bearer <session_token>

Response:
{
  "invitations": [
    {
      "invitation_id": 1,
      "role": "member",
      "created_by": 1,
      "created_at": "2025-01-01T00:00:00Z",
      "expires_at": "2025-01-08T00:00:00Z",
      "max_uses": 10,
      "use_count": 3,
      "revoked_at": null
    }
  ]
}
```

Revoke invitation (org admin only):
```
DELETE /api/v1/organizations/{org_id}/invitations/{invitation_id}
Authorization: Bearer <session_token>

Response: 204 No Content
```

Accept invitation (any authenticated user):
```
POST /api/v1/invitations/{token}/accept
Authorization: Bearer <session_token>

Response:
{
  "organization_id": 1,
  "name": "Acme Corp",
  "role": "member"
}
```

Returns 400 if invitation is expired, revoked, or at max uses. Returns 409 if user is already a member.

Get invitation info (public, for preview):
```
GET /api/v1/invitations/{token}

Response:
{
  "organization_name": "Acme Corp",
  "role": "member",
  "expires_at": "2025-01-08T00:00:00Z", // may be null (never expires)
  "valid": true
}
```

This endpoint is public so the site can show invitation details before the user authenticates.

### Member management

List org members (any org member):
```
GET /api/v1/organizations/{org_id}/members
Authorization: Bearer <session_token>

Response:
{
  "members": [
    {
      "account_id": 1,
      "email": "alice@example.com",
      "name": "Alice",
      "role": "admin",
      "created_at": "2025-01-01T00:00:00Z"
    }
  ]
}
```

Update member role (org admin only):
```
PATCH /api/v1/organizations/{org_id}/members/{account_id}
Authorization: Bearer <session_token>
Content-Type: application/json

{
  "role": "admin"
}

Response: 204 No Content
```

Remove member (org admin only, cannot remove self if last admin):
```
DELETE /api/v1/organizations/{org_id}/members/{account_id}
Authorization: Bearer <session_token>

Response: 204 No Content
```

Leave organization (self):
```
POST /api/v1/organizations/{org_id}/leave
Authorization: Bearer <session_token>

Response: 204 No Content
```

Returns 400 if user is the last admin (must transfer admin first or delete org).

### API key management

API keys are scoped to an organization.

This matches the existing Courier design where an API key identifies both an account and an org context. It also prevents accidentally interacting with the wrong organization.

Each account has a personal organization that is created on signup. "Personal API keys" are simply API keys scoped to that personal organization.

#### Personal API keys (personal org)

List personal API keys:
```
GET /api/v1/me/api-keys
Authorization: Bearer <session_token>

Response:
{
  "api_keys": [
    {
      "id": 1,
      "name": "Personal laptop",
      "created_at": "2025-01-01T00:00:00Z",
      "last_used_at": "2025-01-15T00:00:00Z"
    }
  ]
}
```

Create personal API key:
```
POST /api/v1/me/api-keys
Authorization: Bearer <session_token>
Content-Type: application/json

{
  "name": "Personal laptop"
}

Response:
{
  "id": 2,
  "name": "Personal laptop",
  "api_key": "a1b2c3d4..."  // Only time this is returned (32 hex chars)
}
```

Revoke personal API key:
```
DELETE /api/v1/me/api-keys/{key_id}
Authorization: Bearer <session_token>

Response: 204 No Content
```

#### Organization-scoped API keys

List org API keys (for current user):
```
GET /api/v1/organizations/{org_id}/api-keys
Authorization: Bearer <session_token>

Response:
{
  "api_keys": [
    {
      "id": 3,
      "name": "Work laptop",
      "created_at": "2025-01-01T00:00:00Z",
      "last_used_at": "2025-01-15T00:00:00Z"
    }
  ]
}
```

Create org-scoped API key:
```
POST /api/v1/organizations/{org_id}/api-keys
Authorization: Bearer <session_token>
Content-Type: application/json

{
  "name": "Work laptop"
}

Response:
{
  "id": 3,
  "name": "Work laptop",
  "api_key": "a1b2c3d4..."
}
```

Revoke org-scoped API key:
```
DELETE /api/v1/organizations/{org_id}/api-keys/{key_id}
Authorization: Bearer <session_token>

Response: 204 No Content
```

### Bot accounts

Bot accounts are organization-scoped accounts without GitHub identity, for CI systems and automation.

Create bot account (org admin only):
```
POST /api/v1/organizations/{org_id}/bots
Authorization: Bearer <session_token>
Content-Type: application/json

{
  "name": "CI Bot",
  "responsible_email": "alice@example.com"
}

Response:
{
  "account_id": 5,
  "name": "CI Bot",
  "api_key": "a1b2c3d4..."
}
```

Bot accounts:
- Are not linked to a GitHub identity
- Cannot authenticate via OAuth or have sessions
- Belong to exactly one organization
- The `responsible_email` is for contact purposes, not authentication
- Have their own API keys (managed by org admins)

List bot accounts (org admin only):
```
GET /api/v1/organizations/{org_id}/bots
Authorization: Bearer <session_token>

Response:
{
  "bots": [
    {
      "account_id": 5,
      "name": "CI Bot",
      "responsible_email": "alice@example.com",
      "created_at": "2025-01-01T00:00:00Z"
    }
  ]
}
```

## Authorization Model

### Authentication types

| Type          | Use case | Identifier               |
|---------------|----------|--------------------------|
| Session token | Web UI   | `Bearer <session_token>` |
| API key       | CLI, CI  | `Bearer <api_key>`       |

### Session tokens vs API keys

The key semantic difference between these token types:

- **API keys** encode both an account AND an organization context. When you use an API key, the org is implicit—no need to specify it in the request. This is ideal for CLI/CI where you configure the key once and all operations use that org.

- **Session tokens** identify only the account. Endpoints that operate on org-scoped resources must include the `org_id` in the URL. This is necessary for the web UI where users switch between orgs.

This means:
- `GET /api/v1/cache/cargo/restore` with an API key → uses the org encoded in the key
- `GET /api/v1/organizations/{org_id}/members` with a session token → org specified in URL

### Permission levels

| Action              | member | admin |
|---------------------|--------|-------|
| View org members    | Yes    | Yes   |
| Create invitation   | -      | Yes   |
| Revoke invitation   | -      | Yes   |
| Update member role  | -      | Yes   |
| Remove member       | -      | Yes   |
| Leave org           | Yes    | Yes   |
| Create bot account  | -      | Yes   |
| Manage bot API keys | -      | Yes   |

Account-level actions (not org-scoped):

| Action                      | Self |
|-----------------------------|------|
| View own personal API keys  | Yes  |
| Create own personal API key | Yes  |
| Revoke own personal API key | Yes  |
| View own org API keys       | Yes  |
| Create own org API key      | Yes  |
| Revoke own org API key      | Yes  |

### Personal usage (personal org)

Users who aren't members of any non-personal organization can still:
- Authenticate and maintain a session
- Create and manage personal API keys
- Use Hurry for personal builds (cache stored under their personal org)

Personal API keys are scoped to the user's personal org and can only be used to access that org's data.

## Security Considerations

### OAuth state

OAuth state tokens are generated using a CSPRNG, stored server-side, and expire after 10 minutes. The state parameter prevents CSRF attacks during the OAuth flow.

### PKCE

All OAuth flows use PKCE with the S256 challenge method. The verifier is stored server-side and never exposed to the client.

### OAuth exchange codes

OAuth exchange codes (`auth_code`) are generated using a CSPRNG (192 bits) and encoded as base64url without padding. They expire quickly (60 seconds) and can be redeemed only once. They exist to avoid returning long-lived credentials (session tokens) in URLs.

Implementation notes:
- Only a SHA-256 hash of the auth code is stored server-side (`code_hash`). The client sends the plaintext auth code for redemption.
- Redemption is atomic: validate not expired, validate not redeemed, then set `redeemed_at` and issue a session token.
- The `new_user` signal is stored server-side with the auth code and never trusted from the client.

This reduces the blast radius of accidental disclosure: even if an `auth_code` is captured via logs or browser history, it will typically be expired or already redeemed by the time an attacker can use it.

### Redirect URI validation

The `redirect_uri` parameter is validated against an allowlist of permitted redirect URIs before initiating OAuth.

Validation rules:
- The `redirect_uri` must exactly match one of the configured allowlist entries.
- This list is configured per environment (e.g., strict production callback URL(s), and broader localhost URLs in development).

Exact matching prevents attackers from supplying arbitrary paths on an allowlisted origin (which could otherwise include open-redirect endpoints or paths that leak query parameters).

### Logging and redaction

Courier must not log secrets or credentials, including:
- OAuth callback query parameters (`code`, `state`)
- OAuth exchange codes (`auth_code`)
- Session tokens
- API keys
- `Authorization` headers

### API key generation

API keys use the existing Courier format: 16 random bytes, hex-encoded, resulting in a 32-character string with 128 bits of entropy. The key is returned exactly once at creation time, then only a SHA-256 hash is stored. With 128 bits of entropy, brute-forcing is computationally infeasible.

This maintains compatibility with the existing `db::Postgres::create_token` implementation.

### Session tokens

Session tokens are generated using a CSPRNG with 256 bits of entropy.

Session expiration:
- Tokens expire 24 hours after creation
- Each successful authentication extends expiration to 24 hours from that moment (sliding window)
- Active users who use the dashboard at least once per day stay logged in
- Inactive users must re-authenticate via OAuth after 24 hours

Session invalidation:
- Sessions are deleted when the associated account is disabled (`account.disabled_at` set)
- Sessions can be explicitly revoked via the logout endpoint

### Invitation tokens

Invitation tokens are short, human-friendly codes designed to be easily shared (e.g., in Slack channel headers). Token length varies based on expiration to balance usability with security:

| Expiration        | Length   | Entropy  | Example        |
|-------------------|----------|----------|----------------|
| ≤30 days          | 8 chars  | ~47 bits | `hXkR4pN8`     |
| >30 days or never | 12 chars | ~71 bits | `hXkR4pNq2mYz` |

Short-lived tokens can be shorter because brute-force attacks are time-limited. Never-expiring tokens need more entropy since attackers have unlimited time.

Tokens can also be:
- Use-limited (max redemptions)
- Revoked by admins

### Rate limiting

Rate limiting protects against brute-force attacks and abuse. Authenticated endpoints are rate limited by account/session. Unauthenticated endpoints (OAuth start, auth code exchange, invitation preview) rely on strong randomness, short TTLs, and service-protection limits.

Key endpoints with rate limits:
- `/api/v1/invitations/{token}/accept`: 10/minute per session (protects against brute-forcing invitation tokens)
- `/api/v1/me/api-keys` (POST): 10/minute per session (prevents API key spam)
- `/api/v1/oauth/exchange` (POST): rate limited without IP (auth codes are high-entropy and single-use):
  - Bucketed by the first 12 characters of `auth_code` (protects against rapid replay/hammering on the same code)
  - Optionally capped by a high-ceiling global limiter (protects DB/CPU from volumetric abuse)

The OAuth flow is naturally rate-limited by GitHub's own rate limits on the token exchange endpoint.

### Audit logging

Authorization-related actions are recorded in an audit log table:

```sql
CREATE TABLE audit_log (
  id BIGSERIAL PRIMARY KEY,
  account_id BIGINT REFERENCES account(id),      -- NULL for failed auth attempts
  organization_id BIGINT REFERENCES organization(id),  -- NULL for account-level events
  action TEXT NOT NULL,
  details JSONB,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_audit_log_account ON audit_log(account_id);
CREATE INDEX idx_audit_log_org ON audit_log(organization_id);
CREATE INDEX idx_audit_log_created ON audit_log(created_at);
```

Logged actions include:
- `oauth.success`, `oauth.failure`: OAuth authentication attempts
- `account.created`: New account creation
- `organization.created`: Organization creation
- `invitation.created`, `invitation.accepted`, `invitation.revoked`: Invitation lifecycle
- `member.added`, `member.removed`, `member.role_changed`: Membership changes
- `api_key.created`, `api_key.revoked`: API key management
- `session.created`, `session.revoked`: Session lifecycle

The `details` field contains event-specific context (e.g., old/new role for role changes, invitation ID for accepts).

## Out of Scope

The following are explicitly out of scope for this RFC:

- Web interface: The signup site and management dashboard are separate projects
- Email delivery: Invitations are link-based; email/Slack delivery is future work
- SSO/SAML: Only GitHub OAuth is supported initially
- Multiple identity providers: GitHub only for now
- Organization billing: No payment integration
- GitHub org sync: Organizations are managed entirely within Courier

## Migration

For existing deployments with manually-created orgs and accounts:

1. Existing accounts without `github_identity` continue to work with API keys
2. Existing orgs without any `organization_member` records are treated as legacy
3. Legacy accounts can be linked to GitHub identities by signing in via OAuth
4. A migration tool can bulk-link accounts by email address if desired

## Future Work

- Email/Slack notifications: Send invitation links directly instead of requiring manual sharing
- GitHub org sync: Optional feature to sync membership from GitHub orgs
- Multiple identity providers: GitLab, Bitbucket, Google
- Organization settings: Configure invitation defaults, require approval, etc.
- Team-based permissions: Sub-org groupings with different access levels
