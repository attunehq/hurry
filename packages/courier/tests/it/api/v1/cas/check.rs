//! CAS check (HEAD) endpoint tests.

use color_eyre::Result;
use sqlx::PgPool;

use crate::helpers::{TestFixture, test_blob};

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn check_exists(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let content = b"check exists test";
    let key = test_blob(content);

    fixture
        .client_alice
        .cas_write_bytes(&key, content.to_vec())
        .await?;

    let exists = fixture.client_alice.cas_exists(&key).await?;
    assert!(exists, "blob should exist");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn check_doesnt_exist(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let key = test_blob(b"never written");

    let exists = fixture.client_alice.cas_exists(&key).await?;
    assert!(!exists, "blob should not exist");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn check_then_write_toctou_safety(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let content = b"toctou test";
    let key = test_blob(content);

    let exists_before = fixture.client_alice.cas_exists(&key).await?;
    assert!(!exists_before, "blob should not exist initially");

    fixture
        .client_alice
        .cas_write_bytes(&key, content.to_vec())
        .await?;

    let exists_after = fixture.client_alice.cas_exists(&key).await?;
    assert!(exists_after, "blob should exist after write");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn same_org_users_can_check_each_others_blobs(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let alice_content = b"Alice's data for check";
    let alice_key = test_blob(alice_content);
    fixture
        .client_alice
        .cas_write_bytes(&alice_key, alice_content.to_vec())
        .await?;

    let bob_check_alice = fixture.client_bob.cas_exists(&alice_key).await?;
    assert!(bob_check_alice, "Bob should be able to check Alice's blob");

    let bob_content = b"Bob's data for check";
    let bob_key = test_blob(bob_content);
    fixture
        .client_bob
        .cas_write_bytes(&bob_key, bob_content.to_vec())
        .await?;

    let alice_check_bob = fixture.client_alice.cas_exists(&bob_key).await?;
    assert!(alice_check_bob, "Alice should be able to check Bob's blob");

    Ok(())
}
