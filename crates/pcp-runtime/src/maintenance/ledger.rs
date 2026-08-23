use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::SystemTime,
};

use anyhow::{Context, Result};
use pcp_store::DurablePageInventoryItem;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{MaintenanceConfig, MaintenanceCycleReport, WriteTriggeredMaintenanceConfig};

const LEDGER_RETENTION_MILLIS: u64 = 30 * 24 * 60 * 60 * 1_000;

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MaintenanceLedger {
    #[serde(default)]
    entries: BTreeMap<String, MaintenanceLedgerEntry>,
    #[serde(default)]
    write_trigger: WriteTriggerLedger,
    #[serde(default)]
    relation_reviews: BTreeMap<String, MaintenanceRelationReviewProposal>,
    #[serde(default)]
    scheduler: SchedulerLedger,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceAutomationState {
    NotStarted,
    Waiting,
    Running,
    Failed,
    Stale,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceAutomationStatus {
    pub state: MaintenanceAutomationState,
    pub last_started_at: Option<String>,
    pub last_completed_at: Option<String>,
    pub last_error: Option<String>,
    pub last_report: Option<MaintenanceCycleReport>,
    #[serde(default)]
    pub current_report: Option<MaintenanceCycleReport>,
    pub observed_page_count: usize,
    pub dirty_region_count: usize,
    pub ready_region_count: usize,
    pub pending_relation_review_count: usize,
    pub dirty_regions: Vec<MaintenanceDirtyRegionStatus>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceDirtyRegionStatus {
    pub region: String,
    pub new_page_count: usize,
    pub first_dirty_at: String,
    pub last_write_at: String,
    pub ready: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceRelationReviewProposal {
    pub candidate_id: String,
    pub namespace: String,
    pub relation_type: String,
    pub pages: [MaintenanceRelationReviewPage; 2],
    pub proposed_at: String,
    /// Relation proposals reach this queue only when their structural evidence
    /// is not sufficient for unattended assertion. Keep the reason persisted
    /// with the revision-bound proposal so Console can explain the gate.
    #[serde(default)]
    pub risk: String,
    #[serde(default)]
    pub review_reason: String,
    /// Grounded model explanation of why the two revision-bound Pages should
    /// be reviewed as a possible relation. This remains proposal evidence and
    /// is not itself asserted as a Page relation.
    #[serde(default)]
    pub relation_reason: String,
    pub status: MaintenanceRelationReviewStatus,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceRelationReviewPage {
    pub page_id: String,
    pub revision_id: String,
    pub preview: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceRelationReviewStatus {
    Pending,
    Accepted,
    Rejected,
    Suppressed,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct WriteTriggerLedger {
    observed_revisions: BTreeMap<String, String>,
    dirty_regions: BTreeMap<String, DirtyRegion>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DirtyRegion {
    first_dirty_at_unix_ms: u64,
    last_dirty_at_unix_ms: u64,
    new_page_ids: BTreeSet<String>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SchedulerLedger {
    last_started_at_unix_ms: Option<u64>,
    last_completed_at_unix_ms: Option<u64>,
    last_error: Option<String>,
    last_report: Option<MaintenanceCycleReport>,
    #[serde(default)]
    current_report: Option<MaintenanceCycleReport>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct MaintenanceLedgerEntry {
    outcome: String,
    updated_at_unix_ms: u64,
    retry_after_unix_ms: u64,
}

impl MaintenanceLedger {
    pub(crate) async fn load(path: &Path) -> Result<Self> {
        match tokio::fs::read(path).await {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("decode PCP maintenance state {}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => {
                Err(error).with_context(|| format!("read PCP maintenance state {}", path.display()))
            }
        }
    }

    pub(crate) async fn save(&mut self, path: &Path) -> Result<()> {
        let now = now_unix_ms();
        self.entries.retain(|_, entry| {
            entry
                .retry_after_unix_ms
                .saturating_add(LEDGER_RETENTION_MILLIS)
                >= now
        });
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!(
                    "create PCP maintenance state directory {}",
                    parent.display()
                )
            })?;
        }
        let temporary = path.with_extension("tmp");
        let bytes = serde_json::to_vec_pretty(self).context("encode PCP maintenance state")?;
        tokio::fs::write(&temporary, bytes)
            .await
            .with_context(|| format!("write PCP maintenance state {}", temporary.display()))?;
        tokio::fs::rename(&temporary, path)
            .await
            .with_context(|| format!("publish PCP maintenance state {}", path.display()))
    }

    pub(crate) fn eligible(&self, key: &str) -> bool {
        self.entries
            .get(key)
            .is_none_or(|entry| entry.retry_after_unix_ms <= now_unix_ms())
    }

    pub(crate) fn record(&mut self, key: String, outcome: &str, retry_after_seconds: u64) {
        let now = now_unix_ms();
        self.entries.insert(
            key,
            MaintenanceLedgerEntry {
                outcome: outcome.to_owned(),
                updated_at_unix_ms: now,
                retry_after_unix_ms: now.saturating_add(retry_after_seconds.saturating_mul(1_000)),
            },
        );
    }

    pub(crate) fn active_packing_sets(&self) -> Vec<Vec<String>> {
        let now = now_unix_ms();
        self.entries
            .iter()
            .filter(|(key, entry)| key.starts_with("packing:") && entry.retry_after_unix_ms > now)
            .map(|(key, _)| {
                key.trim_start_matches("packing:")
                    .split(',')
                    .map(str::to_owned)
                    .collect()
            })
            .collect()
    }

    pub(crate) fn active_relation_pairs(&self) -> Vec<[String; 2]> {
        let now = now_unix_ms();
        self.entries
            .iter()
            .filter(|(key, entry)| {
                key.starts_with("relation_pair:") && entry.retry_after_unix_ms > now
            })
            .filter_map(|(key, _)| {
                let mut page_ids = key.trim_start_matches("relation_pair:").split(',');
                let first = page_ids.next()?.to_owned();
                let second = page_ids.next()?.to_owned();
                page_ids.next().is_none().then_some([first, second])
            })
            .chain(self.relation_reviews.values().filter_map(|proposal| {
                matches!(
                    proposal.status,
                    MaintenanceRelationReviewStatus::Pending
                        | MaintenanceRelationReviewStatus::Suppressed
                )
                .then(|| relation_review_page_pair(proposal))
            }))
            .collect()
    }

    pub(crate) fn suppressed_relation_pairs(&self) -> Vec<[String; 2]> {
        self.relation_reviews
            .values()
            .filter(|proposal| proposal.status == MaintenanceRelationReviewStatus::Suppressed)
            .map(relation_review_page_pair)
            .collect()
    }

    /// Persist an operator's decision that this exact Page pair must not be
    /// proposed again.  Keep it alongside review decisions rather than in a
    /// separate blacklist: the reviewed revisions remain auditable, while the
    /// decision blocks this exact Page pair and stays out of the pending queue.
    pub(crate) fn suppress_relation_pair(
        &mut self,
        namespace: String,
        pages: [MaintenanceRelationReviewPage; 2],
        relation_reason: String,
    ) -> Result<()> {
        let candidate_id = relation_review_id(&pages);
        match self.relation_reviews.get_mut(&candidate_id) {
            Some(proposal) if proposal.status == MaintenanceRelationReviewStatus::Pending => {
                proposal.status = MaintenanceRelationReviewStatus::Suppressed;
            }
            Some(proposal) if proposal.status == MaintenanceRelationReviewStatus::Suppressed => {}
            Some(_) => {
                anyhow::bail!("PCP relation decision is already resolved and cannot be suppressed")
            }
            None => {
                self.relation_reviews.insert(
                    candidate_id.clone(),
                    MaintenanceRelationReviewProposal {
                        candidate_id,
                        namespace,
                        relation_type: "related_to".to_owned(),
                        pages,
                        proposed_at: chrono::Utc::now().to_rfc3339(),
                        risk: "operator_suppressed".to_owned(),
                        review_reason:
                            "The operator chose not to suggest this exact Page pair again."
                                .to_owned(),
                        relation_reason,
                        status: MaintenanceRelationReviewStatus::Suppressed,
                    },
                );
            }
        }
        Ok(())
    }

    pub(crate) fn propose_relation_review(
        &mut self,
        namespace: String,
        pages: [MaintenanceRelationReviewPage; 2],
        relation_reason: String,
    ) -> String {
        let candidate_id = relation_review_id(&pages);
        self.relation_reviews
            .entry(candidate_id.clone())
            .or_insert_with(|| MaintenanceRelationReviewProposal {
                candidate_id: candidate_id.clone(),
                namespace,
                relation_type: "related_to".to_owned(),
                pages,
                proposed_at: chrono::Utc::now().to_rfc3339(),
                risk: "manual_review".to_owned(),
                review_reason: "The selected Pages are not a continuous Pack boundary with a shared protected identifier.".to_owned(),
                relation_reason,
                status: MaintenanceRelationReviewStatus::Pending,
            });
        candidate_id
    }

    pub(crate) fn relation_reviews(&self) -> Vec<MaintenanceRelationReviewProposal> {
        self.relation_reviews
            .values()
            .filter(|proposal| proposal.status == MaintenanceRelationReviewStatus::Pending)
            .cloned()
            .collect()
    }

    pub(crate) fn relation_review(
        &self,
        candidate_id: &str,
    ) -> Option<MaintenanceRelationReviewProposal> {
        self.relation_reviews.get(candidate_id).cloned()
    }

    pub(crate) fn resolve_relation_review(
        &mut self,
        candidate_id: &str,
        status: MaintenanceRelationReviewStatus,
    ) -> Result<()> {
        let proposal = self
            .relation_reviews
            .get_mut(candidate_id)
            .context("unknown PCP relation review candidate")?;
        anyhow::ensure!(
            proposal.status == MaintenanceRelationReviewStatus::Pending,
            "PCP relation review candidate is no longer pending"
        );
        proposal.status = status;
        Ok(())
    }

    pub(crate) fn start_scheduled_cycle(&mut self) {
        self.scheduler.last_started_at_unix_ms = Some(now_unix_ms());
        self.scheduler.last_error = None;
        self.scheduler.current_report = Some(MaintenanceCycleReport::default());
    }

    pub(crate) fn update_scheduled_cycle(&mut self, report: MaintenanceCycleReport) {
        self.scheduler.current_report = Some(report);
    }

    pub(crate) fn complete_scheduled_cycle(&mut self, report: MaintenanceCycleReport) {
        self.scheduler.last_completed_at_unix_ms = Some(now_unix_ms());
        self.scheduler.last_error = None;
        self.scheduler.last_report = Some(report);
        self.scheduler.current_report = None;
    }

    pub(crate) fn fail_scheduled_cycle(&mut self, error: impl std::fmt::Display) {
        self.scheduler.last_completed_at_unix_ms = Some(now_unix_ms());
        self.scheduler.last_error = Some(error.to_string());
    }

    pub(crate) fn automation_status(
        &self,
        config: &MaintenanceConfig,
    ) -> MaintenanceAutomationStatus {
        let now = now_unix_ms();
        let ready_regions = self.ready_regions_at(&config.write_trigger, now);
        let state = match (
            self.scheduler.last_started_at_unix_ms,
            self.scheduler.last_completed_at_unix_ms,
            self.scheduler.last_error.as_ref(),
        ) {
            (None, _, _) => MaintenanceAutomationState::NotStarted,
            (Some(_), None, _) => MaintenanceAutomationState::Running,
            (Some(started), Some(completed), _) if completed < started => {
                MaintenanceAutomationState::Running
            }
            (_, _, Some(_)) => MaintenanceAutomationState::Failed,
            (_, Some(completed), _)
                if now.saturating_sub(completed)
                    > config
                        .interval_seconds
                        .saturating_mul(2)
                        .saturating_mul(1_000) =>
            {
                MaintenanceAutomationState::Stale
            }
            _ => MaintenanceAutomationState::Waiting,
        };
        let dirty_regions = self
            .write_trigger
            .dirty_regions
            .iter()
            .map(|(region, dirty)| MaintenanceDirtyRegionStatus {
                region: region.clone(),
                new_page_count: dirty.new_page_ids.len(),
                first_dirty_at: timestamp_string(dirty.first_dirty_at_unix_ms),
                last_write_at: timestamp_string(dirty.last_dirty_at_unix_ms),
                ready: ready_regions.contains(region),
            })
            .collect();
        MaintenanceAutomationStatus {
            state,
            last_started_at: self.scheduler.last_started_at_unix_ms.map(timestamp_string),
            last_completed_at: self
                .scheduler
                .last_completed_at_unix_ms
                .map(timestamp_string),
            last_error: self.scheduler.last_error.clone(),
            last_report: self.scheduler.last_report.clone(),
            current_report: self.scheduler.current_report.clone(),
            observed_page_count: self.write_trigger.observed_revisions.len(),
            dirty_region_count: self.write_trigger.dirty_regions.len(),
            ready_region_count: ready_regions.len(),
            pending_relation_review_count: self.relation_reviews().len(),
            dirty_regions,
        }
    }

    /// Records only current Page-head changes.  The first observation establishes
    /// a watermark; it deliberately does not turn a pre-existing Store backlog
    /// into an automatic semantic-maintenance run.
    pub(crate) fn observe_writes(
        &mut self,
        inventory: &[DurablePageInventoryItem],
        config: &WriteTriggeredMaintenanceConfig,
    ) -> BTreeSet<String> {
        let now = now_unix_ms();
        let current = inventory
            .iter()
            .map(|page| (page.page_id.clone(), page.revision_id.clone()))
            .collect::<BTreeMap<_, _>>();
        if self.write_trigger.observed_revisions.is_empty() {
            self.write_trigger.observed_revisions = current;
            return BTreeSet::new();
        }
        for page in inventory.iter().filter(|page| {
            self.write_trigger.observed_revisions.get(&page.page_id) != Some(&page.revision_id)
        }) {
            let region = maintenance_region_key(page);
            let dirty = self
                .write_trigger
                .dirty_regions
                .entry(region)
                .or_insert_with(|| DirtyRegion {
                    first_dirty_at_unix_ms: now,
                    last_dirty_at_unix_ms: now,
                    new_page_ids: BTreeSet::new(),
                });
            if !self
                .write_trigger
                .observed_revisions
                .contains_key(&page.page_id)
            {
                dirty.new_page_ids.insert(page.page_id.clone());
            }
            dirty.last_dirty_at_unix_ms = now;
        }
        self.write_trigger.observed_revisions = current;
        self.ready_regions_at(config, now)
    }

    pub(crate) fn ready_regions(
        &self,
        config: &WriteTriggeredMaintenanceConfig,
    ) -> BTreeSet<String> {
        self.ready_regions_at(config, now_unix_ms())
    }

    fn ready_regions_at(
        &self,
        config: &WriteTriggeredMaintenanceConfig,
        now: u64,
    ) -> BTreeSet<String> {
        self.write_trigger
            .dirty_regions
            .iter()
            .filter(|(_, dirty)| {
                let quiet_for = now.saturating_sub(dirty.last_dirty_at_unix_ms);
                let waiting_for = now.saturating_sub(dirty.first_dirty_at_unix_ms);
                (dirty.new_page_ids.len() >= config.min_new_pages
                    && quiet_for >= config.quiet_period_seconds.saturating_mul(1_000))
                    || waiting_for >= config.max_wait_seconds.saturating_mul(1_000)
            })
            .map(|(region, _)| region.clone())
            .collect()
    }

    pub(crate) fn region_snapshot(
        inventory: &[DurablePageInventoryItem],
        regions: &BTreeSet<String>,
    ) -> BTreeMap<String, BTreeMap<String, String>> {
        let mut snapshot = BTreeMap::new();
        for page in inventory {
            let region = maintenance_region_key(page);
            if regions.contains(&region) {
                snapshot
                    .entry(region)
                    .or_insert_with(BTreeMap::new)
                    .insert(page.page_id.clone(), page.revision_id.clone());
            }
        }
        snapshot
    }

    /// Clears only regions whose exact Page-head snapshot survived the job.
    /// A concurrent write (or a maintenance write that needs a follow-up pass)
    /// remains dirty and is eligible after another quiet window.
    pub(crate) fn acknowledge_unchanged_regions(
        &mut self,
        expected: &BTreeMap<String, BTreeMap<String, String>>,
        inventory: &[DurablePageInventoryItem],
    ) {
        let current = Self::region_snapshot(
            inventory,
            &expected.keys().cloned().collect::<BTreeSet<_>>(),
        );
        self.write_trigger
            .dirty_regions
            .retain(|region, _| expected.get(region) != current.get(region));
    }
}

pub(crate) fn maintenance_region_key(page: &DurablePageInventoryItem) -> String {
    match page.source_span.as_ref() {
        Some(source) => format!("stream:{}:{}", page.namespace, source.stream_id),
        None => format!("page:{}:{}", page.namespace, page.page_id),
    }
}

fn relation_review_id(pages: &[MaintenanceRelationReviewPage; 2]) -> String {
    let mut revisions = pages
        .iter()
        .map(|page| page.revision_id.as_str())
        .collect::<Vec<_>>();
    revisions.sort_unstable();
    let mut digest = Sha256::new();
    for revision in revisions {
        digest.update(revision.as_bytes());
        digest.update([0]);
    }
    let encoded = format!("mrr_{:x}", digest.finalize());
    encoded[..28].to_owned()
}

fn relation_review_page_pair(proposal: &MaintenanceRelationReviewProposal) -> [String; 2] {
    [
        proposal.pages[0].page_id.clone(),
        proposal.pages[1].page_id.clone(),
    ]
}

pub(crate) fn summary_key(page_id: &str) -> String {
    format!("summary:{page_id}")
}

pub(crate) fn packing_key(page_ids: &[String]) -> String {
    format!("packing:{}", page_ids.join(","))
}

pub(crate) fn selection_window_key(page_ids: &[String]) -> String {
    let mut page_ids = page_ids.to_vec();
    page_ids.sort();
    page_ids.dedup();
    format!("selection_window:{}", page_ids.join(","))
}

pub(crate) fn retention_window_key(revision_ids: &[String]) -> String {
    let mut revision_ids = revision_ids.to_vec();
    revision_ids.sort();
    revision_ids.dedup();
    format!("retention_window:{}", revision_ids.join(","))
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn timestamp_string(unix_ms: u64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(unix_ms as i64)
        .map(|timestamp| timestamp.to_rfc3339())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use pcp_core::PageMutability;

    use super::*;

    fn page(id: &str, revision: &str) -> DurablePageInventoryItem {
        DurablePageInventoryItem {
            page_id: id.to_owned(),
            revision_id: revision.to_owned(),
            namespace: "conversation:test".to_owned(),
            kind: "conversation_event".to_owned(),
            mutability: PageMutability::Sealed,
            created_at: "2026-08-18T00:00:00Z".to_owned(),
            observed_at: None,
            source_span: Some(pcp_core::SourceSpan {
                stream_id: "conversation:write-trigger".to_owned(),
                start: id.parse().unwrap_or(0),
                end: id.parse().unwrap_or(0),
            }),
            media_type: Some("text/plain".to_owned()),
            content_chars: 10,
            snippet: "event".to_owned(),
            facets: None,
            summary_revision_id: None,
            summary_target_revision_id: None,
            summary: None,
            relation_types: Vec::new(),
            packing_protected: false,
        }
    }

    #[test]
    fn write_trigger_establishes_a_baseline_then_waits_for_a_new_page() {
        let mut ledger = MaintenanceLedger::default();
        let trigger = WriteTriggeredMaintenanceConfig {
            min_new_pages: 1,
            quiet_period_seconds: 0,
            max_wait_seconds: 0,
        };
        let first = page("1", "rev_1");
        assert!(ledger.observe_writes(&[first.clone()], &trigger).is_empty());

        let second = page("2", "rev_2");
        let ready = ledger.observe_writes(&[first, second.clone()], &trigger);
        let expected = BTreeSet::from([maintenance_region_key(&second)]);
        assert_eq!(ready, expected);
    }

    #[test]
    fn unchanged_processed_region_is_acknowledged() {
        let mut ledger = MaintenanceLedger::default();
        let trigger = WriteTriggeredMaintenanceConfig {
            min_new_pages: 1,
            quiet_period_seconds: 0,
            max_wait_seconds: 0,
        };
        let first = page("1", "rev_1");
        let second = page("2", "rev_2");
        let _ = ledger.observe_writes(&[first.clone()], &trigger);
        let regions = ledger.observe_writes(&[first.clone(), second.clone()], &trigger);
        let expected =
            MaintenanceLedger::region_snapshot(&[first.clone(), second.clone()], &regions);
        ledger.acknowledge_unchanged_regions(&expected, &[first, second]);
        assert!(ledger.ready_regions(&trigger).is_empty());
    }
}
