use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, SecondsFormat, Utc};
use pcp_client::PcpApi;
use pcp_core::{
    AccessSession, ApplyReconciliationRequest, ExtractTopicRequest, FeedbackAuthority,
    LinkPagesRequest, PACKED_PAGE_MEDIA_TYPE, PackPagesRequest, PageMutability, PageRevisionRef,
    PlanRevisionRetentionRequest, Projection, PutRevisionRetentionLeaseRequest, ReadPagesRequest,
    ReconciliationDisposition, RetentionPolicy, RuntimeUsageEvent, SourceSpan, WriteResult,
    WriteSummaryRequest,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{sync::watch, time::Instant};
use uuid::Uuid;

use super::{
    MaintenanceConfig, MaintenanceMode, MaintenanceReconciliationCandidate,
    MaintenanceWorkerOutcome, MaintenanceWorkerRequest, MaintenanceWorkerResponse,
    PackingMaintenanceConfig, RelationMaintenanceConfig, RetentionMilestone,
    SemanticMaintenanceWorker,
    ledger::{
        MaintenanceAutomationStatus, MaintenanceLedger, MaintenanceRelationReviewPage,
        MaintenanceRelationReviewProposal, MaintenanceRelationReviewStatus, MaintenanceWakeReason,
        maintenance_region_key, packing_key, retention_window_key, selection_window_key,
        summary_key,
    },
    review::{
        MaintenanceReviewItem, MaintenanceReviewOrigin, MaintenanceReviewPayload,
        MaintenanceReviewStatus,
    },
    worker::{
        ArchiveCandidatePage, ArchiveWorkerDecision, ExistingTopicPage, MaintenanceDetailPage,
        MaintenanceRoutingPage, PackingCandidateGroup, PackingCandidatePage, RelationCandidatePage,
    },
};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MaintenanceCycleReport {
    pub inspected_pages: usize,
    pub jobs_advanced: u32,
    pub worker_calls: u32,
    pub summaries_written: u32,
    pub summaries_proposed: u32,
    pub packs_committed: u32,
    pub packs_proposed: u32,
    pub relations_committed: u32,
    pub relations_proposed: u32,
    pub retention_leases_written: u32,
    pub retention_leases_proposed: u32,
    pub topics_proposed: u32,
    pub archives_proposed: u32,
    pub reconciliations_committed: u32,
    pub reconciliations_proposed: u32,
    pub review_items_proposed: u32,
    pub escalated_decisions: u32,
    pub deferred: u32,
}

impl MaintenanceCycleReport {
    fn merge(&mut self, report: Self) {
        self.inspected_pages = self.inspected_pages.max(report.inspected_pages);
        self.jobs_advanced = self.jobs_advanced.saturating_add(report.jobs_advanced);
        self.worker_calls = self.worker_calls.saturating_add(report.worker_calls);
        self.summaries_written = self
            .summaries_written
            .saturating_add(report.summaries_written);
        self.summaries_proposed = self
            .summaries_proposed
            .saturating_add(report.summaries_proposed);
        self.packs_committed = self.packs_committed.saturating_add(report.packs_committed);
        self.packs_proposed = self.packs_proposed.saturating_add(report.packs_proposed);
        self.relations_committed = self
            .relations_committed
            .saturating_add(report.relations_committed);
        self.relations_proposed = self
            .relations_proposed
            .saturating_add(report.relations_proposed);
        self.retention_leases_written = self
            .retention_leases_written
            .saturating_add(report.retention_leases_written);
        self.retention_leases_proposed = self
            .retention_leases_proposed
            .saturating_add(report.retention_leases_proposed);
        self.topics_proposed = self.topics_proposed.saturating_add(report.topics_proposed);
        self.archives_proposed = self
            .archives_proposed
            .saturating_add(report.archives_proposed);
        self.reconciliations_committed = self
            .reconciliations_committed
            .saturating_add(report.reconciliations_committed);
        self.reconciliations_proposed = self
            .reconciliations_proposed
            .saturating_add(report.reconciliations_proposed);
        self.review_items_proposed = self
            .review_items_proposed
            .saturating_add(report.review_items_proposed);
        self.escalated_decisions = self
            .escalated_decisions
            .saturating_add(report.escalated_decisions);
        self.deferred = self.deferred.saturating_add(report.deferred);
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenancePackScan {
    pub captured_at: String,
    pub scan_id: String,
    pub inspected_pages: usize,
    pub eligible_pages: usize,
    pub excluded_pages: usize,
    pub candidate_group_count: usize,
    pub estimated_model_calls: usize,
    pub groups: Vec<MaintenancePackScanGroup>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenancePackScanGroup {
    pub group_id: String,
    pub namespace: String,
    pub kind: String,
    pub source_span: SourceSpan,
    pub page_count: usize,
    pub content_chars: u64,
    pub extends_existing_pack: bool,
}

/// Read-only maintenance inventory. Each phase is scanned from the same current
/// Store snapshot; model evaluation and writes happen only in later phase calls.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceWorkScan {
    pub captured_at: String,
    pub inspected_pages: usize,
    pub packing: MaintenancePackScan,
    pub summary: MaintenanceSummaryScan,
    pub topic: MaintenanceTopicScan,
    pub relation: MaintenanceRelationScan,
}

/// A separate, manual-only content-governance scan.  Unlike ordinary
/// maintenance it is never included in scheduled cycles: the structural
/// signals merely identify Pages that merit a human-reviewed archive decision.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceArchiveScan {
    pub captured_at: String,
    pub scan_id: String,
    pub inspected_pages: usize,
    pub eligible_pages: usize,
    pub estimated_model_calls: usize,
    pub pages: Vec<MaintenanceArchiveScanPage>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceArchiveScanPage {
    pub page_id: String,
    pub revision_id: String,
    pub namespace: String,
    pub kind: String,
    pub observed_at: String,
    pub content_chars: u64,
    pub preview: String,
    /// Structural reasons to inspect, not a claim that the Page is low value.
    pub candidate_signals: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnalyzeMaintenanceArchiveRequest {
    pub scan_id: String,
    pub page_id: String,
    pub revision_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceArchiveDecision {
    Archive,
    Retain,
    Defer,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceArchiveAnalysis {
    pub analyzed_at: String,
    pub decision: MaintenanceArchiveDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<MaintenanceArchiveCandidate>,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceArchiveCandidate {
    pub candidate_id: String,
    pub page_id: String,
    pub revision_id: String,
    pub namespace: String,
    pub kind: String,
    pub observed_at: String,
    pub content_chars: u64,
    pub preview: String,
    pub candidate_signals: Vec<String>,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceSummaryScan {
    pub scan_id: String,
    pub inspected_pages: usize,
    pub eligible_pages: usize,
    pub estimated_model_calls: usize,
    pub pages: Vec<MaintenanceSummaryScanPage>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceSummaryScanPage {
    pub page_id: String,
    pub revision_id: String,
    pub namespace: String,
    pub content_chars: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceRelationScan {
    pub scan_id: String,
    pub inspected_pages: usize,
    pub eligible_pages: usize,
    pub candidate_group_count: usize,
    pub estimated_model_calls: usize,
    pub groups: Vec<MaintenanceRelationScanGroup>,
}

/// A structural candidate window for a topic front door.  The window is only
/// a bounded reading set: it does not assert that the contained Pages share a
/// topic.  The semantic worker must explicitly select its sources.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceTopicScan {
    pub scan_id: String,
    pub inspected_pages: usize,
    pub eligible_pages: usize,
    pub candidate_group_count: usize,
    pub estimated_model_calls: usize,
    pub groups: Vec<MaintenanceTopicScanGroup>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceTopicScanGroup {
    pub group_id: String,
    pub page_count: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceRelationScanGroup {
    pub group_id: String,
    pub anchor_page_id: String,
    pub anchor_revision_id: String,
    pub page_count: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenancePackAnalysis {
    pub analyzed_at: String,
    pub scan_id: String,
    pub batch_index: usize,
    pub batch_count: usize,
    pub candidate_group_count: usize,
    pub analyzed_group_count: usize,
    pub worker_calls: u32,
    pub overlap_retries: u32,
    pub no_candidate_groups: u32,
    pub deferred_groups: u32,
    pub candidates: Vec<MaintenancePackCandidate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<MaintenancePackAnalysisIssue>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenancePackAnalysisIssue {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnalyzeMaintenancePacksRequest {
    pub scan_id: String,
    pub batch_index: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenancePackCandidate {
    pub candidate_id: String,
    pub namespace: String,
    pub kind: String,
    pub source_span: SourceSpan,
    pub input_page_count: usize,
    pub resulting_entry_count: u64,
    pub content_chars: u64,
    pub extends_existing_pack: bool,
    pub pages: Vec<MaintenancePackInput>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenancePackInput {
    pub page_id: String,
    pub revision_id: String,
    pub source_span: SourceSpan,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    pub preview: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyMaintenancePackRequest {
    pub candidate_id: String,
    pub pages: Vec<PageRevisionRef>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnalyzeMaintenanceSummaryRequest {
    pub page_id: String,
    pub revision_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnalyzeMaintenanceSummariesRequest {
    pub scan_id: String,
    pub pages: Vec<PageRevisionRef>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceSummaryAnalysis {
    pub analyzed_at: String,
    pub decision: MaintenanceReviewDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<MaintenanceSummaryCandidate>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceReviewDecision {
    Candidate,
    NoCandidate,
    Defer,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceSummaryCandidate {
    pub candidate_id: String,
    pub page_id: String,
    pub revision_id: String,
    pub namespace: String,
    pub content_chars: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_summary_revision_id: Option<String>,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceSummaryBatchAnalysis {
    pub analyzed_at: String,
    pub requested_pages: usize,
    pub analyzed_pages: usize,
    pub worker_calls: u32,
    pub no_candidate_pages: u32,
    pub deferred_pages: u32,
    pub candidates: Vec<MaintenanceSummaryCandidate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<MaintenanceSummaryAnalysisIssue>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceSummaryAnalysisIssue {
    pub batch_index: usize,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyMaintenanceSummaryRequest {
    pub candidate_id: String,
    pub page_id: String,
    pub revision_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_summary_revision_id: Option<String>,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnalyzeMaintenanceRelationRequest {
    pub scan_id: String,
    pub group_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceRelationAnalysis {
    pub analyzed_at: String,
    pub decision: MaintenanceReviewDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<MaintenanceRelationCandidate>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceRelationCandidate {
    pub candidate_id: String,
    pub namespace: String,
    pub pages: [MaintenanceRelationInput; 2],
    #[serde(default)]
    pub relation_reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceRelationInput {
    pub page_id: String,
    pub revision_id: String,
    pub preview: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyMaintenanceRelationRequest {
    pub candidate_id: String,
    pub pages: [PageRevisionRef; 2],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnalyzeMaintenanceTopicRequest {
    pub scan_id: String,
    pub group_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceTopicAnalysis {
    pub analyzed_at: String,
    pub decision: MaintenanceReviewDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<MaintenanceTopicCandidate>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceTopicCandidate {
    pub candidate_id: String,
    pub namespace: String,
    pub title: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_target: Option<MaintenanceTopicRefreshTarget>,
    pub pages: Vec<MaintenanceTopicInput>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceTopicRefreshTarget {
    pub page_id: String,
    pub revision_id: String,
    pub title: String,
    pub preview: String,
    pub source_page_count: usize,
    pub shared_source_page_count: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceTopicInput {
    pub page_id: String,
    pub revision_id: String,
    pub preview: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyMaintenanceTopicRequest {
    pub candidate_id: String,
    pub title: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_target: Option<PageRevisionRef>,
    pub pages: Vec<PageRevisionRef>,
}

const PACKING_PREVIEW_CHARS: usize = 480;
// A full analysis window already contains enough context for one routing decision.
// Keep requests independent so each result has a single, inspectable source window.
const PACKING_GROUPS_PER_MODEL_CALL: usize = 1;
const MAX_PACK_ENTRY_GAP_SECONDS: i64 = 15 * 60;
const PACKING_RETRY_AFTER_SECONDS: u64 = 86_400;
const MAINTENANCE_READ_BATCH_PAGES: usize = 20;
const MAX_MAINTENANCE_SUMMARY_CHARS: usize = 480;
const SUMMARY_REVIEW_PAGES_PER_MODEL_CALL: usize = 1;
// Summary proposals are routing metadata, not a replacement for source Pages.
// Keep each local-worker decision page-local, while bounding the request-wide read.
const MAX_SUMMARY_REVIEW_INPUT_CHARS: u32 = 24_000;
const MAX_SUMMARY_REVIEW_PAGES_PER_REQUEST: usize = 16;
const ARCHIVE_MINIMUM_AGE_DAYS: i64 = 14;
const MAX_ARCHIVE_CANDIDATES: usize = 40;
const MAX_ARCHIVE_REVIEW_INPUT_CHARS: u32 = 24_000;
const REVIEW_SNOOZE_SECONDS: u64 = 24 * 60 * 60;

pub struct RuntimeMaintainer {
    client: Arc<dyn PcpApi>,
    worker: Arc<dyn SemanticMaintenanceWorker>,
    config: MaintenanceConfig,
    ledger: MaintenanceLedger,
    usage_source: &'static str,
    write_wakeup: Option<watch::Receiver<u64>>,
}

async fn wait_for_scheduler_wakeup(
    write_wakeup: &mut Option<watch::Receiver<u64>>,
    delay_seconds: u64,
    timer_reason: MaintenanceWakeReason,
) -> MaintenanceWakeReason {
    let Some(receiver) = write_wakeup.as_mut() else {
        tokio::time::sleep(Duration::from_secs(delay_seconds)).await;
        return timer_reason;
    };
    let write_channel_closed = tokio::select! {
        _ = tokio::time::sleep(Duration::from_secs(delay_seconds)) => return timer_reason,
        result = receiver.changed() => result.is_err(),
    };
    if write_channel_closed {
        *write_wakeup = None;
        tokio::time::sleep(Duration::from_secs(delay_seconds)).await;
        timer_reason
    } else {
        MaintenanceWakeReason::ExternalWrite
    }
}

impl RuntimeMaintainer {
    pub async fn load(
        client: Arc<dyn PcpApi>,
        worker: Arc<dyn SemanticMaintenanceWorker>,
        config: MaintenanceConfig,
    ) -> Result<Self> {
        Self::load_with_usage_source(client, worker, config, "automatic_maintenance").await
    }

    pub async fn load_with_usage_source(
        client: Arc<dyn PcpApi>,
        worker: Arc<dyn SemanticMaintenanceWorker>,
        config: MaintenanceConfig,
        usage_source: &'static str,
    ) -> Result<Self> {
        config.validate()?;
        let ledger = MaintenanceLedger::load(&config.state_path).await?;
        Ok(Self {
            client,
            worker,
            config,
            ledger,
            usage_source,
            write_wakeup: None,
        })
    }

    pub async fn load_operator_observe_once(
        client: Arc<dyn PcpApi>,
        worker: Arc<dyn SemanticMaintenanceWorker>,
        mut config: MaintenanceConfig,
    ) -> Result<Self> {
        anyhow::ensure!(
            config.enabled && config.mode == MaintenanceMode::Observe,
            "operator maintenance run-once requires enabled observe maintenance"
        );
        config.max_jobs_per_cycle = 1;
        config.validate()?;
        Ok(Self {
            client,
            worker,
            config,
            // An operator smoke run must not be blocked by, or mutate, the normal cadence.
            ledger: MaintenanceLedger::default(),
            usage_source: "manual_maintenance",
            write_wakeup: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        client: Arc<dyn PcpApi>,
        worker: Arc<dyn SemanticMaintenanceWorker>,
        config: MaintenanceConfig,
    ) -> Self {
        Self {
            client,
            worker,
            config,
            ledger: MaintenanceLedger::default(),
            usage_source: "test_maintenance",
            write_wakeup: None,
        }
    }

    pub fn with_write_wakeup(mut self, write_wakeup: watch::Receiver<u64>) -> Self {
        self.write_wakeup = Some(write_wakeup);
        self
    }

    async fn evaluate_worker(
        &self,
        request: MaintenanceWorkerRequest,
    ) -> Result<MaintenanceWorkerOutcome> {
        let operation = worker_operation(&request).to_owned();
        let scopes = worker_scopes(&request, self.client.access());
        let started = Instant::now();
        let outcome = self.worker.evaluate_with_usage(request).await;
        let duration_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        let (usage, failure_kind) = match &outcome {
            Ok(outcome) => (outcome.usage.clone(), None),
            Err(_) => (None, Some("worker_failed".to_owned())),
        };
        let operation = match &outcome {
            Ok(outcome) if outcome.escalated => format!("{operation}_escalated"),
            _ => operation,
        };
        let event = RuntimeUsageEvent {
            event_id: format!("ru_{}", Uuid::new_v4().simple()),
            occurred_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            principal: self.client.access().principal.clone(),
            session_id: self.client.access().session_id.clone(),
            source: self.usage_source.to_owned(),
            operation,
            scopes,
            duration_ms,
            usage,
            failure_kind,
        };
        if let Err(error) = self.client.record_runtime_usage(event).await {
            eprintln!("PCP maintenance model usage write failed: {error:#}");
        }
        outcome
    }

    async fn repair_packing_overlap_worker(
        &self,
        request: MaintenanceWorkerRequest,
    ) -> Result<MaintenanceWorkerOutcome> {
        let operation = format!("{}_repair", worker_operation(&request));
        let scopes = worker_scopes(&request, self.client.access());
        let started = Instant::now();
        let outcome = self
            .worker
            .repair_packing_analysis_overlap_with_usage(request)
            .await;
        let duration_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        let (usage, failure_kind) = match &outcome {
            Ok(outcome) => (outcome.usage.clone(), None),
            Err(_) => (None, Some("worker_failed".to_owned())),
        };
        let event = RuntimeUsageEvent {
            event_id: format!("ru_{}", Uuid::new_v4().simple()),
            occurred_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            principal: self.client.access().principal.clone(),
            session_id: self.client.access().session_id.clone(),
            source: self.usage_source.to_owned(),
            operation,
            scopes,
            duration_ms,
            usage,
            failure_kind,
        };
        if let Err(error) = self.client.record_runtime_usage(event).await {
            eprintln!("PCP maintenance model usage write failed: {error:#}");
        }
        outcome
    }

    pub async fn run_forever(mut self) -> Result<()> {
        let mut wake_reason = if self.config.initial_delay_seconds > 0 {
            self.ledger
                .schedule_initial_wake(self.config.initial_delay_seconds);
            self.ledger.save(&self.config.state_path).await?;
            self.wait_for_wakeup(
                self.config.initial_delay_seconds,
                MaintenanceWakeReason::Startup,
            )
            .await
        } else {
            MaintenanceWakeReason::Startup
        };
        loop {
            let (delay, timer_reason) = match self.run_scheduled_cycle_for(wake_reason).await {
                Ok(report) => {
                    if report.summaries_written > 0
                        || report.summaries_proposed > 0
                        || report.packs_committed > 0
                        || report.packs_proposed > 0
                        || report.relations_committed > 0
                        || report.relations_proposed > 0
                        || report.topics_proposed > 0
                        || report.archives_proposed > 0
                        || report.review_items_proposed > 0
                        || report.retention_leases_written > 0
                        || report.retention_leases_proposed > 0
                    {
                        eprintln!(
                            "PCP maintenance: {} Summary proposed / {} written, {} pack proposed / {} committed, {} relation proposed / {} committed, {} Topic / {} archive reviews proposed, {} retention proposed / {} leased after {} jobs and {} worker calls",
                            report.summaries_proposed,
                            report.summaries_written,
                            report.packs_proposed,
                            report.packs_committed,
                            report.relations_proposed,
                            report.relations_committed,
                            report.topics_proposed,
                            report.archives_proposed,
                            report.retention_leases_proposed,
                            report.retention_leases_written,
                            report.jobs_advanced,
                            report.worker_calls
                        );
                    }
                    let jobs_advanced = report.jobs_advanced;
                    let active_retry = jobs_advanced >= self.config.max_jobs_per_cycle
                        || (jobs_advanced > 0 && self.ledger.has_dirty_regions());
                    let delay = self.ledger.schedule_after_success(&self.config, &report);
                    let timer_reason = if active_retry {
                        MaintenanceWakeReason::ActiveRetry
                    } else {
                        MaintenanceWakeReason::Timer
                    };
                    (delay, timer_reason)
                }
                Err(error) => {
                    eprintln!("PCP maintenance cycle failed: {error:#}");
                    (
                        self.ledger.schedule_after_failure(&self.config),
                        MaintenanceWakeReason::ErrorRetry,
                    )
                }
            };
            self.ledger.save(&self.config.state_path).await?;
            wake_reason = self.wait_for_wakeup(delay, timer_reason).await;
        }
    }

    async fn wait_for_wakeup(
        &mut self,
        delay_seconds: u64,
        timer_reason: MaintenanceWakeReason,
    ) -> MaintenanceWakeReason {
        wait_for_scheduler_wakeup(&mut self.write_wakeup, delay_seconds, timer_reason).await
    }

    pub async fn automation_status(
        config: &MaintenanceConfig,
    ) -> Result<MaintenanceAutomationStatus> {
        Ok(MaintenanceLedger::load(&config.state_path)
            .await?
            .automation_status(config))
    }

    pub async fn run_once(&mut self) -> Result<MaintenanceCycleReport> {
        self.run_once_inner(true, self.config.max_jobs_per_cycle, None, false, false)
            .await
    }

    pub async fn run_bounded_cycle(&mut self) -> Result<MaintenanceCycleReport> {
        let mut aggregate = MaintenanceCycleReport::default();
        while aggregate.jobs_advanced < self.config.max_jobs_per_cycle {
            let report = self.run_once_inner(false, 1, None, false, false).await?;
            let jobs_advanced = report.jobs_advanced;
            aggregate.merge(report);
            if jobs_advanced == 0 {
                break;
            }
        }
        self.ledger.save(&self.config.state_path).await?;
        Ok(aggregate)
    }

    /// The long-running scheduler is write-driven.  Its interval is only the
    /// maximum discovery latency for a lightweight inventory watermark; no
    /// semantic worker is called until a dirty region becomes eligible.
    #[cfg(test)]
    pub(crate) async fn run_scheduled_cycle(&mut self) -> Result<MaintenanceCycleReport> {
        self.run_scheduled_cycle_for(MaintenanceWakeReason::Timer)
            .await
    }

    async fn run_scheduled_cycle_for(
        &mut self,
        wake_reason: MaintenanceWakeReason,
    ) -> Result<MaintenanceCycleReport> {
        self.ledger.start_scheduled_cycle(wake_reason);
        self.ledger.save(&self.config.state_path).await?;
        let result = self.run_scheduled_cycle_inner().await;
        match result {
            Ok(report) => {
                self.ledger.complete_scheduled_cycle(report.clone());
                self.ledger.save(&self.config.state_path).await?;
                Ok(report)
            }
            Err(error) => {
                self.ledger.fail_scheduled_cycle(&error);
                let _ = self.ledger.save(&self.config.state_path).await;
                Err(error)
            }
        }
    }

    async fn run_scheduled_cycle_inner(&mut self) -> Result<MaintenanceCycleReport> {
        let inventory = self.client.durable_page_inventory(Vec::new()).await?;
        self.ledger
            .observe_writes(&inventory, &self.config.write_trigger);
        let mut aggregate = MaintenanceCycleReport {
            inspected_pages: inventory.len(),
            ..MaintenanceCycleReport::default()
        };
        self.ledger.update_scheduled_cycle(aggregate.clone());
        self.ledger.save(&self.config.state_path).await?;
        let regions = self.ledger.ready_regions(&self.config.write_trigger);
        if regions.is_empty() {
            return Ok(aggregate);
        }
        let expected = MaintenanceLedger::region_snapshot(&inventory, &regions);
        while aggregate.jobs_advanced < self.config.max_jobs_per_cycle {
            let report = self
                .run_once_inner(false, 1, Some(&regions), true, true)
                .await?;
            let jobs_advanced = report.jobs_advanced;
            aggregate.merge(report);
            self.ledger.update_scheduled_cycle(aggregate.clone());
            self.ledger.save(&self.config.state_path).await?;
            if jobs_advanced == 0 {
                break;
            }
        }
        let refreshed = self.client.durable_page_inventory(Vec::new()).await?;
        self.ledger
            .observe_writes(&refreshed, &self.config.write_trigger);
        if aggregate.jobs_advanced < self.config.max_jobs_per_cycle {
            self.ledger
                .acknowledge_unchanged_regions(&expected, &refreshed);
        }
        Ok(aggregate)
    }

    async fn scoped_inventory(
        &self,
        regions: Option<&BTreeSet<String>>,
    ) -> Result<Vec<pcp_store::DurablePageInventoryItem>> {
        let inventory = self.client.durable_page_inventory(Vec::new()).await?;
        Ok(match regions {
            Some(regions) => inventory
                .into_iter()
                .filter(|page| regions.contains(&maintenance_region_key(page)))
                .collect(),
            None => inventory,
        })
    }

    pub async fn scan_packing_candidates(&self) -> Result<MaintenancePackScan> {
        let inventory = self.client.durable_page_inventory(Vec::new()).await?;
        let windows = self
            .time_continuous_packing_windows(packing_candidate_windows(
                &inventory,
                &self.config.packing,
            ))
            .await?;
        Ok(packing_scan_from_windows(
            &inventory,
            &windows,
            &self.config.packing,
            &Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        ))
    }

    /// Scans for conservative archive-review candidates. This is deliberately
    /// outside normal maintenance and scheduled cycles: it only produces a
    /// bounded review set and does not call a model or change any Page.
    pub async fn scan_archive_candidates(&self) -> Result<MaintenanceArchiveScan> {
        let inventory = self.client.durable_page_inventory(Vec::new()).await?;
        Ok(archive_scan_from_inventory(&inventory, Utc::now()))
    }

    /// Asks the semantic worker to assess one current candidate. The response
    /// is still only a proposal; archive application remains an explicit
    /// human-controlled lifecycle operation at the Console boundary.
    pub async fn analyze_archive_candidate(
        &self,
        request: AnalyzeMaintenanceArchiveRequest,
    ) -> Result<MaintenanceArchiveAnalysis> {
        let inventory = self.client.durable_page_inventory(Vec::new()).await?;
        let scan = archive_scan_from_inventory(&inventory, Utc::now());
        // Archive review authority is Page-local: unrelated Store writes may change the
        // aggregate scan id while this exact candidate and Revision remain current. Recheck
        // both against the fresh inventory below instead of invalidating the entire batch.
        let scan_changed = request.scan_id != scan.scan_id;
        let scan_page = scan
            .pages
            .iter()
            .find(|page| page.page_id == request.page_id)
            .with_context(|| {
                if scan_changed {
                    "content-governance archive scan changed and this candidate is no longer eligible"
                } else {
                    "content-governance archive candidate no longer exists"
                }
            })?;
        anyhow::ensure!(
            scan_page.revision_id == request.revision_id,
            "content-governance archive candidate revision changed after scan"
        );

        let mut pages = self
            .read_detail_pages(
                vec![scan_page.revision_id.clone()],
                MAX_ARCHIVE_REVIEW_INPUT_CHARS,
            )
            .await?;
        let page = pages
            .pop()
            .context("content-governance archive candidate disappeared")?;
        let response = self
            .evaluate_worker(MaintenanceWorkerRequest::AssessArchive {
                page: ArchiveCandidatePage {
                    page,
                    candidate_signals: scan_page.candidate_signals.clone(),
                },
            })
            .await?;
        let (decision, candidate, reason) = match response.response {
            MaintenanceWorkerResponse::ArchiveReview { outcome, reason } => {
                let reason = validate_archive_reason(reason)?;
                match outcome {
                    ArchiveWorkerDecision::Archive => (
                        MaintenanceArchiveDecision::Archive,
                        Some(build_archive_candidate(scan_page, reason.clone())),
                        reason,
                    ),
                    ArchiveWorkerDecision::Retain => {
                        (MaintenanceArchiveDecision::Retain, None, reason)
                    }
                    ArchiveWorkerDecision::Defer => {
                        (MaintenanceArchiveDecision::Defer, None, reason)
                    }
                }
            }
            MaintenanceWorkerResponse::Defer => (
                MaintenanceArchiveDecision::Defer,
                None,
                "The semantic worker deferred this archive review.".to_owned(),
            ),
            other => anyhow::bail!(
                "semantic worker returned {} for an assess_archive request",
                response_name(&other)
            ),
        };
        Ok(MaintenanceArchiveAnalysis {
            analyzed_at: maintenance_review_timestamp(),
            decision,
            candidate,
            reason,
        })
    }

    pub fn pending_relation_reviews(&self) -> Vec<MaintenanceRelationReviewProposal> {
        self.ledger.relation_reviews()
    }

    pub fn pending_reviews(&self) -> Vec<MaintenanceReviewItem> {
        let mut reviews = self.ledger.review_items();
        reviews.extend(
            self.ledger
                .relation_reviews()
                .into_iter()
                .map(MaintenanceReviewItem::relation),
        );
        reviews.sort_by(|left, right| {
            left.proposed_at
                .cmp(&right.proposed_at)
                .then_with(|| left.candidate_id.cmp(&right.candidate_id))
        });
        reviews
    }

    pub fn review_item(&self, candidate_id: &str) -> Option<MaintenanceReviewItem> {
        self.ledger.review_item(candidate_id).or_else(|| {
            self.ledger
                .relation_review(candidate_id)
                .filter(|proposal| proposal.status == MaintenanceRelationReviewStatus::Pending)
                .map(MaintenanceReviewItem::relation)
        })
    }

    pub async fn resolve_review(
        &mut self,
        candidate_id: &str,
        status: MaintenanceReviewStatus,
    ) -> Result<()> {
        if status == MaintenanceReviewStatus::Deferred {
            self.ledger
                .snooze_review(candidate_id, REVIEW_SNOOZE_SECONDS)?;
        } else if self.ledger.review_item(candidate_id).is_some() {
            self.ledger.resolve_review(candidate_id, status)?;
        } else {
            anyhow::bail!("legacy PCP relation reviews must use their typed decision operation")
        }
        self.ledger.save(&self.config.state_path).await
    }

    pub async fn approve_relation_review(
        &mut self,
        candidate_id: &str,
    ) -> Result<pcp_core::Relation> {
        anyhow::ensure!(
            self.config.applies_changes(),
            "PCP relation review approval requires apply mode"
        );
        let proposal = self
            .ledger
            .relation_review(candidate_id)
            .context("unknown PCP relation review candidate")?;
        anyhow::ensure!(
            proposal.status == MaintenanceRelationReviewStatus::Pending,
            "PCP relation review candidate is no longer pending"
        );
        let revision_ids = proposal
            .pages
            .iter()
            .map(|page| page.revision_id.clone())
            .collect::<Vec<_>>();
        for page in &proposal.pages {
            anyhow::ensure!(
                self.client
                    .current_revision_id(page.page_id.clone())
                    .await?
                    == page.revision_id,
                "PCP relation review candidate changed after proposal"
            );
        }
        let page_ids = [
            proposal.pages[0].page_id.clone(),
            proposal.pages[1].page_id.clone(),
        ];
        anyhow::ensure!(
            !self.related_pair_exists(&page_ids, &revision_ids).await?,
            "PCP relation review candidate is already explicitly related"
        );
        let relation = self
            .client
            .link_pages(LinkPagesRequest {
                from_page_id: proposal.pages[0].page_id.clone(),
                relation_type: proposal.relation_type,
                to_page_id: proposal.pages[1].page_id.clone(),
                basis_revision_ids: revision_ids,
                created_by: self.config.worker_actor(),
                idempotency_key: Some(format!("maintenance:review:{candidate_id}")),
            })
            .await?;
        self.ledger
            .resolve_relation_review(candidate_id, MaintenanceRelationReviewStatus::Accepted)?;
        self.ledger.save(&self.config.state_path).await?;
        Ok(relation)
    }

    pub async fn approve_reconciliation_review(
        &mut self,
        candidate_id: &str,
    ) -> Result<pcp_core::ReconciliationResult> {
        anyhow::ensure!(
            self.config.applies_changes(),
            "PCP reconciliation approval requires apply mode"
        );
        let item = self
            .ledger
            .review_item(candidate_id)
            .context("unknown PCP reconciliation review candidate")?;
        let MaintenanceReviewPayload::Reconciliation(candidate) = item.payload else {
            anyhow::bail!("maintenance review candidate is not a reconciliation")
        };
        anyhow::ensure!(
            item.status == MaintenanceReviewStatus::Pending,
            "PCP reconciliation review candidate is no longer pending"
        );
        let result = self
            .client
            .apply_reconciliation(reconciliation_request(
                &candidate,
                self.config.worker_actor(),
                Some(format!("maintenance:review:{candidate_id}")),
                Some(self.config.worker.actor_id().to_owned()),
            ))
            .await?;
        self.ledger
            .resolve_review(candidate_id, MaintenanceReviewStatus::Accepted)?;
        self.ledger.save(&self.config.state_path).await?;
        Ok(result)
    }

    pub async fn reject_relation_review(
        &mut self,
        candidate_id: &str,
        suppress: bool,
    ) -> Result<()> {
        let status = if suppress {
            MaintenanceRelationReviewStatus::Suppressed
        } else {
            MaintenanceRelationReviewStatus::Rejected
        };
        self.ledger.resolve_relation_review(candidate_id, status)?;
        self.ledger.save(&self.config.state_path).await
    }

    pub async fn scan_maintenance_work(&self) -> Result<MaintenanceWorkScan> {
        let inventory = self.client.durable_page_inventory(Vec::new()).await?;
        let captured_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let windows = self
            .time_continuous_packing_windows(packing_candidate_windows(
                &inventory,
                &self.config.packing,
            ))
            .await?;
        let packing =
            packing_scan_from_windows(&inventory, &windows, &self.config.packing, &captured_at);
        let summary = summary_scan_from_inventory(&inventory, &self.config.summary);
        let active_packing_page_ids = self.active_packing_page_ids();
        let relation = relation_scan_from_inventory(
            &inventory,
            &self.config.relation,
            &active_packing_page_ids,
        );
        let topic =
            topic_scan_from_inventory(&inventory, &self.config.relation, &active_packing_page_ids);
        Ok(MaintenanceWorkScan {
            captured_at,
            inspected_pages: inventory.len(),
            packing,
            summary,
            topic,
            relation,
        })
    }

    pub async fn analyze_packing_candidates(
        &self,
        request: AnalyzeMaintenancePacksRequest,
    ) -> Result<MaintenancePackAnalysis> {
        let inventory = self.client.durable_page_inventory(Vec::new()).await?;
        let windows = self
            .time_continuous_packing_windows(packing_candidate_windows(
                &inventory,
                &self.config.packing,
            ))
            .await?;
        let scan_id = packing_scan_id(&windows, &self.config.packing);
        anyhow::ensure!(
            request.scan_id == scan_id,
            "maintenance packing scan is stale; scan the Store again"
        );

        let batch_count = windows.len().div_ceil(PACKING_GROUPS_PER_MODEL_CALL);
        anyhow::ensure!(
            request.batch_index < batch_count,
            "maintenance packing analysis batch {} is outside 0..{}",
            request.batch_index,
            batch_count
        );
        let batch_start = request.batch_index * PACKING_GROUPS_PER_MODEL_CALL;
        let batch_end = (batch_start + PACKING_GROUPS_PER_MODEL_CALL).min(windows.len());
        let batch = &windows[batch_start..batch_end];
        // Temporal continuity, stream identity, and protected identifiers are
        // candidate gates only. Adjacent Packs can still have changed topic, so
        // every proposed Pack boundary is decided by the semantic worker.
        let groups: Vec<PackingCandidateGroup> = batch
            .iter()
            .map(|window| PackingCandidateGroup {
                group_id: packing_scan_group_id(window),
                pages: window
                    .iter()
                    .map(|page| {
                        PackingCandidatePage::from_inventory(
                            page,
                            self.config.packing.routing_chars_per_page,
                        )
                    })
                    .collect(),
            })
            .collect();

        let worker_request = MaintenanceWorkerRequest::AnalyzePacking {
            groups,
            max_pages_per_candidate: self.config.packing.max_pages,
        };
        let initial_response = self.evaluate_worker(worker_request.clone()).await;
        let (response, worker_calls, overlap_retries) = match initial_response {
            Ok(outcome)
                if matches!(
                    &outcome.response,
                    MaintenanceWorkerResponse::PackingCandidates { candidates }
                        if packing_candidates_overlap(candidates)
                ) =>
            {
                let initial_attempts = outcome.model_attempts;
                let repaired = self.repair_packing_overlap_worker(worker_request).await;
                let repair_attempts = repaired
                    .as_ref()
                    .map(|outcome| outcome.model_attempts)
                    .unwrap_or(1);
                (
                    repaired.map(|outcome| outcome.response),
                    initial_attempts.saturating_add(repair_attempts),
                    1,
                )
            }
            Ok(outcome) => {
                let attempts = outcome.model_attempts;
                (Ok(outcome.response), attempts, 0)
            }
            Err(error) => (Err(error), 1, 0),
        };

        let mut candidates = Vec::new();
        let mut no_candidate_groups = 0_u32;
        let mut deferred_groups = 0_u32;
        let mut issue = None;
        match response {
            Ok(MaintenanceWorkerResponse::PackingCandidates {
                candidates: selected_sets,
            }) => match validate_packing_analysis_batch(batch, selected_sets, &self.config.packing)
            {
                Ok((mut selected, represented_groups)) => {
                    candidates.append(&mut selected);
                    no_candidate_groups = batch.len().saturating_sub(represented_groups) as u32;
                }
                Err(error) => {
                    deferred_groups = batch.len() as u32;
                    issue = Some(MaintenancePackAnalysisIssue {
                        code: "invalid_model_selection".to_owned(),
                        message: if overlap_retries > 0 {
                            format!("after one disjoint-output retry: {error}")
                        } else {
                            error.to_string()
                        },
                    });
                }
            },
            Ok(MaintenanceWorkerResponse::NoCandidate) => {
                no_candidate_groups = batch.len() as u32;
            }
            Ok(MaintenanceWorkerResponse::Defer) => {
                deferred_groups = batch.len() as u32;
            }
            Ok(other) => {
                deferred_groups = batch.len() as u32;
                issue = Some(MaintenancePackAnalysisIssue {
                    code: "unexpected_worker_response".to_owned(),
                    message: format!(
                        "semantic worker returned {} for an analyze_packing request",
                        response_name(&other)
                    ),
                });
            }
            Err(error) => {
                deferred_groups = batch.len() as u32;
                issue = Some(MaintenancePackAnalysisIssue {
                    code: "worker_failure".to_owned(),
                    message: format!("{error:#}"),
                });
            }
        }

        Ok(MaintenancePackAnalysis {
            analyzed_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            scan_id,
            batch_index: request.batch_index,
            batch_count,
            candidate_group_count: windows.len(),
            analyzed_group_count: batch.len(),
            worker_calls,
            overlap_retries,
            no_candidate_groups,
            deferred_groups,
            candidates,
            issue,
        })
    }

    pub async fn apply_pack_candidate(
        &self,
        request: ApplyMaintenancePackRequest,
    ) -> Result<WriteResult> {
        anyhow::ensure!(
            self.config.mode == MaintenanceMode::Apply,
            "maintenance Pack optimization requires apply mode"
        );
        let inventory = self.client.durable_page_inventory(Vec::new()).await?;
        let candidate = validate_pack_application(&inventory, &request, &self.config.packing)?;

        self.client
            .pack_pages(PackPagesRequest {
                pages: request.pages,
                idempotency_key: Some(format!("maintenance:{}", candidate.candidate_id)),
            })
            .await
    }

    pub async fn analyze_summary_candidate(
        &self,
        request: AnalyzeMaintenanceSummaryRequest,
    ) -> Result<MaintenanceSummaryAnalysis> {
        let inventory = self.client.durable_page_inventory(Vec::new()).await?;
        let candidate = inventory
            .iter()
            .find(|page| page.page_id == request.page_id)
            .context("maintenance Summary Page no longer exists")?;
        anyhow::ensure!(
            candidate.revision_id == request.revision_id,
            "maintenance Summary candidate revisions changed after review"
        );
        if !summary_page_eligible(candidate, &self.config.summary)
            || candidate.summary_target_revision_id.as_deref()
                == Some(candidate.revision_id.as_str())
        {
            return Ok(MaintenanceSummaryAnalysis::no_candidate());
        }

        let mut pages = self
            .read_detail_pages(
                vec![candidate.revision_id.clone()],
                self.config.summary.max_input_chars,
            )
            .await?;
        let page = pages
            .pop()
            .context("maintenance Summary Page disappeared")?;
        let source_text = page.content.clone().unwrap_or_default();
        let response = self
            .evaluate_worker(MaintenanceWorkerRequest::SummarizePage {
                page: Box::new(page),
            })
            .await?;
        match response.response {
            MaintenanceWorkerResponse::WriteSummary { content } => {
                let content = normalize_worker_summary(content, &source_text)?;
                Ok(MaintenanceSummaryAnalysis::candidate(
                    build_summary_candidate(candidate, content),
                ))
            }
            MaintenanceWorkerResponse::NoCandidate => {
                Ok(MaintenanceSummaryAnalysis::no_candidate())
            }
            MaintenanceWorkerResponse::Defer => Ok(MaintenanceSummaryAnalysis::defer()),
            other => anyhow::bail!(
                "semantic worker returned {} for a summarize_page review request",
                response_name(&other)
            ),
        }
    }

    pub async fn analyze_summary_candidates(
        &self,
        request: AnalyzeMaintenanceSummariesRequest,
    ) -> Result<MaintenanceSummaryBatchAnalysis> {
        anyhow::ensure!(
            !request.pages.is_empty()
                && request.pages.len() <= MAX_SUMMARY_REVIEW_PAGES_PER_REQUEST,
            "maintenance Summary review accepts 1..={MAX_SUMMARY_REVIEW_PAGES_PER_REQUEST} Pages"
        );
        let inventory = self.client.durable_page_inventory(Vec::new()).await?;
        let scan = summary_scan_from_inventory(&inventory, &self.config.summary);
        anyhow::ensure!(
            request.scan_id == scan.scan_id,
            "maintenance Summary scan is stale; scan the Store again"
        );
        let mut requested_ids = BTreeSet::new();
        anyhow::ensure!(
            request
                .pages
                .iter()
                .all(|page| requested_ids.insert(page.page_id.as_str())),
            "maintenance Summary review contains duplicate Pages"
        );

        let mut eligible = Vec::new();
        let mut no_candidate_pages = 0_u32;
        for requested in &request.pages {
            let Some(page) = inventory
                .iter()
                .find(|page| page.page_id == requested.page_id)
            else {
                no_candidate_pages = no_candidate_pages.saturating_add(1);
                continue;
            };
            if page.revision_id != requested.revision_id
                || !summary_page_eligible(page, &self.config.summary)
                || page.summary_target_revision_id.as_deref() == Some(page.revision_id.as_str())
            {
                no_candidate_pages = no_candidate_pages.saturating_add(1);
                continue;
            }
            eligible.push(page);
        }

        let detail_pages = self
            .read_detail_pages(
                eligible
                    .iter()
                    .map(|page| page.revision_id.clone())
                    .collect(),
                self.config
                    .summary
                    .max_input_chars
                    .min(MAX_SUMMARY_REVIEW_INPUT_CHARS),
            )
            .await?;
        let details = detail_pages
            .into_iter()
            .map(|page| (page.page_id.clone(), page))
            .collect::<HashMap<_, _>>();

        let mut analysis = MaintenanceSummaryBatchAnalysis {
            analyzed_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            requested_pages: request.pages.len(),
            analyzed_pages: 0,
            worker_calls: 0,
            no_candidate_pages,
            deferred_pages: 0,
            candidates: Vec::new(),
            issues: Vec::new(),
        };
        for (page_index, page) in eligible.iter().enumerate() {
            let Some(detail) = details.get(&page.page_id).cloned() else {
                analysis.deferred_pages = analysis.deferred_pages.saturating_add(1);
                analysis.issues.push(MaintenanceSummaryAnalysisIssue {
                    batch_index: page_index,
                    message: format!("Summary Page {} disappeared during review", page.page_id),
                });
                continue;
            };
            analysis.analyzed_pages = analysis.analyzed_pages.saturating_add(1);
            let source_text = detail.content.clone().unwrap_or_default();
            let response = self
                .evaluate_worker(MaintenanceWorkerRequest::SummarizePage {
                    page: Box::new(detail),
                })
                .await;
            match response {
                Ok(outcome) => {
                    analysis.worker_calls =
                        analysis.worker_calls.saturating_add(outcome.model_attempts);
                    match outcome.response {
                        MaintenanceWorkerResponse::WriteSummary { content } => {
                            match normalize_worker_summary(content, &source_text) {
                                Ok(content) => analysis
                                    .candidates
                                    .push(build_summary_candidate(page, content)),
                                Err(error) => {
                                    analysis.deferred_pages =
                                        analysis.deferred_pages.saturating_add(1);
                                    analysis.issues.push(MaintenanceSummaryAnalysisIssue {
                                        batch_index: page_index,
                                        message: error.to_string(),
                                    });
                                }
                            }
                        }
                        MaintenanceWorkerResponse::NoCandidate => {
                            analysis.no_candidate_pages =
                                analysis.no_candidate_pages.saturating_add(1);
                        }
                        MaintenanceWorkerResponse::Defer => {
                            analysis.deferred_pages = analysis.deferred_pages.saturating_add(1);
                        }
                        other => {
                            analysis.deferred_pages = analysis.deferred_pages.saturating_add(1);
                            analysis.issues.push(MaintenanceSummaryAnalysisIssue {
                        batch_index: page_index,
                        message: format!(
                            "semantic worker returned {} for a summarize_page review request",
                            response_name(&other)
                        ),
                    });
                        }
                    }
                }
                Err(error) => {
                    analysis.worker_calls = analysis.worker_calls.saturating_add(1);
                    analysis.deferred_pages = analysis.deferred_pages.saturating_add(1);
                    analysis.issues.push(MaintenanceSummaryAnalysisIssue {
                        batch_index: page_index,
                        message: format!("{error:#}"),
                    });
                }
            }
        }
        Ok(analysis)
    }

    pub async fn apply_summary_candidate(
        &self,
        request: ApplyMaintenanceSummaryRequest,
    ) -> Result<pcp_core::WriteSummaryResult> {
        anyhow::ensure!(
            self.config.mode == MaintenanceMode::Apply,
            "maintenance Summary optimization requires apply mode"
        );
        let inventory = self.client.durable_page_inventory(Vec::new()).await?;
        let page = inventory
            .iter()
            .find(|page| page.page_id == request.page_id)
            .context("maintenance Summary candidate is stale or no longer eligible")?;
        anyhow::ensure!(
            page.revision_id == request.revision_id
                && summary_page_eligible(page, &self.config.summary)
                && page.summary_revision_id == request.expected_summary_revision_id
                && page.summary_target_revision_id.as_deref() != Some(page.revision_id.as_str()),
            "maintenance Summary candidate is stale or no longer eligible"
        );
        let mut details = self
            .read_detail_pages(
                vec![page.revision_id.clone()],
                self.config.summary.max_input_chars,
            )
            .await?;
        let source = details
            .pop()
            .context("maintenance Summary candidate Page disappeared")?;
        let content = normalize_worker_summary(
            request.content,
            source.content.as_deref().unwrap_or_default(),
        )?;
        let candidate = build_summary_candidate(page, content.clone());
        anyhow::ensure!(
            candidate.candidate_id == request.candidate_id
                && candidate.expected_summary_revision_id == request.expected_summary_revision_id,
            "maintenance Summary candidate identity no longer matches the reviewed Page"
        );
        self.client
            .write_summary(WriteSummaryRequest {
                target_page_id: page.page_id.clone(),
                target_revision_id: page.revision_id.clone(),
                expected_summary_revision_id: page.summary_revision_id.clone(),
                content,
                created_by: self.config.worker_actor(),
                tool_or_model: Some(self.config.worker.actor_id().to_owned()),
                provenance: Vec::new(),
                idempotency_key: Some(format!("maintenance:summary:{}", candidate.candidate_id)),
            })
            .await
    }

    pub async fn analyze_relation_candidate(
        &self,
        request: AnalyzeMaintenanceRelationRequest,
    ) -> Result<MaintenanceRelationAnalysis> {
        let inventory = self.client.durable_page_inventory(Vec::new()).await?;
        let active_packing_page_ids = self.active_packing_page_ids();
        let scan = relation_scan_from_inventory(
            &inventory,
            &self.config.relation,
            &active_packing_page_ids,
        );
        anyhow::ensure!(
            request.scan_id == scan.scan_id,
            "maintenance relation scan is stale; scan the Store again"
        );
        let windows =
            relation_candidate_windows(&inventory, &self.config.relation, &active_packing_page_ids);
        let Some(window) = windows
            .into_iter()
            .find(|window| relation_scan_group_id(window) == request.group_id)
        else {
            return Ok(MaintenanceRelationAnalysis::no_candidate());
        };

        let offered = window
            .iter()
            .map(|page| (page.page_id.clone(), page.revision_id.clone()))
            .collect::<HashMap<_, _>>();
        let offered_page_ids = offered.keys().cloned().collect::<BTreeSet<_>>();
        let relation_edges = self.existing_related_pairs(&window).await?;
        let mut recorded_relation_pairs = self.ledger.active_relation_pairs();
        recorded_relation_pairs.extend(self.ledger.rejected_relation_pairs(&offered));
        let excluded_page_pairs = relation_excluded_page_pairs(
            &offered_page_ids,
            &relation_edges,
            &recorded_relation_pairs,
        );
        if all_relation_pairs_excluded(&offered_page_ids, &excluded_page_pairs) {
            return Ok(MaintenanceRelationAnalysis::no_candidate());
        }
        let response = self
            .evaluate_worker(MaintenanceWorkerRequest::SelectRelation {
                pages: window
                    .iter()
                    .map(|page| {
                        RelationCandidatePage::from_inventory(
                            page,
                            self.config.relation.routing_chars_per_page,
                        )
                    })
                    .collect(),
                excluded_page_pairs: excluded_page_pairs.clone(),
            })
            .await?;
        let (mut page_ids, relation_reason) = match response.response {
            MaintenanceWorkerResponse::Relate { page_ids, reason } => {
                (page_ids, validate_relation_reason(reason)?)
            }
            MaintenanceWorkerResponse::NoCandidate => {
                return Ok(MaintenanceRelationAnalysis::no_candidate());
            }
            MaintenanceWorkerResponse::Defer => return Ok(MaintenanceRelationAnalysis::defer()),
            other => anyhow::bail!(
                "semantic worker returned {} for a select_relation review request",
                response_name(&other)
            ),
        };
        if page_ids[0] == page_ids[1]
            || !page_ids.iter().all(|page_id| offered.contains_key(page_id))
        {
            return Ok(MaintenanceRelationAnalysis::defer());
        }
        page_ids.sort();
        if excluded_page_pairs.iter().any(|pair| pair == &page_ids) {
            return Ok(MaintenanceRelationAnalysis::no_candidate());
        }
        let selected = page_ids
            .iter()
            .map(|page_id| {
                window
                    .iter()
                    .find(|page| page.page_id == *page_id)
                    .expect("offered relation Page exists")
            })
            .collect::<Vec<_>>();
        let mut candidate = build_relation_candidate(&selected);
        candidate.relation_reason = relation_reason;
        Ok(MaintenanceRelationAnalysis::candidate(candidate))
    }

    pub async fn apply_relation_candidate(
        &self,
        request: ApplyMaintenanceRelationRequest,
    ) -> Result<pcp_core::Relation> {
        anyhow::ensure!(
            self.config.mode == MaintenanceMode::Apply,
            "maintenance relation optimization requires apply mode"
        );
        let inventory = self.client.durable_page_inventory(Vec::new()).await?;
        let current_by_id = inventory
            .iter()
            .map(|page| (page.page_id.as_str(), page))
            .collect::<HashMap<_, _>>();
        anyhow::ensure!(
            request.pages[0].page_id != request.pages[1].page_id,
            "maintenance relation candidate contains duplicate Pages"
        );
        let mut selected = request
            .pages
            .iter()
            .map(|requested| {
                let page = current_by_id
                    .get(requested.page_id.as_str())
                    .copied()
                    .context("maintenance relation candidate is stale or no longer eligible")?;
                anyhow::ensure!(
                    page.revision_id == requested.revision_id
                        && relation_page_eligible(page, &self.config.relation),
                    "maintenance relation candidate is stale or no longer eligible"
                );
                Ok::<_, anyhow::Error>(page)
            })
            .collect::<Result<Vec<_>>>()?;
        selected.sort_by(|left, right| left.page_id.cmp(&right.page_id));
        anyhow::ensure!(
            selected[0].namespace == selected[1].namespace,
            "maintenance relation candidate Pages no longer share a Scope"
        );
        let candidate = build_relation_candidate(&selected);
        anyhow::ensure!(
            candidate.candidate_id == request.candidate_id,
            "maintenance relation candidate identity no longer matches the reviewed Pages"
        );
        let page_ids = [selected[0].page_id.clone(), selected[1].page_id.clone()];
        anyhow::ensure!(
            !self
                .ledger
                .suppressed_relation_pairs()
                .into_iter()
                .any(|pair| pair == page_ids),
            "maintenance relation candidate was explicitly suppressed by the operator"
        );
        let revision_ids = [
            selected[0].revision_id.clone(),
            selected[1].revision_id.clone(),
        ];
        anyhow::ensure!(
            !self
                .ledger
                .relation_pair_is_rejected(&page_ids, &revision_ids),
            "maintenance relation candidate was rejected for the reviewed revisions"
        );
        if self.related_pair_exists(&page_ids, &revision_ids).await? {
            anyhow::bail!("maintenance relation candidate is already explicitly related");
        }
        self.client
            .link_pages(LinkPagesRequest {
                from_page_id: page_ids[0].clone(),
                relation_type: "related_to".to_owned(),
                to_page_id: page_ids[1].clone(),
                basis_revision_ids: revision_ids.to_vec(),
                created_by: self.config.worker_actor(),
                idempotency_key: Some(format!(
                    "maintenance:related:{}:{}",
                    revision_ids[0], revision_ids[1]
                )),
            })
            .await
    }

    pub async fn suppress_relation_candidate(
        &mut self,
        request: ApplyMaintenanceRelationRequest,
    ) -> Result<()> {
        self.record_relation_candidate_decision(
            request,
            MaintenanceRelationReviewStatus::Suppressed,
        )
        .await
    }

    pub async fn reject_relation_candidate(
        &mut self,
        request: ApplyMaintenanceRelationRequest,
    ) -> Result<()> {
        self.record_relation_candidate_decision(request, MaintenanceRelationReviewStatus::Rejected)
            .await
    }

    async fn record_relation_candidate_decision(
        &mut self,
        request: ApplyMaintenanceRelationRequest,
        status: MaintenanceRelationReviewStatus,
    ) -> Result<()> {
        anyhow::ensure!(
            matches!(
                status,
                MaintenanceRelationReviewStatus::Rejected
                    | MaintenanceRelationReviewStatus::Suppressed
            ),
            "unsupported maintenance relation decision"
        );
        let inventory = self.client.durable_page_inventory(Vec::new()).await?;
        let current_by_id = inventory
            .iter()
            .map(|page| (page.page_id.as_str(), page))
            .collect::<HashMap<_, _>>();
        anyhow::ensure!(
            request.pages[0].page_id != request.pages[1].page_id,
            "maintenance relation candidate contains duplicate Pages"
        );
        let mut selected = request
            .pages
            .iter()
            .map(|requested| {
                let page = current_by_id
                    .get(requested.page_id.as_str())
                    .copied()
                    .context("maintenance relation candidate is stale or no longer eligible")?;
                anyhow::ensure!(
                    page.revision_id == requested.revision_id
                        && relation_page_eligible(page, &self.config.relation),
                    "maintenance relation candidate is stale or no longer eligible"
                );
                Ok::<_, anyhow::Error>(page)
            })
            .collect::<Result<Vec<_>>>()?;
        selected.sort_by(|left, right| left.page_id.cmp(&right.page_id));
        anyhow::ensure!(
            selected[0].namespace == selected[1].namespace,
            "maintenance relation candidate Pages no longer share a Scope"
        );
        let candidate = build_relation_candidate(&selected);
        anyhow::ensure!(
            candidate.candidate_id == request.candidate_id,
            "maintenance relation candidate identity no longer matches the reviewed Pages"
        );
        let namespace = candidate.namespace;
        let pages = candidate.pages.map(|page| MaintenanceRelationReviewPage {
            page_id: page.page_id,
            revision_id: page.revision_id,
            preview: page.preview,
        });
        match status {
            MaintenanceRelationReviewStatus::Rejected => {
                self.ledger
                    .reject_relation_pair(namespace, pages, candidate.relation_reason)?
            }
            MaintenanceRelationReviewStatus::Suppressed => {
                self.ledger
                    .suppress_relation_pair(namespace, pages, candidate.relation_reason)?
            }
            _ => unreachable!("validated relation decision status"),
        }
        self.ledger.save(&self.config.state_path).await
    }

    pub async fn analyze_topic_candidate(
        &self,
        request: AnalyzeMaintenanceTopicRequest,
    ) -> Result<MaintenanceTopicAnalysis> {
        let inventory = self.client.durable_page_inventory(Vec::new()).await?;
        let active_packing_page_ids = self.active_packing_page_ids();
        let scan =
            topic_scan_from_inventory(&inventory, &self.config.relation, &active_packing_page_ids);
        anyhow::ensure!(
            request.scan_id == scan.scan_id,
            "maintenance Topic scan is stale; scan the Store again"
        );
        let windows =
            topic_candidate_windows(&inventory, &self.config.relation, &active_packing_page_ids);
        let Some(window) = windows
            .into_iter()
            .find(|window| topic_scan_group_id(window) == request.group_id)
        else {
            return Ok(MaintenanceTopicAnalysis::no_candidate());
        };
        let offered = window
            .iter()
            .map(|page| page.page_id.clone())
            .collect::<BTreeSet<_>>();
        let existing_topics = existing_topics_for_window(&inventory, &window);
        let response = self
            .evaluate_worker(MaintenanceWorkerRequest::ExtractTopic {
                pages: window
                    .iter()
                    .map(|page| {
                        RelationCandidatePage::from_inventory(
                            page,
                            self.config.relation.routing_chars_per_page,
                        )
                    })
                    .collect(),
                existing_topics: existing_topics.clone(),
                max_source_pages: 8,
            })
            .await?;
        let response = response.response;
        let MaintenanceWorkerResponse::ExtractTopic {
            page_ids,
            title,
            content,
            reason,
            refresh_topic_page_id,
        } = response
        else {
            return match response {
                MaintenanceWorkerResponse::NoCandidate => {
                    Ok(MaintenanceTopicAnalysis::no_candidate())
                }
                MaintenanceWorkerResponse::Defer => Ok(MaintenanceTopicAnalysis::defer()),
                other => anyhow::bail!(
                    "semantic worker returned {} for an extract_topic review request",
                    response_name(&other)
                ),
            };
        };
        anyhow::ensure!(
            (2..=8).contains(&page_ids.len())
                && page_ids.iter().collect::<BTreeSet<_>>().len() == page_ids.len()
                && page_ids.iter().all(|page_id| offered.contains(page_id)),
            "semantic worker selected invalid Topic sources for the reviewed window"
        );
        let selected = page_ids
            .iter()
            .map(|page_id| {
                window
                    .iter()
                    .find(|page| page.page_id == *page_id)
                    .expect("offered Topic Page exists")
            })
            .collect::<Vec<_>>();
        let refresh_target = select_topic_refresh_target(
            &selected,
            &existing_topics,
            refresh_topic_page_id.as_deref(),
        )?;
        Ok(MaintenanceTopicAnalysis::candidate(build_topic_candidate(
            &selected,
            title,
            content,
            Some(reason),
            refresh_target,
        )?))
    }

    pub async fn apply_topic_candidate(
        &self,
        request: ApplyMaintenanceTopicRequest,
    ) -> Result<WriteResult> {
        anyhow::ensure!(
            self.config.mode == MaintenanceMode::Apply,
            "maintenance Topic extraction requires apply mode"
        );
        let inventory = self.client.durable_page_inventory(Vec::new()).await?;
        anyhow::ensure!(
            (2..=64).contains(&request.pages.len()),
            "maintenance Topic candidate needs 2..=64 current source Pages"
        );
        let mut selected = Vec::with_capacity(request.pages.len());
        let mut ids = BTreeSet::new();
        for source in &request.pages {
            anyhow::ensure!(
                ids.insert(source.page_id.as_str()),
                "maintenance Topic candidate has duplicate Pages"
            );
            let current = inventory
                .iter()
                .find(|page| page.page_id == source.page_id)
                .context("maintenance Topic candidate is stale or unavailable")?;
            anyhow::ensure!(
                current.revision_id == source.revision_id && current.kind != "topic_summary",
                "maintenance Topic candidate is stale or unavailable"
            );
            selected.push(current);
        }
        let candidate = build_topic_candidate(
            &selected,
            request.title.clone(),
            request.content.clone(),
            None,
            topic_refresh_target_from_request(
                &inventory,
                &selected,
                request.refresh_target.as_ref(),
            )?,
        )?;
        anyhow::ensure!(
            candidate.candidate_id == request.candidate_id,
            "maintenance Topic candidate identity no longer matches the reviewed Pages"
        );
        self.client
            .extract_topic(ExtractTopicRequest {
                target_topic: candidate
                    .refresh_target
                    .as_ref()
                    .map(|target| PageRevisionRef {
                        page_id: target.page_id.clone(),
                        revision_id: target.revision_id.clone(),
                    }),
                source_pages: request.pages,
                title: candidate.title,
                content: candidate.content,
                created_by: self.config.worker_actor(),
                tool_or_model: Some(self.config.worker.actor_id().to_owned()),
                provenance: Vec::new(),
                idempotency_key: Some(format!("maintenance:topic:{}", candidate.candidate_id)),
            })
            .await
    }

    pub async fn run_once_with_job_limit(
        &mut self,
        max_jobs: u32,
    ) -> Result<MaintenanceCycleReport> {
        anyhow::ensure!(max_jobs > 0, "maintenance job limit must be positive");
        self.run_once_inner(
            true,
            max_jobs.min(self.config.max_jobs_per_cycle),
            None,
            false,
            false,
        )
        .await
    }

    pub async fn run_convergence_once(&mut self, max_jobs: u32) -> Result<MaintenanceCycleReport> {
        anyhow::ensure!(max_jobs > 0, "maintenance job limit must be positive");
        self.run_once_inner(
            true,
            max_jobs.min(self.config.max_jobs_per_cycle),
            None,
            false,
            true,
        )
        .await
    }

    pub async fn run_operator_observe_once(&mut self) -> Result<MaintenanceCycleReport> {
        anyhow::ensure!(
            self.config.mode == MaintenanceMode::Observe,
            "operator maintenance run-once only permits observe mode"
        );
        self.run_once_inner(false, 1, None, false, false).await
    }

    async fn run_once_inner(
        &mut self,
        persist_ledger: bool,
        max_jobs: u32,
        regions: Option<&BTreeSet<String>>,
        scheduled: bool,
        include_governance: bool,
    ) -> Result<MaintenanceCycleReport> {
        let mut inventory = self.scoped_inventory(regions).await?;
        let mut report = MaintenanceCycleReport {
            inspected_pages: inventory.len(),
            ..MaintenanceCycleReport::default()
        };
        let mut jobs_remaining = max_jobs;
        let review_origin = if scheduled {
            MaintenanceReviewOrigin::Automatic
        } else {
            MaintenanceReviewOrigin::Manual
        };

        if self.config.reconciliation.enabled
            && jobs_remaining > 0
            && self
                .run_reconciliation_job(&mut report, review_origin)
                .await
                .context("run PCP feedback reconciliation maintenance job")?
        {
            report.jobs_advanced += 1;
            jobs_remaining -= 1;
            inventory = self.scoped_inventory(regions).await?;
            report.inspected_pages = report.inspected_pages.max(inventory.len());
        }

        // Pack boundaries change the Page surface. Exhaust the current Pack
        // pass before Summary or Relation sees the inventory. If the cycle
        // budget is consumed here, semantic maintenance waits for the next
        // eligible automatic cycle instead of reasoning over a half-packed
        // conversation.
        while self.config.packing.enabled && jobs_remaining > 0 {
            let packing_ran = self
                .run_packing_job(&inventory, &mut report, review_origin)
                .await
                .context("run PCP packing maintenance job")?;
            if !packing_ran {
                break;
            }
            report.jobs_advanced += 1;
            jobs_remaining -= 1;
            inventory = self.scoped_inventory(regions).await?;
            report.inspected_pages = report.inspected_pages.max(inventory.len());
        }

        let summary_ran = self.config.summary.enabled
            && jobs_remaining > 0
            && self
                .run_summary_job(&inventory, &mut report, review_origin)
                .await
                .context("run PCP Summary maintenance job")?;
        if summary_ran {
            report.jobs_advanced += 1;
            jobs_remaining -= 1;
            inventory = self.scoped_inventory(regions).await?;
            report.inspected_pages = report.inspected_pages.max(inventory.len());
        }
        if self.config.relation.enabled
            && jobs_remaining > 0
            && self
                .run_relation_job(&inventory, &mut report)
                .await
                .context("run PCP relation maintenance job")?
        {
            report.jobs_advanced += 1;
            jobs_remaining -= 1;
        }
        if include_governance
            && self.config.relation.enabled
            && jobs_remaining > 0
            && self
                .run_topic_review_job(&inventory, &mut report, review_origin)
                .await
                .context("run PCP Topic maintenance review job")?
        {
            report.jobs_advanced += 1;
            jobs_remaining -= 1;
        }
        if include_governance
            && jobs_remaining > 0
            && self
                .run_archive_review_job(&inventory, &mut report, review_origin)
                .await
                .context("run PCP archive maintenance review job")?
        {
            report.jobs_advanced += 1;
            jobs_remaining -= 1;
        }
        if self.config.retention.enabled && jobs_remaining > 0 {
            if self
                .run_retention_job(&mut report)
                .await
                .context("run PCP semantic retention maintenance job")?
            {
                report.jobs_advanced += 1;
            }
        }

        if persist_ledger {
            self.ledger.save(&self.config.state_path).await?;
        }
        Ok(report)
    }

    async fn run_reconciliation_job(
        &mut self,
        report: &mut MaintenanceCycleReport,
        review_origin: MaintenanceReviewOrigin,
    ) -> Result<bool> {
        let Some(mut signal) = self
            .client
            .pending_feedback(self.config.allowed_scopes.clone(), 1)
            .await?
            .into_iter()
            .next()
        else {
            return Ok(false);
        };
        signal.challenged_revision_ids.retain(|revision_id| {
            self.ledger.eligible(&feedback_reconciliation_key(
                &signal.feedback_revision_id,
                revision_id,
            ))
        });
        if signal.challenged_revision_ids.is_empty() {
            return Ok(false);
        }
        let mut offered_revision_ids = signal.challenged_revision_ids.clone();
        offered_revision_ids.extend(signal.used_revision_ids.iter().cloned());
        offered_revision_ids.sort();
        offered_revision_ids.dedup();
        let mut feedback_pages = self
            .read_detail_pages(
                vec![signal.feedback_revision_id.clone()],
                self.config.reconciliation.max_input_chars,
            )
            .await?;
        let feedback = feedback_pages
            .pop()
            .context("feedback signal Page disappeared")?;
        let targets = self
            .read_detail_pages(
                offered_revision_ids.clone(),
                self.config.reconciliation.max_input_chars,
            )
            .await?;
        anyhow::ensure!(
            targets.len() == offered_revision_ids.len(),
            "one or more feedback target Revisions are no longer readable"
        );
        let outcome = self
            .evaluate_worker(MaintenanceWorkerRequest::ReconcileFeedback {
                signal: signal.clone(),
                feedback: Box::new(feedback.clone()),
                targets: targets.clone(),
            })
            .await?;
        report.worker_calls = report.worker_calls.saturating_add(outcome.model_attempts);
        report.escalated_decisions = report
            .escalated_decisions
            .saturating_add(u32::from(outcome.escalated));
        let model_attempts = outcome.model_attempts;
        let escalated = outcome.escalated;
        let MaintenanceWorkerResponse::ReconcileFeedback {
            target_revision_id,
            disposition,
            rationale,
            scope,
            replacement_revision_id,
        } = outcome.response
        else {
            for target_revision_id in &signal.challenged_revision_ids {
                self.ledger.record(
                    feedback_reconciliation_key(&signal.feedback_revision_id, target_revision_id),
                    "feedback_deferred",
                    self.config.reconciliation.retry_after_seconds,
                );
            }
            report.deferred += 1;
            return Ok(true);
        };
        anyhow::ensure!(
            signal.challenged_revision_ids.contains(&target_revision_id),
            "semantic worker selected a Revision that was context-only, not challenged"
        );
        let target = targets
            .iter()
            .find(|page| page.revision_id == target_revision_id)
            .context("semantic worker feedback target disappeared")?
            .clone();
        let replacement = replacement_revision_id
            .map(|revision_id| {
                anyhow::ensure!(
                    disposition == ReconciliationDisposition::Superseded,
                    "semantic worker supplied a replacement for a non-superseded decision"
                );
                anyhow::ensure!(
                    revision_id != target_revision_id,
                    "semantic worker selected the target as its own replacement"
                );
                let page = targets
                    .iter()
                    .find(|page| page.revision_id == revision_id)
                    .context(
                        "semantic worker selected a replacement outside the offered Revisions",
                    )?;
                Ok(PageRevisionRef {
                    page_id: page.page_id.clone(),
                    revision_id,
                })
            })
            .transpose()?;
        anyhow::ensure!(
            (disposition == ReconciliationDisposition::Superseded) == replacement.is_some(),
            "semantic worker returned an invalid superseded replacement"
        );
        let rationale = rationale.trim().to_owned();
        anyhow::ensure!(
            !rationale.is_empty() && rationale.chars().count() <= 2_000,
            "semantic worker returned an invalid reconciliation rationale"
        );
        let mut basis_revision_ids = vec![
            signal.feedback_revision_id.clone(),
            target_revision_id.clone(),
        ];
        if let Some(replacement) = replacement.as_ref() {
            basis_revision_ids.push(replacement.revision_id.clone());
        }
        basis_revision_ids.sort();
        basis_revision_ids.dedup();
        let candidate = MaintenanceReconciliationCandidate {
            candidate_id: MaintenanceReconciliationCandidate::candidate_id(
                &signal.feedback_revision_id,
                &target_revision_id,
            ),
            signal,
            feedback,
            target,
            disposition,
            rationale,
            scope,
            replacement,
            basis_revision_ids,
        };
        let safe_to_auto_apply = self.config.applies_changes()
            && matches!(
                candidate.signal.authority,
                FeedbackAuthority::SubjectOwner | FeedbackAuthority::TenantAssertion
            )
            && matches!(
                candidate.disposition,
                ReconciliationDisposition::NoSourceChange
                    | ReconciliationDisposition::Qualified
                    | ReconciliationDisposition::Disputed
            );
        let key = feedback_reconciliation_key(
            &candidate.signal.feedback_revision_id,
            &candidate.target.revision_id,
        );
        if safe_to_auto_apply {
            self.client
                .apply_reconciliation(reconciliation_request(
                    &candidate,
                    self.config.worker_actor(),
                    Some(format!("maintenance:{}", candidate.candidate_id)),
                    Some(self.config.worker.actor_id().to_owned()),
                ))
                .await?;
            report.reconciliations_committed += 1;
        } else {
            self.ledger.enqueue_review(
                MaintenanceReviewPayload::Reconciliation(candidate),
                review_origin,
                "Explicit feedback requires a validity decision before default recall changes."
                    .to_owned(),
                model_attempts,
                escalated,
            );
            report.reconciliations_proposed += 1;
            report.review_items_proposed += 1;
        }
        self.ledger.record(
            key,
            "feedback_reconciled_or_pending_review",
            self.config.reconciliation.retry_after_seconds,
        );
        Ok(true)
    }

    async fn run_summary_job(
        &mut self,
        inventory: &[pcp_store::DurablePageInventoryItem],
        report: &mut MaintenanceCycleReport,
        review_origin: MaintenanceReviewOrigin,
    ) -> Result<bool> {
        let eligible = |page: &&pcp_store::DurablePageInventoryItem| {
            summary_page_eligible(page, &self.config.summary)
                && self.ledger.eligible(&summary_key(&page.revision_id))
        };
        let candidate = inventory
            .iter()
            .filter(eligible)
            .find(|page| {
                page.summary_revision_id.is_some()
                    && page.summary_target_revision_id.as_deref() != Some(page.revision_id.as_str())
            })
            .or_else(|| {
                inventory
                    .iter()
                    .filter(eligible)
                    .find(|page| page.summary_revision_id.is_none())
            });
        let Some(candidate) = candidate else {
            return Ok(false);
        };
        let page_id = candidate.revision_id.clone();
        let mut pages = self
            .read_detail_pages(vec![page_id.clone()], self.config.summary.max_input_chars)
            .await?;
        let page = pages.pop().context("Summary candidate disappeared")?;
        let source_text = page.content.clone().unwrap_or_default();
        let outcome = self
            .evaluate_worker(MaintenanceWorkerRequest::SummarizePage {
                page: Box::new(page),
            })
            .await?;
        report.worker_calls = report.worker_calls.saturating_add(outcome.model_attempts);
        report.escalated_decisions = report
            .escalated_decisions
            .saturating_add(u32::from(outcome.escalated));
        let model_attempts = outcome.model_attempts;
        let escalated = outcome.escalated;
        match outcome.response {
            MaintenanceWorkerResponse::WriteSummary { content } => {
                let content = match normalize_worker_summary(content, &source_text) {
                    Ok(content) => content,
                    Err(_) => {
                        self.ledger.record(
                            summary_key(&page_id),
                            "invalid_worker_summary",
                            self.config.summary.retry_after_seconds,
                        );
                        report.deferred += 1;
                        return Ok(true);
                    }
                };
                if self.config.applies_changes() {
                    self.client
                        .write_summary(WriteSummaryRequest {
                            target_page_id: candidate.page_id.clone(),
                            target_revision_id: page_id.clone(),
                            expected_summary_revision_id: candidate.summary_revision_id.clone(),
                            content,
                            created_by: self.config.worker_actor(),
                            tool_or_model: Some(self.config.worker.actor_id().to_owned()),
                            provenance: Vec::new(),
                            idempotency_key: Some(format!("maintenance:summary:{page_id}")),
                        })
                        .await?;
                    report.summaries_written += 1;
                } else {
                    let candidate = build_summary_candidate(candidate, content);
                    self.ledger.enqueue_review(
                        MaintenanceReviewPayload::Summary(candidate),
                        review_origin,
                        "Summary metadata is ready for operator review.".to_owned(),
                        model_attempts,
                        escalated,
                    );
                    self.ledger.record(
                        summary_key(&page_id),
                        "summary_pending_review",
                        self.config.summary.retry_after_seconds,
                    );
                    report.summaries_proposed += 1;
                    report.review_items_proposed += 1;
                }
            }
            MaintenanceWorkerResponse::NoCandidate => {
                if self.config.applies_changes() {
                    self.client
                        .mark_summary_assessed(
                            page_id.clone(),
                            "not_worth_indexing".to_owned(),
                            Some(self.config.worker.actor_id().to_owned()),
                        )
                        .await?;
                } else {
                    self.ledger.record(
                        summary_key(&page_id),
                        "observed_no_candidate",
                        self.config.summary.retry_after_seconds,
                    );
                }
            }
            MaintenanceWorkerResponse::Defer => {
                self.ledger.record(
                    summary_key(&page_id),
                    "deferred",
                    self.config.summary.retry_after_seconds,
                );
                report.deferred += 1;
            }
            other => anyhow::bail!(
                "semantic worker returned {} for a summarize_page request",
                response_name(&other)
            ),
        }
        Ok(true)
    }

    async fn run_relation_job(
        &mut self,
        inventory: &[pcp_store::DurablePageInventoryItem],
        report: &mut MaintenanceCycleReport,
    ) -> Result<bool> {
        let active_packing_page_ids = self.active_packing_page_ids();
        let Some(candidates) =
            relation_candidate_windows(inventory, &self.config.relation, &active_packing_page_ids)
                .into_iter()
                .find(|pages| {
                    self.ledger.eligible(&relation_window_key(
                        &pages
                            .iter()
                            .map(|page| page.revision_id.clone())
                            .collect::<Vec<_>>(),
                    ))
                })
        else {
            return Ok(false);
        };
        let offered = candidates
            .iter()
            .map(|page| (page.page_id.clone(), page.revision_id.clone()))
            .collect::<HashMap<_, _>>();
        let window_key = relation_window_key(
            &candidates
                .iter()
                .map(|page| page.revision_id.clone())
                .collect::<Vec<_>>(),
        );
        let offered_page_ids = offered.keys().cloned().collect::<BTreeSet<_>>();
        let relation_edges = self.existing_related_pairs(&candidates).await?;
        let mut recorded_relation_pairs = self.ledger.active_relation_pairs();
        recorded_relation_pairs.extend(self.ledger.rejected_relation_pairs(&offered));
        let excluded_page_pairs = relation_excluded_page_pairs(
            &offered_page_ids,
            &relation_edges,
            &recorded_relation_pairs,
        );
        if all_relation_pairs_excluded(&offered_page_ids, &excluded_page_pairs) {
            self.ledger.record(
                window_key,
                "all_pairs_excluded",
                self.config.relation.retry_after_seconds,
            );
            return Ok(true);
        }

        let outcome = self
            .evaluate_worker(MaintenanceWorkerRequest::SelectRelation {
                pages: candidates
                    .iter()
                    .map(|page| {
                        RelationCandidatePage::from_inventory(
                            page,
                            self.config.relation.routing_chars_per_page,
                        )
                    })
                    .collect(),
                excluded_page_pairs: excluded_page_pairs.clone(),
            })
            .await?;
        report.worker_calls = report.worker_calls.saturating_add(outcome.model_attempts);
        report.escalated_decisions = report
            .escalated_decisions
            .saturating_add(u32::from(outcome.escalated));
        let model_attempts = outcome.model_attempts;
        let escalated = outcome.escalated;
        let (mut page_ids, relation_reason) = match outcome.response {
            MaintenanceWorkerResponse::Relate { page_ids, reason } => {
                (page_ids, validate_relation_reason(reason)?)
            }
            MaintenanceWorkerResponse::NoCandidate => {
                self.ledger.record(
                    window_key,
                    "no_candidate",
                    self.config.relation.retry_after_seconds,
                );
                return Ok(true);
            }
            MaintenanceWorkerResponse::Defer => {
                self.ledger.record(
                    window_key,
                    "deferred",
                    self.config.relation.retry_after_seconds,
                );
                report.deferred += 1;
                return Ok(true);
            }
            other => anyhow::bail!(
                "semantic worker returned {} for a select_relation request",
                response_name(&other)
            ),
        };
        if page_ids[0] == page_ids[1]
            || !page_ids.iter().all(|page_id| offered.contains_key(page_id))
        {
            self.ledger.record(
                window_key,
                "invalid_worker_selection",
                self.config.relation.retry_after_seconds,
            );
            report.deferred += 1;
            return Ok(true);
        }
        page_ids.sort();
        if excluded_page_pairs.iter().any(|pair| pair == &page_ids) {
            self.ledger.record(
                window_key,
                "reselected_excluded_pair",
                self.config.relation.retry_after_seconds,
            );
            report.deferred += 1;
            return Ok(true);
        }
        let pair_key = relation_pair_key(&page_ids);
        anyhow::ensure!(
            self.ledger.eligible(&pair_key),
            "semantic worker selected a relation pair still in cooldown"
        );
        let revision_ids = page_ids
            .iter()
            .map(|page_id| offered[page_id].clone())
            .collect::<Vec<_>>();
        for (page_id, revision_id) in page_ids.iter().zip(&revision_ids) {
            anyhow::ensure!(
                self.client.current_revision_id(page_id.clone()).await? == *revision_id,
                "relation candidate changed after worker evaluation"
            );
        }
        if self.related_pair_exists(&page_ids, &revision_ids).await? {
            self.ledger.record(
                pair_key,
                "already_related",
                self.config.relation.retry_after_seconds,
            );
            return Ok(true);
        }

        let selected_pages = page_ids
            .iter()
            .map(|page_id| {
                candidates
                    .iter()
                    .find(|page| &page.page_id == page_id)
                    .expect("validated relation Page is offered")
            })
            .collect::<Vec<_>>();
        let requires_review =
            !self.config.applies_changes() || !is_low_risk_automatic_relation(&selected_pages);
        if requires_review {
            let selected = page_ids
                .iter()
                .map(|page_id| {
                    let page = candidates
                        .iter()
                        .find(|page| &page.page_id == page_id)
                        .expect("validated relation Page is offered");
                    MaintenanceRelationReviewPage {
                        page_id: page.page_id.clone(),
                        revision_id: page.revision_id.clone(),
                        preview: page.snippet.clone(),
                    }
                })
                .collect::<Vec<_>>();
            self.ledger.propose_relation_review(
                candidates[0].namespace.clone(),
                [selected[0].clone(), selected[1].clone()],
                relation_reason,
                model_attempts,
                escalated,
            );
            report.relations_proposed += 1;
            report.review_items_proposed += 1;
        } else if self.config.applies_changes() {
            self.client
                .link_pages(LinkPagesRequest {
                    from_page_id: page_ids[0].clone(),
                    relation_type: "related_to".to_owned(),
                    to_page_id: page_ids[1].clone(),
                    basis_revision_ids: revision_ids.clone(),
                    created_by: self.config.worker_actor(),
                    idempotency_key: Some(format!(
                        "maintenance:related:{}:{}",
                        revision_ids[0], revision_ids[1]
                    )),
                })
                .await?;
            report.relations_committed += 1;
        } else {
            report.relations_proposed += 1;
        }
        let outcome = if requires_review {
            "relation_pending_review"
        } else if self.config.applies_changes() {
            "related"
        } else {
            "observed_relation"
        };
        self.ledger
            .record(pair_key, outcome, self.config.relation.retry_after_seconds);
        self.ledger.record(
            window_key,
            outcome,
            self.config.relation.retry_after_seconds,
        );
        Ok(true)
    }

    async fn run_topic_review_job(
        &mut self,
        inventory: &[pcp_store::DurablePageInventoryItem],
        report: &mut MaintenanceCycleReport,
        review_origin: MaintenanceReviewOrigin,
    ) -> Result<bool> {
        let active_packing_page_ids = self.active_packing_page_ids();
        let Some(window) =
            topic_candidate_windows(inventory, &self.config.relation, &active_packing_page_ids)
                .into_iter()
                .find(|pages| {
                    self.ledger.eligible(&topic_window_key(
                        &pages
                            .iter()
                            .map(|page| page.revision_id.clone())
                            .collect::<Vec<_>>(),
                    ))
                })
        else {
            return Ok(false);
        };
        let key = topic_window_key(
            &window
                .iter()
                .map(|page| page.revision_id.clone())
                .collect::<Vec<_>>(),
        );
        let offered = window
            .iter()
            .map(|page| page.page_id.clone())
            .collect::<BTreeSet<_>>();
        let existing_topics = existing_topics_for_window(inventory, &window);
        let outcome = self
            .evaluate_worker(MaintenanceWorkerRequest::ExtractTopic {
                pages: window
                    .iter()
                    .map(|page| {
                        RelationCandidatePage::from_inventory(
                            page,
                            self.config.relation.routing_chars_per_page,
                        )
                    })
                    .collect(),
                existing_topics: existing_topics.clone(),
                max_source_pages: 8,
            })
            .await?;
        report.worker_calls = report.worker_calls.saturating_add(outcome.model_attempts);
        report.escalated_decisions = report
            .escalated_decisions
            .saturating_add(u32::from(outcome.escalated));
        let model_attempts = outcome.model_attempts;
        let escalated = outcome.escalated;
        match outcome.response {
            MaintenanceWorkerResponse::ExtractTopic {
                page_ids,
                title,
                content,
                reason,
                refresh_topic_page_id,
            } => {
                anyhow::ensure!(
                    (2..=8).contains(&page_ids.len())
                        && page_ids.iter().collect::<BTreeSet<_>>().len() == page_ids.len()
                        && page_ids.iter().all(|page_id| offered.contains(page_id)),
                    "semantic worker selected invalid Topic sources"
                );
                let selected = page_ids
                    .iter()
                    .map(|page_id| {
                        window
                            .iter()
                            .find(|page| page.page_id == *page_id)
                            .expect("validated Topic Page is offered")
                    })
                    .collect::<Vec<_>>();
                let refresh_target = select_topic_refresh_target(
                    &selected,
                    &existing_topics,
                    refresh_topic_page_id.as_deref(),
                )?;
                let candidate =
                    build_topic_candidate(&selected, title, content, Some(reason), refresh_target)?;
                self.ledger.enqueue_review(
                    MaintenanceReviewPayload::Topic(candidate),
                    review_origin,
                    "A cross-Page Topic front door requires operator approval.".to_owned(),
                    model_attempts,
                    escalated,
                );
                self.ledger.record(
                    key,
                    "topic_pending_review",
                    self.config.relation.retry_after_seconds,
                );
                report.topics_proposed += 1;
                report.review_items_proposed += 1;
            }
            MaintenanceWorkerResponse::NoCandidate => {
                self.ledger.record(
                    key,
                    "no_topic_candidate",
                    self.config.relation.retry_after_seconds,
                );
            }
            MaintenanceWorkerResponse::Defer => {
                self.ledger.record(
                    key,
                    "topic_deferred",
                    self.config.relation.retry_after_seconds,
                );
                report.deferred += 1;
            }
            other => anyhow::bail!(
                "semantic worker returned {} for an extract_topic request",
                response_name(&other)
            ),
        }
        Ok(true)
    }

    async fn run_archive_review_job(
        &mut self,
        inventory: &[pcp_store::DurablePageInventoryItem],
        report: &mut MaintenanceCycleReport,
        review_origin: MaintenanceReviewOrigin,
    ) -> Result<bool> {
        let scan = archive_scan_from_inventory(inventory, Utc::now());
        let Some(scan_page) = scan
            .pages
            .iter()
            .find(|page| self.ledger.eligible(&archive_review_key(&page.revision_id)))
        else {
            return Ok(false);
        };
        let key = archive_review_key(&scan_page.revision_id);
        let mut pages = self
            .read_detail_pages(
                vec![scan_page.revision_id.clone()],
                MAX_ARCHIVE_REVIEW_INPUT_CHARS,
            )
            .await?;
        let page = pages
            .pop()
            .context("archive maintenance candidate disappeared")?;
        let outcome = self
            .evaluate_worker(MaintenanceWorkerRequest::AssessArchive {
                page: ArchiveCandidatePage {
                    page,
                    candidate_signals: scan_page.candidate_signals.clone(),
                },
            })
            .await?;
        report.worker_calls = report.worker_calls.saturating_add(outcome.model_attempts);
        report.escalated_decisions = report
            .escalated_decisions
            .saturating_add(u32::from(outcome.escalated));
        let model_attempts = outcome.model_attempts;
        let escalated = outcome.escalated;
        match outcome.response {
            MaintenanceWorkerResponse::ArchiveReview { outcome, reason } => {
                let reason = validate_archive_reason(reason)?;
                match outcome {
                    ArchiveWorkerDecision::Archive => {
                        self.ledger.enqueue_review(
                            MaintenanceReviewPayload::Archive(build_archive_candidate(
                                scan_page, reason,
                            )),
                            review_origin,
                            "Archiving is destructive governance and always requires approval."
                                .to_owned(),
                            model_attempts,
                            escalated,
                        );
                        self.ledger.record(
                            key,
                            "archive_pending_review",
                            self.config.relation.retry_after_seconds,
                        );
                        report.archives_proposed += 1;
                        report.review_items_proposed += 1;
                    }
                    ArchiveWorkerDecision::Retain => self.ledger.record(
                        key,
                        "archive_retained",
                        self.config.relation.retry_after_seconds,
                    ),
                    ArchiveWorkerDecision::Defer => {
                        self.ledger.record(
                            key,
                            "archive_deferred",
                            self.config.relation.retry_after_seconds,
                        );
                        report.deferred += 1;
                    }
                }
            }
            MaintenanceWorkerResponse::Defer => {
                self.ledger.record(
                    key,
                    "archive_deferred",
                    self.config.relation.retry_after_seconds,
                );
                report.deferred += 1;
            }
            other => anyhow::bail!(
                "semantic worker returned {} for an assess_archive request",
                response_name(&other)
            ),
        }
        Ok(true)
    }

    /// Only a concrete, still-active Pack proposal blocks relation analysis.
    ///
    /// A Page being *eligible* for packing is not a pending mutation. Excluding
    /// every such Page made manual relation review skip whole conversation
    /// streams indefinitely, including already-packed adjacent episodes.
    fn active_packing_page_ids(&self) -> BTreeSet<String> {
        self.ledger
            .active_packing_sets()
            .into_iter()
            .flatten()
            .collect()
    }

    async fn run_packing_job(
        &mut self,
        inventory: &[pcp_store::DurablePageInventoryItem],
        report: &mut MaintenanceCycleReport,
        review_origin: MaintenanceReviewOrigin,
    ) -> Result<bool> {
        let Some(candidates) = self
            .time_continuous_packing_windows(packing_candidate_windows(
                inventory,
                &self.config.packing,
            ))
            .await?
            .into_iter()
            .find(|pages| {
                self.ledger.eligible(&selection_window_key(
                    &pages
                        .iter()
                        .map(|page| page.page_id.clone())
                        .collect::<Vec<_>>(),
                ))
            })
        else {
            return Ok(false);
        };
        let routing_by_id = candidates
            .iter()
            .map(|page| (page.page_id.clone(), page.revision_id.clone()))
            .collect::<HashMap<_, _>>();
        let routing_pages = candidates
            .iter()
            .map(|page| {
                PackingCandidatePage::from_inventory(
                    page,
                    self.config.packing.routing_chars_per_page,
                )
            })
            .collect::<Vec<_>>();
        if routing_pages.len() < 2 {
            return Ok(false);
        }

        let selection_key = selection_window_key(
            &routing_pages
                .iter()
                .map(|page| page.page_id.clone())
                .collect::<Vec<_>>(),
        );
        if !self.ledger.eligible(&selection_key) {
            return Ok(false);
        }

        let excluded_candidate_sets = self.ledger.active_packing_sets();
        let outcome = self
            .evaluate_worker(MaintenanceWorkerRequest::SelectPacking {
                pages: routing_pages,
                excluded_candidate_sets: excluded_candidate_sets.clone(),
            })
            .await?;
        report.worker_calls = report.worker_calls.saturating_add(outcome.model_attempts);
        report.escalated_decisions = report
            .escalated_decisions
            .saturating_add(u32::from(outcome.escalated));
        let model_attempts = outcome.model_attempts;
        let escalated = outcome.escalated;
        let page_ids = match outcome.response {
            MaintenanceWorkerResponse::Candidate { page_ids } => page_ids,
            MaintenanceWorkerResponse::NoCandidate => {
                self.ledger
                    .record(selection_key, "no_candidate", PACKING_RETRY_AFTER_SECONDS);
                return Ok(true);
            }
            MaintenanceWorkerResponse::Defer => {
                self.ledger
                    .record(selection_key, "deferred", PACKING_RETRY_AFTER_SECONDS);
                report.deferred += 1;
                return Ok(true);
            }
            other => anyhow::bail!(
                "semantic worker returned {} for a select_packing request",
                response_name(&other)
            ),
        };
        let Some(selected_pages) =
            select_packing_items(&candidates, &page_ids, &self.config.packing)
        else {
            self.ledger.record(
                selection_key,
                "invalid_worker_selection",
                PACKING_RETRY_AFTER_SECONDS,
            );
            report.deferred += 1;
            return Ok(true);
        };
        let key = packing_key(&page_ids);
        let mut normalized_page_ids = page_ids.clone();
        normalized_page_ids.sort();
        if !self.ledger.eligible(&key)
            || excluded_candidate_sets.iter().any(|set| {
                let mut set = set.clone();
                set.sort();
                set == normalized_page_ids
            })
        {
            self.ledger.record(
                selection_key,
                "reselected_excluded_pack",
                PACKING_RETRY_AFTER_SECONDS,
            );
            report.deferred += 1;
            return Ok(true);
        }
        if self.config.applies_changes() {
            self.client
                .pack_pages(PackPagesRequest {
                    pages: page_ids
                        .iter()
                        .map(|page_id| PageRevisionRef {
                            page_id: page_id.clone(),
                            revision_id: routing_by_id[page_id].clone(),
                        })
                        .collect(),
                    idempotency_key: Some(format!(
                        "maintenance:{}",
                        key.trim_start_matches("packing:")
                    )),
                })
                .await?;
            report.packs_committed += 1;
        } else {
            self.ledger.enqueue_review(
                MaintenanceReviewPayload::Pack(build_pack_candidate(&selected_pages)),
                review_origin,
                "Pack boundary is ready for operator review.".to_owned(),
                model_attempts,
                escalated,
            );
            self.ledger
                .record(key, "pack_pending_review", PACKING_RETRY_AFTER_SECONDS);
            report.packs_proposed += 1;
            report.review_items_proposed += 1;
        }
        Ok(true)
    }

    async fn run_retention_job(&mut self, report: &mut MaintenanceCycleReport) -> Result<bool> {
        let plan = self
            .client
            .plan_revision_retention(PlanRevisionRetentionRequest {
                scopes: Vec::new(),
                policy: RetentionPolicy {
                    minimum_age_days: self.config.retention.minimum_age_days,
                    keep_recent_revisions_per_page: self
                        .config
                        .retention
                        .keep_recent_revisions_per_page,
                    sample_limit: self
                        .config
                        .retention
                        .candidate_window
                        .saturating_mul(4)
                        .clamp(1, 1_000)
                        .try_into()
                        .unwrap_or(1_000),
                },
            })
            .await?;
        let candidates = plan
            .candidates
            .into_iter()
            .filter(|candidate| {
                !excluded_kind(&candidate.kind, &self.config.retention.excluded_page_kinds)
            })
            .take(self.config.retention.candidate_window)
            .collect::<Vec<_>>();
        let revision_ids = candidates
            .iter()
            .map(|candidate| candidate.revision_id.clone())
            .collect::<Vec<_>>();
        let max_chars = self
            .config
            .retention
            .candidate_window
            .saturating_mul(self.config.retention.routing_chars_per_page.max(1))
            .try_into()
            .unwrap_or(u32::MAX);
        let mut details = self
            .read_detail_pages(revision_ids, max_chars)
            .await?
            .into_iter()
            .map(|page| (page.revision_id.clone(), page))
            .collect::<HashMap<_, _>>();
        let routing_pages = candidates
            .into_iter()
            .filter_map(|candidate| {
                details.remove(&candidate.revision_id).map(|detail| {
                    MaintenanceRoutingPage::from_detail(
                        detail,
                        candidate.kind,
                        self.config.retention.routing_chars_per_page,
                    )
                })
            })
            .collect::<Vec<_>>();
        if routing_pages.is_empty() {
            return Ok(false);
        }
        let key = retention_window_key(
            &routing_pages
                .iter()
                .map(|page| page.revision_id.clone())
                .collect::<Vec<_>>(),
        );
        if !self.ledger.eligible(&key) {
            return Ok(false);
        }
        let offered = routing_pages
            .iter()
            .map(|page| {
                (
                    page.revision_id.clone(),
                    (page.namespace.clone(), page.page_id.clone()),
                )
            })
            .collect::<HashMap<_, _>>();
        let outcome = self
            .evaluate_worker(MaintenanceWorkerRequest::SelectRetentionMilestones {
                pages: routing_pages,
                max_revisions: self.config.retention.max_revisions_per_cycle,
                lease_days: self.config.retention.lease_days,
            })
            .await?;
        report.worker_calls = report.worker_calls.saturating_add(outcome.model_attempts);
        report.escalated_decisions = report
            .escalated_decisions
            .saturating_add(u32::from(outcome.escalated));
        match outcome.response {
            MaintenanceWorkerResponse::Retain { mut milestones } => {
                normalize_milestones(
                    &mut milestones,
                    &offered,
                    self.config.retention.max_revisions_per_cycle,
                )?;
                if self.config.writes_retention_leases() {
                    let expires_at = (Utc::now()
                        + ChronoDuration::days(i64::from(self.config.retention.lease_days)))
                    .to_rfc3339_opts(SecondsFormat::Millis, true);
                    for milestone in &milestones {
                        let (namespace, _) = offered
                            .get(&milestone.revision_id)
                            .context("validated retention milestone disappeared")?;
                        self.client
                            .put_revision_retention_lease(PutRevisionRetentionLeaseRequest {
                                namespace: namespace.clone(),
                                revision_id: milestone.revision_id.clone(),
                                reason: milestone.reason.clone(),
                                expires_at: expires_at.clone(),
                                idempotency_key: format!(
                                    "maintenance:semantic-milestone:{}",
                                    milestone.revision_id
                                ),
                            })
                            .await?;
                    }
                    report.retention_leases_written = report
                        .retention_leases_written
                        .saturating_add(milestones.len().try_into().unwrap_or(u32::MAX));
                    self.ledger.record(
                        key,
                        "retention_leased",
                        self.config.retention.retry_after_seconds,
                    );
                } else {
                    report.retention_leases_proposed = report
                        .retention_leases_proposed
                        .saturating_add(milestones.len().try_into().unwrap_or(u32::MAX));
                    self.ledger.record(
                        key,
                        "observed_retention",
                        self.config.retention.retry_after_seconds,
                    );
                }
            }
            MaintenanceWorkerResponse::NoCandidate => {
                self.ledger.record(
                    key,
                    "no_retention_candidate",
                    self.config.retention.retry_after_seconds,
                );
            }
            MaintenanceWorkerResponse::Defer => {
                self.ledger.record(
                    key,
                    "retention_deferred",
                    self.config.retention.retry_after_seconds,
                );
                report.deferred += 1;
            }
            other => anyhow::bail!(
                "semantic worker returned {} for a select_retention_milestones request",
                response_name(&other)
            ),
        }
        Ok(true)
    }

    async fn read_detail_pages(
        &self,
        page_ids: Vec<String>,
        max_chars: u32,
    ) -> Result<Vec<MaintenanceDetailPage>> {
        let pages = self
            .client
            .read_pages(ReadPagesRequest {
                page_ids: Vec::new(),
                revision_ids: page_ids,
                projections: vec![
                    Projection::Manifest,
                    Projection::Summary,
                    Projection::Payload,
                    Projection::Sources,
                    Projection::Relations,
                    Projection::Facets,
                ],
                max_chars,
            })
            .await?;
        Ok(pages.into_iter().map(MaintenanceDetailPage::from).collect())
    }

    async fn related_pair_exists(
        &self,
        page_ids: &[String; 2],
        revision_ids: &[String],
    ) -> Result<bool> {
        let pages = self
            .client
            .read_pages(ReadPagesRequest {
                page_ids: Vec::new(),
                revision_ids: revision_ids.to_vec(),
                projections: vec![Projection::Manifest, Projection::Relations],
                max_chars: 1,
            })
            .await?;
        anyhow::ensure!(pages.len() == 2, "relation candidates disappeared");
        Ok(pages.iter().any(|page| {
            page.relations.iter().any(|relation| {
                relation.relation_type == "related_to"
                    && ((relation.from_page_id == page_ids[0]
                        && relation.to_page_id == page_ids[1])
                        || (relation.from_page_id == page_ids[1]
                            && relation.to_page_id == page_ids[0]))
            })
        }))
    }

    async fn existing_related_pairs(
        &self,
        candidates: &[pcp_store::DurablePageInventoryItem],
    ) -> Result<Vec<[String; 2]>> {
        let offered = candidates
            .iter()
            .map(|page| page.page_id.clone())
            .collect::<BTreeSet<_>>();
        let revision_ids = candidates
            .iter()
            .map(|page| page.revision_id.clone())
            .collect::<Vec<_>>();
        let mut pages = Vec::with_capacity(revision_ids.len());
        for chunk in revision_ids.chunks(MAINTENANCE_READ_BATCH_PAGES) {
            pages.extend(
                self.client
                    .read_pages(ReadPagesRequest {
                        page_ids: Vec::new(),
                        revision_ids: chunk.to_vec(),
                        projections: vec![Projection::Manifest, Projection::Relations],
                        max_chars: 1,
                    })
                    .await?,
            );
        }
        let mut pairs = BTreeSet::new();
        for relation in pages.iter().flat_map(|page| &page.relations) {
            if relation.relation_type != "related_to"
                || !offered.contains(&relation.from_page_id)
                || !offered.contains(&relation.to_page_id)
            {
                continue;
            }
            let mut pair = [relation.from_page_id.clone(), relation.to_page_id.clone()];
            pair.sort();
            pairs.insert(pair);
        }
        Ok(pairs.into_iter().collect())
    }

    /// A packed Page's outer revision time is only the time of its most recent
    /// rewrite. Hydrate the original entries before offering a merge candidate,
    /// so a quiet-looking container cannot hide a long interruption inside it.
    async fn time_continuous_packing_windows(
        &self,
        windows: Vec<Vec<pcp_store::DurablePageInventoryItem>>,
    ) -> Result<Vec<Vec<pcp_store::DurablePageInventoryItem>>> {
        let revision_ids = windows
            .iter()
            .flatten()
            .map(|page| page.revision_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if revision_ids.is_empty() {
            return Ok(windows);
        }
        let mut pages = Vec::with_capacity(revision_ids.len());
        // `max_chars` is a request-wide budget. Hydrate one Pack at a time so
        // an earlier large payload cannot make a later candidate look absent.
        for revision_id in revision_ids {
            pages.extend(
                self.client
                    .read_pages(ReadPagesRequest {
                        page_ids: Vec::new(),
                        revision_ids: vec![revision_id],
                        projections: vec![Projection::Payload],
                        max_chars: 64_000,
                    })
                    .await?,
            );
        }
        let entry_times = pages
            .into_iter()
            .filter_map(|page| {
                packing_page_entry_times(&page)
                    .ok()
                    .map(|times| (page.revision.revision_id, times))
            })
            .collect::<HashMap<_, _>>();
        Ok(windows
            .into_iter()
            .filter(|window| {
                let times = window
                    .iter()
                    .map(|page| entry_times.get(&page.revision_id))
                    .collect::<Option<Vec<_>>>();
                times.is_some_and(|parts| packing_times_are_continuous(parts.into_iter().flatten()))
            })
            .collect())
    }
}

/// The only automatic relation assertion allowed by the conservative policy:
/// the model has selected two adjacent Pack Pages from one source stream that
/// independently share a protected identifier. Every other model-selected
/// relation remains a revision-bound Console review proposal.
fn is_low_risk_automatic_relation(pages: &[&pcp_store::DurablePageInventoryItem]) -> bool {
    let [left, right] = pages else {
        return false;
    };
    let (Some(left_span), Some(right_span)) = (&left.source_span, &right.source_span) else {
        return false;
    };
    left.namespace == right.namespace
        && left_span.stream_id == right_span.stream_id
        && left.media_type.as_deref() == Some(PACKED_PAGE_MEDIA_TYPE)
        && right.media_type.as_deref() == Some(PACKED_PAGE_MEDIA_TYPE)
        && ((right_span.start <= left_span.end.saturating_add(1))
            || (left_span.start <= right_span.end.saturating_add(1)))
        && shares_protected_identifier(left, right)
}

fn packing_page_entry_times(page: &pcp_core::ReadPage) -> Result<Vec<DateTime<Utc>>> {
    let payload = page
        .revision
        .payload
        .as_ref()
        .context("maintenance packing candidate payload is unavailable")?;
    if payload.media_type != PACKED_PAGE_MEDIA_TYPE {
        return Ok(vec![revision_time(
            &page.revision.created_at,
            page.revision.observed_at.as_deref(),
        )?]);
    }
    let value = serde_json::from_str::<serde_json::Value>(&payload.content)
        .context("decode packed maintenance candidate")?;
    let entries = value
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .context("packed maintenance candidate has no entries")?;
    entries
        .iter()
        .map(|entry| {
            let created_at = entry
                .get("createdAt")
                .and_then(serde_json::Value::as_str)
                .context("packed maintenance entry has no createdAt")?;
            let observed_at = entry.get("observedAt").and_then(serde_json::Value::as_str);
            revision_time(created_at, observed_at)
        })
        .collect()
}

fn revision_time(created_at: &str, observed_at: Option<&str>) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(observed_at.unwrap_or(created_at))
        .map(|time| time.with_timezone(&Utc))
        .context("decode maintenance packing timestamp")
}

fn packing_times_are_continuous<'a>(times: impl Iterator<Item = &'a DateTime<Utc>>) -> bool {
    let times = times.collect::<Vec<_>>();
    !times.is_empty()
        && times.windows(2).all(|pair| {
            let gap = pair[1].signed_duration_since(*pair[0]).num_seconds();
            (0..=MAX_PACK_ENTRY_GAP_SECONDS).contains(&gap)
        })
}

fn relation_candidate_windows(
    inventory: &[pcp_store::DurablePageInventoryItem],
    config: &RelationMaintenanceConfig,
    active_packing_page_ids: &BTreeSet<String>,
) -> Vec<Vec<pcp_store::DurablePageInventoryItem>> {
    // Relation writes are namespace-local. Preserve the inventory ordering inside
    // each namespace, but never offer the worker a pair that `apply_relation_candidate`
    // would have to reject later. Apart from wasting a review round, mixed windows
    // made a semantically relevant local Page compete with unrelated Pages from a
    // different source scope.
    let mut eligible_by_namespace = BTreeMap::<String, Vec<_>>::new();
    for page in inventory.iter().filter(|page| {
        !active_packing_page_ids.contains(&page.page_id) && relation_page_eligible(page, config)
    }) {
        eligible_by_namespace
            .entry(page.namespace.clone())
            .or_default()
            .push(page.clone());
    }
    let window_size = config.candidate_window.max(2);
    let stride = (window_size / 2).max(1);
    // Tenant-declared derivation inputs are the strongest available hint that two
    // current Pages deserve relation review. Provenance remains an evidential
    // dependency, not an asserted Relation, so this only moves the exact pair to
    // the front of the candidate queue.
    let mut windows = Vec::new();
    let mut seen_windows = BTreeSet::new();
    for window in provenance_relation_windows(&eligible_by_namespace) {
        append_relation_window(&mut windows, &mut seen_windows, window);
    }
    // A Pack boundary is often where a question, correction, and its explanation
    // become separate Pages. A broad recency window can bury that pair among many
    // otherwise related conversation Pages. Offer only high-signal boundaries as
    // their own two-Page review: they must be contiguous in the same source stream,
    // both be Pack Pages, and share a protected identifier such as OET or PCP.
    // This is a candidate-generation hint, never an asserted Relation.
    for window in source_boundary_relation_windows(&eligible_by_namespace) {
        append_relation_window(&mut windows, &mut seen_windows, window);
    }
    for eligible in eligible_by_namespace.into_values() {
        let mut start = 0;
        while start < eligible.len() {
            let end = start.saturating_add(window_size).min(eligible.len());
            if end.saturating_sub(start) >= 2 {
                append_relation_window(
                    &mut windows,
                    &mut seen_windows,
                    eligible[start..end].to_vec(),
                );
            }
            if end == eligible.len() {
                break;
            }
            start = start.saturating_add(stride);
        }
    }
    windows
}

fn provenance_relation_windows(
    eligible_by_namespace: &BTreeMap<String, Vec<pcp_store::DurablePageInventoryItem>>,
) -> Vec<Vec<pcp_store::DurablePageInventoryItem>> {
    let mut windows = Vec::new();
    for pages in eligible_by_namespace.values() {
        let pages_by_revision = pages
            .iter()
            .map(|page| (page.revision_id.as_str(), page))
            .collect::<BTreeMap<_, _>>();
        for derived in pages {
            for input_revision_id in &derived.provenance_input_revision_ids {
                let Some(input) = pages_by_revision.get(input_revision_id.as_str()) else {
                    // Historical or out-of-scope inputs remain valid provenance,
                    // but relation maintenance only proposes pairs for current
                    // Page heads that are already eligible in this namespace.
                    continue;
                };
                if input.page_id != derived.page_id {
                    windows.push(vec![derived.clone(), (*input).clone()]);
                }
            }
        }
    }
    windows
}

fn append_relation_window(
    windows: &mut Vec<Vec<pcp_store::DurablePageInventoryItem>>,
    seen_windows: &mut BTreeSet<String>,
    window: Vec<pcp_store::DurablePageInventoryItem>,
) {
    let key = relation_window_key(
        &window
            .iter()
            .map(|page| page.revision_id.clone())
            .collect::<Vec<_>>(),
    );
    if seen_windows.insert(key) {
        windows.push(window);
    }
}

fn source_boundary_relation_windows(
    eligible_by_namespace: &BTreeMap<String, Vec<pcp_store::DurablePageInventoryItem>>,
) -> Vec<Vec<pcp_store::DurablePageInventoryItem>> {
    let mut pages_by_stream = BTreeMap::<(String, String), Vec<_>>::new();
    for (namespace, pages) in eligible_by_namespace {
        for page in pages {
            let Some(span) = &page.source_span else {
                continue;
            };
            if page.media_type.as_deref() != Some(PACKED_PAGE_MEDIA_TYPE) {
                continue;
            }
            pages_by_stream
                .entry((namespace.clone(), span.stream_id.clone()))
                .or_default()
                .push(page.clone());
        }
    }

    let mut windows = Vec::new();
    for pages in pages_by_stream.values_mut() {
        pages.sort_by(|left, right| {
            left.source_span
                .as_ref()
                .expect("source stream page has a source span")
                .start
                .cmp(
                    &right
                        .source_span
                        .as_ref()
                        .expect("source stream page has a source span")
                        .start,
                )
                .then_with(|| left.page_id.cmp(&right.page_id))
        });
        for pair in pages.windows(2) {
            let [left, right] = pair else {
                continue;
            };
            let left_span = left
                .source_span
                .as_ref()
                .expect("source stream page has a source span");
            let right_span = right
                .source_span
                .as_ref()
                .expect("source stream page has a source span");
            if right_span.start <= left_span.end.saturating_add(1)
                && shares_protected_identifier(left, right)
            {
                windows.push(vec![left.clone(), right.clone()]);
            }
        }
    }
    windows
}

fn shares_protected_identifier(
    left: &pcp_store::DurablePageInventoryItem,
    right: &pcp_store::DurablePageInventoryItem,
) -> bool {
    let protected_identifiers = |page: &pcp_store::DurablePageInventoryItem| {
        [page.summary.as_deref(), Some(page.snippet.as_str())]
            .into_iter()
            .flatten()
            .flat_map(ascii_identifier_tokens)
            .filter(|identifier| is_protected_identifier(identifier))
            .map(|identifier| identifier.to_ascii_lowercase())
            .collect::<BTreeSet<_>>()
    };
    let left = protected_identifiers(left);
    let right = protected_identifiers(right);
    !left.is_empty() && !left.is_disjoint(&right)
}

fn relation_window_key(revision_ids: &[String]) -> String {
    let mut revision_ids = revision_ids.to_vec();
    revision_ids.sort();
    revision_ids.dedup();
    format!("relation_window:{}", revision_ids.join(","))
}

fn topic_window_key(revision_ids: &[String]) -> String {
    let mut revision_ids = revision_ids.to_vec();
    revision_ids.sort();
    revision_ids.dedup();
    format!("topic_window:{}", revision_ids.join(","))
}

fn archive_review_key(revision_id: &str) -> String {
    format!("archive_review:{revision_id}")
}

fn relation_pair_key(page_ids: &[String; 2]) -> String {
    format!("relation_pair:{},{}", page_ids[0], page_ids[1])
}

fn connected_relation_pairs(
    offered_page_ids: &BTreeSet<String>,
    relation_edges: &[[String; 2]],
) -> Vec<[String; 2]> {
    let mut components = offered_page_ids
        .iter()
        .map(|page_id| (page_id.clone(), page_id.clone()))
        .collect::<BTreeMap<_, _>>();
    for edge in relation_edges {
        if !offered_page_ids.contains(&edge[0]) || !offered_page_ids.contains(&edge[1]) {
            continue;
        }
        let left = components[&edge[0]].clone();
        let right = components[&edge[1]].clone();
        if left == right {
            continue;
        }
        let (keep, replace) = if left < right {
            (left, right)
        } else {
            (right, left)
        };
        for component in components.values_mut() {
            if *component == replace {
                *component = keep.clone();
            }
        }
    }

    let page_ids = offered_page_ids.iter().collect::<Vec<_>>();
    let mut connected = Vec::new();
    for (index, first) in page_ids.iter().enumerate() {
        for second in &page_ids[index + 1..] {
            if components[*first] == components[*second] {
                connected.push([(*first).clone(), (*second).clone()]);
            }
        }
    }
    connected
}

fn relation_excluded_page_pairs(
    offered_page_ids: &BTreeSet<String>,
    relation_edges: &[[String; 2]],
    suppressed_pairs: &[[String; 2]],
) -> Vec<[String; 2]> {
    // Existing asserted relations exclude their whole connected component.  An
    // operator suppression is intentionally narrower: it excludes only the
    // exact two Pages they reviewed, never unrelated pairs reached through a
    // graph path.
    let mut excluded = connected_relation_pairs(offered_page_ids, relation_edges);
    excluded.extend(suppressed_pairs.iter().filter_map(|pair| {
        (offered_page_ids.contains(&pair[0]) && offered_page_ids.contains(&pair[1]))
            .then(|| normalized_relation_pair(pair.clone()))
    }));
    excluded.sort();
    excluded.dedup();
    excluded
}

fn all_relation_pairs_excluded(
    offered_page_ids: &BTreeSet<String>,
    excluded_page_pairs: &[[String; 2]],
) -> bool {
    let page_ids = offered_page_ids.iter().collect::<Vec<_>>();
    page_ids.len() >= 2
        && page_ids.iter().enumerate().all(|(index, first)| {
            page_ids[index + 1..]
                .iter()
                .all(|second| excluded_page_pairs.contains(&[(*first).clone(), (*second).clone()]))
        })
}

fn normalized_relation_pair(mut pair: [String; 2]) -> [String; 2] {
    pair.sort();
    pair
}

fn packing_scan_from_windows(
    inventory: &[pcp_store::DurablePageInventoryItem],
    windows: &[Vec<pcp_store::DurablePageInventoryItem>],
    config: &PackingMaintenanceConfig,
    captured_at: &str,
) -> MaintenancePackScan {
    let eligible_pages = windows.iter().map(Vec::len).sum::<usize>();
    let scan_id = packing_scan_id(&windows, config);
    let groups = windows
        .iter()
        .map(|window| packing_scan_group(window))
        .collect();
    MaintenancePackScan {
        captured_at: captured_at.to_owned(),
        scan_id,
        inspected_pages: inventory.len(),
        eligible_pages,
        excluded_pages: inventory.len().saturating_sub(eligible_pages),
        candidate_group_count: windows.len(),
        estimated_model_calls: windows.len().div_ceil(PACKING_GROUPS_PER_MODEL_CALL),
        groups,
    }
}

fn summary_scan_from_inventory(
    inventory: &[pcp_store::DurablePageInventoryItem],
    config: &super::SummaryMaintenanceConfig,
) -> MaintenanceSummaryScan {
    let pages = if config.enabled {
        inventory
            .iter()
            .filter(|page| {
                summary_page_eligible(page, config)
                    && page.summary_target_revision_id.as_deref() != Some(page.revision_id.as_str())
            })
            .map(|page| MaintenanceSummaryScanPage {
                page_id: page.page_id.clone(),
                revision_id: page.revision_id.clone(),
                namespace: page.namespace.clone(),
                content_chars: page.content_chars,
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let scan_id = summary_scan_id(&pages, config);
    MaintenanceSummaryScan {
        scan_id,
        inspected_pages: inventory.len(),
        eligible_pages: pages.len(),
        estimated_model_calls: pages.len().div_ceil(SUMMARY_REVIEW_PAGES_PER_MODEL_CALL),
        pages,
    }
}

fn archive_scan_from_inventory(
    inventory: &[pcp_store::DurablePageInventoryItem],
    now: DateTime<Utc>,
) -> MaintenanceArchiveScan {
    let minimum_observed_at = now - ChronoDuration::days(ARCHIVE_MINIMUM_AGE_DAYS);
    let mut pages = inventory
        .iter()
        .filter_map(|page| archive_scan_page(page, minimum_observed_at))
        .collect::<Vec<_>>();
    pages.sort_by(|left, right| {
        left.observed_at
            .cmp(&right.observed_at)
            .then_with(|| left.page_id.cmp(&right.page_id))
    });
    pages.truncate(MAX_ARCHIVE_CANDIDATES);
    let scan_id = archive_scan_id(&pages);
    MaintenanceArchiveScan {
        captured_at: now.to_rfc3339_opts(SecondsFormat::Millis, true),
        scan_id,
        inspected_pages: inventory.len(),
        eligible_pages: pages.len(),
        estimated_model_calls: pages.len(),
        pages,
    }
}

fn archive_scan_page(
    page: &pcp_store::DurablePageInventoryItem,
    minimum_observed_at: DateTime<Utc>,
) -> Option<MaintenanceArchiveScanPage> {
    // Packing and Topic Pages are structural routing objects, not archive
    // candidates. Other links and summaries are review evidence, not an
    // automatic retention veto: an old transient Page can still be safely
    // archived after the worker and a human inspect that evidence.
    // PCP has no per-Page read metrics, so eligibility must never imply a
    // measurement of real-world usage or value.
    if page.media_type.as_deref() == Some(PACKED_PAGE_MEDIA_TYPE)
        || page.kind == "topic_summary"
        || page.packing_protected
    {
        return None;
    }
    let observed_at = page.observed_at.as_deref().unwrap_or(&page.created_at);
    let observed_at = DateTime::parse_from_rfc3339(observed_at)
        .ok()?
        .with_timezone(&Utc);
    if observed_at > minimum_observed_at {
        return None;
    }
    let mut candidate_signals = vec![format!("older_than_{ARCHIVE_MINIMUM_AGE_DAYS}_days")];
    if page.summary_target_revision_id.as_deref() == Some(page.revision_id.as_str()) {
        candidate_signals.push("has_current_routing_summary".to_owned());
    } else {
        candidate_signals.push("no_current_routing_summary".to_owned());
    }
    if page.relation_types.is_empty() {
        candidate_signals.push("no_explicit_relations".to_owned());
    } else {
        candidate_signals.push(format!(
            "explicit_relations:{}",
            page.relation_types.join(",")
        ));
    }
    Some(MaintenanceArchiveScanPage {
        page_id: page.page_id.clone(),
        revision_id: page.revision_id.clone(),
        namespace: page.namespace.clone(),
        kind: page.kind.clone(),
        observed_at: observed_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        content_chars: page.content_chars,
        preview: page.snippet.chars().take(PACKING_PREVIEW_CHARS).collect(),
        candidate_signals,
    })
}

fn archive_scan_id(pages: &[MaintenanceArchiveScanPage]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"content-governance-archive-v1");
    digest.update(ARCHIVE_MINIMUM_AGE_DAYS.to_le_bytes());
    for page in pages {
        digest.update(page.page_id.as_bytes());
        digest.update([0]);
        digest.update(page.revision_id.as_bytes());
        digest.update([0]);
    }
    let encoded = format!("{:x}", digest.finalize());
    format!("mar_{}", &encoded[..24])
}

fn relation_scan_from_inventory(
    inventory: &[pcp_store::DurablePageInventoryItem],
    config: &RelationMaintenanceConfig,
    active_packing_page_ids: &BTreeSet<String>,
) -> MaintenanceRelationScan {
    let eligible_pages = if config.enabled {
        inventory
            .iter()
            .filter(|page| {
                !active_packing_page_ids.contains(&page.page_id)
                    && relation_page_eligible(page, config)
            })
            .count()
    } else {
        0
    };
    let windows = if config.enabled {
        relation_candidate_windows(inventory, config, active_packing_page_ids)
    } else {
        Vec::new()
    };
    let groups = windows
        .iter()
        .map(|window| {
            let anchor = window.first().expect("relation group is non-empty");
            MaintenanceRelationScanGroup {
                group_id: relation_scan_group_id(window),
                anchor_page_id: anchor.page_id.clone(),
                anchor_revision_id: anchor.revision_id.clone(),
                page_count: window.len(),
            }
        })
        .collect::<Vec<_>>();
    MaintenanceRelationScan {
        scan_id: relation_scan_id(&groups, config),
        inspected_pages: inventory.len(),
        eligible_pages,
        candidate_group_count: groups.len(),
        estimated_model_calls: groups.len(),
        groups,
    }
}

fn topic_scan_from_inventory(
    inventory: &[pcp_store::DurablePageInventoryItem],
    config: &RelationMaintenanceConfig,
    active_packing_page_ids: &BTreeSet<String>,
) -> MaintenanceTopicScan {
    let windows = if config.enabled {
        topic_candidate_windows(inventory, config, active_packing_page_ids)
    } else {
        Vec::new()
    };
    let groups = windows
        .iter()
        .map(|window| MaintenanceTopicScanGroup {
            group_id: topic_scan_group_id(window),
            page_count: window.len(),
        })
        .collect::<Vec<_>>();
    let eligible_pages = if config.enabled {
        inventory
            .iter()
            .filter(|page| {
                !active_packing_page_ids.contains(&page.page_id)
                    && relation_page_eligible(page, config)
                    && page.kind != "topic_summary"
            })
            .count()
    } else {
        0
    };
    MaintenanceTopicScan {
        scan_id: topic_scan_id(&groups),
        inspected_pages: inventory.len(),
        eligible_pages,
        candidate_group_count: groups.len(),
        estimated_model_calls: groups.len(),
        groups,
    }
}

fn topic_candidate_windows(
    inventory: &[pcp_store::DurablePageInventoryItem],
    config: &RelationMaintenanceConfig,
    active_packing_page_ids: &BTreeSet<String>,
) -> Vec<Vec<pcp_store::DurablePageInventoryItem>> {
    // Reuse namespace-local structural windows, but make them deliberately
    // smaller than relation reviews. They are prompts for semantic judgment,
    // never a claim that adjacent Pages constitute a Topic.
    let mut seen = BTreeSet::new();
    relation_candidate_windows(inventory, config, active_packing_page_ids)
        .into_iter()
        .filter_map(|window| {
            let pages = window
                .into_iter()
                .filter(|page| page.kind != "topic_summary")
                .take(8)
                .collect::<Vec<_>>();
            (pages.len() >= 2 && seen.insert(topic_scan_group_id(&pages))).then_some(pages)
        })
        .collect()
}

fn existing_topics_for_window(
    inventory: &[pcp_store::DurablePageInventoryItem],
    window: &[pcp_store::DurablePageInventoryItem],
) -> Vec<ExistingTopicPage> {
    let selected = window.iter().collect::<Vec<_>>();
    existing_topics_for_selected(inventory, &selected)
}

fn existing_topics_for_selected(
    inventory: &[pcp_store::DurablePageInventoryItem],
    selected: &[&pcp_store::DurablePageInventoryItem],
) -> Vec<ExistingTopicPage> {
    let Some(namespace) = selected.first().map(|page| page.namespace.as_str()) else {
        return Vec::new();
    };
    let selected_page_ids = selected
        .iter()
        .map(|page| page.page_id.as_str())
        .collect::<BTreeSet<_>>();

    inventory
        .iter()
        .filter(|page| {
            page.namespace == namespace
                && page.kind == "topic_summary"
                && !page.superseded
                && page
                    .topic_source_page_ids
                    .iter()
                    .filter(|page_id| selected_page_ids.contains(page_id.as_str()))
                    .take(2)
                    .count()
                    >= 2
        })
        .map(|page| ExistingTopicPage {
            page_id: page.page_id.clone(),
            revision_id: page.revision_id.clone(),
            title: page
                .facets
                .as_ref()
                .and_then(|facets| facets.get("topicTitle"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Topic")
                .to_owned(),
            routing_text: page
                .summary
                .as_deref()
                .unwrap_or(&page.snippet)
                .chars()
                .take(PACKING_PREVIEW_CHARS)
                .collect(),
            source_page_ids: page.topic_source_page_ids.clone(),
        })
        .collect()
}

fn select_topic_refresh_target(
    selected: &[&pcp_store::DurablePageInventoryItem],
    existing_topics: &[ExistingTopicPage],
    requested_page_id: Option<&str>,
) -> Result<Option<MaintenanceTopicRefreshTarget>> {
    let selected_page_ids = selected
        .iter()
        .map(|page| page.page_id.as_str())
        .collect::<BTreeSet<_>>();
    let selected_topic = if let Some(requested_page_id) = requested_page_id {
        Some(
            existing_topics
                .iter()
                .find(|topic| topic.page_id == requested_page_id)
                .context("semantic worker selected an unavailable Topic refresh target")?,
        )
    } else {
        existing_topics.iter().find(|topic| {
            topic
                .source_page_ids
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
                == selected_page_ids
        })
    };
    let Some(topic) = selected_topic else {
        return Ok(None);
    };
    let topic_source_ids = topic
        .source_page_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let shared_source_page_count = topic_source_ids.intersection(&selected_page_ids).count();
    let union_source_page_count = topic_source_ids.union(&selected_page_ids).count();
    anyhow::ensure!(
        shared_source_page_count >= 2
            && shared_source_page_count.saturating_mul(2) >= union_source_page_count,
        "Topic refresh target does not substantially overlap the selected source Pages"
    );
    Ok(Some(MaintenanceTopicRefreshTarget {
        page_id: topic.page_id.clone(),
        revision_id: topic.revision_id.clone(),
        title: topic.title.clone(),
        preview: topic.routing_text.clone(),
        source_page_count: topic.source_page_ids.len(),
        shared_source_page_count,
    }))
}

fn topic_refresh_target_from_request(
    inventory: &[pcp_store::DurablePageInventoryItem],
    selected: &[&pcp_store::DurablePageInventoryItem],
    requested: Option<&PageRevisionRef>,
) -> Result<Option<MaintenanceTopicRefreshTarget>> {
    let existing_topics = existing_topics_for_selected(inventory, selected);
    let target = select_topic_refresh_target(
        selected,
        &existing_topics,
        requested.map(|target| target.page_id.as_str()),
    )?;
    if let (Some(requested), Some(target)) = (requested, target.as_ref()) {
        anyhow::ensure!(
            requested.revision_id == target.revision_id,
            "maintenance Topic refresh target is stale"
        );
    }
    Ok(target)
}

fn topic_scan_group_id(window: &[pcp_store::DurablePageInventoryItem]) -> String {
    let mut digest = Sha256::new();
    for page in window {
        digest.update(page.page_id.as_bytes());
        digest.update([0]);
        digest.update(page.revision_id.as_bytes());
        digest.update([0]);
    }
    let encoded = format!("{:x}", digest.finalize());
    format!("mtg_{}", &encoded[..24])
}

fn topic_scan_id(groups: &[MaintenanceTopicScanGroup]) -> String {
    let mut digest = Sha256::new();
    for group in groups {
        digest.update(group.group_id.as_bytes());
        digest.update([0]);
    }
    let encoded = format!("{:x}", digest.finalize());
    format!("mts_{}", &encoded[..24])
}

fn summary_scan_id(
    pages: &[MaintenanceSummaryScanPage],
    config: &super::SummaryMaintenanceConfig,
) -> String {
    let mut digest = Sha256::new();
    digest.update(config.minimum_chars.to_le_bytes());
    digest.update(config.max_input_chars.to_le_bytes());
    for page in pages {
        digest.update(page.page_id.as_bytes());
        digest.update([0]);
        digest.update(page.revision_id.as_bytes());
        digest.update([0]);
    }
    let encoded = format!("{:x}", digest.finalize());
    format!("mss_{}", &encoded[..24])
}

fn relation_scan_id(
    groups: &[MaintenanceRelationScanGroup],
    config: &RelationMaintenanceConfig,
) -> String {
    let mut digest = Sha256::new();
    digest.update(config.candidate_window.to_le_bytes());
    digest.update(config.routing_chars_per_page.to_le_bytes());
    for group in groups {
        digest.update(group.group_id.as_bytes());
        digest.update([0]);
    }
    let encoded = format!("{:x}", digest.finalize());
    format!("mrs_{}", &encoded[..24])
}

fn relation_scan_group_id(window: &[pcp_store::DurablePageInventoryItem]) -> String {
    let mut digest = Sha256::new();
    for page in window {
        digest.update(page.page_id.as_bytes());
        digest.update([0]);
        digest.update(page.revision_id.as_bytes());
        digest.update([0]);
    }
    let encoded = format!("{:x}", digest.finalize());
    format!("mrg_{}", &encoded[..24])
}

fn packing_scan_id(
    windows: &[Vec<pcp_store::DurablePageInventoryItem>],
    config: &PackingMaintenanceConfig,
) -> String {
    let mut digest = Sha256::new();
    digest.update(config.max_pages.to_le_bytes());
    digest.update(config.max_input_chars.to_le_bytes());
    digest.update(config.effective_analysis_window_pages().to_le_bytes());
    digest.update(config.routing_chars_per_page.to_le_bytes());
    for window in windows {
        digest.update(packing_scan_group_id(window).as_bytes());
        digest.update([0]);
    }
    let encoded = format!("{:x}", digest.finalize());
    format!("mps_{}", &encoded[..24])
}

fn packing_scan_group_id(window: &[pcp_store::DurablePageInventoryItem]) -> String {
    let mut digest = Sha256::new();
    for page in window {
        digest.update(page.page_id.as_bytes());
        digest.update([0]);
        digest.update(page.revision_id.as_bytes());
        digest.update([0]);
    }
    let encoded = format!("{:x}", digest.finalize());
    format!("mpg_{}", &encoded[..24])
}

fn packing_scan_group(window: &[pcp_store::DurablePageInventoryItem]) -> MaintenancePackScanGroup {
    let first = window.first().expect("packing group is non-empty");
    let last = window.last().expect("packing group is non-empty");
    let first_span = first
        .source_span
        .as_ref()
        .expect("packing group has sourceSpan");
    let last_span = last
        .source_span
        .as_ref()
        .expect("packing group has sourceSpan");
    MaintenancePackScanGroup {
        group_id: packing_scan_group_id(window),
        namespace: first.namespace.clone(),
        kind: first.kind.clone(),
        source_span: SourceSpan {
            stream_id: first_span.stream_id.clone(),
            start: first_span.start,
            end: last_span.end,
        },
        page_count: window.len(),
        content_chars: window.iter().map(|page| page.content_chars).sum(),
        extends_existing_pack: window
            .iter()
            .any(|page| page.media_type.as_deref() == Some(PACKED_PAGE_MEDIA_TYPE)),
    }
}

fn validate_packing_analysis_batch(
    windows: &[Vec<pcp_store::DurablePageInventoryItem>],
    selected_sets: Vec<Vec<String>>,
    config: &PackingMaintenanceConfig,
) -> Result<(Vec<MaintenancePackCandidate>, usize)> {
    anyhow::ensure!(
        !selected_sets.is_empty(),
        "semantic worker returned an empty packing candidate list"
    );
    let mut used_page_ids = BTreeSet::new();
    let mut represented_groups = BTreeSet::new();
    let mut candidates = Vec::with_capacity(selected_sets.len());

    for page_ids in selected_sets {
        anyhow::ensure!(
            page_ids
                .iter()
                .all(|page_id| used_page_ids.insert(page_id.clone())),
            "semantic worker returned overlapping packing candidates"
        );
        let matches = windows
            .iter()
            .enumerate()
            .filter_map(|(index, window)| {
                select_packing_items(window, &page_ids, config).map(|pages| (index, pages))
            })
            .collect::<Vec<_>>();
        anyhow::ensure!(
            matches.len() == 1,
            "semantic worker returned a packing candidate outside one supplied group"
        );
        let (group_index, pages) = matches.into_iter().next().expect("one matching group");
        represented_groups.insert(group_index);
        candidates.push(build_pack_candidate(&pages));
    }
    Ok((candidates, represented_groups.len()))
}

fn packing_candidates_overlap(selected_sets: &[Vec<String>]) -> bool {
    let mut used_page_ids = BTreeSet::new();
    selected_sets
        .iter()
        .flatten()
        .any(|page_id| !used_page_ids.insert(page_id))
}

fn select_packing_items<'a>(
    candidates: &'a [pcp_store::DurablePageInventoryItem],
    page_ids: &[String],
    config: &PackingMaintenanceConfig,
) -> Option<Vec<&'a pcp_store::DurablePageInventoryItem>> {
    if !(2..=config.max_pages).contains(&page_ids.len()) {
        return None;
    }
    let positions = page_ids
        .iter()
        .map(|page_id| {
            candidates
                .iter()
                .position(|candidate| candidate.page_id == *page_id)
        })
        .collect::<Option<Vec<_>>>()?;
    if !positions.windows(2).all(|pair| pair[0] + 1 == pair[1]) {
        return None;
    }
    let selected = positions
        .into_iter()
        .map(|position| &candidates[position])
        .collect::<Vec<_>>();
    let content_chars = selected.iter().fold(0_u64, |total, page| {
        total.saturating_add(page.content_chars)
    });
    let packed_pages = selected
        .iter()
        .filter(|page| page.media_type.as_deref() == Some(PACKED_PAGE_MEDIA_TYPE))
        .count();
    (content_chars <= u64::from(config.max_input_chars) && packed_pages <= 2).then_some(selected)
}

fn validate_pack_application<'a>(
    inventory: &'a [pcp_store::DurablePageInventoryItem],
    request: &ApplyMaintenancePackRequest,
    config: &PackingMaintenanceConfig,
) -> Result<MaintenancePackCandidate> {
    anyhow::ensure!(
        (2..=config.max_pages).contains(&request.pages.len()),
        "maintenance Pack candidate is stale or no longer eligible"
    );
    let mut unique_page_ids = BTreeSet::new();
    anyhow::ensure!(
        request
            .pages
            .iter()
            .all(|page| unique_page_ids.insert(page.page_id.as_str())),
        "maintenance Pack candidate contains duplicate Pages"
    );
    let current_by_id = inventory
        .iter()
        .map(|page| (page.page_id.as_str(), page))
        .collect::<HashMap<_, _>>();
    let mut selected = Vec::with_capacity(request.pages.len());
    for requested in &request.pages {
        let current = current_by_id
            .get(requested.page_id.as_str())
            .copied()
            .context("maintenance Pack candidate is stale or no longer eligible")?;
        anyhow::ensure!(
            current.revision_id == requested.revision_id,
            "maintenance Pack candidate revisions changed after analysis"
        );
        anyhow::ensure!(
            packing_page_eligible(current, config),
            "maintenance Pack candidate is stale or no longer eligible"
        );
        selected.push(current);
    }

    let first = selected.first().expect("validated Pack has Pages");
    let first_span = first
        .source_span
        .as_ref()
        .expect("eligible Pack Page has sourceSpan");
    anyhow::ensure!(
        selected.iter().all(|page| {
            let span = page
                .source_span
                .as_ref()
                .expect("eligible Pack Page has sourceSpan");
            page.namespace == first.namespace
                && page.kind == first.kind
                && span.stream_id == first_span.stream_id
        }) && selected.windows(2).all(|pair| {
            let previous = pair[0]
                .source_span
                .as_ref()
                .expect("eligible Pack Page has sourceSpan");
            let current = pair[1]
                .source_span
                .as_ref()
                .expect("eligible Pack Page has sourceSpan");
            previous.end.checked_add(1) == Some(current.start)
        }),
        "maintenance Pack candidate is stale or no longer eligible"
    );
    let content_chars = selected.iter().fold(0_u64, |total, page| {
        total.saturating_add(page.content_chars)
    });
    let packed_pages = selected
        .iter()
        .filter(|page| page.media_type.as_deref() == Some(PACKED_PAGE_MEDIA_TYPE))
        .count();
    anyhow::ensure!(
        content_chars <= u64::from(config.max_input_chars) && packed_pages <= 2,
        "maintenance Pack candidate is stale or no longer eligible"
    );

    let candidate = build_pack_candidate(&selected);
    anyhow::ensure!(
        candidate.candidate_id == request.candidate_id,
        "maintenance Pack candidate identity no longer matches the analyzed Pages"
    );
    Ok(candidate)
}

fn build_pack_candidate(
    pages: &[&pcp_store::DurablePageInventoryItem],
) -> MaintenancePackCandidate {
    let first = pages.first().expect("packing selection is non-empty");
    let last = pages.last().expect("packing selection is non-empty");
    let first_span = first
        .source_span
        .as_ref()
        .expect("packing candidate has sourceSpan");
    let last_span = last
        .source_span
        .as_ref()
        .expect("packing candidate has sourceSpan");
    let source_span = SourceSpan {
        stream_id: first_span.stream_id.clone(),
        start: first_span.start,
        end: last_span.end,
    };
    let mut digest = Sha256::new();
    for page in pages {
        digest.update(page.page_id.as_bytes());
        digest.update([0]);
        digest.update(page.revision_id.as_bytes());
        digest.update([0]);
    }
    let encoded = format!("{:x}", digest.finalize());
    let candidate_id = format!("mpc_{}", &encoded[..24]);
    MaintenancePackCandidate {
        candidate_id,
        namespace: first.namespace.clone(),
        kind: first.kind.clone(),
        resulting_entry_count: source_span
            .end
            .saturating_sub(source_span.start)
            .saturating_add(1),
        source_span,
        input_page_count: pages.len(),
        content_chars: pages.iter().map(|page| page.content_chars).sum::<u64>(),
        extends_existing_pack: pages
            .iter()
            .any(|page| page.media_type.as_deref() == Some(PACKED_PAGE_MEDIA_TYPE)),
        pages: pages
            .iter()
            .map(|page| MaintenancePackInput {
                page_id: page.page_id.clone(),
                revision_id: page.revision_id.clone(),
                source_span: page
                    .source_span
                    .clone()
                    .expect("packing candidate has sourceSpan"),
                media_type: page.media_type.clone(),
                preview: page.snippet.chars().take(PACKING_PREVIEW_CHARS).collect(),
            })
            .collect(),
    }
}

fn packing_candidate_windows(
    inventory: &[pcp_store::DurablePageInventoryItem],
    config: &PackingMaintenanceConfig,
) -> Vec<Vec<pcp_store::DurablePageInventoryItem>> {
    let mut windows = packing_merge_candidate_windows(inventory, config);
    let mut visited = std::collections::HashSet::new();
    for seed in inventory
        .iter()
        .filter(|page| packing_page_eligible(page, config))
    {
        let Some(span) = seed.source_span.as_ref() else {
            continue;
        };
        let key = (
            seed.namespace.clone(),
            seed.kind.clone(),
            span.stream_id.clone(),
        );
        if !visited.insert(key.clone()) {
            continue;
        }
        let mut group = inventory
            .iter()
            .filter(|page| packing_page_eligible(page, config))
            .filter(|page| {
                let span = page.source_span.as_ref().expect("eligible sourceSpan");
                page.namespace == key.0 && page.kind == key.1 && span.stream_id == key.2
            })
            .cloned()
            .collect::<Vec<_>>();
        group.sort_by_key(|page| {
            page.source_span
                .as_ref()
                .expect("eligible sourceSpan")
                .start
        });

        let mut run = Vec::new();
        for page in group {
            if page.content_chars > u64::from(config.max_input_chars) {
                push_packing_window(&mut windows, &mut run);
                continue;
            }
            let contiguous =
                run.last()
                    .is_none_or(|previous: &pcp_store::DurablePageInventoryItem| {
                        let previous = previous.source_span.as_ref().expect("eligible sourceSpan");
                        let current = page.source_span.as_ref().expect("eligible sourceSpan");
                        previous.end.checked_add(1) == Some(current.start)
                    });
            let fits = run.len() < config.effective_analysis_window_pages();
            if !contiguous || !fits {
                push_packing_window(&mut windows, &mut run);
            }
            run.push(page);
        }
        push_packing_window(&mut windows, &mut run);
    }
    windows
}

fn packing_merge_candidate_windows(
    inventory: &[pcp_store::DurablePageInventoryItem],
    config: &PackingMaintenanceConfig,
) -> Vec<Vec<pcp_store::DurablePageInventoryItem>> {
    let mut pages_by_stream = BTreeMap::<(String, String, String), Vec<_>>::new();
    for page in inventory.iter().filter(|page| {
        packing_page_eligible(page, config)
            && page.media_type.as_deref() == Some(PACKED_PAGE_MEDIA_TYPE)
    }) {
        let span = page
            .source_span
            .as_ref()
            .expect("eligible Pack has sourceSpan");
        pages_by_stream
            .entry((
                page.namespace.clone(),
                page.kind.clone(),
                span.stream_id.clone(),
            ))
            .or_default()
            .push(page.clone());
    }

    let mut windows = Vec::new();
    for pages in pages_by_stream.values_mut() {
        pages.sort_by_key(|page| {
            page.source_span
                .as_ref()
                .expect("eligible Pack has sourceSpan")
                .start
        });
        for pair in pages.windows(2) {
            let [left, right] = pair else {
                continue;
            };
            let left_span = left
                .source_span
                .as_ref()
                .expect("eligible Pack has sourceSpan");
            let right_span = right
                .source_span
                .as_ref()
                .expect("eligible Pack has sourceSpan");
            if left_span.end.checked_add(1) == Some(right_span.start)
                && shares_protected_identifier(left, right)
            {
                windows.push(vec![left.clone(), right.clone()]);
            }
        }
    }
    windows
}

fn packing_page_eligible(
    page: &pcp_store::DurablePageInventoryItem,
    config: &PackingMaintenanceConfig,
) -> bool {
    let packed = page.media_type.as_deref() == Some(PACKED_PAGE_MEDIA_TYPE);
    let valid_shape = if packed {
        page.mutability == PageMutability::Revisioned
    } else {
        page.mutability == PageMutability::Sealed
            && !page.packing_protected
            && page.summary_revision_id.is_none()
    };
    valid_shape
        && page.source_span.is_some()
        && page.content_chars > 0
        && !excluded_kind(&page.kind, &config.excluded_page_kinds)
}

fn push_packing_window(
    windows: &mut Vec<Vec<pcp_store::DurablePageInventoryItem>>,
    run: &mut Vec<pcp_store::DurablePageInventoryItem>,
) {
    if run.len() >= 2
        && run
            .iter()
            .any(|page| page.media_type.as_deref() != Some(PACKED_PAGE_MEDIA_TYPE))
    {
        windows.push(std::mem::take(run));
    } else {
        run.clear();
    }
}

fn normalize_milestones(
    milestones: &mut Vec<RetentionMilestone>,
    offered: &HashMap<String, (String, String)>,
    maximum: usize,
) -> Result<()> {
    milestones.sort_by(|left, right| left.revision_id.cmp(&right.revision_id));
    milestones.dedup_by(|left, right| left.revision_id == right.revision_id);
    anyhow::ensure!(
        !milestones.is_empty() && milestones.len() <= maximum,
        "semantic worker selected an invalid number of retention milestones"
    );
    for milestone in milestones {
        anyhow::ensure!(
            offered.contains_key(&milestone.revision_id),
            "semantic worker selected a Revision outside the retention window"
        );
        let reason = milestone.reason.trim();
        anyhow::ensure!(
            !reason.is_empty() && reason.chars().count() <= 1_000,
            "semantic worker returned an invalid retention reason"
        );
        milestone.reason = reason.to_owned();
    }
    Ok(())
}

fn excluded_kind(kind: &str, excluded: &[String]) -> bool {
    excluded.iter().any(|excluded| excluded == kind)
}

impl MaintenanceSummaryAnalysis {
    fn candidate(candidate: MaintenanceSummaryCandidate) -> Self {
        Self {
            analyzed_at: maintenance_review_timestamp(),
            decision: MaintenanceReviewDecision::Candidate,
            candidate: Some(candidate),
        }
    }

    fn no_candidate() -> Self {
        Self {
            analyzed_at: maintenance_review_timestamp(),
            decision: MaintenanceReviewDecision::NoCandidate,
            candidate: None,
        }
    }

    fn defer() -> Self {
        Self {
            analyzed_at: maintenance_review_timestamp(),
            decision: MaintenanceReviewDecision::Defer,
            candidate: None,
        }
    }
}

impl MaintenanceRelationAnalysis {
    fn candidate(candidate: MaintenanceRelationCandidate) -> Self {
        Self {
            analyzed_at: maintenance_review_timestamp(),
            decision: MaintenanceReviewDecision::Candidate,
            candidate: Some(candidate),
        }
    }

    fn no_candidate() -> Self {
        Self {
            analyzed_at: maintenance_review_timestamp(),
            decision: MaintenanceReviewDecision::NoCandidate,
            candidate: None,
        }
    }

    fn defer() -> Self {
        Self {
            analyzed_at: maintenance_review_timestamp(),
            decision: MaintenanceReviewDecision::Defer,
            candidate: None,
        }
    }
}

impl MaintenanceTopicAnalysis {
    fn candidate(candidate: MaintenanceTopicCandidate) -> Self {
        Self {
            analyzed_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            decision: MaintenanceReviewDecision::Candidate,
            candidate: Some(candidate),
        }
    }

    fn no_candidate() -> Self {
        Self {
            analyzed_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            decision: MaintenanceReviewDecision::NoCandidate,
            candidate: None,
        }
    }

    fn defer() -> Self {
        Self {
            analyzed_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            decision: MaintenanceReviewDecision::Defer,
            candidate: None,
        }
    }
}

fn maintenance_review_timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn summary_page_eligible(
    page: &pcp_store::DurablePageInventoryItem,
    config: &super::SummaryMaintenanceConfig,
) -> bool {
    page.content_chars >= config.minimum_chars as u64
        && (page.media_type.as_deref() == Some(PACKED_PAGE_MEDIA_TYPE)
            || !excluded_kind(&page.kind, &config.excluded_page_kinds))
}

fn relation_page_eligible(
    page: &pcp_store::DurablePageInventoryItem,
    config: &RelationMaintenanceConfig,
) -> bool {
    let has_semantic_input = page.content_chars > 0
        || page
            .summary
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || page.facets.is_some();
    has_semantic_input && !excluded_kind(&page.kind, &config.excluded_page_kinds)
}

fn build_summary_candidate(
    page: &pcp_store::DurablePageInventoryItem,
    content: String,
) -> MaintenanceSummaryCandidate {
    let mut digest = Sha256::new();
    digest.update(page.page_id.as_bytes());
    digest.update([0]);
    digest.update(page.revision_id.as_bytes());
    digest.update([0]);
    if let Some(summary_revision_id) = &page.summary_revision_id {
        digest.update(summary_revision_id.as_bytes());
    }
    digest.update([0]);
    digest.update(content.as_bytes());
    let encoded = format!("{:x}", digest.finalize());
    MaintenanceSummaryCandidate {
        candidate_id: format!("msu_{}", &encoded[..24]),
        page_id: page.page_id.clone(),
        revision_id: page.revision_id.clone(),
        namespace: page.namespace.clone(),
        content_chars: page.content_chars,
        expected_summary_revision_id: page.summary_revision_id.clone(),
        content,
    }
}

fn build_relation_candidate(
    pages: &[&pcp_store::DurablePageInventoryItem],
) -> MaintenanceRelationCandidate {
    assert_eq!(pages.len(), 2, "relation candidate has two Pages");
    let mut inputs = pages
        .iter()
        .map(|page| MaintenanceRelationInput {
            page_id: page.page_id.clone(),
            revision_id: page.revision_id.clone(),
            preview: page
                .summary
                .as_deref()
                .unwrap_or(&page.snippet)
                .chars()
                .take(PACKING_PREVIEW_CHARS)
                .collect(),
        })
        .collect::<Vec<_>>();
    inputs.sort_by(|left, right| left.page_id.cmp(&right.page_id));
    let namespace = pages[0].namespace.clone();
    assert!(pages.iter().all(|page| page.namespace == namespace));
    let mut digest = Sha256::new();
    for page in &inputs {
        digest.update(page.page_id.as_bytes());
        digest.update([0]);
        digest.update(page.revision_id.as_bytes());
        digest.update([0]);
    }
    let encoded = format!("{:x}", digest.finalize());
    MaintenanceRelationCandidate {
        candidate_id: format!("mrl_{}", &encoded[..24]),
        namespace,
        pages: inputs
            .try_into()
            .expect("relation candidate has exactly two inputs"),
        relation_reason: String::new(),
    }
}

fn build_archive_candidate(
    page: &MaintenanceArchiveScanPage,
    reason: String,
) -> MaintenanceArchiveCandidate {
    let mut digest = Sha256::new();
    digest.update(page.page_id.as_bytes());
    digest.update([0]);
    digest.update(page.revision_id.as_bytes());
    digest.update([0]);
    digest.update(reason.as_bytes());
    let encoded = format!("{:x}", digest.finalize());
    MaintenanceArchiveCandidate {
        candidate_id: format!("marc_{}", &encoded[..24]),
        page_id: page.page_id.clone(),
        revision_id: page.revision_id.clone(),
        namespace: page.namespace.clone(),
        kind: page.kind.clone(),
        observed_at: page.observed_at.clone(),
        content_chars: page.content_chars,
        preview: page.preview.clone(),
        candidate_signals: page.candidate_signals.clone(),
        reason,
    }
}

fn validate_archive_reason(reason: String) -> Result<String> {
    let reason = reason.split_whitespace().collect::<Vec<_>>().join(" ");
    anyhow::ensure!(
        reason.chars().count() >= 12,
        "semantic worker archive review reason is too short"
    );
    anyhow::ensure!(
        reason.chars().count() <= 600,
        "semantic worker archive review reason exceeds 600 characters"
    );
    Ok(reason)
}

fn validate_relation_reason(reason: String) -> Result<String> {
    let reason = reason.split_whitespace().collect::<Vec<_>>().join(" ");
    anyhow::ensure!(
        !reason.is_empty(),
        "semantic worker selected a relation without review evidence"
    );
    anyhow::ensure!(
        reason.chars().count() <= 480,
        "semantic worker relation review evidence exceeds 480 characters"
    );
    Ok(reason)
}

fn build_topic_candidate(
    pages: &[&pcp_store::DurablePageInventoryItem],
    title: String,
    content: String,
    reason: Option<String>,
    refresh_target: Option<MaintenanceTopicRefreshTarget>,
) -> Result<MaintenanceTopicCandidate> {
    let title = title.trim().to_owned();
    let content = content.trim().to_owned();
    let reason = reason.map(validate_topic_reason).transpose()?;
    anyhow::ensure!(
        !title.is_empty() && title.chars().count() <= 160,
        "semantic maintenance worker returned an invalid Topic title"
    );
    anyhow::ensure!(
        content.chars().count() >= 120 && content.chars().count() <= 4_000,
        "semantic maintenance worker returned an invalid Topic content"
    );
    anyhow::ensure!(
        (2..=64).contains(&pages.len())
            && pages
                .iter()
                .all(|page| page.namespace == pages[0].namespace),
        "maintenance Topic sources must be 2..=64 Pages in one Scope"
    );
    let mut digest = Sha256::new();
    for page in pages {
        digest.update(page.page_id.as_bytes());
        digest.update([0]);
        digest.update(page.revision_id.as_bytes());
        digest.update([0]);
    }
    digest.update(title.as_bytes());
    digest.update([0]);
    digest.update(content.as_bytes());
    digest.update([0]);
    if let Some(target) = refresh_target.as_ref() {
        digest.update(b"refresh");
        digest.update([0]);
        digest.update(target.page_id.as_bytes());
        digest.update([0]);
        digest.update(target.revision_id.as_bytes());
    } else {
        digest.update(b"create");
    }
    let encoded = format!("{:x}", digest.finalize());
    Ok(MaintenanceTopicCandidate {
        candidate_id: format!("mtp_{}", &encoded[..24]),
        namespace: pages[0].namespace.clone(),
        title,
        content,
        reason,
        refresh_target,
        pages: pages
            .iter()
            .map(|page| MaintenanceTopicInput {
                page_id: page.page_id.clone(),
                revision_id: page.revision_id.clone(),
                preview: page
                    .summary
                    .as_deref()
                    .unwrap_or(&page.snippet)
                    .chars()
                    .take(PACKING_PREVIEW_CHARS)
                    .collect(),
            })
            .collect(),
    })
}

fn validate_topic_reason(reason: String) -> Result<String> {
    let reason = reason.trim().to_owned();
    anyhow::ensure!(
        !reason.is_empty() && reason.chars().count() <= 480,
        "semantic maintenance worker returned an invalid Topic rationale"
    );
    Ok(reason)
}

fn normalize_worker_summary(content: String, source_text: &str) -> Result<String> {
    let content = content.trim();
    anyhow::ensure!(
        !content.is_empty() && content.chars().count() <= MAX_MAINTENANCE_SUMMARY_CHARS,
        "semantic maintenance worker returned an invalid Summary"
    );
    Ok(normalize_known_source_identifiers(content, source_text))
}

fn feedback_reconciliation_key(feedback_revision_id: &str, target_revision_id: &str) -> String {
    format!("feedback:{feedback_revision_id}:{target_revision_id}")
}

fn reconciliation_request(
    candidate: &MaintenanceReconciliationCandidate,
    created_by: pcp_core::Actor,
    idempotency_key: Option<String>,
    tool_or_model: Option<String>,
) -> ApplyReconciliationRequest {
    ApplyReconciliationRequest {
        feedback_revision_id: candidate.signal.feedback_revision_id.clone(),
        target: PageRevisionRef {
            page_id: candidate.target.page_id.clone(),
            revision_id: candidate.target.revision_id.clone(),
        },
        disposition: candidate.disposition.clone(),
        rationale: candidate.rationale.clone(),
        scope: candidate.scope.clone(),
        replacement: candidate.replacement.clone(),
        basis_revision_ids: candidate.basis_revision_ids.clone(),
        created_by,
        tool_or_model,
        idempotency_key,
    }
}

fn normalize_known_source_identifiers(summary: &str, source: &str) -> String {
    let source_identifiers = ascii_identifier_tokens(source)
        .into_iter()
        .filter(|identifier| is_protected_identifier(identifier))
        .collect::<BTreeSet<_>>();
    if source_identifiers.is_empty() {
        return summary.to_owned();
    }

    let mut normalized = String::with_capacity(summary.len());
    let mut token_start = None;
    for (index, character) in summary.char_indices() {
        if is_ascii_identifier_character(character) {
            token_start.get_or_insert(index);
            continue;
        }
        if let Some(start) = token_start.take() {
            normalized.push_str(&normalize_identifier_token(
                &summary[start..index],
                &source_identifiers,
            ));
        }
        normalized.push(character);
    }
    if let Some(start) = token_start {
        normalized.push_str(&normalize_identifier_token(
            &summary[start..],
            &source_identifiers,
        ));
    }
    normalized
}

fn normalize_identifier_token(token: &str, source_identifiers: &BTreeSet<String>) -> String {
    let identifier = token.trim_matches(|character| matches!(character, '_' | '-' | '.'));
    let Some(source_identifier) = source_identifiers.iter().find(|source_identifier| {
        identifier.eq_ignore_ascii_case(source_identifier)
            || identifiers_are_near_matches(identifier, source_identifier)
    }) else {
        return token.to_owned();
    };
    token.replacen(identifier, source_identifier, 1)
}

fn is_ascii_identifier_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
}

fn ascii_identifier_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut start = None;
    for (index, character) in text.char_indices() {
        if is_ascii_identifier_character(character) {
            start.get_or_insert(index);
        } else if let Some(start_index) = start.take() {
            push_ascii_identifier_token(&mut tokens, &text[start_index..index]);
        }
    }
    if let Some(start_index) = start {
        push_ascii_identifier_token(&mut tokens, &text[start_index..]);
    }
    tokens
}

fn push_ascii_identifier_token(tokens: &mut Vec<String>, raw: &str) {
    let token = raw.trim_matches(|character| matches!(character, '_' | '-' | '.'));
    if token.len() >= 3 {
        tokens.push(token.to_owned());
    }
}

fn is_protected_identifier(identifier: &str) -> bool {
    let uppercase = identifier
        .bytes()
        .filter(|byte| byte.is_ascii_uppercase())
        .count();
    identifier.bytes().any(|byte| byte.is_ascii_digit())
        || uppercase >= 2
        || identifier
            .bytes()
            .skip(1)
            .any(|byte| byte.is_ascii_uppercase())
        || identifier.contains(['_', '-', '.'])
}

fn identifiers_are_near_matches(left: &str, right: &str) -> bool {
    if left == right || left.len() < 5 || right.len() < 5 {
        return false;
    }
    let length_difference = left.len().abs_diff(right.len());
    length_difference <= 2 && levenshtein_distance(left, right) <= 2
}

fn levenshtein_distance(left: &str, right: &str) -> usize {
    let left = left.to_ascii_lowercase();
    let right = right.to_ascii_lowercase();
    let right = right.as_bytes();
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    for (left_index, left_byte) in left.bytes().enumerate() {
        let mut current = Vec::with_capacity(right.len() + 1);
        current.push(left_index + 1);
        for (right_index, right_byte) in right.iter().enumerate() {
            let replacement = previous[right_index] + usize::from(left_byte != *right_byte);
            let insertion = current[right_index] + 1;
            let deletion = previous[right_index + 1] + 1;
            current.push(replacement.min(insertion).min(deletion));
        }
        previous = current;
    }
    previous[right.len()]
}

fn worker_operation(request: &MaintenanceWorkerRequest) -> &'static str {
    match request {
        MaintenanceWorkerRequest::SummarizePage { .. } => "summarize_page",
        MaintenanceWorkerRequest::SummarizePages { .. } => "summarize_pages",
        MaintenanceWorkerRequest::SelectPacking { .. } => "select_packing",
        MaintenanceWorkerRequest::AnalyzePacking { .. } => "analyze_packing",
        MaintenanceWorkerRequest::SelectRelation { .. } => "select_relation",
        MaintenanceWorkerRequest::ExtractTopic { .. } => "extract_topic",
        MaintenanceWorkerRequest::AssessArchive { .. } => "assess_archive",
        MaintenanceWorkerRequest::ReconcileFeedback { .. } => "reconcile_feedback",
        MaintenanceWorkerRequest::SelectRetentionMilestones { .. } => "select_retention_milestones",
    }
}

fn worker_scopes(request: &MaintenanceWorkerRequest, access: &AccessSession) -> Vec<String> {
    let mut scopes = match request {
        MaintenanceWorkerRequest::SummarizePage { page } => vec![page.namespace.clone()],
        MaintenanceWorkerRequest::SummarizePages { pages } => {
            pages.iter().map(|page| page.namespace.clone()).collect()
        }
        MaintenanceWorkerRequest::AssessArchive { page } => vec![page.page.namespace.clone()],
        MaintenanceWorkerRequest::ReconcileFeedback {
            feedback, targets, ..
        } => std::iter::once(feedback.namespace.clone())
            .chain(targets.iter().map(|page| page.namespace.clone()))
            .collect(),
        MaintenanceWorkerRequest::SelectRelation { pages, .. }
        | MaintenanceWorkerRequest::ExtractTopic { pages, .. } => {
            pages.iter().map(|page| page.namespace.clone()).collect()
        }
        MaintenanceWorkerRequest::SelectRetentionMilestones { pages, .. } => {
            pages.iter().map(|page| page.namespace.clone()).collect()
        }
        // Packing candidates intentionally carry no Page content or namespace.
        // Attribute those model calls to the scopes the maintenance session may
        // actually operate on, preserving ACL-filtered Runtime reporting.
        MaintenanceWorkerRequest::SelectPacking { .. }
        | MaintenanceWorkerRequest::AnalyzePacking { .. } => access
            .grants
            .iter()
            .map(|grant| grant.namespace.clone())
            .collect(),
    };
    scopes.sort();
    scopes.dedup();
    scopes
}

fn response_name(response: &MaintenanceWorkerResponse) -> &'static str {
    match response {
        MaintenanceWorkerResponse::WriteSummary { .. } => "write_summary",
        MaintenanceWorkerResponse::Summaries { .. } => "summaries",
        MaintenanceWorkerResponse::Candidate { .. } => "candidate",
        MaintenanceWorkerResponse::PackingCandidates { .. } => "packing_candidates",
        MaintenanceWorkerResponse::Relate { .. } => "relate",
        MaintenanceWorkerResponse::ExtractTopic { .. } => "extract_topic",
        MaintenanceWorkerResponse::ArchiveReview { .. } => "archive_review",
        MaintenanceWorkerResponse::ReconcileFeedback { .. } => "reconcile_feedback",
        MaintenanceWorkerResponse::Retain { .. } => "retain",
        MaintenanceWorkerResponse::NoCandidate => "no_candidate",
        MaintenanceWorkerResponse::Defer => "defer",
    }
}

#[cfg(test)]
mod relation_window_tests {
    use std::{
        collections::{BTreeMap, BTreeSet},
        time::Duration,
    };

    use chrono::{TimeZone, Utc};
    use pcp_core::{PACKED_PAGE_MEDIA_TYPE, PageMutability, SourceSpan};
    use pcp_store::DurablePageInventoryItem;

    use super::{
        MaintenanceWakeReason, PackingMaintenanceConfig, RelationMaintenanceConfig,
        archive_scan_from_inventory, existing_topics_for_selected, packing_candidate_windows,
        relation_candidate_windows, select_topic_refresh_target, source_boundary_relation_windows,
        wait_for_scheduler_wakeup,
    };

    #[tokio::test]
    async fn external_write_interrupts_a_long_idle_wait() {
        let (sender, receiver) = tokio::sync::watch::channel(0_u64);
        let mut wakeup = Some(receiver);
        sender.send_modify(|generation| *generation += 1);

        let reason = tokio::time::timeout(
            Duration::from_millis(100),
            wait_for_scheduler_wakeup(&mut wakeup, 3_600, MaintenanceWakeReason::Timer),
        )
        .await
        .expect("write wake should not wait for the safety poll");

        assert_eq!(reason, MaintenanceWakeReason::ExternalWrite);
    }

    fn page(namespace: &str, page_id: &str) -> DurablePageInventoryItem {
        DurablePageInventoryItem {
            page_id: page_id.to_owned(),
            revision_id: format!("rev_{page_id}"),
            namespace: namespace.to_owned(),
            kind: "note".to_owned(),
            mutability: PageMutability::Sealed,
            created_at: "2026-08-18T00:00:00Z".to_owned(),
            observed_at: None,
            source_span: None,
            media_type: Some("text/markdown".to_owned()),
            content_chars: 1,
            snippet: page_id.to_owned(),
            facets: None,
            summary_revision_id: None,
            summary_target_revision_id: None,
            summary: None,
            relation_types: Vec::new(),
            provenance_input_revision_ids: Vec::new(),
            topic_source_page_ids: Vec::new(),
            superseded: false,
            packing_protected: false,
        }
    }

    #[test]
    fn exact_topic_source_set_refreshes_existing_front_door() {
        let first = page("conversation:alpha", "source-1");
        let second = page("conversation:alpha", "source-2");
        let mut topic = page("conversation:alpha", "topic-1");
        topic.kind = "topic_summary".to_owned();
        topic.topic_source_page_ids = vec![first.page_id.clone(), second.page_id.clone()];
        topic.facets = Some(serde_json::json!({"topicTitle": "Existing Topic"}));
        topic.snippet = "Existing routing front door".to_owned();
        let inventory = vec![first.clone(), second.clone(), topic];
        let selected = vec![&inventory[0], &inventory[1]];
        let existing_topics = existing_topics_for_selected(&inventory, &selected);

        let target = select_topic_refresh_target(&selected, &existing_topics, None)
            .expect("select exact Topic refresh target")
            .expect("exact logical source set refreshes instead of creating");

        assert_eq!(target.page_id, "topic-1");
        assert_eq!(target.source_page_count, 2);
        assert_eq!(target.shared_source_page_count, 2);
    }

    #[test]
    fn relation_windows_never_cross_namespaces() {
        let inventory = vec![
            page("conversation:alpha", "alpha-1"),
            page("conversation:beta", "beta-1"),
            page("conversation:alpha", "alpha-2"),
            page("conversation:beta", "beta-2"),
        ];
        let config = RelationMaintenanceConfig {
            enabled: true,
            candidate_window: 2,
            ..RelationMaintenanceConfig::default()
        };

        let windows = relation_candidate_windows(&inventory, &config, &BTreeSet::new());

        assert_eq!(windows.len(), 2);
        assert!(windows.iter().all(|window| {
            window
                .iter()
                .map(|page| &page.namespace)
                .all(|namespace| namespace == &window[0].namespace)
        }));
    }

    #[test]
    fn provenance_inputs_are_prioritized_without_becoming_relations() {
        let source = page("conversation:alpha", "source");
        let mut derived = page("conversation:alpha", "derived");
        derived.provenance_input_revision_ids = vec![source.revision_id.clone()];
        let unrelated = page("conversation:alpha", "unrelated");
        let config = RelationMaintenanceConfig {
            enabled: true,
            candidate_window: 3,
            ..RelationMaintenanceConfig::default()
        };

        let windows = relation_candidate_windows(
            &[unrelated, source.clone(), derived.clone()],
            &config,
            &BTreeSet::new(),
        );

        let prioritized = windows.first().expect("provenance candidate window");
        assert_eq!(prioritized.len(), 2);
        assert_eq!(prioritized[0].page_id, derived.page_id);
        assert_eq!(prioritized[1].page_id, source.page_id);
        assert!(derived.relation_types.is_empty());
        assert!(source.relation_types.is_empty());
    }

    #[test]
    fn source_boundary_windows_prioritize_contiguous_packs_with_a_shared_identifier() {
        let mut first = page("conversation:alpha", "oet-question");
        first.media_type = Some(PACKED_PAGE_MEDIA_TYPE.to_owned());
        first.source_span = Some(SourceSpan {
            stream_id: "host:alpha".to_owned(),
            start: 157,
            end: 158,
        });
        first.snippet = "What is OET?".to_owned();
        let mut second = page("conversation:alpha", "oet-answer");
        second.media_type = Some(PACKED_PAGE_MEDIA_TYPE.to_owned());
        second.source_span = Some(SourceSpan {
            stream_id: "host:alpha".to_owned(),
            start: 159,
            end: 163,
        });
        second.snippet = "OET is the requested formal framework.".to_owned();
        let mut unrelated = page("conversation:alpha", "other-pack");
        unrelated.media_type = Some(PACKED_PAGE_MEDIA_TYPE.to_owned());
        unrelated.source_span = Some(SourceSpan {
            stream_id: "host:alpha".to_owned(),
            start: 164,
            end: 165,
        });
        unrelated.snippet = "A separate discussion of PCP maintenance.".to_owned();

        let windows = source_boundary_relation_windows(&BTreeMap::from([(
            "conversation:alpha".to_owned(),
            vec![unrelated, second.clone(), first.clone()],
        )]));

        assert_eq!(windows.len(), 1);
        assert_eq!(
            windows[0]
                .iter()
                .map(|page| page.page_id.as_str())
                .collect::<Vec<_>>(),
            vec![first.page_id.as_str(), second.page_id.as_str()]
        );
    }

    #[test]
    fn packing_windows_expose_a_coherent_pair_of_adjacent_packs_for_merge() {
        let mut first = page("conversation:alpha", "oet-question");
        first.media_type = Some(PACKED_PAGE_MEDIA_TYPE.to_owned());
        first.mutability = PageMutability::Revisioned;
        first.source_span = Some(SourceSpan {
            stream_id: "host:alpha".to_owned(),
            start: 157,
            end: 158,
        });
        first.snippet = "What is OET?".to_owned();
        let mut second = page("conversation:alpha", "oet-answer");
        second.media_type = Some(PACKED_PAGE_MEDIA_TYPE.to_owned());
        second.mutability = PageMutability::Revisioned;
        second.source_span = Some(SourceSpan {
            stream_id: "host:alpha".to_owned(),
            start: 159,
            end: 163,
        });
        second.snippet = "OET is the requested formal framework.".to_owned();

        let windows = packing_candidate_windows(
            &[first.clone(), second.clone()],
            &PackingMaintenanceConfig::default(),
        );

        assert_eq!(windows.len(), 1);
        assert_eq!(
            windows[0]
                .iter()
                .map(|page| page.page_id.as_str())
                .collect::<Vec<_>>(),
            vec![first.page_id.as_str(), second.page_id.as_str()]
        );
    }

    #[test]
    fn archive_scan_offers_old_non_structural_pages_for_human_review() {
        let eligible = page("conversation:alpha", "old-isolated");
        let mut related = page("conversation:alpha", "old-related");
        related.relation_types = vec!["related_to".to_owned()];
        let mut summarized = page("conversation:alpha", "old-summarized");
        summarized.summary_target_revision_id = Some(summarized.revision_id.clone());
        let mut packed = page("conversation:alpha", "old-packed");
        packed.media_type = Some(PACKED_PAGE_MEDIA_TYPE.to_owned());
        let mut recent = page("conversation:alpha", "recent-isolated");
        recent.observed_at = Some("2026-08-21T00:00:00Z".to_owned());

        let scan = archive_scan_from_inventory(
            &[eligible, related, summarized, packed, recent],
            Utc.with_ymd_and_hms(2026, 9, 2, 0, 0, 0).unwrap(),
        );

        assert_eq!(scan.inspected_pages, 5);
        assert_eq!(scan.eligible_pages, 3);
        assert_eq!(scan.estimated_model_calls, 3);
        assert_eq!(
            scan.pages
                .iter()
                .map(|page| page.page_id.as_str())
                .collect::<Vec<_>>(),
            vec!["old-isolated", "old-related", "old-summarized"]
        );
        assert_eq!(
            scan.pages[0].candidate_signals,
            vec![
                "older_than_14_days".to_owned(),
                "no_current_routing_summary".to_owned(),
                "no_explicit_relations".to_owned(),
            ]
        );
        assert_eq!(
            scan.pages[1].candidate_signals,
            vec![
                "older_than_14_days".to_owned(),
                "no_current_routing_summary".to_owned(),
                "explicit_relations:related_to".to_owned(),
            ]
        );
        assert_eq!(
            scan.pages[2].candidate_signals,
            vec![
                "older_than_14_days".to_owned(),
                "has_current_routing_summary".to_owned(),
                "no_explicit_relations".to_owned(),
            ]
        );
    }
}
