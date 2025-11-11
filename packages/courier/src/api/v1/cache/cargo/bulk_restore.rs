use aerosol::axum::Dep;
use axum::{Json, http::StatusCode, response::IntoResponse};
use clients::courier::v1::cache::{
    CargoBulkRestoreHit, CargoBulkRestoreRequest, CargoBulkRestoreResponse,
};
use color_eyre::eyre::{Report, eyre};
use tap::Pipe;
use tracing::{error, info};

use crate::{auth::AuthenticatedToken, db::Postgres};

/// The max amount of items in a single bulk restore request.
///
/// This limit is based on payload size analysis (see
/// `examples/size_calculator.rs`):
/// - 100 items: ~23 KB request, ~122 KB response (5 artifacts/hit)
/// - 500 items: ~115 KB request, ~611 KB response (5 artifacts/hit)
/// - 1,000 items: ~230 KB request, ~1.2 MB response (5 artifacts/hit)
/// - 10,000 items: ~2.2 MB request, ~12 MB response (5 artifacts/hit)
/// - 100,000 items: ~22 MB request, ~120 MB response (5 artifacts/hit)
///
/// With compression enabled (which reduces JSON by ~70-80%), even 100k items
/// results in manageable payload sizes. This high limit allows the client to
/// avoid complex batching logic while still protecting against excessive
/// requests.
const MAX_BULK_RESTORE_REQUESTS: usize = 100_000;

#[tracing::instrument(skip(auth, body))]
pub async fn handle(
    auth: AuthenticatedToken,
    Dep(db): Dep<Postgres>,
    Json(body): Json<CargoBulkRestoreRequest>,
) -> CacheBulkRestoreResponse {
    info!(requests = body.requests.len(), "cache.bulk_restore.start");

    if body.requests.len() > MAX_BULK_RESTORE_REQUESTS {
        error!(
            count = body.requests.len(),
            max = MAX_BULK_RESTORE_REQUESTS,
            "cache.bulk_restore.too_many_requests"
        );
        return CacheBulkRestoreResponse::InvalidRequest(eyre!(
            "bulk restore limited to {} requests, got {}",
            MAX_BULK_RESTORE_REQUESTS,
            body.requests.len()
        ));
    }

    let mut results = match db.cargo_cache_restore_bulk(&auth, &body.requests).await {
        Ok(results) => results,
        Err(err) => {
            error!(error = ?err, "cache.bulk_restore.error");
            return CacheBulkRestoreResponse::Error(err);
        }
    };

    let mut hits = Vec::new();
    let mut misses = Vec::new();
    for (idx, req) in body.requests.into_iter().enumerate() {
        if let Some(artifacts) = results.remove(&idx) {
            hits.push(
                CargoBulkRestoreHit::builder()
                    .artifacts(artifacts)
                    .request(req)
                    .build(),
            );
        } else {
            misses.push(req);
        }
    }

    info!(
        hits = hits.len(),
        misses = misses.len(),
        "cache.bulk_restore.complete"
    );

    CargoBulkRestoreResponse::builder()
        .hits(hits)
        .misses(misses)
        .build()
        .pipe(CacheBulkRestoreResponse::Success)
}

#[derive(Debug)]
pub enum CacheBulkRestoreResponse {
    Success(CargoBulkRestoreResponse),
    InvalidRequest(Report),
    Error(Report),
}

impl IntoResponse for CacheBulkRestoreResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CacheBulkRestoreResponse::Success(body) => (StatusCode::OK, Json(body)).into_response(),
            CacheBulkRestoreResponse::InvalidRequest(error) => {
                (StatusCode::BAD_REQUEST, format!("{error:?}")).into_response()
            }
            CacheBulkRestoreResponse::Error(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:?}")).into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use clients::courier::v1::cache::{
        ArtifactFile, CargoBulkRestoreHit, CargoBulkRestoreRequest, CargoBulkRestoreResponse,
        CargoRestoreRequest, CargoSaveRequest,
    };
    use color_eyre::{Result, eyre::Context};
    use pretty_assertions::assert_eq as pretty_assert_eq;
    use sqlx::PgPool;

    use crate::api::test_helpers::{test_blob, test_server};

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn bulk_restore_multiple_packages(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        // Save 3 different packages
        let packages = [
            ("serde", "1.0.0", "abc123"),
            ("tokio", "1.28.0", "def456"),
            ("axum", "0.6.0", "ghi789"),
        ];

        let saved_packages = packages
            .iter()
            .map(|(name, version, hash)| {
                let (_, key) = test_blob(format!("{name}_content").as_bytes());

                let save_request = CargoSaveRequest::builder()
                    .package_name(*name)
                    .package_version(*version)
                    .target("x86_64-unknown-linux-gnu")
                    .library_crate_compilation_unit_hash(*hash)
                    .content_hash(format!("content_{hash}"))
                    .artifacts([ArtifactFile::builder()
                        .object_key(&key)
                        .path(format!("lib{name}.rlib"))
                        .mtime_nanos(1000000000000000000u128)
                        .executable(false)
                        .build()])
                    .build();

                (*name, *version, *hash, key, save_request)
            })
            .collect::<Vec<_>>();

        for (_, _, _, _, save_request) in &saved_packages {
            let response = server
                .post("/api/v1/cache/cargo/save")
                .authorization_bearer(auth.token_alice().expose())
                .json(save_request)
                .await;
            response.assert_status(StatusCode::CREATED);
        }

        // Bulk restore all 3
        let bulk_request = CargoBulkRestoreRequest::builder()
            .requests(packages.iter().map(|(name, version, hash)| {
                CargoRestoreRequest::builder()
                    .package_name(*name)
                    .package_version(*version)
                    .target("x86_64-unknown-linux-gnu")
                    .library_crate_compilation_unit_hash(*hash)
                    .build()
            }))
            .build();

        let response = server
            .post("/api/v1/cache/cargo/bulk/restore")
            .authorization_bearer(auth.token_alice().expose())
            .json(&bulk_request)
            .await;

        response.assert_status_ok();
        let bulk_response = response.json::<CargoBulkRestoreResponse>();

        pretty_assert_eq!(bulk_response.hits.len(), 3);
        pretty_assert_eq!(bulk_response.misses.len(), 0);

        // Verify each hit
        for (name, version, hash, key, _) in &saved_packages {
            let hit = bulk_response
                .hits
                .iter()
                .find(|h| h.request.package_name == *name)
                .expect("hit not found");

            let expected_request = CargoRestoreRequest::builder()
                .package_name(*name)
                .package_version(*version)
                .target("x86_64-unknown-linux-gnu")
                .library_crate_compilation_unit_hash(*hash)
                .build();

            let expected_artifact = ArtifactFile::builder()
                .object_key(key)
                .path(format!("lib{name}.rlib"))
                .mtime_nanos(1000000000000000000u128)
                .executable(false)
                .build();

            pretty_assert_eq!(hit.request, expected_request);
            pretty_assert_eq!(hit.artifacts, vec![expected_artifact]);
        }

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn bulk_restore_partial_hits(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        // Save only 2 packages
        let saved_packages = vec![("serde", "1.0.0", "abc123"), ("tokio", "1.28.0", "def456")];

        for (name, version, hash) in &saved_packages {
            let (_, key) = test_blob(format!("{name}_content").as_bytes());

            let save_request = CargoSaveRequest::builder()
                .package_name(*name)
                .package_version(*version)
                .target("x86_64-unknown-linux-gnu")
                .library_crate_compilation_unit_hash(*hash)
                .content_hash(format!("content_{hash}"))
                .artifacts([ArtifactFile::builder()
                    .object_key(&key)
                    .path(format!("lib{name}.rlib"))
                    .mtime_nanos(1000000000000000000u128)
                    .executable(false)
                    .build()])
                .build();

            let response = server
                .post("/api/v1/cache/cargo/save")
                .authorization_bearer(auth.token_alice().expose())
                .json(&save_request)
                .await;
            response.assert_status(StatusCode::CREATED);
        }

        // Request 4 packages (2 exist, 2 don't)
        let all_packages = [
            ("serde", "1.0.0", "abc123"),
            ("tokio", "1.28.0", "def456"),
            ("axum", "0.6.0", "missing1"),
            ("reqwest", "0.11.0", "missing2"),
        ];

        let bulk_request = CargoBulkRestoreRequest::builder()
            .requests(all_packages.iter().map(|(name, version, hash)| {
                CargoRestoreRequest::builder()
                    .package_name(*name)
                    .package_version(*version)
                    .target("x86_64-unknown-linux-gnu")
                    .library_crate_compilation_unit_hash(*hash)
                    .build()
            }))
            .build();

        let response = server
            .post("/api/v1/cache/cargo/bulk/restore")
            .authorization_bearer(auth.token_alice().expose())
            .json(&bulk_request)
            .await;

        response.assert_status_ok();
        let bulk_response = response.json::<CargoBulkRestoreResponse>();

        pretty_assert_eq!(bulk_response.hits.len(), 2);
        pretty_assert_eq!(bulk_response.misses.len(), 2);

        // Verify hits are the ones we saved
        let mut hit_names = bulk_response
            .hits
            .iter()
            .map(|h| h.request.package_name.as_str())
            .collect::<Vec<_>>();
        hit_names.sort_unstable();
        pretty_assert_eq!(hit_names, vec!["serde", "tokio"]);

        // Verify misses are the ones we didn't save
        let mut miss_names = bulk_response
            .misses
            .iter()
            .map(|m| m.package_name.as_str())
            .collect::<Vec<_>>();
        miss_names.sort_unstable();
        pretty_assert_eq!(miss_names, vec!["axum", "reqwest"]);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn bulk_restore_all_misses(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        // Don't save anything, request nonexistent packages
        let bulk_request = CargoBulkRestoreRequest::builder()
            .requests([
                CargoRestoreRequest::builder()
                    .package_name("nonexistent1")
                    .package_version("1.0.0")
                    .target("x86_64-unknown-linux-gnu")
                    .library_crate_compilation_unit_hash("missing1")
                    .build(),
                CargoRestoreRequest::builder()
                    .package_name("nonexistent2")
                    .package_version("2.0.0")
                    .target("x86_64-unknown-linux-gnu")
                    .library_crate_compilation_unit_hash("missing2")
                    .build(),
            ])
            .build();

        let response = server
            .post("/api/v1/cache/cargo/bulk/restore")
            .authorization_bearer(auth.token_alice().expose())
            .json(&bulk_request)
            .await;

        response.assert_status_ok();
        let bulk_response = response.json::<CargoBulkRestoreResponse>();

        pretty_assert_eq!(bulk_response.hits.len(), 0);
        pretty_assert_eq!(bulk_response.misses.len(), 2);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn bulk_restore_empty_request(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let bulk_request = CargoBulkRestoreRequest::builder().build();

        let response = server
            .post("/api/v1/cache/cargo/bulk/restore")
            .authorization_bearer(auth.token_alice().expose())
            .json(&bulk_request)
            .await;

        response.assert_status_ok();
        let bulk_response = response.json::<CargoBulkRestoreResponse>();

        pretty_assert_eq!(bulk_response.hits.len(), 0);
        pretty_assert_eq!(bulk_response.misses.len(), 0);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn bulk_restore_with_build_scripts(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, key) = test_blob(b"proc_macro_content");

        // Save package with build script hashes
        let save_request = CargoSaveRequest::builder()
            .package_name("proc-macro")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("lib_hash")
            .build_script_compilation_unit_hash("build_comp_hash")
            .build_script_execution_unit_hash("build_exec_hash")
            .content_hash("content_hash")
            .artifacts([ArtifactFile::builder()
                .object_key(&key)
                .path("libproc_macro.rlib")
                .mtime_nanos(1000000000000000000u128)
                .executable(false)
                .build()])
            .build();

        let response = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&save_request)
            .await;
        response.assert_status(StatusCode::CREATED);

        // Bulk restore with matching hashes
        let bulk_request = CargoBulkRestoreRequest::builder()
            .requests([CargoRestoreRequest::builder()
                .package_name("proc-macro")
                .package_version("1.0.0")
                .target("x86_64-unknown-linux-gnu")
                .library_crate_compilation_unit_hash("lib_hash")
                .build_script_compilation_unit_hash("build_comp_hash")
                .build_script_execution_unit_hash("build_exec_hash")
                .build()])
            .build();

        let response = server
            .post("/api/v1/cache/cargo/bulk/restore")
            .authorization_bearer(auth.token_alice().expose())
            .json(&bulk_request)
            .await;

        response.assert_status_ok();
        let bulk_response = response.json::<CargoBulkRestoreResponse>();

        let expected_hit = CargoBulkRestoreHit::builder()
            .request(
                CargoRestoreRequest::builder()
                    .package_name("proc-macro")
                    .package_version("1.0.0")
                    .target("x86_64-unknown-linux-gnu")
                    .library_crate_compilation_unit_hash("lib_hash")
                    .build_script_compilation_unit_hash("build_comp_hash")
                    .build_script_execution_unit_hash("build_exec_hash")
                    .build(),
            )
            .artifacts([ArtifactFile::builder()
                .object_key(&key)
                .path("libproc_macro.rlib")
                .mtime_nanos(1000000000000000000u128)
                .executable(false)
                .build()])
            .build();

        let expected = CargoBulkRestoreResponse::builder()
            .hits([expected_hit])
            .build();

        pretty_assert_eq!(bulk_response, expected);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn bulk_restore_wrong_hashes(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        // Save packages with specific hashes
        let packages = vec![("correct", "1.0.0", "correct_hash")];

        for (name, version, hash) in &packages {
            let (_, key) = test_blob(format!("{name}_content").as_bytes());

            let save_request = CargoSaveRequest::builder()
                .package_name(*name)
                .package_version(*version)
                .target("x86_64-unknown-linux-gnu")
                .library_crate_compilation_unit_hash(*hash)
                .content_hash(format!("content_{hash}"))
                .artifacts([ArtifactFile::builder()
                    .object_key(&key)
                    .path(format!("lib{name}.rlib"))
                    .mtime_nanos(1000000000000000000u128)
                    .executable(false)
                    .build()])
                .build();

            let response = server
                .post("/api/v1/cache/cargo/save")
                .authorization_bearer(auth.token_alice().expose())
                .json(&save_request)
                .await;
            response.assert_status(StatusCode::CREATED);
        }

        // Request with mix of correct and incorrect hashes
        let bulk_request = CargoBulkRestoreRequest::builder()
            .requests([
                CargoRestoreRequest::builder()
                    .package_name("correct")
                    .package_version("1.0.0")
                    .target("x86_64-unknown-linux-gnu")
                    .library_crate_compilation_unit_hash("correct_hash")
                    .build(),
                CargoRestoreRequest::builder()
                    .package_name("correct")
                    .package_version("1.0.0")
                    .target("x86_64-unknown-linux-gnu")
                    .library_crate_compilation_unit_hash("wrong_hash")
                    .build(),
            ])
            .build();

        let response = server
            .post("/api/v1/cache/cargo/bulk/restore")
            .authorization_bearer(auth.token_alice().expose())
            .json(&bulk_request)
            .await;

        response.assert_status_ok();
        let bulk_response = response.json::<CargoBulkRestoreResponse>();

        // Only the correct hash should hit
        pretty_assert_eq!(bulk_response.hits.len(), 1);
        pretty_assert_eq!(bulk_response.misses.len(), 1);

        let expected_miss = CargoRestoreRequest::builder()
            .package_name("correct")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("wrong_hash")
            .build();

        pretty_assert_eq!(bulk_response.misses, vec![expected_miss]);
        pretty_assert_eq!(
            bulk_response.hits[0]
                .request
                .library_crate_compilation_unit_hash,
            "correct_hash"
        );

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn bulk_restore_same_package_different_targets(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let targets = vec![
            "x86_64-unknown-linux-gnu",
            "x86_64-apple-darwin",
            "aarch64-apple-darwin",
        ];

        // Save same package for different targets
        for (i, target) in targets.iter().enumerate() {
            let (_, key) = test_blob(format!("content_{target}").as_bytes());

            let save_request = CargoSaveRequest::builder()
                .package_name("cross-platform")
                .package_version("1.0.0")
                .target(*target)
                .library_crate_compilation_unit_hash(format!("hash_{i}"))
                .content_hash(format!("content_{i}"))
                .artifacts([ArtifactFile::builder()
                    .object_key(&key)
                    .path("libcross_platform.rlib")
                    .mtime_nanos(1000000000000000000u128)
                    .executable(false)
                    .build()])
                .build();

            let response = server
                .post("/api/v1/cache/cargo/save")
                .authorization_bearer(auth.token_alice().expose())
                .json(&save_request)
                .await;
            response.assert_status(StatusCode::CREATED);
        }

        // Bulk restore all targets
        let bulk_request = CargoBulkRestoreRequest::builder()
            .requests(targets.iter().enumerate().map(|(i, target)| {
                CargoRestoreRequest::builder()
                    .package_name("cross-platform")
                    .package_version("1.0.0")
                    .target(*target)
                    .library_crate_compilation_unit_hash(format!("hash_{i}"))
                    .build()
            }))
            .build();

        let response = server
            .post("/api/v1/cache/cargo/bulk/restore")
            .authorization_bearer(auth.token_alice().expose())
            .json(&bulk_request)
            .await;

        response.assert_status_ok();
        let bulk_response = response.json::<CargoBulkRestoreResponse>();

        pretty_assert_eq!(bulk_response.hits.len(), 3);
        pretty_assert_eq!(bulk_response.misses.len(), 0);

        // Verify each target is present
        let mut hit_targets = bulk_response
            .hits
            .iter()
            .map(|h| h.request.target.as_str())
            .collect::<Vec<_>>();
        hit_targets.sort_unstable();

        let mut expected_targets = targets.clone();
        expected_targets.sort_unstable();

        pretty_assert_eq!(hit_targets, expected_targets);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn bulk_restore_concurrent(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, key) = test_blob(b"concurrent_content");

        // Save one package
        let save_request = CargoSaveRequest::builder()
            .package_name("concurrent-test")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("hash")
            .content_hash("content_hash")
            .artifacts([ArtifactFile::builder()
                .object_key(&key)
                .path("libconcurrent.rlib")
                .mtime_nanos(1000000000000000000u128)
                .executable(false)
                .build()])
            .build();

        let response = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&save_request)
            .await;
        response.assert_status(StatusCode::CREATED);

        let bulk_request = CargoBulkRestoreRequest::builder()
            .requests([CargoRestoreRequest::builder()
                .package_name("concurrent-test")
                .package_version("1.0.0")
                .target("x86_64-unknown-linux-gnu")
                .library_crate_compilation_unit_hash("hash")
                .build()])
            .build();

        // Make 5 concurrent requests
        let (r1, r2, r3, r4, r5) = tokio::join!(
            server
                .post("/api/v1/cache/cargo/bulk/restore")
                .authorization_bearer(auth.token_alice().expose())
                .json(&bulk_request),
            server
                .post("/api/v1/cache/cargo/bulk/restore")
                .authorization_bearer(auth.token_alice().expose())
                .json(&bulk_request),
            server
                .post("/api/v1/cache/cargo/bulk/restore")
                .authorization_bearer(auth.token_alice().expose())
                .json(&bulk_request),
            server
                .post("/api/v1/cache/cargo/bulk/restore")
                .authorization_bearer(auth.token_alice().expose())
                .json(&bulk_request),
            server
                .post("/api/v1/cache/cargo/bulk/restore")
                .authorization_bearer(auth.token_alice().expose())
                .json(&bulk_request),
        );

        // All should succeed
        for response in [r1, r2, r3, r4, r5] {
            response.assert_status_ok();
            let bulk_response = response.json::<CargoBulkRestoreResponse>();
            pretty_assert_eq!(bulk_response.hits.len(), 1);
            pretty_assert_eq!(bulk_response.misses.len(), 0);
        }

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn bulk_restore_preserves_request_data(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, key) = test_blob(b"preserve_content");

        // Save one package
        let save_request = CargoSaveRequest::builder()
            .package_name("preserve-test")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("hash")
            .build_script_compilation_unit_hash("build_comp")
            .build_script_execution_unit_hash("build_exec")
            .content_hash("content_hash")
            .artifacts([ArtifactFile::builder()
                .object_key(&key)
                .path("libpreserve.rlib")
                .mtime_nanos(1000000000000000000u128)
                .executable(false)
                .build()])
            .build();

        let response = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&save_request)
            .await;
        response.assert_status(StatusCode::CREATED);

        let original_request = CargoRestoreRequest::builder()
            .package_name("preserve-test")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("hash")
            .build_script_compilation_unit_hash("build_comp")
            .build_script_execution_unit_hash("build_exec")
            .build();

        let bulk_request = CargoBulkRestoreRequest::builder()
            .requests([
                original_request.clone(),
                CargoRestoreRequest::builder()
                    .package_name("missing")
                    .package_version("2.0.0")
                    .target("aarch64-apple-darwin")
                    .library_crate_compilation_unit_hash("missing_hash")
                    .build(),
            ])
            .build();

        let response = server
            .post("/api/v1/cache/cargo/bulk/restore")
            .authorization_bearer(auth.token_alice().expose())
            .json(&bulk_request)
            .await;

        response.assert_status_ok();
        let bulk_response = response.json::<CargoBulkRestoreResponse>();

        // Verify hit contains full original request
        pretty_assert_eq!(bulk_response.hits.len(), 1);
        pretty_assert_eq!(bulk_response.hits[0].request, original_request);

        // Verify miss contains full original request
        pretty_assert_eq!(bulk_response.misses.len(), 1);
        pretty_assert_eq!(bulk_response.misses[0].package_name, "missing");
        pretty_assert_eq!(bulk_response.misses[0].package_version, "2.0.0");
        pretty_assert_eq!(bulk_response.misses[0].target, "aarch64-apple-darwin");

        Ok(())
    }
}
