use aerosol::axum::Dep;
use axum::{extract::Path, http::StatusCode, response::IntoResponse};
use color_eyre::eyre::Report;
use tracing::{error, info};

use crate::{
    api::v1::cas::check_allowed,
    auth::{KeySets, StatelessToken},
    db::Postgres,
    storage::{Disk, Key},
};

/// Check whether the given key exists in the CAS.
///
/// This handler implements the HEAD endpoint for checking blob existence without
/// downloading the full content. It performs authorization and then checks if the
/// blob exists on disk.
///
/// ## TOCTOU (Time of Check Time of Use)
///
/// Normally, developers are advised to avoid "exists" checks since they are
/// prone to "TOCTOU" bugs: when you check if something exists, another process
/// or thread might alter the result (removing or adding the item) before you
/// then can act on the result of that check.
///
/// Here, we allow checking for existence because:
/// - If you check for existence before writing and it doesn't exist, and
///   another client does the same thing, writes are idempotent. The CAS always
///   writes items with a key deterministically derived from the value of the
///   content, so it's safe to write multiple times: at most all but one write
///   is wasted time and bandwidth. Not ideal, but okay.
/// - While we don't recommend checking this before reading (just try to read
///   the value instead), since content in the CAS is idempotent and stored
///   according to a key deterministically derived from the value of the content
///   it's always safe to check for existence before reading too even if another
///   client writes unconditionally.
/// - The exists check is mainly intended to allow clients to avoid having to
///   spend the time and bandwidth re-uploading content that already exists,
///   since this can be non-trivial. This tradeoff seems worth the minor amount
///   of extra complexity/potential confusion that having an existence check may
///   bring to the service.
///
/// ## Security
///
/// All accounts have visibility into all keys that any account in the organization
/// has ever written. This is intentional, because we expect accounts to be used
/// by developers on their local machines as well as in CI or other environments
/// like docker builds.
///
/// Even if another organization has written content with the same key, this
/// content is not visible to the current organization unless they have also
/// written it.
#[tracing::instrument]
pub async fn handle(
    token: StatelessToken,
    Dep(keysets): Dep<KeySets>,
    Dep(db): Dep<Postgres>,
    Dep(cas): Dep<Disk>,
    Path(key): Path<Key>,
) -> CasCheckResponse {
    match check_allowed(&keysets, &db, &key, &token).await {
        Ok(true) => {
            if cas.exists(&key).await {
                info!("cas.check.found");
                CasCheckResponse::Found
            } else {
                info!("cas.check.not_found");
                CasCheckResponse::NotFound
            }
        }
        Ok(false) => {
            info!("cas.check.unauthorized");
            CasCheckResponse::NotFound
        }
        Err(err) => {
            error!(error = ?err, "cas.check.error");
            CasCheckResponse::Error(err)
        }
    }
}

#[derive(Debug)]
pub enum CasCheckResponse {
    Found,
    NotFound,
    Error(Report),
}

impl IntoResponse for CasCheckResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CasCheckResponse::Found => StatusCode::OK.into_response(),
            CasCheckResponse::NotFound => StatusCode::NOT_FOUND.into_response(),
            CasCheckResponse::Error(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:?}")).into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use color_eyre::{Result, eyre::Context};
    use sqlx::PgPool;

    use crate::api::test_helpers::{mint_token, test_blob, write_cas};

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn check_exists(pool: PgPool) -> Result<()> {
        const TOKEN: &str = "test-api-key-account1-org1";
        const CONTENT: &[u8] = b"check exists test";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let token = mint_token(&server, TOKEN, 1).await?;
        let key = write_cas(&server, &token, CONTENT).await?;

        let response = server
            .method(axum::http::Method::HEAD, &format!("/api/v1/cas/{key}"))
            .add_header("Authorization", &token)
            .await;

        response.assert_status_ok();

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn check_doesnt_exist(pool: PgPool) -> Result<()> {
        const TOKEN: &str = "test-api-key-account1-org1";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let token = mint_token(&server, TOKEN, 1).await?;
        let (_, nonexistent_key) = test_blob(b"never written");

        let response = server
            .method(
                axum::http::Method::HEAD,
                &format!("/api/v1/cas/{nonexistent_key}"),
            )
            .add_header("Authorization", &token)
            .await;

        response.assert_status(StatusCode::NOT_FOUND);

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn check_with_no_access(pool: PgPool) -> Result<()> {
        const ORG1_TOKEN: &str = "test-api-key-account1-org1";
        const ORG2_TOKEN: &str = "test-api-key-account1-org2";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        // Org1 writes content
        let org1_token = mint_token(&server, ORG1_TOKEN, 1).await?;
        const CONTENT: &[u8] = b"org1 content";
        let key = write_cas(&server, &org1_token, CONTENT).await?;

        // Org2 checks for it (should not see it)
        let org2_token = mint_token(&server, ORG2_TOKEN, 2).await?;
        let response = server
            .method(axum::http::Method::HEAD, &format!("/api/v1/cas/{key}"))
            .add_header("Authorization", &org2_token)
            .await;

        response.assert_status(StatusCode::NOT_FOUND);

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn check_cross_org_access(pool: PgPool) -> Result<()> {
        const ORG1_TOKEN: &str = "test-api-key-account1-org1";
        const ORG2_TOKEN: &str = "test-api-key-account1-org2";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        // Org1 writes content
        let org1_token = mint_token(&server, ORG1_TOKEN, 1).await?;
        const CONTENT: &[u8] = b"org1 content";
        let key = write_cas(&server, &org1_token, CONTENT).await?;

        // Org1 checks for it (should see it)
        let response = server
            .method(axum::http::Method::HEAD, &format!("/api/v1/cas/{key}"))
            .add_header("Authorization", &org1_token)
            .await;
        response.assert_status_ok();

        // Org2 checks for it (should not see it)
        let org2_token = mint_token(&server, ORG2_TOKEN, 2).await?;
        let response = server
            .method(axum::http::Method::HEAD, &format!("/api/v1/cas/{key}"))
            .add_header("Authorization", &org2_token)
            .await;
        response.assert_status(StatusCode::NOT_FOUND);

        // Org2 writes content and should now see it
        let key = write_cas(&server, &org2_token, CONTENT).await?;
        let response = server
            .method(axum::http::Method::HEAD, &format!("/api/v1/cas/{key}"))
            .add_header("Authorization", &org2_token)
            .await;
        response.assert_status_ok();

        // And Org1 should still see it
        let response = server
            .method(axum::http::Method::HEAD, &format!("/api/v1/cas/{key}"))
            .add_header("Authorization", &org1_token)
            .await;
        response.assert_status_ok();

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn check_then_write_toctou_safety(pool: PgPool) -> Result<()> {
        const TOKEN: &str = "test-api-key-account1-org1";
        const CONTENT: &[u8] = b"toctou test";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let token = mint_token(&server, TOKEN, 1).await?;
        let (_, key) = test_blob(CONTENT);

        // Check before write
        let check1 = server
            .method(axum::http::Method::HEAD, &format!("/api/v1/cas/{key}"))
            .add_header("Authorization", &token)
            .await;
        check1.assert_status(StatusCode::NOT_FOUND);

        // Write content
        write_cas(&server, &token, CONTENT).await?;

        // Check after write
        let check2 = server
            .method(axum::http::Method::HEAD, &format!("/api/v1/cas/{key}"))
            .add_header("Authorization", &token)
            .await;
        check2.assert_status_ok();

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn check_org_member_access(pool: PgPool) -> Result<()> {
        const ACCOUNT1_TOKEN: &str = "test-api-key-account1-org1";
        const ACCOUNT2_TOKEN: &str = "test-api-key-account2-org1";
        const CONTENT: &[u8] = b"team content";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        // Account1 writes content
        let account1_token = mint_token(&server, ACCOUNT1_TOKEN, 1).await?;
        let key = write_cas(&server, &account1_token, CONTENT).await?;

        // Account2 (same org) can check it
        let account2_token = mint_token(&server, ACCOUNT2_TOKEN, 1).await?;
        let response = server
            .method(axum::http::Method::HEAD, &format!("/api/v1/cas/{key}"))
            .add_header("Authorization", &account2_token)
            .await;

        response.assert_status_ok();

        Ok(())
    }
}
