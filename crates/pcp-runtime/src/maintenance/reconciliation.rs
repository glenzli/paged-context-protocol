use pcp_core::{FeedbackSignal, PageRevisionRef, ReconciliationDisposition};
use serde::{Deserialize, Serialize};

use super::MaintenanceDetailPage;

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
