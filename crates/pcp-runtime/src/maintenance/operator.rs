use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use pcp_client::{EmbeddedPcpClient, PcpApi};
use pcp_core::{Actor, PackPagesRequest, UnpackPageRequest};
use pcp_store::PcpStore;
use pcp_store::{TombstoneCascadeResult, UnpackPageResult};

use crate::{RuntimeConfig, build_semantic_worker};

use super::{
    AnalyzeMaintenanceArchiveRequest, AnalyzeMaintenancePacksRequest,
    AnalyzeMaintenanceRelationRequest, AnalyzeMaintenanceSummariesRequest,
    AnalyzeMaintenanceSummaryRequest, AnalyzeMaintenanceTopicRequest, ApplyMaintenancePackRequest,
    ApplyMaintenanceRelationRequest, ApplyMaintenanceSummaryRequest, ApplyMaintenanceTopicRequest,
    MaintenanceArchiveAnalysis, MaintenanceArchiveScan, MaintenanceMode, MaintenancePackAnalysis,
    MaintenancePackScan, MaintenanceRelationAnalysis, MaintenanceRelationReviewProposal,
    MaintenanceSummaryAnalysis, MaintenanceSummaryBatchAnalysis, MaintenanceTopicAnalysis,
    MaintenanceWorkScan, RuntimeMaintainer,
};

pub struct MaintenanceOperator {
    identity_id: String,
    maintainer: RuntimeMaintainer,
    repair_client: Arc<dyn PcpApi>,
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
        let repair_client = EmbeddedPcpClient::shared(
            Arc::clone(&store) as Arc<dyn PcpStore>,
            maintenance.repair_access_session(&identity_id),
        );
        let worker = build_semantic_worker(&maintenance.worker)?;
        let maintainer = RuntimeMaintainer::load_with_usage_source(
            client,
            worker,
            maintenance,
            "manual_maintenance",
        )
        .await?;
        Ok(Self {
            identity_id,
            maintainer,
            repair_client,
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

    pub async fn scan_archive_candidates(&self) -> Result<MaintenanceArchiveScan> {
        self.maintainer.scan_archive_candidates().await
    }

    pub async fn analyze_archive(
        &self,
        request: AnalyzeMaintenanceArchiveRequest,
    ) -> Result<MaintenanceArchiveAnalysis> {
        self.maintainer.analyze_archive_candidate(request).await
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

    pub async fn unpack_page(&self, request: UnpackPageRequest) -> Result<UnpackPageResult> {
        self.repair_client.unpack_page(request).await
    }

    pub async fn pack_pages(&self, request: PackPagesRequest) -> Result<pcp_core::WriteResult> {
        self.repair_client.pack_pages(request).await
    }

    pub async fn retire_page(
        &self,
        root_revision_id: String,
        actor: Actor,
    ) -> Result<TombstoneCascadeResult> {
        self.repair_client
            .tombstone_derivation_cascade(root_revision_id, actor)
            .await
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

    pub async fn analyze_topic(
        &self,
        request: AnalyzeMaintenanceTopicRequest,
    ) -> Result<MaintenanceTopicAnalysis> {
        self.maintainer.analyze_topic_candidate(request).await
    }

    pub async fn apply_topic(
        &self,
        request: ApplyMaintenanceTopicRequest,
    ) -> Result<pcp_core::WriteResult> {
        self.maintainer.apply_topic_candidate(request).await
    }

    pub fn pending_relation_reviews(&self) -> Vec<MaintenanceRelationReviewProposal> {
        self.maintainer.pending_relation_reviews()
    }

    pub async fn approve_relation_review(
        &mut self,
        candidate_id: &str,
    ) -> Result<pcp_core::Relation> {
        self.maintainer.approve_relation_review(candidate_id).await
    }

    pub async fn reject_relation_review(
        &mut self,
        candidate_id: &str,
        suppress: bool,
    ) -> Result<()> {
        self.maintainer
            .reject_relation_review(candidate_id, suppress)
            .await
    }
}
