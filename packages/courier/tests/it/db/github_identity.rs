//! Tests for GitHub identity database operations.

use courier::db::Postgres;
use pretty_assertions::assert_eq as pretty_assert_eq;

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn link_and_get_github_identity(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let _org_id = db.create_organization("Test Org").await.unwrap();
    let account_id = db.create_account("test@test.com", None).await.unwrap();

    // Link GitHub identity
    db.link_github_identity(account_id, 12345, "testuser")
        .await
        .unwrap();

    // Get identity
    let identity = db.get_github_identity(account_id).await.unwrap().unwrap();

    pretty_assert_eq!(identity.account_id, account_id);
    pretty_assert_eq!(identity.github_user_id, 12345);
    pretty_assert_eq!(identity.github_username, "testuser");
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn get_account_by_github_id(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let _org_id = db.create_organization("Test Org").await.unwrap();
    let account_id = db
        .create_account("test@test.com", Some("Test User"))
        .await
        .unwrap();

    db.link_github_identity(account_id, 67890, "githubuser")
        .await
        .unwrap();

    // Look up account by GitHub ID
    let account = db.get_account_by_github_id(67890).await.unwrap().unwrap();

    pretty_assert_eq!(account.id, account_id);
    pretty_assert_eq!(account.email, "test@test.com");
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn get_account_by_nonexistent_github_id(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let account = db.get_account_by_github_id(99999).await.unwrap();

    assert!(account.is_none());
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn get_nonexistent_github_identity(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let _org_id = db.create_organization("Test Org").await.unwrap();
    let account_id = db.create_account("test@test.com", None).await.unwrap();

    // Account exists but has no GitHub identity
    let identity = db.get_github_identity(account_id).await.unwrap();

    assert!(identity.is_none());
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn update_github_username(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let _org_id = db.create_organization("Test Org").await.unwrap();
    let account_id = db.create_account("test@test.com", None).await.unwrap();

    db.link_github_identity(account_id, 12345, "oldname")
        .await
        .unwrap();

    // Update username
    db.update_github_username(account_id, "newname")
        .await
        .unwrap();

    let identity = db.get_github_identity(account_id).await.unwrap().unwrap();
    pretty_assert_eq!(identity.github_username, "newname");
}
