use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use pcp_core::{
    AccessAuditEvent, AccessSession, AssessPageValidityRequest, Capabilities, CreateScopeRequest,
    LinkPagesRequest, ReadPage, ReadPagesRequest, Relation, RevisePageRequest, Scope,
    SearchPagesRequest, SearchResult, WritePageRequest, WriteResult, WriteSummaryRequest,
    WriteSummaryResult, WriteValidityResult,
};
use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DurablePageInventoryItem {
    pub page_id: String,
    pub revision_id: String,
    pub namespace: String,
    pub kind: Option<String>,
    pub created_at: String,
    pub observed_at: Option<String>,
    pub content_chars: u64,
    pub snippet: String,
    pub facets: Option<Value>,
    pub summary_revision_id: Option<String>,
    pub summary: Option<String>,
    pub relation_types: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct TombstoneCascadeResult {
    pub retracted_revision_ids: Vec<String>,
    pub restored_page_ids: Vec<String>,
    pub tombstone_revision_ids: Vec<String>,
}

#[async_trait]
pub trait PcpStore: Send + Sync {
    fn owner_id(&self) -> &str;
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
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
    ) -> Result<SearchResult>;
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
}

#[derive(Clone)]
pub struct PcpClient {
    store: Arc<dyn PcpStore>,
    access: AccessSession,
}

impl PcpClient {
    pub fn new(store: Arc<dyn PcpStore>, access: AccessSession) -> Self {
        Self { store, access }
    }

    pub fn owner_id(&self) -> &str {
        self.store.owner_id()
    }

    pub fn capabilities(&self) -> Capabilities {
        self.store.capabilities()
    }

    pub fn access(&self) -> &AccessSession {
        &self.access
    }

    pub async fn integrity_check(&self) -> Result<String> {
        self.store.integrity_check().await
    }

    pub async fn create_scope(&self, request: CreateScopeRequest) -> Result<()> {
        self.store.create_scope(&self.access, request).await
    }

    pub async fn list_scopes(
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

    pub async fn search_pages(&self, request: SearchPagesRequest) -> Result<SearchResult> {
        self.store.search_pages(&self.access, request).await
    }

    pub async fn browse_index(
        &self,
        scopes: Vec<String>,
        excluded_page_kinds: Vec<String>,
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
    ) -> Result<SearchResult> {
        self.store
            .browse_index(
                &self.access,
                scopes,
                excluded_page_kinds,
                limit,
                cursor,
                max_chars,
            )
            .await
    }

    pub async fn read_pages(&self, request: ReadPagesRequest) -> Result<Vec<ReadPage>> {
        self.store.read_pages(&self.access, request).await
    }

    pub async fn current_revision_id(&self, page_id: String) -> Result<String> {
        self.store.current_revision_id(&self.access, page_id).await
    }

    pub async fn page_count(&self, requested_scopes: Vec<String>) -> Result<u64> {
        self.store.page_count(&self.access, requested_scopes).await
    }

    pub async fn content_char_count(&self, requested_scopes: Vec<String>) -> Result<usize> {
        self.store
            .content_char_count(&self.access, requested_scopes)
            .await
    }

    pub async fn write_page(&self, request: WritePageRequest) -> Result<WriteResult> {
        self.store.write_page(&self.access, request).await
    }

    pub async fn revise_page(&self, request: RevisePageRequest) -> Result<WriteResult> {
        self.store.revise_page(&self.access, request).await
    }

    pub async fn link_pages(&self, request: LinkPagesRequest) -> Result<Relation> {
        self.store.link_pages(&self.access, request).await
    }

    pub async fn write_summary(&self, request: WriteSummaryRequest) -> Result<WriteSummaryResult> {
        self.store.write_summary(&self.access, request).await
    }

    pub async fn next_summary_candidate(
        &self,
        minimum_chars: usize,
        excluded_page_kinds: Vec<String>,
    ) -> Result<Option<String>> {
        self.store
            .next_summary_candidate(&self.access, minimum_chars, excluded_page_kinds)
            .await
    }

    pub async fn mark_summary_assessed(
        &self,
        target_revision_id: String,
        outcome: String,
        tool_or_model: Option<String>,
    ) -> Result<()> {
        self.store
            .mark_summary_assessed(&self.access, target_revision_id, outcome, tool_or_model)
            .await
    }

    pub async fn assess_page_validity(
        &self,
        request: AssessPageValidityRequest,
    ) -> Result<WriteValidityResult> {
        self.store.assess_page_validity(&self.access, request).await
    }

    pub async fn tombstone_derivation_cascade(
        &self,
        root_revision_id: String,
        actor: pcp_core::Actor,
    ) -> Result<TombstoneCascadeResult> {
        self.store
            .tombstone_derivation_cascade(&self.access, root_revision_id, actor)
            .await
    }

    pub async fn durable_page_inventory(
        &self,
        excluded_page_kinds: Vec<String>,
    ) -> Result<Vec<DurablePageInventoryItem>> {
        self.store
            .durable_page_inventory(&self.access, excluded_page_kinds)
            .await
    }

    pub async fn access_log(
        &self,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<(Vec<AccessAuditEvent>, Option<String>)> {
        self.store.access_log(&self.access, limit, cursor).await
    }
}
