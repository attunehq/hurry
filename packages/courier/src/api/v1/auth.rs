use axum::{
    Json, Router,
    routing::{delete, post},
};
use serde::{Deserialize, Serialize};

use crate::api::State;

#[derive(Deserialize)]
pub struct MintJwtRequest {
    org_id: i64,
    api_key: Vec<u8>,
}

#[derive(Serialize)]
pub struct MintJwtResponse {
    jwt: String,
}

pub fn router() -> Router<State> {
    Router::new()
        .route("/", post(mint_jwt))
        .route("/", delete(revoke_jwt))
}

async fn mint_jwt(Json(req): Json<MintJwtRequest>) -> Json<MintJwtResponse> {
    todo!("1. Validate api_key against org_id in database");
    todo!("2. Load top N CAS keys for user into in-memory cache");
    todo!("3. Generate JWT with user_id, org_id, org_secret");
    todo!("4. Store JWT session in database with expiration");
    todo!("5. Return JWT to client");
}

async fn revoke_jwt() -> &'static str {
    todo!("1. Extract JWT from request");
    todo!("2. Validate JWT and extract user_id, org_id");
    todo!("3. Mark session as revoked in database");
    todo!("4. Decrement session count in cache");
    "ok"
}
