//! Courier API client types and HTTP client.

use std::{fmt, str::FromStr};

use color_eyre::eyre::bail;
use serde::{Deserialize, Serialize};

pub mod v1;

/// An authentication token for Courier API access.
///
/// This type wraps a token string and ensures it is never accidentally leaked in logs
/// or debug output. To access the actual token value, use the `expose()` method.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Token(String);

impl Token {
    /// Expose the raw token value.
    ///
    /// This method must be called explicitly to access the token string,
    /// preventing accidental exposure in logs or debug output.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

impl FromStr for Token {
    type Err = color_eyre::Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            bail!("token cannot be empty");
        }
        Ok(Self(s.to_string()))
    }
}

impl<S: Into<String>> From<S> for Token {
    fn from(s: S) -> Self {
        Self(s.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_redaction() {
        let token = Token::from("super-secret-token-12345");

        // Verify redaction in debug and display
        assert_eq!(format!("{:?}", token), "[redacted]");
        assert_eq!(format!("{}", token), "[redacted]");

        // Verify expose() returns the actual value
        assert_eq!(token.expose(), "super-secret-token-12345");
    }

    #[test]
    fn test_token_from_str() {
        let token: Token = "test-token".parse().unwrap();
        assert_eq!(token.expose(), "test-token");

        // Empty string should fail
        assert!("".parse::<Token>().is_err());
    }
}
