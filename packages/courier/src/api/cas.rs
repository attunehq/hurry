use axum::{
    body::Body,
    extract::Path,
    http::StatusCode,
    routing::{get, head, put},
    Router,
};

pub fn routes() -> Router {
    Router::new()
        .route("/:key", head(check_cas))
        .route("/:key", get(read_cas))
        .route("/:key", put(write_cas))
}

async fn check_cas(Path(key): Path<String>) -> StatusCode {
    todo!("1. Validate JWT and extract org_id");
    todo!("2. Check if key exists in CAS storage");
    todo!("3. Return 200 if exists, 404 if not");
}

async fn read_cas(Path(key): Path<String>) -> Body {
    todo!("1. Validate JWT and extract org_id, user_id");
    todo!("2. Check if key is in in-memory cache");
    todo!("3. If not in cache, check database for access");
    todo!("4. If access granted, add to cache");
    todo!("5. Stream blob from CAS storage (decompress zstd)");
    todo!("6. Asynchronously record access frequency");
}

async fn write_cas(Path(key): Path<String>, body: Body) -> StatusCode {
    todo!("1. Validate JWT and extract org_id, user_id");
    todo!("2. Stream body to temporary file");
    todo!("3. Compute blake3 hash while streaming");
    todo!("4. Verify hash matches key parameter");
    todo!("5. Compress with zstd level 3");
    todo!("6. Rename to final location (idempotent)");
    todo!("7. Grant org access to key in database");
    todo!("8. Asynchronously record access frequency");
    StatusCode::CREATED
}
