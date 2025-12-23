# Docker Compose with Dashboard

This guide covers running Courier with the full web dashboard and GitHub OAuth authentication. This is ideal for small teams who want a user-friendly interface for managing organizations, API tokens, and team members.

## Overview

This deployment includes:

- Courier API server
- PostgreSQL database
- Web dashboard for team management
- GitHub OAuth for authentication

## Prerequisites

- Docker and Docker Compose
- A GitHub account (for creating an OAuth app)
- Git (to clone the repository)

## Quick Start

### 1. Clone the Repository

```bash
git clone https://github.com/attunehq/hurry.git
cd hurry
```

### 2. Create a GitHub OAuth App

> [!IMPORTANT]
> In this doc we assume the Hurry API (Courier) is running on and accessible at `http://localhost:3000`, but if you plan to have it accessible somewhere else you'll need to account for that during setup.

1. Go to GitHub Settings > Developer settings > OAuth Apps
2. Click "New OAuth App"
3. Fill in the details:
   - Application name: `Hurry (Self-Hosted)` (or your preference)
   - Homepage URL: `http://localhost:3000`
   - Authorization callback URL: `http://localhost:3000/api/v1/oauth/callback`
4. Click "Register application"
5. Note the Client ID
6. Click "Generate a new client secret" and save it immediately

### 3. Configure Environment

Create a `.env` file in the repository root:

```bash
cat > .env << 'EOF'
GITHUB_CLIENT_ID=your-client-id-here
GITHUB_CLIENT_SECRET=your-client-secret-here
OAUTH_REDIRECT_ALLOWLIST=http://localhost:3000
EOF
```

Replace the placeholder values with your GitHub OAuth credentials.

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

Click "Sign in with GitHub" to authenticate. After signing in, you'll land in an automatically created "Personal" organization.
You can create another org, or you can just rename this one, it's up to you.

After your org is ready, you can:
1. Create API tokens and bot accounts
2. Invite team members using invitation links

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
> Don't forget to save these in your shell!

```bash
export HURRY_API_URL=http://localhost:3000
export HURRY_API_TOKEN=their-api-token
```

And then run Hurry:
```bash
hurry cargo build
```

## Non-Localhost Deployments

If you're running Courier on a server (not localhost), you'll need to update the OAuth configuration.

### Update GitHub OAuth App

1. Go to your GitHub OAuth App settings
2. Update Homepage URL to your server's URL (e.g., `https://hurry.internal.example.com`)
3. Update Authorization callback URL (e.g., `https://hurry.internal.example.com/api/v1/oauth/callback`)

### Update Environment

```bash
cat > .env << 'EOF'
GITHUB_CLIENT_ID=your-client-id-here
GITHUB_CLIENT_SECRET=your-client-secret-here
OAUTH_REDIRECT_ALLOWLIST=https://hurry.internal.example.com
EOF
```

### Configure TLS (Recommended)

For non-localhost deployments, you should use TLS. Put a reverse proxy (nginx, Caddy, Traefik, etc) in front of Courier:

```yaml
# Example: Add to docker-compose.override.yml
services:
  caddy:
    image: caddy:2
    ports:
      - "443:443"
      - "80:80"
    volumes:
      - ./Caddyfile:/etc/caddy/Caddyfile
      - caddy_data:/data
    depends_on:
      - courier

volumes:
  caddy_data:
```

```
# Caddyfile
hurry.internal.example.com {
    reverse_proxy courier:3000
}
```

### Client Configuration

Team members configure their clients with the public URL:

```bash
export HURRY_API_URL=https://hurry.internal.example.com
export HURRY_API_TOKEN=their-api-token
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
2. Click the "Promote" or "Demote" button

### Remove Members

1. Go to "Members"
2. Click "Remove" next to the member

### Using the API

You can also manage Courier programmatically using the API. See [scripts/api/README.md](../../scripts/api/README.md) for helper scripts and examples.

## Data Persistence

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

### Storage Location

- `.hurrydata/postgres/data/`: PostgreSQL database files
- `.hurrydata/courier/cas/`: Content-addressed storage (build artifacts)

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

### OAuth "redirect_uri mismatch" error

Your OAuth redirect URL doesn't match. Ensure:

1. GitHub OAuth App callback URL matches exactly
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

# Then take this document from the top
```
