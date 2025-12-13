use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use courier::{
    auth::{RawToken, SessionToken},
    crypto::{generate_invitation_token, generate_oauth_state, generate_pkce},
};
use sha2::{Digest, Sha256};

#[test]
fn api_key_has_correct_length() {
    let key = RawToken::generate();
    assert_eq!(key.expose().len(), 32);
}

#[test]
fn api_key_is_valid_hex() {
    let key = RawToken::generate();
    assert!(hex::decode(key.expose()).is_ok());
}

#[test]
fn session_token_has_correct_length() {
    let token = SessionToken::generate();
    assert_eq!(token.expose().len(), 64);
}

#[test]
fn session_token_is_valid_hex() {
    let token = SessionToken::generate();
    assert!(hex::decode(token.expose()).is_ok());
}

#[test]
fn oauth_state_has_correct_length() {
    let state = generate_oauth_state();
    assert_eq!(state.len(), 32);
}

#[test]
fn oauth_state_is_valid_hex() {
    let state = generate_oauth_state();
    assert!(hex::decode(&state).is_ok());
}

#[test]
fn invitation_token_short_lived_length() {
    let token = generate_invitation_token(false);
    assert_eq!(token.len(), 8);
}

#[test]
fn invitation_token_long_lived_length() {
    let token = generate_invitation_token(true);
    assert_eq!(token.len(), 12);
}

#[test]
fn invitation_token_is_alphanumeric() {
    let token = generate_invitation_token(false);
    assert!(token.chars().all(|c| c.is_ascii_alphanumeric()));

    let token = generate_invitation_token(true);
    assert!(token.chars().all(|c| c.is_ascii_alphanumeric()));
}

#[test]
fn pkce_verifier_has_correct_length() {
    let pkce = generate_pkce();
    // 32 bytes base64url encoded = 43 characters
    assert_eq!(pkce.verifier.len(), 43);
}

#[test]
fn pkce_challenge_has_correct_length() {
    let pkce = generate_pkce();
    // SHA256 hash (32 bytes) base64url encoded = 43 characters
    assert_eq!(pkce.challenge.len(), 43);
}

#[test]
fn pkce_challenge_is_s256_of_verifier() {
    let pkce = generate_pkce();

    // Manually compute the expected challenge
    let mut hasher = Sha256::new();
    hasher.update(pkce.verifier.as_bytes());
    let hash = hasher.finalize();
    let expected_challenge = URL_SAFE_NO_PAD.encode(hash);

    assert_eq!(pkce.challenge, expected_challenge);
}

#[test]
fn tokens_are_unique() {
    // Generate multiple tokens and ensure they're different
    let keys = (0..10).map(|_| RawToken::generate()).collect::<Vec<_>>();
    for i in 0..keys.len() {
        for j in (i + 1)..keys.len() {
            assert_ne!(keys[i].expose(), keys[j].expose());
        }
    }
}
