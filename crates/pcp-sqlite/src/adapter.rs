use anyhow::Result;
use async_trait::async_trait;
use pcp_core::{
    AssessPageValidityRequest, Capabilities, CreateScopeRequest, LinkPagesRequest, ReadPage,
    ReadPagesRequest, Relation, RevisePageRequest, Scope, SearchPagesRequest, SearchResult,
    WritePageRequest, WriteResult, WriteSummaryRequest, WriteSummaryResult, WriteValidityResult,
};
use pcp_store::{DurablePageInventoryItem, PcpStore, TombstoneCascadeResult};

use crate::SqlitePcpStore;

#[async_trait]
impl PcpStore for SqlitePcpStore {
    fn owner_id(&self) -> &str {
        SqlitePcpStore::owner_id(self)
    }

    fn capabilities(&self) -> Capabilities {
        SqlitePcpStore::capabilities(self)
    }

    async fn integrity_check(&self) -> Result<String> {
        SqlitePcpStore::integrity_check(self).await
    }

    async fn local_scope_names(&self) -> Result<Vec<String>> {
        SqlitePcpStore::local_scope_names(self).await
    }

    async fn create_scope(&self, request: CreateScopeRequest) -> Result<()> {
        SqlitePcpStore::create_scope(self, request).await
    }

    async fn list_scopes(
        &self,
        allowed_scopes: Vec<String>,
        query: Option<String>,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<(Vec<Scope>, Option<String>)> {
        SqlitePcpStore::list_scopes(self, allowed_scopes, query, limit, cursor).await
    }

    async fn search_pages(&self, request: SearchPagesRequest) -> Result<SearchResult> {
        SqlitePcpStore::search_pages(self, request).await
    }

    async fn browse_index(
        &self,
        scopes: Vec<String>,
        excluded_page_kinds: Vec<String>,
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
    ) -> Result<SearchResult> {
        SqlitePcpStore::browse_index(self, scopes, excluded_page_kinds, limit, cursor, max_chars)
            .await
    }

    async fn read_pages(
        &self,
        request: ReadPagesRequest,
        allowed_scopes: Vec<String>,
    ) -> Result<Vec<ReadPage>> {
        SqlitePcpStore::read_pages(self, request, allowed_scopes).await
    }

    async fn current_revision_id(
        &self,
        page_id: String,
        allowed_scopes: Vec<String>,
    ) -> Result<String> {
        SqlitePcpStore::current_revision_id(self, page_id, allowed_scopes).await
    }

    async fn page_count(&self, allowed_scopes: Vec<String>) -> Result<u64> {
        SqlitePcpStore::page_count(self, allowed_scopes).await
    }

    async fn content_char_count(&self, allowed_scopes: Vec<String>) -> Result<usize> {
        SqlitePcpStore::content_char_count(self, allowed_scopes).await
    }

    async fn write_page(
        &self,
        request: WritePageRequest,
        allowed_scopes: Vec<String>,
    ) -> Result<WriteResult> {
        SqlitePcpStore::write_page(self, request, allowed_scopes).await
    }

    async fn revise_page(
        &self,
        request: RevisePageRequest,
        allowed_scopes: Vec<String>,
    ) -> Result<WriteResult> {
        SqlitePcpStore::revise_page(self, request, allowed_scopes).await
    }

    async fn link_pages(
        &self,
        request: LinkPagesRequest,
        allowed_scopes: Vec<String>,
    ) -> Result<Relation> {
        SqlitePcpStore::link_pages(self, request, allowed_scopes).await
    }

    async fn write_summary(
        &self,
        request: WriteSummaryRequest,
        allowed_scopes: Vec<String>,
    ) -> Result<WriteSummaryResult> {
        SqlitePcpStore::write_summary(self, request, allowed_scopes).await
    }

    async fn next_summary_candidate(
        &self,
        allowed_scopes: Vec<String>,
        minimum_chars: usize,
        excluded_page_kinds: Vec<String>,
    ) -> Result<Option<String>> {
        SqlitePcpStore::next_summary_candidate(
            self,
            allowed_scopes,
            minimum_chars,
            excluded_page_kinds,
        )
        .await
    }

    async fn mark_summary_assessed(
        &self,
        target_revision_id: String,
        outcome: String,
        tool_or_model: Option<String>,
        allowed_scopes: Vec<String>,
    ) -> Result<()> {
        SqlitePcpStore::mark_summary_assessed(
            self,
            target_revision_id,
            outcome,
            tool_or_model,
            allowed_scopes,
        )
        .await
    }

    async fn assess_page_validity(
        &self,
        request: AssessPageValidityRequest,
        allowed_scopes: Vec<String>,
    ) -> Result<WriteValidityResult> {
        SqlitePcpStore::assess_page_validity(self, request, allowed_scopes).await
    }

    async fn tombstone_derivation_cascade(
        &self,
        root_revision_id: String,
        actor: pcp_core::Actor,
        allowed_scopes: Vec<String>,
    ) -> Result<TombstoneCascadeResult> {
        SqlitePcpStore::tombstone_derivation_cascade(self, root_revision_id, actor, allowed_scopes)
            .await
    }

    async fn durable_page_inventory(
        &self,
        allowed_scopes: Vec<String>,
        excluded_page_kinds: Vec<String>,
    ) -> Result<Vec<DurablePageInventoryItem>> {
        SqlitePcpStore::durable_page_inventory(self, allowed_scopes, excluded_page_kinds).await
    }
}
