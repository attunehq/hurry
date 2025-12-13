//! OAuth state and exchange code database operations.

use color_eyre::{Result, eyre::Context};
use time::OffsetDateTime;

use super::Postgres;
use crate::auth::{AccountId, AuthCode};
use crate::crypto::TokenHash;

/// An OAuth state record from the database.
#[derive(Clone, Debug)]
pub struct OAuthState {
    pub id: i64,
    pub state_token: String,
    pub pkce_verifier: String,
    pub redirect_uri: String,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
}

/// Result of redeeming an OAuth exchange code.
#[derive(Clone, Debug)]
pub struct ExchangeCodeRedemption {
    /// The account ID associated with the exchange code.
    pub account_id: AccountId,
    /// Whether this was a new user signup.
    pub new_user: bool,
}

/// Error when redeeming an OAuth exchange code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RedeemExchangeCodeError {
    /// The exchange code was not found.
    NotFound,
    /// The exchange code has expired.
    Expired,
    /// The exchange code was already redeemed.
    AlreadyRedeemed,
}

impl Postgres {
    /// Store OAuth state for the authorization flow.
    #[tracing::instrument(name = "Postgres::store_oauth_state", skip(pkce_verifier))]
    pub async fn store_oauth_state(
        &self,
        state_token: &str,
        pkce_verifier: &str,
        redirect_uri: &str,
        expires_at: OffsetDateTime,
    ) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO oauth_state (state_token, pkce_verifier, redirect_uri, expires_at)
            VALUES ($1, $2, $3, $4)
            "#,
            state_token,
            pkce_verifier,
            redirect_uri,
            expires_at,
        )
        .execute(&self.pool)
        .await
        .context("store oauth state")?;

        Ok(())
    }

    /// Consume OAuth state (fetch and delete atomically).
    ///
    /// Returns `None` if the state doesn't exist or has expired.
    #[tracing::instrument(name = "Postgres::consume_oauth_state")]
    pub async fn consume_oauth_state(&self, state_token: &str) -> Result<Option<OAuthState>> {
        let row = sqlx::query!(
            r#"
            DELETE FROM oauth_state
            WHERE state_token = $1 AND expires_at > NOW()
            RETURNING id, state_token, pkce_verifier, redirect_uri, created_at, expires_at
            "#,
            state_token,
        )
        .fetch_optional(&self.pool)
        .await
        .context("consume oauth state")?;

        Ok(row.map(|r| OAuthState {
            id: r.id,
            state_token: r.state_token,
            pkce_verifier: r.pkce_verifier,
            redirect_uri: r.redirect_uri,
            created_at: r.created_at,
            expires_at: r.expires_at,
        }))
    }

    /// Clean up expired OAuth state records.
    ///
    /// Returns the number of records deleted.
    #[tracing::instrument(name = "Postgres::cleanup_expired_oauth_state")]
    pub async fn cleanup_expired_oauth_state(&self) -> Result<u64> {
        let result = sqlx::query!(
            r#"
            DELETE FROM oauth_state
            WHERE expires_at < NOW()
            "#,
        )
        .execute(&self.pool)
        .await
        .context("cleanup expired oauth state")?;

        Ok(result.rows_affected())
    }

    /// Create an OAuth exchange code.
    ///
    /// Exchange codes are short-lived (60 seconds), single-use tokens that
    /// the dashboard backend exchanges for a session token server-to-server.
    /// Only a SHA-256 hash of the code is stored.
    #[tracing::instrument(name = "Postgres::create_exchange_code")]
    pub async fn create_exchange_code(
        &self,
        account_id: AccountId,
        redirect_uri: &str,
        new_user: bool,
        expires_at: OffsetDateTime,
    ) -> Result<AuthCode> {
        let code = AuthCode::generate();
        let hash = TokenHash::new(code.expose());
        sqlx::query!(
            r#"
            INSERT INTO oauth_exchange_code (code_hash, account_id, redirect_uri, new_user, expires_at)
            VALUES ($1, $2, $3, $4, $5)
            "#,
            hash.as_bytes(),
            account_id.as_i64(),
            redirect_uri,
            new_user,
            expires_at,
        )
        .execute(&self.pool)
        .await
        .context("create exchange code")?;

        Ok(code)
    }

    /// Redeem an OAuth exchange code (atomically validates and marks as
    /// redeemed).
    ///
    /// Returns the account info if successful, or an error describing why
    /// redemption failed.
    #[tracing::instrument(name = "Postgres::redeem_exchange_code", skip(code))]
    pub async fn redeem_exchange_code(
        &self,
        code: &AuthCode,
    ) -> Result<std::result::Result<ExchangeCodeRedemption, RedeemExchangeCodeError>> {
        let hash = TokenHash::new(code.expose());
        let mut tx = self.pool.begin().await?;

        let row = sqlx::query!(
            r#"
            SELECT id, account_id, new_user, expires_at, redeemed_at
            FROM oauth_exchange_code
            WHERE code_hash = $1
            FOR UPDATE
            "#,
            hash.as_bytes(),
        )
        .fetch_optional(tx.as_mut())
        .await
        .context("fetch exchange code")?;

        let Some(row) = row else {
            return Ok(Err(RedeemExchangeCodeError::NotFound));
        };

        if row.redeemed_at.is_some() {
            return Ok(Err(RedeemExchangeCodeError::AlreadyRedeemed));
        }

        let now = OffsetDateTime::now_utc();
        if row.expires_at <= now {
            return Ok(Err(RedeemExchangeCodeError::Expired));
        }

        sqlx::query!(
            r#"
            UPDATE oauth_exchange_code
            SET redeemed_at = NOW()
            WHERE id = $1
            "#,
            row.id,
        )
        .execute(tx.as_mut())
        .await
        .context("mark exchange code as redeemed")?;

        tx.commit().await?;

        Ok(Ok(ExchangeCodeRedemption {
            account_id: AccountId::from_i64(row.account_id),
            new_user: row.new_user,
        }))
    }

    /// Clean up expired exchange codes.
    ///
    /// Returns the number of codes deleted.
    #[tracing::instrument(name = "Postgres::cleanup_expired_exchange_codes")]
    pub async fn cleanup_expired_exchange_codes(&self) -> Result<u64> {
        let result = sqlx::query!(
            r#"
            DELETE FROM oauth_exchange_code
            WHERE expires_at < NOW()
            "#,
        )
        .execute(&self.pool)
        .await
        .context("cleanup expired exchange codes")?;

        Ok(result.rows_affected())
    }
}
