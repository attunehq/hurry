use axum::Json;
use serde::Serialize;

use crate::auth::{OrgId, StatelessToken, UserId};

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

#[cfg(test)]
mod tests {
    use color_eyre::{Result, eyre::Context};
    use pretty_assertions::assert_eq as pretty_assert_eq;
    use serde_json::{Value, json};
    use sqlx::PgPool;

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn test_validate_stateless_token(pool: PgPool) -> Result<()> {
        const TOKEN: &str = "test-token:user1@test1.com";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let response = server
            .post("/api/v1/auth")
            .add_header("Authorization", format!("Bearer {TOKEN}"))
            .add_header("x-org-id", "1")
            .await;

        response.assert_status_ok();
        let body = response.json::<Value>();
        let token = body["token"].as_str().expect("token as a string");

        let check = server
            .get("/api/v1/auth")
            .add_header("Authorization", token)
            .await;
        check.assert_status_ok();

        let metadata = check.json::<Value>();
        pretty_assert_eq!(
            metadata,
            json!({
                "org_id": 1,
                "user_id": 1
            })
        );

        Ok(())
    }
}
