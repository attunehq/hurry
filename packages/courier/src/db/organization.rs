//! Organization database operations.

use color_eyre::{
    Result,
    eyre::{Context, eyre},
};
use time::OffsetDateTime;

use super::Postgres;
use crate::auth::{AccountId, OrgId, OrgRole};

/// An organization record from the database.
#[derive(Clone, Debug)]
pub struct Organization {
    pub id: OrgId,
    pub name: String,
    pub created_at: OffsetDateTime,
}

/// An organization with the user's role in it.
#[derive(Clone, Debug)]
pub struct OrganizationWithRole {
    pub organization: Organization,
    pub role: OrgRole,
}

impl Postgres {
    /// Create a new organization with the creator as admin.
    ///
    /// This atomically creates the organization and adds the creator as an
    /// admin member. If either step fails, the entire operation is rolled back.
    ///
    /// This is the preferred method for creating organizations in production
    /// code, as it ensures the organization always has at least one admin.
    #[tracing::instrument(name = "Postgres::create_organization_with_admin")]
    pub async fn create_organization_with_admin(
        &self,
        name: &str,
        creator_account_id: AccountId,
    ) -> Result<OrgId> {
        let mut tx = self.pool.begin().await?;

        let row = sqlx::query!(
            r#"
            INSERT INTO organization (name)
            VALUES ($1)
            RETURNING id
            "#,
            name,
        )
        .fetch_one(tx.as_mut())
        .await
        .context("create organization")?;

        let org_id = OrgId::from_i64(row.id);

        sqlx::query!(
            r#"
            INSERT INTO organization_member (organization_id, account_id, role_id)
            VALUES ($1, $2, (SELECT id FROM organization_role WHERE name = 'admin'))
            "#,
            org_id.as_i64(),
            creator_account_id.as_i64(),
        )
        .execute(tx.as_mut())
        .await
        .context("add creator as org admin")?;

        tx.commit().await?;

        Ok(org_id)
    }

    /// Create a new organization without any members.
    ///
    /// **Warning**: This creates an organization with no admin. In production,
    /// prefer `create_organization_with_admin` which atomically adds the
    /// creator as an admin.
    ///
    /// This method is primarily intended for testing.
    #[tracing::instrument(name = "Postgres::create_organization")]
    pub async fn create_organization(&self, name: &str) -> Result<OrgId> {
        let row = sqlx::query!(
            r#"
            INSERT INTO organization (name)
            VALUES ($1)
            RETURNING id
            "#,
            name,
        )
        .fetch_one(&self.pool)
        .await
        .context("create organization")?;

        Ok(OrgId::from_i64(row.id))
    }

    /// Get an organization by ID.
    #[tracing::instrument(name = "Postgres::get_organization")]
    pub async fn get_organization(&self, org_id: OrgId) -> Result<Option<Organization>> {
        let row = sqlx::query!(
            r#"
            SELECT id, name, created_at
            FROM organization
            WHERE id = $1
            "#,
            org_id.as_i64(),
        )
        .fetch_optional(&self.pool)
        .await
        .context("fetch organization")?;

        Ok(row.map(|r| Organization {
            id: OrgId::from_i64(r.id),
            name: r.name,
            created_at: r.created_at,
        }))
    }

    /// Rename an organization.
    ///
    /// Returns `true` if the organization was updated, `false` if not found.
    #[tracing::instrument(name = "Postgres::rename_organization")]
    pub async fn rename_organization(&self, org_id: OrgId, name: &str) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            UPDATE organization
            SET name = $2
            WHERE id = $1
            "#,
            org_id.as_i64(),
            name,
        )
        .execute(&self.pool)
        .await
        .context("rename organization")?;

        Ok(result.rows_affected() > 0)
    }

    /// List all organizations an account is a member of.
    #[tracing::instrument(name = "Postgres::list_organizations_for_account")]
    pub async fn list_organizations_for_account(
        &self,
        account_id: AccountId,
    ) -> Result<Vec<OrganizationWithRole>> {
        let rows = sqlx::query!(
            r#"
            SELECT o.id, o.name, o.created_at, r.name as role_name
            FROM organization o
            JOIN organization_member om ON o.id = om.organization_id
            JOIN organization_role r ON om.role_id = r.id
            WHERE om.account_id = $1
            ORDER BY o.name
            "#,
            account_id.as_i64(),
        )
        .fetch_all(&self.pool)
        .await
        .context("list organizations for account")?;

        rows.into_iter()
            .map(|r| {
                let role = OrgRole::from_db_name(&r.role_name)
                    .ok_or_else(|| eyre!("unknown role: {}", r.role_name))?;
                Ok(OrganizationWithRole {
                    organization: Organization {
                        id: OrgId::from_i64(r.id),
                        name: r.name,
                        created_at: r.created_at,
                    },
                    role,
                })
            })
            .collect()
    }
}
