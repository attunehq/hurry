//! Cryptographic utilities for token hashing and verification.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
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
    /// Parse a token hash from raw bytes.
    pub fn parse(hash: impl Into<Vec<u8>>) -> Self {
        Self(hash.into())
    }

    /// Hash a plaintext token using SHA256.
    pub fn new(token: impl AsRef<[u8]>) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(token.as_ref());
        let hash = hasher.finalize();
        Self(hash.to_vec())
    }

    /// Verify a token against the hash.
    pub fn verify(&self, token: impl AsRef<[u8]>) -> bool {
        Self::new(token) == *self
    }

    /// Get the hash as bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<TokenHash> for TokenHash {
    fn as_ref(&self) -> &TokenHash {
        self
    }
}

/// Generate an OAuth state token.
pub fn generate_oauth_state() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Generate an invitation token.
///
/// The intention is to make the token easy to share but not so easy that they
/// are able to be guessed. The endpoint to accept invitations is rate limited.
///
/// Token length varies based on whether the invitation is long-lived:
/// - Short-lived: 8 characters
/// - Long-lived: 12 characters
pub fn generate_invitation_token(long_lived: bool) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

    let length = if long_lived { 12 } else { 8 };
    let mut rng = rand::thread_rng();

    (0..length)
        .map(|_| {
            let idx = (rng.next_u32() as usize) % ALPHABET.len();
            ALPHABET[idx] as char
        })
        .collect()
}

/// PKCE (Proof Key for Code Exchange) verifier and challenge.
///
/// Used in the OAuth flow to prevent authorization code interception attacks.
#[derive(Clone, Debug)]
pub struct PkceChallenge {
    /// The verifier (stored server-side, used during token exchange).
    pub verifier: String,

    /// The challenge (sent to the authorization server).
    pub challenge: String,
}

/// Generate a PKCE verifier and S256 challenge.
///
/// The verifier is a 43-character base64url-encoded random string (32 bytes).
/// The challenge is the base64url-encoded SHA256 hash of the verifier.
pub fn generate_pkce() -> PkceChallenge {
    let mut verifier_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut verifier_bytes);
    let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);

    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    let challenge = URL_SAFE_NO_PAD.encode(hash);

    PkceChallenge {
        verifier,
        challenge,
    }
}
