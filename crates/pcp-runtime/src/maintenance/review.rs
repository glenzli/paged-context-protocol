use serde::{Deserialize, Serialize};

use super::{
    MaintenanceArchiveCandidate, MaintenancePackCandidate, MaintenanceReconciliationCandidate,
    MaintenanceRelationCandidate, MaintenanceRelationReviewProposal, MaintenanceSummaryCandidate,
    MaintenanceTopicCandidate,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceReviewStatus {
    Pending,
    Accepted,
    Rejected,
    Deferred,
    Suppressed,
    Stale,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceReviewOrigin {
    Automatic,
    Manual,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", content = "candidate", rename_all = "snake_case")]
pub enum MaintenanceReviewPayload {
    Pack(MaintenancePackCandidate),
    Summary(MaintenanceSummaryCandidate),
    Relation(MaintenanceRelationCandidate),
    Topic(MaintenanceTopicCandidate),
    Archive(MaintenanceArchiveCandidate),
    Reconciliation(MaintenanceReconciliationCandidate),
}

impl MaintenanceReviewPayload {
    pub fn candidate_id(&self) -> &str {
        match self {
            Self::Pack(candidate) => &candidate.candidate_id,
            Self::Summary(candidate) => &candidate.candidate_id,
            Self::Relation(candidate) => &candidate.candidate_id,
            Self::Topic(candidate) => &candidate.candidate_id,
            Self::Archive(candidate) => &candidate.candidate_id,
            Self::Reconciliation(candidate) => &candidate.candidate_id,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceReviewItem {
    pub candidate_id: String,
    pub proposed_at: String,
    pub updated_at: String,
    pub origin: MaintenanceReviewOrigin,
    pub status: MaintenanceReviewStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snoozed_until: Option<String>,
    pub reason: String,
    #[serde(default = "default_model_attempts")]
    pub model_attempts: u32,
    #[serde(default)]
    pub escalated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_reason: Option<String>,
    pub payload: MaintenanceReviewPayload,
    /// Live routing projection, not a persisted approval or model verdict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<MaintenanceReviewQueue>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceReviewQueue {
    pub state: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accumulation: Option<ReviewAccumulation>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewAccumulation {
    pub source_pages: usize,
    pub source_chars: u64,
    pub minimum_pages: usize,
    pub minimum_chars: u64,
}

pub(super) fn upgrade_enabled(config: &super::MaintenanceConfig) -> bool {
    matches!(&config.worker, super::MaintenanceWorkerConfig::InferRuntime { review_budget, .. } if review_budget.enabled)
}

pub(super) fn needs_automatic_review(v: &super::MaintenanceVerification) -> bool {
    v.review_state.as_deref() == Some("waiting_budget")
        || (matches!(v.verdict, super::VerificationVerdict::NeedsReview)
            && v.review_steps.is_empty()
            && !matches!(
                v.review_state.as_deref(),
                Some("missing_evidence" | "needs_review")
            ))
}

pub(super) fn review_queue(
    item: &MaintenanceReviewItem,
    config: &super::MaintenanceConfig,
    inventory: &[pcp_store::DurablePageInventoryItem],
) -> MaintenanceReviewQueue {
    let route = |state: &str, reason: String| MaintenanceReviewQueue {
        state: state.into(),
        reason,
        accumulation: None,
    };
    let (verification, automatic) = match &item.payload {
        MaintenanceReviewPayload::Topic(candidate) => {
            let selected = candidate
                .pages
                .iter()
                .filter_map(|source| {
                    inventory.iter().find(|p| {
                        p.page_id == source.page_id
                            && p.revision_id == source.revision_id
                            && !p.superseded
                    })
                })
                .collect::<Vec<_>>();
            if selected.len() != candidate.pages.len()
                || candidate.refresh_target.as_ref().is_some_and(|target| {
                    !inventory.iter().any(|p| {
                        p.page_id == target.page_id
                            && p.revision_id == target.revision_id
                            && !p.superseded
                    })
                })
            {
                return route(
                    "stale",
                    "Source or target revisions changed; the old proposal will be retired.".into(),
                );
            }
            if candidate
                .verification
                .as_ref()
                .is_some_and(|v| matches!(v.verdict, super::VerificationVerdict::Approve))
                && !super::discovery::accumulated(&selected, &config.topic)
            {
                let mut queue = route("waiting_accumulation", "The model approved this proposal; more source material is required for automatic synthesis.".into());
                queue.accumulation = Some(ReviewAccumulation {
                    source_pages: selected.len(),
                    source_chars: selected.iter().map(|p| p.content_chars).sum(),
                    minimum_pages: config.topic.minimum_pages,
                    minimum_chars: config.topic.minimum_total_chars,
                });
                return queue;
            }
            (candidate.verification.as_ref(), config.topic.auto_apply)
        }
        MaintenanceReviewPayload::Relation(candidate) => (
            candidate.verification.as_ref(),
            config.relation.auto_apply_verified,
        ),
        _ => {
            return route(
                "human",
                "This operation requires an explicit review decision.".into(),
            );
        }
    };
    if !config.enabled || !config.applies_changes() || !automatic {
        return route(
            "paused",
            "Automatic application is disabled; the proposal remains available for manual review."
                .into(),
        );
    }
    match verification {
        Some(v) if v.review_state.as_deref() == Some("missing_evidence") => {
            route("human", v.reason.clone())
        }
        Some(v) if matches!(v.verdict, super::VerificationVerdict::Approve) => route(
            "automatic",
            "Model review passed; awaiting current-evidence checks and application.".into(),
        ),
        Some(v) if needs_automatic_review(v) && upgrade_enabled(config) => {
            if v.review_state.as_deref() == Some("waiting_budget") {
                route("waiting_budget", v.reason.clone())
            } else {
                route("automatic", "Awaiting bounded upgraded review; the baseline has not established a need for human input.".into())
            }
        }
        Some(v) if needs_automatic_review(v) => route(
            "paused",
            "Upgraded review is disabled; baseline uncertainty has not been resolved.".into(),
        ),
        Some(v) => route("human", v.reason.clone()),
        None => route(
            "human",
            "No automatic verification is recorded for this proposal.".into(),
        ),
    }
}

impl MaintenanceReviewItem {
    pub(crate) fn pending(
        payload: MaintenanceReviewPayload,
        origin: MaintenanceReviewOrigin,
        reason: String,
        model_attempts: u32,
        escalated: bool,
    ) -> Self {
        let timestamp = chrono::Utc::now().to_rfc3339();
        let escalated = escalated
            || match &payload {
                MaintenanceReviewPayload::Topic(c) => c
                    .verification
                    .as_ref()
                    .is_some_and(|v| !v.review_steps.is_empty()),
                MaintenanceReviewPayload::Relation(c) => c
                    .verification
                    .as_ref()
                    .is_some_and(|v| !v.review_steps.is_empty()),
                _ => false,
            };
        Self {
            candidate_id: payload.candidate_id().to_owned(),
            proposed_at: timestamp.clone(),
            updated_at: timestamp,
            origin,
            status: MaintenanceReviewStatus::Pending,
            snoozed_until: None,
            reason,
            model_attempts: model_attempts.max(1),
            escalated,
            decision_reason: None,
            payload,
            queue: None,
        }
    }

    pub(crate) fn relation(proposal: MaintenanceRelationReviewProposal) -> Self {
        let payload = MaintenanceReviewPayload::Relation(MaintenanceRelationCandidate {
            candidate_id: proposal.candidate_id.clone(),
            namespace: proposal.namespace,
            pages: proposal.pages.map(|page| super::MaintenanceRelationInput {
                page_id: page.page_id,
                revision_id: page.revision_id,
                preview: page.preview,
            }),
            relation_reason: proposal.relation_reason,
            verification: proposal.verification,
        });
        Self {
            candidate_id: proposal.candidate_id,
            proposed_at: proposal.proposed_at.clone(),
            updated_at: proposal.proposed_at,
            origin: MaintenanceReviewOrigin::Automatic,
            status: MaintenanceReviewStatus::Pending,
            snoozed_until: proposal.snoozed_until,
            reason: proposal.review_reason,
            model_attempts: proposal.model_attempts,
            escalated: proposal.escalated,
            decision_reason: None,
            payload,
            queue: None,
        }
    }
}

fn default_model_attempts() -> u32 {
    1
}
