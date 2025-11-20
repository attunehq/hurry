//! Cargo cache save endpoint tests.

use clients::courier::v1::cache::{
    CargoRestoreRequest2, CargoSaveRequest2, CargoSaveUnitRequest, SavedUnitCacheKey,
};
use color_eyre::Result;
use pretty_assertions::assert_eq as pretty_assert_eq;
use sqlx::PgPool;
use tap::Pipe;

use crate::helpers::{TestFixture, test_saved_unit};

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn basic_save_flow(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let unit = test_saved_unit("hash-basic");
    let key = SavedUnitCacheKey::builder().unit("hash-basic").build();
    let request = CargoSaveUnitRequest::builder()
        .key(&key)
        .unit(unit.clone())
        .build();
    let save_request = CargoSaveRequest2::new([request]);

    fixture.client_alice.cargo_cache_save2(save_request).await?;

    let restore_request = CargoRestoreRequest2::new([key.clone()]);
    let response = fixture
        .client_alice
        .cargo_cache_restore2(restore_request)
        .await?
        .expect("restore should return data");

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
    let key = SavedUnitCacheKey::builder().unit("hash-idempotent").build();
    let request = CargoSaveUnitRequest::builder()
        .key(&key)
        .unit(unit.clone())
        .build();
    let save_request = CargoSaveRequest2::new([request.clone()]);

    fixture
        .client_alice
        .cargo_cache_save2(save_request.clone())
        .await?;
    fixture.client_alice.cargo_cache_save2(save_request).await?;

    let restore_request = CargoRestoreRequest2::new([key.clone()]);
    let response = fixture
        .client_alice
        .cargo_cache_restore2(restore_request)
        .await?
        .expect("restore should return data");

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
            let key = SavedUnitCacheKey::builder().unit(*hash).build();
            CargoSaveUnitRequest::builder()
                .key(&key)
                .unit(unit.clone())
                .build()
        })
        .collect::<Vec<_>>();

    let save_request = CargoSaveRequest2::new(requests);
    fixture.client_alice.cargo_cache_save2(save_request).await?;

    let keys = units
        .iter()
        .map(|(hash, _)| SavedUnitCacheKey::builder().unit(*hash).build())
        .collect::<Vec<_>>();

    let restore_request = CargoRestoreRequest2::new(keys.clone());
    let response = fixture
        .client_alice
        .cargo_cache_restore2(restore_request)
        .await?
        .expect("restore should return data");

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
            let key = SavedUnitCacheKey::builder().unit(*hash).build();
            CargoSaveUnitRequest::builder()
                .key(&key)
                .unit(unit.clone())
                .build()
        })
        .collect::<Vec<_>>();

    let save_request = CargoSaveRequest2::new(requests);
    fixture.client_alice.cargo_cache_save2(save_request).await?;

    let keys = units
        .iter()
        .map(|(hash, _)| SavedUnitCacheKey::builder().unit(*hash).build())
        .collect::<Vec<_>>();

    let restore_request = CargoRestoreRequest2::new(keys.clone());
    let response = fixture
        .client_alice
        .cargo_cache_restore2(restore_request)
        .await?
        .expect("restore should return data");

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
            let key = SavedUnitCacheKey::builder().unit(hash).build();
            let request = CargoSaveUnitRequest::builder()
                .key(&key)
                .unit(unit.clone())
                .build();
            let save_request = CargoSaveRequest2::new([request]);
            fixture.client_alice.cargo_cache_save2(save_request)
        })
        .collect::<Vec<_>>()
        .pipe(futures::future::try_join_all)
        .await?;

    let keys = units
        .iter()
        .map(|(hash, _)| SavedUnitCacheKey::builder().unit(hash).build())
        .collect::<Vec<_>>();

    let restore_request = CargoRestoreRequest2::new(keys.clone());
    let response = fixture
        .client_alice
        .cargo_cache_restore2(restore_request)
        .await?
        .expect("restore should return data");

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
    let key = SavedUnitCacheKey::builder().unit("hash-noauth").build();

    let request = CargoSaveUnitRequest::builder().key(&key).unit(unit).build();
    let save_request = CargoSaveRequest2::new([request]);

    let client_no_auth = fixture.client_with_token("")?;
    let result = client_no_auth.cargo_cache_save2(save_request).await;
    assert!(result.is_err(), "save without auth should fail");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn save_invalid_token_returns_401(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let unit = test_saved_unit("hash-invalidtoken");
    let key = SavedUnitCacheKey::builder()
        .unit("hash-invalidtoken")
        .build();

    let request = CargoSaveUnitRequest::builder().key(&key).unit(unit).build();
    let save_request = CargoSaveRequest2::new([request]);

    let client = fixture.client_with_token("invalid-token-that-does-not-exist")?;
    let result = client.cargo_cache_save2(save_request).await;
    assert!(result.is_err(), "save with invalid token should fail");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn save_revoked_token_returns_401(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let unit = test_saved_unit("hash-revokedtoken");
    let key = SavedUnitCacheKey::builder()
        .unit("hash-revokedtoken")
        .build();

    let request = CargoSaveUnitRequest::builder().key(&key).unit(unit).build();
    let save_request = CargoSaveRequest2::new([request]);

    let client = fixture.client_with_token(fixture.auth.token_alice_revoked().expose())?;
    let result = client.cargo_cache_save2(save_request).await;
    assert!(result.is_err(), "save with revoked token should fail");

    Ok(())
}
