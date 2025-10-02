-- Organizations
create table organization (
    id bigserial primary key not null,
    name text not null,
    created timestamptz not null default now()
);

-- Users
create table "user" (
    id bigserial primary key not null,
    organization_id bigint references organization(id) not null,
    email text not null unique,
    created timestamptz not null default now()
);

-- API Keys
create table api_key (
    id bigserial primary key not null,
    user_id bigint references "user"(id) not null,
    content bytea not null,
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
create table frequency_user_cas_key (
    user_id bigint references "user"(id) not null,
    cas_key_id bigint references cas_key(id) not null,
    accessed timestamptz not null default now(),
    primary key (user_id, cas_key_id, accessed)
);

create index idx_frequency_user_key_recent
    on frequency_user_cas_key(user_id, cas_key_id, accessed desc);
