//! Tests for API key database operations.

use courier::db::Postgres;
use pretty_assertions::assert_eq as pretty_assert_eq;

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn create_api_key(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Org").await.unwrap();
    let account_id = db.create_account("test@test.com", None).await.unwrap();

    let (key_id, token) = db
        .create_api_key(account_id, "Test Key", org_id)
        .await
        .unwrap();

    // Verify the key was created
    let key = db.get_api_key(key_id).await.unwrap().unwrap();

    pretty_assert_eq!(key.id, key_id);
    pretty_assert_eq!(key.account_id, account_id);
    pretty_assert_eq!(key.organization_id, org_id);
    pretty_assert_eq!(key.name, "Test Key");
    assert!(key.revoked_at.is_none());

    // Token should be 32 hex chars (16 bytes)
    pretty_assert_eq!(token.expose().len(), 32);
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn list_org_api_keys(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org1_id = db.create_organization("Org 1").await.unwrap();
    let org2_id = db.create_organization("Org 2").await.unwrap();
    let account_id = db.create_account("test@test.com", None).await.unwrap();

    // Create keys for different orgs
    db.create_api_key(account_id, "Org1 Key 1", org1_id)
        .await
        .unwrap();
    db.create_api_key(account_id, "Org1 Key 2", org1_id)
        .await
        .unwrap();
    db.create_api_key(account_id, "Org2 Key", org2_id)
        .await
        .unwrap();

    let org1_keys = db.list_org_api_keys(account_id, org1_id).await.unwrap();
    let org2_keys = db.list_org_api_keys(account_id, org2_id).await.unwrap();

    pretty_assert_eq!(org1_keys.len(), 2);
    pretty_assert_eq!(org2_keys.len(), 1);
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn revoke_api_key(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Org").await.unwrap();
    let account_id = db.create_account("test@test.com", None).await.unwrap();

    let (key_id, _token) = db
        .create_api_key(account_id, "Test Key", org_id)
        .await
        .unwrap();

    // Revoke
    let revoked = db.revoke_api_key(key_id).await.unwrap();
    assert!(revoked);

    // Should have revoked_at set
    let key = db.get_api_key(key_id).await.unwrap().unwrap();
    assert!(key.revoked_at.is_some());

    // Should not appear in org key list
    let keys = db.list_org_api_keys(account_id, org_id).await.unwrap();
    assert!(keys.is_empty());

    // Revoking again returns false
    let revoked_again = db.revoke_api_key(key_id).await.unwrap();
    assert!(!revoked_again);
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn get_nonexistent_api_key(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let key = db
        .get_api_key(courier::auth::ApiKeyId::from_i64(99999))
        .await
        .unwrap();

    assert!(key.is_none());
}
