//! Cargo cache reset endpoint tests.

use clients::courier::v1::cache::{
    CargoRestoreRequest2, CargoSaveRequest2, CargoSaveUnitRequest, SavedUnitCacheKey,
};
use color_eyre::Result;
use sqlx::PgPool;

use crate::helpers::{TestFixture, test_saved_unit};

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn resets_cache(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let unit = test_saved_unit("hash-reset");
    let key = SavedUnitCacheKey::builder().unit("hash-reset").build();
    let request = CargoSaveUnitRequest::builder().key(&key).unit(unit).build();
    let save_request = CargoSaveRequest2::new([request]);

    fixture.client_alice.cargo_cache_save2(save_request).await?;

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
    let key_alice = SavedUnitCacheKey::builder().unit("hash-alice").build();
    let request_alice = CargoSaveUnitRequest::builder()
        .key(&key_alice)
        .unit(unit_alice)
        .build();
    let save_request_alice = CargoSaveRequest2::new([request_alice]);

    let unit_charlie = test_saved_unit("hash-charlie");
    let key_charlie = SavedUnitCacheKey::builder().unit("hash-charlie").build();
    let request_charlie = CargoSaveUnitRequest::builder()
        .key(&key_charlie)
        .unit(unit_charlie)
        .build();
    let save_request_charlie = CargoSaveRequest2::new([request_charlie]);

    fixture
        .client_alice
        .cargo_cache_save2(save_request_alice)
        .await?;
    fixture
        .client_charlie
        .cargo_cache_save2(save_request_charlie)
        .await?;

    fixture.client_alice.cache_reset().await?;

    let restore_alice = CargoRestoreRequest2::new([key_alice]);
    let response_alice = fixture
        .client_alice
        .cargo_cache_restore2(restore_alice)
        .await?;
    assert!(response_alice.is_none(), "org A's cache should be deleted");

    let restore_charlie = CargoRestoreRequest2::new([key_charlie]);
    let response_charlie = fixture
        .client_charlie
        .cargo_cache_restore2(restore_charlie)
        .await?;
    assert!(
        response_charlie.is_some(),
        "org B's cache should still exist"
    );

    Ok(())
}
