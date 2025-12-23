# Self-Hosting Hurry

Hurry supports self-hosting through Courier, the API server that provides content-addressed storage (CAS) for build artifacts. This guide covers two deployment options depending on your needs.

## Architecture Overview

A self-hosted Hurry deployment consists of:

- Courier: The API server that stores and serves build artifacts
- PostgreSQL: Database for user accounts, organizations, and cache metadata
- CAS Storage: Disk storage for compressed build artifacts
- Dashboard (optional): Web UI for managing organizations, API tokens, and team members

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

## Deployment Options

Choose the option that best fits your needs:

### [Headless Mode](self-hosting/headless.md)

Best for solo developers, local development, and CI runners.

- No GitHub OAuth required
- No web dashboard
- Manage everything via CLI scripts
- Simplest setup

### [Docker Compose with Dashboard](self-hosting/docker-compose.md)

Best for small teams and on-premise deployments.

- Full web dashboard for team management
- GitHub OAuth for authentication
- Docker Compose for easy deployment
- Suitable for localhost or internal network

## Client Configuration

Once your Courier instance is running, configure the `hurry` CLI to use it:

```bash
# Point hurry at your self-hosted instance
export HURRY_API_URL=http://your-courier-host:3000
export HURRY_API_TOKEN=your-api-token

# Use hurry as normal
hurry cargo build
```

## Requirements

> [!NOTE]
> It may be possible to run the software on lower resources than these, but we haven't tested those configurations.

### Hurry

| Component  | Minimum | Recommended |
|------------|---------|-------------|
| CPU        | 2 core  | 10+ core    |
| Memory     | 2 GB    | 4 GB        |

### Courier

> [!NOTE]
> Hardware requirements, especially storage, scale with your codebase and team size. The CAS uses content-addressed deduplication, so identical artifacts are stored only once regardless of how many projects use them.

| Component  | Minimum | Recommended |
|------------|---------|-------------|
| PostgreSQL | 18+     | 18+         |
| Disk (CAS) | 10 GB   | 100+ GB     |
| CPU        | 2 core  | 4+ core    |
| Memory     | 2 GB    | 4 GB        |

## Next Steps

1. Choose a [deployment option](#deployment-options) above
2. Follow the setup guide for your chosen option
3. Create an organization and API tokens
4. Configure your `hurry` CLI to use your instance
