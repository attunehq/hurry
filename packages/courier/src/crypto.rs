//! Cryptographic utilities for token hashing and verification.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::auth::{AuthCode, RawToken, SessionToken};

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

/// Generate a new API key token with 128 bits of entropy.
///
/// Returns a 32-character hex string (16 random bytes, hex-encoded).
/// This matches the existing Courier API key format.
pub fn generate_api_key() -> RawToken {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    RawToken::new(hex::encode(bytes))
}

/// Generate a new session token with 256 bits of entropy.
///
/// Returns a 64-character hex string (32 random bytes, hex-encoded).
/// Session tokens have higher entropy than API keys for additional security.
pub fn generate_session_token() -> SessionToken {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    SessionToken::new(hex::encode(bytes))
}

/// Generate an OAuth state token with 128 bits of entropy.
///
/// Returns a 32-character hex string. Used to prevent CSRF attacks
/// during the OAuth flow.
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

/// Generate an OAuth exchange code with 192 bits of entropy.
///
/// Returns a base64url-encoded string (24 random bytes, no padding).
/// Exchange codes are short-lived (60 seconds) and single-use.
///
/// This is used in the two-step OAuth flow: the callback returns an auth_code
/// in the URL, which the dashboard backend exchanges server-to-server for a
/// session token.
pub fn generate_auth_code() -> AuthCode {
    let mut bytes = [0u8; 24]; // 192 bits
    rand::thread_rng().fill_bytes(&mut bytes);
    AuthCode::new(URL_SAFE_NO_PAD.encode(bytes))
}
