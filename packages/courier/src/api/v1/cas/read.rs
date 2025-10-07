use aerosol::axum::Dep;
use axum::{body::Body, extract::Path, http::StatusCode, response::IntoResponse};
use color_eyre::eyre::Report;
use tokio_util::io::ReaderStream;
use tracing::{error, info, warn};

use crate::{
    api::v1::cas::check_allowed,
    auth::{KeySets, StatelessToken},
    db::Postgres,
    storage::{Disk, Key},
};

/// Read the content from the CAS for the given key.
///
/// ## Security
///
/// All users have visibility into all keys that any user in the organization
/// has ever written. This is intentional, because we expect users to run hurry
/// on their local dev machines as well as in CI or other environments like
/// docker builds.
///
/// Even if another organization has written content with the same key, this
/// content is not visible to the current organization unless they have also
/// written it.
#[tracing::instrument]
pub async fn handle(
    token: StatelessToken,
    Dep(keysets): Dep<KeySets>,
    Dep(cas): Dep<Disk>,
    Dep(db): Dep<Postgres>,
    Path(key): Path<Key>,
) -> CasReadResponse {
    match check_allowed(&keysets, &db, &key, &token).await {
        Ok(true) => match cas.read(&key).await {
            Ok(reader) => {
                // We record access frequency asynchronously to avoid blocking
                // the overall request, since access frequency is a "nice to
                // have" feature while latency is a "must have" feature.
                let user_id = token.user_id;
                tokio::spawn(async move {
                    if let Err(err) = db.record_cas_key_access(user_id, &key).await {
                        warn!(error = ?err, "cas.read.record_access_failed");
                    }
                });

                info!("cas.read.success");

                let stream = ReaderStream::new(reader);
                CasReadResponse::Found(Body::from_stream(stream))
            }
            Err(err) => {
                error!(error = ?err, "cas.read.error");
                CasReadResponse::Error(err)
            }
        },
        Ok(false) => {
            info!("cas.read.not_found");
            CasReadResponse::NotFound
        }
        Err(err) => {
            error!(error = ?err, "cas.read.check_allowed_error");
            CasReadResponse::Error(err)
        }
    }
}

#[derive(Debug)]
pub enum CasReadResponse {
    Found(Body),
    NotFound,
    Error(Report),
}

impl IntoResponse for CasReadResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CasReadResponse::Found(body) => (StatusCode::OK, body).into_response(),
            CasReadResponse::NotFound => StatusCode::NOT_FOUND.into_response(),
            CasReadResponse::Error(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:?}")).into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use color_eyre::{Result, eyre::Context};
    use pretty_assertions::assert_eq as pretty_assert_eq;
    use sqlx::PgPool;

    use crate::api::test_helpers::{mint_token, test_blob, write_cas};

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn read_after_write(pool: PgPool) -> Result<()> {
        const TOKEN: &str = "test-token:user1@test1.com";
        const CONTENT: &[u8] = b"read test content";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let token = mint_token(&server, TOKEN, 1).await?;
        let key = write_cas(&server, &token, CONTENT).await?;

        let response = server
            .get(&format!("/api/v1/cas/{key}"))
            .add_header("Authorization", &token)
            .await;

        response.assert_status_ok();
        let body = response.as_bytes();
        pretty_assert_eq!(body.as_ref(), CONTENT);

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn read_nonexistent_key(pool: PgPool) -> Result<()> {
        const TOKEN: &str = "test-token:user1@test1.com";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let token = mint_token(&server, TOKEN, 1).await?;
        let (_, nonexistent_key) = test_blob(b"never written");

        let response = server
            .get(&format!("/api/v1/cas/{nonexistent_key}"))
            .add_header("Authorization", &token)
            .await;

        response.assert_status(StatusCode::NOT_FOUND);

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn read_with_no_access(pool: PgPool) -> Result<()> {
        const ORG1_TOKEN: &str = "test-token:user1@test1.com";
        const ORG2_TOKEN: &str = "test-token:user1@test2.com";
        const CONTENT: &[u8] = b"org1 private content";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        // Org1 writes content
        let org1_token = mint_token(&server, ORG1_TOKEN, 1).await?;
        let key = write_cas(&server, &org1_token, CONTENT).await?;

        // Org2 tries to read it
        let org2_token = mint_token(&server, ORG2_TOKEN, 2).await?;
        let response = server
            .get(&format!("/api/v1/cas/{key}"))
            .add_header("Authorization", &org2_token)
            .await;

        response.assert_status(StatusCode::NOT_FOUND);

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn read_grants_access_to_org_member(pool: PgPool) -> Result<()> {
        const USER1_TOKEN: &str = "test-token:user1@test1.com";
        const USER2_TOKEN: &str = "test-token:user2@test1.com";
        const CONTENT: &[u8] = b"team shared content";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        // User1 writes content
        let user1_token = mint_token(&server, USER1_TOKEN, 1).await?;
        let key = write_cas(&server, &user1_token, CONTENT).await?;

        // User2 (same org) reads it
        let user2_token = mint_token(&server, USER2_TOKEN, 1).await?;
        let response = server
            .get(&format!("/api/v1/cas/{key}"))
            .add_header("Authorization", &user2_token)
            .await;

        response.assert_status_ok();
        let body = response.as_bytes();
        pretty_assert_eq!(body.as_ref(), CONTENT);

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn read_large_blob(pool: PgPool) -> Result<()> {
        const TOKEN: &str = "test-token:user1@test1.com";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let token = mint_token(&server, TOKEN, 1).await?;
        let content = vec![0xFF; 5 * 1024 * 1024]; // 5MB blob
        let key = write_cas(&server, &token, &content).await?;

        let response = server
            .get(&format!("/api/v1/cas/{key}"))
            .add_header("Authorization", &token)
            .await;

        response.assert_status_ok();
        let body = response.as_bytes();
        pretty_assert_eq!(body.len(), content.len());
        pretty_assert_eq!(body.as_ref(), content.as_slice());

        Ok(())
    }
}
