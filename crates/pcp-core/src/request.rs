use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Actor, LifecycleStatus, PagePayload, Projection, ProvenanceEvent, SearchMode, SearchTermMatch,
    SourceRef, ValidityStanding,
};

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateScopeRequest {
    pub owner_id: String,
    pub namespace: String,
    pub scope_type: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_namespace: Option<String>,
    pub visibility: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitialRelation {
    #[serde(alias = "relation_type")]
    pub relation_type: String,
    #[serde(alias = "to_revision_id")]
    pub to_revision_id: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WritePageRequest {
    pub owner_id: String,
    pub namespace: String,
    pub visibility: String,
    pub lifecycle_status: LifecycleStatus,
    pub created_by: Actor,
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
    pub revision_ids: Vec<String>,
    pub projections: Vec<Projection>,
    pub max_chars: u32,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteSummaryRequest {
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

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssessPageValidityRequest {
    pub target_revision_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_assessment_id: Option<String>,
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
    pub from_revision_id: String,
    pub relation_type: String,
    pub to_revision_id: String,
    pub created_by: Actor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}
