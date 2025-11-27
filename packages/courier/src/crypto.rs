//! Cryptographic utilities for token hashing and verification.

use sha2::{Digest, Sha256};

/// A hashed API token.
///
/// Hashed tokens use SHA2 (SHA256) algorithm: when you call `new`, the
/// plaintext token is hashed to produce a deterministic binary hash.
/// Verification compares the hash of the provided plaintext token against the
/// stored hash.
///
/// Note: it's not a _security issue_ to leak this value, but they're not really
/// _intended to be sent to clients_. Instead, the goal is to have clients send
/// the plaintext forms and then we fetch these types from the database to
/// validate the plaintext form of the token. For this reason, this type does
/// not implement `Serialize` or `Deserialize`- if you want to add them, take a
/// moment to think about why that is, because you probably aren't doing the
/// right thing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenHash(Vec<u8>);

impl TokenHash {
    /// Currently only used in tests. If used elsewhere, feel free to make this
    /// generally available.
    #[allow(dead_code)]
    pub fn parse(hash: impl Into<Vec<u8>>) -> Self {
        Self(hash.into())
    }

    /// Create a new instance from the given plaintext token.
    pub fn new(token: impl AsRef<[u8]>) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(token.as_ref());
        let hash = hasher.finalize();
        Self(hash.to_vec())
    }

    /// Currently only used in tests. If used elsewhere, feel free to make this
    /// generally available.
    #[allow(dead_code)]
    pub fn verify(&self, token: impl AsRef<[u8]>) -> bool {
        Self::new(token) == *self
    }

    /// Get the hash as bytes for storage or transmission.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<TokenHash> for TokenHash {
    fn as_ref(&self) -> &TokenHash {
        self
    }
}
