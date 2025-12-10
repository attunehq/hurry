# Phase 1: Database Schema & Migrations

Parent: `.scratch/self-service-signup.md`

## Overview

Create database migrations to support self-service signup. Two migrations:
1. **0008**: Add all new tables (additive, non-breaking) - DEPLOY IMMEDIATELY
2. **0009**: Migrate account model (remove `account.organization_id`) - DEPLOY AFTER CODE UPDATED

## Migration 0008: Add Self-Service Tables

This migration is **additive only** - safe to deploy to production immediately.

### New Tables

#### `github_identity`
Links GitHub user to Courier account (1:1).
```sql
CREATE TABLE github_identity (
  id BIGSERIAL PRIMARY KEY,
  account_id BIGINT NOT NULL REFERENCES account(id) UNIQUE,
  github_user_id BIGINT NOT NULL UNIQUE,
  github_username TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

#### `organization_role`
Role definitions (member, admin). Using table instead of enum for easier extension.
```sql
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

#### `organization_member`
Account membership in organizations. Replaces `account.organization_id`.
```sql
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

#### `organization_invitation`
Invitation links for joining organizations.
```sql
CREATE TABLE organization_invitation (
  id BIGSERIAL PRIMARY KEY,
  organization_id BIGINT NOT NULL REFERENCES organization(id),
  token TEXT NOT NULL UNIQUE,
  role_id BIGINT NOT NULL REFERENCES organization_role(id),
  created_by BIGINT NOT NULL REFERENCES account(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  expires_at TIMESTAMPTZ NOT NULL,
  max_uses INT,
  use_count INT NOT NULL DEFAULT 0,
  revoked_at TIMESTAMPTZ
);

CREATE INDEX idx_invitation_org ON organization_invitation(organization_id);
CREATE INDEX idx_invitation_expires ON organization_invitation(expires_at);
```

#### `invitation_redemption`
Tracks who used which invitation.
```sql
CREATE TABLE invitation_redemption (
  id BIGSERIAL PRIMARY KEY,
  invitation_id BIGINT NOT NULL REFERENCES organization_invitation(id),
  account_id BIGINT NOT NULL REFERENCES account(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE (invitation_id, account_id)
);
```

#### `oauth_state`
Temporary storage for OAuth flow state (PKCE).
```sql
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

#### `user_session`
Active user sessions for web UI.
```sql
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

#### `audit_log`
Security event logging.
```sql
CREATE TABLE audit_log (
  id BIGSERIAL PRIMARY KEY,
  account_id BIGINT REFERENCES account(id),
  organization_id BIGINT REFERENCES organization(id),
  action TEXT NOT NULL,
  details JSONB,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_audit_log_account ON audit_log(account_id);
CREATE INDEX idx_audit_log_org ON audit_log(organization_id);
CREATE INDEX idx_audit_log_created ON audit_log(created_at);
```

### Table Modifications

#### `account` - Add columns
```sql
ALTER TABLE account ADD COLUMN disabled_at TIMESTAMPTZ;
ALTER TABLE account ADD COLUMN name TEXT;
```

#### `api_key` - Add organization scope
```sql
ALTER TABLE api_key ADD COLUMN organization_id BIGINT REFERENCES organization(id);
```

Note: `organization_id` is nullable. NULL = personal API key, non-NULL = org-scoped.

## Migration 0009: Remove account.organization_id

This migration runs AFTER all code is updated to use `organization_member`.

```sql
-- Step 1: Populate organization_member from existing account.organization_id
-- Existing accounts become admins of their current org
INSERT INTO organization_member (organization_id, account_id, role_id)
SELECT
  a.organization_id,
  a.id,
  (SELECT id FROM organization_role WHERE name = 'admin')
FROM account a
WHERE a.organization_id IS NOT NULL
ON CONFLICT DO NOTHING;

-- Step 2: Populate api_key.organization_id from account relationships
-- This preserves existing behavior where API key inherits org from account
UPDATE api_key
SET organization_id = account.organization_id
FROM account
WHERE api_key.account_id = account.id
AND api_key.organization_id IS NULL
AND account.organization_id IS NOT NULL;

-- Step 3: Remove the old column
ALTER TABLE account DROP COLUMN organization_id;
```

## Down Migrations

### 0008.down.sql
Drop all new tables and columns in reverse order:
1. Drop `audit_log`
2. Drop `user_session`
3. Drop `oauth_state`
4. Drop `invitation_redemption`
5. Drop `organization_invitation`
6. Drop `organization_member`
7. Drop `organization_role`
8. Drop `github_identity`
9. Drop `api_key.organization_id`
10. Drop `account.name`
11. Drop `account.disabled_at`

### 0009.down.sql
Re-add `account.organization_id` and repopulate from `organization_member`:
```sql
-- Re-add column
ALTER TABLE account ADD COLUMN organization_id BIGINT REFERENCES organization(id);

-- Repopulate from organization_member (pick first org if multiple)
UPDATE account
SET organization_id = om.organization_id
FROM (
  SELECT DISTINCT ON (account_id) account_id, organization_id
  FROM organization_member
  ORDER BY account_id, created_at ASC
) om
WHERE account.id = om.account_id;
```

## Checklist

- [ ] Write `0008_add_self_service_signup.up.sql`
- [ ] Write `0008_add_self_service_signup.down.sql`
- [ ] Update `schema/schema.sql` to include all new tables
- [ ] Run migrations locally: `cargo run -p courier -- migrate --database-url $COURIER_DATABASE_URL`
- [ ] Run `make sqlx-prepare`
- [ ] Verify tests still pass: `cargo nextest run -p courier`

Note: Migration 0009 will be written later, after Phase 7 (API Key Management) is complete.

## Testing the Migration

```bash
# Start fresh database
docker compose down -v
docker compose up -d

# Run migrations
cargo run -p courier -- migrate --database-url "postgresql://courier:courier@localhost:5432/courier"

# Verify tables exist
psql "postgresql://courier:courier@localhost:5432/courier" -c "\dt"

# Run tests
cargo nextest run -p courier
```
