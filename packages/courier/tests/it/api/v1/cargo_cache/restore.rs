//! Cargo cache restore endpoint tests.

use clients::courier::v1::{
    GlibcVersion, SavedUnitHash,
    cache::{CargoRestoreRequest, CargoSaveRequest, CargoSaveUnitRequest},
};
use color_eyre::Result;
use pretty_assertions::assert_eq as pretty_assert_eq;
use sqlx::PgPool;

use crate::helpers::{TestFixture, test_saved_unit};

const GLIBC_VERSION: GlibcVersion = GlibcVersion {
    major: 2,
    minor: 41,
    patch: 0,
};

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn restore_after_save(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let unit = test_saved_unit("serde-v1");
    let key = unit.unit_hash().clone();
    let request = CargoSaveUnitRequest::builder()
        .unit(unit.clone())
        .resolved_target(String::from("x86_64-unknown-linux-gnu"))
        .maybe_linux_glibc_version(Some(GLIBC_VERSION))
        .build();

    let save_request = CargoSaveRequest::new([request]);
    fixture.client_alice.cargo_cache_save(save_request).await?;

    let restore_request = CargoRestoreRequest::new([key.clone()], Some(GLIBC_VERSION));
    let response = fixture
        .client_alice
        .cargo_cache_restore(restore_request)
        .await?;

    let restored_unit = response
        .iter()
        .find(|(k, _)| *k == &key)
        .map(|(_, v)| v)
        .expect("unit should be in response");
    pretty_assert_eq!(restored_unit, &unit);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn restore_nonexistent_cache(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let key = SavedUnitHash::from("nonexistent");
    let restore_request = CargoRestoreRequest::new([key], Some(GLIBC_VERSION));

    let response = fixture
        .client_alice
        .cargo_cache_restore(restore_request)
        .await?;
    assert!(
        response.is_empty(),
        "restore of nonexistent cache should be empty"
    );

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn restore_multiple_units(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let units = [
        ("hash-serde", test_saved_unit("hash-serde")),
        ("hash-tokio", test_saved_unit("hash-tokio")),
        ("hash-axum", test_saved_unit("hash-axum")),
    ];

    let requests = units
        .iter()
        .map(|(hash, unit)| {
            let unit = unit.clone();
            assert_eq!(unit.unit_hash().as_str(), *hash);
            CargoSaveUnitRequest::builder()
                .unit(unit)
                .resolved_target(String::from("x86_64-unknown-linux-gnu"))
                .maybe_linux_glibc_version(Some(GLIBC_VERSION))
                .build()
        })
        .collect::<Vec<_>>();

    let save_request = CargoSaveRequest::new(requests);
    fixture.client_alice.cargo_cache_save(save_request).await?;

    let keys = units
        .iter()
        .map(|(hash, _)| SavedUnitHash::from(*hash))
        .collect::<Vec<_>>();

    let restore_request = CargoRestoreRequest::new(keys.clone(), Some(GLIBC_VERSION));
    let response = fixture
        .client_alice
        .cargo_cache_restore(restore_request)
        .await?;

    for ((hash, unit), key) in units.iter().zip(keys.iter()) {
        let restored_unit = response
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v)
            .unwrap_or_else(|| panic!("unit {hash} should be in response"));
        pretty_assert_eq!(restored_unit, unit);
    }

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn restore_partial_miss(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let unit = test_saved_unit("hash-exists");
    let key_exists = unit.unit_hash().clone();
    let request = CargoSaveUnitRequest::builder()
        .unit(unit.clone())
        .resolved_target(String::from("x86_64-unknown-linux-gnu"))
        .maybe_linux_glibc_version(Some(GLIBC_VERSION))
        .build();

    let save_request = CargoSaveRequest::new([request]);
    fixture.client_alice.cargo_cache_save(save_request).await?;

    let key_missing = SavedUnitHash::from("hash-missing");
    let restore_request = CargoRestoreRequest::new(
        [key_exists.clone(), key_missing.clone()],
        Some(GLIBC_VERSION),
    );
    let response = fixture
        .client_alice
        .cargo_cache_restore(restore_request)
        .await?;

    let has_exists = response.iter().any(|(k, _)| k == &key_exists);
    let has_missing = response.iter().any(|(k, _)| k == &key_missing);
    assert!(has_exists, "existing unit should be in response");
    assert!(!has_missing, "missing unit should not be in response");

    let restored_unit = response
        .iter()
        .find(|(k, _)| *k == &key_exists)
        .map(|(_, v)| v)
        .expect("existing unit should be available");
    pretty_assert_eq!(restored_unit, &unit);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn concurrent_restores_same_cache(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let unit = test_saved_unit("concurrent-v1");
    let key = unit.unit_hash().clone();
    let request = CargoSaveUnitRequest::builder()
        .unit(unit.clone())
        .resolved_target(String::from("x86_64-unknown-linux-gnu"))
        .maybe_linux_glibc_version(Some(GLIBC_VERSION))
        .build();

    let save_request = CargoSaveRequest::new([request]);
    fixture.client_alice.cargo_cache_save(save_request).await?;

    let restores = (0..10)
        .map(|_| {
            let restore_request = CargoRestoreRequest::new([key.clone()], Some(GLIBC_VERSION));
            fixture.client_alice.cargo_cache_restore(restore_request)
        })
        .collect::<Vec<_>>();

    let results = futures::future::try_join_all::<Vec<_>>(restores).await?;
    for response in results {
        let restored_unit = response
            .iter()
            .find(|(k, _)| *k == &key)
            .map(|(_, v)| v)
            .expect("unit should be in response");
        pretty_assert_eq!(restored_unit, &unit);
    }

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn org_cannot_restore_other_orgs_cache(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let unit = test_saved_unit("hash-org-a");
    let key = unit.unit_hash().clone();
    let request = CargoSaveUnitRequest::builder()
        .unit(unit)
        .resolved_target(String::from("x86_64-unknown-linux-gnu"))
        .maybe_linux_glibc_version(Some(GLIBC_VERSION))
        .build();

    let save_request = CargoSaveRequest::new([request]);
    fixture.client_alice.cargo_cache_save(save_request).await?;

    let restore_request = CargoRestoreRequest::new([key], Some(GLIBC_VERSION));
    let response = fixture
        .client_charlie
        .cargo_cache_restore(restore_request)
        .await?;
    assert!(
        response.is_empty(),
        "org B should not be able to restore org A's cache"
    );

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn restore_missing_auth_returns_401(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let key = SavedUnitHash::from("hash-noauth");
    let restore_request = CargoRestoreRequest::new([key], Some(GLIBC_VERSION));

    let client_no_auth = fixture.client_with_token("")?;
    let result = client_no_auth.cargo_cache_restore(restore_request).await;
    assert!(result.is_err(), "restore without auth should fail");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn restore_invalid_token_returns_401(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let key = SavedUnitHash::from("hash-invalidtoken");

    let restore_request = CargoRestoreRequest::new([key], Some(GLIBC_VERSION));
    let client = fixture.client_with_token("invalid-token-that-does-not-exist")?;
    let result = client.cargo_cache_restore(restore_request).await;
    assert!(result.is_err(), "restore with invalid token should fail");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn restore_revoked_token_returns_401(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let key = SavedUnitHash::from("hash-revokedtoken");

    let restore_request = CargoRestoreRequest::new([key], Some(GLIBC_VERSION));
    let client = fixture.client_with_token(fixture.auth.token_alice_revoked().expose())?;
    let result = client.cargo_cache_restore(restore_request).await;
    assert!(result.is_err(), "restore with revoked token should fail");

    Ok(())
}
