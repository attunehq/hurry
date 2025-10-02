use std::path::PathBuf;

use color_eyre::Result;

pub struct Disk {
    root: PathBuf,
}

impl Disk {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn key_path(&self, _key: &[u8]) -> PathBuf {
        todo!("1. Convert key to hex string");
        todo!("2. Use first 2 chars as first level dir");
        todo!("3. Use second 2 chars as second level dir");
        todo!("4. Return path: root/ab/cd/abcd1234...");
    }

    pub async fn exists(&self, _key: &[u8]) -> bool {
        todo!("Check if blob file exists at key_path")
    }

    pub async fn read(&self, _key: &[u8]) -> Result<Vec<u8>> {
        todo!("1. Read blob from key_path");
        todo!("2. Decompress with zstd");
        todo!("3. Return raw content");
    }

    pub async fn write(&self, _key: &[u8], _content: &[u8]) -> Result<()> {
        todo!("1. Check if blob already exists; if so return early");
        todo!("2. Compress content with zstd level 3");
        todo!("3. Write to temporary file");
        todo!("4. Rename to final destination (atomic, idempotent)");
        todo!("5. Create parent directories if needed");
    }
}
