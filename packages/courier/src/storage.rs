use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use async_compression::Level;
use async_compression::tokio::bufread::ZstdDecoder;
use async_compression::tokio::write::ZstdEncoder;
use clients::{LOCAL_BUFFER_SIZE, NETWORK_BUFFER_SIZE};
use color_eyre::eyre::bail;
use color_eyre::{Result, eyre::Context};
use derive_more::{Debug, Display};
use tap::Pipe;
use tokio::fs::{File, create_dir_all, metadata, remove_file, rename};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio_util::compat::{FuturesAsyncReadCompatExt, FuturesAsyncWriteCompatExt};
use tracing::warn;
use uuid::Uuid;

pub use clients::courier::v1::Key;

/// Implements the CAS storage interface on disk.
///
/// ## File structure
///
/// The CAS is a two-level directory structure of files where each file is named
/// for the hex encoded representation of the Blake3 hash of the file content.
/// Each file is prefixed with two levels of folders computed from the first two
/// and next two characters of the hex representation of [`Key`].
///
/// No path details are exposed from the CAS on purpose: instead, users must use
/// the methods on this struct to interact with files inside the CAS.
///
/// ## Compression
///
/// The CAS transparently compresses the content of each file with zstd level 3.
/// Users should always write the uncompressed content to the CAS; reads get the
/// same content that was written.
///
/// ## Idempotency
///
/// The CAS is idempotent: if a file already exists, it is not written again.
/// This is safe because the key is computed from the content of the file, so if
/// the file already exists it must have the same content.
///
/// ## Atomic writes
///
/// The CAS uses write-then-rename to ensure that writes are atomic. If a file
/// already exists, it is not written again. This is safe because the key is
/// computed from the content of the file, so if the file already exists it must
/// have the same content.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display)]
#[debug("Disk(root = {})", self.root.display())]
#[display("{}", root.display())]
pub struct Disk {
    root: PathBuf,
}

impl Disk {
    /// The default buffer size to use for reading and writing.
    ///
    /// We use a relatively large buffer size because:
    /// - We assume we're typically reading/writing relatively large blobs (in
    ///   the megabytes) mostly backed by network transfers.
    /// - The `Blake3` hash implementation benefits from SIMD instructions if we
    ///   feed it larger chunks.
    ///
    /// At the same time we don't want to go overboard: copying the buffer can't
    /// be so large that it blocks the event loop for too long, and we're
    /// serving many clients at once.
    const DEFAULT_BUF_SIZE: usize = 64 * 1024;

    /// Create a new instance in the provided directory.
    ///
    /// If the directory does not already exist, it is created when the first
    /// file is written to the CAS instance.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Create a new instance in a temporary directory.
    #[cfg(test)]
    pub async fn new_temp() -> Result<(Self, async_tempfile::TempDir)> {
        let root = async_tempfile::TempDir::new()
            .await
            .context("create temp directory")?;
        Ok((Self::new(root.dir_path()), root))
    }

    /// Validate that the CAS is accessible and writable.
    #[tracing::instrument(name = "Disk::ping")]
    pub async fn ping(&self) -> Result<()> {
        static PING_KEY: LazyLock<Key> = LazyLock::new(|| Key::from_blake3(blake3::hash(b"ping")));
        const PING_CONTENT: &[u8] = b"ping";

        self.write_buffered(&PING_KEY, PING_CONTENT).await?;
        let content = self.read_buffered(&PING_KEY).await?;
        if content != PING_CONTENT {
            bail!(
                "ping CAS failed; unexpected content: {}",
                String::from_utf8_lossy(&content)
            );
        }

        Ok(())
    }

    /// Get the path to the file for the given key.
    ///
    /// Example:
    /// ```not_rust
    /// Key("abcd1234...") -> root/ab/cd/abcd1234...
    /// ```
    ///
    /// Note: this is a method on `Disk` rather than on `Key` because in the
    /// future we may add other kinds of storage implementations, and this is
    /// unique to the `Disk` implementation.
    fn key_path(&self, key: &Key) -> PathBuf {
        // We use two-level prefixes to keep folder sizes relatively small,
        // since huge folders containing millions of files can have performance
        // issues depending on the filesystem.
        //
        // This also allows us to add new volumes at different levels in the
        // future if we need to do so for storage or other reasons.
        let hex = key.to_hex();
        let prefix1 = hex.chars().take(2).collect::<String>();
        let prefix2 = hex.chars().skip(2).take(2).collect::<String>();
        self.root.join(prefix1).join(prefix2).join(&hex)
    }

    /// Check if a blob exists in storage.
    ///
    /// Normally "exists" checks are prone to race conditions (commonly known as
    /// "TOCTOU" or "Time of Check, Time of Use"), but for the CAS it's safe
    /// because:
    /// - Blobs are always stored by their key, and their key is derived from
    ///   their content; this means that once a blob is written there's never a
    ///   reason to write it again or otherwise modify it.
    /// - In the current design we don't ever delete blobs. In the future we may
    ///   do so and in that world we may no longer be able to trust "exists"
    ///   checks, but that's not the world we're in today.
    ///
    /// Returns `Ok(true)` if the key exists, `Ok(false)` if it does not exist,
    /// and `Err` if there was an error checking (e.g., permission denied).
    #[tracing::instrument(name = "Disk::exists")]
    pub async fn exists(&self, key: &Key) -> Result<bool> {
        let path = self.key_path(key);
        match metadata(&path).await {
            Ok(_) => Ok(true),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err).context(format!("check if blob exists at {path:?}")),
        }
    }

    /// Get the size of the content for the provided key.
    #[tracing::instrument(name = "Disk::size")]
    pub async fn size(&self, key: &Key) -> Result<Option<u64>> {
        let path = self.key_path(key);
        let size = self
            .read_size(key)
            .await
            .with_context(|| format!("read size for {key:?}"))?;

        match size {
            Some(size) => Ok(Some(size)),
            None => match self.read_inner(key).await {
                Ok(mut content) => {
                    let size = tokio::io::copy(&mut content, &mut tokio::io::empty())
                        .await
                        .with_context(|| format!("read content for {key:?}"))?;
                    self.write_size(key, size)
                        .await
                        .with_context(|| format!("write size for {key:?}"))?;
                    Ok(Some(size))
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(err) => Err(err).context(format!("read content of blob at {path:?}")),
            },
        }
    }

    /// Get the compressed size of the content for the provided key.
    ///
    /// [`Disk::size`] reports the uncompressed size of the data. This method,
    /// in contrast, reports the compressed size of the data.
    #[tracing::instrument(name = "Disk::size_compressed")]
    pub async fn size_compressed(&self, key: &Key) -> Result<Option<u64>> {
        let path = self.key_path(key);
        match tokio::fs::metadata(&path).await {
            Ok(metadata) => Ok(Some(metadata.len())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err).context(format!("read metadata of blob at {path:?}")),
        }
    }

    /// Read the size of the content for the provided key, if it exists.
    #[tracing::instrument(name = "Disk::read_size")]
    async fn read_size(&self, key: &Key) -> Result<Option<u64>> {
        let path = self.key_path(key);
        let size_path = path.with_extension("size");
        match tokio::fs::read(&size_path).await {
            Ok(bytes) => match bytes.try_into().map(u64::from_be_bytes) {
                Ok(size) => Ok(Some(size)),
                Err(buf) => bail!("invalid big-endian u64: {buf:?}"),
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).context(format!("read size of blob at {path:?}")),
        }
    }

    /// Best-effort: write the size of the content for the provided key.
    ///
    /// Since we store the files compressed, later when we need their size, we
    /// can't simply check the size- we have to read the `{key}.size` file.
    /// This method creates that file.
    ///
    /// If this method fails, it's logged as a warning.
    #[tracing::instrument]
    async fn write_size(&self, key: &Key, size: u64) -> Result<()> {
        let path = self.key_path(key);
        let size_path = path.with_extension("size");
        tokio::fs::write(&size_path, &size.to_be_bytes())
            .await
            .with_context(|| format!("write size file at {size_path:?}"))
    }

    /// Read the content from storage for the provided key.
    ///
    /// Note: the returned reader is buffered with the capacity of
    /// [`Disk::DEFAULT_BUF_SIZE`]; callers should probably not buffer further.
    #[tracing::instrument(name = "Disk::read")]
    pub async fn read(&self, key: &Key) -> Result<impl AsyncRead + Unpin + 'static> {
        self.read_inner(key)
            .await
            .with_context(|| format!("open blob file {:?}", self.key_path(key)))
    }

    /// Read the compressed content from storage for the provided key.
    ///
    /// [`Disk::read`] works with the data in its uncompressed form, meaning
    /// that compression is transparently handled. This method, in contrast,
    /// reads the compressed data from disk without decompressing it.
    ///
    /// Note: the returned reader is buffered with the capacity of
    /// [`Disk::DEFAULT_BUF_SIZE`]; callers should probably not buffer further.
    #[tracing::instrument(name = "Disk::read_compressed")]
    pub async fn read_compressed(&self, key: &Key) -> Result<impl AsyncRead + Unpin + 'static> {
        let path = self.key_path(key);
        File::open(&path)
            .await
            .map(|reader| BufReader::with_capacity(Self::DEFAULT_BUF_SIZE, reader))
            .with_context(|| format!("open blob file {:?}", self.key_path(key)))
    }

    /// Write the content to storage for the provided key.
    ///
    /// Note: This method does NOT check if the key already exists. Callers
    /// should check via `exists()` first if they want to avoid unnecessary
    /// work. The method will handle the AlreadyExists case gracefully
    /// during the final rename operation.
    #[tracing::instrument(name = "Disk::write", skip(content))]
    pub async fn write(&self, key: &Key, content: impl AsyncRead + Unpin) -> Result<()> {
        let path = self.key_path(key);

        if let Some(parent) = path.parent() {
            create_dir_all(parent)
                .await
                .with_context(|| format!("create parent directory {parent:?} for {path:?}"))?;
        }

        // We want to try to saturate write buffers as much as possible: for
        // example the `Blake3` hash implementation benefits from SIMD
        // instructions if we feed it larger chunks.
        let mut content = BufReader::with_capacity(Self::DEFAULT_BUF_SIZE, content);

        // We need to write the content to a temporary file first:
        // - Once the file exists in its final destination it's assumed that it'll never
        //   change, so we can't partially write the content.
        // - Other instances could be trying to write the same file at the same time; a
        //   rename is atomic but a partial write is not.
        let temp = temp_path(&path);
        let file = File::create(&temp).await.context("create temporary file")?;

        // We don't have solid data on a better default for zstd compression
        // level, so we start with the default.
        let mut encoder = ZstdEncoder::with_quality(file, Level::Default);

        // While we're writing we also need to compute the hash of the content
        // to make sure that it actually matches the key we were provided.
        let (hash, size) = hashed_copy(&mut content, &mut encoder)
            .await
            .with_context(|| format!("write content to {temp:?}"))?;

        // Even if the hash didn't match we still need to finalize the write so
        // that we can delete the temp file before returning.
        encoder.shutdown().await.context("flush zstd encoder")?;
        let mut file = encoder.into_inner();
        file.flush().await.context("flush file")?;
        drop(file);

        if key != hash {
            if let Err(err) = remove_file(&temp).await {
                warn!("failed to remove temp file {temp:?}: {err}");
            }
            bail!("hash mismatch: {hash:?} != {key:?}");
        }

        // Atomically rename the temp file to the final destination.
        // If the file already exists, we can just abort: file contents never
        // change and are always named by their content hash.
        match rename(&temp, &path).await {
            Ok(()) => self
                .write_size(key, size)
                .await
                .with_context(|| format!("write size for {key:?}")),
            Err(err) => {
                if let Err(err) = remove_file(&temp).await {
                    warn!("failed to remove temp file {temp:?}: {err}");
                }
                if err.kind() == std::io::ErrorKind::AlreadyExists {
                    Ok(())
                } else {
                    Err(err).context(format!("rename {temp:?} to {path:?}"))
                }
            }
        }
    }

    /// Write the compressed content to storage for the provided key.
    ///
    /// [`Disk::write`] works with the data in its uncompressed form, meaning
    /// that compression is transparently handled. This method, in contrast,
    /// expects the data to already be compressed when writing.
    ///
    /// Important: [`Disk`] assumes `zstd` compression. As such, this method
    /// also validates that the data provided is able to be decompressed using a
    /// `zstd` decoder. This is done mainly in order to validate the content key
    /// (which is expected to be generated from the _uncompressed_ content) but
    /// it also has the happy side effect of not allowing anything to be written
    /// here which can't be read later using [`Disk::read`].
    ///
    /// Note: This method does NOT check if the key already exists. Callers
    /// should check via `exists()` first if they want to avoid unnecessary
    /// work. The method will handle the AlreadyExists case gracefully
    /// during the final rename operation.
    #[tracing::instrument(name = "Disk::write_compressed", skip(content))]
    pub async fn write_compressed(&self, key: &Key, content: impl AsyncRead + Unpin) -> Result<()> {
        let path = self.key_path(key);

        if let Some(parent) = path.parent() {
            create_dir_all(parent)
                .await
                .with_context(|| format!("create parent directory {parent:?} for {path:?}"))?;
        }

        // We want to try to saturate write buffers as much as possible: for
        // example the `Blake3` hash implementation benefits from SIMD
        // instructions if we feed it larger chunks.
        let mut content = BufReader::with_capacity(Self::DEFAULT_BUF_SIZE, content);

        // We need to write the content to a temporary file first:
        // - Once the file exists in its final destination it's assumed that it'll never
        //   change, so we can't partially write the content.
        // - Other instances could be trying to write the same file at the same time; a
        //   rename is atomic but a partial write is not.
        let temp = temp_path(&path);
        let mut file = File::create(&temp).await.context("create temporary file")?;

        // While we're writing we also need to compute the hash of the content
        // to make sure that it actually matches the key we were provided.
        let (hash, size) = hashed_copy_compressed(&mut content, &mut file)
            .await
            .with_context(|| format!("write content to {temp:?}"))?;

        // Even if the hash didn't match we still need to finalize the write so
        // that we can delete the temp file before returning.
        file.flush().await.context("flush file")?;
        drop(file);

        if key != hash {
            if let Err(err) = remove_file(&temp).await {
                warn!("failed to remove temp file {temp:?}: {err}");
            }
            bail!("hash mismatch of uncompressed content: {hash:?} != {key:?}");
        }

        // Atomically rename the temp file to the final destination.
        // If the file already exists, we can just abort: file contents never
        // change and are always named by their content hash.
        match rename(&temp, &path).await {
            Ok(()) => self
                .write_size(key, size)
                .await
                .with_context(|| format!("write uncompressed size for {key:?}")),
            Err(err) => {
                if let Err(err) = remove_file(&temp).await {
                    warn!("failed to remove temp file {temp:?}: {err}");
                }
                if err.kind() == std::io::ErrorKind::AlreadyExists {
                    Ok(())
                } else {
                    Err(err).context(format!("rename {temp:?} to {path:?}"))
                }
            }
        }
    }

    /// Read and buffer the entire content from storage.
    async fn read_buffered(&self, key: &Key) -> Result<Vec<u8>> {
        let mut content = self.read(key).await?;
        let mut buffer = Vec::new();
        tokio::io::copy(&mut content, &mut buffer)
            .await
            .with_context(|| "read decompressed blob content")?;
        Ok(buffer)
    }

    /// Write buffered content to storage.
    async fn write_buffered(&self, key: &Key, content: impl AsRef<[u8]>) -> Result<()> {
        let cursor = Cursor::new(content.as_ref());
        self.write(key, cursor).await
    }

    async fn read_inner(
        &self,
        key: &Key,
    ) -> Result<impl AsyncRead + Unpin + 'static, std::io::Error> {
        let path = self.key_path(key);
        File::open(&path)
            .await
            .map(BufReader::new)
            .map(ZstdDecoder::new)
            .map(|reader| BufReader::with_capacity(Self::DEFAULT_BUF_SIZE, reader))
    }
}

/// Generate a temporary file path in the same directory as the target.
///
/// We do this instead of using a prebuilt tempfile crate (like
/// `async_tempfile` or `tempfile`) because:
/// - We need to persist the tempfile to disk after writing it; `async_tempfile`
///   doesn't support this at all.
/// - Other solutions like `tempfile` do support this, but don't allow
///   persisting across file systems; due to the fact that the CAS is mounted on
///   a network file system this will likely be a problem.
///
/// Unfortunately this does open the application to an issue where if it opens a
/// tempfile and then crashes before finishing it can leave an orphaned file.
/// We'll just have to deal with this using cleanup logic if it becomes a
/// problem; since we have a max request deadline in the API we can reasonably
/// assume that temp files still alive after a generous time period like a day
/// or something can be cleaned up.
fn temp_path(target: &Path) -> PathBuf {
    let mut temp = target.as_os_str().to_owned();
    temp.push(".tmp.");
    temp.push(Uuid::new_v4().to_string());
    PathBuf::from(temp)
}

/// Copy the content from the source reader into the target writer while
/// computing the hash of the copied content.
///
/// Returns the hash of the content and the number of bytes copied.
async fn hashed_copy(
    mut source: impl AsyncRead + Unpin,
    mut target: impl AsyncWrite + Unpin,
) -> Result<(blake3::Hash, u64)> {
    // We set the buffer size to this value because it's called out by the
    // `blake3` docs on the `update_reader` method:
    // https://docs.rs/blake3/1.8.2/blake3/struct.Hasher.html#method.update_reader
    //
    // At the same time we don't necessarily want to write out 64KB at a time
    // because we don't want to take too long between `await` calls and block
    // the runtime, and the Blake3 docs imply that it won't benefit from a
    // buffer larger than 16KB.
    let mut buffer = vec![0; 16 * 1024];
    let mut hasher = blake3::Hasher::new();
    let mut copied = 0;
    loop {
        let n = source.read(&mut buffer).await.context("read source")?;
        if n == 0 {
            break;
        }

        let chunk = &buffer[..n];
        hasher.update(chunk);
        target.write_all(chunk).await.context("write target")?;
        copied += n as u64;
    }

    Ok((hasher.finalize(), copied))
}

/// Copy the content from the source reader into the target writer while
/// simultaneously decompressing the source reader and computing the hash of the
/// decompressed content.
///
/// The decompressed content is only used for calculating the hash; the
/// compressed content is what's written to the destination.
///
/// Returns the hash of the _uncompressed_ content and the number of
/// _uncompressed_ bytes copied. The intention of this is to enable the
/// compressed and uncompressed disk APIs to smoothly interoperate: for example
/// [`Disk::write_compressed`] needs to know the uncompressed size so that it
/// can write a size file that [`Disk::size`] can read. Meanwhile
/// [`Disk::size_compressed`] doesn't need to know the size ahead of time, as it
/// can just check the metadata of the actual file on disk.
async fn hashed_copy_compressed(
    mut source: impl AsyncRead + Unpin,
    mut target: impl AsyncWrite + Unpin,
) -> Result<(blake3::Hash, u64)> {
    // We set the buffer size to this value because it's called out by the
    // `blake3` docs on the `update_reader` method:
    // https://docs.rs/blake3/1.8.2/blake3/struct.Hasher.html#method.update_reader
    let (tee_reader, tee_writer) = piper::pipe(NETWORK_BUFFER_SIZE);

    let copy = async move || -> Result<()> {
        let mut tee = tee_writer.compat_write();
        let mut buffer = vec![0; LOCAL_BUFFER_SIZE];
        loop {
            let n = source.read(&mut buffer).await.context("read source")?;
            if n == 0 {
                break;
            }

            let chunk = &buffer[..n];
            target.write_all(chunk).await.context("write target")?;
            tee.write_all(chunk).await.context("write tee")?;
        }
        Ok(())
    };

    let hash = async move || -> Result<(blake3::Hash, u64)> {
        let mut tee = tee_reader
            .compat()
            .pipe(BufReader::new)
            .pipe(ZstdDecoder::new);
        let mut buffer = vec![0; LOCAL_BUFFER_SIZE];
        let mut hasher = blake3::Hasher::new();
        let mut copied = 0;
        loop {
            let n = tee.read(&mut buffer).await.context("read tee")?;
            if n == 0 {
                break;
            }
            let chunk = &buffer[..n];
            hasher.update(chunk);
            copied += n as u64;
        }
        Ok((hasher.finalize(), copied))
    };

    let (metrics, _) = tokio::try_join!(hash(), copy())?;
    Ok(metrics)
}

#[cfg(test)]
mod tests {
    use super::{Disk, Key, hashed_copy};
    use color_eyre::Result;
    use pretty_assertions::assert_eq as pretty_assert_eq;
    use proptest::{prop_assert, prop_assert_eq, prop_assert_ne, prop_assume};
    use simple_test_case::test_case;
    use std::io::Cursor;
    use test_strategy::proptest;
    use tokio::io::AsyncReadExt;

    #[track_caller]
    fn compress(data: impl AsRef<[u8]>) -> Vec<u8> {
        zstd::bulk::compress(data.as_ref(), 0).expect("compress")
    }

    #[track_caller]
    fn decompress(data: impl AsRef<[u8]>) -> Vec<u8> {
        zstd::bulk::decompress(data.as_ref(), 100 * 1024 * 1024).expect("decompress")
    }

    fn key_for(input: &[u8]) -> Key {
        Key::from_blake3(blake3::hash(input))
    }

    #[test_case(Vec::from(b"hello world\n"); "short input")]
    #[test_case(Vec::from(b"hello world\n").repeat(10000); "long input")]
    #[test_log::test(tokio::test)]
    async fn hashed_copy(input: Vec<u8>) -> Result<()> {
        let _ = color_eyre::install();

        let mut output = Vec::new();
        let (hash, _) = super::hashed_copy(Cursor::new(&input), &mut output).await?;

        pretty_assert_eq!(
            hex::encode(&input),
            hex::encode(output),
            "copies content faithfully"
        );

        let expected_hash = blake3::hash(&input);
        pretty_assert_eq!(hash, expected_hash, "computes the correct hash");

        Ok(())
    }

    #[proptest(async = "tokio")]
    async fn hashed_copy_arbitrary(#[any] input: Vec<u8>) {
        let mut output = Vec::new();
        let (hash, _) = super::hashed_copy(Cursor::new(&input), &mut output)
            .await
            .expect("hashed copy");

        prop_assert_eq!(
            hex::encode(&input),
            hex::encode(output),
            "copies content faithfully"
        );

        let expected_hash = blake3::hash(&input);
        prop_assert_eq!(hash, expected_hash, "computes the correct hash");
    }

    #[proptest(async = "tokio")]
    async fn write_read_roundtrip(#[any] content: Vec<u8>) -> Result<()> {
        let _ = color_eyre::install();

        let (storage, _temp) = Disk::new_temp().await?;

        let key = key_for(&content);
        storage.write_buffered(&key, &content).await?;
        pretty_assert_eq!(storage.exists(&key).await?, true);

        let read_content = storage.read_buffered(&key).await?;
        pretty_assert_eq!(read_content, content);

        Ok(())
    }

    #[proptest(async = "tokio")]
    async fn write_idempotent(#[any] content: Vec<u8>) -> Result<()> {
        let _ = color_eyre::install();

        let (storage, _temp) = Disk::new_temp().await?;

        let key = key_for(&content);
        storage.write_buffered(&key, &content).await?;
        storage.write_buffered(&key, &content).await?;

        let read_content = storage.read_buffered(&key).await?;
        pretty_assert_eq!(read_content, content);

        Ok(())
    }

    #[proptest(async = "tokio")]
    async fn write_concurrent(#[any] content: Vec<u8>) -> Result<()> {
        let _ = color_eyre::install();

        let (storage, _temp) = Disk::new_temp().await?;

        let key = key_for(&content);
        tokio::try_join!(
            storage.write_buffered(&key, &content),
            storage.write_buffered(&key, &content)
        )?;

        let read_content = storage.read_buffered(&key).await?;
        pretty_assert_eq!(read_content, content);

        Ok(())
    }

    #[test_log::test(tokio::test)]
    async fn nonexistent() -> Result<()> {
        let _ = color_eyre::install();

        let (storage, _temp) = Disk::new_temp().await?;

        let key = key_for(b"nonexistent");

        assert!(!storage.exists(&key).await?);
        assert!(storage.read_buffered(&key).await.is_err());

        Ok(())
    }

    #[proptest(async = "tokio")]
    async fn verify_directory_structure(#[any] content: Vec<u8>) {
        let (storage, temp) = Disk::new_temp().await.expect("temp dir");

        let key = key_for(&content);
        let key_hex = key.to_hex();
        storage.write_buffered(&key, &content).await.expect("write");

        let expected_path = temp
            .dir_path()
            .join(&key_hex[0..2]) // First 2 chars
            .join(&key_hex[2..4]) // Second 2 chars
            .join(&key_hex); // Full key

        assert!(
            expected_path.exists(),
            "Expected path {expected_path:?} does not exist",
        );

        // Content should be compressed, so we don't expect it to match.
        let file_contents = std::fs::read(&expected_path).expect("read");
        prop_assert_ne!(&file_contents, &content);

        // But decompressing gives us the original
        prop_assert_eq!(decompress(&file_contents), content);
    }

    #[proptest(async = "tokio")]
    async fn multiple_blobs(#[any] blobs: Vec<Vec<u8>>) {
        let (storage, _temp) = Disk::new_temp().await.expect("temp dir");

        for content in &blobs {
            let key = key_for(content);
            storage.write_buffered(&key, &content).await.expect("write");
        }

        for content in blobs {
            let key = key_for(&content);
            let read_content = storage.read_buffered(&key).await.expect("read");
            prop_assert_eq!(read_content, content);
        }
    }

    /// The test helper `write_buffered` and `read_buffered` use the streaming
    /// API internally so this test is mainly just to double check that they
    /// work as expected.
    #[proptest(async = "tokio")]
    async fn streaming_roundtrip(#[any] content: Vec<u8>) {
        let (storage, _temp) = Disk::new_temp().await.expect("temp dir");

        let key = key_for(&content);
        let cursor = Cursor::new(&content);
        storage.write(&key, cursor).await.expect("write");

        let mut reader = storage.read(&key).await.expect("read");
        let mut read_content = Vec::new();
        reader
            .read_to_end(&mut read_content)
            .await
            .expect("read to end");

        prop_assert_eq!(read_content, content);
    }

    #[proptest(async = "tokio")]
    async fn exists(#[any] content: Vec<u8>) {
        let (storage, _temp) = Disk::new_temp().await.expect("temp dir");

        let key = key_for(&content);
        prop_assert!(
            !storage.exists(&key).await.expect("exists check"),
            "doesn't exist before write"
        );
        storage.write_buffered(&key, &content).await.expect("write");
        prop_assert!(
            storage.exists(&key).await.expect("exists check"),
            "exists after write"
        );
    }

    #[proptest(async = "tokio")]
    async fn read_compressed_basic(#[any] content: Vec<u8>) -> Result<()> {
        let _ = color_eyre::install();

        let (storage, _temp) = Disk::new_temp().await?;

        let key = key_for(&content);
        storage.write_buffered(&key, &content).await?;

        let mut compressed_reader = storage.read_compressed(&key).await?;
        let mut compressed_content = Vec::new();
        compressed_reader
            .read_to_end(&mut compressed_content)
            .await?;

        let decompressed = decompress(&compressed_content);
        pretty_assert_eq!(
            decompressed,
            content,
            "decompressing read_compressed output gives original content"
        );

        Ok(())
    }

    #[proptest(async = "tokio")]
    async fn write_compressed_basic(#[any] content: Vec<u8>) -> Result<()> {
        let _ = color_eyre::install();

        let (storage, _temp) = Disk::new_temp().await?;

        let key = key_for(&content);
        let compressed = compress(&content);
        storage
            .write_compressed(&key, Cursor::new(&compressed))
            .await?;

        pretty_assert_eq!(storage.exists(&key).await?, true);

        let read_content = storage.read_buffered(&key).await?;
        pretty_assert_eq!(
            read_content,
            content,
            "reading via normal read() gives original content"
        );

        Ok(())
    }

    #[proptest(async = "tokio")]
    async fn write_compressed_roundtrip(#[any] content: Vec<u8>) -> Result<()> {
        let _ = color_eyre::install();

        let (storage, _temp) = Disk::new_temp().await?;

        let key = key_for(&content);
        let compressed = compress(&content);
        storage
            .write_compressed(&key, Cursor::new(&compressed))
            .await?;

        let mut compressed_reader = storage.read_compressed(&key).await?;
        let mut read_compressed = Vec::new();
        compressed_reader.read_to_end(&mut read_compressed).await?;

        pretty_assert_eq!(
            decompress(&read_compressed),
            content,
            "read_compressed after write_compressed roundtrips"
        );

        Ok(())
    }

    #[proptest(async = "tokio")]
    async fn write_then_read_compressed(#[any] content: Vec<u8>) -> Result<()> {
        let _ = color_eyre::install();

        let (storage, _temp) = Disk::new_temp().await?;

        let key = key_for(&content);
        storage.write_buffered(&key, &content).await?;

        let mut compressed_reader = storage.read_compressed(&key).await?;
        let mut compressed_content = Vec::new();
        compressed_reader
            .read_to_end(&mut compressed_content)
            .await?;

        let decompressed = decompress(&compressed_content);
        pretty_assert_eq!(
            decompressed,
            content,
            "write() then read_compressed() works"
        );

        Ok(())
    }

    #[proptest(async = "tokio")]
    async fn write_compressed_then_read(#[any] content: Vec<u8>) -> Result<()> {
        let _ = color_eyre::install();

        let (storage, _temp) = Disk::new_temp().await?;

        let key = key_for(&content);
        let compressed = compress(&content);
        storage
            .write_compressed(&key, Cursor::new(&compressed))
            .await?;

        let read_content = storage.read_buffered(&key).await?;
        pretty_assert_eq!(
            read_content,
            content,
            "write_compressed() then read() works"
        );

        Ok(())
    }

    #[proptest(async = "tokio")]
    async fn size_returns_uncompressed_size(#[any] content: Vec<u8>) -> Result<()> {
        let _ = color_eyre::install();

        let (storage, _temp) = Disk::new_temp().await?;

        let key = key_for(&content);
        storage.write_buffered(&key, &content).await?;

        let size = storage.size(&key).await?;
        pretty_assert_eq!(
            size,
            Some(content.len() as u64),
            "size() returns uncompressed size"
        );

        Ok(())
    }

    #[proptest(async = "tokio")]
    async fn size_compressed_returns_compressed_size(#[any] content: Vec<u8>) -> Result<()> {
        let _ = color_eyre::install();

        let (storage, _temp) = Disk::new_temp().await?;

        let key = key_for(&content);
        storage.write_buffered(&key, &content).await?;

        let compressed_size = storage.size_compressed(&key).await?;
        let expected_compressed = compress(&content);

        pretty_assert_eq!(
            compressed_size,
            Some(expected_compressed.len() as u64),
            "size_compressed() returns compressed size"
        );

        Ok(())
    }

    #[proptest(async = "tokio")]
    async fn size_consistency_write_vs_write_compressed(#[any] content: Vec<u8>) -> Result<()> {
        let _ = color_eyre::install();

        let (storage1, _temp1) = Disk::new_temp().await?;
        let (storage2, _temp2) = Disk::new_temp().await?;

        let key = key_for(&content);

        storage1.write_buffered(&key, &content).await?;
        let compressed = compress(&content);
        storage2
            .write_compressed(&key, Cursor::new(&compressed))
            .await?;

        let size1 = storage1.size(&key).await?;
        let size2 = storage2.size(&key).await?;
        pretty_assert_eq!(
            size1,
            size2,
            "write() and write_compressed() produce same uncompressed size"
        );

        let compressed_size1 = storage1.size_compressed(&key).await?;
        let compressed_size2 = storage2.size_compressed(&key).await?;
        pretty_assert_eq!(
            compressed_size1,
            compressed_size2,
            "write() and write_compressed() produce same compressed size"
        );

        Ok(())
    }

    #[proptest(async = "tokio")]
    async fn write_compressed_validates_hash(
        #[any] content: Vec<u8>,
        #[any] wrong_content: Vec<u8>,
    ) {
        prop_assume!(content != wrong_content);

        let (storage, _temp) = Disk::new_temp().await.expect("temp dir");

        let key = key_for(&content);
        let wrong_compressed = compress(&wrong_content);

        let result = storage
            .write_compressed(&key, Cursor::new(&wrong_compressed))
            .await;

        prop_assert!(
            result.is_err(),
            "write_compressed() rejects content with wrong hash"
        );
    }

    #[proptest(async = "tokio")]
    async fn write_compressed_idempotent(#[any] content: Vec<u8>) -> Result<()> {
        let _ = color_eyre::install();

        let (storage, _temp) = Disk::new_temp().await?;

        let key = key_for(&content);
        let compressed = compress(&content);
        storage
            .write_compressed(&key, Cursor::new(&compressed))
            .await?;
        storage
            .write_compressed(&key, Cursor::new(&compressed))
            .await?;

        let read_content = storage.read_buffered(&key).await?;
        pretty_assert_eq!(read_content, content);

        Ok(())
    }

    #[proptest(async = "tokio")]
    async fn write_compressed_concurrent(#[any] content: Vec<u8>) -> Result<()> {
        let _ = color_eyre::install();

        let (storage, _temp) = Disk::new_temp().await?;

        let key = key_for(&content);
        let compressed = compress(&content);
        tokio::try_join!(
            storage.write_compressed(&key, Cursor::new(&compressed)),
            storage.write_compressed(&key, Cursor::new(&compressed))
        )?;

        let read_content = storage.read_buffered(&key).await?;
        pretty_assert_eq!(read_content, content);

        Ok(())
    }

    #[proptest(async = "tokio")]
    async fn size_nonexistent(#[any] content: Vec<u8>) -> Result<()> {
        let _ = color_eyre::install();

        let (storage, _temp) = Disk::new_temp().await?;

        let key = key_for(&content);
        let size = storage.size(&key).await?;
        pretty_assert_eq!(size, None, "size() returns None for nonexistent blob");

        let compressed_size = storage.size_compressed(&key).await?;
        pretty_assert_eq!(
            compressed_size,
            None,
            "size_compressed() returns None for nonexistent blob"
        );

        Ok(())
    }
}
