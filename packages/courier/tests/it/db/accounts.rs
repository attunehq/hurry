//! Tests for account database operations.

use courier::db::Postgres;
use pretty_assertions::assert_eq as pretty_assert_eq;

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn create_and_get_account(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    // Create an account
    let account_id = db
        .create_account("test@example.com", Some("Test User"))
        .await
        .unwrap();

    // Get the account
    let account = db.get_account(account_id).await.unwrap().unwrap();

    pretty_assert_eq!(account.id, account_id);
    pretty_assert_eq!(account.email, "test@example.com");
    pretty_assert_eq!(account.name.as_deref(), Some("Test User"));
    assert!(account.disabled_at.is_none());
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn create_account_without_name(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let account_id = db.create_account("noname@test.com", None).await.unwrap();

    let account = db.get_account(account_id).await.unwrap().unwrap();

    pretty_assert_eq!(account.name, None);
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn get_nonexistent_account(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let account = db
        .get_account(courier::auth::AccountId::from_i64(99999))
        .await
        .unwrap();

    assert!(account.is_none());
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn update_account_email(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let account_id = db.create_account("old@test.com", None).await.unwrap();

    db.update_account_email(account_id, "new@test.com")
        .await
        .unwrap();

    let account = db.get_account(account_id).await.unwrap().unwrap();
    pretty_assert_eq!(account.email, "new@test.com");
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn update_account_name(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let account_id = db.create_account("test@test.com", None).await.unwrap();

    // Set name
    db.update_account_name(account_id, Some("New Name"))
        .await
        .unwrap();
    let account = db.get_account(account_id).await.unwrap().unwrap();
    pretty_assert_eq!(account.name.as_deref(), Some("New Name"));

    // Clear name
    db.update_account_name(account_id, None).await.unwrap();
    let account = db.get_account(account_id).await.unwrap().unwrap();
    pretty_assert_eq!(account.name, None);
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn disable_and_enable_account(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let account_id = db.create_account("test@test.com", None).await.unwrap();

    // Disable account
    db.disable_account(account_id).await.unwrap();
    let account = db.get_account(account_id).await.unwrap().unwrap();
    assert!(account.disabled_at.is_some());

    // Re-enable account
    db.enable_account(account_id).await.unwrap();
    let account = db.get_account(account_id).await.unwrap().unwrap();
    assert!(account.disabled_at.is_none());
}
