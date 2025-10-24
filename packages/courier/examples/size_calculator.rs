//! Calculate typical request/response sizes for bulk restore operations.
//!
//! This example calculates the JSON payload sizes for bulk cargo cache restore
//! operations at various scales. The results were used to determine the
//! server's request limit of 100,000 items (see `MAX_BULK_RESTORE_REQUESTS` in
//! `src/api/v1/cache/cargo/bulk_restore.rs`).
//!
//! ## Key Findings
//!
//! With typical package metadata (package name ~12 chars, version ~7 chars,
//! target 24 chars, hashes 16 chars) and artifact files (path ~55 chars,
//! key 64 chars, mtime, executable flag):
//!
//! - Single CargoRestoreRequest: ~234 bytes
//! - Single ArtifactFile: ~197 bytes
//!
//! Bulk request sizes (uncompressed):
//! - 100 items: ~23 KB
//! - 500 items: ~115 KB
//! - 1,000 items: ~230 KB
//! - 10,000 items: ~2.2 MB
//! - 100,000 items: ~22 MB
//!
//! Bulk response sizes (uncompressed, assuming 5 artifacts per hit):
//! - 100 hits: ~122 KB
//! - 500 hits: ~611 KB
//! - 1,000 hits: ~1.2 MB
//! - 10,000 hits: ~12 MB
//! - 100,000 hits: ~120 MB
//!
//! With HTTP compression enabled (which reduces JSON by ~70-80%), even 100k
//! items results in manageable payload sizes (~4-6 MB request, ~24-36 MB
//! response).
//!
//! ## Usage
//!
//! ```bash
//! cargo run --package courier --example size_calculator
//! ```

use clients::courier::v1::{
    Key,
    cache::{
        ArtifactFile, CargoBulkRestoreHit, CargoBulkRestoreRequest, CargoBulkRestoreResponse,
        CargoRestoreRequest,
    },
};

fn main() {
    println!("Bulk Restore Size Analysis");
    println!("==========================\n");

    // Typical values based on real cargo builds
    let typical_package_name = "serde_derive"; // ~12 chars
    let typical_version = "1.0.197"; // ~7 chars
    let typical_target = "x86_64-unknown-linux-gnu"; // 24 chars
    let typical_hash = "a1b2c3d4e5f6g7h8"; // 16 chars (typical hash prefix)
    let typical_key = Key::from_buffer(b"test"); // 64 char hex string
    let typical_path = "target/debug/deps/libserde_derive-a1b2c3d4e5f6g7h8.so"; // ~55 chars

    // Create a typical restore request
    let typical_request = CargoRestoreRequest::builder()
        .package_name(typical_package_name)
        .package_version(typical_version)
        .target(typical_target)
        .library_crate_compilation_unit_hash(typical_hash)
        .build();

    // Create a typical artifact
    let typical_artifact = ArtifactFile::builder()
        .object_key(&typical_key)
        .path(typical_path)
        .mtime_nanos(1000000000000000000u128)
        .executable(false)
        .build();

    // Serialize to see actual JSON size
    let request_json = serde_json::to_string(&typical_request).unwrap();
    let artifact_json = serde_json::to_string(&typical_artifact).unwrap();

    println!("Single Item Sizes:");
    println!("  CargoRestoreRequest: {} bytes", request_json.len());
    println!("  ArtifactFile:        {} bytes", artifact_json.len());
    println!();

    // Calculate bulk request sizes
    for count in [1, 10, 50, 100, 500, 1000, 5000, 10000] {
        let bulk_request = CargoBulkRestoreRequest::builder()
            .requests((0..count).map(|_| typical_request.clone()))
            .build();

        let json = serde_json::to_string(&bulk_request).unwrap();
        let kb = json.len() as f64 / 1024.0;
        let mb = kb / 1024.0;

        println!("Bulk Request with {} items:", count);
        println!("  Size: {} bytes ({:.2} KB, {:.3} MB)", json.len(), kb, mb);
    }

    println!();

    // Calculate bulk response sizes with varying artifact counts
    for (count, artifacts_per_hit) in [(100, 5), (500, 5), (1000, 5), (500, 10), (500, 50)] {
        let hits = (0..count)
            .map(|_| {
                CargoBulkRestoreHit::builder()
                    .request(typical_request.clone())
                    .artifacts((0..artifacts_per_hit).map(|_| typical_artifact.clone()))
                    .build()
            })
            .collect::<Vec<_>>();

        let bulk_response = CargoBulkRestoreResponse::builder().hits(hits).build();

        let json = serde_json::to_string(&bulk_response).unwrap();
        let kb = json.len() as f64 / 1024.0;
        let mb = kb / 1024.0;

        println!("Bulk Response with {count} hits × {artifacts_per_hit} artifacts:");
        println!("  Size: {} bytes ({:.2} KB, {:.3} MB)", json.len(), kb, mb);
    }
}
