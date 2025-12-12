//! Global test helpers for courier integration tests.
//!
//! This module provides shared test infrastructure for spawning test servers,
//! managing authentication, and creating test fixtures.

use std::collections::HashMap;

use aerosol::Aero;
use async_tempfile::TempDir;
use clients::{
    Token,
    courier::v1::{
        Client, Fingerprint, GlibcVersion, Key, LibraryCrateUnitPlan, LibraryFiles, SavedUnit,
        SavedUnitHash, UnitPlanInfo,
        cache::{CargoSaveRequest, CargoSaveUnitRequest},
    },
};
use color_eyre::{Result, eyre::Context};
use courier::{
    api,
    auth::{AccountId, OrgId, RawToken},
    db, storage,
};
use futures::{StreamExt, TryStreamExt, stream};
use sqlx::PgPool;
use url::Url;

const GLIBC_VERSION: GlibcVersion = GlibcVersion {
    major: 2,
    minor: 41,
    patch: 0,
};

/// Test fixture containing a spawned server and authentication context.
pub struct TestFixture {
    /// Base URL of the server.
    pub base_url: Url,

    /// The Courier v1 client for the Alice user (Acme Corp).
    pub client_alice: Client,

    /// The Courier v1 client for the Bob user (Acme Corp).
    pub client_bob: Client,

    /// The Courier v1 client for the Charlie user (Widget Inc).
    pub client_charlie: Client,

    /// Raw auth tokens for all users.
    pub auth: TestAuth,

    /// Database connection for direct queries in tests.
    pub db: db::Postgres,

    /// Temporary directory that will be cleaned up after the test.
    pub _temp: TempDir,
}

impl TestFixture {
    /// Spawn a new test server with isolated database and storage.
    ///
    /// The database pool should come from the `#[sqlx::test]` macro, which
    /// provides an isolated database for each test.
    pub async fn spawn(pool: PgPool) -> Result<Self> {
        let db = db::Postgres { pool };
        let auth = TestAuth::seed(&db).await?;
        let (storage, _temp) = storage::Disk::new_temp()
            .await
            .context("create temp storage")?;
        let state = Aero::new().with(storage).with(db.clone());
        let router = api::router(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .context("bind test server")?;
        let local_addr = listener.local_addr().context("get local addr")?;
        let base_url = Url::parse(&format!("http://{local_addr}")).context("parse base URL")?;

        // TODO: This leaves the server running after the test, which isn't the
        // end of the world (it's shut down when the process ends) but isn't
        // ideal.
        tokio::task::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("test server failed");
        });

        let client_alice = Client::new(base_url.clone(), auth.token_alice().expose().into())?;
        let client_bob = Client::new(base_url.clone(), auth.token_bob().expose().into())?;
        let client_charlie = Client::new(base_url.clone(), auth.token_charlie().expose().into())?;

        Ok(Self {
            base_url,
            client_alice,
            client_bob,
            client_charlie,
            auth,
            db,
            _temp,
        })
    }

    /// Create a client with a specific token (useful for testing invalid
    /// tokens).
    pub fn client_with_token(&self, token: impl Into<Token>) -> Result<Client> {
        Client::new(self.base_url.clone(), token.into())
    }
}

/// Fixture authentication information.
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

    pub fn org_acme(&self) -> OrgId {
        self.org_ids
            .get(Self::ORG_ACME)
            .copied()
            .expect("Acme org missing")
    }

    pub fn org_widget(&self) -> OrgId {
        self.org_ids
            .get(Self::ORG_WIDGET)
            .copied()
            .expect("Widget org missing")
    }

    pub fn token_alice(&self) -> &RawToken {
        self.tokens
            .get(Self::ACCT_ALICE)
            .expect("Alice token missing")
    }

    pub fn token_bob(&self) -> &RawToken {
        self.tokens.get(Self::ACCT_BOB).expect("Bob token missing")
    }

    pub fn token_charlie(&self) -> &RawToken {
        self.tokens
            .get(Self::ACCT_CHARLIE)
            .expect("Charlie token missing")
    }

    pub fn token_alice_revoked(&self) -> &RawToken {
        self.revoked_tokens
            .get(Self::ACCT_ALICE)
            .expect("Alice revoked token missing")
    }

    pub fn token_bob_revoked(&self) -> &RawToken {
        self.revoked_tokens
            .get(Self::ACCT_BOB)
            .expect("Bob revoked token missing")
    }

    pub fn token_charlie_revoked(&self) -> &RawToken {
        self.revoked_tokens
            .get(Self::ACCT_CHARLIE)
            .expect("Charlie revoked token missing")
    }

    /// Seed the database with test authentication data.
    pub async fn seed(db: &db::Postgres) -> Result<Self> {
        let org_ids = sqlx::query!(
            r#"
            INSERT INTO organization (name, created_at) VALUES
                ($1, now()),
                ($2, now())
            RETURNING id, name
            "#,
            Self::ORG_ACME,
            Self::ORG_WIDGET
        )
        .fetch_all(&db.pool)
        .await
        .context("insert organizations")?
        .into_iter()
        .map(|row| (row.name, OrgId::from_i64(row.id)))
        .collect::<HashMap<_, _>>();

        let account_ids = sqlx::query!(
            r#"
            INSERT INTO account (organization_id, email, created_at) VALUES
                (2, $1, now()),
                (2, $2, now()),
                (3, $3, now())
            RETURNING id, email
            "#,
            Self::ACCT_ALICE,
            Self::ACCT_BOB,
            Self::ACCT_CHARLIE
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
                    .create_token(account_id, &format!("{account}-token"))
                    .await
                    .with_context(|| format!("set up {account}"))?;
                Result::<_>::Ok((account.to_string(), hash))
            })
            .try_collect::<HashMap<_, _>>()
            .await?;

        let revoked_tokens = stream::iter(&account_ids)
            .then(|(account, &account_id)| async move {
                let hash = db
                    .create_token(account_id, &format!("{account}-revoked"))
                    .await
                    .with_context(|| format!("set up {account}"))?;
                db.revoke_token(&hash)
                    .await
                    .with_context(|| format!("revoke token for {account}"))?;
                Result::<_>::Ok((account.to_string(), hash))
            })
            .try_collect::<HashMap<_, _>>()
            .await?;

        sqlx::query!("SELECT setval('organization_id_seq', (SELECT MAX(id) FROM organization))")
            .fetch_one(&db.pool)
            .await
            .context("reset organization sequence")?;

        sqlx::query!("SELECT setval('account_id_seq', (SELECT MAX(id) FROM account))")
            .fetch_one(&db.pool)
            .await
            .context("reset account sequence")?;

        Ok(Self {
            org_ids,
            account_ids,
            tokens,
            revoked_tokens,
        })
    }
}

/// Compute the key of a blob of test content.
pub fn test_blob(content: &[u8]) -> Key {
    Key::from_buffer(content)
}

/// Create a test SavedUnit for cargo cache tests with the given unit hash.
pub fn test_saved_unit(unit_hash: impl Into<SavedUnitHash>) -> SavedUnit {
    let unit_hash = unit_hash.into();
    let info = UnitPlanInfo::builder()
        .unit_hash(&unit_hash)
        .package_name("test-package")
        .crate_name("test_crate")
        .maybe_target_arch(Some("x86_64-unknown-linux-gnu"))
        .build();

    let files = LibraryFiles::builder()
        .output_files(vec![])
        .fingerprint(Fingerprint::from("test-fingerprint"))
        .dep_info_file(test_blob(b"dep-info"))
        .encoded_dep_info_file(test_blob(b"encoded-dep-info"))
        .build();

    let plan = LibraryCrateUnitPlan::builder()
        .info(info)
        .src_path("test.rs")
        .outputs(vec![])
        .build();

    SavedUnit::LibraryCrate(files, plan)
}

/// Create a cargo save request from a unit with the given unit hash.
pub fn test_cargo_save_request(
    unit_hash: impl Into<SavedUnitHash>,
) -> (CargoSaveRequest, SavedUnitHash) {
    let unit_hash = unit_hash.into();
    let unit = test_saved_unit(&unit_hash);
    let key = unit.unit_hash().clone();
    let request = CargoSaveUnitRequest::builder()
        .unit(unit)
        .resolved_target(String::from("x86_64-unknown-linux-gnu"))
        .maybe_linux_glibc_version(Some(GLIBC_VERSION))
        .build();
    let save_request = CargoSaveRequest::new([request]);
    (save_request, key)
}
