//! Tests for session database operations.

use courier::{auth::SessionToken, db::Postgres};
use pretty_assertions::assert_eq as pretty_assert_eq;
use time::{Duration, OffsetDateTime};

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn create_and_validate_session(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let account_id = db.create_account("test@test.com", None).await.unwrap();

    let token = SessionToken::generate();
    let expires_at = OffsetDateTime::now_utc() + Duration::hours(24);

    let _session_id = db
        .create_session(account_id, &token, expires_at)
        .await
        .unwrap();

    // Validate the session
    let context = db.validate_session(&token).await.unwrap().unwrap();

    pretty_assert_eq!(context.account_id, account_id);
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn validate_expired_session(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let account_id = db.create_account("test@test.com", None).await.unwrap();

    let token = SessionToken::generate();
    let expires_at = OffsetDateTime::now_utc() - Duration::hours(1); // Already expired

    db.create_session(account_id, &token, expires_at)
        .await
        .unwrap();

    // Should fail validation because it's expired
    let context = db.validate_session(&token).await.unwrap();

    assert!(context.is_none());
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn validate_disabled_account_session(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let account_id = db.create_account("test@test.com", None).await.unwrap();

    let token = SessionToken::generate();
    let expires_at = OffsetDateTime::now_utc() + Duration::hours(24);

    db.create_session(account_id, &token, expires_at)
        .await
        .unwrap();

    // Disable the account
    db.disable_account(account_id).await.unwrap();

    // Should fail validation because account is disabled
    let context = db.validate_session(&token).await.unwrap();

    assert!(context.is_none());
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn validate_invalid_session(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let token = SessionToken::new("invalid_token");

    let context = db.validate_session(&token).await.unwrap();

    assert!(context.is_none());
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn revoke_session(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let account_id = db.create_account("test@test.com", None).await.unwrap();

    let token = SessionToken::generate();
    let expires_at = OffsetDateTime::now_utc() + Duration::hours(24);

    db.create_session(account_id, &token, expires_at)
        .await
        .unwrap();

    // Revoke the session
    let revoked = db.revoke_session(&token).await.unwrap();
    assert!(revoked);

    // Should no longer be valid
    let context = db.validate_session(&token).await.unwrap();
    assert!(context.is_none());
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn revoke_nonexistent_session(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let token = SessionToken::new("nonexistent");

    let revoked = db.revoke_session(&token).await.unwrap();

    assert!(!revoked);
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn revoke_all_sessions(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let account_id = db.create_account("test@test.com", None).await.unwrap();

    let expires_at = OffsetDateTime::now_utc() + Duration::hours(24);

    // Create multiple sessions
    let token1 = SessionToken::generate();
    let token2 = SessionToken::generate();

    db.create_session(account_id, &token1, expires_at)
        .await
        .unwrap();
    db.create_session(account_id, &token2, expires_at)
        .await
        .unwrap();

    // Revoke all
    let count = db.revoke_all_sessions(account_id).await.unwrap();

    pretty_assert_eq!(count, 2);

    // Both should be invalid
    assert!(db.validate_session(&token1).await.unwrap().is_none());
    assert!(db.validate_session(&token2).await.unwrap().is_none());
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn extend_session(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let account_id = db.create_account("test@test.com", None).await.unwrap();

    let token = SessionToken::generate();
    let initial_expires = OffsetDateTime::now_utc() + Duration::hours(1);

    db.create_session(account_id, &token, initial_expires)
        .await
        .unwrap();

    // Extend the session
    let new_expires = OffsetDateTime::now_utc() + Duration::hours(48);
    let extended = db.extend_session(&token, new_expires).await.unwrap();

    assert!(extended);

    // Session should still be valid
    let context = db.validate_session(&token).await.unwrap();
    assert!(context.is_some());
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn cleanup_expired_sessions(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let account_id = db.create_account("test@test.com", None).await.unwrap();

    // Create expired session
    let expired_token = SessionToken::generate();
    db.create_session(
        account_id,
        &expired_token,
        OffsetDateTime::now_utc() - Duration::hours(1),
    )
    .await
    .unwrap();

    // Create valid session
    let valid_token = SessionToken::generate();
    db.create_session(
        account_id,
        &valid_token,
        OffsetDateTime::now_utc() + Duration::hours(1),
    )
    .await
    .unwrap();

    // Cleanup
    let deleted = db.cleanup_expired_sessions().await.unwrap();

    pretty_assert_eq!(deleted, 1);

    // Valid session should still work
    assert!(db.validate_session(&valid_token).await.unwrap().is_some());
}
