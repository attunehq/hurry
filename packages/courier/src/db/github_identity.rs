//! GitHub identity database operations.

use color_eyre::{Result, eyre::Context};
use time::OffsetDateTime;

use super::Postgres;
use crate::auth::AccountId;

/// A GitHub identity record from the database.
#[derive(Clone, Debug)]
pub struct GitHubIdentity {
    pub id: i64,
    pub account_id: AccountId,
    pub github_user_id: i64,
    pub github_username: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

impl Postgres {
    /// Link a GitHub identity to an account.
    #[tracing::instrument(name = "Postgres::link_github_identity")]
    pub async fn link_github_identity(
        &self,
        account_id: AccountId,
        github_user_id: i64,
        github_username: &str,
    ) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO github_identity (account_id, github_user_id, github_username)
            VALUES ($1, $2, $3)
            "#,
            account_id.as_i64(),
            github_user_id,
            github_username,
        )
        .execute(&self.pool)
        .await
        .context("link github identity")?;

        Ok(())
    }

    /// Get the GitHub identity for an account.
    #[tracing::instrument(name = "Postgres::get_github_identity")]
    pub async fn get_github_identity(
        &self,
        account_id: AccountId,
    ) -> Result<Option<GitHubIdentity>> {
        let row = sqlx::query!(
            r#"
            SELECT id, account_id, github_user_id, github_username, created_at, updated_at
            FROM github_identity
            WHERE account_id = $1
            "#,
            account_id.as_i64(),
        )
        .fetch_optional(&self.pool)
        .await
        .context("fetch github identity")?;

        Ok(row.map(|r| GitHubIdentity {
            id: r.id,
            account_id: AccountId::from_i64(r.account_id),
            github_user_id: r.github_user_id,
            github_username: r.github_username,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }))
    }

    /// Update the GitHub username for an identity.
    #[tracing::instrument(name = "Postgres::update_github_username")]
    pub async fn update_github_username(
        &self,
        account_id: AccountId,
        github_username: &str,
    ) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE github_identity
            SET github_username = $2, updated_at = NOW()
            WHERE account_id = $1
            "#,
            account_id.as_i64(),
            github_username,
        )
        .execute(&self.pool)
        .await
        .context("update github username")?;

        Ok(())
    }
}
