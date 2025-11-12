# Database Management Scripts

Shell scripts for managing organizations, accounts, and API tokens in the Courier database.

## Prerequisites

- `COURIER_DATABASE_URL` environment variable must be set
- `psql` command-line tool must be available
- `openssl` must be available (for token generation)

## Organization Management

### Create Organization
```bash
./scripts/db/org-create <org-name>
```

### List Organizations
```bash
./scripts/db/org-list
```

### Read Organization
```bash
./scripts/db/org-read <org-id>
```

### Update Organization
```bash
./scripts/db/org-update <org-id> <new-name>
```

### Delete Organization
```bash
./scripts/db/org-delete <org-id>
```
Note: Will fail if the organization has accounts.

## Account Management

### Create Account
```bash
./scripts/db/account-create <org-id> <email>
```

### List Accounts
```bash
# List all accounts
./scripts/db/account-list

# List accounts for a specific organization
./scripts/db/account-list <org-id>
```

### Read Account
```bash
./scripts/db/account-read <account-id>
```

### Update Account
```bash
./scripts/db/account-update <account-id> <new-email>
```

### Delete Account
```bash
./scripts/db/account-delete <account-id>
```
Note: Will fail if the account has API keys.

## Token Management

### Create Token
```bash
./scripts/db/token-create <account-id>
```
This generates a new API token and displays it. **Save the token immediately** as it cannot be retrieved later.

### List Tokens
```bash
# List all tokens
./scripts/db/token-list

# List tokens for a specific account
./scripts/db/token-list <account-id>
```

### Lookup Token
```bash
./scripts/db/token-lookup <plaintext-token>
```
Look up token information using the plaintext token value. Shows token ID, associated account, organization, and revocation status.

### Revoke Token
```bash
# Revoke by token ID
./scripts/db/token-revoke <token-id>

# Revoke by plaintext token
./scripts/db/token-revoke <plaintext-token>
```
Accepts either a numeric token ID or the plaintext token value.

## Example Workflow

```bash
# Create an organization
./scripts/db/org-create "Acme Corp"
# Output: Organization created successfully:
#   1 | Acme Corp | 2025-11-12 10:30:00-08

# Create an account in that organization
./scripts/db/account-create 1 "user@acme.com"
# Output: Account created successfully:
#   1 | 1 | user@acme.com | 2025-11-12 10:31:00-08

# Create an API token for the account
./scripts/db/token-create 1
# Output: API token created successfully:
#   1 | 1 | 2025-11-12 10:32:00-08
#
# TOKEN (save this, it will not be shown again):
#   a3f8b9c... (64 character hex string)

# List all tokens to see status
./scripts/db/token-list
# Shows token ID, account ID, email, creation time, last access, and revocation status

# Look up a token by its plaintext value
./scripts/db/token-lookup "a3f8b9c..."
# Shows token info including ID, account, and organization

# Revoke a token when done (by ID or plaintext)
./scripts/db/token-revoke 1
# or
./scripts/db/token-revoke "a3f8b9c..."
```

## Error Handling

All scripts:
- Check for `COURIER_DATABASE_URL` environment variable
- Validate input parameters
- Display clear error messages on stderr
- Exit with non-zero status codes on failure
- Handle database constraint violations gracefully

## Notes

- Token hashing uses SHA-256 for storage security
- Tokens are 64-character hexadecimal strings (32 random bytes)
- Revoked tokens remain in the database with `revoked_at` timestamp
- Foreign key constraints prevent orphaned records
