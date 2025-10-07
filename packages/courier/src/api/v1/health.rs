use aerosol::axum::Dep;
use axum::{http::StatusCode, response::IntoResponse};
use color_eyre::{
    Section, SectionExt,
    eyre::{Report, eyre},
};
use tracing::error;

use crate::{db::Postgres, storage::Disk};

/// Health check endpoint.
///
/// ## Validation
///
/// Validates that the database and CAS are accessible before responding.
#[tracing::instrument]
pub async fn handle(Dep(db): Dep<Postgres>, Dep(cas): Dep<Disk>) -> PingResponse {
    let (db, cas) = tokio::join!(db.ping(), cas.ping());
    match (db, cas) {
        (Ok(_), Ok(_)) => PingResponse::Success,
        (Err(db_err), Err(cas_err)) => {
            error!(?db_err, ?cas_err, "ping database and CAS");
            PingResponse::Error(
                eyre!("ping database and CAS")
                    .section(format!("{db_err}").header("Database:"))
                    .section(format!("{cas_err}").header("CAS:")),
            )
        }
        (Err(err), Ok(_)) => {
            error!(?err, "ping database");
            PingResponse::Error(err)
        }
        (Ok(_), Err(err)) => {
            error!(?err, "ping CAS");
            PingResponse::Error(err)
        }
    }
}

#[derive(Debug)]
pub enum PingResponse {
    Success,
    Error(Report),
}

impl IntoResponse for PingResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            PingResponse::Success => StatusCode::OK.into_response(),
            PingResponse::Error(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:?}")).into_response()
            }
        }
    }
}
