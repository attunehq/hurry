use axum::Json;
use serde::Serialize;

use crate::auth::{AccountId, OrgId, StatelessToken};

#[derive(Debug, Serialize)]
pub struct StatelessTokenMetadata {
    pub org_id: OrgId,
    pub account_id: AccountId,
}

/// Validates a stateless token and returns the org and account IDs parsed from the
/// token. This endpoint is mainly intended for debugging/validating that the
/// client token implementation is working correctly.
#[tracing::instrument]
pub async fn handle(token: StatelessToken) -> Json<StatelessTokenMetadata> {
    Json(StatelessTokenMetadata {
        org_id: token.org_id,
        account_id: token.account_id,
    })
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use color_eyre::{Result, eyre::Context};
    use pretty_assertions::assert_eq as pretty_assert_eq;
    use serde_json::{Value, json};
    use sqlx::PgPool;

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn validate_stateless_token_happy_path(pool: PgPool) -> Result<()> {
        const TOKEN: &str = "test-api-key-account1-org1";
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
                "account_id": 1
            })
        );

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn validate_with_invalid_stateless_token(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let response = server
            .get("/api/v1/auth")
            .add_header("Authorization", "invalid-stateless-token")
            .await;

        response.assert_status(StatusCode::UNAUTHORIZED);

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn validate_with_missing_authorization(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let response = server.get("/api/v1/auth").await;

        response.assert_status(StatusCode::UNAUTHORIZED);

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn validate_with_empty_authorization(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let response = server
            .get("/api/v1/auth")
            .add_header("Authorization", "")
            .await;

        response.assert_status(StatusCode::BAD_REQUEST);

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn validate_with_bearer_prefix_on_stateless(pool: PgPool) -> Result<()> {
        const TOKEN: &str = "test-api-key-account1-org1";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        // Mint a stateless token
        let mint_response = server
            .post("/api/v1/auth")
            .add_header("Authorization", format!("Bearer {TOKEN}"))
            .add_header("x-org-id", "1")
            .await;
        mint_response.assert_status_ok();
        let body = mint_response.json::<Value>();
        let token = body["token"].as_str().expect("token as a string");

        // Validate with Bearer prefix (should work)
        let check = server
            .get("/api/v1/auth")
            .add_header("Authorization", format!("Bearer {token}"))
            .await;
        check.assert_status_ok();

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn validate_token_from_different_org(pool: PgPool) -> Result<()> {
        const ORG1_TOKEN: &str = "test-api-key-account1-org1";
        const ORG2_TOKEN: &str = "test-api-key-account1-org2";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        // Mint token for org 1
        let org1_response = server
            .post("/api/v1/auth")
            .add_header("Authorization", format!("Bearer {ORG1_TOKEN}"))
            .add_header("x-org-id", "1")
            .await;
        org1_response.assert_status_ok();
        let org1_body = org1_response.json::<Value>();
        let org1_stateless = org1_body["token"].as_str().expect("token as a string");

        // Mint token for org 2
        let org2_response = server
            .post("/api/v1/auth")
            .add_header("Authorization", format!("Bearer {ORG2_TOKEN}"))
            .add_header("x-org-id", "2")
            .await;
        org2_response.assert_status_ok();
        let org2_body = org2_response.json::<Value>();
        let org2_stateless = org2_body["token"].as_str().expect("token as a string");

        // Validate org1 token returns org 1
        let check1 = server
            .get("/api/v1/auth")
            .add_header("Authorization", org1_stateless)
            .await;
        check1.assert_status_ok();
        let metadata1 = check1.json::<Value>();
        pretty_assert_eq!(metadata1["org_id"], 1);

        // Validate org2 token returns org 2
        let check2 = server
            .get("/api/v1/auth")
            .add_header("Authorization", org2_stateless)
            .await;
        check2.assert_status_ok();
        let metadata2 = check2.json::<Value>();
        pretty_assert_eq!(metadata2["org_id"], 2);

        Ok(())
    }
}
