use std::{
    fs::Permissions,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use tokio::fs;
use uuid::Uuid;

use super::{
    MaintenanceCycleReport, MaintenanceWorkerRequest, MaintenanceWorkerResponse,
    SemanticMaintenanceWorker,
};

const MAX_AUDIT_RECORDS: usize = 64;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceRunAuditRecord {
    pub job_id: String,
    pub mode: String,
    pub max_jobs: u32,
    pub reason: String,
    pub started_at: String,
    pub duration_ms: u64,
    pub status: String,
    pub events: Vec<MaintenanceRunAuditEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<MaintenanceCycleReport>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceRunAuditEvent {
    pub state: String,
    pub at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_stage: Option<String>,
}

#[derive(Default, Deserialize, Serialize)]
struct MaintenanceAuditLog {
    records: Vec<MaintenanceRunAuditRecord>,
}

pub struct MaintenanceRunAudit {
    record: MaintenanceRunAuditRecord,
    started: Instant,
    events: Arc<Mutex<Vec<MaintenanceRunAuditEvent>>>,
}

impl MaintenanceRunAudit {
    pub fn queued(reason: String) -> Self {
        Self::queued_with_limits("observe", 1, reason)
    }

    pub fn queued_with_limits(mode: &str, max_jobs: u32, reason: String) -> Self {
        let started_at = now();
        let events = Arc::new(Mutex::new(vec![event("queued", None, None, None)]));
        Self {
            record: MaintenanceRunAuditRecord {
                job_id: format!("mrun_{}", Uuid::new_v4().simple()),
                mode: mode.to_owned(),
                max_jobs,
                reason,
                started_at,
                duration_ms: 0,
                status: "queued".to_owned(),
                events: Vec::new(),
                report: None,
            },
            started: Instant::now(),
            events,
        }
    }

    pub fn worker(
        &self,
        inner: Arc<dyn SemanticMaintenanceWorker>,
    ) -> Arc<dyn SemanticMaintenanceWorker> {
        Arc::new(AuditedWorker {
            inner,
            events: Arc::clone(&self.events),
        })
    }

    pub fn complete(mut self, report: MaintenanceCycleReport) -> MaintenanceRunAuditRecord {
        self.record.status = "completed".to_owned();
        self.record.report = Some(report);
        self.finish(None)
    }

    pub fn fail(mut self, failure_stage: &str) -> MaintenanceRunAuditRecord {
        self.record.status = "failed".to_owned();
        self.finish(Some(failure_stage))
    }

    fn finish(mut self, failure_stage: Option<&str>) -> MaintenanceRunAuditRecord {
        if let Some(stage) = failure_stage {
            self.events
                .lock()
                .expect("maintenance audit events")
                .push(event("failed", None, None, Some(stage)));
        } else {
            self.events
                .lock()
                .expect("maintenance audit events")
                .push(event("completed", None, None, None));
        }
        self.record.duration_ms = self
            .started
            .elapsed()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX);
        self.record.events = self
            .events
            .lock()
            .expect("maintenance audit events")
            .clone();
        self.record
    }
}

pub async fn persist_audit(path: &Path, record: MaintenanceRunAuditRecord) -> Result<()> {
    let mut log = match fs::read(path).await {
        Ok(bytes) => serde_json::from_slice::<MaintenanceAuditLog>(&bytes)
            .context("decode PCP maintenance audit")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            MaintenanceAuditLog::default()
        }
        Err(error) => return Err(error).context("read PCP maintenance audit"),
    };
    log.records.push(record);
    let surplus = log.records.len().saturating_sub(MAX_AUDIT_RECORDS);
    if surplus > 0 {
        log.records.drain(..surplus);
    }
    let bytes = serde_json::to_vec_pretty(&log).context("encode PCP maintenance audit")?;
    let parent = path
        .parent()
        .context("PCP maintenance audit path has no parent")?;
    fs::create_dir_all(parent)
        .await
        .context("create PCP maintenance audit directory")?;
    let temporary = audit_temporary_path(path);
    fs::write(&temporary, bytes)
        .await
        .context("write PCP maintenance audit")?;
    fs::rename(&temporary, path)
        .await
        .context("publish PCP maintenance audit")?;
    fs::set_permissions(path, Permissions::from_mode(0o600))
        .await
        .context("secure PCP maintenance audit")?;
    Ok(())
}

struct AuditedWorker {
    inner: Arc<dyn SemanticMaintenanceWorker>,
    events: Arc<Mutex<Vec<MaintenanceRunAuditEvent>>>,
}

#[async_trait]
impl SemanticMaintenanceWorker for AuditedWorker {
    async fn evaluate(
        &self,
        request: MaintenanceWorkerRequest,
    ) -> Result<MaintenanceWorkerResponse> {
        let operation = operation_name(&request).to_owned();
        self.events
            .lock()
            .expect("maintenance audit events")
            .push(event("worker_started", Some(operation), None, None));
        match self.inner.evaluate(request).await {
            Ok(response) => {
                self.events
                    .lock()
                    .expect("maintenance audit events")
                    .push(event(
                        "worker_response",
                        None,
                        Some(response_name(&response).to_owned()),
                        None,
                    ));
                Ok(response)
            }
            Err(error) => {
                self.events
                    .lock()
                    .expect("maintenance audit events")
                    .push(event("failed", None, None, Some("worker")));
                Err(error)
            }
        }
    }
}

fn event(
    state: &str,
    operation: Option<String>,
    response: Option<String>,
    failure_stage: Option<&str>,
) -> MaintenanceRunAuditEvent {
    MaintenanceRunAuditEvent {
        state: state.to_owned(),
        at: now(),
        operation,
        response,
        failure_stage: failure_stage.map(str::to_owned),
    }
}

fn operation_name(request: &MaintenanceWorkerRequest) -> &'static str {
    match request {
        MaintenanceWorkerRequest::SummarizePage { .. } => "summarize_page",
        MaintenanceWorkerRequest::SummarizePages { .. } => "summarize_pages",
        MaintenanceWorkerRequest::SelectPacking { .. } => "select_packing",
        MaintenanceWorkerRequest::AnalyzePacking { .. } => "analyze_packing",
        MaintenanceWorkerRequest::SelectRelation { .. } => "select_relation",
        MaintenanceWorkerRequest::SelectRetentionMilestones { .. } => "select_retention_milestones",
    }
}

fn response_name(response: &MaintenanceWorkerResponse) -> &'static str {
    match response {
        MaintenanceWorkerResponse::WriteSummary { .. } => "write_summary",
        MaintenanceWorkerResponse::Summaries { .. } => "summaries",
        MaintenanceWorkerResponse::Candidate { .. } => "candidate",
        MaintenanceWorkerResponse::PackingCandidates { .. } => "packing_candidates",
        MaintenanceWorkerResponse::Relate { .. } => "relate",
        MaintenanceWorkerResponse::Retain { .. } => "retain",
        MaintenanceWorkerResponse::NoCandidate => "no_candidate",
        MaintenanceWorkerResponse::Defer => "defer",
    }
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn audit_temporary_path(path: &Path) -> PathBuf {
    path.with_extension(format!("tmp-{}", Uuid::new_v4().simple()))
}

#[cfg(test)]
mod tests {
    use super::MaintenanceAuditLog;

    #[test]
    fn legacy_reports_default_new_semantic_counters() {
        let log: MaintenanceAuditLog = serde_json::from_str(
            r#"{
                "records": [{
                    "jobId": "mrun_legacy",
                    "mode": "observe",
                    "maxJobs": 1,
                    "reason": "legacy",
                    "startedAt": "2026-08-15T00:00:00.000Z",
                    "durationMs": 1,
                    "status": "completed",
                    "events": [],
                    "report": {
                        "inspectedPages": 874,
                        "workerCalls": 1,
                        "summariesWritten": 0,
                        "summariesProposed": 0,
                        "retentionLeasesWritten": 0,
                        "retentionLeasesProposed": 0,
                        "deferred": 0
                    }
                }]
            }"#,
        )
        .expect("decode a legacy maintenance audit");

        let report = log.records[0].report.as_ref().expect("legacy audit report");
        assert_eq!(report.packs_committed, 0);
        assert_eq!(report.packs_proposed, 0);
        assert_eq!(report.relations_committed, 0);
        assert_eq!(report.relations_proposed, 0);
    }
}
