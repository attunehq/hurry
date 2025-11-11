//! Cryptographic utilities for token hashing and verification.

use color_eyre::Result;
use sha2::{Digest, Sha256};

/// A hashed API token.
///
/// Hashed tokens use SHA2 (SHA256) algorithm: when you call `new`, the
/// plaintext token is hashed to produce a deterministic hex string.
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
pub struct TokenHash(String);

impl TokenHash {
    /// Currently only used in tests. If used elsewhere, feel free to make this
    /// generally available.
    #[cfg(test)]
    pub fn parse(hash: impl AsRef<str>) -> Result<Self> {
        Ok(Self(String::from(hash.as_ref())))
    }

    /// Create a new instance from the given plaintext token.
    pub fn new(token: impl AsRef<str>) -> Result<Self> {
        let mut hasher = Sha256::new();
        hasher.update(token.as_ref().as_bytes());
        let hash = hasher.finalize();
        Ok(Self(format!("{:x}", hash)))
    }

    /// Currently only used in tests. If used elsewhere, feel free to make this
    /// generally available.
    #[cfg(test)]
    pub fn verify(&self, token: impl AsRef<str>) -> bool {
        match Self::new(token) {
            Ok(candidate) => candidate == *self,
            Err(_) => false,
        }
    }

    /// Get the hash as a string for storage or transmission.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Currently only used in tests. If used elsewhere, feel free to make this
    /// generally available.
    #[cfg(test)]
    pub fn to_string(&self) -> String {
        self.0.clone()
    }
}

impl AsRef<TokenHash> for TokenHash {
    fn as_ref(&self) -> &TokenHash {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify() {
        let plain = "test-token-12345";
        let token = TokenHash::new(plain).expect("hash token");

        assert!(token.verify(plain), "valid token verifies");
        assert!(!token.verify("abcd"), "invalid token fails");
    }

    #[test]
    fn deterministic_hash() {
        let plain = "test-token-12345";

        let token1 = TokenHash::new(plain).expect("hash token");
        let token2 = TokenHash::new(plain).expect("hash token");

        assert_eq!(token1, token2, "same plaintext produces same SHA256 hash");
    }

    #[test]
    fn roundtrip() {
        let plain = "test-token-12345";
        let token = TokenHash::new(plain).expect("hash token");

        // Simulate database roundtrip
        let encoded = token.to_string();
        let parsed = TokenHash::parse(&encoded).expect("parse encoded token");

        assert_eq!(token, parsed, "decoded token should match original");
        assert!(token.verify(plain), "original token should validate");
        assert!(parsed.verify(plain), "decoded token should validate");
    }
}
