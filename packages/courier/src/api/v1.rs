use axum::{Router, routing::get};

use crate::api::State;

pub mod cache;
pub mod cas;
pub mod health;

pub fn router() -> Router<State> {
    Router::new()
        .nest("/cache", cache::router())
        .nest("/cas", cas::router())
        .route("/health", get(health::handle))
}

#[cfg(test)]
mod tests {
    use axum::body::Bytes;
    use axum::http::StatusCode;
    use color_eyre::{Result, eyre::Context};
    use pretty_assertions::{assert_eq as pretty_assert_eq, assert_ne as pretty_assert_ne};
    use sqlx::PgPool;

    use crate::api::test_helpers::{test_blob, test_server};

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn full_client_workflow(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;
        let token_alice = auth.token_alice().expose();

        // Step 1: Write some blobs
        let blob1_content = b"first blob content";
        let blob2_content = b"second blob content with more data";
        let blob3_content = vec![0xFF; 1024]; // Binary data

        let (_, blob1_key) = test_blob(blob1_content);
        let (_, blob2_key) = test_blob(blob2_content);
        let (_, blob3_key) = test_blob(&blob3_content);

        for (key, content) in [
            (&blob1_key, Bytes::from_static(blob1_content)),
            (&blob2_key, Bytes::from_static(blob2_content)),
            (&blob3_key, Bytes::copy_from_slice(&blob3_content)),
        ] {
            let write_response = server
                .put(&format!("/api/v1/cas/{key}"))
                .authorization_bearer(&token_alice)
                .bytes(content)
                .await;
            write_response.assert_status(StatusCode::CREATED);
        }

        // Step 2: Read those blobs back and validate content
        let read1 = server
            .get(&format!("/api/v1/cas/{blob1_key}"))
            .authorization_bearer(&token_alice)
            .await;
        read1.assert_status_ok();
        pretty_assert_eq!(read1.as_bytes().as_ref(), blob1_content);

        let read2 = server
            .get(&format!("/api/v1/cas/{blob2_key}"))
            .authorization_bearer(&token_alice)
            .await;
        read2.assert_status_ok();
        pretty_assert_eq!(read2.as_bytes().as_ref(), blob2_content);

        let read3 = server
            .get(&format!("/api/v1/cas/{blob3_key}"))
            .authorization_bearer(&token_alice)
            .await;
        read3.assert_status_ok();
        pretty_assert_eq!(read3.as_bytes().as_ref(), blob3_content.as_slice());

        // Step 3: Try to read a blob that doesn't exist yet
        let new_blob_content = b"blob that doesn't exist yet";
        let (_, new_blob_key) = test_blob(new_blob_content);

        let read_nonexistent = server
            .get(&format!("/api/v1/cas/{new_blob_key}"))
            .authorization_bearer(&token_alice)
            .await;
        read_nonexistent.assert_status(StatusCode::NOT_FOUND);

        // Step 4: Check that the blob doesn't exist
        let check_nonexistent = server
            .method(
                axum::http::Method::HEAD,
                &format!("/api/v1/cas/{new_blob_key}"),
            )
            .authorization_bearer(&token_alice)
            .await;
        check_nonexistent.assert_status(StatusCode::NOT_FOUND);

        // Step 5: Write the new blob
        let write_new = server
            .put(&format!("/api/v1/cas/{new_blob_key}"))
            .authorization_bearer(&token_alice)
            .bytes(Bytes::from_static(new_blob_content))
            .await;
        write_new.assert_status(StatusCode::CREATED);

        // Step 6: Check that it now exists
        let check_exists = server
            .method(
                axum::http::Method::HEAD,
                &format!("/api/v1/cas/{new_blob_key}"),
            )
            .authorization_bearer(&token_alice)
            .await;
        check_exists.assert_status_ok();

        // Step 7: Read it back and verify content
        let read_new = server
            .get(&format!("/api/v1/cas/{new_blob_key}"))
            .authorization_bearer(&token_alice)
            .await;
        read_new.assert_status_ok();
        pretty_assert_eq!(read_new.as_bytes().as_ref(), new_blob_content);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn request_id_echoed_when_provided(pool: PgPool) -> Result<()> {
        let (server, _auth, _tmp) = test_server(pool).await.context("create test server")?;

        let client_request_id = "client-provided-12345";

        let response = server
            .get("/api/v1/health")
            .add_header("x-request-id", client_request_id)
            .await;

        response.assert_status_ok();
        let response_request_id = response
            .headers()
            .get("x-request-id")
            .expect("x-request-id header should be present")
            .to_str()
            .expect("x-request-id should be valid UTF-8");

        pretty_assert_eq!(response_request_id, client_request_id);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn request_id_generated_when_not_provided(pool: PgPool) -> Result<()> {
        let (server, _auth, _tmp) = test_server(pool).await.context("create test server")?;

        let response1 = server.get("/api/v1/health").await;
        response1.assert_status_ok();
        let request_id1 = response1
            .headers()
            .get("x-request-id")
            .expect("x-request-id header should be present")
            .to_str()
            .expect("x-request-id should be valid UTF-8");

        let response2 = server.get("/api/v1/health").await;
        response2.assert_status_ok();
        let request_id2 = response2
            .headers()
            .get("x-request-id")
            .expect("x-request-id header should be present")
            .to_str()
            .expect("x-request-id should be valid UTF-8");

        // Ensure both IDs are valid UUIDs
        assert!(
            uuid::Uuid::parse_str(request_id1).is_ok(),
            "request_id1 should be a valid UUID: {request_id1}"
        );
        assert!(
            uuid::Uuid::parse_str(request_id2).is_ok(),
            "request_id2 should be a valid UUID: {request_id2}"
        );

        // Ensure the two requests got different IDs
        pretty_assert_ne!(
            request_id1,
            request_id2,
            "Different requests should get different request IDs"
        );

        Ok(())
    }
}
