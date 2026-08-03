use std::sync::Arc;

use chrono::{SecondsFormat, Utc};
use pcp_client::PcpApi;
use pcp_core::{
    AccessAuditEvent, AccessPermission, AccessSession, Actor, ActorType, AssessPageValidityRequest,
    Capabilities, CreateScopeRequest, InitialRelation, LifecycleStatus, LinkPagesRequest,
    PagePayload, Projection, ProvenanceEvent, ReadPage, ReadPagesRequest, Relation,
    RevisePageRequest, Scope, SearchFilters, SearchMode, SearchPagesRequest, SearchResult,
    SearchTermMatch, ValidityStanding, WritePageRequest, WriteSummaryRequest,
};
use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::wrapper::{Json, Parameters},
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Serialize;

const SERVER_INSTRUCTIONS: &str = "PCP is a durable graph of immutable Pages. Call pcp_whoami before cross-Scope work. Search or browse compact routing text, then read only useful exact Pages. On writes, provide content and exact source Page IDs; the host records actor, time, provenance, and structural Relations. A later correction or Summary is a new Page, never an in-place edit. PCP does not decide user profile, conversation policy, or what deserves attention; those remain Host decisions.";

#[derive(Clone)]
pub struct PcpMcpServer {
    client: Arc<dyn PcpApi>,
}

#[derive(Debug, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeResult {
    owner_id: String,
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
    page_ids: Vec<String>,
    #[serde(default)]
    view: Option<String>,
    #[serde(default = "default_max_chars")]
    max_chars: u32,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WritePageParams {
    #[serde(default)]
    scope: Option<String>,
    content: String,
    #[serde(default)]
    based_on_page_ids: Vec<String>,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteSummaryParams {
    target_page_id: String,
    content: String,
    #[serde(default)]
    based_on_page_ids: Vec<String>,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssessPageParams {
    target_page_id: String,
    standing: ValidityStanding,
    rationale: String,
    evidence_page_ids: Vec<String>,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupersedePageParams {
    target_page_id: String,
    content: String,
    #[serde(default)]
    based_on_page_ids: Vec<String>,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatePagesParams {
    from_page_id: String,
    relation_type: String,
    to_page_id: String,
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
    created: bool,
}

#[derive(Debug, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryWriteResult {
    target_page_id: String,
    summary_page_id: String,
    created: bool,
}

#[derive(Debug, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssessmentWriteResult {
    target_page_id: String,
    assessment_page_id: String,
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
}

#[tool_router]
impl PcpMcpServer {
    #[tool(
        name = "pcp_describe",
        description = "Inspect this PCP Store's owner, protocol capabilities, limits, and integrity before planning a larger operation.",
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
            owner_id: self.client.owner_id().to_owned(),
            integrity,
            capabilities: self.client.capabilities(),
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
        description = "Find immutable Page candidates. Use auto normally, exact for a literal anchor, graph for one Page ID, and recent for time-ordered browsing. Read only selected Pages.",
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
        description = "Read exact immutable Pages. content returns the Page itself, context adds interpretation and nearby Relations, and full adds source/provenance diagnostics.",
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
            revision_ids: params.page_ids,
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
        description = "Write one immutable durable Page. Supply its content, optional Scope, and exact Pages it is based on; the host records actor, time, provenance, and derived_from links.",
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
        let namespace = writable_scope(self.client.as_ref(), params.scope.as_deref())?;
        let actor = session_actor(self.client.as_ref());
        let request = WritePageRequest {
            owner_id: self.client.owner_id().to_owned(),
            namespace,
            visibility: "private".to_owned(),
            lifecycle_status: LifecycleStatus::Active,
            created_by: actor.clone(),
            observed_at: None,
            valid_from: None,
            valid_to: None,
            payload: Some(PagePayload {
                media_type: "text/markdown".to_owned(),
                content: params.content,
            }),
            source_refs: Vec::new(),
            facets: None,
            provenance: (!params.based_on_page_ids.is_empty())
                .then(|| provenance("derive", &actor, params.based_on_page_ids.clone()))
                .into_iter()
                .collect(),
            initial_relations: derived_relations(params.based_on_page_ids),
            idempotency_key: None,
        };
        let written = self
            .client
            .write_page(request)
            .await
            .map_err(|error| operation_error("write PCP Page", error))?;
        Ok(Json(PageWriteResult {
            page_id: written.revision_id,
            created: written.created,
        }))
    }

    #[tool(
        name = "pcp_supersede_page",
        description = "Write an immutable successor to a current Page. The target remains recoverable and the host advances its Ref when one exists.",
        annotations(
            title = "Supersede PCP Page",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_supersede_page(
        &self,
        Parameters(params): Parameters<SupersedePageParams>,
    ) -> Result<Json<PageWriteResult>, McpError> {
        let target = self
            .client
            .read_pages(ReadPagesRequest {
                revision_ids: vec![params.target_page_id.clone()],
                projections: vec![Projection::Manifest, Projection::Facets],
                max_chars: 256,
            })
            .await
            .map_err(|error| operation_error("read PCP supersede target", error))?
            .into_iter()
            .next()
            .ok_or_else(|| operation_error("read PCP supersede target", "Page not found"))?;
        let actor = session_actor(self.client.as_ref());
        let mut inputs = params.based_on_page_ids.clone();
        inputs.push(params.target_page_id.clone());
        inputs.sort();
        inputs.dedup();
        let request = RevisePageRequest {
            page_id: target.revision.page_id,
            expected_revision_id: params.target_page_id,
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
            provenance: vec![provenance("supersede", &actor, inputs)],
            initial_relations: derived_relations(params.based_on_page_ids),
            idempotency_key: None,
        };
        let written = self
            .client
            .revise_page(request)
            .await
            .map_err(|error| operation_error("supersede PCP Page", error))?;
        Ok(Json(PageWriteResult {
            page_id: written.revision_id,
            created: written.created,
        }))
    }

    #[tool(
        name = "pcp_write_summary",
        description = "Write an immutable routing Summary Page for one exact target Page. A later better Summary becomes another Page linked to the prior one.",
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
        let mut inputs = params.based_on_page_ids;
        inputs.push(params.target_page_id.clone());
        inputs.sort();
        inputs.dedup();
        let request = WriteSummaryRequest {
            target_revision_id: params.target_page_id,
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
            target_page_id: written.target_revision_id,
            summary_page_id: written.summary_revision_id,
            created: written.created,
        }))
    }

    #[tool(
        name = "pcp_relate_pages",
        description = "Add one meaningful directed Relation between immutable Pages. Structural Relations are created automatically by dedicated write tools.",
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
            from_revision_id: params.from_page_id,
            relation_type: params.relation_type,
            to_revision_id: params.to_page_id,
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
        description = "Write an immutable assessment Page when later evidence materially changes how another Page should be used. The host links the assessment, evidence, and prior assessment.",
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
        let request = AssessPageValidityRequest {
            target_revision_id: params.target_page_id,
            expected_assessment_id: None,
            standing: params.standing,
            rationale: params.rationale,
            scope: None,
            basis_revision_ids: params.evidence_page_ids,
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
            target_page_id: written.target_revision_id,
            assessment_page_id: written.assessment_id,
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

fn writable_scope(client: &dyn PcpApi, requested: Option<&str>) -> Result<String, McpError> {
    let scopes = client
        .access()
        .scopes_with_permissions(&[AccessPermission::Write]);
    if let Some(requested) = requested {
        return scopes
            .contains(&requested.to_owned())
            .then(|| requested.to_owned())
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!("Scope is not writable in this PCP session: {requested}"),
                    None,
                )
            });
    }
    match scopes.as_slice() {
        [only] => Ok(only.clone()),
        [] => Err(McpError::invalid_params(
            "this PCP session has no writable Scope".to_owned(),
            None,
        )),
        _ => Err(McpError::invalid_params(
            "scope is required when this PCP session can write more than one Scope".to_owned(),
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

fn derived_relations(page_ids: Vec<String>) -> Vec<InitialRelation> {
    page_ids
        .into_iter()
        .map(|to_revision_id| InitialRelation {
            relation_type: "derived_from".to_owned(),
            to_revision_id,
        })
        .collect()
}

fn provenance(operation: &str, actor: &Actor, input_revision_ids: Vec<String>) -> ProvenanceEvent {
    ProvenanceEvent {
        operation: operation.to_owned(),
        actor: actor.clone(),
        timestamp: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        input_revision_ids,
        tool_or_model: Some(actor.actor_id.clone()),
    }
}

fn operation_error(context: &str, error: impl std::fmt::Display) -> McpError {
    McpError::internal_error(format!("{context}: {error}"), None)
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::SystemTime};

    use pcp_client::{EmbeddedPcpClient, PcpApi};
    use pcp_core::{AccessPrincipal, AccessPrincipalType, AccessSession, CreateScopeRequest};
    use pcp_sqlite::SqlitePcpStore;
    use pcp_store::PcpStore;
    use rmcp::{ServiceExt, handler::server::wrapper::Parameters, model::CallToolRequestParams};

    use super::{AccessLogParams, PcpMcpServer, SearchPagesParams, WritePageParams};

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
        let owner_id = store.owner_id().to_owned();
        let namespace = "project:mcp-test".to_owned();
        let server = PcpMcpServer::new(full_client(store, vec![namespace.clone()]));

        server
            .pcp_create_scope(Parameters(CreateScopeRequest {
                owner_id: owner_id.clone(),
                namespace: namespace.clone(),
                display_name: "MCP Test".to_owned(),
                description: None,
                parent_namespace: None,
                visibility: "private".to_owned(),
            }))
            .await
            .expect("create authorized scope");
        assert!(
            server
                .pcp_create_scope(Parameters(CreateScopeRequest {
                    owner_id: owner_id.clone(),
                    namespace: "project:denied".to_owned(),
                    display_name: "Denied".to_owned(),
                    description: None,
                    parent_namespace: None,
                    visibility: "private".to_owned(),
                }))
                .await
                .is_err()
        );

        let written = server
            .pcp_write_page(Parameters(WritePageParams {
                scope: Some(namespace.clone()),
                content: "A durable context engine preserves exact Page identity.".to_owned(),
                based_on_page_ids: Vec::new(),
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
        assert_eq!(found.hits[0].revision_id, written.page_id);
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
}
