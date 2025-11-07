//! Benchmarks for cargo cache restore operations.
//!
//! These benchmarks compare individual restore requests vs bulk restore
//! to measure the performance improvement of batching.
//!
//! ## Setup
//!
//! These benchmarks require a running Courier server. Set the server URL
//! using the `HURRY_COURIER_URL` environment variable:
//!
//! ```bash
//! export HURRY_COURIER_URL=http://localhost:3000
//! cargo bench --package courier --bench cache_restore
//! ```
//!
//! To start a local server:
//!
//! ```bash
//! docker compose up courier
//! ```

use clients::{
    Token,
    courier::v1::{
        Client,
        cache::{ArtifactFile, CargoRestoreRequest, CargoSaveRequest},
    },
};
use std::env;
use url::Url;

const PACKAGE_COUNTS: &[usize] = &[1, 5, 10, 25, 50, 100, 1000, 10000];

fn main() {
    divan::main();
}

/// Get the Courier URL from environment or panic with a helpful message.
fn courier_url() -> Url {
    env::var("HURRY_COURIER_URL")
        .expect("HURRY_COURIER_URL must be set to run benchmarks")
        .parse()
        .expect("HURRY_COURIER_URL must be a valid URL")
}

/// Get the Courier token from environment or panic with a helpful message.
fn courier_token() -> Token {
    env::var("HURRY_COURIER_TOKEN")
        .expect("HURRY_COURIER_TOKEN must be set to run benchmarks")
        .parse()
        .expect("HURRY_COURIER_TOKEN must be a valid token")
}

/// Test data helpers for cache benchmarks.
mod helpers {
    use super::*;
    use clients::courier::v1::Key;

    /// Generate a test cache entry for a package.
    ///
    /// Returns (save_request, restore_request) tuple.
    pub fn generate_cache_entry(
        package_index: usize,
    ) -> (CargoSaveRequest, CargoRestoreRequest, Key) {
        let package_name = format!("test-package-{package_index}");
        let package_version = String::from("1.0.0");
        let target = String::from("x86_64-unknown-linux-gnu");
        let lib_hash = format!("lib_hash_{package_index}");
        let content_hash = format!("content_hash_{package_index}");

        // Generate test artifact data
        let artifact_data = format!("artifact_content_{package_index}").into_bytes();
        let key = Key::from_buffer(&artifact_data);

        let artifact = ArtifactFile::builder()
            .object_key(&key)
            .path(format!("lib{package_name}.rlib"))
            .mtime_nanos(1000000000000000000u128)
            .executable(false)
            .build();

        let save_request = CargoSaveRequest::builder()
            .package_name(&package_name)
            .package_version(&package_version)
            .target(&target)
            .library_crate_compilation_unit_hash(&lib_hash)
            .content_hash(&content_hash)
            .artifacts([artifact])
            .build();

        let restore_request = CargoRestoreRequest::builder()
            .package_name(&package_name)
            .package_version(&package_version)
            .target(&target)
            .library_crate_compilation_unit_hash(&lib_hash)
            .build();

        (save_request, restore_request, key)
    }
}

mod restore {
    use super::*;

    /// Benchmark restoring packages one at a time (current approach).
    #[divan::bench(args = PACKAGE_COUNTS, sample_count = 5)]
    fn individual(bencher: divan::Bencher, count: usize) {
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let client = Client::new(courier_url(), courier_token()).expect("create client");

        bencher
            .with_inputs(|| {
                // Setup: save all packages first
                let entries = (0..count)
                    .map(helpers::generate_cache_entry)
                    .collect::<Vec<_>>();

                for (i, (save_request, _, key)) in entries.iter().enumerate() {
                    // Write the artifact blob to CAS
                    let artifact_data = format!("artifact_content_{i}").into_bytes();
                    runtime
                        .block_on(client.cas_write_bytes(key, artifact_data))
                        .expect("write artifact");

                    // Save the cache entry
                    runtime
                        .block_on(client.cargo_cache_save(save_request.clone()))
                        .expect("save cache");
                }

                entries
                    .into_iter()
                    .map(|(_, restore_request, _)| restore_request)
                    .collect::<Vec<_>>()
            })
            .bench_values(|restore_requests| {
                runtime.block_on(async {
                    for request in restore_requests {
                        client
                            .cargo_cache_restore(request)
                            .await
                            .expect("restore cache");
                    }
                });
            });
    }

    /// Benchmark restoring packages in bulk (new approach).
    #[divan::bench(args = PACKAGE_COUNTS, sample_count = 5)]
    fn bulk(bencher: divan::Bencher, count: usize) {
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let client = Client::new(courier_url(), courier_token()).expect("create client");

        bencher
            .with_inputs(|| {
                // Setup: save all packages first
                let entries = (0..count)
                    .map(helpers::generate_cache_entry)
                    .collect::<Vec<_>>();

                for (i, (save_request, _, key)) in entries.iter().enumerate() {
                    // Write the artifact blob to CAS
                    let artifact_data = format!("artifact_content_{i}").into_bytes();
                    runtime
                        .block_on(client.cas_write_bytes(key, artifact_data))
                        .expect("write artifact");

                    // Save the cache entry
                    runtime
                        .block_on(client.cargo_cache_save(save_request.clone()))
                        .expect("save cache");
                }

                entries
                    .into_iter()
                    .map(|(_, restore_request, _)| restore_request)
                    .collect::<Vec<_>>()
            })
            .bench_values(|restore_requests| {
                runtime.block_on(async {
                    client
                        .cargo_cache_restore_bulk(restore_requests)
                        .await
                        .expect("bulk restore cache");
                });
            });
    }
}
