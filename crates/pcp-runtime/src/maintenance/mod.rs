mod audit;
mod candidate_review;
mod config;
mod coordinator;
mod discovery;
mod infer_worker;
mod ledger;
mod operator;
mod reconciliation;
mod review;
mod review_budget;
mod review_escalation;
mod topic_policy;
mod update_discovery;
mod worker;

use std::{sync::Arc, time::Duration};

use anyhow::Result;

pub use audit::{MaintenanceRunAudit, MaintenanceRunAuditRecord, persist_audit};
pub use config::{
    MaintenanceConfig, MaintenanceMode, MaintenanceWorkerConfig, PackingMaintenanceConfig,
    PeriodicReviewConfig, ReconciliationMaintenanceConfig, RelationMaintenanceConfig,
    RetentionMaintenanceConfig, SummaryMaintenanceConfig, TopicMaintenanceConfig,
    WriteTriggeredMaintenanceConfig,
};
pub use coordinator::{
    AnalyzeMaintenanceArchiveRequest, AnalyzeMaintenancePacksRequest,
    AnalyzeMaintenanceRelationRequest, AnalyzeMaintenanceSummariesRequest,
    AnalyzeMaintenanceSummaryRequest, AnalyzeMaintenanceTopicRequest, ApplyMaintenancePackRequest,
    ApplyMaintenanceRelationRequest, ApplyMaintenanceSummaryRequest, ApplyMaintenanceTopicRequest,
    MaintenanceArchiveAnalysis, MaintenanceArchiveCandidate, MaintenanceArchiveDecision,
    MaintenanceArchiveScan, MaintenanceArchiveScanPage, MaintenanceCycleReport,
    MaintenancePackAnalysis, MaintenancePackAnalysisIssue, MaintenancePackCandidate,
    MaintenancePackInput, MaintenancePackScan, MaintenancePackScanGroup,
    MaintenanceRelationAnalysis, MaintenanceRelationCandidate, MaintenanceRelationInput,
    MaintenanceRelationScan, MaintenanceRelationScanGroup, MaintenanceReviewDecision,
    MaintenanceSummaryAnalysis, MaintenanceSummaryAnalysisIssue, MaintenanceSummaryBatchAnalysis,
    MaintenanceSummaryCandidate, MaintenanceSummaryScan, MaintenanceSummaryScanPage,
    MaintenanceTopicAnalysis, MaintenanceTopicCandidate, MaintenanceTopicInput,
    MaintenanceTopicRefreshTarget, MaintenanceTopicScan, MaintenanceTopicScanGroup,
    MaintenanceWorkScan, RuntimeMaintainer,
};
pub use infer_worker::InferRuntimeSemanticWorker;
pub use ledger::{
    MaintenanceAutomationState, MaintenanceAutomationStatus, MaintenanceCycleRecord,
    MaintenanceDirtyRegionStatus, MaintenanceRelationReviewPage, MaintenanceRelationReviewProposal,
    MaintenanceRelationReviewStatus, MaintenanceWakeReason,
};
pub use operator::MaintenanceOperator;
pub use reconciliation::MaintenanceReconciliationCandidate;
pub use review::{
    MaintenanceReviewItem, MaintenanceReviewOrigin, MaintenanceReviewPayload,
    MaintenanceReviewStatus,
};
pub use review_budget::{BudgetSnapshot, ReviewBudgetConfig, ReviewTier, TokenLimitMode};
pub use worker::{
    CommandSemanticWorker, MaintenanceDetailPage, MaintenanceRelation, MaintenanceReviewStep,
    MaintenanceRoutingPage, MaintenanceSummarySelection, MaintenanceVerification,
    MaintenanceWorkerOutcome, MaintenanceWorkerRequest, MaintenanceWorkerResponse,
    PackingCandidateGroup, RelationCandidatePage, RetentionMilestone, SemanticMaintenanceWorker,
    TopicReviewFeedback, VerificationVerdict, VerifiedMaintenanceRevision,
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
            escalation_deployment_id,
            escalation_operations,
            review_budget,
            ..
        } => Ok(Arc::new(
            InferRuntimeSemanticWorker::new(
                credential_file.clone(),
                Duration::from_secs(*timeout_seconds),
                summary_deployment_id.clone(),
                reasoning_deployment_id.clone(),
                relation_deployment_id.clone(),
                escalation_deployment_id.clone(),
                escalation_operations.clone(),
            )?
            .with_review_budget(review_budget.clone()),
        )),
    }
}

#[cfg(test)]
mod tests;
