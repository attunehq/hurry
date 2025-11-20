//! CAS read endpoint tests.

use color_eyre::Result;
use pretty_assertions::assert_eq as pretty_assert_eq;
use sqlx::PgPool;

use crate::helpers::{TestFixture, test_blob};

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn read_after_write(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let content = b"read test content";
    let key = test_blob(content);

    fixture
        .client_alice
        .cas_write_bytes(&key, content.to_vec())
        .await?;

    let read = fixture
        .client_alice
        .cas_read_bytes(&key)
        .await?
        .expect("blob should exist");
    pretty_assert_eq!(read.as_slice(), content);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn read_nonexistent_key(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let key = test_blob(b"never written");

    let read = fixture.client_alice.cas_read_bytes(&key).await?;
    assert!(read.is_none(), "blob should not exist");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn read_large_blob(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let content = vec![0xFF; 5 * 1024 * 1024];
    let key = test_blob(&content);

    fixture
        .client_alice
        .cas_write_bytes(&key, content.clone())
        .await?;

    let read = fixture
        .client_alice
        .cas_read_bytes(&key)
        .await?
        .expect("blob should exist");
    pretty_assert_eq!(read.len(), content.len());
    pretty_assert_eq!(read.as_slice(), content.as_slice());

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn read_missing_auth_returns_401(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let content = b"test blob";
    let key = test_blob(content);

    fixture
        .client_alice
        .cas_write_bytes(&key, content.to_vec())
        .await?;

    let client_no_auth = fixture.client_with_token("")?;
    let result = client_no_auth.cas_read_bytes(&key).await;
    assert!(result.is_err(), "read without auth should fail");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn read_invalid_token_returns_401(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let content = b"test blob";
    let key = test_blob(content);

    fixture
        .client_alice
        .cas_write_bytes(&key, content.to_vec())
        .await?;

    let client = fixture.client_with_token("invalid-token-that-does-not-exist")?;
    let result = client.cas_read_bytes(&key).await;
    assert!(result.is_err(), "read with invalid token should fail");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn read_revoked_token_returns_401(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let content = b"test blob";
    let key = test_blob(content);

    fixture
        .client_alice
        .cas_write_bytes(&key, content.to_vec())
        .await?;

    let client = fixture.client_with_token(fixture.auth.token_alice_revoked().expose())?;
    let result = client.cas_read_bytes(&key).await;
    assert!(result.is_err(), "read with revoked token should fail");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn org_cannot_read_other_orgs_blob(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let content = b"org A private data";
    let key = test_blob(content);

    fixture
        .client_alice
        .cas_write_bytes(&key, content.to_vec())
        .await?;

    let read_charlie = fixture.client_charlie.cas_read_bytes(&key).await?;
    assert!(
        read_charlie.is_none(),
        "org B should not be able to read org A's blob"
    );

    let read_alice = fixture
        .client_alice
        .cas_read_bytes(&key)
        .await?
        .expect("org A should be able to read its own blob");
    pretty_assert_eq!(read_alice.as_slice(), content);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn same_content_different_orgs_separate_access(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let content = b"shared content";
    let key = test_blob(content);

    fixture
        .client_alice
        .cas_write_bytes(&key, content.to_vec())
        .await?;

    let read_charlie_before = fixture.client_charlie.cas_read_bytes(&key).await?;
    assert!(
        read_charlie_before.is_none(),
        "org B should not have access yet"
    );

    fixture
        .client_charlie
        .cas_write_bytes(&key, content.to_vec())
        .await?;

    let read_charlie_after = fixture
        .client_charlie
        .cas_read_bytes(&key)
        .await?
        .expect("org B should have access after writing");
    pretty_assert_eq!(read_charlie_after.as_slice(), content);

    let read_alice = fixture
        .client_alice
        .cas_read_bytes(&key)
        .await?
        .expect("org A should still have access");
    pretty_assert_eq!(read_alice.as_slice(), content);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn same_org_users_can_access_each_others_blobs(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let alice_content = b"Alice's data";
    let alice_key = test_blob(alice_content);
    fixture
        .client_alice
        .cas_write_bytes(&alice_key, alice_content.to_vec())
        .await?;

    let bob_read_alice = fixture
        .client_bob
        .cas_read_bytes(&alice_key)
        .await?
        .expect("Bob should be able to read Alice's blob");
    pretty_assert_eq!(bob_read_alice.as_slice(), alice_content);

    let bob_content = b"Bob's data";
    let bob_key = test_blob(bob_content);
    fixture
        .client_bob
        .cas_write_bytes(&bob_key, bob_content.to_vec())
        .await?;

    let alice_read_bob = fixture
        .client_alice
        .cas_read_bytes(&bob_key)
        .await?
        .expect("Alice should be able to read Bob's blob");
    pretty_assert_eq!(alice_read_bob.as_slice(), bob_content);

    Ok(())
}
