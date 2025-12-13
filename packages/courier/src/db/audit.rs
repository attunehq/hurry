//! Audit log database operations.

use color_eyre::{Result, eyre::Context};

use super::Postgres;
use crate::auth::{AccountId, OrgId};

impl Postgres {
    /// Log an audit event.
    #[tracing::instrument(name = "Postgres::log_audit_event", skip(details))]
    pub async fn log_audit_event(
        &self,
        account_id: Option<AccountId>,
        organization_id: Option<OrgId>,
        action: &str,
        details: Option<serde_json::Value>,
    ) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO audit_log (account_id, organization_id, action, details)
            VALUES ($1, $2, $3, $4)
            "#,
            account_id.map(|id| id.as_i64()),
            organization_id.map(|id| id.as_i64()),
            action,
            details,
        )
        .execute(&self.pool)
        .await
        .context("log audit event")?;

        Ok(())
    }
}
