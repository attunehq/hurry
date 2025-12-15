//! Organization member database operations.

use color_eyre::{
    Result,
    eyre::{Context, eyre},
};
use time::OffsetDateTime;

use super::Postgres;
use crate::auth::{AccountId, OrgId, OrgRole};

/// An organization member record from the database.
#[derive(Clone, Debug)]
pub struct OrganizationMember {
    pub account_id: AccountId,
    pub email: String,
    pub name: Option<String>,
    pub role: OrgRole,
    pub created_at: OffsetDateTime,
    pub has_github_identity: bool,
}

impl Postgres {
    /// Add a member to an organization.
    #[tracing::instrument(name = "Postgres::add_organization_member")]
    pub async fn add_organization_member(
        &self,
        org_id: OrgId,
        account_id: AccountId,
        role: OrgRole,
    ) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO organization_member (organization_id, account_id, role_id)
            VALUES ($1, $2, (SELECT id FROM organization_role WHERE name = $3))
            "#,
            org_id.as_i64(),
            account_id.as_i64(),
            role.as_db_name(),
        )
        .execute(&self.pool)
        .await
        .context("add organization member")?;

        Ok(())
    }

    /// Remove a member from an organization.
    #[tracing::instrument(name = "Postgres::remove_organization_member")]
    pub async fn remove_organization_member(
        &self,
        org_id: OrgId,
        account_id: AccountId,
    ) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            DELETE FROM organization_member
            WHERE organization_id = $1 AND account_id = $2
            "#,
            org_id.as_i64(),
            account_id.as_i64(),
        )
        .execute(&self.pool)
        .await
        .context("remove organization member")?;

        Ok(result.rows_affected() > 0)
    }

    /// Update a member's role in an organization.
    #[tracing::instrument(name = "Postgres::update_member_role")]
    pub async fn update_member_role(
        &self,
        org_id: OrgId,
        account_id: AccountId,
        role: OrgRole,
    ) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            UPDATE organization_member
            SET role_id = (SELECT id FROM organization_role WHERE name = $3)
            WHERE organization_id = $1 AND account_id = $2
            "#,
            org_id.as_i64(),
            account_id.as_i64(),
            role.as_db_name(),
        )
        .execute(&self.pool)
        .await
        .context("update member role")?;

        Ok(result.rows_affected() > 0)
    }

    /// Get a member's role in an organization.
    #[tracing::instrument(name = "Postgres::get_member_role")]
    pub async fn get_member_role(
        &self,
        org_id: OrgId,
        account_id: AccountId,
    ) -> Result<Option<OrgRole>> {
        let row = sqlx::query!(
            r#"
            SELECT r.name as role_name
            FROM organization_member om
            JOIN organization_role r ON om.role_id = r.id
            WHERE om.organization_id = $1 AND om.account_id = $2
            "#,
            org_id.as_i64(),
            account_id.as_i64(),
        )
        .fetch_optional(&self.pool)
        .await
        .context("get member role")?;

        match row {
            Some(r) => {
                let role = OrgRole::from_db_name(&r.role_name)
                    .ok_or_else(|| eyre!("unknown role: {}", r.role_name))?;
                Ok(Some(role))
            }
            None => Ok(None),
        }
    }

    /// List all members of an organization.
    #[tracing::instrument(name = "Postgres::list_organization_members")]
    pub async fn list_organization_members(
        &self,
        org_id: OrgId,
    ) -> Result<Vec<OrganizationMember>> {
        let rows = sqlx::query!(
            r#"
            SELECT
                a.id as account_id,
                a.email,
                a.name,
                r.name as role_name,
                om.created_at,
                gi.id IS NOT NULL as "has_github_identity!"
            FROM organization_member om
            JOIN account a ON om.account_id = a.id
            JOIN organization_role r ON om.role_id = r.id
            LEFT JOIN github_identity gi ON gi.account_id = a.id
            WHERE om.organization_id = $1
            ORDER BY a.email
            "#,
            org_id.as_i64(),
        )
        .fetch_all(&self.pool)
        .await
        .context("list organization members")?;

        rows.into_iter()
            .map(|r| {
                let role = OrgRole::from_db_name(&r.role_name)
                    .ok_or_else(|| eyre!("unknown role: {}", r.role_name))?;
                Ok(OrganizationMember {
                    account_id: AccountId::from_i64(r.account_id),
                    email: r.email,
                    name: r.name,
                    role,
                    created_at: r.created_at,
                    has_github_identity: r.has_github_identity,
                })
            })
            .collect()
    }

    /// Check if an account is the last human admin of an organization.
    ///
    /// Bot accounts (those without a GitHub identity) are excluded from this
    /// check, so a human can leave even if bot admins remain.
    #[tracing::instrument(name = "Postgres::is_last_admin")]
    pub async fn is_last_admin(&self, org_id: OrgId, account_id: AccountId) -> Result<bool> {
        // Check if the account is a human admin (has GitHub identity and is admin)
        let row = sqlx::query!(
            r#"
            SELECT EXISTS(
                SELECT 1
                FROM organization_member om
                JOIN organization_role r ON om.role_id = r.id
                JOIN github_identity gi ON gi.account_id = om.account_id
                WHERE om.organization_id = $1
                  AND om.account_id = $2
                  AND r.name = 'admin'
            ) as "is_human_admin!"
            "#,
            org_id.as_i64(),
            account_id.as_i64(),
        )
        .fetch_one(&self.pool)
        .await
        .context("check if human admin")?;

        if !row.is_human_admin {
            return Ok(false);
        }

        // Count total human admins
        let row = sqlx::query!(
            r#"
            SELECT COUNT(*) as count
            FROM organization_member om
            JOIN organization_role r ON om.role_id = r.id
            JOIN github_identity gi ON gi.account_id = om.account_id
            WHERE om.organization_id = $1 AND r.name = 'admin'
            "#,
            org_id.as_i64(),
        )
        .fetch_one(&self.pool)
        .await
        .context("count human admins")?;

        let human_admin_count = row.count.unwrap_or(0);
        Ok(human_admin_count == 1)
    }
}
