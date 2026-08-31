mod health;

use anyhow::Result;
use async_trait::async_trait;
use pcp_core::{
    AccessAuditEvent, AccessSession, ApplyReconciliationRequest, ArchivePageRequest,
    AssessPageValidityRequest, BrowseIndexOrder, Capabilities, CollectRevisionRetentionRequest,
    CreateScopeRequest, ExtractTopicRequest, FeedbackSignal, FeedbackSubmission, IngestPageRequest,
    LinkPagesRequest, PackPagesRequest, PageLifecycleTransitionResult, PageMutability,
    PlanRevisionRetentionRequest, PutRevisionRetentionLeaseRequest, QueryAuditEvent, ReadPage,
    ReadPagesRequest, ReconciliationResult, Relation, RepairPageRequest,
    RestoreArchivedPageRequest, RevisePageRequest, RevisionCollectionResult,
    RevisionRetentionLease, RevisionRetentionPlan, Scope, SearchHit, SearchPagesRequest,
    SearchResult, SourceSpan, SubmitFeedbackRequest, UnpackPageRequest, WritePageRequest,
    WriteResult, WriteSummaryRequest, WriteSummaryResult, WriteValidityResult,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use health::{
    ActivityHealth, GraphHealth, HealthSnapshot, HealthTimelineBucket, NamedCount, OperationHealth,
    PackingHealth, QueryAuditMethodHealth, QueryAuditSummary, RecallHealth,
    RuntimeModelUsageHealth, RuntimeModelUsageSourceHealth, ScopeHealth, StorageHealth,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DurablePageInventoryItem {
    pub page_id: String,
    pub revision_id: String,
    pub namespace: String,
    pub kind: String,
    pub mutability: PageMutability,
    pub created_at: String,
    pub observed_at: Option<String>,
    pub source_span: Option<SourceSpan>,
    pub media_type: Option<String>,
    pub content_chars: u64,
    pub snippet: String,
    pub facets: Option<Value>,
    pub summary_revision_id: Option<String>,
    #[serde(default)]
    pub summary_target_revision_id: Option<String>,
    pub summary: Option<String>,
    pub relation_types: Vec<String>,
    /// Exact Revisions recorded as provenance inputs for this current head.
    /// Maintenance may prioritize these pairs, but they are not Relations.
    #[serde(default)]
    pub provenance_input_revision_ids: Vec<String>,
    /// Stable source Page identities for a current extracted Topic head.
    /// Empty for ordinary Pages and legacy Topic heads without extraction
    /// metadata.
    #[serde(default)]
    pub topic_source_page_ids: Vec<String>,
    /// Whether another active Page explicitly supersedes this Page.
    #[serde(default)]
    pub superseded: bool,
    #[serde(default)]
    pub packing_protected: bool,
}

/// Current, user-visible Page heads for a content-library browse.
///
/// This is intentionally separate from [`SearchResult`]: retrieval indexes may
/// choose a summary or other projection as a search surface, while this result
/// always counts and returns the current source Pages in the library.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentLibraryResult {
    pub hits: Vec<SearchHit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub total_pages: u64,
    pub total_content_chars: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentLibraryScope {
    pub namespace: String,
    pub page_count: u64,
    pub content_chars: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentLibrarySummary {
    pub page_count: u64,
    pub content_chars: u64,
    pub scopes: Vec<ContentLibraryScope>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TombstoneCascadeResult {
    pub retracted_revision_ids: Vec<String>,
    pub restored_page_ids: Vec<String>,
    pub tombstone_revision_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnpackPageResult {
    pub retired_page_id: String,
    pub retired_revision_id: String,
    pub restored_pages: Vec<pcp_core::PageRevisionRef>,
    pub retracted_relation_ids: Vec<String>,
}

#[async_trait]
pub trait PcpStore: Send + Sync {
    fn identity_id(&self) -> &str;
    fn capabilities(&self) -> Capabilities;

    async fn integrity_check(&self) -> Result<String>;
    async fn local_scope_names(&self) -> Result<Vec<String>>;
    async fn create_scope(&self, access: &AccessSession, request: CreateScopeRequest)
    -> Result<()>;
    async fn list_scopes(
        &self,
        access: &AccessSession,
        requested_scopes: Vec<String>,
        query: Option<String>,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<(Vec<Scope>, Option<String>)>;
    async fn search_pages(
        &self,
        access: &AccessSession,
        request: SearchPagesRequest,
    ) -> Result<SearchResult>;
    async fn browse_index(
        &self,
        access: &AccessSession,
        scopes: Vec<String>,
        excluded_page_kinds: Vec<String>,
        order: BrowseIndexOrder,
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
    ) -> Result<SearchResult>;
    async fn browse_content_pages(
        &self,
        access: &AccessSession,
        scopes: Vec<String>,
        query: Option<String>,
        order: BrowseIndexOrder,
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
    ) -> Result<ContentLibraryResult>;
    /// Browse the default retrieval surface. A current extracted topic Page
    /// stands in front of its retained source Pages here.
    async fn browse_retrieval_pages(
        &self,
        access: &AccessSession,
        scopes: Vec<String>,
        query: Option<String>,
        order: BrowseIndexOrder,
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
    ) -> Result<ContentLibraryResult>;
    async fn content_library_summary(
        &self,
        access: &AccessSession,
        requested_scopes: Vec<String>,
    ) -> Result<ContentLibrarySummary>;
    async fn read_pages(
        &self,
        access: &AccessSession,
        request: ReadPagesRequest,
    ) -> Result<Vec<ReadPage>>;
    async fn current_revision_id(&self, access: &AccessSession, page_id: String) -> Result<String>;
    async fn page_count(
        &self,
        access: &AccessSession,
        requested_scopes: Vec<String>,
    ) -> Result<u64>;
    async fn content_char_count(
        &self,
        access: &AccessSession,
        requested_scopes: Vec<String>,
    ) -> Result<usize>;
    async fn plan_revision_retention(
        &self,
        access: &AccessSession,
        request: PlanRevisionRetentionRequest,
    ) -> Result<RevisionRetentionPlan>;
    async fn collect_revision_retention(
        &self,
        access: &AccessSession,
        request: CollectRevisionRetentionRequest,
    ) -> Result<RevisionCollectionResult>;
    async fn put_revision_retention_lease(
        &self,
        access: &AccessSession,
        request: PutRevisionRetentionLeaseRequest,
    ) -> Result<RevisionRetentionLease>;
    async fn active_revision_retention_leases(
        &self,
        access: &AccessSession,
        requested_scopes: Vec<String>,
        limit: u32,
    ) -> Result<Vec<RevisionRetentionLease>>;
    async fn ingest_page(
        &self,
        access: &AccessSession,
        request: IngestPageRequest,
    ) -> Result<WriteResult>;
    async fn submit_feedback(
        &self,
        access: &AccessSession,
        request: SubmitFeedbackRequest,
    ) -> Result<FeedbackSubmission>;
    async fn write_page(
        &self,
        access: &AccessSession,
        request: WritePageRequest,
    ) -> Result<WriteResult>;
    async fn revise_page(
        &self,
        access: &AccessSession,
        request: RevisePageRequest,
    ) -> Result<WriteResult>;
    /// Runtime/admin repair for a current Page head. Implementations must
    /// retain the replaced Revision and enforce optimistic concurrency.
    async fn repair_page(
        &self,
        access: &AccessSession,
        request: RepairPageRequest,
    ) -> Result<WriteResult>;
    async fn archive_page(
        &self,
        access: &AccessSession,
        request: ArchivePageRequest,
    ) -> Result<PageLifecycleTransitionResult>;
    async fn restore_archived_page(
        &self,
        access: &AccessSession,
        request: RestoreArchivedPageRequest,
    ) -> Result<PageLifecycleTransitionResult>;
    async fn pack_pages(
        &self,
        access: &AccessSession,
        request: PackPagesRequest,
    ) -> Result<WriteResult>;
    async fn unpack_page(
        &self,
        access: &AccessSession,
        request: UnpackPageRequest,
    ) -> Result<UnpackPageResult>;
    async fn link_pages(
        &self,
        access: &AccessSession,
        request: LinkPagesRequest,
    ) -> Result<Relation>;
    async fn write_summary(
        &self,
        access: &AccessSession,
        request: WriteSummaryRequest,
    ) -> Result<WriteSummaryResult>;
    async fn extract_topic(
        &self,
        access: &AccessSession,
        request: ExtractTopicRequest,
    ) -> Result<WriteResult>;
    async fn next_summary_candidate(
        &self,
        access: &AccessSession,
        minimum_chars: usize,
        excluded_page_kinds: Vec<String>,
    ) -> Result<Option<String>>;
    async fn mark_summary_assessed(
        &self,
        access: &AccessSession,
        target_revision_id: String,
        outcome: String,
        tool_or_model: Option<String>,
    ) -> Result<()>;
    async fn assess_page_validity(
        &self,
        access: &AccessSession,
        request: AssessPageValidityRequest,
    ) -> Result<WriteValidityResult>;
    async fn pending_feedback(
        &self,
        access: &AccessSession,
        requested_scopes: Vec<String>,
        limit: u32,
    ) -> Result<Vec<FeedbackSignal>>;
    async fn apply_reconciliation(
        &self,
        access: &AccessSession,
        request: ApplyReconciliationRequest,
    ) -> Result<ReconciliationResult>;
    async fn tombstone_derivation_cascade(
        &self,
        access: &AccessSession,
        root_revision_id: String,
        actor: pcp_core::Actor,
    ) -> Result<TombstoneCascadeResult>;
    async fn durable_page_inventory(
        &self,
        access: &AccessSession,
        excluded_page_kinds: Vec<String>,
    ) -> Result<Vec<DurablePageInventoryItem>>;
    async fn access_log(
        &self,
        access: &AccessSession,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<(Vec<AccessAuditEvent>, Option<String>)>;
    /// Runtime-owned, content-free query observability. This is intentionally
    /// separate from tenant operations: the Runtime writes it after an actual
    /// provider query has completed.
    async fn record_runtime_query_audit(&self, event: QueryAuditEvent) -> Result<()>;
    /// Content-free model usage emitted by Runtime query and maintenance
    /// workers. This must never contain prompts, Page content, or output text.
    async fn record_runtime_usage(&self, event: pcp_core::RuntimeUsageEvent) -> Result<()>;
    async fn query_audit_summary(
        &self,
        access: &AccessSession,
        requested_scopes: Vec<String>,
        window_hours: u32,
    ) -> Result<QueryAuditSummary>;
    async fn health_snapshot(
        &self,
        access: &AccessSession,
        requested_scopes: Vec<String>,
        window_hours: u32,
    ) -> Result<HealthSnapshot>;
}
