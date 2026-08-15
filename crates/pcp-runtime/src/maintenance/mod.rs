mod audit;
mod config;
mod coordinator;
mod infer_worker;
mod ledger;
mod worker;

use std::{sync::Arc, time::Duration};

use anyhow::Result;

pub use audit::{MaintenanceRunAudit, MaintenanceRunAuditRecord, persist_audit};
pub use config::{
    MaintenanceConfig, MaintenanceMode, MaintenanceWorkerConfig, PackingMaintenanceConfig,
    RelationMaintenanceConfig, RetentionMaintenanceConfig, SummaryMaintenanceConfig,
};
pub use coordinator::{MaintenanceCycleReport, RuntimeMaintainer};
pub use infer_worker::InferRuntimeSemanticWorker;
pub use worker::{
    CommandSemanticWorker, MaintenanceDetailPage, MaintenanceRelation, MaintenanceRoutingPage,
    MaintenanceWorkerRequest, MaintenanceWorkerResponse, RelationCandidatePage, RetentionMilestone,
    SemanticMaintenanceWorker,
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
            max_output_tokens,
            ..
        } => Ok(Arc::new(InferRuntimeSemanticWorker::new(
            credential_file.clone(),
            Duration::from_secs(*timeout_seconds),
            *max_output_tokens,
        )?)),
    }
}

#[cfg(test)]
mod tests;
