mod health;

use anyhow::Result;
use async_trait::async_trait;
use pcp_core::{
    AccessAuditEvent, AccessSession, AssessPageValidityRequest, BrowseIndexOrder, Capabilities,
    CollectRevisionRetentionRequest, CreateScopeRequest, IngestPageRequest, LinkPagesRequest,
    PackPagesRequest, PageMutability, PlanRevisionRetentionRequest,
    PutRevisionRetentionLeaseRequest, ReadPage, ReadPagesRequest, Relation, RevisePageRequest,
    RevisionCollectionResult, RevisionRetentionLease, RevisionRetentionPlan, Scope, SearchHit,
    SearchPagesRequest, SearchResult, SourceSpan, WritePageRequest, WriteResult,
    WriteSummaryRequest, WriteSummaryResult, WriteValidityResult,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use health::{
    ActivityHealth, GraphHealth, HealthSnapshot, HealthTimelineBucket, NamedCount, OperationHealth,
    PackingHealth, RecallHealth, ScopeHealth, StorageHealth,
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
    async fn pack_pages(
        &self,
        access: &AccessSession,
        request: PackPagesRequest,
    ) -> Result<WriteResult>;
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
    async fn health_snapshot(
        &self,
        access: &AccessSession,
        requested_scopes: Vec<String>,
        window_hours: u32,
    ) -> Result<HealthSnapshot>;
}
