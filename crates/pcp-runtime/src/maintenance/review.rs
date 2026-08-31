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
    pub payload: MaintenanceReviewPayload,
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
            payload,
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
            payload,
        }
    }
}

fn default_model_attempts() -> u32 {
    1
}
