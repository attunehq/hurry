//! CAS write endpoint tests.

use color_eyre::Result;
use pretty_assertions::assert_eq as pretty_assert_eq;
use sqlx::PgPool;

use crate::helpers::{TestFixture, test_blob};

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn basic_write_flow(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let content = b"hello world";
    let key = test_blob(content);

    fixture
        .client_alice
        .cas_write_bytes(&key, content.to_vec())
        .await?;

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn idempotent_writes(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let content = b"idempotent test";
    let key = test_blob(content);

    fixture
        .client_alice
        .cas_write_bytes(&key, content.to_vec())
        .await?;
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
async fn idempotent_write_large_blob(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let content = vec![0xCD; 2 * 1024 * 1024];
    let key = test_blob(&content);

    fixture
        .client_alice
        .cas_write_bytes(&key, content.clone())
        .await?;
    fixture
        .client_alice
        .cas_write_bytes(&key, content.clone())
        .await?;

    let read = fixture
        .client_alice
        .cas_read_bytes(&key)
        .await?
        .expect("blob should exist");
    pretty_assert_eq!(read.as_slice(), content.as_slice());

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn invalid_key_hash(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let wrong_key = test_blob(b"different content");
    let actual_content = b"actual content";

    let result = fixture
        .client_alice
        .cas_write_bytes(&wrong_key, actual_content.to_vec())
        .await;
    assert!(result.is_err(), "write with wrong hash should fail");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn large_blob_write(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let content = vec![0xAB; 1024 * 1024];
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
async fn concurrent_writes_same_blob(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let content = b"concurrent write test content";
    let key = test_blob(content);

    let mut handles = Vec::new();
    for _ in 0..10 {
        let client = fixture.client_alice.clone();
        let key = key.clone();
        let content = content.to_vec();
        handles.push(tokio::spawn(async move {
            client.cas_write_bytes(&key, content).await
        }));
    }

    for handle in handles {
        handle.await??;
    }

    let read = fixture
        .client_alice
        .cas_read_bytes(&key)
        .await?
        .expect("blob should exist");
    pretty_assert_eq!(read.as_slice(), content);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn concurrent_writes_different_blobs(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let blobs = (0..10)
        .map(|i| format!("concurrent blob {i}").into_bytes())
        .collect::<Vec<_>>();
    let keys = blobs.iter().map(|b| test_blob(b)).collect::<Vec<_>>();

    let mut handles = Vec::new();
    for (key, content) in keys.iter().zip(&blobs) {
        let client = fixture.client_alice.clone();
        let key = key.clone();
        let content = content.clone();
        handles.push(tokio::spawn(async move {
            client.cas_write_bytes(&key, content).await
        }));
    }

    for handle in handles {
        handle.await??;
    }

    for (key, expected) in keys.iter().zip(&blobs) {
        let read = fixture
            .client_alice
            .cas_read_bytes(key)
            .await?
            .expect("blob should exist");
        pretty_assert_eq!(read.as_slice(), expected.as_slice());
    }

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn write_missing_auth_returns_401(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let content = b"test blob";
    let key = test_blob(content);

    let client_no_auth = fixture.client_with_token("")?;
    let result = client_no_auth.cas_write_bytes(&key, content.to_vec()).await;
    assert!(result.is_err(), "write without auth should fail");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn write_invalid_token_returns_401(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let content = b"test blob";
    let key = test_blob(content);

    let client = fixture.client_with_token("invalid-token-that-does-not-exist")?;
    let result = client.cas_write_bytes(&key, content.to_vec()).await;
    assert!(result.is_err(), "write with invalid token should fail");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn write_revoked_token_returns_401(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let content = b"test blob";
    let key = test_blob(content);

    let client = fixture.client_with_token(fixture.auth.token_alice_revoked().expose())?;
    let result = client.cas_write_bytes(&key, content.to_vec()).await;
    assert!(result.is_err(), "write with revoked token should fail");

    Ok(())
}
