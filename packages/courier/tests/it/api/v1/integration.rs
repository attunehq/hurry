//! End-to-end integration tests for the v1 API.

use color_eyre::Result;
use pretty_assertions::assert_eq as pretty_assert_eq;
use pretty_assertions::assert_ne as pretty_assert_ne;
use sqlx::PgPool;

use crate::helpers::{TestFixture, test_blob};

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn full_client_workflow(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let client = &fixture.client_alice;

    let blob1_content = b"first blob content";
    let blob2_content = b"second blob content with more data";
    let blob3_content = vec![0xFF; 1024];

    let blob1_key = test_blob(blob1_content);
    let blob2_key = test_blob(blob2_content);
    let blob3_key = test_blob(&blob3_content);

    client
        .cas_write_bytes(&blob1_key, blob1_content.to_vec())
        .await?;
    client
        .cas_write_bytes(&blob2_key, blob2_content.to_vec())
        .await?;
    client
        .cas_write_bytes(&blob3_key, blob3_content.clone())
        .await?;

    let read1 = client
        .cas_read_bytes(&blob1_key)
        .await?
        .expect("blob1 should exist");
    pretty_assert_eq!(read1.as_slice(), blob1_content);

    let read2 = client
        .cas_read_bytes(&blob2_key)
        .await?
        .expect("blob2 should exist");
    pretty_assert_eq!(read2.as_slice(), blob2_content);

    let read3 = client
        .cas_read_bytes(&blob3_key)
        .await?
        .expect("blob3 should exist");
    pretty_assert_eq!(read3.as_slice(), blob3_content.as_slice());

    let new_blob_content = b"blob that doesn't exist yet";
    let new_blob_key = test_blob(new_blob_content);

    let read_nonexistent = client.cas_read_bytes(&new_blob_key).await?;
    assert!(read_nonexistent.is_none(), "blob should not exist yet");

    let check_nonexistent = client.cas_exists(&new_blob_key).await?;
    assert!(!check_nonexistent, "blob should not exist yet");

    client
        .cas_write_bytes(&new_blob_key, new_blob_content.to_vec())
        .await?;

    let check_exists = client.cas_exists(&new_blob_key).await?;
    assert!(check_exists, "blob should now exist");

    let read_new = client
        .cas_read_bytes(&new_blob_key)
        .await?
        .expect("new blob should exist");
    pretty_assert_eq!(read_new.as_slice(), new_blob_content);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn request_id_echoed_when_provided(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let client_request_id = "client-provided-12345";
    let url = fixture.base_url.join("api/v1/health")?;

    let response = reqwest::Client::new()
        .get(url)
        .header("x-request-id", client_request_id)
        .send()
        .await?;

    let response_request_id = response
        .headers()
        .get("x-request-id")
        .expect("x-request-id header should be present")
        .to_str()
        .expect("x-request-id should be valid UTF-8");

    pretty_assert_eq!(response_request_id, client_request_id);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn request_id_generated_when_not_provided(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let url = fixture.base_url.join("api/v1/health")?;

    let response1 = reqwest::Client::new().get(url.clone()).send().await?;
    let request_id1 = response1
        .headers()
        .get("x-request-id")
        .expect("x-request-id header should be present")
        .to_str()
        .expect("x-request-id should be valid UTF-8");

    let response2 = reqwest::Client::new().get(url).send().await?;
    let request_id2 = response2
        .headers()
        .get("x-request-id")
        .expect("x-request-id header should be present")
        .to_str()
        .expect("x-request-id should be valid UTF-8");

    assert!(
        uuid::Uuid::parse_str(request_id1).is_ok(),
        "request_id1 should be a valid UUID: {request_id1}"
    );
    assert!(
        uuid::Uuid::parse_str(request_id2).is_ok(),
        "request_id2 should be a valid UUID: {request_id2}"
    );

    pretty_assert_ne!(
        request_id1,
        request_id2,
        "Different requests should get different request IDs"
    );

    Ok(())
}
