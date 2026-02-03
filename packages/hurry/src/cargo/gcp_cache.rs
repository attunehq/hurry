//! GCP-based cargo cache implementation.
//!
//! This module provides a serverless cache implementation that stores both
//! unit metadata and file contents directly in a GCP Cloud Storage bucket,
//! bypassing the need for a Courier server.

use color_eyre::{Result, eyre::Context as _};
use derive_more::Debug;
use tracing::instrument;

use crate::{
    cargo::{
        UnitPlan, Workspace,
        cache::Restored,
    },
    gcp_cas::GcpCas,
    progress::TransferBar,
};

mod gcp_restore;
mod gcp_save;

pub use gcp_restore::restore_units_gcp;
pub use gcp_save::save_units_gcp;

/// GCP-based cargo cache.
#[derive(Debug, Clone)]
pub struct GcpCargoCache {
    cas: GcpCas,
    ws: Workspace,
}

impl GcpCargoCache {
    /// Open a GCP-based cargo cache.
    #[instrument(name = "GcpCargoCache::open")]
    pub async fn open(bucket: String, ws: Workspace) -> Result<Self> {
        let cas = GcpCas::new(bucket);
        cas.ping().await.context("ping GCS bucket")?;
        Ok(Self { cas, ws })
    }

    /// Get a reference to the CAS.
    pub fn cas(&self) -> &GcpCas {
        &self.cas
    }

    /// Get a reference to the workspace.
    pub fn workspace(&self) -> &Workspace {
        &self.ws
    }

    /// Restore artifacts from GCS cache.
    #[instrument(name = "GcpCargoCache::restore", skip_all)]
    pub async fn restore(&self, units: &Vec<UnitPlan>, progress: &TransferBar) -> Result<Restored> {
        restore_units_gcp(&self.cas, &self.ws, units, progress).await
    }

    /// Save artifacts to GCS cache.
    #[instrument(name = "GcpCargoCache::save", skip_all)]
    pub async fn save(
        &self,
        units: Vec<UnitPlan>,
        restored: Restored,
        progress: &TransferBar,
    ) -> Result<()> {
        let mut last_uploaded_units = 0u64;
        let mut last_uploaded_files = 0u64;
        let mut last_uploaded_bytes = 0u64;
        save_units_gcp(&self.cas, self.ws.clone(), units, restored, |p| {
            progress.inc(p.uploaded_units.saturating_sub(last_uploaded_units));
            last_uploaded_units = p.uploaded_units;
            progress.add_files(p.uploaded_files.saturating_sub(last_uploaded_files));
            last_uploaded_files = p.uploaded_files;
            progress.add_bytes(p.uploaded_bytes.saturating_sub(last_uploaded_bytes));
            last_uploaded_bytes = p.uploaded_bytes;
        })
        .await
    }
}
