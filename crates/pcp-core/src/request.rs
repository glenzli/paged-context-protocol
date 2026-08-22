use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Actor, LifecycleStatus, PageMutability, PagePayload, Projection, ProvenanceEvent, SearchMode,
    SearchTermMatch, SourceRef, SourceSpan, ValidityStanding,
};

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateScopeRequest {
    pub namespace: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_namespace: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitialRelation {
    pub relation_type: String,
    pub to_page_id: String,
    #[serde(default)]
    pub basis_revision_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WritePageRequest {
    pub namespace: String,
    pub lifecycle_status: LifecycleStatus,
    pub kind: String,
    #[serde(default)]
    pub mutability: PageMutability,
    pub created_by: Actor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<PagePayload>,
    #[serde(default)]
    pub source_refs: Vec<SourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facets: Option<Value>,
    #[serde(default)]
    pub provenance: Vec<ProvenanceEvent>,
    #[serde(default)]
    pub initial_relations: Vec<InitialRelation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

/// Minimal producer-facing write. Runtime supplies identity, actor, lifecycle,
/// and sealed mutability from the authenticated session.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestPageRequest {
    pub namespace: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<PagePayload>,
    #[serde(default)]
    pub source_refs: Vec<SourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facets: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_event_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisePageRequest {
    pub page_id: String,
    pub expected_revision_id: String,
    pub created_by: Actor,
    pub lifecycle_status: LifecycleStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<PagePayload>,
    #[serde(default)]
    pub source_refs: Vec<SourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facets: Option<Value>,
    #[serde(default)]
    pub provenance: Vec<ProvenanceEvent>,
    #[serde(default)]
    pub initial_relations: Vec<InitialRelation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

/// Reversibly remove a Page from the default retrieval and graph surface.
///
/// Archival is content governance rather than a content revision: the current
/// revision and its asserted relations remain addressable for audit and can be
/// restored without reconstructing historical content.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivePageRequest {
    pub page_id: String,
    pub expected_revision_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Restore an archived Page to the default retrieval and graph surface.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreArchivedPageRequest {
    pub page_id: String,
    pub expected_revision_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageRevisionRef {
    pub page_id: String,
    pub revision_id: String,
}

/// Ordered exact inputs for one lossless packed Page.
///
/// Inputs are sealed leaves, with at most one current packed Page acting as a
/// stable anchor whose flat payload will be extended.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackPagesRequest {
    pub pages: Vec<PageRevisionRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

/// Restore the sealed source Pages held losslessly by one current packed Page.
///
/// This operator-only repair primitive never guesses how assertions about the
/// combined episode should be redistributed.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnpackPageRequest {
    pub page_id: String,
    pub expected_revision_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchFilters {
    #[serde(default)]
    pub relation_types: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_before: Option<String>,
    #[serde(default)]
    pub lifecycle_status: Vec<LifecycleStatus>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPagesRequest {
    pub query: String,
    pub scopes: Vec<String>,
    pub mode: SearchMode,
    #[serde(default)]
    pub term_match: SearchTermMatch,
    #[serde(default = "default_search_projections")]
    pub projections: Vec<Projection>,
    #[serde(default)]
    pub filters: SearchFilters,
    pub limit: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

pub fn default_search_projections() -> Vec<Projection> {
    vec![Projection::Summary, Projection::Payload, Projection::Facets]
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadPagesRequest {
    #[serde(default)]
    pub page_ids: Vec<String>,
    #[serde(default)]
    pub revision_ids: Vec<String>,
    pub projections: Vec<Projection>,
    pub max_chars: u32,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteSummaryRequest {
    pub target_page_id: String,
    pub target_revision_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_summary_revision_id: Option<String>,
    pub content: String,
    pub created_by: Actor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_or_model: Option<String>,
    #[serde(default)]
    pub provenance: Vec<ProvenanceEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

/// Create one revisioned front-door Page for a bounded, explicitly selected
/// topic. Source Pages remain addressable evidence; retrieval merely prefers
/// this Page while every source Revision stays current.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractTopicRequest {
    /// Ordered exact source Revisions. They must be current, active Pages in
    /// one Scope at the time the extraction is committed.
    pub source_pages: Vec<PageRevisionRef>,
    pub title: String,
    pub content: String,
    pub created_by: Actor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_or_model: Option<String>,
    #[serde(default)]
    pub provenance: Vec<ProvenanceEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssessPageValidityRequest {
    pub target_page_id: String,
    pub target_revision_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_assessment_revision_id: Option<String>,
    pub standing: ValidityStanding,
    pub rationale: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
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
pub struct LinkPagesRequest {
    pub from_page_id: String,
    pub relation_type: String,
    pub to_page_id: String,
    #[serde(default)]
    pub basis_revision_ids: Vec<String>,
    pub created_by: Actor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}
