use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Actor, PagePayload, PageRevisionRef, Relation, SourceRef, WriteValidityResult};

/// Tenant-declared reason for asking PCP to revisit recalled context.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackKind {
    Challenge,
    Correction,
    PreferenceChange,
    ScopeException,
}

impl FeedbackKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Challenge => "challenge",
            Self::Correction => "correction",
            Self::PreferenceChange => "preference_change",
            Self::ScopeException => "scope_exception",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "challenge" => Some(Self::Challenge),
            "correction" => Some(Self::Correction),
            "preference_change" => Some(Self::PreferenceChange),
            "scope_exception" => Some(Self::ScopeException),
            _ => None,
        }
    }
}

/// Authority claimed by the tenant for one feedback event.
///
/// PCP records this claim but does not inspect or authenticate tenant-owned
/// source material. Maintenance may use it as review context, never as proof.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackAuthority {
    SubjectOwner,
    TenantAssertion,
    ExternalClaim,
    Unknown,
}

impl FeedbackAuthority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SubjectOwner => "subject_owner",
            Self::TenantAssertion => "tenant_assertion",
            Self::ExternalClaim => "external_claim",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "subject_owner" => Some(Self::SubjectOwner),
            "tenant_assertion" => Some(Self::TenantAssertion),
            "external_claim" => Some(Self::ExternalClaim),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitFeedbackRequest {
    pub namespace: String,
    pub kind: FeedbackKind,
    pub authority: FeedbackAuthority,
    pub payload: PagePayload,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
    #[serde(default)]
    pub source_refs: Vec<SourceRef>,
    /// Exact Revisions the user or tenant explicitly challenged.
    /// Challenged Revisions that have not yet received a reconciliation
    /// decision. A multi-target feedback signal remains pending until this is
    /// empty.
    pub challenged_revision_ids: Vec<String>,
    /// Exact Revisions actually used to produce the challenged response.
    /// This may be a superset of `challengedRevisionIds`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub used_revision_ids: Vec<String>,
    /// Additional correction evidence, including content written after the
    /// challenged response. This is not evidence that the old response used.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_revision_ids: Vec<String>,
    /// Opaque tenant-owned locator for the response or interaction. PCP does
    /// not dereference or render this value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_event_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackSubmission {
    pub feedback_page_id: String,
    pub feedback_revision_id: String,
    pub created: bool,
    pub challenged_revision_ids: Vec<String>,
    pub used_revision_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_revision_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackStatus {
    Pending,
    Applied,
    Dismissed,
}

impl FeedbackStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Applied => "applied",
            Self::Dismissed => "dismissed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "applied" => Some(Self::Applied),
            "dismissed" => Some(Self::Dismissed),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackSignal {
    pub feedback_page_id: String,
    pub feedback_revision_id: String,
    pub namespace: String,
    pub kind: FeedbackKind,
    pub authority: FeedbackAuthority,
    pub status: FeedbackStatus,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_ref: Option<String>,
    pub challenged_revision_ids: Vec<String>,
    pub used_revision_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_revision_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationDisposition {
    NoSourceChange,
    Qualified,
    Disputed,
    Superseded,
    Retracted,
}

impl ReconciliationDisposition {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NoSourceChange => "no_source_change",
            Self::Qualified => "qualified",
            Self::Disputed => "disputed",
            Self::Superseded => "superseded",
            Self::Retracted => "retracted",
        }
    }
}

/// One reviewed, atomic reconciliation of explicit tenant feedback.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyReconciliationRequest {
    /// Absent for a Runtime-discovered update reviewed by the operator. Such
    /// proposals live only in the maintenance ledger until approved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback_revision_id: Option<String>,
    pub target: PageRevisionRef,
    /// Exact validity head displayed at review time; None means no assessment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_assessment_revision_id: Option<String>,
    pub disposition: ReconciliationDisposition,
    /// Optional explanation, not a new knowledge assertion.
    #[serde(default)]
    pub rationale: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Required for `superseded`; forbidden for other dispositions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement: Option<PageRevisionRef>,
    #[serde(default)]
    pub basis_revision_ids: Vec<String>,
    pub created_by: Actor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_or_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback_revision_id: Option<String>,
    pub target: PageRevisionRef,
    pub disposition: ReconciliationDisposition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validity: Option<WriteValidityResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes_relation: Option<Relation>,
    /// Current derived Revisions whose summaries, topics, relations, or other
    /// projections should be revisited after this decision.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affected_revision_ids: Vec<String>,
    pub created: bool,
}
