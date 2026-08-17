mod audit;
mod config;
mod coordinator;
mod infer_worker;
mod ledger;
mod operator;
mod worker;

use std::{sync::Arc, time::Duration};

use anyhow::Result;

pub use audit::{MaintenanceRunAudit, MaintenanceRunAuditRecord, persist_audit};
pub use config::{
    MaintenanceConfig, MaintenanceMode, MaintenanceWorkerConfig, PackingMaintenanceConfig,
    RelationMaintenanceConfig, RetentionMaintenanceConfig, SummaryMaintenanceConfig,
};
pub use coordinator::{
    AnalyzeMaintenancePacksRequest, AnalyzeMaintenanceRelationRequest,
    AnalyzeMaintenanceSummariesRequest, AnalyzeMaintenanceSummaryRequest,
    ApplyMaintenancePackRequest, ApplyMaintenanceRelationRequest, ApplyMaintenanceSummaryRequest,
    MaintenanceCycleReport, MaintenancePackAnalysis, MaintenancePackAnalysisIssue,
    MaintenancePackCandidate, MaintenancePackInput, MaintenancePackScan, MaintenancePackScanGroup,
    MaintenanceRelationAnalysis, MaintenanceRelationCandidate, MaintenanceRelationInput,
    MaintenanceRelationScan, MaintenanceRelationScanGroup, MaintenanceReviewDecision,
    MaintenanceSummaryAnalysis, MaintenanceSummaryAnalysisIssue, MaintenanceSummaryBatchAnalysis,
    MaintenanceSummaryCandidate, MaintenanceSummaryScan, MaintenanceSummaryScanPage,
    MaintenanceWorkScan, RuntimeMaintainer,
};
pub use infer_worker::InferRuntimeSemanticWorker;
pub use operator::MaintenanceOperator;
pub use worker::{
    CommandSemanticWorker, MaintenanceDetailPage, MaintenanceRelation, MaintenanceRoutingPage,
    MaintenanceSummarySelection, MaintenanceWorkerRequest, MaintenanceWorkerResponse,
    PackingCandidateGroup, RelationCandidatePage, RetentionMilestone, SemanticMaintenanceWorker,
};

pub fn build_semantic_worker(
    config: &MaintenanceWorkerConfig,
) -> Result<Arc<dyn SemanticMaintenanceWorker>> {
    match config {
        MaintenanceWorkerConfig::Command {
            program,
            args,
            timeout_seconds,
            ..
        } => Ok(Arc::new(CommandSemanticWorker::new(
            program.clone(),
            args.clone(),
            Duration::from_secs(*timeout_seconds),
        ))),
        MaintenanceWorkerConfig::InferRuntime {
            credential_file,
            timeout_seconds,
            summary_deployment_id,
            reasoning_deployment_id,
            relation_deployment_id,
            ..
        } => Ok(Arc::new(InferRuntimeSemanticWorker::new(
            credential_file.clone(),
            Duration::from_secs(*timeout_seconds),
            summary_deployment_id.clone(),
            reasoning_deployment_id.clone(),
            relation_deployment_id.clone(),
        )?)),
    }
}

#[cfg(test)]
mod tests;
