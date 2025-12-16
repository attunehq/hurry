//! Account database operations.

use color_eyre::{Result, eyre::Context};
use time::OffsetDateTime;

use super::Postgres;
use crate::auth::{AccountId, OrgId};

/// An account record from the database.
///
/// Note: Organization membership is tracked via the `organization_member`
/// table, not directly on the account. Use `list_organizations_for_account` to
/// get an account's organizations.
#[derive(Clone, Debug)]
pub struct Account {
    pub id: AccountId,
    pub email: String,
    pub name: Option<String>,
    pub disabled_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
}

/// Result of a new user signup via GitHub OAuth.
#[derive(Clone, Debug)]
pub struct SignupResult {
    /// The account ID of the new user.
    pub account_id: AccountId,
    /// The ID of the default organization created for the user.
    pub org_id: OrgId,
}

impl Postgres {
    /// Create a new account with GitHub identity and default organization.
    ///
    /// This is the transactional signup flow for new users via GitHub OAuth.
    /// It atomically:
    /// 1. Creates the account
    /// 2. Links the GitHub identity
    /// 3. Creates a default organization
    /// 4. Adds the user as admin of the organization
    ///
    /// If any step fails, the entire operation is rolled back.
    #[tracing::instrument(name = "Postgres::signup_with_github")]
    pub async fn signup_with_github(
        &self,
        email: &str,
        name: Option<&str>,
        github_user_id: i64,
        github_username: &str,
        org_name: &str,
    ) -> Result<SignupResult> {
        let mut tx = self.pool.begin().await?;

        let account_row = sqlx::query!(
            r#"
            INSERT INTO account (email, name)
            VALUES ($1, $2)
            RETURNING id
            "#,
            email,
            name,
        )
        .fetch_one(tx.as_mut())
        .await
        .context("create account")?;

        let account_id = AccountId::from_i64(account_row.id);

        sqlx::query!(
            r#"
            INSERT INTO github_identity (account_id, github_user_id, github_username)
            VALUES ($1, $2, $3)
            "#,
            account_id.as_i64(),
            github_user_id,
            github_username,
        )
        .execute(tx.as_mut())
        .await
        .context("link github identity")?;

        let org_row = sqlx::query!(
            r#"
            INSERT INTO organization (name)
            VALUES ($1)
            RETURNING id
            "#,
            org_name,
        )
        .fetch_one(tx.as_mut())
        .await
        .context("create organization")?;

        let org_id = OrgId::from_i64(org_row.id);

        sqlx::query!(
            r#"
            INSERT INTO organization_member (organization_id, account_id, role_id)
            VALUES ($1, $2, (SELECT id FROM organization_role WHERE name = 'admin'))
            "#,
            org_id.as_i64(),
            account_id.as_i64(),
        )
        .execute(tx.as_mut())
        .await
        .context("add user as org admin")?;

        tx.commit().await?;

        Ok(SignupResult { account_id, org_id })
    }

    /// Create a new account.
    ///
    /// Note: This only creates the account record. Use
    /// `add_organization_member` to associate the account with an
    /// organization.
    #[tracing::instrument(name = "Postgres::create_account")]
    pub async fn create_account(&self, email: &str, name: Option<&str>) -> Result<AccountId> {
        let row = sqlx::query!(
            r#"
            INSERT INTO account (email, name)
            VALUES ($1, $2)
            RETURNING id
            "#,
            email,
            name,
        )
        .fetch_one(&self.pool)
        .await
        .context("insert account")?;

        Ok(AccountId::from_i64(row.id))
    }

    /// Get an account by ID.
    #[tracing::instrument(name = "Postgres::get_account")]
    pub async fn get_account(&self, account_id: AccountId) -> Result<Option<Account>> {
        let row = sqlx::query!(
            r#"
            SELECT id, email, name, disabled_at, created_at
            FROM account
            WHERE id = $1
            "#,
            account_id.as_i64(),
        )
        .fetch_optional(&self.pool)
        .await
        .context("fetch account")?;

        Ok(row.map(|r| Account {
            id: AccountId::from_i64(r.id),
            email: r.email,
            name: r.name,
            disabled_at: r.disabled_at,
            created_at: r.created_at,
        }))
    }

    /// Get an account by GitHub user ID (via github_identity table).
    #[tracing::instrument(name = "Postgres::get_account_by_github_id")]
    pub async fn get_account_by_github_id(&self, github_user_id: i64) -> Result<Option<Account>> {
        let row = sqlx::query!(
            r#"
            SELECT a.id, a.email, a.name, a.disabled_at, a.created_at
            FROM account a
            JOIN github_identity gi ON a.id = gi.account_id
            WHERE gi.github_user_id = $1
            "#,
            github_user_id,
        )
        .fetch_optional(&self.pool)
        .await
        .context("fetch account by github id")?;

        Ok(row.map(|r| Account {
            id: AccountId::from_i64(r.id),
            email: r.email,
            name: r.name,
            disabled_at: r.disabled_at,
            created_at: r.created_at,
        }))
    }

    /// Update an account's email address.
    #[tracing::instrument(name = "Postgres::update_account_email")]
    pub async fn update_account_email(&self, account_id: AccountId, email: &str) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE account
            SET email = $2
            WHERE id = $1
            "#,
            account_id.as_i64(),
            email,
        )
        .execute(&self.pool)
        .await
        .context("update account email")?;

        Ok(())
    }

    /// Update an account's name.
    #[tracing::instrument(name = "Postgres::update_account_name")]
    pub async fn update_account_name(
        &self,
        account_id: AccountId,
        name: Option<&str>,
    ) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE account
            SET name = $2
            WHERE id = $1
            "#,
            account_id.as_i64(),
            name,
        )
        .execute(&self.pool)
        .await
        .context("update account name")?;

        Ok(())
    }

    /// Disable an account, preventing all API access.
    #[tracing::instrument(name = "Postgres::disable_account")]
    pub async fn disable_account(&self, account_id: AccountId) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE account
            SET disabled_at = NOW()
            WHERE id = $1
            "#,
            account_id.as_i64(),
        )
        .execute(&self.pool)
        .await
        .context("disable account")?;

        Ok(())
    }

    /// Re-enable a previously disabled account.
    #[tracing::instrument(name = "Postgres::enable_account")]
    pub async fn enable_account(&self, account_id: AccountId) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE account
            SET disabled_at = NULL
            WHERE id = $1
            "#,
            account_id.as_i64(),
        )
        .execute(&self.pool)
        .await
        .context("enable account")?;

        Ok(())
    }
}
