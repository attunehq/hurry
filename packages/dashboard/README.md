# Hurry Console

Vite + React Router console for managing Hurry accounts, orgs, invitations, API keys, and bots.

## Local dev

### Option 1: Vite dev server (recommended for development)

This gives you hot module reloading for fast iteration.

Prereqs:
- Courier running at `http://localhost:3000`

From this directory:

```bash
npm install
npm run dev
```

The Vite dev server proxies `/api/*` to `http://localhost:3000` so the app can call the API without CORS.

Open the console at `http://localhost:5173`.

### Option 2: Courier serving the console

This mimics production by having Courier serve the built console.

First, build the console:

```bash
cd packages/dashboard
npm install
npm run build
```

Then start Courier with `CONSOLE_DIR` pointing to the build output:

```bash
CONSOLE_DIR=packages/dashboard/build/client cargo run -p courier -- serve \
  --database-url postgres://localhost/courier \
  --cas-root /tmp/courier-cas
```

Open the console at `http://localhost:3000`.

### Auth

Preferred: GitHub OAuth (if configured on your Courier instance).

Dev fallback: create a session token using the repo scripts, then paste it into the console:

```bash
export COURIER_URL=http://localhost:3000
export COURIER_DATABASE_URL=postgres://localhost/courier
export COURIER_TOKEN=$(../../scripts/api/login "dev@example.com" "dev-user" "Dev User")
echo "$COURIER_TOKEN"
```

Open the console and use "Use a session token".

## Production deployment

The console is bundled into the Courier Docker image. Courier serves the console at `/` and the API at `/api/*`.

### Docker image

Build locally (from repo root):

```bash
docker build -f docker/courier/Dockerfile -t courier .
```

Or pull from ECR (after pushing to main):

```bash
docker pull 419868707503.dkr.ecr.us-west-1.amazonaws.com/courier:<tag>
```

### Environment variables

**Courier runtime** (console-related):

| Variable | Required | Description |
|----------|----------|-------------|
| `CONSOLE_DIR` | No | Path to console static files. Set automatically in Docker image (`/courier/console`). Omit to disable console serving. |

The Docker image sets `CONSOLE_DIR=/courier/console` by default, so the console is served automatically.

### Running

```bash
docker run -p 3000:3000 \
  -e COURIER_DATABASE_URL=postgres://... \
  -e CAS_ROOT=/data/cas \
  -v /path/to/cas:/data/cas \
  courier serve
```

The console will be available at `http://localhost:3000/` and the API at `http://localhost:3000/api/`.

### Health check

The container health check hits `/api/v1/health`.
