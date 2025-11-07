//! Benchmarks for CAS transport operations (upload/download).
//!
//! These benchmarks measure the transfer rate of various CAS operations
//! across different data sizes and transport methods.
//!
//! ## Setup
//!
//! These benchmarks require a running Courier server. Set the server URL
//! using the `HURRY_COURIER_URL` environment variable:
//!
//! ```bash
//! export HURRY_COURIER_URL=http://localhost:3000
//! cargo bench --package courier
//! ```
//!
//! To start a local server:
//!
//! ```bash
//! docker compose up courier
//! ```

use clients::{
    Token,
    courier::v1::{Client, Key},
};
use futures::StreamExt;
use rand::RngCore;
use std::env;
use tokio::io::AsyncReadExt;
use url::Url;

const KB: usize = 1_024;
const MB: usize = 1_048_576;
const GB: usize = 1_073_741_824;

const SIZES: &[usize] = &[KB, 10 * KB, 100 * KB, MB, 10 * MB, 50 * MB, 100 * MB, GB];
const BULK_SIZES: &[usize] = &[KB, 10 * KB, 100 * KB, MB, 10 * MB];
const BULK_COUNTS: &[usize] = &[1, 10, 50, 100];

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

/// Test data generator for CAS benchmarks.
mod helpers {
    use super::*;

    /// Generate random test data of the specified size.
    ///
    /// Returns a tuple of (Key, Vec<u8>) where the key is the blake3 hash
    /// of the generated data.
    pub fn generate_test_data(size: usize) -> (Key, Vec<u8>) {
        let mut data = vec![0u8; size];
        rand::thread_rng().fill_bytes(&mut data);
        let key = Key::from_buffer(&data);
        (key, data)
    }
}

mod upload {
    use super::*;

    #[divan::bench(args = SIZES, sample_count = 5)]
    fn bytes(bencher: divan::Bencher, size: usize) {
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let client = Client::new(courier_url(), courier_token()).expect("create client");

        bencher
            .with_inputs(|| helpers::generate_test_data(size))
            .bench_values(|(key, data)| {
                runtime
                    .block_on(client.cas_write_bytes(&key, data))
                    .expect("upload");
            });
    }

    #[divan::bench(args = SIZES, sample_count = 5)]
    fn streaming(bencher: divan::Bencher, size: usize) {
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let client = Client::new(courier_url(), courier_token()).expect("create client");

        bencher
            .with_inputs(|| helpers::generate_test_data(size))
            .bench_values(|(key, data)| {
                runtime
                    .block_on(async {
                        let cursor = std::io::Cursor::new(data);
                        let reader = tokio_util::compat::FuturesAsyncReadCompatExt::compat(
                            futures::io::AllowStdIo::new(cursor),
                        );
                        client.cas_write(&key, reader).await
                    })
                    .expect("upload");
            });
    }

    #[divan::bench(args = BULK_SIZES, consts = BULK_COUNTS, sample_count = 5)]
    fn bulk<const COUNT: usize>(bencher: divan::Bencher, size: usize) {
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let client = Client::new(courier_url(), courier_token()).expect("create client");

        bencher
            .with_inputs(|| {
                (0..COUNT)
                    .map(|_| helpers::generate_test_data(size))
                    .collect::<Vec<_>>()
            })
            .bench_values(|items| {
                runtime
                    .block_on(async {
                        let stream = futures::stream::iter(items);
                        client.cas_write_bulk(stream).await
                    })
                    .expect("bulk upload");
            });
    }
}

mod download {
    use super::*;

    #[divan::bench(args = SIZES, sample_count = 5)]
    fn bytes(bencher: divan::Bencher, size: usize) {
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let client = Client::new(courier_url(), courier_token()).expect("create client");

        bencher
            .with_inputs(|| {
                let (key, data) = helpers::generate_test_data(size);
                runtime
                    .block_on(client.cas_write_bytes(&key, data))
                    .expect("pre-upload");
                key
            })
            .bench_values(|key| {
                runtime
                    .block_on(client.cas_read_bytes(&key))
                    .expect("download")
                    .expect("data exists");
            });
    }

    #[divan::bench(args = SIZES, sample_count = 5)]
    fn streaming(bencher: divan::Bencher, size: usize) {
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let client = Client::new(courier_url(), courier_token()).expect("create client");

        bencher
            .with_inputs(|| {
                let (key, data) = helpers::generate_test_data(size);
                runtime
                    .block_on(client.cas_write_bytes(&key, data))
                    .expect("pre-upload");
                key
            })
            .bench_values(|key| {
                runtime.block_on(async {
                    let mut reader = client.cas_read(&key).await.expect("read").expect("exists");
                    let mut buffer = Vec::new();
                    reader.read_to_end(&mut buffer).await.expect("read to end");
                    buffer
                });
            });
    }

    #[divan::bench(args = BULK_SIZES, consts = BULK_COUNTS, sample_count = 5)]
    fn bulk<const COUNT: usize>(bencher: divan::Bencher, size: usize) {
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let client = Client::new(courier_url(), courier_token()).expect("create client");

        bencher
            .with_inputs(|| {
                let items = (0..COUNT)
                    .map(|_| helpers::generate_test_data(size))
                    .collect::<Vec<_>>();

                for (key, data) in &items {
                    runtime
                        .block_on(client.cas_write_bytes(key, data.clone()))
                        .expect("pre-upload");
                }

                items.into_iter().map(|(key, _)| key).collect::<Vec<_>>()
            })
            .bench_values(|keys| {
                runtime.block_on(async {
                    let mut stream = client.cas_read_bulk(keys).await.expect("bulk read");
                    let mut count = 0;
                    while let Some(result) = stream.next().await {
                        result.expect("read item");
                        count += 1;
                    }
                    count
                });
            });
    }
}
