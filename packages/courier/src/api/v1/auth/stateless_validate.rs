use axum::Json;
use serde::Serialize;

use crate::auth::{StatelessToken, OrgId, UserId};

#[derive(Debug, Serialize)]
pub struct StatelessTokenMetadata {
    pub org_id: OrgId,
    pub user_id: UserId,
}

/// Validates a stateless token and returns the org and user IDs parsed from the
/// token. This endpoint is mainly intended for debugging/validating that the
/// client token implementation is working correctly.
pub async fn handle(token: StatelessToken) -> Json<StatelessTokenMetadata> {
    Json(StatelessTokenMetadata {
        org_id: token.org_id,
        user_id: token.user_id,
    })
}
