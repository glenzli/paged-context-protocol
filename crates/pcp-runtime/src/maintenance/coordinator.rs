use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use chrono::{Duration as ChronoDuration, SecondsFormat, Utc};
use pcp_client::PcpApi;
use pcp_core::{
    LinkPagesRequest, PACKED_PAGE_MEDIA_TYPE, PackPagesRequest, PageMutability, PageRevisionRef,
    PlanRevisionRetentionRequest, Projection, PutRevisionRetentionLeaseRequest, ReadPagesRequest,
    RetentionPolicy, WriteSummaryRequest,
};
use serde::{Deserialize, Serialize};

use super::{
    MaintenanceConfig, MaintenanceMode, MaintenanceWorkerRequest, MaintenanceWorkerResponse,
    PackingMaintenanceConfig, RelationMaintenanceConfig, RetentionMilestone,
    SemanticMaintenanceWorker,
    ledger::{
        MaintenanceLedger, packing_key, retention_window_key, selection_window_key, summary_key,
    },
    worker::{
        MaintenanceDetailPage, MaintenanceRoutingPage, PackingCandidatePage, RelationCandidatePage,
    },
};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
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

const PACKING_CANDIDATE_WINDOW: usize = 32;
const PACKING_ROUTING_CHARS: usize = 480;
const PACKING_RETRY_AFTER_SECONDS: u64 = 86_400;

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
            let retry_soon = match self.run_once().await {
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
        self.run_once_inner(true).await
    }

    pub async fn run_operator_observe_once(&mut self) -> Result<MaintenanceCycleReport> {
        anyhow::ensure!(
            self.config.mode == MaintenanceMode::Observe,
            "operator maintenance run-once only permits observe mode"
        );
        self.run_once_inner(false).await
    }

    async fn run_once_inner(&mut self, persist_ledger: bool) -> Result<MaintenanceCycleReport> {
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
        if self.config.packing.enabled
            && jobs_remaining > 0
            && self
                .run_packing_job(&inventory, &mut report)
                .await
                .context("run PCP packing maintenance job")?
        {
            jobs_remaining -= 1;
        }
        if self.config.relation.enabled
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
        let candidates = relation_candidate_window(
            inventory,
            &self.config.relation,
            self.config.packing.enabled,
        );
        if candidates.len() < 2 {
            return Ok(false);
        }
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
        if !self.ledger.eligible(&window_key) {
            return Ok(false);
        }

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
            self.ledger.record(
                window_key,
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
        self.ledger.record(
            pair_key,
            if self.config.applies_changes() {
                "related"
            } else {
                "observed_relation"
            },
            self.config.relation.retry_after_seconds,
        );
        self.ledger.record(
            window_key,
            if self.config.applies_changes() {
                "related"
            } else {
                "observed_relation"
            },
            self.config.relation.retry_after_seconds,
        );
        Ok(true)
    }

    async fn run_packing_job(
        &mut self,
        inventory: &[pcp_store::DurablePageInventoryItem],
        report: &mut MaintenanceCycleReport,
    ) -> Result<bool> {
        let Some(candidates) = packing_candidate_window(inventory, &self.config.packing) else {
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
        anyhow::ensure!(
            unique.len() == page_ids.len()
                && (2..=self.config.packing.max_pages).contains(&page_ids.len()),
            "semantic worker selected an invalid number of unique packing Pages"
        );
        anyhow::ensure!(
            page_ids
                .iter()
                .all(|page_id| routing_by_id.contains_key(page_id)),
            "semantic worker selected a Page outside the offered packing window"
        );
        let positions = page_ids
            .iter()
            .map(|page_id| routing_by_id[page_id].0)
            .collect::<Vec<_>>();
        anyhow::ensure!(
            positions.windows(2).all(|pair| pair[0] + 1 == pair[1]),
            "semantic worker must return an ordered contiguous packing subset"
        );
        let key = packing_key(&page_ids);
        anyhow::ensure!(
            self.ledger.eligible(&key)
                && !excluded_candidate_sets.iter().any(|set| {
                    let mut set = set.clone();
                    set.sort();
                    set == page_ids
                }),
            "semantic worker selected a packing candidate still in cooldown"
        );
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
}

fn relation_candidate_window(
    inventory: &[pcp_store::DurablePageInventoryItem],
    config: &RelationMaintenanceConfig,
    packing_enabled: bool,
) -> Vec<pcp_store::DurablePageInventoryItem> {
    inventory
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
        .take(config.candidate_window)
        .cloned()
        .collect()
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

fn packing_candidate_window(
    inventory: &[pcp_store::DurablePageInventoryItem],
    config: &PackingMaintenanceConfig,
) -> Option<Vec<pcp_store::DurablePageInventoryItem>> {
    let eligible = |page: &&pcp_store::DurablePageInventoryItem| {
        let packed = page.media_type.as_deref() == Some(PACKED_PAGE_MEDIA_TYPE);
        let valid_shape = if packed {
            page.mutability == PageMutability::Revisioned
        } else {
            page.mutability == PageMutability::Sealed
                && page.summary_revision_id.is_none()
                && page.relation_types.is_empty()
        };
        valid_shape
            && page.source_span.is_some()
            && page.content_chars > 0
            && !excluded_kind(&page.kind, &config.excluded_page_kinds)
    };
    let mut visited = std::collections::HashSet::new();
    for seed in inventory.iter().filter(eligible) {
        let span = seed.source_span.as_ref()?;
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

        let mut best = Vec::new();
        let mut run = Vec::new();
        let mut run_chars = 0_u64;
        let mut run_anchors = 0_usize;
        for page in group {
            if page.content_chars > u64::from(config.max_input_chars) {
                if run.len() >= 2 {
                    best = std::mem::take(&mut run);
                } else {
                    run.clear();
                }
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
                if run.len() >= 2 {
                    best = std::mem::take(&mut run);
                } else {
                    run.clear();
                }
                run_chars = 0;
                run_anchors = 0;
            }
            run_chars = run_chars.saturating_add(page.content_chars);
            run_anchors += usize::from(page_is_anchor);
            run.push(page);
        }
        if run.len() >= 2 {
            best = run;
        }
        if best.len() >= 2 {
            best.truncate(PACKING_CANDIDATE_WINDOW.min(config.max_pages));
            return Some(best);
        }
    }
    None
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
        MaintenanceWorkerResponse::Relate { .. } => "relate",
        MaintenanceWorkerResponse::Retain { .. } => "retain",
        MaintenanceWorkerResponse::NoCandidate => "no_candidate",
        MaintenanceWorkerResponse::Defer => "defer",
    }
}
