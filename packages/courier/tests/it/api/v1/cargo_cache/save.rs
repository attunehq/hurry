//! Cargo cache save endpoint tests.

use clients::courier::v1::{
    GlibcVersion, SavedUnitHash,
    cache::{CargoRestoreRequest, CargoSaveRequest, CargoSaveUnitRequest},
};
use color_eyre::Result;
use pretty_assertions::assert_eq as pretty_assert_eq;
use sqlx::PgPool;
use tap::Pipe;

use crate::helpers::{TestFixture, test_saved_unit};

const GLIBC_VERSION: GlibcVersion = GlibcVersion {
    major: 2,
    minor: 41,
    patch: 0,
};

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn basic_save_flow(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let unit = test_saved_unit("hash-basic");
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
        .expect("unit should be restored");
    pretty_assert_eq!(restored_unit, &unit);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn idempotent_saves(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let unit = test_saved_unit("hash-idempotent");
    let key = unit.unit_hash().clone();
    let request = CargoSaveUnitRequest::builder()
        .unit(unit.clone())
        .resolved_target(String::from("x86_64-unknown-linux-gnu"))
        .maybe_linux_glibc_version(Some(GLIBC_VERSION))
        .build();
    let save_request = CargoSaveRequest::new([request.clone()]);

    fixture
        .client_alice
        .cargo_cache_save(save_request.clone())
        .await?;
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
        .expect("unit should be restored");
    pretty_assert_eq!(restored_unit, &unit);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn save_multiple_packages(pool: PgPool) -> Result<()> {
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
            // Ensure the unit hash matches the test hash
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
            .unwrap_or_else(|| panic!("unit {hash} should be restored"));
        pretty_assert_eq!(restored_unit, unit);
    }

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn save_same_package_different_hashes(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let units = [
        ("serde-v1", test_saved_unit("serde-v1")),
        ("serde-v2", test_saved_unit("serde-v2")),
        ("serde-v3", test_saved_unit("serde-v3")),
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
            .unwrap_or_else(|| panic!("unit {hash} should be restored"));
        pretty_assert_eq!(restored_unit, unit);
    }

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn concurrent_saves_different_packages(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let units = (0..10)
        .map(|i| {
            let hash = format!("hash-{i}");
            (hash.clone(), test_saved_unit(&hash))
        })
        .collect::<Vec<_>>();

    units
        .iter()
        .map(|(hash, unit)| {
            let unit = unit.clone();
            assert_eq!(unit.unit_hash().as_str(), hash);
            let request = CargoSaveUnitRequest::builder()
                .unit(unit)
                .resolved_target(String::from("x86_64-unknown-linux-gnu"))
                .maybe_linux_glibc_version(Some(GLIBC_VERSION))
                .build();
            let save_request = CargoSaveRequest::new([request]);
            fixture.client_alice.cargo_cache_save(save_request)
        })
        .collect::<Vec<_>>()
        .pipe(futures::future::try_join_all)
        .await?;

    let keys = units
        .iter()
        .map(|(hash, _)| SavedUnitHash::from(hash.as_str()))
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
            .unwrap_or_else(|| panic!("unit {hash} should be restored"));
        pretty_assert_eq!(restored_unit, unit);
    }

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn save_missing_auth_returns_401(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let unit = test_saved_unit("hash-noauth");
    let request = CargoSaveUnitRequest::builder()
        .unit(unit)
        .resolved_target(String::from("x86_64-unknown-linux-gnu"))
        .maybe_linux_glibc_version(Some(GLIBC_VERSION))
        .build();
    let save_request = CargoSaveRequest::new([request]);

    let client_no_auth = fixture.client_with_token("")?;
    let result = client_no_auth.cargo_cache_save(save_request).await;
    assert!(result.is_err(), "save without auth should fail");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn save_invalid_token_returns_401(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let unit = test_saved_unit("hash-invalidtoken");
    let request = CargoSaveUnitRequest::builder()
        .unit(unit)
        .resolved_target(String::from("x86_64-unknown-linux-gnu"))
        .maybe_linux_glibc_version(Some(GLIBC_VERSION))
        .build();
    let save_request = CargoSaveRequest::new([request]);

    let client = fixture.client_with_token("invalid-token-that-does-not-exist")?;
    let result = client.cargo_cache_save(save_request).await;
    assert!(result.is_err(), "save with invalid token should fail");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn save_revoked_token_returns_401(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let unit = test_saved_unit("hash-revokedtoken");
    let request = CargoSaveUnitRequest::builder()
        .unit(unit)
        .resolved_target(String::from("x86_64-unknown-linux-gnu"))
        .maybe_linux_glibc_version(Some(GLIBC_VERSION))
        .build();
    let save_request = CargoSaveRequest::new([request]);

    let client = fixture.client_with_token(fixture.auth.token_alice_revoked().expose())?;
    let result = client.cargo_cache_save(save_request).await;
    assert!(result.is_err(), "save with revoked token should fail");

    Ok(())
}
