use aerosol::axum::Dep;
use axum::{
    Json, Router,
    routing::{delete, post},
};
use color_eyre::eyre::Context;
use serde::Serialize;

use crate::{
    api::State,
    auth::{OrgId, RawToken},
    db::Postgres,
};

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize)]
pub struct MintJwtResponse {
    jwt: String,
}

pub fn router() -> Router<State> {
    Router::new()
        .route("/", post(mint_jwt))
        .route("/", delete(revoke_jwt))
}

async fn mint_jwt(token: RawToken, org_id: OrgId, Dep(db): Dep<Postgres>) -> Json<MintJwtResponse> {
    let _token = db
        .validate(org_id.into(), token)
        .await
        .context("validate token");

    todo!()
}

async fn revoke_jwt() -> &'static str {
    todo!()
}
