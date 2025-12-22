//! Cargo cache reset endpoint tests.

use clients::courier::v1::{
    GlibcVersion,
    cache::{CargoRestoreRequest, CargoSaveRequest, CargoSaveUnitRequest},
};
use color_eyre::Result;
use pretty_assertions::assert_eq as pretty_assert_eq;
use reqwest::StatusCode;
use sqlx::PgPool;

use crate::helpers::{TestFixture, test_saved_unit};

const GLIBC_VERSION: GlibcVersion = GlibcVersion {
    major: 2,
    minor: 41,
    patch: 0,
};

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn resets_cache(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let unit = test_saved_unit("hash-reset");
    let request = CargoSaveUnitRequest::builder()
        .unit(unit)
        .resolved_target(String::from("x86_64-unknown-linux-gnu"))
        .maybe_linux_glibc_version(Some(GLIBC_VERSION))
        .build();
    let save_request = CargoSaveRequest::new([request]);

    fixture.client_alice.cargo_cache_save(save_request).await?;

    let count_before = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM cargo_saved_unit")
        .fetch_one(&fixture.db.pool)
        .await?;
    assert!(
        count_before > 0,
        "database should contain saved unit before reset"
    );

    fixture.client_alice.cache_reset().await?;

    let count_after = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM cargo_saved_unit")
        .fetch_one(&fixture.db.pool)
        .await?;
    assert!(count_after == 0, "database should be empty after reset");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn org_reset_only_deletes_own_data(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let unit_alice = test_saved_unit("hash-alice");
    let key_alice = unit_alice.unit_hash().clone();
    let request_alice = CargoSaveUnitRequest::builder()
        .unit(unit_alice)
        .resolved_target(String::from("x86_64-unknown-linux-gnu"))
        .maybe_linux_glibc_version(Some(GLIBC_VERSION))
        .build();
    let save_request_alice = CargoSaveRequest::new([request_alice]);

    let unit_charlie = test_saved_unit("hash-charlie");
    let key_charlie = unit_charlie.unit_hash().clone();
    let request_charlie = CargoSaveUnitRequest::builder()
        .unit(unit_charlie)
        .resolved_target(String::from("x86_64-unknown-linux-gnu"))
        .maybe_linux_glibc_version(Some(GLIBC_VERSION))
        .build();
    let save_request_charlie = CargoSaveRequest::new([request_charlie]);

    fixture
        .client_alice
        .cargo_cache_save(save_request_alice)
        .await?;
    fixture
        .client_charlie
        .cargo_cache_save(save_request_charlie)
        .await?;

    fixture.client_alice.cache_reset().await?;

    let restore_alice = CargoRestoreRequest::new([key_alice], Some(GLIBC_VERSION));
    let response_alice = fixture
        .client_alice
        .cargo_cache_restore(restore_alice)
        .await?;
    assert!(response_alice.is_empty(), "org A's cache should be deleted");

    let restore_charlie = CargoRestoreRequest::new([key_charlie], Some(GLIBC_VERSION));
    let response_charlie = fixture
        .client_charlie
        .cargo_cache_restore(restore_charlie)
        .await?;
    assert!(
        !response_charlie.is_empty(),
        "org B's cache should still exist"
    );

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn non_admin_forbidden(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let url = fixture.base_url.join("api/v1/cache/cargo/reset")?;

    // Bob is a member (not admin) of Acme Corp
    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(fixture.auth.token_bob().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}
