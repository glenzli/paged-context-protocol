use std::sync::Arc;

use chrono::{SecondsFormat, Utc};
use pcp_client::PcpApi;
use pcp_core::{
    AccessAuditEvent, AccessPermission, AccessSession, Actor, ActorType, AssessPageValidityRequest,
    BrowseIndexOrder, Capabilities, CreateScopeRequest, ExpandGraphRequest, ExtractTopicRequest,
    FeedbackAuthority, FeedbackKind, IngestPageRequest, IntentEffort, LifecycleStatus,
    LinkPagesRequest, PageMutability, PagePayload, PageRevisionRef, Projection, ProvenanceEvent,
    QueryContextRequest, ReadPage, ReadPagesRequest, Relation, RevisePageRequest, Scope,
    SearchFilters, SearchMode, SearchPagesRequest, SearchResult, SearchTermMatch, SourceRef,
    SourceSpan, SubmitFeedbackRequest, ValidityStanding, WritePageRequest, WriteSummaryRequest,
};
use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::wrapper::{Json, Parameters},
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Serialize;

const SERVER_INSTRUCTIONS: &str = "PCP is an identity-scoped durable graph of stable Pages with immutable Revisions. Call pcp_whoami before cross-Scope work. Prefer pcp_semantic_search for conservative semantic retrieval; use pcp_match_intent only when a Router review is justified, and pcp_search_pages only for deterministic inspection. Use pageId for stable identity and Relations; use revisionId for exact evidence and provenance. pcp_expand_graph returns only a bounded, anchored ACL-filtered slice, never the entire graph. Ordinary producers should use pcp_ingest_page. When a user explicitly challenges recalled evidence, call pcp_submit_feedback with exact challenged and actually-used revision IDs; do not silently rewrite or delete the recalled Page. Maintained interpretations use the advanced write and revise tools.";

#[derive(Clone)]
pub struct PcpMcpServer {
    client: Arc<dyn PcpApi>,
}

#[derive(Debug, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeResult {
    identity_id: String,
    integrity: String,
    capabilities: Capabilities,
}

#[derive(Debug, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhoAmIResult {
    access: AccessSession,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListScopesParams {
    #[serde(default)]
    query: Option<String>,
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Debug, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListScopesResult {
    scopes: Vec<Scope>,
    next_cursor: Option<String>,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowseIndexParams {
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    excluded_page_kinds: Vec<String>,
    #[serde(default)]
    order: BrowseIndexOrder,
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default = "default_max_chars")]
    max_chars: u32,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPagesParams {
    query: String,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    strategy: Option<String>,
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadPagesParams {
    #[serde(default)]
    page_ids: Vec<String>,
    #[serde(default)]
    revision_ids: Vec<String>,
    #[serde(default)]
    view: Option<String>,
    #[serde(default = "default_max_chars")]
    max_chars: u32,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryContextParams {
    query: String,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default = "default_limit")]
    result_limit: u32,
    #[serde(default)]
    context_budget_chars: Option<u32>,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntentMatchParams {
    query: String,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default = "default_limit")]
    result_limit: u32,
    #[serde(default)]
    context_budget_chars: Option<u32>,
    #[serde(default)]
    effort: IntentEffortParam,
}

#[derive(Debug, Default, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentEffortParam {
    Low,
    #[default]
    Medium,
    High,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpandGraphParams {
    anchor_page_ids: Vec<String>,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    max_depth: Option<u8>,
    #[serde(default)]
    max_nodes: Option<u32>,
    #[serde(default)]
    max_edges: Option<u32>,
    #[serde(default)]
    view: Option<String>,
    #[serde(default)]
    max_chars: Option<u32>,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WritePageParams {
    #[serde(default)]
    scope: Option<String>,
    #[serde(default = "default_page_kind")]
    kind: String,
    #[serde(default)]
    mutability: PageMutability,
    content: String,
    #[serde(default)]
    based_on_revision_ids: Vec<String>,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestPageParams {
    #[serde(default)]
    scope: Option<String>,
    #[serde(default = "default_page_kind")]
    kind: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    source_refs: Vec<SourceRef>,
    /// Exact PCP Revisions used by the tenant to produce this source Page.
    /// Runtime records trusted provenance; this does not create a Relation.
    #[serde(default)]
    based_on_revision_ids: Vec<String>,
    #[serde(default)]
    observed_at: Option<String>,
    #[serde(default)]
    source_span: Option<SourceSpan>,
    #[serde(default)]
    external_event_id: Option<String>,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitFeedbackParams {
    #[serde(default)]
    scope: Option<String>,
    kind: FeedbackKind,
    #[serde(default = "default_feedback_authority")]
    authority: FeedbackAuthority,
    content: String,
    challenged_revision_ids: Vec<String>,
    #[serde(default)]
    used_revision_ids: Vec<String>,
    #[serde(default)]
    response_ref: Option<String>,
    #[serde(default)]
    observed_at: Option<String>,
    #[serde(default)]
    source_refs: Vec<SourceRef>,
    #[serde(default)]
    external_event_id: Option<String>,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteSummaryParams {
    target_page_id: String,
    target_revision_id: String,
    content: String,
    #[serde(default)]
    based_on_revision_ids: Vec<String>,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractTopicParams {
    /// Existing Topic head to refresh when the selected sources continue the
    /// same stable subject.
    #[serde(default)]
    target_topic: Option<PageRevisionRef>,
    /// Ordered exact current source Page/revision pairs from one Scope.
    source_pages: Vec<PageRevisionRef>,
    title: String,
    content: String,
    #[serde(default)]
    based_on_revision_ids: Vec<String>,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssessPageParams {
    target_page_id: String,
    target_revision_id: String,
    standing: ValidityStanding,
    rationale: String,
    evidence_revision_ids: Vec<String>,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisePageParams {
    target_page_id: String,
    expected_revision_id: String,
    content: String,
    #[serde(default)]
    based_on_revision_ids: Vec<String>,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatePagesParams {
    from_page_id: String,
    relation_type: String,
    to_page_id: String,
    #[serde(default)]
    basis_revision_ids: Vec<String>,
}

#[derive(Debug, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadPagesResult {
    pages: Vec<ReadPage>,
}

#[derive(Debug, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationResult {
    operation: String,
    completed: bool,
}

#[derive(Debug, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageWriteResult {
    page_id: String,
    revision_id: String,
    created: bool,
}

#[derive(Debug, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackWriteResult {
    feedback_page_id: String,
    feedback_revision_id: String,
    created: bool,
    challenged_revision_ids: Vec<String>,
    used_revision_ids: Vec<String>,
}

#[derive(Debug, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryWriteResult {
    target_page_id: String,
    target_revision_id: String,
    summary_page_id: String,
    summary_revision_id: String,
    created: bool,
}

#[derive(Debug, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicExtractionResult {
    topic_page_id: String,
    topic_revision_id: String,
    created: bool,
}

#[derive(Debug, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssessmentWriteResult {
    target_page_id: String,
    target_revision_id: String,
    assessment_page_id: String,
    assessment_revision_id: String,
    created: bool,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessLogParams {
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Debug, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessLogResult {
    events: Vec<AccessAuditEvent>,
    next_cursor: Option<String>,
}

impl PcpMcpServer {
    pub fn new(client: Arc<dyn PcpApi>) -> Self {
        Self { client }
    }

    async fn read_exact_revisions(
        &self,
        revision_ids: Vec<String>,
    ) -> Result<Vec<ReadPage>, McpError> {
        if revision_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.client
            .read_pages(ReadPagesRequest {
                page_ids: Vec::new(),
                revision_ids,
                projections: vec![
                    Projection::Manifest,
                    Projection::Sources,
                    Projection::Facets,
                ],
                max_chars: 32_000,
            })
            .await
            .map_err(|error| operation_error("resolve PCP Revisions", error))
    }
}

#[tool_router]
impl PcpMcpServer {
    #[tool(
        name = "pcp_describe",
        description = "Inspect this PCP Store's Identity, protocol capabilities, limits, and integrity before planning a larger operation.",
        annotations(
            title = "Describe PCP Store",
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_describe(&self) -> Result<Json<DescribeResult>, McpError> {
        let integrity = self
            .client
            .integrity_check()
            .await
            .map_err(|error| operation_error("check PCP Store integrity", error))?;
        Ok(Json(DescribeResult {
            identity_id: self.client.identity_id().to_owned(),
            integrity,
            capabilities: self.client.capabilities(),
        }))
    }

    #[tool(
        name = "pcp_ingest_page",
        description = "Ingest one immutable source event into the authenticated PCP identity. Runtime supplies identity, actor, lifecycle, and sealed mutability. Provide text, opaque sourceRefs, or both; optional basedOnRevisionIds records trusted derivation provenance without asserting a Relation, and sourceSpan enables later lossless packing.",
        annotations(
            title = "Ingest PCP Source Page",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_ingest_page(
        &self,
        Parameters(params): Parameters<IngestPageParams>,
    ) -> Result<Json<PageWriteResult>, McpError> {
        let namespace = operation_scope(
            self.client.as_ref(),
            params.scope.as_deref(),
            AccessPermission::Ingest,
            "ingest",
        )?;
        let payload = params.content.map(|content| PagePayload {
            media_type: "text/markdown".to_owned(),
            content,
        });
        let written = self
            .client
            .ingest_page(IngestPageRequest {
                namespace,
                kind: params.kind,
                observed_at: params.observed_at,
                source_span: params.source_span,
                payload,
                source_refs: params.source_refs,
                based_on_revision_ids: params.based_on_revision_ids,
                facets: None,
                external_event_id: params.external_event_id,
            })
            .await
            .map_err(|error| operation_error("ingest PCP Page", error))?;
        Ok(Json(PageWriteResult {
            page_id: written.page_id,
            revision_id: written.revision_id,
            created: written.created,
        }))
    }

    #[tool(
        name = "pcp_submit_feedback",
        description = "Record an explicit user or tenant challenge against exact recalled Revisions. challengedRevisionIds names the disputed evidence; usedRevisionIds records the full exact context actually used by the tenant response. PCP stores the feedback for reconciliation but does not dereference responseRef or tenant-owned sources.",
        annotations(
            title = "Submit PCP Feedback",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_submit_feedback(
        &self,
        Parameters(params): Parameters<SubmitFeedbackParams>,
    ) -> Result<Json<FeedbackWriteResult>, McpError> {
        let namespace = operation_scope(
            self.client.as_ref(),
            params.scope.as_deref(),
            AccessPermission::Ingest,
            "feedback submission",
        )?;
        let written = self
            .client
            .submit_feedback(SubmitFeedbackRequest {
                namespace,
                kind: params.kind,
                authority: params.authority,
                payload: PagePayload {
                    media_type: "text/markdown".to_owned(),
                    content: params.content,
                },
                observed_at: params.observed_at,
                source_refs: params.source_refs,
                challenged_revision_ids: params.challenged_revision_ids,
                used_revision_ids: params.used_revision_ids,
                response_ref: params.response_ref,
                external_event_id: params.external_event_id,
            })
            .await
            .map_err(|error| operation_error("submit PCP feedback", error))?;
        Ok(Json(FeedbackWriteResult {
            feedback_page_id: written.feedback_page_id,
            feedback_revision_id: written.feedback_revision_id,
            created: written.created,
            challenged_revision_ids: written.challenged_revision_ids,
            used_revision_ids: written.used_revision_ids,
        }))
    }

    #[tool(
        name = "pcp_whoami",
        description = "Inspect the server-injected client principal, session, exact Scope grants, and operation permissions. Tool arguments cannot change this identity.",
        annotations(
            title = "Inspect PCP Access Session",
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_whoami(&self) -> Result<Json<WhoAmIResult>, McpError> {
        Ok(Json(WhoAmIResult {
            access: self.client.access().clone(),
        }))
    }

    #[tool(
        name = "pcp_list_scopes",
        description = "List the authorized PCP Scopes available to this server. Use this before cross-project search or writes when the namespace is unknown.",
        annotations(
            title = "List PCP Scopes",
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_list_scopes(
        &self,
        Parameters(params): Parameters<ListScopesParams>,
    ) -> Result<Json<ListScopesResult>, McpError> {
        let (scopes, next_cursor) = self
            .client
            .list_scopes(Vec::new(), params.query, params.limit, params.cursor)
            .await
            .map_err(|error| operation_error("list PCP Scopes", error))?;
        Ok(Json(ListScopesResult {
            scopes,
            next_cursor,
        }))
    }

    #[tool(
        name = "pcp_create_scope",
        description = "Create or confirm a PCP Scope owned by this Store before writing Pages into a new namespace.",
        annotations(
            title = "Create PCP Scope",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_create_scope(
        &self,
        Parameters(request): Parameters<CreateScopeRequest>,
    ) -> Result<Json<OperationResult>, McpError> {
        self.client
            .create_scope(request)
            .await
            .map_err(|error| operation_error("create PCP Scope", error))?;
        Ok(Json(OperationResult {
            operation: "create_scope".to_owned(),
            completed: true,
        }))
    }

    #[tool(
        name = "pcp_search_pages",
        description = "Find current Page heads. Each hit has a stable pageId and exact revisionId. Use auto normally, exact for a literal anchor, graph for one stable Page ID, and recent for time-ordered browsing.",
        annotations(
            title = "Search PCP Pages",
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_search_pages(
        &self,
        Parameters(params): Parameters<SearchPagesParams>,
    ) -> Result<Json<SearchResult>, McpError> {
        let request = SearchPagesRequest {
            query: params.query,
            scopes: params.scopes,
            mode: parse_search_strategy(params.strategy.as_deref().unwrap_or("auto"))?,
            term_match: SearchTermMatch::Any,
            projections: pcp_core::default_search_projections(),
            filters: SearchFilters::default(),
            limit: params.limit,
            cursor: params.cursor,
        };
        self.client
            .search_pages(request)
            .await
            .map(Json)
            .map_err(|error| operation_error("search PCP Pages", error))
    }

    #[tool(
        name = "pcp_semantic_search",
        description = "Assemble a bounded semantic context result through the configured Runtime. It retrieves independently relevant Pages and uses asserted Relations only as conservative rank adjustments.",
        annotations(
            title = "Semantic Search PCP Context",
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_semantic_search(
        &self,
        Parameters(params): Parameters<QueryContextParams>,
    ) -> Result<Json<serde_json::Value>, McpError> {
        let response = self
            .client
            .semantic_search(QueryContextRequest {
                query: params.query,
                scopes: params.scopes,
                result_limit: Some(params.result_limit),
                context_budget_chars: params.context_budget_chars,
            })
            .await
            .map_err(|error| operation_error("semantic search PCP context", error))?;
        serde_json::to_value(response)
            .map(Json)
            .map_err(|error| operation_error("serialize semantic PCP context", error))
    }

    #[tool(
        name = "pcp_match_intent",
        description = "Ask the configured Runtime Router to expand and review bounded semantic and relation candidates before assembling a context result. Use only when semantic search alone is insufficient.",
        annotations(
            title = "Match PCP Intent",
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_match_intent(
        &self,
        Parameters(params): Parameters<IntentMatchParams>,
    ) -> Result<Json<serde_json::Value>, McpError> {
        let effort = match params.effort {
            IntentEffortParam::Low => IntentEffort::Low,
            IntentEffortParam::Medium => IntentEffort::Medium,
            IntentEffortParam::High => IntentEffort::High,
        };
        let response = self
            .client
            .match_intent(
                QueryContextRequest {
                    query: params.query,
                    scopes: params.scopes,
                    result_limit: Some(params.result_limit),
                    context_budget_chars: params.context_budget_chars,
                },
                effort,
            )
            .await
            .map_err(|error| operation_error("match PCP intent", error))?;
        serde_json::to_value(response)
            .map(Json)
            .map_err(|error| operation_error("serialize PCP intent match", error))
    }

    #[tool(
        name = "pcp_expand_graph",
        description = "Read a bounded ACL-filtered neighborhood around explicit page IDs. This is not an unanchored or whole-graph export; use read view to control node detail.",
        annotations(
            title = "Expand PCP Graph Slice",
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_expand_graph(
        &self,
        Parameters(params): Parameters<ExpandGraphParams>,
    ) -> Result<Json<serde_json::Value>, McpError> {
        let response = pcp_client::expand_graph(
            self.client.as_ref(),
            ExpandGraphRequest {
                anchor_page_ids: params.anchor_page_ids,
                scopes: params.scopes,
                max_depth: params.max_depth,
                max_nodes: params.max_nodes,
                max_edges: params.max_edges,
                projections: read_view(params.view.as_deref().unwrap_or("context"))?,
                max_chars: params.max_chars,
            },
        )
        .await
        .map_err(|error| operation_error("expand PCP graph", error))?;
        serde_json::to_value(response)
            .map(Json)
            .map_err(|error| operation_error("serialize PCP graph slice", error))
    }

    #[tool(
        name = "pcp_browse_index",
        description = "Browse compact routing text without guessing keywords. Follow promising Page IDs with pcp_read_pages.",
        annotations(
            title = "Browse PCP Index",
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_browse_index(
        &self,
        Parameters(params): Parameters<BrowseIndexParams>,
    ) -> Result<Json<SearchResult>, McpError> {
        self.client
            .browse_index(
                params.scopes,
                params.excluded_page_kinds,
                params.order,
                params.limit,
                params.cursor,
                params.max_chars,
            )
            .await
            .map(Json)
            .map_err(|error| operation_error("browse PCP index", error))
    }

    #[tool(
        name = "pcp_read_pages",
        description = "Read current heads by stable pageId and exact snapshots by revisionId. content returns content, context adds interpretation and Page Relations, and full adds source/provenance diagnostics.",
        annotations(
            title = "Read PCP Pages",
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_read_pages(
        &self,
        Parameters(params): Parameters<ReadPagesParams>,
    ) -> Result<Json<ReadPagesResult>, McpError> {
        let request = ReadPagesRequest {
            page_ids: params.page_ids,
            revision_ids: params.revision_ids,
            projections: read_view(params.view.as_deref().unwrap_or("content"))?,
            max_chars: params.max_chars,
        };
        let pages = self
            .client
            .read_pages(request)
            .await
            .map_err(|error| operation_error("read PCP Pages", error))?;
        Ok(Json(ReadPagesResult { pages }))
    }

    #[tool(
        name = "pcp_write_page",
        description = "Advanced Page creation for maintained understanding. Exact source Revisions become provenance; Page Relations must be asserted separately when they have navigation value. Ordinary source events should use pcp_ingest_page.",
        annotations(
            title = "Write PCP Page",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_write_page(
        &self,
        Parameters(params): Parameters<WritePageParams>,
    ) -> Result<Json<PageWriteResult>, McpError> {
        let namespace = operation_scope(
            self.client.as_ref(),
            params.scope.as_deref(),
            AccessPermission::Write,
            "advanced writes",
        )?;
        let actor = session_actor(self.client.as_ref());
        let source_pages = self
            .read_exact_revisions(params.based_on_revision_ids)
            .await?;
        let source_revisions = source_pages
            .iter()
            .map(|page| page.revision.revision_id.clone())
            .collect::<Vec<_>>();
        let request = WritePageRequest {
            namespace,
            lifecycle_status: LifecycleStatus::Active,
            kind: params.kind,
            mutability: params.mutability,
            created_by: actor.clone(),
            observed_at: None,
            source_span: None,
            valid_from: None,
            valid_to: None,
            payload: Some(PagePayload {
                media_type: "text/markdown".to_owned(),
                content: params.content,
            }),
            source_refs: Vec::new(),
            facets: None,
            provenance: (!source_revisions.is_empty())
                .then(|| provenance("derive", &actor, source_revisions))
                .into_iter()
                .collect(),
            initial_relations: Vec::new(),
            idempotency_key: None,
        };
        let written = self
            .client
            .write_page(request)
            .await
            .map_err(|error| operation_error("write PCP Page", error))?;
        Ok(Json(PageWriteResult {
            page_id: written.page_id,
            revision_id: written.revision_id,
            created: written.created,
        }))
    }

    #[tool(
        name = "pcp_revise_page",
        description = "Publish a new immutable Revision of one revisioned Page. pageId stays stable and expectedRevisionId prevents overwriting a newer head.",
        annotations(
            title = "Revise PCP Page",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_revise_page(
        &self,
        Parameters(params): Parameters<RevisePageParams>,
    ) -> Result<Json<PageWriteResult>, McpError> {
        let target = self
            .client
            .read_pages(ReadPagesRequest {
                page_ids: vec![params.target_page_id.clone()],
                revision_ids: Vec::new(),
                projections: vec![Projection::Manifest, Projection::Facets],
                max_chars: 256,
            })
            .await
            .map_err(|error| operation_error("read PCP revision target", error))?
            .into_iter()
            .next()
            .ok_or_else(|| operation_error("read PCP revision target", "Page not found"))?;
        if target.revision.revision_id != params.expected_revision_id {
            return Err(operation_error(
                "revise PCP Page",
                "expectedRevisionId is not the current Page head",
            ));
        }
        let actor = session_actor(self.client.as_ref());
        let source_pages = self
            .read_exact_revisions(params.based_on_revision_ids)
            .await?;
        let mut inputs = source_pages
            .iter()
            .map(|page| page.revision.revision_id.clone())
            .collect::<Vec<_>>();
        inputs.push(target.revision.revision_id.clone());
        inputs.sort();
        inputs.dedup();
        let request = RevisePageRequest {
            page_id: target.page.page_id,
            expected_revision_id: params.expected_revision_id,
            created_by: actor.clone(),
            lifecycle_status: LifecycleStatus::Active,
            observed_at: None,
            valid_from: None,
            valid_to: None,
            payload: Some(PagePayload {
                media_type: "text/markdown".to_owned(),
                content: params.content,
            }),
            source_refs: Vec::new(),
            facets: target.revision.facets,
            provenance: vec![provenance("revise", &actor, inputs)],
            initial_relations: Vec::new(),
            idempotency_key: None,
        };
        let written = self
            .client
            .revise_page(request)
            .await
            .map_err(|error| operation_error("revise PCP Page", error))?;
        Ok(Json(PageWriteResult {
            page_id: written.page_id,
            revision_id: written.revision_id,
            created: written.created,
        }))
    }

    #[tool(
        name = "pcp_write_summary",
        description = "Create or revise the stable routing Summary Page for one exact target Revision.",
        annotations(
            title = "Write PCP Summary",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_write_summary(
        &self,
        Parameters(params): Parameters<WriteSummaryParams>,
    ) -> Result<Json<SummaryWriteResult>, McpError> {
        let actor = session_actor(self.client.as_ref());
        let target = self
            .client
            .read_pages(ReadPagesRequest {
                page_ids: Vec::new(),
                revision_ids: vec![params.target_revision_id.clone()],
                projections: vec![Projection::Manifest],
                max_chars: 256,
            })
            .await
            .map_err(|error| operation_error("read PCP Summary target", error))?
            .into_iter()
            .next()
            .ok_or_else(|| operation_error("read PCP Summary target", "Revision not found"))?;
        if target.page.page_id != params.target_page_id {
            return Err(operation_error(
                "write PCP Summary",
                "targetPageId and targetRevisionId do not match",
            ));
        }
        let resolved = self
            .read_exact_revisions(params.based_on_revision_ids)
            .await?;
        let mut inputs = resolved
            .iter()
            .map(|page| page.revision.revision_id.clone())
            .collect::<Vec<_>>();
        inputs.push(params.target_revision_id.clone());
        inputs.sort();
        inputs.dedup();
        let request = WriteSummaryRequest {
            target_page_id: params.target_page_id,
            target_revision_id: params.target_revision_id,
            expected_summary_revision_id: None,
            content: params.content,
            created_by: actor.clone(),
            tool_or_model: Some(actor.actor_id.clone()),
            provenance: vec![provenance("summarize", &actor, inputs)],
            idempotency_key: None,
        };
        let written = self
            .client
            .write_summary(request)
            .await
            .map_err(|error| operation_error("write PCP Summary", error))?;
        Ok(Json(SummaryWriteResult {
            target_page_id: written.target_page_id,
            target_revision_id: written.target_revision_id,
            summary_page_id: written.summary_page_id,
            summary_revision_id: written.summary_revision_id,
            created: written.created,
        }))
    }

    #[tool(
        name = "pcp_extract_topic",
        description = "Create a revisioned topic front-door Page from two or more exact source Revisions in one Scope. This is logical, reversible routing compaction: sources remain readable evidence, but semantic and intent retrieval will prefer the topic Page until it is superseded.",
        annotations(
            title = "Extract PCP Topic",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_extract_topic(
        &self,
        Parameters(params): Parameters<ExtractTopicParams>,
    ) -> Result<Json<TopicExtractionResult>, McpError> {
        if params.source_pages.len() < 2 {
            return Err(operation_error(
                "extract PCP topic",
                "sourcePages requires at least two exact source Pages",
            ));
        }
        let actor = session_actor(self.client.as_ref());
        let source_revision_ids = params
            .source_pages
            .iter()
            .map(|source| source.revision_id.clone())
            .collect::<Vec<_>>();
        let sources = self
            .read_exact_revisions(source_revision_ids.clone())
            .await?;
        if sources.len() != params.source_pages.len()
            || sources
                .iter()
                .zip(&params.source_pages)
                .any(|(source, requested)| {
                    source.page.page_id != requested.page_id
                        || source.revision.revision_id != requested.revision_id
                })
        {
            return Err(operation_error(
                "extract PCP topic",
                "every sourcePages entry must identify one readable exact Page Revision",
            ));
        }
        let mut inputs = source_revision_ids;
        inputs.extend(params.based_on_revision_ids);
        inputs.sort();
        inputs.dedup();
        let written = self
            .client
            .extract_topic(ExtractTopicRequest {
                target_topic: params.target_topic,
                source_pages: params.source_pages,
                title: params.title,
                content: params.content,
                created_by: actor.clone(),
                tool_or_model: Some(actor.actor_id.clone()),
                provenance: vec![provenance("extract_topic", &actor, inputs)],
                idempotency_key: None,
            })
            .await
            .map_err(|error| operation_error("extract PCP topic", error))?;
        Ok(Json(TopicExtractionResult {
            topic_page_id: written.page_id,
            topic_revision_id: written.revision_id,
            created: written.created,
        }))
    }

    #[tool(
        name = "pcp_relate_pages",
        description = "Add one meaningful directed Relation between stable Pages, with optional exact basis Revisions.",
        annotations(
            title = "Relate PCP Pages",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_relate_pages(
        &self,
        Parameters(params): Parameters<RelatePagesParams>,
    ) -> Result<Json<Relation>, McpError> {
        let request = LinkPagesRequest {
            from_page_id: params.from_page_id,
            relation_type: params.relation_type,
            to_page_id: params.to_page_id,
            basis_revision_ids: params.basis_revision_ids,
            created_by: session_actor(self.client.as_ref()),
            idempotency_key: None,
        };
        self.client
            .link_pages(request)
            .await
            .map(Json)
            .map_err(|error| operation_error("link PCP Pages", error))
    }

    #[tool(
        name = "pcp_assess_validity",
        description = "Create or revise the validity assessment for one exact target Revision using exact evidence Revisions.",
        annotations(
            title = "Assess PCP Validity",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_assess_validity(
        &self,
        Parameters(params): Parameters<AssessPageParams>,
    ) -> Result<Json<AssessmentWriteResult>, McpError> {
        let actor = session_actor(self.client.as_ref());
        let target = self
            .read_exact_revisions(vec![params.target_revision_id.clone()])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| operation_error("read PCP validity target", "Revision not found"))?;
        if target.page.page_id != params.target_page_id {
            return Err(operation_error(
                "assess PCP validity",
                "targetPageId and targetRevisionId do not match",
            ));
        }
        self.read_exact_revisions(params.evidence_revision_ids.clone())
            .await?;
        let request = AssessPageValidityRequest {
            target_page_id: params.target_page_id,
            target_revision_id: params.target_revision_id,
            expected_assessment_revision_id: None,
            standing: params.standing,
            rationale: params.rationale,
            scope: None,
            basis_revision_ids: params.evidence_revision_ids,
            created_by: actor.clone(),
            tool_or_model: Some(actor.actor_id.clone()),
            idempotency_key: None,
        };
        let written = self
            .client
            .assess_page_validity(request)
            .await
            .map_err(|error| operation_error("assess PCP Page validity", error))?;
        Ok(Json(AssessmentWriteResult {
            target_page_id: written.target_page_id,
            target_revision_id: written.target_revision_id,
            assessment_page_id: written.assessment_page_id,
            assessment_revision_id: written.assessment_revision_id,
            created: written.created,
        }))
    }

    #[tool(
        name = "pcp_access_log",
        description = "Read recent metadata-only PCP access events visible to this client. Requires audit permission and never includes query or Page content.",
        annotations(
            title = "Read PCP Access Log",
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_access_log(
        &self,
        Parameters(params): Parameters<AccessLogParams>,
    ) -> Result<Json<AccessLogResult>, McpError> {
        let (events, next_cursor) = self
            .client
            .access_log(params.limit, params.cursor)
            .await
            .map_err(|error| operation_error("read PCP access log", error))?;
        Ok(Json(AccessLogResult {
            events,
            next_cursor,
        }))
    }
}

#[tool_handler]
impl ServerHandler for PcpMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("paged-context-protocol", "0.1.0"))
            .with_instructions(SERVER_INSTRUCTIONS)
    }
}

fn default_limit() -> u32 {
    20
}

fn default_max_chars() -> u32 {
    8_000
}

fn default_page_kind() -> String {
    "document".to_owned()
}

fn default_feedback_authority() -> FeedbackAuthority {
    FeedbackAuthority::Unknown
}

fn parse_search_strategy(value: &str) -> Result<SearchMode, McpError> {
    match value {
        "auto" => Ok(SearchMode::Auto),
        "exact" => Ok(SearchMode::Exact),
        "text" => Ok(SearchMode::Text),
        "graph" => Ok(SearchMode::Graph),
        "recent" => Ok(SearchMode::Temporal),
        other => Err(McpError::invalid_params(
            format!("unknown PCP search strategy: {other}"),
            None,
        )),
    }
}

fn read_view(value: &str) -> Result<Vec<Projection>, McpError> {
    match value {
        "content" => Ok(vec![
            Projection::Manifest,
            Projection::Payload,
            Projection::Facets,
        ]),
        "context" => Ok(vec![
            Projection::Manifest,
            Projection::Summary,
            Projection::Validity,
            Projection::Payload,
            Projection::Relations,
            Projection::Facets,
        ]),
        "full" => Ok(vec![
            Projection::Manifest,
            Projection::Summary,
            Projection::Validity,
            Projection::Payload,
            Projection::Sources,
            Projection::Provenance,
            Projection::Relations,
            Projection::Facets,
            Projection::History,
        ]),
        other => Err(McpError::invalid_params(
            format!("unknown PCP read view: {other}"),
            None,
        )),
    }
}

fn operation_scope(
    client: &dyn PcpApi,
    requested: Option<&str>,
    permission: AccessPermission,
    operation: &str,
) -> Result<String, McpError> {
    let scopes = client.access().scopes_with_permissions(&[permission]);
    if let Some(requested) = requested {
        return scopes
            .contains(&requested.to_owned())
            .then(|| requested.to_owned())
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!("Scope is not authorized for {operation}: {requested}"),
                    None,
                )
            });
    }
    match scopes.as_slice() {
        [only] => Ok(only.clone()),
        [] => Err(McpError::invalid_params(
            format!("this PCP session has no Scope authorized for {operation}"),
            None,
        )),
        _ => Err(McpError::invalid_params(
            format!(
                "scope is required when this PCP session is authorized for {operation} in more than one Scope"
            ),
            None,
        )),
    }
}

fn session_actor(client: &dyn PcpApi) -> Actor {
    let principal = &client.access().principal;
    let actor_type = match principal.principal_type {
        pcp_core::AccessPrincipalType::Host => ActorType::System,
        pcp_core::AccessPrincipalType::ModelClient => ActorType::Model,
        pcp_core::AccessPrincipalType::Cli | pcp_core::AccessPrincipalType::Service => {
            ActorType::Tool
        }
    };
    Actor {
        actor_type,
        actor_id: principal.principal_id.clone(),
    }
}

fn provenance(operation: &str, actor: &Actor, input_revision_ids: Vec<String>) -> ProvenanceEvent {
    ProvenanceEvent {
        operation: operation.to_owned(),
        actor: actor.clone(),
        timestamp: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        input_revision_ids,
        tool_or_model: Some(actor.actor_id.clone()),
        reason: None,
    }
}

fn operation_error(context: &str, error: impl std::fmt::Display) -> McpError {
    McpError::internal_error(format!("{context}: {error}"), None)
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::SystemTime};

    use pcp_client::{AccessMode, EmbeddedPcpClient, PcpApi};
    use pcp_core::{
        AccessPrincipal, AccessPrincipalType, AccessSession, CreateScopeRequest, FeedbackAuthority,
        FeedbackKind,
    };
    use pcp_sqlite::SqlitePcpStore;
    use pcp_store::PcpStore;
    use rmcp::{ServiceExt, handler::server::wrapper::Parameters, model::CallToolRequestParams};

    use super::{
        AccessLogParams, IngestPageParams, PcpMcpServer, SearchPagesParams, SubmitFeedbackParams,
        WritePageParams,
    };

    #[tokio::test]
    async fn tools_write_search_and_enforce_scope_access() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("pcp-mcp-{nonce}"));
        std::fs::create_dir_all(&root).expect("create test directory");
        let store = Arc::new(
            SqlitePcpStore::open(root.join("context.sqlite3"))
                .await
                .expect("open store"),
        );
        let namespace = "project:mcp-test".to_owned();
        let server = PcpMcpServer::new(full_client(store, vec![namespace.clone()]));

        server
            .pcp_create_scope(Parameters(CreateScopeRequest {
                namespace: namespace.clone(),
                display_name: "MCP Test".to_owned(),
                description: None,
                parent_namespace: None,
            }))
            .await
            .expect("create authorized scope");
        assert!(
            server
                .pcp_create_scope(Parameters(CreateScopeRequest {
                    namespace: "project:denied".to_owned(),
                    display_name: "Denied".to_owned(),
                    description: None,
                    parent_namespace: None,
                }))
                .await
                .is_err()
        );

        let written = server
            .pcp_write_page(Parameters(WritePageParams {
                scope: Some(namespace.clone()),
                kind: "document".to_owned(),
                mutability: pcp_core::PageMutability::Sealed,
                content: "A durable context engine preserves exact Page identity.".to_owned(),
                based_on_revision_ids: Vec::new(),
            }))
            .await
            .expect("write page")
            .0;

        let found = server
            .pcp_search_pages(Parameters(SearchPagesParams {
                query: "Page identity".to_owned(),
                scopes: Vec::new(),
                strategy: Some("text".to_owned()),
                limit: 10,
                cursor: None,
            }))
            .await
            .expect("search page")
            .0;
        assert_eq!(found.hits.len(), 1);
        assert_eq!(found.hits[0].page_id, written.page_id);
        let who = server.pcp_whoami().await.expect("inspect access session").0;
        assert_eq!(who.access.principal.principal_id, "client:pcp-mcp-test");
        let audit = server
            .pcp_access_log(Parameters(AccessLogParams {
                limit: 20,
                cursor: None,
            }))
            .await
            .expect("read access log")
            .0;
        assert!(audit.events.iter().any(|event| {
            event.operation == "write_page" && event.principal.principal_id == "client:pcp-mcp-test"
        }));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn contribute_session_ingests_without_advanced_write_authority() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("pcp-mcp-contribute-{nonce}"));
        std::fs::create_dir_all(&root).expect("create test directory");
        let store = Arc::new(
            SqlitePcpStore::open(root.join("context.sqlite3"))
                .await
                .expect("open store"),
        );
        let namespace = "project:mcp-contribute-test".to_owned();
        PcpMcpServer::new(full_client(Arc::clone(&store), vec![namespace.clone()]))
            .pcp_create_scope(Parameters(CreateScopeRequest {
                namespace: namespace.clone(),
                display_name: "MCP contribute test".to_owned(),
                description: None,
                parent_namespace: None,
            }))
            .await
            .expect("create authorized scope");

        let tenant = PcpMcpServer::new(contribute_client(store, vec![namespace.clone()]));
        let ingested = tenant
            .pcp_ingest_page(Parameters(IngestPageParams {
                scope: Some(namespace.clone()),
                kind: "conversation_event".to_owned(),
                content: Some("A tenant contributes one source event.".to_owned()),
                source_refs: Vec::new(),
                based_on_revision_ids: Vec::new(),
                observed_at: None,
                source_span: None,
                external_event_id: Some("mcp:contribute:test".to_owned()),
            }))
            .await
            .expect("ingest with contribute permission")
            .0;
        let feedback = tenant
            .pcp_submit_feedback(Parameters(SubmitFeedbackParams {
                scope: Some(namespace.clone()),
                kind: FeedbackKind::Challenge,
                authority: FeedbackAuthority::TenantAssertion,
                content: "The user explicitly challenged the recalled event.".to_owned(),
                challenged_revision_ids: vec![ingested.revision_id.clone()],
                used_revision_ids: vec![ingested.revision_id],
                response_ref: Some("tenant:response:mcp-test".to_owned()),
                observed_at: None,
                source_refs: Vec::new(),
                external_event_id: Some("mcp:feedback:test".to_owned()),
            }))
            .await
            .expect("submit feedback with contribute permission")
            .0;
        assert!(feedback.created);
        assert!(
            tenant
                .pcp_write_page(Parameters(WritePageParams {
                    scope: Some(namespace),
                    kind: "maintained_claim".to_owned(),
                    mutability: pcp_core::PageMutability::Revisioned,
                    content: "A tenant must not publish maintained state.".to_owned(),
                    based_on_revision_ids: Vec::new(),
                }))
                .await
                .is_err()
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn stdio_protocol_initializes_and_advertises_structured_tools() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("pcp-mcp-wire-{nonce}"));
        std::fs::create_dir_all(&root).expect("create test directory");
        let store = Arc::new(
            SqlitePcpStore::open(root.join("context.sqlite3"))
                .await
                .expect("open store"),
        );
        let server =
            PcpMcpServer::new(full_client(store, vec!["project:protocol-test".to_owned()]));
        let (server_io, client_io) = tokio::io::duplex(128 * 1024);
        let server_task = tokio::spawn(async move {
            server
                .serve(server_io)
                .await
                .expect("initialize server")
                .waiting()
                .await
                .expect("run server");
        });
        let client = ().serve(client_io).await.expect("initialize client");
        let tools = client.list_all_tools().await.expect("list tools");
        assert!(tools.iter().any(|tool| tool.name == "pcp_search_pages"));
        assert!(tools.iter().any(|tool| tool.name == "pcp_whoami"));
        assert!(tools.iter().any(|tool| tool.name == "pcp_access_log"));
        assert!(tools.iter().any(|tool| tool.name == "pcp_submit_feedback"));
        assert!(tools.iter().any(|tool| {
            tool.name == "pcp_write_page"
                && tool
                    .annotations
                    .as_ref()
                    .and_then(|annotations| annotations.read_only_hint)
                    == Some(false)
        }));
        let described = client
            .call_tool(CallToolRequestParams::new("pcp_describe"))
            .await
            .expect("call describe");
        assert!(described.structured_content.is_some());
        let who = client
            .call_tool(CallToolRequestParams::new("pcp_whoami"))
            .await
            .expect("call whoami");
        assert!(who.structured_content.is_some());
        drop(client);
        server_task.await.expect("join server");
        let _ = std::fs::remove_dir_all(root);
    }

    fn full_client(store: Arc<SqlitePcpStore>, scopes: Vec<String>) -> Arc<dyn PcpApi> {
        let access = AccessSession::full_control(
            AccessPrincipal {
                principal_id: "client:pcp-mcp-test".to_owned(),
                principal_type: AccessPrincipalType::ModelClient,
                display_name: Some("PCP MCP test".to_owned()),
            },
            "session:pcp-mcp-test",
            scopes,
        );
        let store: Arc<dyn PcpStore> = store;
        EmbeddedPcpClient::shared(store, access)
    }

    fn contribute_client(store: Arc<SqlitePcpStore>, scopes: Vec<String>) -> Arc<dyn PcpApi> {
        let access = AccessMode::Contribute.session(
            AccessPrincipal {
                principal_id: "client:pcp-mcp-contribute-test".to_owned(),
                principal_type: AccessPrincipalType::ModelClient,
                display_name: Some("PCP MCP contribute test".to_owned()),
            },
            "session:pcp-mcp-contribute-test",
            scopes,
            false,
        );
        let store: Arc<dyn PcpStore> = store;
        EmbeddedPcpClient::shared(store, access)
    }
}
