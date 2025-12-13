//! Tests for OAuth state database operations.

use courier::{crypto, db::Postgres};
use pretty_assertions::assert_eq as pretty_assert_eq;
use time::{Duration, OffsetDateTime};

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn store_and_consume_oauth_state(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let state_token = crypto::generate_oauth_state();
    let pkce = crypto::generate_pkce();
    let redirect_uri = "https://example.com/callback";
    let expires_at = OffsetDateTime::now_utc() + Duration::minutes(10);

    // Store state
    db.store_oauth_state(&state_token, &pkce.verifier, redirect_uri, expires_at)
        .await
        .unwrap();

    // Consume state
    let state = db.consume_oauth_state(&state_token).await.unwrap().unwrap();

    pretty_assert_eq!(state.state_token, state_token);
    pretty_assert_eq!(state.pkce_verifier, pkce.verifier);
    pretty_assert_eq!(state.redirect_uri, redirect_uri);
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn consume_state_is_atomic(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let state_token = crypto::generate_oauth_state();
    let pkce = crypto::generate_pkce();
    let expires_at = OffsetDateTime::now_utc() + Duration::minutes(10);

    db.store_oauth_state(
        &state_token,
        &pkce.verifier,
        "https://example.com",
        expires_at,
    )
    .await
    .unwrap();

    // First consume succeeds
    let state = db.consume_oauth_state(&state_token).await.unwrap();
    assert!(state.is_some());

    // Second consume fails (state is gone)
    let state = db.consume_oauth_state(&state_token).await.unwrap();
    assert!(state.is_none());
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn consume_expired_state_fails(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let state_token = crypto::generate_oauth_state();
    let pkce = crypto::generate_pkce();
    let expires_at = OffsetDateTime::now_utc() - Duration::minutes(1); // Already expired

    db.store_oauth_state(
        &state_token,
        &pkce.verifier,
        "https://example.com",
        expires_at,
    )
    .await
    .unwrap();

    // Should fail because expired
    let state = db.consume_oauth_state(&state_token).await.unwrap();

    assert!(state.is_none());
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn consume_nonexistent_state(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let state = db.consume_oauth_state("nonexistent_state").await.unwrap();

    assert!(state.is_none());
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn cleanup_expired_oauth_state(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let expired_state = crypto::generate_oauth_state();
    let valid_state = crypto::generate_oauth_state();
    let pkce = crypto::generate_pkce();

    // Create expired state
    db.store_oauth_state(
        &expired_state,
        &pkce.verifier,
        "https://example.com",
        OffsetDateTime::now_utc() - Duration::minutes(1),
    )
    .await
    .unwrap();

    // Create valid state
    db.store_oauth_state(
        &valid_state,
        &pkce.verifier,
        "https://example.com",
        OffsetDateTime::now_utc() + Duration::minutes(10),
    )
    .await
    .unwrap();

    // Cleanup
    let deleted = db.cleanup_expired_oauth_state().await.unwrap();

    pretty_assert_eq!(deleted, 1);

    // Valid state should still be consumable
    let state = db.consume_oauth_state(&valid_state).await.unwrap();
    assert!(state.is_some());
}
