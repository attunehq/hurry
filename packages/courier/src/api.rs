//! API endpoint handlers for the service.
//!
//! ## Dependency injection
//!
//! We use [`aerosol`][^1] to manage dependencies and inject them into handlers.
//! Reference [`State`] for the list of dependencies; note that when providing
//! dependencies that are in this required list you need to provide them in
//! reverse order of the list.
//!
//! Items that are in the list can be extracted in handlers using the
//! [`Dep`](aerosol::axum::Dep) extractor.
//!
//! [^1]: https://docs.rs/aerosol
//!
//! ## Response types
//!
//! Most handlers return a response type that implements
//! [`IntoResponse`](axum::response::IntoResponse)[^2]. This is a trait that
//! allows handlers to return a response without having to manually implement
//! the response type.
//!
//! We do it this way instead of just returning a more generic response type
//! because it supports better documentation and makes it easier to realize if
//! you're writing backwards-incompatible changes to the API.
//!
//! For documentation, we can in the future add `utoipa` and then use it to
//! annotate the response type with documentation which is then automatically
//! rendered for the user in the OpenAPI spec.
//!
//! [^2]: https://docs.rs/axum/latest/axum/response/trait.IntoResponse.html

use std::time::{Duration, Instant};

use aerosol::Aero;
use axum::{
    Router, extract::DefaultBodyLimit, extract::Request, http::HeaderValue, middleware::Next,
    response::Response,
};
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer, decompression::RequestDecompressionLayer,
    limit::RequestBodyLimitLayer, timeout::TimeoutLayer,
};
use tracing::Instrument;
use uuid::Uuid;

pub mod v1;

/// Request timeout is set to accommodate bulk operations transferring large
/// amounts of data. 30 minutes allows for 10GB transfers over slower
/// connections (~50 Mbps) while still protecting against indefinitely hanging
/// connections.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(1800);

/// Body size limit for streaming operations (CAS uploads). Single artifacts
/// can be large (e.g., libaws_sdk_s3 produces 125MB rlibs) and bulk operations
/// may transfer many artifacts in one request.
const MAX_BODY_SIZE: usize = 10 * 1024 * 1024 * 1024; // 10GB

/// Body size limit for JSON deserialization. Set to accommodate bulk metadata
/// operations like bulk restore requests.
const MAX_JSON_BODY_SIZE: usize = 100 * 1024 * 1024; // 100MB

pub type State = Aero![crate::db::Postgres, crate::storage::Disk,];

pub fn router(state: State) -> Router {
    let middleware = ServiceBuilder::new()
        .layer(RequestDecompressionLayer::new())
        .layer(CompressionLayer::new())
        .layer(RequestBodyLimitLayer::new(MAX_BODY_SIZE))
        .layer(TimeoutLayer::new(REQUEST_TIMEOUT));

    Router::new()
        .nest("/api/v1", v1::router())
        .layer(DefaultBodyLimit::max(MAX_JSON_BODY_SIZE))
        .layer(middleware)
        .layer(axum::middleware::from_fn(trace_request))
        .with_state(state)
}

async fn trace_request(request: Request, next: Next) -> Response {
    const REQUEST_ID_HEADER: &str = "x-request-id";
    let id = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|id| id.to_str().map(|id| id.to_string()).ok())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let start = Instant::now();
    let url = request.uri().to_string();
    let method = request.method().to_string();

    let span = tracing::info_span!("http.request", %id, %url, %method);
    async move {
        let mut response = next.run(request).await;
        let status = response.status();
        let duration = start.elapsed();
        tracing::info!(%id, %url, %method, %status, ?duration, "http.request.response");

        if let Ok(id) = HeaderValue::from_str(&id) {
            response.headers_mut().insert(REQUEST_ID_HEADER, id);
        }
        response
    }
    .instrument(span)
    .await
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "Helpers have lots of stuff we might not use right now"
)]
pub(crate) mod test_helpers {
    use std::collections::HashMap;

    use aerosol::Aero;
    use async_tempfile::TempDir;
    use axum::body::Bytes;
    use axum::http::StatusCode;
    use axum_test::{TestServer, TestServerConfig};
    use clients::courier::v1::Key;
    use color_eyre::{
        Result,
        eyre::{Context, eyre},
    };
    use duplicate::duplicate_item;
    use futures::{StreamExt, TryStreamExt, stream};
    use sqlx::PgPool;

    use crate::auth::{AccountId, OrgId, RawToken};
    use crate::db::Postgres;
    use crate::{api, db, storage};

    /// Create an isolated test server with the given database pool:
    /// - The database pool is intended to come from the [`sqlx::test`] macro
    /// - Creates a new [`Disk`](crate::storage::Disk) instance in a temp
    ///   directory
    /// - Seeds test authentication data and returns the plaintext tokens
    pub async fn test_server(pool: PgPool) -> Result<(TestServer, TestAuth, TempDir)> {
        let db = db::Postgres { pool };
        let auth = TestAuth::seed(&db).await?;
        let (storage, temp) = storage::Disk::new_temp()
            .await
            .context("create temp storage")?;
        let state = Aero::new().with(storage).with(db);
        let router = api::router(state);
        let server = TestServerConfig::default()
            .build(router)
            .map_err(|e| eyre!("create test server: {e}"))?;
        Ok((server, auth, temp))
    }

    /// Fixture authentication information.
    /// Use the associated constants for this type to select supported data.
    #[derive(Debug, Clone)]
    pub struct TestAuth {
        pub org_ids: HashMap<String, OrgId>,
        pub account_ids: HashMap<String, AccountId>,
        pub tokens: HashMap<String, RawToken>,
        pub revoked_tokens: HashMap<String, RawToken>,
    }

    impl TestAuth {
        pub const ORG_ACME: &str = "Acme Corp";
        pub const ORG_WIDGET: &str = "Widget Inc";

        pub const ACCT_ALICE: &str = "alice@acme.com";
        pub const ACCT_BOB: &str = "bob@acme.com";
        pub const ACCT_CHARLIE: &str = "charlie@widget.com";

        #[duplicate_item(
            method constant;
            [ org_acme ] [ ORG_ACME ];
            [ org_widget ] [ ORG_WIDGET ];
        )]
        #[track_caller]
        pub fn method(&self) -> OrgId {
            self.org_ids
                .get(Self::constant)
                .copied()
                .unwrap_or_else(|| panic!("unknown org: {}", Self::constant))
        }

        #[duplicate_item(
            method constant;
            [ token_alice ] [ ACCT_ALICE ];
            [ token_bob ] [ ACCT_BOB ];
            [ token_charlie ] [ ACCT_CHARLIE ];
        )]
        #[track_caller]
        pub fn method(&self) -> &RawToken {
            self.tokens
                .get(Self::constant)
                .unwrap_or_else(|| panic!("unknown account: {}", Self::constant))
        }

        #[duplicate_item(
            method constant;
            [ token_alice_revoked ] [ ACCT_ALICE ];
            [ token_bob_revoked ] [ ACCT_BOB ];
            [ token_charlie_revoked ] [ ACCT_CHARLIE ];
        )]
        #[track_caller]
        pub fn method(&self) -> &RawToken {
            self.revoked_tokens
                .get(Self::constant)
                .unwrap_or_else(|| panic!("unknown account: {}", Self::constant))
        }

        /// Seed the database with test authentication data, then return it.
        pub async fn seed(db: &Postgres) -> Result<TestAuth> {
            // Insert organizations; skip ID 1 as that's the "default" org.
            let org_ids = sqlx::query!(
                r#"
                INSERT INTO organization (name, created_at) VALUES
                    ($1, now()),
                    ($2, now())
                RETURNING id, name
                "#,
                TestAuth::ORG_ACME,
                TestAuth::ORG_WIDGET
            )
            .fetch_all(&db.pool)
            .await
            .context("insert organizations")?
            .into_iter()
            .map(|row| (row.name, OrgId::from_i64(row.id)))
            .collect::<HashMap<_, _>>();

            // Insert accounts
            let account_ids = sqlx::query!(
                r#"
                INSERT INTO account (organization_id, email, created_at) VALUES
                    (2, $1, now()),
                    (2, $2, now()),
                    (3, $3, now())
                RETURNING id, email
                "#,
                TestAuth::ACCT_ALICE,
                TestAuth::ACCT_BOB,
                TestAuth::ACCT_CHARLIE
            )
            .fetch_all(&db.pool)
            .await
            .context("insert accounts")?
            .into_iter()
            .map(|row| (row.email, AccountId::from_i64(row.id)))
            .collect::<HashMap<_, _>>();

            let tokens = stream::iter(&account_ids)
                .then(|(account, &account_id)| async move {
                    let hash = db
                        .create_token(account_id)
                        .await
                        .with_context(|| format!("set up {account}"))?;
                    Result::<_>::Ok((account.to_string(), hash))
                })
                .try_collect::<HashMap<_, _>>()
                .await?;
            let revoked_tokens = stream::iter(&account_ids)
                .then(|(account, &account_id)| async move {
                    let hash = db
                        .create_token(account_id)
                        .await
                        .with_context(|| format!("set up {account}"))?;
                    db.revoke_token(&hash)
                        .await
                        .with_context(|| format!("revoke token for {account}"))?;
                    Result::<_>::Ok((account.to_string(), hash))
                })
                .try_collect::<HashMap<_, _>>()
                .await?;

            // Reset sequences to avoid conflicts
            sqlx::query!(
                "SELECT setval('organization_id_seq', (SELECT MAX(id) FROM organization))"
            )
            .fetch_one(&db.pool)
            .await
            .context("reset organization sequence")?;
            sqlx::query!("SELECT setval('account_id_seq', (SELECT MAX(id) FROM account))")
                .fetch_one(&db.pool)
                .await
                .context("reset account sequence")?;

            Ok(TestAuth {
                org_ids,
                account_ids,
                tokens,
                revoked_tokens,
            })
        }
    }

    /// Generate test content and compute its key.
    pub fn test_blob(content: &[u8]) -> (Vec<u8>, Key) {
        let hash = blake3::hash(content);
        (content.to_vec(), Key::from_blake3(hash))
    }

    /// Write a blob to CAS and return the key.
    /// Requires an authentication token to be explicitly provided.
    pub async fn write_cas(server: &TestServer, content: &[u8], token: &str) -> Result<Key> {
        let (_, key) = test_blob(content);

        let response = server
            .put(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(token)
            .bytes(Bytes::copy_from_slice(content))
            .await;

        response.assert_status(StatusCode::CREATED);
        Ok(key)
    }
}
