use std::sync::Arc;
pub mod context_hub;
pub mod model_context;
use std::{
    collections::{BTreeSet, HashSet, VecDeque},
    str::FromStr,
};

use anyhow::Result;
use async_trait::async_trait;
use pcp_core::{
    AccessAuditEvent, AccessPermission, AccessPrincipal, AccessSession, ApplyReconciliationRequest,
    AssessPageValidityRequest, BrowseIndexOrder, Capabilities, CollectRevisionRetentionRequest,
    CreateScopeRequest, ExpandGraphRequest, ExtractTopicRequest, FeedbackSignal,
    FeedbackSubmission, GraphEdgeDirection, GraphSliceEdge, GraphSliceResponse, IngestPageRequest,
    IntentEffort, LinkPagesRequest, PackPagesRequest, PlanRevisionRetentionRequest, Projection,
    PutRevisionRetentionLeaseRequest, QueryContextRequest, QueryContextResponse, ReadPage,
    ReadPagesRequest, ReconciliationResult, Relation, RepairPageRequest, RevisePageRequest,
    RevisionCollectionResult, RevisionRetentionLease, RevisionRetentionPlan, Scope, ScopeGrant,
    SearchFilters, SearchMode, SearchPagesRequest, SearchResult, SearchTermMatch,
    SubmitFeedbackRequest, UnpackPageRequest, WritePageRequest, WriteResult, WriteSummaryRequest,
    WriteSummaryResult, WriteValidityResult,
};
use pcp_store::PcpStore;
pub use pcp_store::{
    ContentLibraryFilter, ContentLibraryResult, ContentLibraryScope, ContentLibrarySummary,
    ContentPageRole, DurablePageInventoryItem, HealthSnapshot, QueryAuditSummary,
    TombstoneCascadeResult, UnpackPageResult,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessMode {
    Observe,
    Read,
    Audit,
    Contribute,
    Write,
    Repair,
    Admin,
}

impl AccessMode {
    pub fn session(
        self,
        principal: AccessPrincipal,
        session_id: impl Into<String>,
        scopes: Vec<String>,
        allow_cross_scope_derivation: bool,
    ) -> AccessSession {
        let grants = scopes
            .into_iter()
            .map(|namespace| ScopeGrant {
                namespace,
                permissions: self.permissions(allow_cross_scope_derivation),
            })
            .collect();
        AccessSession::new(principal, session_id, grants)
    }

    /// Build a Runtime-attested session whose permissions apply to every
    /// current and future Scope in one Store.
    ///
    /// Callers must enforce the deployment boundary before selecting this
    /// constructor. Runtime configuration restricts it to administrative
    /// service endpoints; tenant enrollment never uses it.
    pub fn store_wide_session(
        self,
        principal: AccessPrincipal,
        session_id: impl Into<String>,
        scopes: Vec<String>,
        allow_cross_scope_derivation: bool,
    ) -> AccessSession {
        self.session(principal, session_id, scopes, allow_cross_scope_derivation)
            .with_store_permissions(self.permissions(allow_cross_scope_derivation))
    }

    fn permissions(self, allow_cross_scope_derivation: bool) -> Vec<AccessPermission> {
        if self == Self::Observe {
            return vec![AccessPermission::Observe];
        }
        let mut permissions = vec![
            AccessPermission::ListScopes,
            AccessPermission::Search,
            AccessPermission::ReadSummary,
            AccessPermission::ReadDetail,
        ];
        if matches!(self, Self::Contribute | Self::Write | Self::Admin) {
            permissions.push(AccessPermission::Ingest);
        }
        if matches!(self, Self::Write | Self::Admin) {
            permissions.extend([
                AccessPermission::Write,
                AccessPermission::Revise,
                AccessPermission::Summarize,
                AccessPermission::Link,
                AccessPermission::Assess,
            ]);
        }
        if matches!(self, Self::Repair | Self::Admin) {
            permissions.push(AccessPermission::Repair);
        }
        if matches!(self, Self::Audit | Self::Admin) {
            permissions.push(AccessPermission::Audit);
        }
        if self == Self::Admin {
            permissions.extend([
                AccessPermission::ManageLifecycle,
                AccessPermission::ManageScope,
                AccessPermission::Retract,
                AccessPermission::Collect,
            ]);
        }
        if allow_cross_scope_derivation {
            permissions.push(AccessPermission::DeriveAcrossScopes);
        }
        permissions
    }
}

impl FromStr for AccessMode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "observe" => Ok(Self::Observe),
            "read" => Ok(Self::Read),
            "audit" => Ok(Self::Audit),
            "contribute" => Ok(Self::Contribute),
            "write" => Ok(Self::Write),
            "repair" => Ok(Self::Repair),
            "admin" => Ok(Self::Admin),
            other => anyhow::bail!("unsupported PCP access mode: {other}"),
        }
    }
}

/// Minimal transport-independent surface exposed to an ordinary tenant.
///
/// Implementations may bind an in-process Store or a server-attested remote
/// session. Callers must not depend on the deployment form.
#[async_trait]
pub trait PcpTenantApi: Send + Sync {
    fn identity_id(&self) -> &str;
    fn capabilities(&self) -> Capabilities;
    fn access(&self) -> &AccessSession;

    /// Optional Runtime extension. Plain Store clients remain valid without it.
    async fn context_hub(
        &self,
        _request: context_hub::ContextHubRequest,
    ) -> Result<serde_json::Value> {
        anyhow::bail!("Runtime context hub is unavailable on this endpoint")
    }

    async fn list_scopes(
        &self,
        requested_scopes: Vec<String>,
        query: Option<String>,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<(Vec<Scope>, Option<String>)>;
    async fn search_pages(&self, request: SearchPagesRequest) -> Result<SearchResult>;
    async fn browse_index(
        &self,
        scopes: Vec<String>,
        excluded_page_kinds: Vec<String>,
        order: BrowseIndexOrder,
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
    ) -> Result<SearchResult>;
    async fn browse_content_pages(
        &self,
        scopes: Vec<String>,
        query: Option<String>,
        order: BrowseIndexOrder,
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
        filter: ContentLibraryFilter,
    ) -> Result<ContentLibraryResult>;
    /// Browse the default retrieval surface, where topic extractions stand in
    /// front of their retained source Pages.
    async fn browse_retrieval_pages(
        &self,
        scopes: Vec<String>,
        query: Option<String>,
        order: BrowseIndexOrder,
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
    ) -> Result<ContentLibraryResult>;
    async fn content_library_summary(
        &self,
        requested_scopes: Vec<String>,
    ) -> Result<ContentLibrarySummary>;
    async fn read_pages(&self, request: ReadPagesRequest) -> Result<Vec<ReadPage>>;
    async fn ingest_page(&self, request: IngestPageRequest) -> Result<WriteResult>;
    /// Persist explicit user/tenant feedback against the exact Revisions that
    /// were challenged and actually used by the tenant response.
    async fn submit_feedback(&self, request: SubmitFeedbackRequest) -> Result<FeedbackSubmission>;

    /// Context assembly needs Runtime-owned providers; embedded Store clients
    /// fail closed rather than silently substituting a weaker mode.
    async fn semantic_search(&self, _request: QueryContextRequest) -> Result<QueryContextResponse> {
        anyhow::bail!(
            "semantic_search is unavailable: connect to a Runtime endpoint with an embedding provider configured"
        )
    }

    async fn match_intent(
        &self,
        _request: QueryContextRequest,
        _effort: IntentEffort,
    ) -> Result<QueryContextResponse> {
        anyhow::bail!(
            "match_intent is unavailable: connect to a Runtime endpoint with a Router provider configured"
        )
    }
}

/// Read a small, anchored neighborhood through the same tenant API used for
/// ordinary search/read. Each hop is independently ACL-filtered by the Store.
pub async fn expand_graph(
    client: &dyn PcpTenantApi,
    request: ExpandGraphRequest,
) -> Result<GraphSliceResponse> {
    let mut anchors = request.anchor_page_ids;
    anchors.sort();
    anchors.dedup();
    anyhow::ensure!(
        !anchors.is_empty(),
        "expand_graph requires at least one anchorPageId"
    );
    anyhow::ensure!(
        anchors.len() <= 16,
        "expand_graph accepts at most 16 anchors"
    );

    let max_depth = request.max_depth.unwrap_or(1).clamp(1, 3);
    let max_nodes = request.max_nodes.unwrap_or(64).clamp(1, 240) as usize;
    let max_edges = request.max_edges.unwrap_or(128).clamp(1, 480) as usize;
    anyhow::ensure!(anchors.len() <= max_nodes, "anchorPageIds exceed maxNodes");
    let max_chars = request.max_chars.unwrap_or(24_000).clamp(1_000, 64_000);
    let mut projections = request.projections;
    if projections.is_empty() {
        projections = vec![
            Projection::Manifest,
            Projection::Summary,
            Projection::Payload,
        ];
    }

    let mut seen: BTreeSet<String> = anchors.iter().cloned().collect();
    let mut frontier: VecDeque<(String, u8)> = anchors.into_iter().map(|id| (id, 0)).collect();
    let mut edges = Vec::new();
    let mut edge_keys = HashSet::new();
    let mut truncated = false;

    while let Some((origin, depth)) = frontier.pop_front() {
        if depth >= max_depth || edges.len() >= max_edges {
            truncated |= depth < max_depth;
            continue;
        }
        let result = client
            .search_pages(SearchPagesRequest {
                query: origin.clone(),
                scopes: request.scopes.clone(),
                mode: SearchMode::Graph,
                term_match: SearchTermMatch::All,
                projections: vec![Projection::Manifest],
                filters: SearchFilters::default(),
                limit: 64,
                cursor: None,
            })
            .await?;
        truncated |= result.next_cursor.is_some();
        for hit in result.hits {
            let neighbor = hit.page_id;
            for edge in hit.graph_edges {
                if edges.len() >= max_edges {
                    truncated = true;
                    break;
                }
                let (from_page_id, to_page_id) = match edge.direction {
                    GraphEdgeDirection::Outgoing => (origin.clone(), neighbor.clone()),
                    GraphEdgeDirection::Incoming => (neighbor.clone(), origin.clone()),
                };
                let key = format!(
                    "{}|{}|{}|{:?}|{:?}",
                    from_page_id, to_page_id, edge.relation_type, edge.edge_kind, edge.direction
                );
                if !edge_keys.insert(key) {
                    continue;
                }
                if !seen.contains(&neighbor) && seen.len() >= max_nodes {
                    truncated = true;
                    continue;
                }
                let is_new = seen.insert(neighbor.clone());
                edges.push(GraphSliceEdge {
                    from_page_id,
                    to_page_id,
                    relation_type: edge.relation_type,
                    edge_kind: edge.edge_kind,
                    direction_from_origin: edge.direction,
                    basis_revision_ids: edge.basis_revision_ids,
                });
                if is_new && depth + 1 < max_depth {
                    frontier.push_back((neighbor.clone(), depth + 1));
                }
            }
        }
    }

    let nodes = client
        .read_pages(ReadPagesRequest {
            page_ids: seen.into_iter().collect(),
            revision_ids: Vec::new(),
            projections,
            max_chars,
        })
        .await?;
    Ok(GraphSliceResponse {
        nodes,
        edges,
        truncated,
    })
}

/// Privileged Runtime, maintainer, and operator surface.
///
/// Ordinary tenants should be typed against [`PcpTenantApi`]. This superset
/// retains maintenance operations needed by Runtime-owned policy and local
/// administrative tools.
#[async_trait]
pub trait PcpApi: PcpTenantApi {
    /// Runtime-owned, content-free model accounting. This is only implemented
    /// by local Runtime clients; remote tenants cannot submit telemetry.
    async fn record_runtime_usage(&self, _event: pcp_core::RuntimeUsageEvent) -> Result<()> {
        anyhow::bail!("Runtime model usage recording is unavailable on this client")
    }
    async fn integrity_check(&self) -> Result<String>;
    async fn create_scope(&self, request: CreateScopeRequest) -> Result<()>;
    async fn current_revision_id(&self, page_id: String) -> Result<String>;
    async fn page_count(&self, requested_scopes: Vec<String>) -> Result<u64>;
    async fn content_char_count(&self, requested_scopes: Vec<String>) -> Result<usize>;
    async fn plan_revision_retention(
        &self,
        request: PlanRevisionRetentionRequest,
    ) -> Result<RevisionRetentionPlan>;
    async fn collect_revision_retention(
        &self,
        request: CollectRevisionRetentionRequest,
    ) -> Result<RevisionCollectionResult>;
    async fn put_revision_retention_lease(
        &self,
        request: PutRevisionRetentionLeaseRequest,
    ) -> Result<RevisionRetentionLease>;
    async fn active_revision_retention_leases(
        &self,
        requested_scopes: Vec<String>,
        limit: u32,
    ) -> Result<Vec<RevisionRetentionLease>>;
    async fn write_page(&self, request: WritePageRequest) -> Result<WriteResult>;
    async fn revise_page(&self, request: RevisePageRequest) -> Result<WriteResult>;
    /// Development/admin repair of the current Page head. This is not part of
    /// [`PcpTenantApi`] and requires the Runtime-bound repair permission.
    async fn repair_page(&self, request: RepairPageRequest) -> Result<WriteResult>;
    /// Administrative single-Page deletion, separate from derivation retraction.
    async fn delete_page(&self, request: pcp_core::DeletePageRequest) -> Result<WriteResult>;
    async fn archive_page(
        &self,
        request: pcp_core::ArchivePageRequest,
    ) -> Result<pcp_core::PageLifecycleTransitionResult>;
    async fn restore_archived_page(
        &self,
        request: pcp_core::RestoreArchivedPageRequest,
    ) -> Result<pcp_core::PageLifecycleTransitionResult>;
    async fn pack_pages(&self, request: PackPagesRequest) -> Result<WriteResult>;
    async fn unpack_page(&self, request: UnpackPageRequest) -> Result<UnpackPageResult>;
    async fn link_pages(&self, request: LinkPagesRequest) -> Result<Relation>;
    async fn write_summary(&self, request: WriteSummaryRequest) -> Result<WriteSummaryResult>;
    async fn extract_topic(&self, request: ExtractTopicRequest) -> Result<WriteResult>;
    async fn next_summary_candidate(
        &self,
        minimum_chars: usize,
        excluded_page_kinds: Vec<String>,
    ) -> Result<Option<String>>;
    async fn mark_summary_assessed(
        &self,
        target_revision_id: String,
        outcome: String,
        tool_or_model: Option<String>,
    ) -> Result<()>;
    async fn assess_page_validity(
        &self,
        request: AssessPageValidityRequest,
    ) -> Result<WriteValidityResult>;
    async fn pending_feedback(
        &self,
        requested_scopes: Vec<String>,
        limit: u32,
    ) -> Result<Vec<FeedbackSignal>>;
    async fn apply_reconciliation(
        &self,
        request: ApplyReconciliationRequest,
    ) -> Result<ReconciliationResult>;
    async fn tombstone_derivation_cascade(
        &self,
        root_revision_id: String,
        actor: pcp_core::Actor,
    ) -> Result<TombstoneCascadeResult>;
    async fn durable_page_inventory(
        &self,
        excluded_page_kinds: Vec<String>,
    ) -> Result<Vec<DurablePageInventoryItem>>;
    async fn access_log(
        &self,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<(Vec<AccessAuditEvent>, Option<String>)>;
    async fn health_snapshot(
        &self,
        requested_scopes: Vec<String>,
        window_hours: u32,
    ) -> Result<HealthSnapshot>;
    async fn query_audit_summary(
        &self,
        requested_scopes: Vec<String>,
        window_hours: u32,
    ) -> Result<QueryAuditSummary>;
}

#[derive(Clone)]
pub struct EmbeddedPcpClient {
    store: Arc<dyn PcpStore>,
    access: AccessSession,
    successful_write_observer: Option<Arc<dyn Fn() + Send + Sync>>,
    context_hub: Option<Arc<dyn context_hub::ContextHubService>>,
}

impl EmbeddedPcpClient {
    pub fn new(store: Arc<dyn PcpStore>, access: AccessSession) -> Self {
        Self {
            store,
            access,
            successful_write_observer: None,
            context_hub: None,
        }
    }

    /// Observes successful content mutations issued through this client. The
    /// callback is deliberately attached per client so Runtime can wake
    /// maintenance for tenant/operator writes without treating the
    /// maintainer's own writes as new external pressure.
    pub fn with_successful_write_observer(mut self, observer: Arc<dyn Fn() + Send + Sync>) -> Self {
        self.successful_write_observer = Some(observer);
        self
    }

    pub fn shared(store: Arc<dyn PcpStore>, access: AccessSession) -> Arc<dyn PcpApi> {
        Arc::new(Self::new(store, access))
    }

    pub fn with_context_hub(mut self, service: Arc<dyn context_hub::ContextHubService>) -> Self {
        self.context_hub = Some(service);
        self
    }

    pub fn tenant_shared(store: Arc<dyn PcpStore>, access: AccessSession) -> Arc<dyn PcpTenantApi> {
        Arc::new(Self::new(store, access))
    }

    fn observe_successful_write<T>(&self, result: Result<T>) -> Result<T> {
        if result.is_ok()
            && let Some(observer) = self.successful_write_observer.as_ref()
        {
            observer();
        }
        result
    }
}

#[async_trait]
impl PcpTenantApi for EmbeddedPcpClient {
    async fn context_hub(
        &self,
        request: context_hub::ContextHubRequest,
    ) -> Result<serde_json::Value> {
        let service = self.context_hub.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Runtime context hub is unavailable on this endpoint")
        })?;
        let result = service.execute(&self.access, request).await;
        // Only a formal promotion breaks maintenance backoff. Candidate/card
        // traffic stays operational and must not schedule Page maintenance.
        if result
            .as_ref()
            .is_ok_and(|value| value["status"] == "promoted")
        {
            self.observe_successful_write(result)
        } else {
            result
        }
    }

    fn identity_id(&self) -> &str {
        self.store.identity_id()
    }

    fn capabilities(&self) -> Capabilities {
        let mut capabilities = self.store.capabilities();
        if self.context_hub.is_some() {
            capabilities.features.push("runtime_context_hub".into());
        }
        capabilities
    }

    fn access(&self) -> &AccessSession {
        &self.access
    }

    async fn list_scopes(
        &self,
        requested_scopes: Vec<String>,
        query: Option<String>,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<(Vec<Scope>, Option<String>)> {
        self.store
            .list_scopes(&self.access, requested_scopes, query, limit, cursor)
            .await
    }

    async fn search_pages(&self, request: SearchPagesRequest) -> Result<SearchResult> {
        self.store.search_pages(&self.access, request).await
    }

    async fn browse_index(
        &self,
        scopes: Vec<String>,
        excluded_page_kinds: Vec<String>,
        order: BrowseIndexOrder,
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
    ) -> Result<SearchResult> {
        self.store
            .browse_index(
                &self.access,
                scopes,
                excluded_page_kinds,
                order,
                limit,
                cursor,
                max_chars,
            )
            .await
    }

    async fn browse_content_pages(
        &self,
        scopes: Vec<String>,
        query: Option<String>,
        order: BrowseIndexOrder,
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
        filter: ContentLibraryFilter,
    ) -> Result<ContentLibraryResult> {
        self.store
            .browse_content_pages(
                &self.access,
                scopes,
                query,
                order,
                limit,
                cursor,
                max_chars,
                filter,
            )
            .await
    }

    async fn browse_retrieval_pages(
        &self,
        scopes: Vec<String>,
        query: Option<String>,
        order: BrowseIndexOrder,
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
    ) -> Result<ContentLibraryResult> {
        self.store
            .browse_retrieval_pages(&self.access, scopes, query, order, limit, cursor, max_chars)
            .await
    }

    async fn content_library_summary(
        &self,
        requested_scopes: Vec<String>,
    ) -> Result<ContentLibrarySummary> {
        self.store
            .content_library_summary(&self.access, requested_scopes)
            .await
    }

    async fn read_pages(&self, request: ReadPagesRequest) -> Result<Vec<ReadPage>> {
        self.store.read_pages(&self.access, request).await
    }

    async fn ingest_page(&self, request: IngestPageRequest) -> Result<WriteResult> {
        let result = self.store.ingest_page(&self.access, request).await;
        self.observe_successful_write(result)
    }

    async fn submit_feedback(&self, request: SubmitFeedbackRequest) -> Result<FeedbackSubmission> {
        let result = self.store.submit_feedback(&self.access, request).await;
        self.observe_successful_write(result)
    }
}

#[async_trait]
impl PcpApi for EmbeddedPcpClient {
    async fn record_runtime_usage(&self, event: pcp_core::RuntimeUsageEvent) -> Result<()> {
        self.store.record_runtime_usage(event).await
    }

    async fn integrity_check(&self) -> Result<String> {
        self.store.integrity_check().await
    }

    async fn create_scope(&self, request: CreateScopeRequest) -> Result<()> {
        self.store.create_scope(&self.access, request).await
    }

    async fn current_revision_id(&self, page_id: String) -> Result<String> {
        self.store.current_revision_id(&self.access, page_id).await
    }

    async fn page_count(&self, requested_scopes: Vec<String>) -> Result<u64> {
        self.store.page_count(&self.access, requested_scopes).await
    }

    async fn content_char_count(&self, requested_scopes: Vec<String>) -> Result<usize> {
        self.store
            .content_char_count(&self.access, requested_scopes)
            .await
    }

    async fn plan_revision_retention(
        &self,
        request: PlanRevisionRetentionRequest,
    ) -> Result<RevisionRetentionPlan> {
        self.store
            .plan_revision_retention(&self.access, request)
            .await
    }

    async fn collect_revision_retention(
        &self,
        request: CollectRevisionRetentionRequest,
    ) -> Result<RevisionCollectionResult> {
        self.store
            .collect_revision_retention(&self.access, request)
            .await
    }

    async fn put_revision_retention_lease(
        &self,
        request: PutRevisionRetentionLeaseRequest,
    ) -> Result<RevisionRetentionLease> {
        self.store
            .put_revision_retention_lease(&self.access, request)
            .await
    }

    async fn active_revision_retention_leases(
        &self,
        requested_scopes: Vec<String>,
        limit: u32,
    ) -> Result<Vec<RevisionRetentionLease>> {
        self.store
            .active_revision_retention_leases(&self.access, requested_scopes, limit)
            .await
    }

    async fn write_page(&self, request: WritePageRequest) -> Result<WriteResult> {
        let result = self.store.write_page(&self.access, request).await;
        self.observe_successful_write(result)
    }

    async fn revise_page(&self, request: RevisePageRequest) -> Result<WriteResult> {
        let result = self.store.revise_page(&self.access, request).await;
        self.observe_successful_write(result)
    }

    async fn repair_page(&self, request: RepairPageRequest) -> Result<WriteResult> {
        let result = self.store.repair_page(&self.access, request).await;
        self.observe_successful_write(result)
    }

    async fn delete_page(&self, request: pcp_core::DeletePageRequest) -> Result<WriteResult> {
        let result = self.store.delete_page(&self.access, request).await;
        self.observe_successful_write(result)
    }

    async fn archive_page(
        &self,
        request: pcp_core::ArchivePageRequest,
    ) -> Result<pcp_core::PageLifecycleTransitionResult> {
        let result = self.store.archive_page(&self.access, request).await;
        self.observe_successful_write(result)
    }

    async fn restore_archived_page(
        &self,
        request: pcp_core::RestoreArchivedPageRequest,
    ) -> Result<pcp_core::PageLifecycleTransitionResult> {
        let result = self
            .store
            .restore_archived_page(&self.access, request)
            .await;
        self.observe_successful_write(result)
    }

    async fn pack_pages(&self, request: PackPagesRequest) -> Result<WriteResult> {
        let result = self.store.pack_pages(&self.access, request).await;
        self.observe_successful_write(result)
    }

    async fn unpack_page(&self, request: UnpackPageRequest) -> Result<UnpackPageResult> {
        let result = self.store.unpack_page(&self.access, request).await;
        self.observe_successful_write(result)
    }

    async fn link_pages(&self, request: LinkPagesRequest) -> Result<Relation> {
        let result = self.store.link_pages(&self.access, request).await;
        self.observe_successful_write(result)
    }

    async fn write_summary(&self, request: WriteSummaryRequest) -> Result<WriteSummaryResult> {
        let result = self.store.write_summary(&self.access, request).await;
        self.observe_successful_write(result)
    }

    async fn extract_topic(&self, request: ExtractTopicRequest) -> Result<WriteResult> {
        let result = self.store.extract_topic(&self.access, request).await;
        self.observe_successful_write(result)
    }

    async fn next_summary_candidate(
        &self,
        minimum_chars: usize,
        excluded_page_kinds: Vec<String>,
    ) -> Result<Option<String>> {
        self.store
            .next_summary_candidate(&self.access, minimum_chars, excluded_page_kinds)
            .await
    }

    async fn mark_summary_assessed(
        &self,
        target_revision_id: String,
        outcome: String,
        tool_or_model: Option<String>,
    ) -> Result<()> {
        self.store
            .mark_summary_assessed(&self.access, target_revision_id, outcome, tool_or_model)
            .await
    }

    async fn assess_page_validity(
        &self,
        request: AssessPageValidityRequest,
    ) -> Result<WriteValidityResult> {
        let result = self.store.assess_page_validity(&self.access, request).await;
        self.observe_successful_write(result)
    }

    async fn pending_feedback(
        &self,
        requested_scopes: Vec<String>,
        limit: u32,
    ) -> Result<Vec<FeedbackSignal>> {
        self.store
            .pending_feedback(&self.access, requested_scopes, limit)
            .await
    }

    async fn apply_reconciliation(
        &self,
        request: ApplyReconciliationRequest,
    ) -> Result<ReconciliationResult> {
        let result = self.store.apply_reconciliation(&self.access, request).await;
        self.observe_successful_write(result)
    }

    async fn tombstone_derivation_cascade(
        &self,
        root_revision_id: String,
        actor: pcp_core::Actor,
    ) -> Result<TombstoneCascadeResult> {
        let result = self
            .store
            .tombstone_derivation_cascade(&self.access, root_revision_id, actor)
            .await;
        self.observe_successful_write(result)
    }

    async fn durable_page_inventory(
        &self,
        excluded_page_kinds: Vec<String>,
    ) -> Result<Vec<DurablePageInventoryItem>> {
        self.store
            .durable_page_inventory(&self.access, excluded_page_kinds)
            .await
    }

    async fn access_log(
        &self,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<(Vec<AccessAuditEvent>, Option<String>)> {
        self.store.access_log(&self.access, limit, cursor).await
    }

    async fn health_snapshot(
        &self,
        requested_scopes: Vec<String>,
        window_hours: u32,
    ) -> Result<HealthSnapshot> {
        self.store
            .health_snapshot(&self.access, requested_scopes, window_hours)
            .await
    }

    async fn query_audit_summary(
        &self,
        requested_scopes: Vec<String>,
        window_hours: u32,
    ) -> Result<QueryAuditSummary> {
        self.store
            .query_audit_summary(&self.access, requested_scopes, window_hours)
            .await
    }
}

#[cfg(test)]
mod tests {
    use pcp_core::{AccessPermission, AccessPrincipal, AccessPrincipalType};

    use super::AccessMode;

    #[test]
    fn audit_mode_can_inspect_without_mutating() {
        let session = AccessMode::Audit.session(
            AccessPrincipal {
                principal_id: "operator:test".to_owned(),
                principal_type: AccessPrincipalType::Service,
                display_name: None,
            },
            "session:test",
            vec!["project:test".to_owned()],
            false,
        );

        assert!(session.allows("project:test", AccessPermission::ReadDetail));
        assert!(session.allows("project:test", AccessPermission::Audit));
        assert!(!session.allows("project:test", AccessPermission::Write));
        assert!(!session.allows("project:test", AccessPermission::Retract));
    }

    #[test]
    fn contribute_mode_can_ingest_without_maintenance_authority() {
        let session = AccessMode::Contribute.session(
            AccessPrincipal {
                principal_id: "host:tenant-test".to_owned(),
                principal_type: AccessPrincipalType::Host,
                display_name: None,
            },
            "session:tenant-test",
            vec!["project:test".to_owned()],
            false,
        );

        assert!(session.allows("project:test", AccessPermission::ReadDetail));
        assert!(session.allows("project:test", AccessPermission::Ingest));
        assert!(!session.allows("project:test", AccessPermission::Write));
        assert!(!session.allows("project:test", AccessPermission::Revise));
        assert!(!session.allows("project:test", AccessPermission::Summarize));
        assert!(!session.allows("project:test", AccessPermission::Link));
        assert!(!session.allows("project:test", AccessPermission::Assess));
        assert!(!session.allows("project:test", AccessPermission::Audit));
    }

    #[test]
    fn repair_mode_can_replace_a_head_without_general_mutation_authority() {
        let session = AccessMode::Repair.session(
            AccessPrincipal {
                principal_id: "service:repair-test".to_owned(),
                principal_type: AccessPrincipalType::Service,
                display_name: None,
            },
            "session:repair-test",
            vec!["project:test".to_owned()],
            false,
        );

        assert!(session.allows("project:test", AccessPermission::ReadDetail));
        assert!(session.allows("project:test", AccessPermission::Repair));
        assert!(!session.allows("project:test", AccessPermission::Ingest));
        assert!(!session.allows("project:test", AccessPermission::Write));
        assert!(!session.allows("project:test", AccessPermission::Revise));
        assert!(!session.allows("project:test", AccessPermission::ManageLifecycle));
        assert!(!session.allows("project:test", AccessPermission::ManageScope));
    }

    #[test]
    fn observe_mode_exposes_only_aggregate_observation() {
        let session = AccessMode::Observe.session(
            AccessPrincipal {
                principal_id: "observer:test".to_owned(),
                principal_type: AccessPrincipalType::Service,
                display_name: None,
            },
            "session:observer",
            vec!["project:test".to_owned()],
            false,
        );

        assert!(session.allows("project:test", AccessPermission::Observe));
        assert!(!session.allows("project:test", AccessPermission::ListScopes));
        assert!(!session.allows("project:test", AccessPermission::Search));
        assert!(!session.allows("project:test", AccessPermission::ReadSummary));
        assert!(!session.allows("project:test", AccessPermission::ReadDetail));
        assert!(!session.allows("project:test", AccessPermission::Ingest));
        assert!(!session.allows("project:test", AccessPermission::Audit));
        assert!(!session.allows("project:test", AccessPermission::Write));
        assert!(!session.allows("project:test", AccessPermission::Collect));
    }

    #[test]
    fn admin_mode_can_manage_page_lifecycle() {
        let session = AccessMode::Admin.session(
            AccessPrincipal {
                principal_id: "operator:admin-test".to_owned(),
                principal_type: AccessPrincipalType::Service,
                display_name: None,
            },
            "session:admin-test",
            vec!["project:test".to_owned()],
            false,
        );

        assert!(session.allows("project:test", AccessPermission::ManageLifecycle));
    }

    #[test]
    fn store_wide_admin_applies_to_future_scopes_without_literal_grants() {
        let session = AccessMode::Admin.store_wide_session(
            AccessPrincipal {
                principal_id: "operator:local".to_owned(),
                principal_type: AccessPrincipalType::Service,
                display_name: Some("PCP Console".to_owned()),
            },
            "session:operator",
            Vec::new(),
            true,
        );

        assert!(session.grants.is_empty());
        assert!(session.allows("user:current", AccessPermission::ReadDetail));
        assert!(session.allows("project:created-later", AccessPermission::ManageScope));
        assert!(session.has_store_permissions(&[
            AccessPermission::Search,
            AccessPermission::ReadDetail,
            AccessPermission::ManageLifecycle,
        ]));
        assert!(session.allows(
            "project:created-later",
            AccessPermission::DeriveAcrossScopes
        ));
    }
}
