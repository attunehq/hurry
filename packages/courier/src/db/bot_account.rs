//! Bot account database operations.

use color_eyre::{Result, eyre::Context};
use time::OffsetDateTime;

use super::Postgres;
use crate::auth::{AccountId, OrgId, RawToken};
use crate::crypto::TokenHash;

/// A bot account record from the database.
///
/// Bot accounts are organization-scoped accounts without GitHub identity,
/// used for CI systems and automation.
#[derive(Clone, Debug)]
pub struct BotAccount {
    pub id: AccountId,
    pub name: Option<String>,
    pub email: String,
    pub created_at: OffsetDateTime,
}

impl Postgres {
    /// Create a bot account for an organization.
    ///
    /// Bot accounts:
    /// - Have no GitHub identity
    /// - Belong to exactly one organization (as member role by default)
    /// - Use `email` field for the responsible person's contact email
    /// - Get an initial API key created
    ///
    /// This is the only time the token is available in plaintext.
    #[tracing::instrument(name = "Postgres::create_bot_account")]
    pub async fn create_bot_account(
        &self,
        org_id: OrgId,
        name: &str,
        responsible_email: &str,
    ) -> Result<(AccountId, RawToken)> {
        let mut tx = self.pool.begin().await?;

        let row = sqlx::query!(
            r#"
            INSERT INTO account (email, name)
            VALUES ($1, $2)
            RETURNING id
            "#,
            responsible_email,
            name,
        )
        .fetch_one(tx.as_mut())
        .await
        .context("create bot account")?;

        let account_id = AccountId::from_i64(row.id);

        sqlx::query!(
            r#"
            INSERT INTO organization_member (organization_id, account_id, role_id)
            VALUES ($1, $2, (SELECT id FROM organization_role WHERE name = 'member'))
            "#,
            org_id.as_i64(),
            account_id.as_i64(),
        )
        .execute(tx.as_mut())
        .await
        .context("add bot to organization")?;

        let token = RawToken::generate();
        let hash = TokenHash::new(token.expose());
        let key_name = format!("{name} API Key");

        sqlx::query!(
            r#"
            INSERT INTO api_key (account_id, name, hash, organization_id)
            VALUES ($1, $2, $3, $4)
            "#,
            account_id.as_i64(),
            key_name,
            hash.as_bytes(),
            org_id.as_i64(),
        )
        .execute(tx.as_mut())
        .await
        .context("create bot api key")?;

        tx.commit().await?;

        Ok((account_id, token))
    }

    /// List bot accounts for an organization.
    ///
    /// Bot accounts are accounts that:
    /// - Are members of the organization
    /// - Have no GitHub identity linked
    #[tracing::instrument(name = "Postgres::list_bot_accounts")]
    pub async fn list_bot_accounts(&self, org_id: OrgId) -> Result<Vec<BotAccount>> {
        let rows = sqlx::query!(
            r#"
            SELECT a.id, a.name, a.email, a.created_at
            FROM account a
            JOIN organization_member om ON a.id = om.account_id
            WHERE om.organization_id = $1
              AND NOT EXISTS (
                  SELECT 1 FROM github_identity gi WHERE gi.account_id = a.id
              )
            ORDER BY a.created_at DESC
            "#,
            org_id.as_i64(),
        )
        .fetch_all(&self.pool)
        .await
        .context("list bot accounts")?;

        Ok(rows
            .into_iter()
            .map(|r| BotAccount {
                id: AccountId::from_i64(r.id),
                name: r.name,
                email: r.email,
                created_at: r.created_at,
            })
            .collect())
    }
}
