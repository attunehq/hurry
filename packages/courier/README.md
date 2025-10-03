
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

```sh
docker compose run migrate
```
