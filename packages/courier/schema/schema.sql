-- Schema file for Courier.
-- This file is maintained by hand; we use `sql-schema` to generate migrations.
--
-- After making changes to this file, run `sql-schema` to generate a migration
-- within the root of the `courier` package:
-- ```
-- sql-schema migration --name {new name here}
-- ```

-- Organizations
create table organizations (
    id bigserial primary key not null,
    name text not null,
    created timestamptz not null default now()
);

-- Users
create table users (
    id bigserial primary key not null,
    organization_id bigint references organizations(id) not null,
    email text not null unique,
    created timestamptz not null default now()
);

-- API Keys
create table api_keys (
    id bigserial primary key not null,
    user_id bigint references users(id) not null,
    content text not null,
    created timestamptz not null default now(),
    accessed timestamptz not null default now(),
    revoked timestamptz,
    unique(content)
);

-- CAS Key Index
create table cas_keys (
    id bigserial primary key not null,
    content bytea not null,
    created timestamptz not null default now(),
    unique(content)
);

-- Access Control
create table cas_access (
    org_id bigint references organizations(id) not null,
    cas_key_id bigint references cas_keys(id) not null,
    created timestamptz not null default now(),
    primary key (org_id, cas_key_id)
);

-- Frequency Tracking
create table frequency_user_cas_key (
    user_id bigint references users(id) not null,
    cas_key_id bigint references cas_keys(id) not null,
    accessed timestamptz not null default now(),
    primary key (user_id, cas_key_id, accessed)
);

create index idx_frequency_user_key_recent
    on frequency_user_cas_key(user_id, cas_key_id, accessed desc);
