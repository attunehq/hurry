//! Hashing operations and types.

use color_eyre::{Result, eyre::Context};
use tokio::io::AsyncReadExt;
use tracing::{instrument, trace};

use client::courier::v1::Key;

use crate::{fs, path::AbsFilePath};

/// Hash the contents of the file at the specified path.
#[instrument(name = "hash_file")]
pub async fn hash_file(path: &AbsFilePath) -> Result<Key> {
    let mut file = fs::open_file(path).await.context("open file")?;
    let mut hasher = blake3::Hasher::new();
    let mut data = vec![0; 64 * 1024];
    let mut bytes = 0;
    loop {
        let len = file.read(&mut data).await.context("read chunk")?;
        if len == 0 {
            break;
        }
        hasher.update(&data[..len]);
        bytes += len;
    }
    let hash = hasher.finalize();
    let key = Key::from_blake3_hash(hash);
    trace!(?path, hash = %key, ?bytes, "hash file");
    Ok(key)
}
