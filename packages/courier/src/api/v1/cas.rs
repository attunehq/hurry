use axum::{
    Router,
    routing::{get, head, put},
};
use color_eyre::{Result, eyre::Context};
use tracing::info;

use crate::{
    api::State,
    auth::{KeySets, StatelessToken},
    db::Postgres,
    storage::Key,
};

pub mod check;
pub mod read;
pub mod write;

pub fn router() -> Router<State> {
    Router::new()
        .route("/{key}", head(check::handle))
        .route("/{key}", get(read::handle))
        .route("/{key}", put(write::handle))
}

/// Check if the given key is allowed for the given token.
///
/// If the key is visible in `keysets` then we can grant access immediately.
/// Otherwise, we need to check if the account has access to the key in the
/// database. If the account has access to the key according to the database, then
/// we add the key to `keysets` and grant access. Otherwise, we return `false`.
#[tracing::instrument]
async fn check_allowed(
    keysets: &KeySets,
    db: &Postgres,
    key: &Key,
    token: &StatelessToken,
) -> Result<bool> {
    let allowed = keysets.organization(token.org_id);
    if !allowed.contains(key) {
        let access = db
            .account_has_cas_key(token.account_id, key)
            .await
            .context("check account has cas key")?;
        if access {
            info!("cas.auth.cache_miss");
            allowed.insert(key.clone());
        } else {
            info!("cas.auth.unauthorized");
            return Ok(false);
        }
    }
    Ok(true)
}
