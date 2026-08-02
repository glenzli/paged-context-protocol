use anyhow::Result;
use async_trait::async_trait;
use pcp_core::{
    AssessPageValidityRequest, Capabilities, CreateScopeRequest, LinkPagesRequest, ReadPage,
    ReadPagesRequest, Relation, RevisePageRequest, Scope, SearchPagesRequest, SearchResult,
    WritePageRequest, WriteResult, WriteSummaryRequest, WriteSummaryResult, WriteValidityResult,
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
    async fn create_scope(&self, request: CreateScopeRequest) -> Result<()>;
    async fn list_scopes(
        &self,
        allowed_scopes: Vec<String>,
        query: Option<String>,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<(Vec<Scope>, Option<String>)>;
    async fn search_pages(&self, request: SearchPagesRequest) -> Result<SearchResult>;
    async fn browse_index(
        &self,
        scopes: Vec<String>,
        excluded_page_kinds: Vec<String>,
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
    ) -> Result<SearchResult>;
    async fn read_pages(
        &self,
        request: ReadPagesRequest,
        allowed_scopes: Vec<String>,
    ) -> Result<Vec<ReadPage>>;
    async fn current_revision_id(
        &self,
        page_id: String,
        allowed_scopes: Vec<String>,
    ) -> Result<String>;
    async fn page_count(&self, allowed_scopes: Vec<String>) -> Result<u64>;
    async fn content_char_count(&self, allowed_scopes: Vec<String>) -> Result<usize>;
    async fn write_page(
        &self,
        request: WritePageRequest,
        allowed_scopes: Vec<String>,
    ) -> Result<WriteResult>;
    async fn revise_page(
        &self,
        request: RevisePageRequest,
        allowed_scopes: Vec<String>,
    ) -> Result<WriteResult>;
    async fn link_pages(
        &self,
        request: LinkPagesRequest,
        allowed_scopes: Vec<String>,
    ) -> Result<Relation>;
    async fn write_summary(
        &self,
        request: WriteSummaryRequest,
        allowed_scopes: Vec<String>,
    ) -> Result<WriteSummaryResult>;
    async fn next_summary_candidate(
        &self,
        allowed_scopes: Vec<String>,
        minimum_chars: usize,
        excluded_page_kinds: Vec<String>,
    ) -> Result<Option<String>>;
    async fn mark_summary_assessed(
        &self,
        target_revision_id: String,
        outcome: String,
        tool_or_model: Option<String>,
        allowed_scopes: Vec<String>,
    ) -> Result<()>;
    async fn assess_page_validity(
        &self,
        request: AssessPageValidityRequest,
        allowed_scopes: Vec<String>,
    ) -> Result<WriteValidityResult>;
    async fn tombstone_derivation_cascade(
        &self,
        root_revision_id: String,
        actor: pcp_core::Actor,
        allowed_scopes: Vec<String>,
    ) -> Result<TombstoneCascadeResult>;
    async fn durable_page_inventory(
        &self,
        allowed_scopes: Vec<String>,
        excluded_page_kinds: Vec<String>,
    ) -> Result<Vec<DurablePageInventoryItem>>;
}
