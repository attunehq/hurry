-- Schema file for Courier.
-- This file is maintained by hand; we use `sql-schema` to generate migrations.
--
-- After making changes to this file, run `sql-schema` to generate a migration
-- within the root of the `courier` package:
-- ```
-- sql-schema migration --name {new name here}
-- ```

-- Organization
create table organization (
    id bigserial primary key not null,
    name text not null,
    created timestamptz not null default now()
);

-- Account
create table account (
    id bigserial primary key not null,
    organization_id bigint references organization(id) not null,
    email text not null unique,
    created timestamptz not null default now()
);

-- API Key
create table api_key (
    id bigserial primary key not null,
    account_id bigint references account(id) not null,
    content text not null,
    created timestamptz not null default now(),
    accessed timestamptz not null default now(),
    revoked timestamptz,
    unique(content)
);

-- CAS Key Index
create table cas_key (
    id bigserial primary key not null,
    content bytea not null,
    created timestamptz not null default now(),
    unique(content)
);

-- Access Control
create table cas_access (
    org_id bigint references organization(id) not null,
    cas_key_id bigint references cas_key(id) not null,
    created timestamptz not null default now(),
    primary key (org_id, cas_key_id)
);

-- Frequency Tracking
create table frequency_account_cas_key (
    account_id bigint references account(id) not null,
    cas_key_id bigint references cas_key(id) not null,
    accessed timestamptz not null default now(),
    primary key (account_id, cas_key_id, accessed)
);

create index idx_frequency_account_key_recent
    on frequency_account_cas_key(account_id, cas_key_id, accessed desc);
