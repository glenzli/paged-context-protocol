use anyhow::Result;
use pcp_core::{
    FeedbackSignal, LifecycleStatus, PageRevisionRef, Projection, ReadPagesRequest,
    ReconciliationDisposition,
};
use serde::{Deserialize, Serialize};

use super::{MaintenanceDetailPage, MaintenanceReviewStatus, RuntimeMaintainer};

/// Human-review view for explicit feedback or a discovered content update.
/// It preserves the exact offered Revisions and never contains a
/// tenant source-provider expansion.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceReconciliationCandidate {
    pub candidate_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<FeedbackSignal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<MaintenanceDetailPage>,
    pub target: MaintenanceDetailPage,
    /// Exact evidence shown alongside the old content, including replacements.
    #[serde(default)]
    pub evidence: Vec<MaintenanceDetailPage>,
    #[serde(default)]
    pub expected_assessment_revision_id: Option<String>,
    pub disposition: ReconciliationDisposition,
    pub rationale: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement: Option<PageRevisionRef>,
    pub basis_revision_ids: Vec<String>,
}

impl MaintenanceReconciliationCandidate {
    pub(crate) fn candidate_id(feedback_revision_id: &str, target_revision_id: &str) -> String {
        format!("reconcile:{feedback_revision_id}:{target_revision_id}")
    }
}

/// Keep proposal explanations in the review ledger. Terminal decisions are
/// already expressed by standing, exact evidence and (for replacement) a
/// supersedes Relation; only qualifications/disputes need explanatory content.
pub(super) fn assessment_rationale(
    disposition: &ReconciliationDisposition,
    review_rationale: &str,
) -> String {
    match disposition {
        ReconciliationDisposition::Qualified | ReconciliationDisposition::Disputed => {
            review_rationale.trim().to_owned()
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_decisions_do_not_copy_review_explanations_into_content() {
        for disposition in [
            ReconciliationDisposition::Superseded,
            ReconciliationDisposition::Retracted,
            ReconciliationDisposition::NoSourceChange,
        ] {
            assert!(assessment_rationale(&disposition, "why the proposal was made").is_empty());
        }
        for disposition in [
            ReconciliationDisposition::Qualified,
            ReconciliationDisposition::Disputed,
        ] {
            assert_eq!(
                assessment_rationale(&disposition, " scope-limited grounds "),
                "scope-limited grounds"
            );
        }
    }
}

impl RuntimeMaintainer {
    pub(super) async fn feedback_is_current(&self, signal: &FeedbackSignal) -> Result<bool> {
        let pages = self
            .client
            .read_pages(ReadPagesRequest {
                page_ids: vec![signal.feedback_page_id.clone()],
                revision_ids: Vec::new(),
                projections: vec![Projection::Manifest],
                max_chars: 4_000,
            })
            .await?;
        Ok(pages.iter().any(|page| {
            page.page.page_id == signal.feedback_page_id
                && page.page.head_revision_id == signal.feedback_revision_id
                && page.page.lifecycle_status == LifecycleStatus::Active
        }))
    }

    /// Refresh the persisted inbox on load and before analysis. Store repeats
    /// the head check atomically on apply, including edits racing with this read.
    pub(super) async fn refresh_feedback_reviews(&mut self) -> Result<()> {
        let bindings = self.ledger.pending_feedback_reviews();
        let mut stale = Vec::new();
        for chunk in bindings.chunks(64) {
            let mut ids = chunk
                .iter()
                .map(|(_, page, _)| page.clone())
                .collect::<Vec<_>>();
            ids.sort();
            ids.dedup();
            let pages = self
                .client
                .read_pages(ReadPagesRequest {
                    page_ids: ids,
                    revision_ids: Vec::new(),
                    projections: vec![Projection::Manifest],
                    max_chars: 4_000,
                })
                .await?;
            for (candidate_id, page_id, revision_id) in chunk {
                let current = pages.iter().any(|page| {
                    &page.page.page_id == page_id
                        && &page.page.head_revision_id == revision_id
                        && page.page.lifecycle_status == LifecycleStatus::Active
                });
                if !current {
                    self.ledger
                        .resolve_review(candidate_id, MaintenanceReviewStatus::Stale)?;
                    stale.push(candidate_id.clone());
                }
            }
        }
        if !stale.is_empty() {
            self.ledger
                .persist_stale_reviews(&self.config.state_path, &stale)
                .await?;
        }
        Ok(())
    }
}
