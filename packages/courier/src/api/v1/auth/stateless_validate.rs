use axum::Json;
use serde::Serialize;

use crate::auth::{AuthenticatedStatelessToken, OrgId, UserId};

#[derive(Debug, Serialize)]
pub struct StatelessTokenMetadata {
    pub org_id: OrgId,
    pub user_id: UserId,
}

pub async fn handle(token: AuthenticatedStatelessToken) -> Json<StatelessTokenMetadata> {
    Json(StatelessTokenMetadata {
        org_id: token.org_id,
        user_id: token.user_id,
    })
}
