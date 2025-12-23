# Headless Self-Hosting

This guide covers running Courier in headless mode without the web dashboard or GitHub OAuth. This is the simplest deployment option, ideal for solo developers, local development, or CI runners.

## Overview

In headless mode:

- No GitHub OAuth app required
- No web dashboard
- Manage organizations and API tokens via CLI scripts
- All authentication uses bot accounts with API tokens

## Prerequisites

- Docker and Docker Compose
- `curl` and `jq` (for management scripts)
- Git (to clone the repository)

## Quick Start

### 1. Clone the Repository

```bash
git clone https://github.com/attunehq/hurry.git
cd hurry
```

### 2. Start the Services

```bash
docker compose up -d
```

Wait for Courier to be ready:

```bash
until curl -sf http://localhost:3000/api/v1/health > /dev/null; do
  sleep 0.5
done
```

### 3. Create an Organization and Bot Account

Set up environment variables for the management scripts:

```bash
export COURIER_URL=http://localhost:3000
export COURIER_DATABASE_URL=postgres://courier:courier@localhost:5432/courier
```

Create a user session with a fake github oauth binding in the database:

```bash
export COURIER_TOKEN=$(./scripts/api/login "admin@example.com" "admin" "Admin User")
```

View your organizations:

```bash
./scripts/api/org-list
```

Example output:

```json
{
  "organizations": [
    {
      "id": 2,
      "name": "Personal",
      "role": "admin",
      "created_at": "2025-01-15T10:30:00Z"
    }
  ]
}
```

Note the organization ID from the output (the `id` field, `2` in this example).

### 4. Create an API Token for Hurry

> [!IMPORTANT]
> After an API token is shown once, it is never visible in plain text again. Make sure to save it!

For personal use, create an API token (replace `<org-id>` with your organization ID from above):

```bash
./scripts/api/key-create <org-id> "my-laptop"
```

Example output:

```json
{
  "id": 1,
  "name": "my-laptop",
  "token": "b6a78c0f5d2f3e61e82b346fe79e6d95",
  "created_at": "2025-01-15T10:32:00Z"
}
```

For CI/automation you can also create a bot account instead:

```bash
./scripts/api/bot-create <org-id> "CI Bot" "admin@example.com"
```

Example output:

```json
{
  "account_id": 2,
  "name": "CI Bot",
  "api_key": "e2d6aace85c96034b24544417c567944"
}
```

### 5. Configure Hurry

> [!TIP]
> Don't forget to save these in your shell!

Export the tokens to your shell:

```bash
export HURRY_API_URL=http://localhost:3000
export HURRY_API_TOKEN=your-token-here
```

And then run Hurry:

```bash
hurry cargo build
```

## Managing Your Instance

### List API Tokens

> [!NOTE]
> The API token content itself cannot be retrieved after initial creation; this endpoint only lists token metadata.

```bash
./scripts/api/key-list <org-id>
```

Example output:

```json
{
  "api_keys": [
    {
      "id": 1,
      "name": "my-laptop",
      "account_id": 1,
      "account_email": "admin@example.com",
      "bot": false,
      "created_at": "2025-01-15T10:32:00Z",
      "accessed_at": "2025-01-15T10:32:00Z"
    }
  ]
}
```

### Revoke an API Token

```bash
./scripts/api/key-revoke <org-id> <key-id>
```

### View All Bots

```bash
./scripts/api/bot-list <org-id>
```

Example output:

```json
{
  "bots": [
    {
      "account_id": 2,
      "name": "CI Bot",
      "responsible_email": "admin@example.com",
      "created_at": "2025-01-15T10:33:00Z"
    }
  ]
}
```

See [scripts/api/README.md](../../scripts/api/README.md) for the complete API reference.

## Data Persistence

Data is stored in `.hurrydata/` in the repository root:

- `.hurrydata/postgres/data/`: PostgreSQL database files
- `.hurrydata/courier/cas/`: Content-addressed storage (build artifacts)

To back up your instance:

```bash
# Stop services for consistency
docker compose down

# Backup
tar -czf hurry-backup.tar.gz .hurrydata/

# Restart
docker compose up -d
```

To restore:

```bash
docker compose down
tar -xzf hurry-backup.tar.gz
docker compose up -d
```

## Updating

To update to a newer version:

```bash
git pull
docker compose down
docker compose build
docker compose up -d
```

The `docker compose up` command runs migrations before starting Courier.

## Stopping and Starting

```bash
# Stop all services
docker compose down

# Start all services
docker compose up -d
```

## Troubleshooting

### "Connection refused" errors

Ensure Courier is running:

```bash
docker compose ps
docker compose logs courier
```

### "Invalid or revoked token" errors

Your API token may have been revoked. Create a new one:

```bash
export COURIER_TOKEN=$(./scripts/api/login "admin@example.com")
./scripts/api/key-create <org-id> "new-key"
```

### Database connection issues

Check PostgreSQL is healthy:

```bash
docker compose exec postgres pg_isready -U courier
```

### View logs

```bash
# All services
docker compose logs -f

# Just Courier
docker compose logs -f courier
```

### Reset everything

To start fresh:

```bash
docker compose down
rm -rf .hurrydata

# Then take this document from the top
```
