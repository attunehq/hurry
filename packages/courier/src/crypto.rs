//! Cryptographic utilities for token hashing and verification.

use argon2::{
    Argon2, PasswordHasher, PasswordVerifier,
    password_hash::{PasswordHashString, SaltString, rand_core::OsRng},
};
use color_eyre::{Result, eyre::eyre};

/// A hashed API token.
///
/// Hashed tokens use the Argon2 algorithm:
/// - When you call `new`, a salt is generated and used for the token.
/// - Serialization methods like `to_str` encode the algorithm and salt.
/// - When you call `parse`, the algorithm and salt are parsed out.
///
/// Note: it's not a _security issue_ to leak this value, but they're not really
/// _intended to be sent to clients_. Instead, the goal is to have clients send
/// the plaintext forms and then we fetch these types from the database to
/// validate the plaintext form of the token. For this reason, this type does
/// not implement `Serialize` or `Deserialize`- if you want to add them, take a
/// moment to think about why that is, because you probably aren't doing the
/// right thing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenHash(PasswordHashString);

impl TokenHash {
    /// Parse the instance from how it is stored.
    pub fn parse(phc: impl AsRef<str>) -> Result<Self> {
        PasswordHashString::new(phc.as_ref())
            .map(Self)
            .map_err(|err| eyre!("parse as argon2 phc: {err:?}"))
    }

    /// Create a new instance from the given plaintext token.
    pub fn new(token: impl AsRef<str>) -> Result<Self> {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(token.as_ref().as_bytes(), &salt)
            .map(PasswordHashString::from)
            .map(Self)
            .map_err(|err| eyre!("create argon2 phc: {err:?}"))
    }

    /// Verify a plaintext token (e.g. provided by a user) against this hash.
    pub fn verify(&self, token: impl AsRef<str>) -> bool {
        Argon2::default()
            .verify_password(token.as_ref().as_bytes(), &self.0.password_hash())
            .is_ok()
    }

    /// Get the hash as a string for storage or transmission.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Create an owned string representing the token.
    pub fn to_string(&self) -> String {
        self.0.to_string()
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
    fn different_salts() {
        let plain = "test-token-12345";

        let token1 = TokenHash::new(plain).expect("hash token");
        let token2 = TokenHash::new(plain).expect("hash token");

        assert!(token1.verify(plain), "token1 validates");
        assert!(token2.verify(plain), "token2 validates");
        assert_ne!(
            token1, token2,
            "two different salts create two different tokens"
        );
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
