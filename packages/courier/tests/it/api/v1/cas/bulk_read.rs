//! CAS bulk read endpoint tests.

use clients::courier::v1::Key;
use color_eyre::Result;
use futures::stream::StreamExt;
use pretty_assertions::assert_eq as pretty_assert_eq;
use sqlx::PgPool;

use crate::helpers::{TestFixture, test_blob};

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn bulk_read_multiple_blobs(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let blob1 = b"first blob content".to_vec();
    let blob2 = b"second blob content".to_vec();
    let blob3 = b"third blob content".to_vec();

    let key1 = test_blob(&blob1);
    let key2 = test_blob(&blob2);
    let key3 = test_blob(&blob3);

    fixture
        .client_alice
        .cas_write_bytes(&key1, blob1.clone())
        .await?;
    fixture
        .client_alice
        .cas_write_bytes(&key2, blob2.clone())
        .await?;
    fixture
        .client_alice
        .cas_write_bytes(&key3, blob3.clone())
        .await?;

    let mut bulk_read_stream = fixture
        .client_alice
        .cas_read_bulk([&key1, &key2, &key3])
        .await?;

    let mut results = Vec::new();
    while let Some(result) = bulk_read_stream.next().await {
        results.push(result?);
    }

    assert_eq!(results.len(), 3);

    for (key, content) in results {
        if key == key1 {
            pretty_assert_eq!(content, blob1);
        } else if key == key2 {
            pretty_assert_eq!(content, blob2);
        } else if key == key3 {
            pretty_assert_eq!(content, blob3);
        } else {
            panic!("Unexpected key: {key:?}");
        }
    }

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn bulk_read_missing_keys(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let existing_blob = b"existing blob".to_vec();
    let existing_key = test_blob(&existing_blob);
    fixture
        .client_alice
        .cas_write_bytes(&existing_key, existing_blob.clone())
        .await?;

    let missing_key = test_blob(&vec![0u8; 32]);

    let mut bulk_read_stream = fixture
        .client_alice
        .cas_read_bulk([&existing_key, &missing_key])
        .await?;

    let mut results = Vec::new();
    while let Some(result) = bulk_read_stream.next().await {
        results.push(result?);
    }

    assert_eq!(results.len(), 1, "only existing blob should be returned");

    let (key, content) = &results[0];
    pretty_assert_eq!(key, &existing_key);
    pretty_assert_eq!(content, &existing_blob);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn bulk_read_empty_request(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let keys: Vec<&Key> = Vec::new();
    let mut bulk_read_stream = fixture.client_alice.cas_read_bulk(keys).await?;

    let mut results = Vec::new();
    while let Some(result) = bulk_read_stream.next().await {
        results.push(result?);
    }

    assert_eq!(results.len(), 0, "empty request should return no blobs");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn bulk_read_invalid_keys(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let url = fixture.base_url.join("api/v1/cas/bulk/read")?;

    let invalid_json = r#"{"keys": ["not-a-hex-key", "also-invalid"]}"#;

    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(fixture.auth.token_alice().expose())
        .header("Content-Type", "application/json")
        .body(invalid_json)
        .send()
        .await?;

    assert_eq!(
        response.status().as_u16(),
        422,
        "should return 422 Unprocessable Entity"
    );

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn bulk_read_compressed(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let blob1 = b"compressed blob 1".to_vec();
    let blob2 = b"compressed blob 2".to_vec();
    let blob3 = b"compressed blob 3".to_vec();

    let key1 = test_blob(&blob1);
    let key2 = test_blob(&blob2);
    let key3 = test_blob(&blob3);

    fixture
        .client_alice
        .cas_write_bytes(&key1, blob1.clone())
        .await?;
    fixture
        .client_alice
        .cas_write_bytes(&key2, blob2.clone())
        .await?;
    fixture
        .client_alice
        .cas_write_bytes(&key3, blob3.clone())
        .await?;

    let mut bulk_read_stream = fixture
        .client_alice
        .cas_read_bulk([&key1, &key2, &key3])
        .await?;

    let mut results = Vec::new();
    while let Some(result) = bulk_read_stream.next().await {
        results.push(result?);
    }

    assert_eq!(results.len(), 3);

    for (key, content) in results {
        if key == key1 {
            pretty_assert_eq!(content, blob1);
        } else if key == key2 {
            pretty_assert_eq!(content, blob2);
        } else if key == key3 {
            pretty_assert_eq!(content, blob3);
        } else {
            panic!("Unexpected key: {key:?}");
        }
    }

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn bulk_read_uncompressed_explicit(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let blob1 = b"uncompressed blob 1".to_vec();
    let blob2 = b"uncompressed blob 2".to_vec();

    let key1 = test_blob(&blob1);
    let key2 = test_blob(&blob2);

    fixture
        .client_alice
        .cas_write_bytes(&key1, blob1.clone())
        .await?;
    fixture
        .client_alice
        .cas_write_bytes(&key2, blob2.clone())
        .await?;

    let mut bulk_read_stream = fixture.client_alice.cas_read_bulk([&key1, &key2]).await?;

    let mut results = Vec::new();
    while let Some(result) = bulk_read_stream.next().await {
        results.push(result?);
    }

    assert_eq!(results.len(), 2);

    for (key, content) in results {
        if key == key1 {
            pretty_assert_eq!(content, blob1);
        } else if key == key2 {
            pretty_assert_eq!(content, blob2);
        } else {
            panic!("Unexpected key: {key:?}");
        }
    }

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn bulk_read_compressed_missing_keys(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let existing_blob = b"existing compressed blob".to_vec();
    let existing_key = test_blob(&existing_blob);
    fixture
        .client_alice
        .cas_write_bytes(&existing_key, existing_blob.clone())
        .await?;

    let missing_key = test_blob(&vec![0u8; 32]);

    let mut bulk_read_stream = fixture
        .client_alice
        .cas_read_bulk([&existing_key, &missing_key])
        .await?;

    let mut results = Vec::new();
    while let Some(result) = bulk_read_stream.next().await {
        results.push(result?);
    }

    assert_eq!(results.len(), 1, "only existing blob should be returned");

    let (key, content) = &results[0];
    pretty_assert_eq!(key, &existing_key);
    pretty_assert_eq!(content, &existing_blob);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn bulk_read_filters_inaccessible_blobs(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let org_a_blob1 = b"org A blob 1".to_vec();
    let org_a_blob2 = b"org A blob 2".to_vec();
    let org_b_blob = b"org B blob".to_vec();

    let org_a_key1 = test_blob(&org_a_blob1);
    let org_a_key2 = test_blob(&org_a_blob2);
    let org_b_key = test_blob(&org_b_blob);

    fixture
        .client_alice
        .cas_write_bytes(&org_a_key1, org_a_blob1.clone())
        .await?;
    fixture
        .client_alice
        .cas_write_bytes(&org_a_key2, org_a_blob2.clone())
        .await?;
    fixture
        .client_charlie
        .cas_write_bytes(&org_b_key, org_b_blob.clone())
        .await?;

    let mut bulk_read_stream = fixture
        .client_alice
        .cas_read_bulk([&org_a_key1, &org_a_key2, &org_b_key])
        .await?;

    let mut results = Vec::new();
    while let Some(result) = bulk_read_stream.next().await {
        results.push(result?);
    }

    assert_eq!(results.len(), 2, "only Org A's blobs should be returned");

    for (key, content) in results {
        if key == org_a_key1 {
            pretty_assert_eq!(content, org_a_blob1);
        } else if key == org_a_key2 {
            pretty_assert_eq!(content, org_a_blob2);
        } else {
            panic!("Unexpected key (should not get Org B's blob): {key:?}");
        }
    }

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn same_org_users_can_bulk_read_each_others_blobs(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let alice_blob1 = b"Alice's first blob".to_vec();
    let alice_blob2 = b"Alice's second blob".to_vec();
    let bob_blob1 = b"Bob's first blob".to_vec();
    let bob_blob2 = b"Bob's second blob".to_vec();

    let alice_key1 = test_blob(&alice_blob1);
    let alice_key2 = test_blob(&alice_blob2);
    let bob_key1 = test_blob(&bob_blob1);
    let bob_key2 = test_blob(&bob_blob2);

    fixture
        .client_alice
        .cas_write_bytes(&alice_key1, alice_blob1.clone())
        .await?;
    fixture
        .client_alice
        .cas_write_bytes(&alice_key2, alice_blob2.clone())
        .await?;
    fixture
        .client_bob
        .cas_write_bytes(&bob_key1, bob_blob1.clone())
        .await?;
    fixture
        .client_bob
        .cas_write_bytes(&bob_key2, bob_blob2.clone())
        .await?;

    let mut bob_reads_alice = fixture
        .client_bob
        .cas_read_bulk([&alice_key1, &alice_key2])
        .await?;
    let mut bob_results = Vec::new();
    while let Some(result) = bob_reads_alice.next().await {
        bob_results.push(result?);
    }
    assert_eq!(bob_results.len(), 2);

    let mut alice_reads_bob = fixture
        .client_alice
        .cas_read_bulk([&bob_key1, &bob_key2])
        .await?;
    let mut alice_results = Vec::new();
    while let Some(result) = alice_reads_bob.next().await {
        alice_results.push(result?);
    }
    assert_eq!(alice_results.len(), 2);

    Ok(())
}
