use axum::{
    Router,
    routing::{get, head, put},
};
use color_eyre::{Result, eyre::Context};

use crate::{
    api::State,
    auth::{AuthenticatedStatelessToken, KeySets},
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
/// Otherwise, we need to check if the user has access to the key in the
/// database. If the user has access to the key according to the database, then
/// we add the key to `keysets` and grant access. Otherwise, we return `false`.
async fn check_allowed(
    keysets: &KeySets,
    db: &Postgres,
    key: &Key,
    token: &AuthenticatedStatelessToken,
) -> Result<bool> {
    let allowed = keysets.organization(token.org_id);
    if !allowed.contains(&key) {
        let access = db
            .user_has_cas_key(token.user_id, &key)
            .await
            .context("check user has cas key")?;
        if access {
            allowed.insert(key.clone());
        } else {
            return Ok(false);
        }
    }
    Ok(true)
}
