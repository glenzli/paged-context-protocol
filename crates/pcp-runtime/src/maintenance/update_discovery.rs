//! Bounded discovery of possible changes between ordinary Pages. The scan is
//! not an assertion: only explicit operator approval changes validity.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use pcp_core::{PageRevisionRef, ReconciliationDisposition};
use pcp_store::DurablePageInventoryItem;

use super::{
    MaintenanceCycleReport, MaintenanceReconciliationCandidate, MaintenanceReviewOrigin,
    MaintenanceReviewPayload, MaintenanceReviewStatus, MaintenanceWorkerRequest,
    MaintenanceWorkerResponse, RuntimeMaintainer,
};

impl RuntimeMaintainer {
    pub(super) async fn run_update_discovery(
        &mut self,
        changed_region: &[DurablePageInventoryItem],
        report: &mut MaintenanceCycleReport,
        origin: MaintenanceReviewOrigin,
    ) -> Result<bool> {
        // This inventory is constrained by the maintainer's real Scope grants.
        // Older evidence may be outside the region that woke the scheduler.
        let inventory = self
            .client
            .durable_page_inventory(self.config.allowed_scopes.clone())
            .await?;
        let eligible_new: BTreeSet<_> = changed_region
            .iter()
            .map(|page| &page.revision_id)
            .collect();
        let pairs = candidate_pairs(&inventory);
        let Some((target, evidence)) = pairs.into_iter().find(|(target, evidence)| {
            let key = pair_key(target, evidence);
            eligible_new.contains(&evidence.revision_id)
                && self.ledger.eligible(&key)
                && self.ledger.review_item(&key).is_none_or(|item| {
                    matches!(
                        item.status,
                        MaintenanceReviewStatus::Deferred | MaintenanceReviewStatus::Stale
                    )
                })
                && !self
                    .ledger
                    .review_items()
                    .iter()
                    .any(|item| match &item.payload {
                        MaintenanceReviewPayload::Reconciliation(candidate) => {
                            candidate.target.revision_id == target.revision_id
                        }
                        _ => false,
                    })
        }) else {
            return Ok(false);
        };
        let key = pair_key(target, evidence);
        let budget = self.config.reconciliation.max_input_chars;
        if target.content_chars.saturating_add(evidence.content_chars) > u64::from(budget / 2) {
            self.ledger.record(
                key,
                "update_evidence_exceeds_budget",
                self.config.reconciliation.retry_after_seconds,
            );
            return Ok(false);
        }
        let pages = self
            .read_detail_pages(
                vec![target.revision_id.clone(), evidence.revision_id.clone()],
                budget,
            )
            .await?;
        let target_page = pages
            .iter()
            .find(|page| page.revision_id == target.revision_id)
            .cloned()
            .context("update target is no longer readable")?;
        let evidence_page = pages
            .iter()
            .find(|page| page.revision_id == evidence.revision_id)
            .cloned()
            .context("update evidence is no longer readable")?;
        if [&target_page, &evidence_page]
            .iter()
            .zip([target, evidence])
            .any(|(page, inventory)| {
                page.content
                    .as_ref()
                    .is_none_or(|text| (text.chars().count() as u64) < inventory.content_chars)
            })
        {
            self.ledger.record(
                key,
                "update_evidence_incomplete",
                self.config.reconciliation.retry_after_seconds,
            );
            return Ok(false);
        }
        // Capture before inference, not after it: another review can finish
        // while the worker is running, and must make this proposal stale.
        let expected_assessment_revision_id = self
            .reconciliation_assessment_head(&target.revision_id)
            .await?;
        let outcome = self
            .evaluate_worker(MaintenanceWorkerRequest::ReviewUpdate {
                target: Box::new(target_page.clone()),
                evidence: Box::new(evidence_page.clone()),
            })
            .await?;
        report.worker_calls += outcome.model_attempts;
        report.escalated_decisions += u32::from(outcome.escalated);
        let (disposition, rationale, scope, replacement_revision_id) = match outcome.response {
            MaintenanceWorkerResponse::NoCandidate | MaintenanceWorkerResponse::Defer => {
                self.ledger.record(
                    key,
                    "no_confirmed_update",
                    self.config.reconciliation.retry_after_seconds,
                );
                report.deferred += 1;
                return Ok(true);
            }
            MaintenanceWorkerResponse::ReconcileFeedback {
                target_revision_id,
                disposition,
                rationale,
                scope,
                replacement_revision_id,
            } => {
                anyhow::ensure!(
                    target_revision_id == target.revision_id,
                    "worker selected an unoffered update target"
                );
                (disposition, rationale, scope, replacement_revision_id)
            }
            _ => anyhow::bail!("worker returned an invalid content update decision"),
        };
        anyhow::ensure!(
            matches!(
                disposition,
                ReconciliationDisposition::Qualified
                    | ReconciliationDisposition::Disputed
                    | ReconciliationDisposition::Superseded
            ),
            "ordinary content discovery cannot retract a Page"
        );
        anyhow::ensure!(
            !rationale.trim().is_empty() && rationale.chars().count() <= 2_000,
            "worker returned an invalid update rationale"
        );
        anyhow::ensure!(
            scope
                .as_ref()
                .is_none_or(|value| value.chars().count() <= 1_000),
            "update scope is too long"
        );
        let superseded = disposition == ReconciliationDisposition::Superseded;
        anyhow::ensure!(
            if superseded {
                replacement_revision_id.as_deref() == Some(evidence.revision_id.as_str())
            } else {
                replacement_revision_id.is_none()
            },
            "worker selected an invalid update replacement"
        );
        let candidate = MaintenanceReconciliationCandidate {
            candidate_id: key.clone(),
            signal: None,
            feedback: None,
            target: target_page,
            evidence: vec![evidence_page],
            expected_assessment_revision_id,
            disposition,
            rationale,
            scope,
            replacement: superseded.then(|| PageRevisionRef {
                page_id: evidence.page_id.clone(),
                revision_id: evidence.revision_id.clone(),
            }),
            basis_revision_ids: vec![target.revision_id.clone(), evidence.revision_id.clone()],
        };
        self.ledger.enqueue_review(MaintenanceReviewPayload::Reconciliation(candidate), origin,
            "New content may change an earlier claim. No validity change occurs until this proposal is approved.".to_owned(),
            outcome.model_attempts, outcome.escalated);
        self.ledger.record(
            key,
            "update_pending_review",
            self.config.reconciliation.retry_after_seconds,
        );
        report.reconciliations_proposed += 1;
        report.review_items_proposed += 1;
        Ok(true)
    }
}

fn pair_key(target: &DurablePageInventoryItem, evidence: &DurablePageInventoryItem) -> String {
    format!("update:{}:{}", target.revision_id, evidence.revision_id)
}

fn candidate_pairs(
    inventory: &[DurablePageInventoryItem],
) -> Vec<(&DurablePageInventoryItem, &DurablePageInventoryItem)> {
    let mut pages: Vec<_> = inventory
        .iter()
        .filter(|page| {
            !page.superseded
                && page.content_chars > 0
                && !matches!(
                    page.kind.as_str(),
                    "feedback_signal" | "validity_assessment" | "topic_summary" | "summary"
                )
        })
        .collect();
    pages.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.page_id.cmp(&b.page_id))
    });
    let by_revision: BTreeMap<_, _> = pages
        .iter()
        .map(|page| (page.revision_id.as_str(), *page))
        .collect();
    let terms: BTreeMap<_, _> = pages
        .iter()
        .map(|page| {
            (
                page.page_id.as_str(),
                subject_terms(page.summary.as_deref().unwrap_or(&page.snippet)),
            )
        })
        .collect();
    let mut pairs = Vec::new();
    let mut seen = BTreeSet::new();
    // At most 48 fresh anchors, each with exact provenance and two best
    // lexical subject candidates. These are only hints for semantic review.
    for new in pages.iter().take(48) {
        for revision in new.provenance_input_revision_ids.iter().take(8) {
            if let Some(old) = by_revision.get(revision.as_str()) {
                if old.page_id != new.page_id && seen.insert(pair_key(old, new)) {
                    pairs.push((*old, *new));
                }
            }
        }
        let new_terms = &terms[new.page_id.as_str()];
        let mut matches: Vec<_> = pages
            .iter()
            .filter(|old| old.page_id != new.page_id && old.created_at < new.created_at)
            .filter_map(|old| {
                let old_terms = &terms[old.page_id.as_str()];
                let shared = new_terms.intersection(old_terms).count();
                let shorter = new_terms.len().min(old_terms.len());
                (shared >= 4 && shared * 3 >= shorter)
                    .then_some((shared * 100 / shorter.max(1), *old))
            })
            .collect();
        matches.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| b.1.created_at.cmp(&a.1.created_at))
        });
        for (_, old) in matches.into_iter().take(2) {
            if seen.insert(pair_key(old, new)) {
                pairs.push((old, *new));
            }
        }
    }
    pairs
}

fn subject_terms(text: &str) -> BTreeSet<String> {
    let text: String = text.chars().take(800).collect::<String>().to_lowercase();
    let mut terms: BTreeSet<_> = text
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|word| {
            word.len() >= 3
                && !matches!(
                    *word,
                    "the"
                        | "and"
                        | "that"
                        | "this"
                        | "for"
                        | "with"
                        | "from"
                        | "was"
                        | "are"
                        | "has"
                        | "have"
                        | "not"
                        | "can"
                )
        })
        .map(str::to_owned)
        .collect();
    let chars: Vec<_> = text.chars().collect();
    for pair in chars.windows(2) {
        if pair.iter().all(|c| matches!(c, '\u{4e00}'..='\u{9fff}')) {
            terms.insert(pair.iter().collect());
        }
    }
    terms
}

#[cfg(test)]
mod tests {
    use super::subject_terms;
    #[test]
    fn subject_hints_cover_chinese_and_identifiers_without_timestamps_as_authority() {
        let terms = subject_terms("PCP 用户偏好使用 Rust");
        assert!(terms.contains("pcp") && terms.contains("rust") && terms.contains("偏好"));
        assert!(!subject_terms("the and this with").contains("the"));
    }
}
