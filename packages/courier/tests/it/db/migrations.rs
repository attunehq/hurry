//! Migration validation tests.

use color_eyre::Result;
use courier::db::Postgres;
use sqlx::PgPool;

/// Test that migration validation fails when migrations haven't been applied.
///
/// This test uses `migrations = false` to get a database without migrations,
/// then verifies that `validate_migrations` correctly detects the pending
/// state.
#[sqlx::test(migrations = false)]
async fn validate_migrations_fails_without_migrations(pool: PgPool) -> Result<()> {
    let db = Postgres { pool };

    let result = db.validate_migrations().await;
    assert!(result.is_err(), "should fail when migrations are pending");

    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("pending migrations"),
        "error should mention pending migrations: {err}"
    );
    assert!(
        err.contains("Run 'courier migrate' first"),
        "error should give actionable guidance: {err}"
    );

    Ok(())
}

/// Test that migration validation succeeds when all migrations are applied.
#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn validate_migrations_succeeds_with_migrations(pool: PgPool) -> Result<()> {
    let db = Postgres { pool };

    db.validate_migrations().await?;

    Ok(())
}
