# API Management Scripts

Shell scripts for managing users, organizations, and API tokens via the Courier API.

## Prerequisites

- `COURIER_URL` environment variable must be set (e.g., `http://localhost:3000`)
- `COURIER_TOKEN` environment variable must be set for authenticated endpoints
- `curl` and `jq` must be available
- For `login`, you also need `COURIER_DATABASE_URL` (it creates users directly in the database)

## Debugging

Set `COURIER_DEBUG=1` to echo curl commands before running them:

```bash
COURIER_DEBUG=1 ./scripts/api/me
```

## Validation

Run the full validation script to exercise all endpoints:

```bash
./scripts/api/validate
```

This creates test users, organizations, API keys, invitations, and bot accounts to verify everything works correctly. Output includes curl commands for documentation.

## Authentication

### Login (Fake GitHub OAuth)

For development without a GitHub OAuth app configured, you can create a user and session directly:

```bash
export COURIER_TOKEN=$(./scripts/api/login <email> [github-username] [name])
```

This bypasses the normal GitHub OAuth flow and:
1. Creates an account with a fake GitHub identity (if it doesn't exist)
2. Creates a default organization for the user
3. Creates a session token and outputs it to stdout

**Note**: This requires `COURIER_DATABASE_URL` since it writes directly to the database.

### Logout

```bash
./scripts/api/logout
```

Revokes the current session. You should then `unset COURIER_TOKEN`.

## User Profile

### Get Current User

```bash
./scripts/api/me
```

Shows the authenticated user's profile information.

### List My Organizations

```bash
./scripts/api/org-list
```

Shows all organizations the current user belongs to.

## Organization Management

### Create Organization

```bash
./scripts/api/org-create <name>
```

Creates a new organization with the current user as admin.

### List Members

```bash
./scripts/api/org-members <org-id>
```

Lists members of an organization.

### Update Member Role

```bash
./scripts/api/member-role <org-id> <account-id> <role>
```

Updates a member's role (`admin` or `member`). Admin only.

### Remove Member

```bash
./scripts/api/member-remove <org-id> <account-id>
```

Removes a member from an organization. Admin only.

### Leave Organization

```bash
./scripts/api/org-leave <org-id>
```

Leave an organization (cannot be the last admin).

## API Key Management

### Create API Key

```bash
./scripts/api/key-create <org-id> <name>
```

Creates a new API key for the organization. **Save the token immediately** as it cannot be retrieved later.

### List API Keys

```bash
./scripts/api/key-list <org-id>
```

Lists all API keys for an organization.

### Revoke API Key

```bash
./scripts/api/key-revoke <org-id> <key-id>
```

Revokes an API key.

## Invitation Management

### Create Invitation

```bash
./scripts/api/invite-create <org-id> [role] [max-uses]
```

Creates an invitation link. Role defaults to "member". Admin only.

### List Invitations

```bash
./scripts/api/invite-list <org-id>
```

Lists all invitations for an organization. Admin only.

### Preview Invitation

```bash
./scripts/api/invite-preview <token>
```

Preview invitation details (public, no auth required).

### Accept Invitation

```bash
./scripts/api/invite-accept <token>
```

Accepts an invitation using the invitation token.

### Revoke Invitation

```bash
./scripts/api/invite-revoke <org-id> <invitation-id>
```

Revokes an invitation. Admin only.

## Bot Accounts

### Create Bot

```bash
./scripts/api/bot-create <org-id> <name> <responsible-email>
```

Creates a bot account for CI/automation. Returns an API key. Admin only.

### List Bots

```bash
./scripts/api/bot-list <org-id>
```

Lists bot accounts in an organization. Admin only.

## Example Workflow

```bash
# Set environment variables
export COURIER_URL=http://localhost:3000
export COURIER_DATABASE_URL=postgres://localhost/courier

# Create a development user and login
export COURIER_TOKEN=$(./scripts/api/login "dev@example.com" "dev-user" "Dev User")

# Check your profile
./scripts/api/me
# Output: {"id": 1, "email": "dev@example.com", ...}

# List your organizations
./scripts/api/org-list
# Output: {"organizations": [{"id": 1, "name": "dev-user's Org", ...}]}

# Create an API key for your org
./scripts/api/key-create 1 "my-dev-key"
# Output: {"id": 1, "name": "my-dev-key", "token": "...", ...}

# Create an invitation link
./scripts/api/invite-create 1 member 5
# Output: {"id": 1, "token": "abc123", ...}

# Create a bot for CI
./scripts/api/bot-create 1 "CI Bot" "ops@example.com"
# Output: {"account_id": 2, "name": "CI Bot", "api_key": "..."}

# Logout when done
./scripts/api/logout
unset COURIER_TOKEN
```

## Environment Variables

| Variable | Description | Example |
|----------|-------------|---------|
| `COURIER_URL` | Base URL of the Courier API | `http://localhost:3000` |
| `COURIER_TOKEN` | Session token for authentication | (output from `login`) |
| `COURIER_DATABASE_URL` | PostgreSQL connection string (for `login` only) | `postgres://localhost/courier` |
| `COURIER_DEBUG` | Set to `1` to echo curl commands | `1` |

## Error Handling

All scripts:
- Check for required environment variables
- Validate input parameters
- Display clear error messages on stderr
- Exit with non-zero status codes on failure
- Parse JSON responses and format output nicely

## Notes

- The `login` script requires direct database access and is for development only
- Session tokens expire after 24 hours
- API keys do not expire but can be revoked
- Invitation tokens can have expiration and usage limits
