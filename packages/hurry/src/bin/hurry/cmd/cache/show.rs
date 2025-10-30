use color_eyre::{Result, eyre::WrapErr};
use tracing::instrument;

use hurry::fs::user_global_cache_path;

#[instrument]
pub async fn exec() -> Result<()> {
    let cache_path = user_global_cache_path()
        .await
        .context("get user global cache path")?;
    println!("{cache_path}");
    Ok(())
}
