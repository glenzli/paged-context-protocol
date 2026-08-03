use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetentionPolicy {
    #[serde(default = "default_minimum_age_days")]
    pub minimum_age_days: u32,
    #[serde(default = "default_keep_recent_revisions")]
    pub keep_recent_revisions_per_page: u32,
    #[serde(default = "default_sample_limit")]
    pub sample_limit: u32,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            minimum_age_days: default_minimum_age_days(),
            keep_recent_revisions_per_page: default_keep_recent_revisions(),
            sample_limit: default_sample_limit(),
        }
    }
}

fn default_minimum_age_days() -> u32 {
    30
}

fn default_keep_recent_revisions() -> u32 {
    2
}

fn default_sample_limit() -> u32 {
    100
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRevisionRetentionRequest {
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub policy: RetentionPolicy,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PutRevisionRetentionLeaseRequest {
    pub namespace: String,
    pub revision_id: String,
    pub reason: String,
    pub expires_at: String,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectRevisionRetentionRequest {
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub policy: RetentionPolicy,
    pub revision_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionRetentionLease {
    pub lease_id: String,
    pub page_id: String,
    pub revision_id: String,
    pub namespace: String,
    pub holder_principal_id: String,
    pub reason: String,
    pub created_at: String,
    pub updated_at: String,
    pub expires_at: String,
}

#[derive(
    Clone, Copy, Debug, Deserialize, Eq, Hash, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum RetentionProtectionReason {
    CurrentHead,
    SealedEvidence,
    RecentRevisionWindow,
    MinimumAgeWindow,
    RelationEndpoint,
    RelationBasis,
    ProjectionHead,
    SummaryRecord,
    ValidityRecord,
    IdempotencyWindow,
    ProvenanceDependency,
    ExplicitLease,
    InvalidTimestamp,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetentionReasonCount {
    pub reason: RetentionProtectionReason,
    pub revisions: u64,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionRetentionCandidate {
    pub page_id: String,
    pub revision_id: String,
    pub namespace: String,
    pub kind: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_revision_id: Option<String>,
    pub estimated_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtectedRevisionSample {
    pub page_id: String,
    pub revision_id: String,
    pub namespace: String,
    pub kind: String,
    pub created_at: String,
    pub estimated_bytes: u64,
    pub reasons: Vec<RetentionProtectionReason>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionRetentionPlan {
    pub generated_at: String,
    pub cutoff_at: String,
    pub scopes: Vec<String>,
    pub policy: RetentionPolicy,
    pub scanned_pages: u64,
    pub scanned_revisions: u64,
    pub protected_revisions: u64,
    pub candidate_revisions: u64,
    pub candidate_pages: u64,
    pub candidate_estimated_bytes: u64,
    pub past_window_idempotency_records: u64,
    pub active_retention_leases: u64,
    pub protection_reasons: Vec<RetentionReasonCount>,
    pub candidates: Vec<RevisionRetentionCandidate>,
    pub protected_samples: Vec<ProtectedRevisionSample>,
    pub candidates_truncated: bool,
    pub protected_samples_truncated: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionCollectionResult {
    pub collected_at: String,
    pub collected_revisions: u64,
    pub collected_pages: u64,
    pub reclaimed_estimated_bytes: u64,
    pub past_window_idempotency_records_removed: u64,
    pub expired_retention_leases_removed: u64,
    pub revision_ids: Vec<String>,
}
