//! Audit log database operations.

use color_eyre::{Result, eyre::Context};
use time::OffsetDateTime;

use super::Postgres;
use crate::auth::{AccountId, OrgId};

/// A single audit log entry.
#[derive(Debug)]
pub struct AuditLogEntry {
    pub id: i64,
    pub account_id: Option<AccountId>,
    pub action: String,
    pub details: Option<serde_json::Value>,
    pub created_at: OffsetDateTime,
    /// The account's email at the time of the query (if account still exists).
    pub account_email: Option<String>,
    /// The account's name at the time of the query (if account still exists).
    pub account_name: Option<String>,
}

/// Cursor for paginating audit log entries.
///
/// Uses (created_at, id) for stable ordering since multiple events can have the
/// same timestamp.
#[derive(Debug, Clone)]
pub struct AuditLogCursor {
    pub created_at: OffsetDateTime,
    pub id: i64,
}

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

    /// List audit log entries for an organization using cursor-based
    /// pagination.
    ///
    /// Returns entries ordered by most recent first. Pass `None` for `cursor`
    /// to get the first page. Use the last entry's (created_at, id) as the
    /// cursor for subsequent pages.
    #[tracing::instrument(name = "Postgres::list_audit_log")]
    pub async fn list_audit_log(
        &self,
        organization_id: OrgId,
        limit: i64,
        cursor: Option<AuditLogCursor>,
    ) -> Result<Vec<AuditLogEntry>> {
        match cursor {
            Some(cursor) => {
                let rows = sqlx::query!(
                    r#"
                    SELECT
                        al.id,
                        al.account_id,
                        al.action,
                        al.details,
                        al.created_at,
                        a.email AS "account_email?",
                        a.name AS "account_name?"
                    FROM audit_log al
                    LEFT JOIN account a ON al.account_id = a.id
                    WHERE al.organization_id = $1
                      AND (al.created_at, al.id) < ($2, $3)
                    ORDER BY al.created_at DESC, al.id DESC
                    LIMIT $4
                    "#,
                    organization_id.as_i64(),
                    cursor.created_at,
                    cursor.id,
                    limit,
                )
                .fetch_all(&self.pool)
                .await
                .context("list audit log with cursor")?;

                Ok(rows
                    .into_iter()
                    .map(|row| AuditLogEntry {
                        id: row.id,
                        account_id: row.account_id.map(AccountId::from_i64),
                        action: row.action,
                        details: row.details,
                        created_at: row.created_at,
                        account_email: row.account_email,
                        account_name: row.account_name,
                    })
                    .collect())
            }
            None => {
                let rows = sqlx::query!(
                    r#"
                    SELECT
                        al.id,
                        al.account_id,
                        al.action,
                        al.details,
                        al.created_at,
                        a.email AS "account_email?",
                        a.name AS "account_name?"
                    FROM audit_log al
                    LEFT JOIN account a ON al.account_id = a.id
                    WHERE al.organization_id = $1
                    ORDER BY al.created_at DESC, al.id DESC
                    LIMIT $2
                    "#,
                    organization_id.as_i64(),
                    limit,
                )
                .fetch_all(&self.pool)
                .await
                .context("list audit log")?;

                Ok(rows
                    .into_iter()
                    .map(|row| AuditLogEntry {
                        id: row.id,
                        account_id: row.account_id.map(AccountId::from_i64),
                        action: row.action,
                        details: row.details,
                        created_at: row.created_at,
                        account_email: row.account_email,
                        account_name: row.account_name,
                    })
                    .collect())
            }
        }
    }
}
