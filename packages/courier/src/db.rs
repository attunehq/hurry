use color_eyre::Result;
use sqlx::{PgPool, migrate::Migrator};

pub struct Postgres {
    pool: PgPool,
}

impl Postgres {
    /// The migrator for the database.
    pub const MIGRATOR: Migrator = sqlx::migrate!("./schema/migrations");

    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = PgPool::connect(database_url).await?;
        Ok(Self { pool })
    }

    pub async fn validate_api_key(&self, _org_id: i64, _api_key: &[u8]) -> Result<i64> {
        todo!("Query api_key table to validate and return user_id")
    }

    pub async fn get_org_secret(&self, _org_id: i64) -> Result<Vec<u8>> {
        todo!("Get organization secret for JWT")
    }

    pub async fn store_jwt_session(
        &self,
        _user_id: i64,
        _org_id: i64,
        _expires_at: i64,
    ) -> Result<()> {
        todo!("Store JWT session in database")
    }

    pub async fn revoke_jwt_session(&self, _user_id: i64, _org_id: i64) -> Result<()> {
        todo!("Mark JWT session as revoked")
    }

    pub async fn check_cas_access(&self, _org_id: i64, _cas_key: &[u8]) -> Result<bool> {
        todo!("Check if org has access to CAS key")
    }

    pub async fn grant_cas_access(&self, _org_id: i64, _cas_key: &[u8]) -> Result<()> {
        todo!("Grant org access to CAS key (insert into cas_access)")
    }

    pub async fn load_top_cas_keys(&self, _user_id: i64, _limit: usize) -> Result<Vec<Vec<u8>>> {
        todo!("Query frequency_user_cas_key to get top N most accessed keys")
    }

    pub async fn record_cas_access(&self, _user_id: i64, _cas_key: &[u8]) -> Result<()> {
        todo!("Asynchronously record access to cas key")
    }
}
