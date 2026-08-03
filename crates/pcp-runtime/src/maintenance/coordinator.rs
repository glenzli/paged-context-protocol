use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use chrono::{Duration as ChronoDuration, SecondsFormat, Utc};
use pcp_client::PcpApi;
use pcp_core::{
    ConsolidatePagesRequest, LifecycleStatus, PagePayload, PlanRevisionRetentionRequest,
    Projection, PutRevisionRetentionLeaseRequest, ReadPagesRequest, RetentionPolicy,
    WriteSummaryRequest,
};
use serde::Serialize;

use super::{
    MaintenanceConfig, MaintenanceWorkerRequest, MaintenanceWorkerResponse, RetentionMilestone,
    SemanticMaintenanceWorker,
    ledger::{
        MaintenanceLedger, compaction_key, retention_window_key, selection_window_key, summary_key,
    },
    worker::{MaintenanceDetailPage, MaintenanceRoutingPage},
};

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceCycleReport {
    pub inspected_pages: usize,
    pub worker_calls: u32,
    pub summaries_written: u32,
    pub summaries_proposed: u32,
    pub consolidations_committed: u32,
    pub consolidations_proposed: u32,
    pub retention_leases_written: u32,
    pub retention_leases_proposed: u32,
    pub kept_separate: u32,
    pub deferred: u32,
}

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
            let retry_soon = match self.run_once().await {
                Ok(report)
                    if report.summaries_written > 0
                        || report.summaries_proposed > 0
                        || report.consolidations_committed > 0
                        || report.consolidations_proposed > 0
                        || report.retention_leases_written > 0
                        || report.retention_leases_proposed > 0 =>
                {
                    eprintln!(
                        "PCP maintenance: {} Summary proposed / {} written, {} consolidation proposed / {} committed, {} retention proposed / {} leased after {} worker calls",
                        report.summaries_proposed,
                        report.summaries_written,
                        report.consolidations_proposed,
                        report.consolidations_committed,
                        report.retention_leases_proposed,
                        report.retention_leases_written,
                        report.worker_calls
                    );
                    false
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
        let inventory = self.client.durable_page_inventory(Vec::new()).await?;
        let mut report = MaintenanceCycleReport {
            inspected_pages: inventory.len(),
            ..MaintenanceCycleReport::default()
        };
        let mut jobs_remaining = self.config.max_jobs_per_cycle;

        if self.config.summary.enabled
            && jobs_remaining > 0
            && self
                .run_summary_job(&inventory, &mut report)
                .await
                .context("run PCP Summary maintenance job")?
        {
            jobs_remaining -= 1;
        }
        if self.config.compaction.enabled
            && jobs_remaining > 0
            && self
                .run_compaction_job(&inventory, &mut report)
                .await
                .context("run PCP compaction maintenance job")?
        {
            jobs_remaining -= 1;
        }
        if self.config.retention.enabled && jobs_remaining > 0 {
            self.run_retention_job(&mut report)
                .await
                .context("run PCP semantic retention maintenance job")?;
        }

        self.ledger.save(&self.config.state_path).await?;
        Ok(report)
    }

    async fn run_summary_job(
        &mut self,
        inventory: &[pcp_store::DurablePageInventoryItem],
        report: &mut MaintenanceCycleReport,
    ) -> Result<bool> {
        let candidate = inventory.iter().find(|page| {
            page.summary_revision_id.is_none()
                && page.content_chars >= self.config.summary.minimum_chars as u64
                && !excluded_kind(&page.kind, &self.config.summary.excluded_page_kinds)
                && self.ledger.eligible(&summary_key(&page.revision_id))
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
                ensure_nonempty_worker_content(&content)?;
                if self.config.applies_changes() {
                    self.client
                        .write_summary(WriteSummaryRequest {
                            target_page_id: candidate.page_id.clone(),
                            target_revision_id: page_id.clone(),
                            expected_summary_revision_id: None,
                            content,
                            created_by: self.config.worker_actor(),
                            tool_or_model: Some(self.config.worker.actor_id.clone()),
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
            MaintenanceWorkerResponse::KeepSeparate { .. } => {
                if self.config.applies_changes() {
                    self.client
                        .mark_summary_assessed(
                            page_id.clone(),
                            "not_worth_indexing".to_owned(),
                            Some(self.config.worker.actor_id.clone()),
                        )
                        .await?;
                } else {
                    self.ledger.record(
                        summary_key(&page_id),
                        "observed_keep_separate",
                        self.config.summary.retry_after_seconds,
                    );
                }
                report.kept_separate += 1;
            }
            MaintenanceWorkerResponse::Defer { .. } => {
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

    async fn run_compaction_job(
        &mut self,
        inventory: &[pcp_store::DurablePageInventoryItem],
        report: &mut MaintenanceCycleReport,
    ) -> Result<bool> {
        let routing_pages = inventory
            .iter()
            .filter(|page| {
                page.content_chars > 0
                    && !excluded_kind(&page.kind, &self.config.compaction.excluded_page_kinds)
            })
            .take(self.config.compaction.candidate_window)
            .cloned()
            .map(|page| {
                MaintenanceRoutingPage::from_inventory(
                    page,
                    self.config.compaction.routing_chars_per_page,
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

        let routing_by_id = routing_pages
            .iter()
            .map(|page| {
                (
                    page.page_id.clone(),
                    (page.namespace.clone(), page.revision_id.clone()),
                )
            })
            .collect::<HashMap<_, _>>();
        let excluded_candidate_sets = self.ledger.active_compaction_sets();
        report.worker_calls += 1;
        let selection = self
            .worker
            .evaluate(MaintenanceWorkerRequest::SelectConsolidation {
                pages: routing_pages,
                max_pages: self.config.compaction.max_pages_per_candidate,
                excluded_candidate_sets: excluded_candidate_sets.clone(),
            })
            .await?;
        let mut page_ids = match selection {
            MaintenanceWorkerResponse::Candidate { page_ids, .. } => page_ids,
            MaintenanceWorkerResponse::NoCandidate { .. } => {
                self.ledger.record(
                    selection_key,
                    "no_candidate",
                    self.config.compaction.retry_after_seconds,
                );
                return Ok(true);
            }
            MaintenanceWorkerResponse::Defer { .. } => {
                self.ledger.record(
                    selection_key,
                    "deferred",
                    self.config.compaction.retry_after_seconds,
                );
                report.deferred += 1;
                return Ok(true);
            }
            other => anyhow::bail!(
                "semantic worker returned {} for a select_consolidation request",
                response_name(&other)
            ),
        };
        page_ids.sort();
        page_ids.dedup();
        anyhow::ensure!(
            (2..=self.config.compaction.max_pages_per_candidate).contains(&page_ids.len()),
            "semantic worker selected an invalid number of consolidation Pages"
        );
        anyhow::ensure!(
            page_ids
                .iter()
                .all(|page_id| routing_by_id.contains_key(page_id)),
            "semantic worker selected a Page outside the offered candidate window"
        );
        let namespace = &routing_by_id[&page_ids[0]].0;
        anyhow::ensure!(
            page_ids
                .iter()
                .all(|page_id| &routing_by_id[page_id].0 == namespace),
            "semantic worker selected consolidation Pages from different Scopes"
        );
        let key = compaction_key(&page_ids);
        anyhow::ensure!(
            self.ledger.eligible(&key)
                && !excluded_candidate_sets.iter().any(|set| {
                    let mut set = set.clone();
                    set.sort();
                    set == page_ids
                }),
            "semantic worker selected a consolidation candidate still in cooldown"
        );

        let selected_revision_ids = page_ids
            .iter()
            .map(|page_id| routing_by_id[page_id].1.clone())
            .collect();
        let pages = self
            .read_detail_pages(
                selected_revision_ids,
                self.config.compaction.max_input_chars,
            )
            .await?;
        anyhow::ensure!(
            pages.len() == page_ids.len(),
            "one or more consolidation candidates disappeared before evaluation"
        );
        report.worker_calls += 1;
        match self
            .worker
            .evaluate(MaintenanceWorkerRequest::ConsolidatePages {
                pages: pages.clone(),
            })
            .await?
        {
            MaintenanceWorkerResponse::Consolidate {
                canonical_page_id,
                content,
            } => {
                ensure_nonempty_worker_content(&content)?;
                anyhow::ensure!(
                    page_ids.contains(&canonical_page_id),
                    "semantic worker chose a canonical Page outside the candidate set"
                );
                if self.config.applies_changes() {
                    let canonical = pages
                        .iter()
                        .find(|page| page.page_id == canonical_page_id)
                        .context("find canonical consolidation Page")?;
                    let observed_at = pages
                        .iter()
                        .filter_map(|page| page.observed_at.clone())
                        .max();
                    self.client
                        .consolidate_pages(ConsolidatePagesRequest {
                            canonical_page_id: canonical.page_id.clone(),
                            expected_canonical_revision_id: canonical.revision_id.clone(),
                            replaced_pages: pages
                                .iter()
                                .filter(|page| page.page_id != canonical.page_id)
                                .map(|page| pcp_core::ConsolidationInput {
                                    page_id: page.page_id.clone(),
                                    expected_revision_id: page.revision_id.clone(),
                                })
                                .collect(),
                            created_by: self.config.worker_actor(),
                            lifecycle_status: LifecycleStatus::Active,
                            observed_at,
                            valid_from: None,
                            valid_to: None,
                            payload: Some(PagePayload {
                                media_type: "text/markdown".to_owned(),
                                content,
                            }),
                            source_refs: canonical.source_refs.clone(),
                            facets: canonical.facets.clone(),
                            provenance: Vec::new(),
                            idempotency_key: Some(format!(
                                "maintenance:{}",
                                key.trim_start_matches("compaction:")
                            )),
                        })
                        .await?;
                    report.consolidations_committed += 1;
                } else {
                    self.ledger.record(
                        key,
                        "observed_consolidation",
                        self.config.compaction.retry_after_seconds,
                    );
                    report.consolidations_proposed += 1;
                }
            }
            MaintenanceWorkerResponse::KeepSeparate { .. } => {
                self.ledger.record(
                    key,
                    "keep_separate",
                    self.config.compaction.retry_after_seconds,
                );
                report.kept_separate += 1;
            }
            MaintenanceWorkerResponse::Defer { .. } => {
                self.ledger
                    .record(key, "deferred", self.config.compaction.retry_after_seconds);
                report.deferred += 1;
            }
            other => anyhow::bail!(
                "semantic worker returned {} for a consolidate_pages request",
                response_name(&other)
            ),
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
                !excluded_kind(
                    &Some(candidate.kind.clone()),
                    &self.config.retention.excluded_page_kinds,
                )
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
                if self.config.applies_retention_changes() {
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
            MaintenanceWorkerResponse::NoCandidate { .. } => {
                self.ledger.record(
                    key,
                    "no_retention_candidate",
                    self.config.retention.retry_after_seconds,
                );
            }
            MaintenanceWorkerResponse::Defer { .. } => {
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

fn excluded_kind(kind: &Option<String>, excluded: &[String]) -> bool {
    kind.as_ref().is_some_and(|kind| excluded.contains(kind))
}

fn ensure_nonempty_worker_content(content: &str) -> Result<()> {
    anyhow::ensure!(
        !content.trim().is_empty(),
        "semantic maintenance worker returned empty content"
    );
    Ok(())
}

fn response_name(response: &MaintenanceWorkerResponse) -> &'static str {
    match response {
        MaintenanceWorkerResponse::WriteSummary { .. } => "write_summary",
        MaintenanceWorkerResponse::Candidate { .. } => "candidate",
        MaintenanceWorkerResponse::Consolidate { .. } => "consolidate",
        MaintenanceWorkerResponse::Retain { .. } => "retain",
        MaintenanceWorkerResponse::KeepSeparate { .. } => "keep_separate",
        MaintenanceWorkerResponse::NoCandidate { .. } => "no_candidate",
        MaintenanceWorkerResponse::Defer { .. } => "defer",
    }
}
