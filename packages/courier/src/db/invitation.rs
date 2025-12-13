//! Invitation database operations.

use color_eyre::{
    Result,
    eyre::{Context, eyre},
};
use time::OffsetDateTime;

use super::Postgres;
use crate::auth::{AccountId, InvitationId, OrgId, OrgRole};

/// An invitation record from the database.
#[derive(Clone, Debug)]
pub struct Invitation {
    pub id: InvitationId,
    pub organization_id: OrgId,
    pub role: OrgRole,
    pub created_by: AccountId,
    pub created_at: OffsetDateTime,
    /// Expiration timestamp. None means the invitation never expires.
    pub expires_at: Option<OffsetDateTime>,
    pub max_uses: Option<i32>,
    pub use_count: i32,
    pub revoked_at: Option<OffsetDateTime>,
}

/// Public invitation info (for preview without authentication).
#[derive(Clone, Debug)]
pub struct InvitationPreview {
    pub organization_name: String,
    pub role: OrgRole,
    /// Expiration timestamp. None means the invitation never expires.
    pub expires_at: Option<OffsetDateTime>,
    pub valid: bool,
}

/// Result of accepting an invitation.
#[derive(Clone, Debug)]
pub enum AcceptInvitationResult {
    /// Successfully joined the organization.
    Success {
        organization_id: OrgId,
        organization_name: String,
        role: OrgRole,
    },
    /// Invitation not found.
    NotFound,
    /// Invitation has been revoked.
    Revoked,
    /// Invitation has expired.
    Expired,
    /// Invitation has reached its maximum uses.
    MaxUsesReached,
    /// Account is already a member of the organization.
    AlreadyMember,
}

impl Postgres {
    /// Create a new invitation.
    ///
    /// The token should be generated using
    /// `crypto::generate_invitation_token()`.
    ///
    /// If `expires_at` is None, the invitation never expires.
    #[tracing::instrument(name = "Postgres::create_invitation", skip(token))]
    pub async fn create_invitation(
        &self,
        org_id: OrgId,
        token: &str,
        role: OrgRole,
        created_by: AccountId,
        expires_at: Option<OffsetDateTime>,
        max_uses: Option<i32>,
    ) -> Result<InvitationId> {
        let row = sqlx::query!(
            r#"
            INSERT INTO organization_invitation
                (organization_id, token, role_id, created_by, expires_at, max_uses)
            VALUES
                ($1, $2, (SELECT id FROM organization_role WHERE name = $3), $4, $5, $6)
            RETURNING id
            "#,
            org_id.as_i64(),
            token,
            role.as_db_name(),
            created_by.as_i64(),
            expires_at,
            max_uses,
        )
        .fetch_one(&self.pool)
        .await
        .context("create invitation")?;

        Ok(InvitationId::from_i64(row.id))
    }

    /// Get an invitation by its token.
    #[tracing::instrument(name = "Postgres::get_invitation_by_token", skip(token))]
    pub async fn get_invitation_by_token(&self, token: &str) -> Result<Option<Invitation>> {
        let row = sqlx::query!(
            r#"
            SELECT i.id, i.organization_id, r.name as role_name, i.created_by,
                   i.created_at, i.expires_at, i.max_uses, i.use_count, i.revoked_at
            FROM organization_invitation i
            JOIN organization_role r ON i.role_id = r.id
            WHERE i.token = $1
            "#,
            token,
        )
        .fetch_optional(&self.pool)
        .await
        .context("get invitation by token")?;

        match row {
            Some(r) => {
                let role = OrgRole::from_db_name(&r.role_name)
                    .ok_or_else(|| eyre!("unknown role: {}", r.role_name))?;
                Ok(Some(Invitation {
                    id: InvitationId::from_i64(r.id),
                    organization_id: OrgId::from_i64(r.organization_id),
                    role,
                    created_by: AccountId::from_i64(r.created_by),
                    created_at: r.created_at,
                    expires_at: r.expires_at,
                    max_uses: r.max_uses,
                    use_count: r.use_count,
                    revoked_at: r.revoked_at,
                }))
            }
            None => Ok(None),
        }
    }

    /// Get public invitation info for preview (without authentication).
    #[tracing::instrument(name = "Postgres::get_invitation_preview", skip(token))]
    pub async fn get_invitation_preview(&self, token: &str) -> Result<Option<InvitationPreview>> {
        let row = sqlx::query!(
            r#"
            SELECT o.name as org_name, r.name as role_name, i.expires_at, i.revoked_at,
                   i.max_uses, i.use_count
            FROM organization_invitation i
            JOIN organization o ON i.organization_id = o.id
            JOIN organization_role r ON i.role_id = r.id
            WHERE i.token = $1
            "#,
            token,
        )
        .fetch_optional(&self.pool)
        .await
        .context("get invitation preview")?;

        match row {
            Some(r) => {
                let role = OrgRole::from_db_name(&r.role_name)
                    .ok_or_else(|| eyre!("unknown role: {}", r.role_name))?;
                let now = OffsetDateTime::now_utc();
                // Valid if: not revoked, not expired (or never expires), and not at max uses
                let not_expired = r.expires_at.is_none_or(|exp| exp > now);
                let valid = r.revoked_at.is_none()
                    && not_expired
                    && r.max_uses.is_none_or(|max| r.use_count < max);
                Ok(Some(InvitationPreview {
                    organization_name: r.org_name,
                    role,
                    expires_at: r.expires_at,
                    valid,
                }))
            }
            None => Ok(None),
        }
    }

    /// Accept an invitation (atomic: increment use_count, add member, log
    /// redemption).
    ///
    /// Returns the organization info if successful.
    #[tracing::instrument(name = "Postgres::accept_invitation", skip(token))]
    pub async fn accept_invitation(
        &self,
        token: &str,
        account_id: AccountId,
    ) -> Result<AcceptInvitationResult> {
        let mut tx = self.pool.begin().await?;

        let invitation = sqlx::query!(
            r#"
            SELECT i.id, i.organization_id, r.name as role_name, i.expires_at,
                   i.max_uses, i.use_count, i.revoked_at
            FROM organization_invitation i
            JOIN organization_role r ON i.role_id = r.id
            WHERE i.token = $1
            FOR UPDATE
            "#,
            token,
        )
        .fetch_optional(tx.as_mut())
        .await
        .context("fetch invitation for update")?;

        let Some(inv) = invitation else {
            return Ok(AcceptInvitationResult::NotFound);
        };

        // If the invitation expiration is none, that means it never expires.
        let now = OffsetDateTime::now_utc();
        if inv.revoked_at.is_some_and(|revoked| revoked <= now) {
            return Ok(AcceptInvitationResult::Revoked);
        }
        if inv.expires_at.is_some_and(|expires| expires <= now) {
            return Ok(AcceptInvitationResult::Expired);
        }
        if inv.max_uses.is_some_and(|used| inv.use_count >= used) {
            return Ok(AcceptInvitationResult::MaxUsesReached);
        }

        let org_id = OrgId::from_i64(inv.organization_id);
        let role = OrgRole::from_db_name(&inv.role_name)
            .ok_or_else(|| eyre!("unknown role: {}", inv.role_name))?;

        let existing = sqlx::query!(
            r#"
            SELECT 1 as exists
            FROM organization_member
            WHERE organization_id = $1 AND account_id = $2
            "#,
            org_id.as_i64(),
            account_id.as_i64(),
        )
        .fetch_optional(tx.as_mut())
        .await
        .context("check existing membership")?;

        if existing.is_some() {
            return Ok(AcceptInvitationResult::AlreadyMember);
        }

        sqlx::query!(
            r#"
            UPDATE organization_invitation
            SET use_count = use_count + 1
            WHERE id = $1
            "#,
            inv.id,
        )
        .execute(tx.as_mut())
        .await
        .context("increment use count")?;

        sqlx::query!(
            r#"
            INSERT INTO organization_member (organization_id, account_id, role_id)
            VALUES ($1, $2, (SELECT id FROM organization_role WHERE name = $3))
            "#,
            org_id.as_i64(),
            account_id.as_i64(),
            role.as_db_name(),
        )
        .execute(tx.as_mut())
        .await
        .context("add organization member")?;

        sqlx::query!(
            r#"
            INSERT INTO invitation_redemption (invitation_id, account_id)
            VALUES ($1, $2)
            "#,
            inv.id,
            account_id.as_i64(),
        )
        .execute(tx.as_mut())
        .await
        .context("log invitation redemption")?;

        let org = sqlx::query!(
            r#"
            SELECT name FROM organization WHERE id = $1
            "#,
            org_id.as_i64(),
        )
        .fetch_one(tx.as_mut())
        .await
        .context("fetch organization name")?;

        tx.commit().await?;

        Ok(AcceptInvitationResult::Success {
            organization_id: org_id,
            organization_name: org.name,
            role,
        })
    }

    /// Revoke an invitation.
    #[tracing::instrument(name = "Postgres::revoke_invitation")]
    pub async fn revoke_invitation(&self, invitation_id: InvitationId) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            UPDATE organization_invitation
            SET revoked_at = NOW()
            WHERE id = $1 AND revoked_at IS NULL
            "#,
            invitation_id.as_i64(),
        )
        .execute(&self.pool)
        .await
        .context("revoke invitation")?;

        Ok(result.rows_affected() > 0)
    }

    /// List all invitations for an organization.
    #[tracing::instrument(name = "Postgres::list_invitations")]
    pub async fn list_invitations(&self, org_id: OrgId) -> Result<Vec<Invitation>> {
        let rows = sqlx::query!(
            r#"
            SELECT i.id, i.organization_id, r.name as role_name, i.created_by,
                   i.created_at, i.expires_at, i.max_uses, i.use_count, i.revoked_at
            FROM organization_invitation i
            JOIN organization_role r ON i.role_id = r.id
            WHERE i.organization_id = $1
            ORDER BY i.created_at DESC
            "#,
            org_id.as_i64(),
        )
        .fetch_all(&self.pool)
        .await
        .context("list invitations")?;

        rows.into_iter()
            .map(|r| {
                let role = OrgRole::from_db_name(&r.role_name)
                    .ok_or_else(|| eyre!("unknown role: {}", r.role_name))?;
                Ok(Invitation {
                    id: InvitationId::from_i64(r.id),
                    organization_id: OrgId::from_i64(r.organization_id),
                    role,
                    created_by: AccountId::from_i64(r.created_by),
                    created_at: r.created_at,
                    expires_at: r.expires_at,
                    max_uses: r.max_uses,
                    use_count: r.use_count,
                    revoked_at: r.revoked_at,
                })
            })
            .collect()
    }
}
