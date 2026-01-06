# Self-Hosting Hurry

This guide covers self-hosting Hurry using Docker Compose with the web dashboard and GitHub App authentication.

## Architecture Overview

A self-hosted Hurry deployment consists of:

- Courier: Serves the dashboard, cache metadata, and build artifacts
- PostgreSQL: Database for user accounts, organizations, and cache metadata
- CAS Storage: Disk storage for build artifacts

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   hurry     │────▶│   Courier   │────▶│ PostgreSQL  │
│   (CLI)     │     │   (API)     │     │             │
└─────────────┘     └──────┬──────┘     └─────────────┘
                           │
                           ▼
                    ┌─────────────┐
                    │ CAS Storage │
                    │   (disk)    │
                    └─────────────┘
```

## Prerequisites

- Docker and Docker Compose
- A GitHub account (for creating a GitHub App and logging in)
- Git (to clone the repository)

## Quick Start

### 1. Clone the Repository

```bash
git clone https://github.com/attunehq/hurry.git
cd hurry
```

### 2. Create a GitHub App

> [!IMPORTANT]
> This guide assumes Courier is running at `http://localhost:3000`. If you're deploying elsewhere, replace this URL with the URL at which you're deploying Courier. We recommend setting up Courier behind a TLS termination proxy for non-local deployments.

1. Go to the organization you want to set up, then Organization Settings > Developer settings > GitHub Apps.
  - Replace $ORG with your organization name to go there directly: `https://github.com/organizations/$ORG/settings/apps`
2. Click "New GitHub App"
3. Fill in the details:
   - GitHub App name: `$ORG Hurry Self-Hosted` (must be unique across GitHub)
   - Homepage URL: `http://localhost:3000`
   - Callback URL: `http://localhost:3000/api/v1/oauth/callback`
   - Uncheck "Webhook > Active" (not needed)
4. Under "Account permissions", set "Email addresses" to "Read-only"
5. Click "Create GitHub App"
6. Note the Client ID (not the App ID)
7. Scroll down and click "Generate a new client secret" - save it immediately

### 3. Configure Environment

Create a `.env` file in the repository root:

```bash
cat > .env << 'EOF'
GITHUB_CLIENT_ID=your-client-id-here
GITHUB_CLIENT_SECRET=your-client-secret-here
OAUTH_REDIRECT_ALLOWLIST=http://localhost:3000
EOF
```

Replace the placeholder values with your GitHub App credentials.

### 4. Start the Services

```bash
docker compose up -d
```

This starts PostgreSQL, runs migrations, and starts Courier with the dashboard.

Wait for Courier to be ready:

```bash
until curl -sf http://localhost:3000/api/v1/health > /dev/null; do
  sleep 0.5
done
```

### 5. Access the Dashboard

Open http://localhost:3000 in your browser.

Click "Sign in with GitHub" to authenticate. After signing in, you'll land in an automatically created "Personal" organization. You can create another org or rename this one.

### 6. Create API Tokens

> [!IMPORTANT]
> After an API token is shown once, it is never visible in plain text again. Make sure to save it!

In the dashboard:

1. Navigate to your organization
2. Go to "API Tokens"
3. Click "Create API Token"
4. Give it a name and copy the token

For CI/automation, create a bot account:

1. Go to "Bots"
2. Click "Create Bot"
3. Enter a name and responsible email
4. Copy the API token

### 7. Configure Hurry Clients

> [!TIP]
> Don't forget to save these in your shell configuration!

```bash
export HURRY_API_URL=http://localhost:3000
export HURRY_API_TOKEN=your-api-token
```

Then run Hurry:

```bash
hurry cargo build
```

## Team Management

### Invite Team Members

1. In the dashboard, go to your organization
2. Click "Invitations"
3. Create an invitation link (optionally set max uses)
4. Share the link with team members

When they click the link, they'll sign in with GitHub and join your organization.

### Manage Roles

Organization members can have one of two roles:

- Member: Can use the cache and view organization info
- Admin: Can manage API tokens, bots, and invitations

To change a member's role:

1. Go to "Members" in your organization
2. Click "Make Admin" or "Make Member"

### Remove Members

1. Go to "Members"
2. Click "Remove" next to the member

### Using the API

You can also manage Courier programmatically using the API. See [scripts/api/README.md](../scripts/api/README.md) for helper scripts and examples.

## Data Persistence

Data is stored in `.hurrydata/` in the repository root:

- `.hurrydata/postgres/data/`: PostgreSQL database files
- `.hurrydata/courier/cas/`: Content-addressed storage (build artifacts)

### Backup

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

### Clear Remote Cache

To clear server cache:

```bash
hurry cache reset --remote
```

This removes all cache metadata for your organization from the Courier server. This does not remove the actual artifacts from disk.

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

## Requirements

> [!NOTE]
> It is likely possible to run the software on lower resources than these, but we haven't tested those configurations.

### Hurry

These recommendations are mainly driven by Rust compilation needs, although Hurry also does compression and decompression in parallel which benefits from multiple cores.

| Component | Minimum | Recommended |
|-----------|---------|-------------|
| CPU       | 2 core  | 10+ core    |
| Memory    | 2 GB    | 4 GB        |

### Courier

> [!NOTE]
> Hardware requirements, especially storage, scale with your codebase and team size. The CAS uses content-addressed deduplication, so identical artifacts are stored only once regardless of how many projects use them.

| Component  | Minimum | Recommended |
|------------|---------|-------------|
| PostgreSQL | 18+     | 18+         |
| Disk (CAS) | 10 GB   | 100+ GB     |
| CPU        | 2 core  | 4+ core     |
| Memory     | 2 GB    | 4 GB        |

## Troubleshooting

### "redirect_uri mismatch" error

Your redirect URL doesn't match. Ensure:

1. GitHub App callback URL matches exactly
2. `OAUTH_REDIRECT_ALLOWLIST` in `.env` matches
3. No trailing slashes

### "Invalid or expired session" errors

Sessions expire after 24 hours. Sign in again via the dashboard.

### Dashboard shows blank page

Check browser console for errors. Common causes:

- CORS issues (check `OAUTH_REDIRECT_ALLOWLIST`)
- Courier not running (`docker compose ps`)

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

# Then follow this guide from the top
```
