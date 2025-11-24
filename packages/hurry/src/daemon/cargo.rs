use std::{collections::HashMap, sync::Arc};

use axum::{
    Router,
    extract::{Json, State},
    routing::{get, post},
};
use color_eyre::Result;
use dashmap::DashMap;
use derive_more::Debug;
use serde::{Deserialize, Serialize};
use tracing::{Instrument, error, info, instrument};
use url::Url;
use uuid::Uuid;

use crate::{
    cargo::{Restored, SaveProgress, UnitPlan, Workspace, save_units},
    cas::CourierCas,
};
use clients::{Courier, Token};

#[derive(Debug, Clone)]
pub struct CargoDaemonState {
    uploads: Arc<DashMap<Uuid, CargoUploadStatus>>,
}

impl Default for CargoDaemonState {
    fn default() -> Self {
        Self {
            uploads: Arc::new(DashMap::new()),
        }
    }
}

pub fn cargo_router() -> Router<CargoDaemonState> {
    Router::new()
        .route("/upload", post(upload))
        .route("/status", post(status))
        .route("/status/all", get(status_all))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CargoUploadRequest {
    pub request_id: Uuid,
    pub courier_url: Url,
    pub courier_token: Token,
    pub ws: Workspace,
    #[debug(skip)]
    pub units: Vec<UnitPlan>,
    pub skip: Restored,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct CargoUploadResponse {
    pub ok: bool,
}

#[instrument]
async fn upload(
    State(state): State<CargoDaemonState>,
    Json(req): Json<CargoUploadRequest>,
) -> Json<CargoUploadResponse> {
    let request_id = req.request_id;
    state.uploads.insert(
        request_id,
        CargoUploadStatus::InProgress(SaveProgress {
            uploaded_units: 0,
            total_units: req.units.len() as u64,
            uploaded_files: 0,
            uploaded_bytes: 0,
        }),
    );
    let span = tracing::info_span!("upload_worker", ?request_id);
    tokio::spawn(
        async move {
            let courier = Courier::new(req.courier_url, req.courier_token)?;
            let cas = CourierCas::new(courier.clone());
            let upload = save_units(&courier, &cas, req.ws, req.units, req.skip, |progress| {
                state
                    .uploads
                    .insert(request_id, CargoUploadStatus::InProgress(progress.clone()));
            })
            .await;
            match upload {
                Ok(()) => {
                    info!(?request_id, "upload completed successfully");
                    state
                        .uploads
                        .insert(request_id, CargoUploadStatus::Complete);
                }
                Err(err) => {
                    error!(?err, ?request_id, "upload failed");
                    state
                        .uploads
                        .insert(request_id, CargoUploadStatus::Complete);
                }
            }
            Result::<_>::Ok(())
        }
        .instrument(span),
    );
    Json(CargoUploadResponse { ok: true })
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum CargoUploadStatus {
    InProgress(SaveProgress),
    Complete,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct CargoUploadStatusRequest {
    pub request_id: Uuid,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct CargoUploadStatusResponse {
    pub status: Option<CargoUploadStatus>,
}

#[instrument]
async fn status(
    State(state): State<CargoDaemonState>,
    Json(req): Json<CargoUploadStatusRequest>,
) -> Json<CargoUploadStatusResponse> {
    let status = state
        .uploads
        .get(&req.request_id)
        .map(|r| r.value().to_owned());
    Json(CargoUploadStatusResponse { status })
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct CargoUploadStatusAllResponse {
    pub statuses: HashMap<Uuid, CargoUploadStatus>,
}

#[instrument]
async fn status_all(State(state): State<CargoDaemonState>) -> Json<CargoUploadStatusAllResponse> {
    let statuses = state
        .uploads
        .iter()
        .map(|entry| (*entry.key(), entry.value().clone()))
        .collect();
    Json(CargoUploadStatusAllResponse { statuses })
}
