//! User session database operations.

use color_eyre::{Result, eyre::Context};
use time::OffsetDateTime;

use super::Postgres;
use crate::auth::{AccountId, SessionContext, SessionId, SessionToken};
use crate::crypto::TokenHash;

/// A user session record from the database.
#[derive(Clone, Debug)]
pub struct UserSession {
    pub id: SessionId,
    pub account_id: AccountId,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub last_accessed_at: OffsetDateTime,
}

impl Postgres {
    /// Create a new user session.
    ///
    /// The session token should be generated using
    /// `crypto::generate_session_token()`. The token is hashed before
    /// storage.
    #[tracing::instrument(name = "Postgres::create_session", skip(token))]
    pub async fn create_session(
        &self,
        account_id: AccountId,
        token: &SessionToken,
        expires_at: OffsetDateTime,
    ) -> Result<SessionId> {
        let hash = TokenHash::new(token.expose());
        let row = sqlx::query!(
            r#"
            INSERT INTO user_session (account_id, session_token, expires_at)
            VALUES ($1, $2, $3)
            RETURNING id
            "#,
            account_id.as_i64(),
            hash.as_bytes(),
            expires_at,
        )
        .fetch_one(&self.pool)
        .await
        .context("create session")?;

        Ok(SessionId::from_i64(row.id))
    }

    /// Validate a session token and return the session context.
    ///
    /// Returns `None` if the token is invalid, expired, or the account is
    /// disabled. On successful validation, extends the session expiration
    /// (sliding window) and updates `last_accessed_at`.
    #[tracing::instrument(name = "Postgres::validate_session", skip(token))]
    pub async fn validate_session(&self, token: &SessionToken) -> Result<Option<SessionContext>> {
        let hash = TokenHash::new(token.expose());
        let row = sqlx::query!(
            r#"
            SELECT us.id, us.account_id
            FROM user_session us
            JOIN account a ON us.account_id = a.id
            WHERE us.session_token = $1
              AND us.expires_at > NOW()
              AND a.disabled_at IS NULL
            "#,
            hash.as_bytes(),
        )
        .fetch_optional(&self.pool)
        .await
        .context("validate session")?;

        let Some(row) = row else {
            return Ok(None);
        };

        // Update last_accessed_at and extend expiration (sliding window: 24 hours from
        // now)
        sqlx::query!(
            r#"
            UPDATE user_session
            SET last_accessed_at = NOW(),
                expires_at = NOW() + INTERVAL '24 hours'
            WHERE id = $1
            "#,
            row.id,
        )
        .execute(&self.pool)
        .await
        .context("update session last_accessed_at and expires_at")?;

        Ok(Some(SessionContext {
            account_id: AccountId::from_i64(row.account_id),
            session_token: token.clone(),
        }))
    }

    /// Revoke a specific session.
    #[tracing::instrument(name = "Postgres::revoke_session", skip(token))]
    pub async fn revoke_session(&self, token: &SessionToken) -> Result<bool> {
        let hash = TokenHash::new(token.expose());
        let result = sqlx::query!(
            r#"
            DELETE FROM user_session
            WHERE session_token = $1
            "#,
            hash.as_bytes(),
        )
        .execute(&self.pool)
        .await
        .context("revoke session")?;

        Ok(result.rows_affected() > 0)
    }

    /// Revoke all sessions for an account.
    #[tracing::instrument(name = "Postgres::revoke_all_sessions")]
    pub async fn revoke_all_sessions(&self, account_id: AccountId) -> Result<u64> {
        let result = sqlx::query!(
            r#"
            DELETE FROM user_session
            WHERE account_id = $1
            "#,
            account_id.as_i64(),
        )
        .execute(&self.pool)
        .await
        .context("revoke all sessions")?;

        Ok(result.rows_affected())
    }

    /// Extend a session's expiration time.
    #[tracing::instrument(name = "Postgres::extend_session", skip(token))]
    pub async fn extend_session(
        &self,
        token: &SessionToken,
        new_expires_at: OffsetDateTime,
    ) -> Result<bool> {
        let hash = TokenHash::new(token.expose());
        let result = sqlx::query!(
            r#"
            UPDATE user_session
            SET expires_at = $2, last_accessed_at = NOW()
            WHERE session_token = $1
            "#,
            hash.as_bytes(),
            new_expires_at,
        )
        .execute(&self.pool)
        .await
        .context("extend session")?;

        Ok(result.rows_affected() > 0)
    }

    /// Clean up expired sessions.
    ///
    /// Returns the number of sessions deleted.
    #[tracing::instrument(name = "Postgres::cleanup_expired_sessions")]
    pub async fn cleanup_expired_sessions(&self) -> Result<u64> {
        let result = sqlx::query!(
            r#"
            DELETE FROM user_session
            WHERE expires_at < NOW()
            "#,
        )
        .execute(&self.pool)
        .await
        .context("cleanup expired sessions")?;

        Ok(result.rows_affected())
    }
}
