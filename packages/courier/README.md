
# Courier

Courier is the API service for Hurry, providing CAS functionality (and in the future, caching functionality as well).

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
