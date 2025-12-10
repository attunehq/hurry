# Self-Service Signup Implementation

RFC: `docs/rfc/0003-self-service-signup.md`
Branch: `jssblck/self-service-signup`

## Status: COMPLETE - Phase 3 (Database Layer)

## Overview

Implementing self-service signup for Courier via GitHub OAuth. Users authenticate with GitHub to establish identity, and Courier provisions their account. Organization membership managed via invitation system.

## Architecture: Pragmatic Approach

We're using a **horizontal layer** approach - building complete layers (schema → types → DB → endpoints) rather than vertical slices. Each layer is independently testable and deployable.

**Key principles:**
- Full RFC implementation, but in manageable PRs
- Database schema complete upfront (migration 0008)
- Account model migration separate (migration 0009) - runs after code is updated
- No over-engineering: direct db calls, simple helpers, no trait hierarchies
- Integration tests after each phase

## Key Decisions (from user clarification)

1. **GitHub App Config**: Environment variables via `courier serve` args (`GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET`, redirect allowlist as comma-separated URLs)
2. **Account Model**: Remove `account.organization_id`, use only `organization_member` join table
3. **API Key Scope**: Add nullable `organization_id` to `api_key` table (NULL = personal)
4. **Session Tokens**: Separate `user_session` table (not in `api_key`)
5. **Background Jobs**: Lazy cleanup on request (no background task)
6. **Rate Limiting**: Implement now with `tower-governor`
7. **Audit Logging**: Full implementation now
8. **Migrations**: Additive migrations preserving existing data
9. **Invitation Tokens**: Alphanumeric (a-zA-Z0-9)

## PR Strategy

### PR 1: Database Schema (SAFE, MERGE FIRST)
- Migration 0008 (all new tables, additive only)
- Update schema.sql
- `make sqlx-prepare`

### PR 2: Core Types & OAuth Client
- Auth types (SessionToken, OrgRole, SessionContext)
- oauth.rs (GitHubClient)
- crypto.rs helpers (token generation, PKCE)
- Unit tests

### PR 3: Database Operations Layer
- All new db.rs methods (~30 methods)
- Integration tests for each operation

### PR 4: OAuth Endpoints
- oauth.rs endpoints (start, callback, logout)
- SessionContext extractor
- Update DI container
- Integration tests with mocked GitHub

### PR 5: User & Organization Management
- me.rs endpoints (current user, list orgs)
- organizations.rs endpoints (create, members, leave)
- Authorization helpers
- Integration tests

### PR 6: Invitation System
- invitations.rs endpoints (create, list, revoke, get, accept)
- Integration tests

### PR 7: API Key Management
- api_keys.rs endpoints (personal + org scoped)
- Update AuthenticatedToken for Option<OrgId>
- Integration tests

### PR 8: Rate Limiting & Audit Logging
- rate_limit.rs
- Apply to sensitive endpoints
- Audit logging in all handlers
- Integration tests

### PR 9: Account Model Migration (BREAKING)
- Migration 0009 (data migration + DROP COLUMN)
- Update tests

### PR 10: Bot Accounts (Optional)
- bots.rs endpoints
- Integration tests

## Implementation Checklist

### Phase 1: Database Schema & Migrations ✓
- [x] Plan migration strategy
- [x] Create migration 0008: Add self-service signup tables
  - [x] `github_identity` table
  - [x] `organization_role` table (with seed data)
  - [x] `organization_member` table
  - [x] `organization_invitation` table
  - [x] `invitation_redemption` table
  - [x] `oauth_state` table
  - [x] `user_session` table
  - [x] `audit_log` table
  - [x] Modify `account`: add `disabled_at`, `name` columns
  - [x] Modify `api_key`: add nullable `organization_id` column
- [x] Create migration 0008 down file
- [x] Update `schema/schema.sql` to reflect final state
- [x] Run `make sqlx-prepare`
- [x] Verify migrations run cleanly

### Phase 2: Core Types & Auth Infrastructure ✓
- [x] Add `SessionToken` type to auth.rs (redacted debug like RawToken)
- [x] Add `OrgRole` enum to auth.rs (Member, Admin)
- [x] Add `SessionContext` struct to auth.rs
- [x] Add GitHub OAuth config to `ServeConfig` in main.rs
- [x] Create `oauth.rs` module with `GitHub` struct
- [x] Add PKCE helpers to crypto.rs (`generate_pkce`)
- [x] Add `generate_session_token` to crypto.rs
- [x] Add `generate_invitation_token` to crypto.rs
- [x] Add `generate_oauth_state` to crypto.rs
- [x] Add dependencies: `base64`, `oauth2`, `reqwest[json]`
- [x] Unit tests for crypto functions

### Phase 3: Database Layer ✓
- [x] Account operations (create, get, get_by_github_id, update_email, disable)
- [x] GitHub identity operations (link, get, update_username)
- [x] Session operations (create, validate, revoke, revoke_all, extend, cleanup)
- [x] OAuth state operations (store, consume, cleanup)
- [x] Organization operations (create, get, list_for_account)
- [x] Membership operations (add, remove, update_role, get_role, list, is_last_admin)
- [x] Invitation operations (create, get, accept, revoke, list)
- [x] Audit log operation (log_event)
- [x] API key operations (create with org_id, list personal, list org)
- [x] `validate_session()` for session token validation (separate from api_key validate())
- [x] Integration tests for all DB methods (55 new tests)

### Phase 4: OAuth Flow Endpoints
- [ ] Create `api/v1/oauth.rs` module
- [ ] `GET /api/v1/oauth/github/start` handler
- [ ] `GET /api/v1/oauth/github/callback` handler
- [ ] `POST /api/v1/oauth/logout` handler
- [ ] Implement `SessionContext` extractor (FromRequestParts)
- [ ] Add `Option<GitHubClient>` to api::State
- [ ] Wire oauth router in v1.rs
- [ ] Integration tests with mocked GitHub API

### Phase 5: Session & User Endpoints
- [ ] Create `api/v1/me.rs` module
- [ ] `GET /api/v1/me` handler
- [ ] `GET /api/v1/me/organizations` handler
- [ ] Wire me router in v1.rs
- [ ] Integration tests

### Phase 6: Organization Management Endpoints
- [ ] Create `api/v1/organizations.rs` module
- [ ] `POST /api/v1/organizations` handler (create org)
- [ ] `GET /api/v1/organizations/{org_id}/members` handler
- [ ] `PATCH /api/v1/organizations/{org_id}/members/{account_id}` handler
- [ ] `DELETE /api/v1/organizations/{org_id}/members/{account_id}` handler
- [ ] `POST /api/v1/organizations/{org_id}/leave` handler
- [ ] Authorization helper: `require_org_admin()`
- [ ] Authorization helper: `require_org_member()`
- [ ] Wire organizations router in v1.rs
- [ ] Integration tests

### Phase 7: Invitation Endpoints
- [ ] Create `api/v1/invitations.rs` module
- [ ] `POST /api/v1/organizations/{org_id}/invitations` handler
- [ ] `GET /api/v1/organizations/{org_id}/invitations` handler
- [ ] `DELETE /api/v1/organizations/{org_id}/invitations/{id}` handler
- [ ] `GET /api/v1/invitations/{token}` handler (public)
- [ ] `POST /api/v1/invitations/{token}/accept` handler
- [ ] Wire invitations router in v1.rs
- [ ] Integration tests

### Phase 8: API Key Management Endpoints
- [ ] Update `AuthenticatedToken` to have `org_id: Option<OrgId>`
- [ ] Add `require_org()` method to AuthenticatedToken
- [ ] `GET /api/v1/me/api-keys` handler
- [ ] `POST /api/v1/me/api-keys` handler
- [ ] `DELETE /api/v1/me/api-keys/{key_id}` handler
- [ ] `GET /api/v1/organizations/{org_id}/api-keys` handler
- [ ] `POST /api/v1/organizations/{org_id}/api-keys` handler
- [ ] `DELETE /api/v1/organizations/{org_id}/api-keys/{key_id}` handler
- [ ] Update existing cache/CAS handlers to use `require_org()`
- [ ] Integration tests

### Phase 9: Rate Limiting & Audit Logging
- [ ] Create `rate_limit.rs` module
- [ ] Configure rate limiter (10 req/min per account)
- [ ] Apply to `/invitations/{token}/accept`
- [ ] Apply to `/me/api-keys` POST
- [ ] Add audit logging calls to all state-changing handlers
- [ ] Integration tests for rate limits

### Phase 10: Account Model Migration
- [ ] Create migration 0009 up (populate organization_member, api_key.org_id, drop column)
- [ ] Create migration 0009 down (re-add column, repopulate)
- [ ] Update test fixtures for new model
- [ ] Verify all tests pass

### Phase 11: Bot Account Endpoints (Optional)
- [ ] `POST /api/v1/organizations/{org_id}/bots` handler
- [ ] `GET /api/v1/organizations/{org_id}/bots` handler
- [ ] Integration tests

## Files to Create

### New Files
- [x] `packages/courier/src/oauth.rs` - GitHub OAuth client ✓
- [ ] `packages/courier/src/rate_limit.rs` - Rate limiting config
- [ ] `packages/courier/src/api/v1/oauth.rs` - OAuth endpoints
- [ ] `packages/courier/src/api/v1/me.rs` - User endpoints
- [ ] `packages/courier/src/api/v1/organizations.rs` - Org endpoints
- [ ] `packages/courier/src/api/v1/invitations.rs` - Invitation endpoints
- [x] `packages/courier/schema/migrations/0008_add_self_service_signup.up.sql` ✓
- [x] `packages/courier/schema/migrations/0008_add_self_service_signup.down.sql` ✓
- [ ] `packages/courier/schema/migrations/0009_remove_account_org_id.up.sql`
- [ ] `packages/courier/schema/migrations/0009_remove_account_org_id.down.sql`
- [ ] `packages/courier/tests/it/api/v1/oauth.rs` - OAuth tests
- [ ] `packages/courier/tests/it/api/v1/me.rs` - Me endpoint tests
- [ ] `packages/courier/tests/it/api/v1/organizations.rs` - Org tests
- [ ] `packages/courier/tests/it/api/v1/invitations.rs` - Invitation tests
- [x] `packages/courier/tests/it/crypto.rs` - Crypto unit tests ✓

### Files Modified
- [x] `packages/courier/Cargo.toml` - Added base64, oauth2, reqwest[json] ✓
- [x] `packages/courier/src/lib.rs` - Exported oauth module ✓
- [x] `packages/courier/src/main.rs` - Added GitHub OAuth config to ServeConfig ✓
- [ ] `packages/courier/src/api.rs` - Update State type, add middleware
- [ ] `packages/courier/src/api/v1.rs` - Register new routers
- [x] `packages/courier/src/auth.rs` - Added SessionToken, OrgRole, SessionContext ✓
- [ ] `packages/courier/src/db.rs` - Add ~30 new methods
- [x] `packages/courier/src/crypto.rs` - Added token/PKCE generation ✓
- [x] `packages/courier/schema/schema.sql` - Updated canonical schema ✓
- [ ] `packages/courier/tests/it/helpers.rs` - Update TestAuth for sessions
- [x] `packages/courier/tests/it/main.rs` - Registered crypto test module ✓

## Dependencies to Add

```bash
# Already added:
cargo add oauth2 --package courier  # ✓
cargo add base64 --package courier  # ✓
cargo add reqwest --features json --package courier  # ✓

# Still needed:
cargo add tower-governor --package courier
cargo add --dev wiremock --package courier  # for mocking GitHub API
```

## Current Progress

**Current Phase**: 3 - Database Layer (COMPLETE)
**Current Task**: Ready for Phase 4 (OAuth Flow Endpoints)

## Context for Resume

If resuming after context reset:
1. Read this file first
2. Read RFC at `docs/rfc/0003-self-service-signup.md`
3. Check git status for any in-progress changes
4. Look at the checklist above - find the first unchecked item
5. Continue implementation from there

### Completed Work Summary

**Phase 1 (Database Schema)** - 2 commits:
- Migration 0008 with all new tables (github_identity, organization_role, organization_member, etc.)
- Modified account and api_key tables

**Phase 2 (Core Types & Auth)** - 2 commits:
- auth.rs: SessionToken, OrgRole, SessionContext
- crypto.rs: generate_api_key, generate_session_token, generate_oauth_state, generate_invitation_token, generate_pkce
- oauth.rs: GitHub client, GitHubConfig, fetch_user, fetch_emails
- main.rs: OAuth config in ServeConfig
- 13 crypto unit tests

**Phase 3 (Database Layer)** - 1 commit:
- auth.rs: Added InvitationId, SessionId, ApiKeyId types
- db.rs: ~30 new database methods across 9 operation categories:
  - Account: create, get, get_by_github_id, update_email, update_name, disable, enable
  - GitHub Identity: link, get, update_username
  - Sessions: create, validate, revoke, revoke_all, extend, cleanup_expired
  - OAuth State: store, consume, cleanup_expired
  - Organizations: create, get, list_for_account
  - Memberships: add, remove, update_role, get_role, list, is_last_admin
  - Invitations: create, get_by_token, get_preview, accept, revoke, list
  - Audit Log: log_event
  - API Keys: create (with org_id), list_personal, list_org, revoke, get
- Added `time` dependency for OffsetDateTime (matches sqlx's time feature)
- 55 new integration tests across 8 test modules

## Data Flow Reference

### OAuth Signup Flow
```
1. User → GET /api/v1/oauth/github/start?redirect_uri=...
2. Validate redirect_uri, generate PKCE + state, store in oauth_state
3. Redirect to GitHub OAuth authorize URL
4. User authorizes → GitHub redirects to /api/v1/oauth/github/callback
5. Consume oauth_state, exchange code for token, fetch user profile
6. Find or create account + github_identity
7. Create session in user_session
8. Redirect to redirect_uri with ?session=TOKEN&new_user=true/false
```

### Session Validation Flow
```
1. Request with Authorization: Bearer <token>
2. AuthenticatedToken/SessionContext extractor
3. Try api_key table first (existing behavior)
4. If not found, try user_session table
5. Return appropriate auth context
```

## Security Notes

- PKCE: S256 challenge method, verifier stored server-side
- Session tokens: 256 bits entropy (64 hex chars)
- Invitation tokens: 47-71 bits based on expiry (8-12 alphanumeric)
- OAuth state: 128 bits entropy (32 hex chars), 10 min expiry
- All tokens hashed with SHA-256 before storage
- Redirect URI validated against allowlist
- Rate limiting on sensitive endpoints
