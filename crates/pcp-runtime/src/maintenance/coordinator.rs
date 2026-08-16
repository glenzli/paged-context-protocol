use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use chrono::{Duration as ChronoDuration, SecondsFormat, Utc};
use pcp_client::PcpApi;
use pcp_core::{
    LinkPagesRequest, PACKED_PAGE_MEDIA_TYPE, PackPagesRequest, PageMutability, PageRevisionRef,
    PlanRevisionRetentionRequest, Projection, PutRevisionRetentionLeaseRequest, ReadPagesRequest,
    RetentionPolicy, SourceSpan, WriteResult, WriteSummaryRequest,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    MaintenanceConfig, MaintenanceMode, MaintenanceWorkerRequest, MaintenanceWorkerResponse,
    PackingMaintenanceConfig, RelationMaintenanceConfig, RetentionMilestone,
    SemanticMaintenanceWorker,
    ledger::{
        MaintenanceLedger, packing_key, retention_window_key, selection_window_key, summary_key,
    },
    worker::{
        MaintenanceDetailPage, MaintenanceRoutingPage, PackingCandidateGroup, PackingCandidatePage,
        RelationCandidatePage,
    },
};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MaintenanceCycleReport {
    pub inspected_pages: usize,
    pub worker_calls: u32,
    pub summaries_written: u32,
    pub summaries_proposed: u32,
    pub packs_committed: u32,
    pub packs_proposed: u32,
    pub relations_committed: u32,
    pub relations_proposed: u32,
    pub retention_leases_written: u32,
    pub retention_leases_proposed: u32,
    pub deferred: u32,
}

impl MaintenanceCycleReport {
    fn merge(&mut self, report: Self) {
        self.inspected_pages = self.inspected_pages.max(report.inspected_pages);
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

const PACKING_CANDIDATE_WINDOW: usize = 32;
const PACKING_ROUTING_CHARS: usize = 480;
const PACKING_GROUPS_PER_MODEL_CALL: usize = 8;
const PACKING_RETRY_AFTER_SECONDS: u64 = 86_400;
const MAINTENANCE_READ_BATCH_PAGES: usize = 20;
const MAX_MAINTENANCE_SUMMARY_CHARS: usize = 1_200;

pub struct RuntimeMaintainer {
    client: Arc<dyn PcpApi>,
    worker: Arc<dyn SemanticMaintenanceWorker>,
    config: MaintenanceConfig,
    ledger: MaintenanceLedger,
}

impl RuntimeMaintainer {
    pub async fn load(
        client: Arc<dyn PcpApi>,
        worker: Arc<dyn SemanticMaintenanceWorker>,
        config: MaintenanceConfig,
    ) -> Result<Self> {
        config.validate()?;
        let ledger = MaintenanceLedger::load(&config.state_path).await?;
        Ok(Self {
            client,
            worker,
            config,
            ledger,
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
        }
    }

    pub async fn run_forever(mut self) -> Result<()> {
        if self.config.initial_delay_seconds > 0 {
            tokio::time::sleep(Duration::from_secs(self.config.initial_delay_seconds)).await;
        }
        loop {
            let retry_soon = match self.run_bounded_cycle().await {
                Ok(report)
                    if report.summaries_written > 0
                        || report.summaries_proposed > 0
                        || report.packs_committed > 0
                        || report.packs_proposed > 0
                        || report.relations_committed > 0
                        || report.relations_proposed > 0
                        || report.retention_leases_written > 0
                        || report.retention_leases_proposed > 0 =>
                {
                    eprintln!(
                        "PCP maintenance: {} Summary proposed / {} written, {} pack proposed / {} committed, {} relation proposed / {} committed, {} retention proposed / {} leased after {} worker calls",
                        report.summaries_proposed,
                        report.summaries_written,
                        report.packs_proposed,
                        report.packs_committed,
                        report.relations_proposed,
                        report.relations_committed,
                        report.retention_leases_proposed,
                        report.retention_leases_written,
                        report.worker_calls
                    );
                    report.worker_calls >= self.config.max_jobs_per_cycle
                }
                Ok(_) => false,
                Err(error) => {
                    eprintln!("PCP maintenance cycle failed: {error:#}");
                    true
                }
            };
            let delay = if retry_soon {
                self.config.interval_seconds.min(30)
            } else {
                self.config.interval_seconds
            };
            tokio::time::sleep(Duration::from_secs(delay)).await;
        }
    }

    pub async fn run_once(&mut self) -> Result<MaintenanceCycleReport> {
        self.run_once_inner(true, self.config.max_jobs_per_cycle)
            .await
    }

    pub async fn run_bounded_cycle(&mut self) -> Result<MaintenanceCycleReport> {
        let mut aggregate = MaintenanceCycleReport::default();
        while aggregate.worker_calls < self.config.max_jobs_per_cycle {
            let report = self.run_once_inner(true, 1).await?;
            let worker_calls = report.worker_calls;
            aggregate.merge(report);
            if worker_calls == 0 {
                break;
            }
        }
        Ok(aggregate)
    }

    pub async fn scan_packing_candidates(&self) -> Result<MaintenancePackScan> {
        let inventory = self.client.durable_page_inventory(Vec::new()).await?;
        let windows = packing_candidate_windows(&inventory, &self.config.packing);
        let eligible_pages = windows.iter().map(Vec::len).sum::<usize>();
        let scan_id = packing_scan_id(&windows, &self.config.packing);
        let groups = windows
            .iter()
            .map(|window| packing_scan_group(window))
            .collect();
        Ok(MaintenancePackScan {
            captured_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            scan_id,
            inspected_pages: inventory.len(),
            eligible_pages,
            excluded_pages: inventory.len().saturating_sub(eligible_pages),
            candidate_group_count: windows.len(),
            estimated_model_calls: windows.len().div_ceil(PACKING_GROUPS_PER_MODEL_CALL),
            groups,
        })
    }

    pub async fn analyze_packing_candidates(
        &self,
        request: AnalyzeMaintenancePacksRequest,
    ) -> Result<MaintenancePackAnalysis> {
        let inventory = self.client.durable_page_inventory(Vec::new()).await?;
        let windows = packing_candidate_windows(&inventory, &self.config.packing);
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
        let groups = batch
            .iter()
            .map(|window| PackingCandidateGroup {
                group_id: packing_scan_group_id(window),
                pages: window
                    .iter()
                    .map(|page| PackingCandidatePage::from_inventory(page, PACKING_ROUTING_CHARS))
                    .collect(),
            })
            .collect();

        let mut candidates = Vec::new();
        let worker_calls = 1_u32;
        let mut no_candidate_groups = 0_u32;
        let mut deferred_groups = 0_u32;
        let mut issue = None;
        match self
            .worker
            .evaluate(MaintenanceWorkerRequest::AnalyzePacking {
                groups,
                max_pages_per_candidate: self.config.packing.max_pages,
            })
            .await
        {
            Ok(MaintenanceWorkerResponse::PackingCandidates {
                candidates: selected_sets,
            }) => match validate_packing_analysis_batch(
                batch,
                selected_sets,
                self.config.packing.max_pages,
            ) {
                Ok((mut selected, represented_groups)) => {
                    candidates.append(&mut selected);
                    no_candidate_groups = batch.len().saturating_sub(represented_groups) as u32;
                }
                Err(error) => {
                    deferred_groups = batch.len() as u32;
                    issue = Some(MaintenancePackAnalysisIssue {
                        code: "invalid_model_selection".to_owned(),
                        message: error.to_string(),
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
        let requested_ids = request
            .pages
            .iter()
            .map(|page| page.page_id.clone())
            .collect::<Vec<_>>();
        let candidate = packing_candidate_windows(&inventory, &self.config.packing)
            .into_iter()
            .find_map(|window| {
                let selected =
                    select_packing_items(&window, &requested_ids, self.config.packing.max_pages)?;
                let candidate = build_pack_candidate(&selected);
                (candidate.candidate_id == request.candidate_id).then_some(candidate)
            })
            .context("maintenance Pack candidate is stale or no longer eligible")?;
        anyhow::ensure!(
            candidate
                .pages
                .iter()
                .zip(&request.pages)
                .all(|(candidate, requested)| {
                    candidate.page_id == requested.page_id
                        && candidate.revision_id == requested.revision_id
                }),
            "maintenance Pack candidate revisions changed after scanning"
        );

        self.client
            .pack_pages(PackPagesRequest {
                pages: request.pages,
                idempotency_key: Some(format!("maintenance:{}", candidate.candidate_id)),
            })
            .await
    }

    pub async fn run_once_with_job_limit(
        &mut self,
        max_jobs: u32,
    ) -> Result<MaintenanceCycleReport> {
        anyhow::ensure!(max_jobs > 0, "maintenance job limit must be positive");
        self.run_once_inner(true, max_jobs.min(self.config.max_jobs_per_cycle))
            .await
    }

    pub async fn run_operator_observe_once(&mut self) -> Result<MaintenanceCycleReport> {
        anyhow::ensure!(
            self.config.mode == MaintenanceMode::Observe,
            "operator maintenance run-once only permits observe mode"
        );
        self.run_once_inner(false, 1).await
    }

    async fn run_once_inner(
        &mut self,
        persist_ledger: bool,
        max_jobs: u32,
    ) -> Result<MaintenanceCycleReport> {
        let inventory = self.client.durable_page_inventory(Vec::new()).await?;
        let mut report = MaintenanceCycleReport {
            inspected_pages: inventory.len(),
            ..MaintenanceCycleReport::default()
        };
        let mut jobs_remaining = max_jobs;

        if self.config.summary.enabled
            && jobs_remaining > 0
            && self
                .run_summary_job(&inventory, &mut report)
                .await
                .context("run PCP Summary maintenance job")?
        {
            jobs_remaining -= 1;
        }
        let packing_ran = self.config.packing.enabled
            && jobs_remaining > 0
            && self
                .run_packing_job(&inventory, &mut report)
                .await
                .context("run PCP packing maintenance job")?;
        if packing_ran {
            jobs_remaining -= 1;
        }
        if self.config.relation.enabled
            && !packing_ran
            && jobs_remaining > 0
            && self
                .run_relation_job(&inventory, &mut report)
                .await
                .context("run PCP relation maintenance job")?
        {
            jobs_remaining -= 1;
        }
        if self.config.retention.enabled && jobs_remaining > 0 {
            self.run_retention_job(&mut report)
                .await
                .context("run PCP semantic retention maintenance job")?;
        }

        if persist_ledger {
            self.ledger.save(&self.config.state_path).await?;
        }
        Ok(report)
    }

    async fn run_summary_job(
        &mut self,
        inventory: &[pcp_store::DurablePageInventoryItem],
        report: &mut MaintenanceCycleReport,
    ) -> Result<bool> {
        let eligible = |page: &&pcp_store::DurablePageInventoryItem| {
            page.content_chars >= self.config.summary.minimum_chars as u64
                && (page.media_type.as_deref() == Some(PACKED_PAGE_MEDIA_TYPE)
                    || !excluded_kind(&page.kind, &self.config.summary.excluded_page_kinds))
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
        report.worker_calls += 1;
        match self
            .worker
            .evaluate(MaintenanceWorkerRequest::SummarizePage {
                page: Box::new(page),
            })
            .await?
        {
            MaintenanceWorkerResponse::WriteSummary { content } => {
                let content = match normalize_worker_summary(content) {
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
                    self.ledger.record(
                        summary_key(&page_id),
                        "observed_write_summary",
                        self.config.summary.retry_after_seconds,
                    );
                    report.summaries_proposed += 1;
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
        let Some(candidates) = relation_candidate_windows(
            inventory,
            &self.config.relation,
            self.config.packing.enabled,
        )
        .into_iter()
        .find(|pages| {
            self.ledger.eligible(&relation_window_key(
                &pages
                    .iter()
                    .map(|page| page.revision_id.clone())
                    .collect::<Vec<_>>(),
            ))
        }) else {
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
        let mut relation_edges = self.existing_related_pairs(&candidates).await?;
        relation_edges.extend(
            self.ledger
                .active_relation_pairs()
                .into_iter()
                .filter(|pair| {
                    pair.iter()
                        .all(|page_id| offered_page_ids.contains(page_id))
                }),
        );
        relation_edges.sort();
        relation_edges.dedup();
        let excluded_page_pairs = connected_relation_pairs(&offered_page_ids, &relation_edges);

        report.worker_calls += 1;
        let response = self
            .worker
            .evaluate(MaintenanceWorkerRequest::SelectRelation {
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
        let mut page_ids = match response {
            MaintenanceWorkerResponse::Relate { page_ids } => page_ids,
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
        anyhow::ensure!(
            page_ids[0] != page_ids[1]
                && page_ids.iter().all(|page_id| offered.contains_key(page_id)),
            "semantic worker selected an invalid relation pair"
        );
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

        if self.config.applies_changes() {
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
        let outcome = if self.config.applies_changes() {
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

    async fn run_packing_job(
        &mut self,
        inventory: &[pcp_store::DurablePageInventoryItem],
        report: &mut MaintenanceCycleReport,
    ) -> Result<bool> {
        let Some(candidates) = packing_candidate_windows(inventory, &self.config.packing)
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
            .enumerate()
            .map(|(index, page)| (page.page_id.clone(), (index, page.revision_id.clone())))
            .collect::<HashMap<_, _>>();
        let routing_pages = candidates
            .iter()
            .map(|page| PackingCandidatePage::from_inventory(page, PACKING_ROUTING_CHARS))
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
        report.worker_calls += 1;
        let selection = self
            .worker
            .evaluate(MaintenanceWorkerRequest::SelectPacking {
                pages: routing_pages,
                excluded_candidate_sets: excluded_candidate_sets.clone(),
            })
            .await?;
        let page_ids = match selection {
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
        let unique = page_ids.iter().collect::<std::collections::HashSet<_>>();
        if unique.len() != page_ids.len()
            || !(2..=self.config.packing.max_pages).contains(&page_ids.len())
            || !page_ids
                .iter()
                .all(|page_id| routing_by_id.contains_key(page_id))
        {
            self.ledger.record(
                selection_key,
                "invalid_worker_selection",
                PACKING_RETRY_AFTER_SECONDS,
            );
            report.deferred += 1;
            return Ok(true);
        }
        let positions = page_ids
            .iter()
            .map(|page_id| routing_by_id[page_id].0)
            .collect::<Vec<_>>();
        if !positions.windows(2).all(|pair| pair[0] + 1 == pair[1]) {
            self.ledger.record(
                selection_key,
                "invalid_worker_selection",
                PACKING_RETRY_AFTER_SECONDS,
            );
            report.deferred += 1;
            return Ok(true);
        }
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
                            revision_id: routing_by_id[page_id].1.clone(),
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
            self.ledger
                .record(key, "observed_pack", PACKING_RETRY_AFTER_SECONDS);
            report.packs_proposed += 1;
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
        report.worker_calls += 1;
        match self
            .worker
            .evaluate(MaintenanceWorkerRequest::SelectRetentionMilestones {
                pages: routing_pages,
                max_revisions: self.config.retention.max_revisions_per_cycle,
                lease_days: self.config.retention.lease_days,
            })
            .await?
        {
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
}

fn relation_candidate_windows(
    inventory: &[pcp_store::DurablePageInventoryItem],
    config: &RelationMaintenanceConfig,
    packing_enabled: bool,
) -> Vec<Vec<pcp_store::DurablePageInventoryItem>> {
    let eligible = inventory
        .iter()
        .filter(|page| {
            let unpacked_stream_leaf = packing_enabled
                && page.mutability == PageMutability::Sealed
                && page.source_span.is_some()
                && page.media_type.as_deref() != Some(PACKED_PAGE_MEDIA_TYPE);
            let has_semantic_input = page.content_chars > 0
                || page
                    .summary
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                || page.facets.is_some();
            !unpacked_stream_leaf
                && has_semantic_input
                && !excluded_kind(&page.kind, &config.excluded_page_kinds)
        })
        .cloned()
        .collect::<Vec<_>>();
    let window_size = config.candidate_window.max(2);
    let stride = (window_size / 2).max(1);
    let mut windows = Vec::new();
    let mut start = 0;
    while start < eligible.len() {
        let end = start.saturating_add(window_size).min(eligible.len());
        if end.saturating_sub(start) >= 2 {
            windows.push(eligible[start..end].to_vec());
        }
        if end == eligible.len() {
            break;
        }
        start = start.saturating_add(stride);
    }
    windows
}

fn relation_window_key(revision_ids: &[String]) -> String {
    let mut revision_ids = revision_ids.to_vec();
    revision_ids.sort();
    revision_ids.dedup();
    format!("relation_window:{}", revision_ids.join(","))
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

fn packing_scan_id(
    windows: &[Vec<pcp_store::DurablePageInventoryItem>],
    config: &PackingMaintenanceConfig,
) -> String {
    let mut digest = Sha256::new();
    digest.update(config.max_pages.to_le_bytes());
    digest.update(config.max_input_chars.to_le_bytes());
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
    max_pages: usize,
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
                select_packing_items(window, &page_ids, max_pages).map(|pages| (index, pages))
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

fn select_packing_items<'a>(
    candidates: &'a [pcp_store::DurablePageInventoryItem],
    page_ids: &[String],
    max_pages: usize,
) -> Option<Vec<&'a pcp_store::DurablePageInventoryItem>> {
    if !(2..=max_pages).contains(&page_ids.len()) {
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
    Some(
        positions
            .into_iter()
            .map(|position| &candidates[position])
            .collect(),
    )
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
                preview: page.snippet.chars().take(PACKING_ROUTING_CHARS).collect(),
            })
            .collect(),
    }
}

fn packing_candidate_windows(
    inventory: &[pcp_store::DurablePageInventoryItem],
    config: &PackingMaintenanceConfig,
) -> Vec<Vec<pcp_store::DurablePageInventoryItem>> {
    let eligible = |page: &&pcp_store::DurablePageInventoryItem| {
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
    };
    let mut visited = std::collections::HashSet::new();
    let mut windows = Vec::new();
    for seed in inventory.iter().filter(eligible) {
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
            .filter(eligible)
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
        let mut run_chars = 0_u64;
        let mut run_anchors = 0_usize;
        for page in group {
            if page.content_chars > u64::from(config.max_input_chars) {
                push_packing_window(&mut windows, &mut run, config.max_pages);
                run_chars = 0;
                run_anchors = 0;
                continue;
            }
            let page_is_anchor = page.media_type.as_deref() == Some(PACKED_PAGE_MEDIA_TYPE);
            let contiguous =
                run.last()
                    .is_none_or(|previous: &pcp_store::DurablePageInventoryItem| {
                        let previous = previous.source_span.as_ref().expect("eligible sourceSpan");
                        let current = page.source_span.as_ref().expect("eligible sourceSpan");
                        previous.end.checked_add(1) == Some(current.start)
                    });
            let fits = run.len() < config.max_pages
                && run_chars.saturating_add(page.content_chars)
                    <= u64::from(config.max_input_chars)
                && run_anchors + usize::from(page_is_anchor) <= 1;
            if !contiguous || !fits {
                push_packing_window(&mut windows, &mut run, config.max_pages);
                run_chars = 0;
                run_anchors = 0;
            }
            run_chars = run_chars.saturating_add(page.content_chars);
            run_anchors += usize::from(page_is_anchor);
            run.push(page);
        }
        push_packing_window(&mut windows, &mut run, config.max_pages);
    }
    windows
}

fn push_packing_window(
    windows: &mut Vec<Vec<pcp_store::DurablePageInventoryItem>>,
    run: &mut Vec<pcp_store::DurablePageInventoryItem>,
    max_pages: usize,
) {
    if run.len() >= 2 {
        run.truncate(PACKING_CANDIDATE_WINDOW.min(max_pages));
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

fn normalize_worker_summary(content: String) -> Result<String> {
    let content = content.trim();
    anyhow::ensure!(
        !content.is_empty() && content.chars().count() <= MAX_MAINTENANCE_SUMMARY_CHARS,
        "semantic maintenance worker returned an invalid Summary"
    );
    Ok(content.to_owned())
}

fn response_name(response: &MaintenanceWorkerResponse) -> &'static str {
    match response {
        MaintenanceWorkerResponse::WriteSummary { .. } => "write_summary",
        MaintenanceWorkerResponse::Candidate { .. } => "candidate",
        MaintenanceWorkerResponse::PackingCandidates { .. } => "packing_candidates",
        MaintenanceWorkerResponse::Relate { .. } => "relate",
        MaintenanceWorkerResponse::Retain { .. } => "retain",
        MaintenanceWorkerResponse::NoCandidate => "no_candidate",
        MaintenanceWorkerResponse::Defer => "defer",
    }
}
