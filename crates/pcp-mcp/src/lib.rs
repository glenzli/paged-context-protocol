use std::sync::Arc;

use pcp_client::PcpApi;
use pcp_core::{
    AccessAuditEvent, AccessSession, AssessPageValidityRequest, Capabilities, CreateScopeRequest,
    LinkPagesRequest, ReadPage, ReadPagesRequest, Relation, RevisePageRequest, Scope,
    SearchPagesRequest, SearchResult, WritePageRequest, WriteResult, WriteSummaryRequest,
    WriteSummaryResult, WriteValidityResult,
};
use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::wrapper::{Json, Parameters},
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Serialize;

const SERVER_INSTRUCTIONS: &str = "PCP is a durable Page/Revision context store. Call pcp_whoami before planning cross-Scope work. Search or browse the Summary index first, then read exact Revisions only when useful. Preserve Scope, provenance, and source Revision IDs on writes. Summaries route to Detail and never replace it. PCP does not decide user profile, conversation policy, or what deserves attention; those remain Host decisions.";

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

#[derive(Debug, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadPagesResult {
    pages: Vec<ReadPage>,
}

#[derive(Debug, JsonSchema, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentRevisionParams {
    page_id: String,
}

#[derive(Debug, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentRevisionResult {
    page_id: String,
    revision_id: String,
}

#[derive(Debug, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationResult {
    operation: String,
    completed: bool,
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
        description = "Search authorized PCP Pages by Summary, payload, facets, time, exact text, or graph relation. Treat hits as routing candidates and read exact Revisions only when needed.",
        annotations(
            title = "Search PCP Pages",
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_search_pages(
        &self,
        Parameters(request): Parameters<SearchPagesRequest>,
    ) -> Result<Json<SearchResult>, McpError> {
        self.client
            .search_pages(request)
            .await
            .map(Json)
            .map_err(|error| operation_error("search PCP Pages", error))
    }

    #[tool(
        name = "pcp_browse_index",
        description = "Browse the compact PCP Summary index without a query when the relevant wording is unknown. Follow promising Revision IDs with pcp_read_pages.",
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
        description = "Read exact PCP Revisions with explicitly selected Projections. Prefer Summary first and request payload or provenance only when it changes the task.",
        annotations(
            title = "Read PCP Pages",
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_read_pages(
        &self,
        Parameters(request): Parameters<ReadPagesRequest>,
    ) -> Result<Json<ReadPagesResult>, McpError> {
        let pages = self
            .client
            .read_pages(request)
            .await
            .map_err(|error| operation_error("read PCP Pages", error))?;
        Ok(Json(ReadPagesResult { pages }))
    }

    #[tool(
        name = "pcp_current_revision",
        description = "Resolve a stable Page ID to its current authorized Revision ID before revising it.",
        annotations(
            title = "Resolve PCP Revision",
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_current_revision(
        &self,
        Parameters(params): Parameters<CurrentRevisionParams>,
    ) -> Result<Json<CurrentRevisionResult>, McpError> {
        let revision_id = self
            .client
            .current_revision_id(params.page_id.clone())
            .await
            .map_err(|error| operation_error("resolve current PCP Revision", error))?;
        Ok(Json(CurrentRevisionResult {
            page_id: params.page_id,
            revision_id,
        }))
    }

    #[tool(
        name = "pcp_write_page",
        description = "Write a new immutable PCP Page Revision with explicit Scope, actor, source references, provenance, and optional relations.",
        annotations(
            title = "Write PCP Page",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_write_page(
        &self,
        Parameters(request): Parameters<WritePageRequest>,
    ) -> Result<Json<WriteResult>, McpError> {
        self.client
            .write_page(request)
            .await
            .map(Json)
            .map_err(|error| operation_error("write PCP Page", error))
    }

    #[tool(
        name = "pcp_revise_page",
        description = "Append a new Revision to an existing PCP Page using expectedRevisionId for conflict detection; prior Revisions remain recoverable.",
        annotations(
            title = "Revise PCP Page",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_revise_page(
        &self,
        Parameters(request): Parameters<RevisePageRequest>,
    ) -> Result<Json<WriteResult>, McpError> {
        self.client
            .revise_page(request)
            .await
            .map(Json)
            .map_err(|error| operation_error("revise PCP Page", error))
    }

    #[tool(
        name = "pcp_write_summary",
        description = "Write or revise a sparse Summary Projection for one exact target Revision. The Summary routes recall and must retain provenance to Detail.",
        annotations(
            title = "Write PCP Summary",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_write_summary(
        &self,
        Parameters(request): Parameters<WriteSummaryRequest>,
    ) -> Result<Json<WriteSummaryResult>, McpError> {
        self.client
            .write_summary(request)
            .await
            .map(Json)
            .map_err(|error| operation_error("write PCP Summary", error))
    }

    #[tool(
        name = "pcp_link_pages",
        description = "Create a typed directed Relation between two exact PCP Revisions without rewriting either Page.",
        annotations(
            title = "Link PCP Pages",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_link_pages(
        &self,
        Parameters(request): Parameters<LinkPagesRequest>,
    ) -> Result<Json<Relation>, McpError> {
        self.client
            .link_pages(request)
            .await
            .map(Json)
            .map_err(|error| operation_error("link PCP Pages", error))
    }

    #[tool(
        name = "pcp_assess_validity",
        description = "Record a revisable standing for one exact Revision when later evidence confirms, qualifies, disputes, supersedes, or retracts it. This does not delete history.",
        annotations(
            title = "Assess PCP Validity",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pcp_assess_validity(
        &self,
        Parameters(request): Parameters<AssessPageValidityRequest>,
    ) -> Result<Json<WriteValidityResult>, McpError> {
        self.client
            .assess_page_validity(request)
            .await
            .map(Json)
            .map_err(|error| operation_error("assess PCP Page validity", error))
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

fn operation_error(context: &str, error: impl std::fmt::Display) -> McpError {
    McpError::internal_error(format!("{context}: {error}"), None)
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::SystemTime};

    use pcp_client::{EmbeddedPcpClient, PcpApi};
    use pcp_core::{
        AccessPrincipal, AccessPrincipalType, AccessSession, Actor, ActorType, CreateScopeRequest,
        LifecycleStatus, PagePayload, Projection, SearchFilters, SearchMode, SearchPagesRequest,
        SearchTermMatch, WritePageRequest,
    };
    use pcp_sqlite::SqlitePcpStore;
    use pcp_store::PcpStore;
    use rmcp::{ServiceExt, handler::server::wrapper::Parameters, model::CallToolRequestParams};

    use super::{AccessLogParams, PcpMcpServer};

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
                scope_type: "project".to_owned(),
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
                    scope_type: "project".to_owned(),
                    display_name: "Denied".to_owned(),
                    description: None,
                    parent_namespace: None,
                    visibility: "private".to_owned(),
                }))
                .await
                .is_err()
        );

        let written = server
            .pcp_write_page(Parameters(WritePageRequest {
                owner_id,
                namespace: namespace.clone(),
                visibility: "private".to_owned(),
                lifecycle_status: LifecycleStatus::Active,
                created_by: Actor {
                    actor_type: ActorType::Model,
                    actor_id: "model:test".to_owned(),
                },
                observed_at: None,
                valid_from: None,
                valid_to: None,
                payload: Some(PagePayload {
                    media_type: "text/markdown".to_owned(),
                    content: "A durable context engine preserves exact revision identity."
                        .to_owned(),
                }),
                source_refs: Vec::new(),
                facets: None,
                provenance: Vec::new(),
                initial_relations: Vec::new(),
                idempotency_key: Some("mcp:test:write".to_owned()),
            }))
            .await
            .expect("write page")
            .0;

        let found = server
            .pcp_search_pages(Parameters(SearchPagesRequest {
                query: "revision identity".to_owned(),
                scopes: Vec::new(),
                mode: SearchMode::Text,
                term_match: SearchTermMatch::All,
                projections: vec![Projection::Payload],
                filters: SearchFilters::default(),
                limit: 10,
                cursor: None,
            }))
            .await
            .expect("search page")
            .0;
        assert_eq!(found.hits.len(), 1);
        assert_eq!(found.hits[0].revision_id, written.revision_id);
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
