//! API key database operations.

use color_eyre::{
    Result,
    eyre::{Context, bail},
};
use rand::RngCore;
use time::OffsetDateTime;

use super::Postgres;
use crate::auth::{AccountId, ApiKeyId, AuthenticatedToken, OrgId, RawToken};
use crate::crypto::TokenHash;

/// An API key record from the database.
#[derive(Clone, Debug)]
pub struct ApiKey {
    pub id: ApiKeyId,
    pub account_id: AccountId,
    pub organization_id: OrgId,
    pub name: String,
    pub created_at: OffsetDateTime,
    pub accessed_at: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
}

/// An API key with account email (for org listing).
#[derive(Debug)]
pub struct OrgApiKey {
    pub id: ApiKeyId,
    pub account_id: AccountId,
    pub name: String,
    pub account_email: String,
    pub created_at: OffsetDateTime,
    pub accessed_at: OffsetDateTime,
    pub has_github_identity: bool,
}

impl Postgres {
    /// Lookup account and org for a raw token by direct hash comparison.
    #[tracing::instrument(name = "Postgres::token_lookup", skip(token))]
    async fn token_lookup(
        &self,
        token: impl AsRef<RawToken>,
    ) -> Result<Option<(AccountId, OrgId)>> {
        let hash = TokenHash::new(token.as_ref().expose());
        let row = sqlx::query!(
            r#"
            SELECT
                api_key.account_id,
                api_key.organization_id
            FROM api_key
            WHERE api_key.hash = $1 AND api_key.revoked_at IS NULL
            "#,
            hash.as_bytes(),
        )
        .fetch_optional(&self.pool)
        .await
        .context("query for token")?;

        Ok(row.map(|r| {
            (
                AccountId::from_i64(r.account_id),
                OrgId::from_i64(r.organization_id),
            )
        }))
    }

    /// Validate a raw token against the database.
    ///
    /// Returns `Some(AuthenticatedToken)` if the token is valid and not
    /// revoked, otherwise returns `None`. Errors are only returned for
    /// database failures.
    #[tracing::instrument(name = "Postgres::validate", skip(token))]
    pub async fn validate(&self, token: impl Into<RawToken>) -> Result<Option<AuthenticatedToken>> {
        let token = token.into();
        Ok(self
            .token_lookup(&token)
            .await?
            .map(|(account_id, org_id)| AuthenticatedToken {
                account_id,
                org_id,
                plaintext: token,
            }))
    }

    /// Generate a new token for the account in the database.
    /// Currently only used in tests. If used elsewhere, feel free to make this
    /// generally available.
    #[allow(dead_code)]
    #[tracing::instrument(name = "Postgres::create_token")]
    pub async fn create_token(
        &self,
        account: AccountId,
        org_id: OrgId,
        name: &str,
    ) -> Result<RawToken> {
        let plaintext = {
            let mut plaintext = [0u8; 16];
            rand::thread_rng()
                .try_fill_bytes(&mut plaintext)
                .context("generate plaintext key")?;
            hex::encode(plaintext)
        };

        let token = TokenHash::new(&plaintext);
        sqlx::query!(
            r#"
            INSERT INTO api_key (account_id, name, hash, organization_id)
            VALUES ($1, $2, $3, $4)
            "#,
            account.as_i64(),
            name,
            token.as_bytes(),
            org_id.as_i64(),
        )
        .execute(&self.pool)
        .await
        .context("insert token")?;

        Ok(RawToken::new(plaintext))
    }

    /// Revoke the specified token.
    /// Currently only used in tests. If used elsewhere, feel free to make this
    /// generally available.
    #[allow(dead_code)]
    #[tracing::instrument(name = "Postgres::revoke_token", skip(token))]
    pub async fn revoke_token(&self, token: impl AsRef<RawToken>) -> Result<()> {
        let hash = TokenHash::new(token.as_ref().expose());

        let results = sqlx::query!(
            r#"
            UPDATE api_key
            SET revoked_at = now()
            WHERE hash = $1
            "#,
            hash.as_bytes(),
        )
        .execute(&self.pool)
        .await
        .context("revoke token")?;

        if results.rows_affected() == 0 {
            bail!("no such token to revoke in the database");
        }

        Ok(())
    }

    /// Create a new API key scoped to an organization.
    ///
    /// This is the only time the token is available in plaintext.
    #[tracing::instrument(name = "Postgres::create_api_key")]
    pub async fn create_api_key(
        &self,
        account_id: AccountId,
        name: &str,
        organization_id: OrgId,
    ) -> Result<(ApiKeyId, RawToken)> {
        let token = RawToken::generate();
        let hash = TokenHash::new(token.expose());

        let row = sqlx::query!(
            r#"
            INSERT INTO api_key (account_id, name, hash, organization_id)
            VALUES ($1, $2, $3, $4)
            RETURNING id
            "#,
            account_id.as_i64(),
            name,
            hash.as_bytes(),
            organization_id.as_i64(),
        )
        .fetch_one(&self.pool)
        .await
        .context("create api key")?;

        Ok((ApiKeyId::from_i64(row.id), token))
    }

    /// List API keys for an account in a specific org.
    #[tracing::instrument(name = "Postgres::list_org_api_keys")]
    pub async fn list_org_api_keys(
        &self,
        account_id: AccountId,
        org_id: OrgId,
    ) -> Result<Vec<ApiKey>> {
        let rows = sqlx::query!(
            r#"
            SELECT id, account_id, organization_id, name, created_at, accessed_at, revoked_at
            FROM api_key
            WHERE account_id = $1 AND organization_id = $2 AND revoked_at IS NULL
            ORDER BY created_at DESC
            "#,
            account_id.as_i64(),
            org_id.as_i64(),
        )
        .fetch_all(&self.pool)
        .await
        .context("list org api keys")?;

        Ok(rows
            .into_iter()
            .map(|r| ApiKey {
                id: ApiKeyId::from_i64(r.id),
                account_id: AccountId::from_i64(r.account_id),
                organization_id: OrgId::from_i64(r.organization_id),
                name: r.name,
                created_at: r.created_at,
                accessed_at: r.accessed_at,
                revoked_at: r.revoked_at,
            })
            .collect())
    }

    /// Revoke an API key by ID.
    #[tracing::instrument(name = "Postgres::revoke_api_key")]
    pub async fn revoke_api_key(&self, key_id: ApiKeyId) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            UPDATE api_key
            SET revoked_at = NOW()
            WHERE id = $1 AND revoked_at IS NULL
            "#,
            key_id.as_i64(),
        )
        .execute(&self.pool)
        .await
        .context("revoke api key")?;

        Ok(result.rows_affected() > 0)
    }

    /// Get an API key by ID.
    #[tracing::instrument(name = "Postgres::get_api_key")]
    pub async fn get_api_key(&self, key_id: ApiKeyId) -> Result<Option<ApiKey>> {
        let row = sqlx::query!(
            r#"
            SELECT id, account_id, organization_id, name, created_at, accessed_at, revoked_at
            FROM api_key
            WHERE id = $1
            "#,
            key_id.as_i64(),
        )
        .fetch_optional(&self.pool)
        .await
        .context("get api key")?;

        Ok(row.map(|r| ApiKey {
            id: ApiKeyId::from_i64(r.id),
            account_id: AccountId::from_i64(r.account_id),
            organization_id: OrgId::from_i64(r.organization_id),
            name: r.name,
            created_at: r.created_at,
            accessed_at: r.accessed_at,
            revoked_at: r.revoked_at,
        }))
    }

    /// List all API keys for an organization.
    ///
    /// Includes account email for display purposes.
    #[tracing::instrument(name = "Postgres::list_all_org_api_keys")]
    pub async fn list_all_org_api_keys(&self, org_id: OrgId) -> Result<Vec<OrgApiKey>> {
        let rows = sqlx::query!(
            r#"
            SELECT
                api_key.id,
                api_key.account_id,
                api_key.name,
                api_key.created_at,
                api_key.accessed_at,
                account.email as account_email,
                gi.id IS NOT NULL as "has_github_identity!"
            FROM api_key
            JOIN account ON api_key.account_id = account.id
            LEFT JOIN github_identity gi ON gi.account_id = account.id
            WHERE api_key.organization_id = $1 AND api_key.revoked_at IS NULL
            ORDER BY api_key.created_at DESC
            "#,
            org_id.as_i64(),
        )
        .fetch_all(&self.pool)
        .await
        .context("list all org api keys")?;

        Ok(rows
            .into_iter()
            .map(|r| OrgApiKey {
                id: ApiKeyId::from_i64(r.id),
                account_id: AccountId::from_i64(r.account_id),
                name: r.name,
                account_email: r.account_email,
                created_at: r.created_at,
                accessed_at: r.accessed_at,
                has_github_identity: r.has_github_identity,
            })
            .collect())
    }
}
