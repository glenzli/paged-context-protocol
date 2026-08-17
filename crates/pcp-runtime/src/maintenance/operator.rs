use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use pcp_client::EmbeddedPcpClient;
use pcp_store::PcpStore;

use crate::{RuntimeConfig, build_semantic_worker};

use super::{
    AnalyzeMaintenancePacksRequest, AnalyzeMaintenanceRelationRequest,
    AnalyzeMaintenanceSummariesRequest, AnalyzeMaintenanceSummaryRequest,
    ApplyMaintenancePackRequest, ApplyMaintenanceRelationRequest, ApplyMaintenanceSummaryRequest,
    MaintenanceMode, MaintenancePackAnalysis, MaintenancePackScan, MaintenanceRelationAnalysis,
    MaintenanceSummaryAnalysis, MaintenanceSummaryBatchAnalysis, MaintenanceWorkScan,
    RuntimeMaintainer,
};

pub struct MaintenanceOperator {
    identity_id: String,
    maintainer: RuntimeMaintainer,
}

impl MaintenanceOperator {
    pub async fn load(config_path: impl AsRef<Path>) -> Result<Self> {
        let mut runtime = RuntimeConfig::load(config_path)?;
        let mut maintenance = runtime
            .maintenance
            .take()
            .context("PCP runtime config has no maintenance section")?;
        maintenance.enabled = true;
        maintenance.mode = MaintenanceMode::Apply;
        maintenance.packing.enabled = true;
        maintenance.summary.enabled = true;
        maintenance.relation.enabled = true;
        maintenance.validate()?;

        let store = Arc::new(
            pcp_sqlite::SqlitePcpStore::open(runtime.store_path.clone())
                .await
                .with_context(|| format!("open PCP Store {}", runtime.store_path.display()))?,
        );
        let identity_id = store.identity_id().to_owned();
        let client = EmbeddedPcpClient::shared(
            Arc::clone(&store) as Arc<dyn PcpStore>,
            maintenance.access_session(&identity_id),
        );
        let worker = build_semantic_worker(&maintenance.worker)?;
        let maintainer = RuntimeMaintainer::load(client, worker, maintenance).await?;
        Ok(Self {
            identity_id,
            maintainer,
        })
    }

    pub fn identity_id(&self) -> &str {
        &self.identity_id
    }

    pub async fn scan_packing(&self) -> Result<MaintenancePackScan> {
        self.maintainer.scan_packing_candidates().await
    }

    pub async fn scan_maintenance_work(&self) -> Result<MaintenanceWorkScan> {
        self.maintainer.scan_maintenance_work().await
    }

    pub async fn analyze_packing(
        &self,
        request: AnalyzeMaintenancePacksRequest,
    ) -> Result<MaintenancePackAnalysis> {
        self.maintainer.analyze_packing_candidates(request).await
    }

    pub async fn apply_pack(
        &self,
        request: ApplyMaintenancePackRequest,
    ) -> Result<pcp_core::WriteResult> {
        self.maintainer.apply_pack_candidate(request).await
    }

    pub async fn analyze_summary(
        &self,
        request: AnalyzeMaintenanceSummaryRequest,
    ) -> Result<MaintenanceSummaryAnalysis> {
        self.maintainer.analyze_summary_candidate(request).await
    }

    pub async fn analyze_summaries(
        &self,
        request: AnalyzeMaintenanceSummariesRequest,
    ) -> Result<MaintenanceSummaryBatchAnalysis> {
        self.maintainer.analyze_summary_candidates(request).await
    }

    pub async fn apply_summary(
        &self,
        request: ApplyMaintenanceSummaryRequest,
    ) -> Result<pcp_core::WriteSummaryResult> {
        self.maintainer.apply_summary_candidate(request).await
    }

    pub async fn analyze_relation(
        &self,
        request: AnalyzeMaintenanceRelationRequest,
    ) -> Result<MaintenanceRelationAnalysis> {
        self.maintainer.analyze_relation_candidate(request).await
    }

    pub async fn apply_relation(
        &self,
        request: ApplyMaintenanceRelationRequest,
    ) -> Result<pcp_core::Relation> {
        self.maintainer.apply_relation_candidate(request).await
    }
}
