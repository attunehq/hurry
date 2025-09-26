use std::str::FromStr as _;

use color_eyre::{Result, eyre::Context as _};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tap::Tap;
use tracing::{instrument, trace};

use crate::{
    fs, mk_rel_dir, mk_rel_file,
    path::{AbsDirPath, JoinWith as _},
};

#[derive(Debug, Clone)]
pub struct CargoCache {
    db: SqlitePool,
}

impl CargoCache {
    #[instrument(name = "CargoCache::open")]
    async fn open(conn: &str) -> Result<Self> {
        let options = SqliteConnectOptions::from_str(conn)
            .context("parse sqlite connection string")?
            .create_if_missing(true);
        let db = SqlitePoolOptions::new()
            .connect_with(options)
            .await
            .context("connecting to cargo cache database")?;
        sqlx::migrate!("src/cargo/cache/db/migrations")
            .run(&db)
            .await
            .context("running migrations")?;
        Ok(Self { db })
    }

    #[instrument(name = "CargoCache::open_dir")]
    pub async fn open_dir(dir: &AbsDirPath) -> Result<Self> {
        let dbfile = dir.join(mk_rel_file!("cache.db"));
        if !fs::exists(dbfile.as_std_path()).await {
            fs::create_dir_all(dir)
                .await
                .context("create cache directory")?;
        }

        Self::open(&format!("sqlite://{}", dbfile)).await
    }

    #[instrument(name = "CargoCache::open_default")]
    pub async fn open_default() -> Result<Self> {
        let cache = fs::user_global_cache_path()
            .await
            .context("finding user cache path")?
            .join(mk_rel_dir!("cargo"));
        Self::open_dir(&cache).await
    }

    #[instrument(name = "CargoCache::get")]
    pub async fn get(&self, key: &str) -> Result<()> {
        todo!()
        // sqlx::query!("SELECT * FROM cache WHERE key = ?")
        //     .bind(key)
        //     .fetch_one(&self.db)
        //     .await
        //     .context("get cache entry")
    }
}
