//! Integration tests for API key management endpoints.

use color_eyre::Result;
use pretty_assertions::assert_eq as pretty_assert_eq;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::helpers::TestFixture;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CreateApiKeyResponse {
    id: i64,
    name: String,
    token: String,
    created_at: String,
}

#[derive(Debug, Serialize)]
struct CreateApiKeyRequest {
    name: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OrgApiKeyListResponse {
    api_keys: Vec<OrgApiKeyEntry>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OrgApiKeyEntry {
    id: i64,
    name: String,
    account_id: i64,
    account_email: String,
    created_at: String,
    accessed_at: String,
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn list_org_api_keys(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/api-keys"))?;

    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::OK);

    let list = response.json::<OrgApiKeyListResponse>().await?;
    // Alice and Bob each have 1 active org-scoped token to Acme (revoked excluded)
    pretty_assert_eq!(list.api_keys.len(), 2);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn list_org_api_keys_non_member_forbidden(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/api-keys"))?;

    // Charlie is not a member of Acme
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(fixture.auth.session_charlie().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn create_org_api_key(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/api-keys"))?;

    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&CreateApiKeyRequest {
            name: String::from("CI/CD Key"),
        })
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::CREATED);

    let key = response.json::<CreateApiKeyResponse>().await?;
    pretty_assert_eq!(key.name, "CI/CD Key");
    assert!(!key.token.is_empty());

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn create_org_api_key_empty_name_fails(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/api-keys"))?;

    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&CreateApiKeyRequest {
            name: String::from("  "),
        })
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn create_org_api_key_member_can_create(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/api-keys"))?;

    // Bob is a member (not admin) of Acme
    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(fixture.auth.session_bob().expose())
        .json(&CreateApiKeyRequest {
            name: String::from("Bob's Key"),
        })
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::CREATED);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn create_org_api_key_non_member_forbidden(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/api-keys"))?;

    // Charlie is not a member of Acme
    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(fixture.auth.session_charlie().expose())
        .json(&CreateApiKeyRequest {
            name: String::from("Charlie's Key"),
        })
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn delete_org_api_key_owner_can_delete(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();

    // Bob creates a key
    let create_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/api-keys"))?;
    let create_response = reqwest::Client::new()
        .post(create_url)
        .bearer_auth(fixture.auth.session_bob().expose())
        .json(&CreateApiKeyRequest {
            name: String::from("Bob's Key"),
        })
        .send()
        .await?;

    let key = create_response.json::<CreateApiKeyResponse>().await?;

    // Bob deletes his own key
    let delete_url = fixture.base_url.join(&format!(
        "api/v1/organizations/{org_id}/api-keys/{}",
        key.id
    ))?;
    let delete_response = reqwest::Client::new()
        .delete(delete_url)
        .bearer_auth(fixture.auth.session_bob().expose())
        .send()
        .await?;

    pretty_assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn delete_org_api_key_admin_can_delete_any(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();

    // Bob creates a key
    let create_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/api-keys"))?;
    let create_response = reqwest::Client::new()
        .post(create_url)
        .bearer_auth(fixture.auth.session_bob().expose())
        .json(&CreateApiKeyRequest {
            name: String::from("Bob's Key"),
        })
        .send()
        .await?;

    let key = create_response.json::<CreateApiKeyResponse>().await?;

    // Alice (admin) deletes Bob's key
    let delete_url = fixture.base_url.join(&format!(
        "api/v1/organizations/{org_id}/api-keys/{}",
        key.id
    ))?;
    let delete_response = reqwest::Client::new()
        .delete(delete_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;

    pretty_assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn delete_org_api_key_member_cannot_delete_others(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();

    // Alice creates a key
    let create_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/api-keys"))?;
    let create_response = reqwest::Client::new()
        .post(create_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&CreateApiKeyRequest {
            name: String::from("Alice's Key"),
        })
        .send()
        .await?;

    let key = create_response.json::<CreateApiKeyResponse>().await?;

    // Bob (member, not admin) tries to delete Alice's key
    let delete_url = fixture.base_url.join(&format!(
        "api/v1/organizations/{org_id}/api-keys/{}",
        key.id
    ))?;
    let delete_response = reqwest::Client::new()
        .delete(delete_url)
        .bearer_auth(fixture.auth.session_bob().expose())
        .send()
        .await?;

    pretty_assert_eq!(delete_response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn delete_org_api_key_wrong_org_not_found(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_acme = fixture.auth.org_acme().as_i64();
    let org_widget = fixture.auth.org_widget().as_i64();

    // Alice creates a key in Acme
    let create_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_acme}/api-keys"))?;
    let create_response = reqwest::Client::new()
        .post(create_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&CreateApiKeyRequest {
            name: String::from("Acme Key"),
        })
        .send()
        .await?;

    let key = create_response.json::<CreateApiKeyResponse>().await?;

    // Charlie tries to delete it via Widget org (should 404, not reveal it exists)
    let delete_url = fixture.base_url.join(&format!(
        "api/v1/organizations/{org_widget}/api-keys/{}",
        key.id
    ))?;
    let delete_response = reqwest::Client::new()
        .delete(delete_url)
        .bearer_auth(fixture.auth.session_charlie().expose())
        .send()
        .await?;

    pretty_assert_eq!(delete_response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn delete_org_api_key_not_found(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/api-keys/99999"))?;

    let response = reqwest::Client::new()
        .delete(url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}
