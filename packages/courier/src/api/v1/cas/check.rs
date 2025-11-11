use aerosol::axum::Dep;
use axum::{extract::Path, http::StatusCode, response::IntoResponse};
use color_eyre::eyre::Report;
use tracing::{error, info};

use crate::{
    auth::AuthenticatedToken,
    db::Postgres,
    storage::{Disk, Key},
};

/// Check whether the given key exists in the CAS.
///
/// This handler implements the HEAD endpoint for checking blob existence
/// without downloading the full content.
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
#[tracing::instrument(skip(auth))]
pub async fn handle(
    auth: AuthenticatedToken,
    Dep(db): Dep<Postgres>,
    Dep(cas): Dep<Disk>,
    Path(key): Path<Key>,
) -> CasCheckResponse {
    // Check if org has access to this CAS key
    // Return NotFound (not Forbidden) to avoid leaking information about blob
    // existence
    match db.check_cas_access(auth.org_id, &key).await {
        Ok(true) => {}
        Ok(false) => {
            info!("cas.check.no_access");
            return CasCheckResponse::NotFound;
        }
        Err(err) => {
            error!(error = ?err, "cas.check.access_check_error");
            return CasCheckResponse::Error(err);
        }
    }

    // Check if blob exists
    match cas.exists(&key).await {
        Ok(true) => {
            info!("cas.check.found");
            CasCheckResponse::Found
        }
        Ok(false) => {
            info!("cas.check.not_found");
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
    use axum::http::{Method, StatusCode};
    use color_eyre::{Result, eyre::Context};
    use sqlx::PgPool;

    use crate::api::test_helpers::{test_blob, test_server, write_cas};

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn check_exists(pool: PgPool) -> Result<()> {
        const CONTENT: &[u8] = b"check exists test";
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let key = write_cas(&server, CONTENT, auth.token_alice().expose()).await?;

        let response = server
            .method(Method::HEAD, &format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_alice().expose())
            .await;

        response.assert_status_ok();

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn check_doesnt_exist(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, nonexistent_key) = test_blob(b"never written");

        let response = server
            .method(Method::HEAD, &format!("/api/v1/cas/{nonexistent_key}"))
            .authorization_bearer(auth.token_alice().expose())
            .await;

        response.assert_status(StatusCode::NOT_FOUND);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn check_then_write_toctou_safety(pool: PgPool) -> Result<()> {
        const CONTENT: &[u8] = b"toctou test";
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, key) = test_blob(CONTENT);

        // Check before write
        let check1 = server
            .method(axum::http::Method::HEAD, &format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_alice().expose())
            .await;
        check1.assert_status(StatusCode::NOT_FOUND);

        // Write content
        write_cas(&server, CONTENT, auth.token_alice().expose()).await?;

        // Check after write
        let check2 = server
            .method(axum::http::Method::HEAD, &format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_alice().expose())
            .await;
        check2.assert_status_ok();

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn same_org_users_can_check_each_others_blobs(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        // Alice writes a blob
        const ALICE_CONTENT: &[u8] = b"Alice's data for check";
        let alice_key = write_cas(&server, ALICE_CONTENT, auth.token_alice().expose()).await?;

        // Bob (same org) can check Alice's blob exists
        let response = server
            .method(Method::HEAD, &format!("/api/v1/cas/{alice_key}"))
            .authorization_bearer(auth.token_bob().expose())
            .await;

        response.assert_status_ok();

        // Bob writes a blob
        const BOB_CONTENT: &[u8] = b"Bob's data for check";
        let bob_key = write_cas(&server, BOB_CONTENT, auth.token_bob().expose()).await?;

        // Alice can check Bob's blob exists
        let response = server
            .method(Method::HEAD, &format!("/api/v1/cas/{bob_key}"))
            .authorization_bearer(auth.token_alice().expose())
            .await;

        response.assert_status_ok();

        Ok(())
    }
}
