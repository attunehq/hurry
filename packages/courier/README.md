
# Hurry API (Courier)

Courier is the API service for Hurry, providing CAS functionality (and in the future, caching functionality as well).

## Account management

See `scripts/db` for account management scripts; these are intended for use in any environment.

## Running Courier

Run Courier with the `serve` subcommand:
```sh
courier serve
```

Note that there are several required arguments/environment variables for this command; view them in the help output:
```sh
courier serve --help
```

Alternatively, run it in Docker:
```sh
docker compose up
```

### Local Development Setup

For local development with authentication enabled, use:
```sh
make reset-local-cache
```

This will:
1. Stop any running containers
2. Clear local data
3. Start PostgreSQL
4. Apply all migrations
5. Load test auth fixtures

After running, you'll see available test tokens:
```
Local auth fixture loaded. Available tokens:
  acme-alice-token-001         (alice@acme.com, Acme Corp)
  acme-bob-token-001           (bob@acme.com, Acme Corp)
  widget-charlie-token-001     (charlie@widget.com, Widget Inc)
```

You can then use these tokens to test authenticated requests:
```sh
hurry cargo build \
  --hurry-api-url http://localhost:3000 \
  --hurry-api-token acme-alice-token-001
```

To load just the auth fixtures without resetting everything:
```sh
make courier-local-auth
```

### Testing GitHub OAuth Locally

To test the full GitHub OAuth flow locally:

1. Go to the GitHub OAuth app settings: https://github.com/organizations/attunehq/settings/apps/attune-hurry-dev

2. Click "Generate a new client secret" (if you don't already have one)

3. Add the credentials to your `.env` file:
   ```sh
   GITHUB_CLIENT_ID="<copy Client ID from the OAuth app page>"
   GITHUB_CLIENT_SECRET="<paste the generated secret>"
   OAUTH_REDIRECT_ALLOWLIST=http://localhost:3000,http://localhost:5173
   ```

4. Run Courier (it will read from `.env` automatically):
   ```sh
   cargo run -p courier --release -- serve
   ```

5. Start the dashboard dev server (in another terminal):
   ```sh
   cd packages/dashboard && npm run dev
   ```

6. Visit http://localhost:5173 and click "Sign in with GitHub"

## Migrations

The canonical database state is at `schema/schema.sql`.
We use [`sql-schema`](https://lib.rs/crates/sql-schema) to manage migrations; the server binary is able to apply its migrations if run with the correct command.

> [!TIP]
> You should run Postgres inside Docker; these docs assume you're doing so and it's a lot easier.

### Generating new migrations

After making changes to the canonical schema file, run:
```sh
sql-schema migration --name {new name here}
```

> [!IMPORTANT]
> As the docs for `sql-schema` state, the tool is experimental; make sure to double check your migration files.

### Applying migrations

When you run `docker compose up` this is done automatically; you should only have to do this if you have a long-running database instance and you're running Courier locally.

#### Option 1: Using sqlx-cli (recommended for development)
```sh
cargo sqlx migrate run --source packages/courier/schema/migrations/
```

This is the fastest option for local development since it applies migrations directly from the filesystem without rebuilding.

#### Option 2: Using the courier binary
```sh
docker compose run --build migrate
```

The `courier migrate` command exists so that when we cut a release, that release's migrations can be applied using the binary itself (migrations are embedded at compile time). This is the production deployment approach. We don't auto-apply migrations on server startup to reduce the risk of accidentally migrating the wrong environment.

Note: The Docker approach requires `--build` to ensure the image includes your latest migrations.
