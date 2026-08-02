use std::{collections::HashSet, sync::Arc};

use pcp_core::{
    AssessPageValidityRequest, Capabilities, CreateScopeRequest, LinkPagesRequest, ReadPage,
    ReadPagesRequest, Relation, RevisePageRequest, Scope, SearchPagesRequest, SearchResult,
    WritePageRequest, WriteResult, WriteSummaryRequest, WriteSummaryResult, WriteValidityResult,
};
use pcp_store::PcpStore;
use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::wrapper::{Json, Parameters},
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Serialize;

const SERVER_INSTRUCTIONS: &str = "PCP is a durable Page/Revision context store. Search or browse the Summary index first, then read exact Revisions only when useful. Preserve Scope, provenance, and source Revision IDs on writes. Summaries route to Detail and never replace it. PCP does not decide user profile, conversation policy, or what deserves attention; those remain Host decisions.";

#[derive(Clone)]
pub struct PcpMcpServer {
    store: Arc<dyn PcpStore>,
    restricted_scopes: Option<Arc<HashSet<String>>>,
}

#[derive(Debug, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeResult {
    owner_id: String,
    integrity: String,
    capabilities: Capabilities,
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

impl PcpMcpServer {
    pub fn new(store: Arc<dyn PcpStore>, restricted_scopes: Option<Vec<String>>) -> Self {
        Self {
            store,
            restricted_scopes: restricted_scopes
                .map(|scopes| Arc::new(scopes.into_iter().collect())),
        }
    }

    async fn allowed_scopes(&self, requested: &[String]) -> Result<Vec<String>, McpError> {
        let local = self
            .store
            .local_scope_names()
            .await
            .map_err(|error| operation_error("list authorized PCP Scopes", error))?
            .into_iter()
            .collect::<HashSet<_>>();
        let available = match self.restricted_scopes.as_ref() {
            Some(restricted) => local
                .intersection(restricted)
                .cloned()
                .collect::<HashSet<_>>(),
            None => local,
        };
        if requested.is_empty() {
            let mut scopes = available.into_iter().collect::<Vec<_>>();
            scopes.sort();
            return Ok(scopes);
        }
        let mut resolved = Vec::with_capacity(requested.len());
        for scope in requested {
            if !available.contains(scope) {
                return Err(McpError::invalid_params(
                    format!("PCP Scope is not authorized or does not exist: {scope}"),
                    None,
                ));
            }
            if !resolved.contains(scope) {
                resolved.push(scope.clone());
            }
        }
        Ok(resolved)
    }

    fn authorize_owner(&self, owner_id: &str) -> Result<(), McpError> {
        if owner_id != self.store.owner_id() {
            return Err(McpError::invalid_params(
                "ownerId does not match this PCP Store".to_owned(),
                None,
            ));
        }
        Ok(())
    }

    fn authorize_new_scope(&self, namespace: &str) -> Result<(), McpError> {
        if let Some(restricted) = self.restricted_scopes.as_ref()
            && !restricted.contains(namespace)
        {
            return Err(McpError::invalid_params(
                format!("PCP Scope is outside the configured allow list: {namespace}"),
                None,
            ));
        }
        Ok(())
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
            .store
            .integrity_check()
            .await
            .map_err(|error| operation_error("check PCP Store integrity", error))?;
        Ok(Json(DescribeResult {
            owner_id: self.store.owner_id().to_owned(),
            integrity,
            capabilities: self.store.capabilities(),
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
        let allowed = self.allowed_scopes(&[]).await?;
        let (scopes, next_cursor) = self
            .store
            .list_scopes(allowed, params.query, params.limit, params.cursor)
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
        self.authorize_owner(&request.owner_id)?;
        self.authorize_new_scope(&request.namespace)?;
        self.store
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
        Parameters(mut request): Parameters<SearchPagesRequest>,
    ) -> Result<Json<SearchResult>, McpError> {
        request.scopes = self.allowed_scopes(&request.scopes).await?;
        self.store
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
        let scopes = self.allowed_scopes(&params.scopes).await?;
        self.store
            .browse_index(
                scopes,
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
        let allowed = self.allowed_scopes(&[]).await?;
        let pages = self
            .store
            .read_pages(request, allowed)
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
        let allowed = self.allowed_scopes(&[]).await?;
        let revision_id = self
            .store
            .current_revision_id(params.page_id.clone(), allowed)
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
        self.authorize_owner(&request.owner_id)?;
        let allowed = self
            .allowed_scopes(std::slice::from_ref(&request.namespace))
            .await?;
        self.store
            .write_page(request, allowed)
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
        let allowed = self.allowed_scopes(&[]).await?;
        self.store
            .revise_page(request, allowed)
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
        let allowed = self.allowed_scopes(&[]).await?;
        self.store
            .write_summary(request, allowed)
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
        let allowed = self.allowed_scopes(&[]).await?;
        self.store
            .link_pages(request, allowed)
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
        let allowed = self.allowed_scopes(&[]).await?;
        self.store
            .assess_page_validity(request, allowed)
            .await
            .map(Json)
            .map_err(|error| operation_error("assess PCP Page validity", error))
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

    use pcp_core::{
        Actor, ActorType, CreateScopeRequest, LifecycleStatus, PagePayload, Projection,
        SearchFilters, SearchMode, SearchPagesRequest, SearchTermMatch, WritePageRequest,
    };
    use pcp_sqlite::SqlitePcpStore;
    use rmcp::{ServiceExt, handler::server::wrapper::Parameters, model::CallToolRequestParams};

    use super::PcpMcpServer;

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
        let server = PcpMcpServer::new(store, Some(vec![namespace.clone()]));

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
        let server = PcpMcpServer::new(store, None);
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
        drop(client);
        server_task.await.expect("join server");
        let _ = std::fs::remove_dir_all(root);
    }
}
