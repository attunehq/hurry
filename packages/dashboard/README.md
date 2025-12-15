# Hurry Console

Vite + React Router console for managing Hurry accounts, orgs, invitations, API keys, and bots.

## Local dev

Prereqs:
- Courier running at `http://localhost:3000`

From this directory:

```bash
npm install
npm run dev
```

The Vite dev server proxies `/api/*` to `http://localhost:3000` so the app can call the API without CORS.

### Auth

Preferred: GitHub OAuth (if configured on your Courier instance).

Dev fallback: create a session token using the repo scripts, then paste it into the console:

```bash
export COURIER_URL=http://localhost:3000
export COURIER_DATABASE_URL=postgres://localhost/courier
export COURIER_TOKEN=$(../../scripts/api/login "dev@example.com" "dev-user" "Dev User")
echo "$COURIER_TOKEN"
```

Open the console at `http://localhost:5173/auth` and use "Use a session token".

## Config

- `VITE_API_ORIGIN` (optional): API origin (default: same-origin). Example: `http://localhost:3000`
