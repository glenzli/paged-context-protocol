use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorType {
    User,
    Model,
    Tool,
    System,
}

impl ActorType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Model => "model",
            Self::Tool => "tool",
            Self::System => "system",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Self::User),
            "model" => Some(Self::Model),
            "tool" => Some(Self::Tool),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Actor {
    pub actor_type: ActorType,
    pub actor_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleStatus {
    Active,
    Superseded,
    Archived,
    Tombstoned,
}

impl LifecycleStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Superseded => "superseded",
            Self::Archived => "archived",
            Self::Tombstoned => "tombstoned",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "superseded" => Some(Self::Superseded),
            "archived" => Some(Self::Archived),
            "tombstoned" => Some(Self::Tombstoned),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    Auto,
    Exact,
    Text,
    Graph,
    Temporal,
}

impl SearchMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Exact => "exact",
            Self::Text => "text",
            Self::Graph => "graph",
            Self::Temporal => "temporal",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchTermMatch {
    #[default]
    All,
    Any,
}

impl SearchTermMatch {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Any => "any",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Projection {
    Manifest,
    Summary,
    Validity,
    Payload,
    Sources,
    Provenance,
    Relations,
    Facets,
    History,
}

impl Projection {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Manifest => "manifest",
            Self::Summary => "summary",
            Self::Validity => "validity",
            Self::Payload => "payload",
            Self::Sources => "sources",
            Self::Provenance => "provenance",
            Self::Relations => "relations",
            Self::Facets => "facets",
            Self::History => "history",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidityStanding {
    Live,
    Qualified,
    Disputed,
    Superseded,
    Retracted,
    Unknown,
}

impl ValidityStanding {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Qualified => "qualified",
            Self::Disputed => "disputed",
            Self::Superseded => "superseded",
            Self::Retracted => "retracted",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "live" => Some(Self::Live),
            "qualified" => Some(Self::Qualified),
            "disputed" => Some(Self::Disputed),
            "superseded" => Some(Self::Superseded),
            "retracted" => Some(Self::Retracted),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PagePayload {
    pub media_type: String,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    #[serde(alias = "source_type")]
    pub source_type: String,
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvenanceEvent {
    pub operation: String,
    pub actor: Actor,
    pub timestamp: String,
    #[serde(default, rename = "inputPageIds", alias = "inputRevisionIds")]
    pub input_revision_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_or_model: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageRevision {
    #[serde(rename = "refId")]
    pub page_id: String,
    #[serde(rename = "pageId", alias = "revisionId")]
    pub revision_id: String,
    pub owner_id: String,
    pub namespace: String,
    pub visibility: String,
    pub lifecycle_status: LifecycleStatus,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<String>,
    pub created_by: Actor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<PagePayload>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<SourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facets: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provenance: Vec<ProvenanceEvent>,
}

/// Canonical public name for an immutable PCP content node.
///
/// `PageRevision` remains as an internal compatibility name while hosts migrate
/// from the pre-0.6 mutable Page/Revision model.
pub type Page = PageRevision;

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scope {
    pub owner_id: String,
    pub namespace: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_namespace: Option<String>,
    pub visibility: String,
    pub created_at: String,
    pub updated_at: String,
    pub page_count: u64,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Relation {
    pub relation_id: String,
    #[serde(rename = "fromPageId", alias = "fromRevisionId")]
    pub from_revision_id: String,
    pub relation_type: String,
    #[serde(rename = "toPageId", alias = "toRevisionId")]
    pub to_revision_id: String,
    pub created_by: Actor,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageSummary {
    #[serde(rename = "summaryRefId")]
    pub summary_page_id: String,
    #[serde(rename = "summaryPageId", alias = "summaryRevisionId")]
    pub summary_revision_id: String,
    #[serde(rename = "targetPageId", alias = "targetRevisionId")]
    pub target_revision_id: String,
    pub content: String,
    pub created_by: Actor,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_or_model: Option<String>,
    #[serde(default)]
    pub provenance: Vec<ProvenanceEvent>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageValidity {
    #[serde(rename = "assessmentPageId", alias = "assessmentId")]
    pub assessment_id: String,
    #[serde(
        default,
        rename = "previousAssessmentPageId",
        alias = "previousAssessmentId",
        skip_serializing_if = "Option::is_none"
    )]
    pub previous_assessment_id: Option<String>,
    #[serde(rename = "targetPageId", alias = "targetRevisionId")]
    pub target_revision_id: String,
    pub standing: ValidityStanding,
    pub rationale: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub assessed_at: String,
    pub created_by: Actor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_or_model: Option<String>,
    #[serde(default, rename = "basisPageIds", alias = "basisRevisionIds")]
    pub basis_revision_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageValidityHint {
    #[serde(rename = "assessmentPageId", alias = "assessmentId")]
    pub assessment_id: String,
    pub standing: ValidityStanding,
    pub rationale: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub assessed_at: String,
    pub basis_revision_count: u32,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadPage {
    #[serde(rename = "page", alias = "revision")]
    pub revision: PageRevision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<PageSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validity: Option<PageValidity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validity_history: Vec<PageValidity>,
    #[serde(default)]
    pub relations: Vec<Relation>,
    #[serde(default)]
    #[serde(rename = "lineage", alias = "history")]
    pub history: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub protocol_version: String,
    pub search_modes: Vec<SearchMode>,
    pub projections: Vec<Projection>,
    pub max_search_results: u32,
    pub max_read_pages: u32,
    pub max_read_chars: u32,
    pub supports_event_ingest: bool,
    pub supports_immutable_pages: bool,
    pub supports_refs: bool,
    pub supports_revision_conflicts: bool,
    #[serde(default)]
    pub supports_consolidation: bool,
    pub supports_durable_deletion: bool,
    pub supports_provenance_graph: bool,
    pub supports_access_sessions: bool,
    pub supports_access_audit: bool,
    pub relation_types: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    #[serde(rename = "refId")]
    pub page_id: String,
    #[serde(rename = "pageId", alias = "revisionId")]
    pub revision_id: String,
    pub namespace: String,
    pub lifecycle_status: LifecycleStatus,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
    pub snippet: String,
    pub matched_by: String,
    pub matched_projection: String,
    #[serde(
        default,
        rename = "summaryPageId",
        alias = "summaryRevisionId",
        skip_serializing_if = "Option::is_none"
    )]
    pub summary_revision_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facets: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validity: Option<PageValidityHint>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteResult {
    #[serde(rename = "refId")]
    pub page_id: String,
    #[serde(rename = "pageId", alias = "revisionId")]
    pub revision_id: String,
    pub created: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteSummaryResult {
    #[serde(rename = "targetPageId", alias = "targetRevisionId")]
    pub target_revision_id: String,
    #[serde(rename = "summaryRefId")]
    pub summary_page_id: String,
    #[serde(rename = "summaryPageId", alias = "summaryRevisionId")]
    pub summary_revision_id: String,
    pub created: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteValidityResult {
    #[serde(rename = "targetPageId", alias = "targetRevisionId")]
    pub target_revision_id: String,
    #[serde(rename = "assessmentPageId", alias = "assessmentId")]
    pub assessment_id: String,
    pub created: bool,
}
