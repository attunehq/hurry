//! CAS bulk write endpoint tests.

use clients::courier::v1::Key;
use color_eyre::Result;
use futures::stream::{self, StreamExt};
use pretty_assertions::assert_eq as pretty_assert_eq;
use sqlx::PgPool;

use crate::helpers::{TestFixture, test_blob};

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn bulk_write_multiple_blobs(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let blob1 = b"first blob content".to_vec();
    let blob2 = b"second blob content".to_vec();
    let blob3 = b"third blob content".to_vec();

    let key1 = test_blob(&blob1);
    let key2 = test_blob(&blob2);
    let key3 = test_blob(&blob3);

    let entries = vec![
        (key1.clone(), blob1.clone()),
        (key2.clone(), blob2.clone()),
        (key3.clone(), blob3.clone()),
    ];
    let entries_stream = stream::iter(entries);

    let response = fixture.client_alice.cas_write_bulk(entries_stream).await?;

    assert_eq!(response.written.len(), 3);
    assert!(response.written.contains(&key1));
    assert!(response.written.contains(&key2));
    assert!(response.written.contains(&key3));
    assert!(response.skipped.is_empty());
    assert!(response.errors.is_empty());

    let read1 = fixture
        .client_alice
        .cas_read_bytes(&key1)
        .await?
        .expect("blob1 should exist");
    pretty_assert_eq!(read1, blob1);

    let read2 = fixture
        .client_alice
        .cas_read_bytes(&key2)
        .await?
        .expect("blob2 should exist");
    pretty_assert_eq!(read2, blob2);

    let read3 = fixture
        .client_alice
        .cas_read_bytes(&key3)
        .await?
        .expect("blob3 should exist");
    pretty_assert_eq!(read3, blob3);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn bulk_write_idempotent(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let blob = b"idempotent blob".to_vec();
    let key = test_blob(&blob);

    let entries1 = vec![(key.clone(), blob.clone())];
    let response1 = fixture
        .client_alice
        .cas_write_bulk(stream::iter(entries1))
        .await?;

    assert_eq!(response1.written.len(), 1);
    assert!(response1.written.contains(&key));
    assert!(response1.skipped.is_empty());

    let entries2 = vec![(key.clone(), blob.clone())];
    let response2 = fixture
        .client_alice
        .cas_write_bulk(stream::iter(entries2))
        .await?;

    assert!(response2.written.is_empty());
    assert_eq!(response2.skipped.len(), 1);
    assert!(response2.skipped.contains(&key));

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn bulk_write_invalid_hash(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let actual_content = b"actual content".to_vec();
    let wrong_key = test_blob(b"different content");

    let entries = vec![(wrong_key.clone(), actual_content)];
    let response = fixture
        .client_alice
        .cas_write_bulk(stream::iter(entries))
        .await?;

    assert!(response.written.is_empty());
    assert!(response.skipped.is_empty());
    assert_eq!(response.errors.len(), 1);

    let error = response
        .errors
        .iter()
        .next()
        .expect("should have one error");
    assert_eq!(error.key, wrong_key);
    assert!(
        error.error.to_lowercase().contains("hash")
            || error.error.to_lowercase().contains("mismatch")
    );

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn bulk_write_invalid_filename(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let url = fixture.base_url.join("api/v1/cas/bulk/write")?;

    let mut tar_builder = async_tar::Builder::new(Vec::new());
    let content = b"test content";
    let mut header = async_tar::Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar_builder
        .append_data(&mut header, "not-a-valid-hex-key", content.as_slice())
        .await?;
    let tar_bytes = tar_builder.into_inner().await?;

    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(fixture.auth.token_alice().expose())
        .header("Content-Type", "application/x-tar+zstd")
        .body(tar_bytes)
        .send()
        .await?;

    assert_eq!(
        response.status().as_u16(),
        400,
        "should return 400 Bad Request"
    );

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn bulk_write_partial_success(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let valid_content = b"valid content".to_vec();
    let valid_key = test_blob(&valid_content);

    let invalid_content = b"actual content".to_vec();
    let invalid_key = test_blob(b"different content");

    let entries = vec![
        (valid_key.clone(), valid_content.clone()),
        (invalid_key.clone(), invalid_content),
    ];
    let response = fixture
        .client_alice
        .cas_write_bulk(stream::iter(entries))
        .await?;

    assert_eq!(response.written.len(), 1);
    assert!(response.written.contains(&valid_key));
    assert!(response.skipped.is_empty());
    assert_eq!(response.errors.len(), 1);

    let error = response
        .errors
        .iter()
        .next()
        .expect("should have one error");
    assert_eq!(error.key, invalid_key);

    let read_valid = fixture
        .client_alice
        .cas_read_bytes(&valid_key)
        .await?
        .expect("valid blob should exist");
    pretty_assert_eq!(read_valid, valid_content);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn bulk_write_empty_tar(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let entries = Vec::<(Key, Vec<u8>)>::new();
    let response = fixture
        .client_alice
        .cas_write_bulk(stream::iter(entries))
        .await?;

    assert!(response.written.is_empty());
    assert!(response.skipped.is_empty());
    assert!(response.errors.is_empty());

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn bulk_write_compressed(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let blob1 = b"compressed blob 1".to_vec();
    let blob2 = b"compressed blob 2".to_vec();
    let blob3 = b"compressed blob 3".to_vec();

    let key1 = test_blob(&blob1);
    let key2 = test_blob(&blob2);
    let key3 = test_blob(&blob3);

    let entries = vec![
        (key1.clone(), blob1.clone()),
        (key2.clone(), blob2.clone()),
        (key3.clone(), blob3.clone()),
    ];
    let response = fixture
        .client_alice
        .cas_write_bulk(stream::iter(entries))
        .await?;

    assert_eq!(response.written.len(), 3);
    assert!(response.errors.is_empty());

    let read1 = fixture
        .client_alice
        .cas_read_bytes(&key1)
        .await?
        .expect("blob1 should exist");
    pretty_assert_eq!(read1, blob1);

    let read2 = fixture
        .client_alice
        .cas_read_bytes(&key2)
        .await?
        .expect("blob2 should exist");
    pretty_assert_eq!(read2, blob2);

    let read3 = fixture
        .client_alice
        .cas_read_bytes(&key3)
        .await?
        .expect("blob3 should exist");
    pretty_assert_eq!(read3, blob3);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn bulk_write_compressed_idempotent(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let blob = b"idempotent compressed blob".to_vec();
    let key = test_blob(&blob);

    let entries1 = vec![(key.clone(), blob.clone())];
    let response1 = fixture
        .client_alice
        .cas_write_bulk(stream::iter(entries1))
        .await?;

    assert_eq!(response1.written.len(), 1);
    assert!(response1.written.contains(&key));

    let entries2 = vec![(key.clone(), blob.clone())];
    let response2 = fixture
        .client_alice
        .cas_write_bulk(stream::iter(entries2))
        .await?;

    assert!(response2.written.is_empty());
    assert_eq!(response2.skipped.len(), 1);
    assert!(response2.skipped.contains(&key));

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn bulk_write_compressed_invalid_hash(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let actual_content = b"actual content".to_vec();
    let wrong_key = test_blob(b"different content");

    let entries = vec![(wrong_key.clone(), actual_content)];
    let response = fixture
        .client_alice
        .cas_write_bulk(stream::iter(entries))
        .await?;

    assert!(response.written.is_empty());
    assert!(response.skipped.is_empty());
    assert_eq!(response.errors.len(), 1);

    let error = response
        .errors
        .iter()
        .next()
        .expect("should have one error");
    assert_eq!(error.key, wrong_key);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn bulk_write_compressed_partial_success(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let valid_content = b"valid compressed content".to_vec();
    let valid_key = test_blob(&valid_content);

    let invalid_content = b"actual content".to_vec();
    let invalid_key = test_blob(b"different content");

    let entries = vec![
        (valid_key.clone(), valid_content.clone()),
        (invalid_key.clone(), invalid_content),
    ];
    let response = fixture
        .client_alice
        .cas_write_bulk(stream::iter(entries))
        .await?;

    assert_eq!(response.written.len(), 1);
    assert!(response.written.contains(&valid_key));
    assert_eq!(response.errors.len(), 1);

    let read_valid = fixture
        .client_alice
        .cas_read_bytes(&valid_key)
        .await?
        .expect("valid blob should exist");
    pretty_assert_eq!(read_valid, valid_content);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn bulk_write_compressed_roundtrip(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let blob1 = b"first blob".to_vec();
    let blob2 = b"second blob".to_vec();

    let key1 = test_blob(&blob1);
    let key2 = test_blob(&blob2);

    let entries = vec![(key1.clone(), blob1.clone()), (key2.clone(), blob2.clone())];
    let response = fixture
        .client_alice
        .cas_write_bulk(stream::iter(entries))
        .await?;

    assert_eq!(response.written.len(), 2);

    let mut bulk_read_stream = fixture.client_alice.cas_read_bulk([&key1, &key2]).await?;

    let mut results = Vec::new();
    while let Some(result) = bulk_read_stream.next().await {
        results.push(result?);
    }

    assert_eq!(results.len(), 2);

    let (read_key1, read_content1) = &results[0];
    let (read_key2, read_content2) = &results[1];

    if read_key1 == &key1 {
        pretty_assert_eq!(read_content1, &blob1);
        pretty_assert_eq!(read_key2, &key2);
        pretty_assert_eq!(read_content2, &blob2);
    } else {
        pretty_assert_eq!(read_key1, &key2);
        pretty_assert_eq!(read_content1, &blob2);
        pretty_assert_eq!(read_key2, &key1);
        pretty_assert_eq!(read_content2, &blob1);
    }

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn bulk_write_grants_access_when_blob_exists(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let content = b"shared blob content".to_vec();
    let key = test_blob(&content);

    let entries_alice = vec![(key.clone(), content.clone())];
    let response_alice = fixture
        .client_alice
        .cas_write_bulk(stream::iter(entries_alice))
        .await?;

    assert_eq!(response_alice.written.len(), 1);
    assert!(response_alice.written.contains(&key));

    let read_charlie_before = fixture.client_charlie.cas_read_bytes(&key).await?;
    assert!(
        read_charlie_before.is_none(),
        "Charlie should not have access yet"
    );

    let entries_charlie = vec![(key.clone(), content.clone())];
    let response_charlie = fixture
        .client_charlie
        .cas_write_bulk(stream::iter(entries_charlie))
        .await?;

    assert_eq!(
        response_charlie.written.len(),
        1,
        "Charlie's write should report blob as written (granting access)"
    );
    assert!(response_charlie.written.contains(&key));
    assert!(response_charlie.skipped.is_empty());

    let read_charlie_after = fixture
        .client_charlie
        .cas_read_bytes(&key)
        .await?
        .expect("Charlie should now have access");
    pretty_assert_eq!(read_charlie_after, content);

    Ok(())
}
