-- Schema file for Courier.
--
-- After making changes to this file, create a migration in ./migrations to
-- apply the new changes. Each migration should be sequentially ordered after
-- the previous one using its numeric prefix.

-- Organizations in the instance.
CREATE TABLE organization (
  id BIGSERIAL PRIMARY KEY,
  name TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Each distinct actor in the application is an "account"; this could be humans
-- or it could be bots. In the case of bots, the "email" field is for where the
-- person/team owning the bot can be reached.
CREATE TABLE account (
  id BIGSERIAL PRIMARY KEY,
  organization_id BIGINT NOT NULL REFERENCES organization(id),
  email TEXT NOT NULL UNIQUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Keys for accounts to use to authenticate.
CREATE TABLE api_key (
  id BIGSERIAL PRIMARY KEY,
  account_id BIGINT NOT NULL REFERENCES account(id),
  hash BYTEA NOT NULL UNIQUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  accessed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  revoked_at TIMESTAMPTZ
);

-- Lists CAS keys known about by the database.
--
-- Since the CAS keys are actually on disk, technically there could be keys
-- that exist that are not in the database (or vice versa) but the ones in the
-- database are the only ones that the application knows exist.
CREATE TABLE cas_key (
  id BIGSERIAL PRIMARY KEY,
  content BYTEA NOT NULL UNIQUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Controls what organizations have access to a given CAS key.
--
-- We deduplicate CAS keys: if two organizations both save the same content,
-- we only actually store one copy of it (since they're keyed by content, they
-- are by defintion safe to deduplicate).
--
-- Organizations are given access after they upload the content themselves.
CREATE TABLE cas_access (
  organization_id BIGINT NOT NULL REFERENCES organization(id),
  cas_key_id BIGINT NOT NULL REFERENCES cas_key(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (organization_id, cas_key_id)
);

-- Models `UnitPlanInfo`.
--
-- Within a given `Vec<SavedUnit>`, each entry has a `UnitPlanInfo` attached
-- and this value is _always_ the same across the entire plan.
--
-- Given this, we use `UnitPlanInfo` as our "primary data source". To
-- reconstruct a `Vec<SavedUnit>`:
-- - Find the plan by its `unit_hash`
-- - Join to `SavedUnit` instances through `cargo_unit_plan_saved_unit`
-- - Order them by `entry_order`.
create table cargo_unit_plan_info (
  id bigserial primary key,
  organization_id bigint not null references organization(id),

  unit_hash text not null,
  package_name text not null,
  crate_name text not null,
  target_arch text,

  created_at timestamptz not null default now(),
  unique(
    unit_hash,
    package_name,
    crate_name,
    target_arch
  )
);

-- Stores `SavedUnit` instances.
--
-- We store these using JSONB encoding:
-- - Many of the inner types use relatively extensive heterogenous types which
--   are difficult to model well in SQL.
-- - We don't really ever need to compose a `SavedUnit` instance from smaller
--   components; they tend to have a lot of local data that can't really be
--   shared.
-- - We expect the amount of data duplication to be relatively low.
--
-- If any of these points end up false in the future we can explore normalizing
-- the data into tables or moving parts of this into the CAS or some other
-- strategy.
--
-- Important: We _do_ expect that any instance where the actual content of files
-- is stored in this type is replaced with the CAS key.
create table cargo_saved_unit (
  id bigserial primary key,
  hash text not null references cargo_unit_plan_info(unit_hash),
  data jsonb not null,
  created_at timestamptz not null default now(),
  unique(hash)
);

-- Maps multiple `SavedUnit` instances to a given `UnitPlanInfo`.
create table cargo_unit_plan_saved_unit (
  id bigserial primary key,
  entry_order int not null,
  unit_plan__id bigint not null references cargo_unit_plan_info(id),
  saved_unit_id bigint not null references cargo_saved_unit(id),
);
