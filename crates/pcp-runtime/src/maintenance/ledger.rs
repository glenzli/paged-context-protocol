use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs::OpenOptions,
    os::fd::AsRawFd,
    path::Path,
    time::SystemTime,
};

use anyhow::{Context, Result};
use pcp_store::DurablePageInventoryItem;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    MaintenanceConfig, MaintenanceCycleReport, WriteTriggeredMaintenanceConfig,
    review::{
        MaintenanceReviewItem, MaintenanceReviewOrigin, MaintenanceReviewPayload,
        MaintenanceReviewStatus,
    },
};

const LEDGER_RETENTION_MILLIS: u64 = 30 * 24 * 60 * 60 * 1_000;
const ACTIVE_RETRY_SECONDS: u64 = 30;
const ERROR_RETRY_MAX_SECONDS: u64 = 30 * 60;

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
    review_items: BTreeMap<String, MaintenanceReviewItem>,
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
    #[serde(default)]
    pub next_wake_at: Option<String>,
    #[serde(default)]
    pub last_wake_reason: Option<MaintenanceWakeReason>,
    #[serde(default)]
    pub idle_cycles: u32,
    #[serde(default)]
    pub consecutive_failures: u32,
    pub observed_page_count: usize,
    pub dirty_region_count: usize,
    pub ready_region_count: usize,
    pub pending_relation_review_count: usize,
    pub pending_review_count: usize,
    pub dirty_regions: Vec<MaintenanceDirtyRegionStatus>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceWakeReason {
    Startup,
    Timer,
    ExternalWrite,
    ActiveRetry,
    ErrorRetry,
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
    #[serde(default = "default_model_attempts")]
    pub model_attempts: u32,
    #[serde(default)]
    pub escalated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snoozed_until: Option<String>,
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
    Deferred,
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
    #[serde(default)]
    next_wake_at_unix_ms: Option<u64>,
    #[serde(default)]
    last_wake_reason: Option<MaintenanceWakeReason>,
    #[serde(default)]
    idle_cycles: u32,
    #[serde(default)]
    consecutive_failures: u32,
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
        let _lock = MaintenanceLedgerLock::acquire(path)?;
        if let Ok(bytes) = tokio::fs::read(path).await
            && let Ok(persisted) = serde_json::from_slice::<Self>(&bytes)
        {
            self.merge_persisted_reviews(persisted);
        }
        self.write_locked(path).await
    }

    /// Inbox refresh is not a scheduling run. Publish only its stale review
    /// transitions under the same lock, retaining concurrent cadence/work state.
    pub(crate) async fn persist_stale_reviews(
        &mut self,
        path: &Path,
        ids: &[String],
    ) -> Result<()> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let _lock = MaintenanceLedgerLock::acquire(path)?;
        let mut persisted = Self::load(path).await?;
        for id in ids {
            let Some(local) = self.review_items.get(id) else {
                continue;
            };
            anyhow::ensure!(
                local.status == MaintenanceReviewStatus::Stale,
                "inbox refresh can only persist stale reviews"
            );
            if let Some(current) = persisted.review_items.get(id)
                && current.status != MaintenanceReviewStatus::Pending
            {
                // Do not rewrite a completed decision from another operator.
                self.review_items.insert(id.clone(), current.clone());
                continue;
            }
            persisted.review_items.insert(id.clone(), local.clone());
        }
        persisted.write_locked(path).await
    }

    // Caller owns MaintenanceLedgerLock for this path.
    async fn write_locked(&self, path: &Path) -> Result<()> {
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        let bytes = serde_json::to_vec_pretty(self).context("encode PCP maintenance state")?;
        tokio::fs::write(&temporary, bytes)
            .await
            .with_context(|| format!("write PCP maintenance state {}", temporary.display()))?;
        tokio::fs::rename(&temporary, path)
            .await
            .with_context(|| format!("publish PCP maintenance state {}", path.display()))
    }

    fn merge_persisted_reviews(&mut self, persisted: Self) {
        for (candidate_id, persisted_item) in persisted.review_items {
            match self.review_items.get(&candidate_id) {
                Some(local_item) if local_item.updated_at >= persisted_item.updated_at => {}
                _ => {
                    self.review_items.insert(candidate_id, persisted_item);
                }
            }
        }
        for (candidate_id, persisted_proposal) in persisted.relation_reviews {
            match self.relation_reviews.get(&candidate_id) {
                Some(local_proposal)
                    if local_proposal.status != MaintenanceRelationReviewStatus::Pending => {}
                Some(_)
                    if persisted_proposal.status == MaintenanceRelationReviewStatus::Pending => {}
                _ => {
                    self.relation_reviews
                        .insert(candidate_id, persisted_proposal);
                }
            }
        }
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

    pub(crate) fn rejected_relation_pairs(
        &self,
        current_revision_ids: &HashMap<String, String>,
    ) -> Vec<[String; 2]> {
        self.relation_reviews
            .values()
            .filter(|proposal| proposal.status == MaintenanceRelationReviewStatus::Rejected)
            .filter(|proposal| {
                proposal
                    .pages
                    .iter()
                    .all(|page| current_revision_ids.get(&page.page_id) == Some(&page.revision_id))
            })
            .map(relation_review_page_pair)
            .collect()
    }

    pub(crate) fn relation_pair_is_rejected(
        &self,
        page_ids: &[String; 2],
        revision_ids: &[String; 2],
    ) -> bool {
        let current_revision_ids = page_ids
            .iter()
            .cloned()
            .zip(revision_ids.iter().cloned())
            .collect::<HashMap<_, _>>();
        self.rejected_relation_pairs(&current_revision_ids)
            .into_iter()
            .any(|pair| pair == *page_ids)
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
        self.record_relation_pair_decision(
            namespace,
            pages,
            relation_reason,
            MaintenanceRelationReviewStatus::Suppressed,
            "operator_suppressed",
            "The operator chose not to suggest this exact Page pair again.",
        )
    }

    /// Persist a negative decision for the exact reviewed revisions. Unlike a
    /// suppression, a later revision of either Page may be reviewed again.
    pub(crate) fn reject_relation_pair(
        &mut self,
        namespace: String,
        pages: [MaintenanceRelationReviewPage; 2],
        relation_reason: String,
    ) -> Result<()> {
        self.record_relation_pair_decision(
            namespace,
            pages,
            relation_reason,
            MaintenanceRelationReviewStatus::Rejected,
            "operator_rejected",
            "The operator rejected this revision-bound Page relation.",
        )
    }

    fn record_relation_pair_decision(
        &mut self,
        namespace: String,
        pages: [MaintenanceRelationReviewPage; 2],
        relation_reason: String,
        status: MaintenanceRelationReviewStatus,
        risk: &str,
        review_reason: &str,
    ) -> Result<()> {
        anyhow::ensure!(
            matches!(
                status,
                MaintenanceRelationReviewStatus::Rejected
                    | MaintenanceRelationReviewStatus::Suppressed
            ),
            "unsupported PCP relation decision"
        );
        let candidate_id = relation_review_id(&pages);
        match self.relation_reviews.get_mut(&candidate_id) {
            Some(proposal) if proposal.status == MaintenanceRelationReviewStatus::Pending => {
                proposal.status = status;
            }
            Some(proposal) if proposal.status == status => {}
            Some(_) => {
                anyhow::bail!("PCP relation decision is already resolved")
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
                        risk: risk.to_owned(),
                        review_reason: review_reason.to_owned(),
                        relation_reason,
                        model_attempts: 1,
                        escalated: false,
                        snoozed_until: None,
                        status,
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
        model_attempts: u32,
        escalated: bool,
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
                model_attempts: model_attempts.max(1),
                escalated,
                snoozed_until: None,
                status: MaintenanceRelationReviewStatus::Pending,
            });
        candidate_id
    }

    pub(crate) fn relation_reviews(&self) -> Vec<MaintenanceRelationReviewProposal> {
        self.relation_reviews
            .values()
            .filter(|proposal| {
                proposal.status == MaintenanceRelationReviewStatus::Pending
                    && snooze_is_visible(proposal.snoozed_until.as_deref())
            })
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

    pub(crate) fn enqueue_review(
        &mut self,
        payload: MaintenanceReviewPayload,
        origin: MaintenanceReviewOrigin,
        reason: String,
        model_attempts: u32,
        escalated: bool,
    ) -> String {
        let candidate_id = payload.candidate_id().to_owned();
        let retry_reconciliation = matches!(payload, MaintenanceReviewPayload::Reconciliation(_))
            && self
                .review_items
                .get(&candidate_id)
                .is_some_and(|item| item.status != MaintenanceReviewStatus::Pending);
        if retry_reconciliation {
            self.review_items.insert(
                candidate_id.clone(),
                MaintenanceReviewItem::pending(payload, origin, reason, model_attempts, escalated),
            );
            return candidate_id;
        }
        self.review_items
            .entry(candidate_id.clone())
            .or_insert_with(|| {
                MaintenanceReviewItem::pending(payload, origin, reason, model_attempts, escalated)
            });
        candidate_id
    }

    pub(crate) fn review_items(&self) -> Vec<MaintenanceReviewItem> {
        self.review_items
            .values()
            .filter(|item| {
                item.status == MaintenanceReviewStatus::Pending
                    && snooze_is_visible(item.snoozed_until.as_deref())
            })
            .cloned()
            .collect()
    }

    pub(crate) fn review_item(&self, candidate_id: &str) -> Option<MaintenanceReviewItem> {
        self.review_items.get(candidate_id).cloned()
    }

    pub(crate) fn pending_feedback_reviews(&self) -> Vec<(String, String, String)> {
        self.review_items
            .values()
            .filter_map(|item| {
                // Include snoozed reviews: they must not become actionable later.
                if item.status != MaintenanceReviewStatus::Pending {
                    return None;
                }
                let MaintenanceReviewPayload::Reconciliation(candidate) = &item.payload else {
                    return None;
                };
                let signal = candidate.signal.as_ref()?;
                Some((
                    item.candidate_id.clone(),
                    signal.feedback_page_id.clone(),
                    signal.feedback_revision_id.clone(),
                ))
            })
            .collect()
    }

    pub(crate) fn resolve_review(
        &mut self,
        candidate_id: &str,
        status: MaintenanceReviewStatus,
    ) -> Result<()> {
        anyhow::ensure!(
            status != MaintenanceReviewStatus::Pending,
            "PCP maintenance review resolution cannot remain pending"
        );
        let item = self
            .review_items
            .get_mut(candidate_id)
            .context("unknown PCP maintenance review candidate")?;
        anyhow::ensure!(
            item.status == MaintenanceReviewStatus::Pending,
            "PCP maintenance review candidate is no longer pending"
        );
        item.status = status;
        item.updated_at = chrono::Utc::now().to_rfc3339();
        Ok(())
    }

    pub(crate) fn snooze_review(&mut self, candidate_id: &str, seconds: u64) -> Result<()> {
        let until =
            chrono::Utc::now() + chrono::Duration::seconds(seconds.try_into().unwrap_or(i64::MAX));
        if let Some(item) = self.review_items.get_mut(candidate_id) {
            anyhow::ensure!(
                item.status == MaintenanceReviewStatus::Pending,
                "PCP maintenance review candidate is no longer pending"
            );
            item.snoozed_until = Some(until.to_rfc3339());
            item.updated_at = chrono::Utc::now().to_rfc3339();
            return Ok(());
        }
        let proposal = self
            .relation_reviews
            .get_mut(candidate_id)
            .context("unknown PCP maintenance review candidate")?;
        anyhow::ensure!(
            proposal.status == MaintenanceRelationReviewStatus::Pending,
            "PCP maintenance review candidate is no longer pending"
        );
        proposal.snoozed_until = Some(until.to_rfc3339());
        Ok(())
    }

    pub(crate) fn start_scheduled_cycle(&mut self, wake_reason: MaintenanceWakeReason) {
        self.scheduler.last_started_at_unix_ms = Some(now_unix_ms());
        self.scheduler.last_error = None;
        self.scheduler.current_report = Some(MaintenanceCycleReport::default());
        self.scheduler.next_wake_at_unix_ms = None;
        self.scheduler.last_wake_reason = Some(wake_reason);
        if wake_reason == MaintenanceWakeReason::ExternalWrite {
            self.scheduler.idle_cycles = 0;
        }
    }

    pub(crate) fn update_scheduled_cycle(&mut self, report: MaintenanceCycleReport) {
        self.scheduler.current_report = Some(report);
    }

    pub(crate) fn complete_scheduled_cycle(&mut self, report: MaintenanceCycleReport) {
        self.scheduler.last_completed_at_unix_ms = Some(now_unix_ms());
        self.scheduler.last_error = None;
        self.scheduler.last_report = Some(report);
        self.scheduler.current_report = None;
        self.scheduler.consecutive_failures = 0;
    }

    pub(crate) fn fail_scheduled_cycle(&mut self, error: impl std::fmt::Display) {
        self.scheduler.last_completed_at_unix_ms = Some(now_unix_ms());
        self.scheduler.last_error = Some(error.to_string());
    }

    pub(crate) fn schedule_after_success(
        &mut self,
        config: &MaintenanceConfig,
        report: &MaintenanceCycleReport,
    ) -> u64 {
        self.scheduler.consecutive_failures = 0;
        let delay = if report.jobs_advanced >= config.max_jobs_per_cycle {
            self.scheduler.idle_cycles = 0;
            ACTIVE_RETRY_SECONDS
        } else if !self.write_trigger.dirty_regions.is_empty() {
            self.scheduler.idle_cycles = 0;
            if report.jobs_advanced > 0 {
                ACTIVE_RETRY_SECONDS
            } else {
                self.next_dirty_deadline_seconds(&config.write_trigger)
                    .unwrap_or(config.interval_seconds)
            }
        } else if report.jobs_advanced > 0 {
            self.scheduler.idle_cycles = 0;
            config.interval_seconds
        } else {
            self.scheduler.idle_cycles = self.scheduler.idle_cycles.saturating_add(1);
            exponential_delay(
                config.interval_seconds,
                config.max_interval_seconds,
                self.scheduler.idle_cycles.saturating_sub(1),
            )
        };
        self.record_next_wake(delay);
        delay
    }

    pub(crate) fn has_dirty_regions(&self) -> bool {
        !self.write_trigger.dirty_regions.is_empty()
    }

    pub(crate) fn schedule_initial_wake(&mut self, delay_seconds: u64) {
        self.record_next_wake(delay_seconds);
    }

    pub(crate) fn schedule_after_failure(&mut self, config: &MaintenanceConfig) -> u64 {
        self.scheduler.idle_cycles = 0;
        self.scheduler.consecutive_failures = self.scheduler.consecutive_failures.saturating_add(1);
        let ceiling = config.interval_seconds.min(ERROR_RETRY_MAX_SECONDS).max(1);
        let delay = exponential_delay(
            ACTIVE_RETRY_SECONDS.min(ceiling),
            ceiling,
            self.scheduler.consecutive_failures.saturating_sub(1),
        );
        self.record_next_wake(delay);
        delay
    }

    fn record_next_wake(&mut self, delay_seconds: u64) {
        self.scheduler.next_wake_at_unix_ms =
            Some(now_unix_ms().saturating_add(delay_seconds.saturating_mul(1_000)));
    }

    fn next_dirty_deadline_seconds(&self, config: &WriteTriggeredMaintenanceConfig) -> Option<u64> {
        let now = now_unix_ms();
        self.write_trigger
            .dirty_regions
            .values()
            .map(|dirty| {
                let max_wait_deadline = dirty
                    .first_dirty_at_unix_ms
                    .saturating_add(config.max_wait_seconds.saturating_mul(1_000));
                let deadline = if dirty.new_page_ids.len() >= config.min_new_pages {
                    max_wait_deadline.min(
                        dirty
                            .last_dirty_at_unix_ms
                            .saturating_add(config.quiet_period_seconds.saturating_mul(1_000)),
                    )
                } else {
                    max_wait_deadline
                };
                deadline.saturating_sub(now).div_ceil(1_000).max(1)
            })
            .min()
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
                        .max_interval_seconds
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
            next_wake_at: self.scheduler.next_wake_at_unix_ms.map(timestamp_string),
            last_wake_reason: self.scheduler.last_wake_reason,
            idle_cycles: self.scheduler.idle_cycles,
            consecutive_failures: self.scheduler.consecutive_failures,
            observed_page_count: self.write_trigger.observed_revisions.len(),
            dirty_region_count: self.write_trigger.dirty_regions.len(),
            ready_region_count: ready_regions.len(),
            pending_relation_review_count: self.relation_reviews().len(),
            pending_review_count: self
                .review_items()
                .len()
                .saturating_add(self.relation_reviews().len()),
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

fn exponential_delay(base: u64, ceiling: u64, exponent: u32) -> u64 {
    base.saturating_mul(1_u64.checked_shl(exponent.min(63)).unwrap_or(u64::MAX))
        .min(ceiling)
}

struct MaintenanceLedgerLock(std::fs::File);

impl MaintenanceLedgerLock {
    fn acquire(path: &Path) -> Result<Self> {
        let lock_path = path.with_extension("lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("open PCP maintenance lock {}", lock_path.display()))?;
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        anyhow::ensure!(
            result == 0,
            "lock PCP maintenance state {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        );
        Ok(Self(file))
    }
}

impl Drop for MaintenanceLedgerLock {
    fn drop(&mut self) {
        let _ = unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
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

fn default_model_attempts() -> u32 {
    1
}

fn snooze_is_visible(value: Option<&str>) -> bool {
    value
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .is_none_or(|until| until <= chrono::Utc::now())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use pcp_core::PageMutability;

    use super::*;
    use crate::maintenance::{
        MaintenanceMode, MaintenanceWorkerConfig, PackingMaintenanceConfig,
        ReconciliationMaintenanceConfig, RelationMaintenanceConfig, RetentionMaintenanceConfig,
        SummaryMaintenanceConfig,
    };

    fn scheduler_config() -> MaintenanceConfig {
        MaintenanceConfig {
            enabled: true,
            mode: MaintenanceMode::Apply,
            state_path: PathBuf::from("maintenance-test.json"),
            allowed_scopes: vec!["conversation:test".to_owned()],
            interval_seconds: 10,
            max_interval_seconds: 80,
            initial_delay_seconds: 0,
            write_trigger: WriteTriggeredMaintenanceConfig {
                min_new_pages: 8,
                quiet_period_seconds: 10,
                max_wait_seconds: 60,
            },
            max_jobs_per_cycle: 2,
            principal_id: "service:test-maintainer".to_owned(),
            principal_name: "Test maintainer".to_owned(),
            worker: MaintenanceWorkerConfig::Command {
                program: PathBuf::from("/bin/false"),
                args: Vec::new(),
                timeout_seconds: 1,
                actor_id: "model:test-maintainer".to_owned(),
                actor_type: "model".to_owned(),
            },
            summary: SummaryMaintenanceConfig::default(),
            packing: PackingMaintenanceConfig::default(),
            relation: RelationMaintenanceConfig::default(),
            reconciliation: ReconciliationMaintenanceConfig::default(),
            retention: RetentionMaintenanceConfig::default(),
        }
    }

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
            provenance_input_revision_ids: Vec::new(),
            topic_source_page_ids: Vec::new(),
            superseded: false,
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
    fn persisted_review_resolution_wins_over_a_stale_pending_writer() {
        let payload =
            MaintenanceReviewPayload::Summary(crate::maintenance::MaintenanceSummaryCandidate {
                candidate_id: "msu_test".to_owned(),
                page_id: "pg_1".to_owned(),
                revision_id: "rev_1".to_owned(),
                namespace: "conversation:test".to_owned(),
                content_chars: 100,
                expected_summary_revision_id: None,
                content: "A bounded routing summary.".to_owned(),
            });
        let mut stale = MaintenanceLedger::default();
        stale.enqueue_review(
            payload.clone(),
            MaintenanceReviewOrigin::Automatic,
            "review".to_owned(),
            1,
            false,
        );
        let mut persisted = MaintenanceLedger::default();
        persisted.enqueue_review(
            payload,
            MaintenanceReviewOrigin::Automatic,
            "review".to_owned(),
            1,
            false,
        );
        persisted
            .resolve_review("msu_test", MaintenanceReviewStatus::Accepted)
            .expect("resolve persisted review");
        persisted
            .review_items
            .get_mut("msu_test")
            .expect("persisted review")
            .updated_at = "9999-01-01T00:00:00Z".to_owned();

        stale.merge_persisted_reviews(persisted);

        assert_eq!(
            stale.review_item("msu_test").expect("merged review").status,
            MaintenanceReviewStatus::Accepted
        );
    }

    #[test]
    fn snoozing_keeps_a_review_unresolved_but_out_of_the_current_inbox() {
        let mut ledger = MaintenanceLedger::default();
        ledger.enqueue_review(
            MaintenanceReviewPayload::Summary(crate::maintenance::MaintenanceSummaryCandidate {
                candidate_id: "msu_snooze".to_owned(),
                page_id: "pg_1".to_owned(),
                revision_id: "rev_1".to_owned(),
                namespace: "conversation:test".to_owned(),
                content_chars: 100,
                expected_summary_revision_id: None,
                content: "A bounded routing summary.".to_owned(),
            }),
            MaintenanceReviewOrigin::Manual,
            "review".to_owned(),
            1,
            false,
        );

        ledger
            .snooze_review("msu_snooze", 60)
            .expect("snooze review");

        assert!(ledger.review_items().is_empty());
        assert_eq!(
            ledger
                .review_item("msu_snooze")
                .expect("unresolved snoozed review")
                .status,
            MaintenanceReviewStatus::Pending
        );
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

    #[test]
    fn idle_cycles_back_off_to_the_configured_safety_poll_ceiling() {
        let config = scheduler_config();
        let mut ledger = MaintenanceLedger::default();
        let report = MaintenanceCycleReport::default();

        assert_eq!(ledger.schedule_after_success(&config, &report), 10);
        assert_eq!(ledger.schedule_after_success(&config, &report), 20);
        assert_eq!(ledger.schedule_after_success(&config, &report), 40);
        assert_eq!(ledger.schedule_after_success(&config, &report), 80);
        assert_eq!(ledger.schedule_after_success(&config, &report), 80);
        assert_eq!(ledger.scheduler.idle_cycles, 5);
        assert!(ledger.scheduler.next_wake_at_unix_ms.is_some());
    }

    #[test]
    fn an_external_write_breaks_idle_backoff() {
        let config = scheduler_config();
        let mut ledger = MaintenanceLedger::default();
        let report = MaintenanceCycleReport::default();
        let _ = ledger.schedule_after_success(&config, &report);
        let _ = ledger.schedule_after_success(&config, &report);
        assert_eq!(ledger.scheduler.idle_cycles, 2);

        ledger.start_scheduled_cycle(MaintenanceWakeReason::ExternalWrite);
        assert_eq!(ledger.schedule_after_success(&config, &report), 10);
        assert_eq!(ledger.scheduler.idle_cycles, 1);
        assert_eq!(
            ledger.scheduler.last_wake_reason,
            Some(MaintenanceWakeReason::ExternalWrite)
        );
    }

    #[test]
    fn write_pressure_uses_quiet_or_absolute_deadlines_instead_of_idle_backoff() {
        let config = scheduler_config();
        let mut ledger = MaintenanceLedger::default();
        let report = MaintenanceCycleReport::default();
        let mut inventory = vec![page("1", "rev_1")];
        let _ = ledger.observe_writes(&inventory, &config.write_trigger);

        inventory.push(page("2", "rev_2"));
        let _ = ledger.observe_writes(&inventory, &config.write_trigger);
        let sparse_delay = ledger.schedule_after_success(&config, &report);
        assert!((59..=60).contains(&sparse_delay));

        for id in 3..=9 {
            inventory.push(page(&id.to_string(), &format!("rev_{id}")));
        }
        let _ = ledger.observe_writes(&inventory, &config.write_trigger);
        let pressure_delay = ledger.schedule_after_success(&config, &report);
        assert!((9..=10).contains(&pressure_delay));
        assert_eq!(ledger.scheduler.idle_cycles, 0);
    }

    #[test]
    fn failures_use_a_bounded_independent_retry_backoff() {
        let mut config = scheduler_config();
        config.interval_seconds = 600;
        config.max_interval_seconds = 3_600;
        let mut ledger = MaintenanceLedger::default();

        assert_eq!(ledger.schedule_after_failure(&config), 30);
        assert_eq!(ledger.schedule_after_failure(&config), 60);
        assert_eq!(ledger.schedule_after_failure(&config), 120);
        assert_eq!(ledger.scheduler.consecutive_failures, 3);
        assert_eq!(ledger.scheduler.idle_cycles, 0);
    }

    #[test]
    fn older_scheduler_state_loads_with_adaptive_fields_at_safe_defaults() {
        let ledger: MaintenanceLedger = serde_json::from_str(
            r#"{
                "entries": {},
                "writeTrigger": {"observedRevisions": {}, "dirtyRegions": {}},
                "relationReviews": {},
                "reviewItems": {},
                "scheduler": {
                    "lastStartedAtUnixMs": 10,
                    "lastCompletedAtUnixMs": 11,
                    "lastError": null,
                    "lastReport": null,
                    "currentReport": null
                }
            }"#,
        )
        .expect("load pre-adaptive maintenance ledger");

        assert_eq!(ledger.scheduler.idle_cycles, 0);
        assert_eq!(ledger.scheduler.consecutive_failures, 0);
        assert!(ledger.scheduler.next_wake_at_unix_ms.is_none());
        assert!(ledger.scheduler.last_wake_reason.is_none());
    }
}
