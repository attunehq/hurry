# RFC 0001: Courier - Content-Addressed Storage Service

## Overview

Courier is hurry's content-addressed storage (CAS) service. It stores build artifacts indexed by the Blake3 hash of their content, enabling fast restoration of cached build outputs across git branches, worktrees, and development contexts for multiple users in an org.

The service is designed for extremely bursty traffic patterns with many small requests that must be served with minimal latency. Multiple organizations and users can share the same service while maintaining proper isolation and access controls.

## Design Principles

### Performance first

Latency and throughput are paramount. To minimize latency and maximize throughput:

- Client and server negotiate minting a JWT for use for the session once; this minimizes database round trips.
- Bookkeeping operations such as access tracking are updated asynchronously
- The ingress routes clients to the same backends depending on their org to maximize local service caching
- Clients are never redirected to external storage like S3

### Multi-Tenant Architecture

Organizations are the primary unit of isolation. Multiple users within an org (including CI bots) share visibility into the same cached content. Blobs are deduplicated across organizations but access is strictly controlled: an org can only access a blob after storing it themselves. In other words, deduplication across orgs must appear to users as if it isn't happening; this is strictly an implementation optimization in the backend.

## Technology Stack

- Serialization: MsgPack
- Web framework: Axum
- Database: AWS Aurora (Postgres)
- Blob storage: AWS EFS

## Client Flow

1. Client authenticates and mints a JWT on startup
2. Client requests CAS objects by Blake3 hash
3. Client performs local build
4. Client stores new or updated CAS objects back to Courier

> [!IMPORTANT]
> Clients always include an `org_id` in the request (we'll probably implement this with a header). The ingress uses this to always route the request to the correct backend. The backend then validates that the `org_id` in the request is actually correct before serving anything. This is critical to understand for the rest of this design.

## Authentication

Courier clients are primarily authenticated through a standard API key; they then negotiate a JWT at startup and use that.

### JWT Negotiation

When a client starts a session, it requests a JWT:

1. Client provides `org_id` and `api_key`
2. Server validates those credentials and mints a JWT containing `account_id` and `org_id`
3. Server preloads the account's N most frequently accessed CAS keys into the in-memory set `K` for the org
4. Server returns the JWT to the client

### CAS Key Preloading

The N most frequently accessed CAS keys for each active account session are stored in memory, looking something like this:
```rust
// Pseudocode
static ACCOUNT_KEYS = LockfreeHashMap<OrgId, LruHashSet<N, Blake3>>;
```

The intention here is that when an account negotiates a JWT, the server then loads "these are the CAS keys the account will likely access". Hopefully we'll be able to make N large enough that effectively all requests hit this preloaded set. Doing this allows us to then validate whether the account has access in subsequent requests without actually hitting the database to check permissions.

If the account requests a key that isn't in the set, the server falls back to checking the database for whether the org has access to that key; if so the request is served as normal and also the key is inserted into the set.

Different accounts with the same org get routed to the same backend instance and therefore reference the same set; these sets are additive until all active sessions are dropped and/or time out at which point the entire set is dropped.

> [!NOTE]
> To start with we'll set `N` to `100,000`:
> - 100 active org sessions on the server
> - 100,000 keys in the set
> - Not accounting for overhead, that's ~320 MiB of memory
> - Even after overhead should easily fit in 1 GiB, which is trivial for an API server
>
> In the future we may make this dynamic or increase it.

### Read Authorization

Read requests are important because we fundamentally cannot allow users to read content they haven't written.

On each read request:

1. Validate JWT (expiration, signature, `org_id` in the JWT matches the one in the request header)
2. Read CAS key `k` from the request.
3. Check if `k` is in the set `K` for the org (this is the set of "known allowed" keys)
4. If `k` is not in `K`, check database for access rights; if successful update `K` otherwise reject request
5. Serve the blob and asynchronously update access frequency

> [!TIP]
> If the server crashes or reboots while this is happening, performance gracefully degrades instead of the overall system failing. In the future we may add a way to recover from this happening.

### Write Authorization

Writes are simpler: if the user has a valid JWT, they can write new content.

1. Validate JWT (expiration, signature, `org_id` in the JWT matches the one in the request header)
2. Write the content to blob storage
3. Write access rights to the database

### JWT Revocation

Each JWT has a set expiration time which is tracked server-side so that the server can clean up session resources once it expires. Clients ideally also notify the server when they're done with the JWT by hitting a revocation endpoint, which immediately triggers cleanup.

The specifics of how we'll track this depends on the specifics of our JWT implementation, so I've left off any pseudocode examples.

## Database Schema

> [!NOTE]
> The schemas below are minimal; we may add other things to the tables. For example, CRUD dates or usernames or whatever.

### Organizations

Organizations are the top-level entity.

```sql
create table organization (
    id bigserial primary key not null,
    name text not null
)
```

### Accounts

Accounts belong to a single organization. Each account within an org shares access to the same cached content. We still track them separately so that we can maintain different frequency per account - for example, we expect CI to have a very different frequency of key access than a developer on their local machine. For the service, there is no difference between a bot account and a human account.

```sql
create table account (
    id bigserial primary key not null,
    organization_id bigint references organization.id not null
)
```

### API Keys

API keys are scoped to individual accounts for tracking and revocation purposes.

```sql
create table api_key (
    id bigserial primary key not null,
    account_id bigint references account.id not null,
    content bytea not null,
    unique(content)
)
```

### CAS Key Index

Index for deduplicating multiple references to the same CAS key. CAS keys are Blake3 hashes, which are 32 bytes long; this is somewhat large to reference over and over again compared to e.g. a 64 bit integer. This index is **not authoritative**: the authoritative way to know whether a key is in the CAS is to check the actual CAS on disk.

```sql
create table cas_key (
    id bigserial primary key not null,
    content bytea not null,
    unique(content)
)
```

### Access Control

Blobs are deduplicated globally but access is per-organization. An org can only access a blob after storing it themselves.

```sql
create table cas_access (
    org_id bigint references organization.id not null,
    cas_key_id bigint references cas_key.id not null,
    primary key (org_id, cas_key_id)
)
```

### Frequency Tracking

We want to store frequency of CAS key usage per account.

```sql
create table frequency_account_cas_key (
  account_id bigint references account.id not null,
  cas_key_id bigint references cas_key.id not null,
  accessed timestamptz not null default now(),
  primary key (account_id, cas_key_id, accessed)
)
```

So checking frequency over a given time period is just "count the rows in a given time period".

We'll add an index to support this:
```sql
create index idx_frequency_account_key_recent
    on frequency_account_cas_key(account_id, cas_key_id, accessed desc);
```

The query for which will look something like this:
```sql
SELECT cas_key_id, COUNT(*) as freq
FROM frequency_account_cas_key
WHERE account_id = ? AND accessed > now() - interval '7 days'
GROUP BY cas_key_id
ORDER BY freq DESC
LIMIT 100000;
```

We'll control the size of the table by removing old records on a timeframe; codebase CAS keys are assumed to be subject to pretty extreme recency bias so we'll probably keep this pretty short, like 7 days or less. If an account isn't active for 7 days the assumption is that the fallback methods of "basing their frequency on the rest of their org" is likely the best bet: they probably went on vacation and are coming back and starting something new.

## Content-Addressed Storage

### Storage Model

The CAS is conceptually simple: a map from Blake3 hash to blob content.

```rust
struct CasEntry(Vec<u8>);
struct Cas(HashMap<Blake3, CasEntry>);
```

Blobs are stored directly to disk; we're assuming Amazon EFS here so that multiple servers can share the same disk. The actual blob content is also compressed with `zstd`; note that the key is computed from the _raw_ content and not the compressed content.

Blobs are organized pretty simply with two levels of folder prefixes:
- Top level is named the first two characters of the hex representation of the Blake3 hash
- Second level is named the second two characters of the hex representation of the Blake3 hash
- Third level is the blob itself, in a file named the hex representation of the Blake3 hash

For example:
```
cas/
  ab/
    cd/
      abcd1234...
      abcd5678...
  wx/
    yz/
      wxyz0987...
```

The reasoning for this is just to keep folder sizes relatively small.

### Upserts

Multiple API instances could write to the same blob, but all blobs are functionally idempotent: we name them with the hash of their content so even if we were to write multiple times the content will always be the same. Given this we follow the below approach:

1. Check if the blob already exists; if so do nothing.
2. Write the blob to a temporary location on disk.
3. Rename the blob to its final destination; if a "file already exists" error occurs assume another instance wrote it.

We do this because a rename is an idempotent operation; writing multiple chunks is not. This also allows us to support streaming on the API side: we can store the content to disk and compute its key as we write it out, then move it to its final destination based on that key.

## Endpoints

In the future we may add batching endpoints but will wait to add that until later. Today we will just have "get" and "set" endpoints for the CAS operations:
- `HEAD /api/v1/cas/:key`: Check if content for this key already exists (to avoid re-uploading)
- `GET /api/v1/cas/:key`: Stream the content (chunked transfer encoding) from the CAS with the provided key.
- `PUT /api/v1/cas/:key`: Stream the content (chunked transfer encoding) from the body into the CAS. The server validates the key is correct and rejects the request without saving the data if not.

Otherwise we just have auth endpoints which operate as described in the auth section:
- `POST /api/v1/auth`: Negotiate a new JWT.
- `DELETE /api/v1/auth`: Revoke the JWT provided in the request.

And of course we'll have the usual suspects for org management, health checks, etc.

### API health

Instead of explicitly rate limiting users, we'll just enforce that each API server:
- Has a max request deadline: starting with 15 seconds
- Has a max number of requests in flight: starting with 1,000
- Has a small queue of requests that are pending: starting with 100
- Has a max body size limit: starting with 100MiB
- Authenticates JWTs for 15 minutes: clients can always get a new JWT

> [!TIP]
> It's always easier to increase limits than to reduce them, so we're starting relatively small.

### Monitoring and metrics

None for now, we'll just review the server traces in kubernetes. We'll add these if we keep the product.
