use serde::{Deserialize, Serialize};

use crate::{
    AccessDecision, AccessPrincipal, GraphEdgeDirection, GraphEdgeKind, PageValidityHint,
    Projection, ReadPage, SourceSpan,
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentEffort {
    Low,
    #[default]
    Medium,
    High,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryContextRequest {
    pub query: String,
    /// Empty means every scope already granted to the connected principal.
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub result_limit: Option<u32>,
    #[serde(default)]
    pub context_budget_chars: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryContextResponse {
    pub scopes: Vec<String>,
    pub visibility: QueryVisibility,
    pub result_limit: u32,
    pub context_budget_chars: u32,
    pub anchor_count: usize,
    pub related_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_indexed_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_embedded_count: Option<usize>,
    /// Number of embedding requests executed while retrieving candidates.
    /// Embedding providers currently do not report token usage, so this is
    /// exposed separately from the reported Router token counters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_model_calls: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent_match: Option<IntentMatchAudit>,
    pub entries: Vec<ContextPackEntry>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryVisibility {
    AllAuthorized,
    Scoped,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextPackEntry {
    pub rank: usize,
    pub anchor_rank: usize,
    pub page_id: String,
    pub revision_id: String,
    pub namespace: String,
    pub kind: String,
    pub matched_by: String,
    pub matched_projection: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_score: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structural_boost: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub structural_relations: Vec<QueryRelation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent_reason: Option<String>,
    pub detail: ContextDetail,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<QueryRelation>,
    pub source_projection_truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provenance_revision_ids: Vec<String>,
    /// Current assessment for this exact Revision. Qualified and disputed
    /// entries remain recallable, but consumers must preserve this caveat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validity: Option<PageValidityHint>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryRelation {
    pub relation_type: String,
    pub direction: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextDetail {
    Payload,
    Excerpt,
    Summary,
    Reference,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntentMatchAudit {
    pub effort: IntentEffort,
    pub router_rounds: usize,
    pub router_usage: RouterTokenUsage,
    pub semantic_probes: Vec<String>,
    pub exact_terms: Vec<String>,
    pub candidate_count: usize,
    pub relation_candidates_considered: usize,
    pub consulted_count: usize,
    pub catalog_pages_considered: usize,
    pub stopped_reason: String,
}

/// Provider-reported token accounting for one or more model responses.
///
/// The same shape is used by retrieval Routers and maintenance workers. It
/// deliberately carries only usage counters, never prompts or completion text.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelTokenUsage {
    pub reported_responses: usize,
    pub unreported_responses: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cached_input_tokens: u64,
    pub reasoning_tokens: u64,
}

/// Backwards-compatible name for the usage reported by intent Router calls.
pub type RouterTokenUsage = ModelTokenUsage;

impl ModelTokenUsage {
    pub fn add_assign(&mut self, other: &Self) {
        self.reported_responses = self
            .reported_responses
            .saturating_add(other.reported_responses);
        self.unreported_responses = self
            .unreported_responses
            .saturating_add(other.unreported_responses);
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.total_tokens = self.total_tokens.saturating_add(other.total_tokens);
        self.cached_input_tokens = self
            .cached_input_tokens
            .saturating_add(other.cached_input_tokens);
        self.reasoning_tokens = self.reasoning_tokens.saturating_add(other.reasoning_tokens);
    }

    pub fn response_count(&self) -> u64 {
        (self.reported_responses + self.unreported_responses)
            .try_into()
            .unwrap_or(u64::MAX)
    }
}

/// Content-free model-usage record owned by the PCP Runtime. The operation
/// identifies the PCP workflow only; prompts, Page text, and provider output
/// are intentionally excluded.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeUsageEvent {
    pub event_id: String,
    pub occurred_at: String,
    pub principal: AccessPrincipal,
    pub session_id: String,
    pub source: String,
    pub operation: String,
    pub scopes: Vec<String>,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ModelTokenUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<String>,
}

/// A privacy-preserving Runtime record for a completed retrieval request.
///
/// It deliberately excludes the query text, Router prompts, candidate reasons,
/// and Page content. The Console can therefore account for retrieval behavior
/// and Router token usage without turning the audit log into a second memory
/// store.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryAuditEvent {
    pub event_id: String,
    pub occurred_at: String,
    pub principal: AccessPrincipal,
    pub session_id: String,
    pub method: QueryAuditMethod,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<IntentEffort>,
    pub scopes: Vec<String>,
    pub decision: AccessDecision,
    pub duration_ms: u64,
    pub anchor_count: u64,
    pub related_count: u64,
    pub context_chars: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_indexed_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_embedded_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub router_rounds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub router_usage: Option<RouterTokenUsage>,
    /// A small stable category such as `provider_unavailable` or `failed`.
    /// The original provider error is intentionally not persisted here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryAuditMethod {
    SemanticSearch,
    MatchIntent,
}

/// A bounded, ACL-filtered graph neighborhood rooted at explicit Page IDs.
/// PCP intentionally does not expose an unanchored whole-graph export.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpandGraphRequest {
    pub anchor_page_ids: Vec<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub max_depth: Option<u8>,
    #[serde(default)]
    pub max_nodes: Option<u32>,
    #[serde(default)]
    pub max_edges: Option<u32>,
    #[serde(default)]
    pub projections: Vec<Projection>,
    #[serde(default)]
    pub max_chars: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphSliceResponse {
    pub nodes: Vec<ReadPage>,
    pub edges: Vec<GraphSliceEdge>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphSliceEdge {
    pub from_page_id: String,
    pub to_page_id: String,
    pub relation_type: String,
    pub edge_kind: GraphEdgeKind,
    pub direction_from_origin: GraphEdgeDirection,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub basis_revision_ids: Vec<String>,
}
