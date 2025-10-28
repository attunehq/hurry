use color_eyre::{Result, eyre::Context as _};
use hurry::fs::user_global_cache_path;
use tracing::instrument;

#[instrument]
pub async fn exec() -> Result<()> {
    let cache_path = user_global_cache_path()
        .await
        .context("get user global cache path")?;
    println!("{cache_path}");
    Ok(())
}
