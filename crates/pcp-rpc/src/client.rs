use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use async_trait::async_trait;
use pcp_client::{
    DurablePageInventoryItem, HealthSnapshot, PcpApi, PcpTenantApi, TombstoneCascadeResult,
    UnpackPageResult,
};
use pcp_core::{
    AccessAuditEvent, AccessSession, AssessPageValidityRequest, Capabilities,
    CollectRevisionRetentionRequest, CreateScopeRequest, IngestPageRequest, LinkPagesRequest,
    PackPagesRequest, PlanRevisionRetentionRequest, PutRevisionRetentionLeaseRequest, ReadPage,
    ReadPagesRequest, Relation, RevisePageRequest, RevisionCollectionResult,
    RevisionRetentionLease, RevisionRetentionPlan, Scope, SearchPagesRequest, SearchResult,
    UnpackPageRequest, WritePageRequest, WriteResult, WriteSummaryRequest, WriteSummaryResult,
    WriteValidityResult,
};
use tokio::net::UnixStream;

use crate::wire::{
    PcpDescriptor, RpcOperation, RpcOutcome, RpcRequest, RpcResponse, RpcValue, read_frame,
    write_frame,
};

const RPC_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct RemotePcpClient {
    socket_path: Arc<PathBuf>,
    descriptor: PcpDescriptor,
    next_request_id: Arc<AtomicU64>,
}

impl RemotePcpClient {
    pub async fn connect(socket_path: impl AsRef<Path>) -> Result<Self> {
        let socket_path = Arc::new(socket_path.as_ref().to_path_buf());
        let descriptor = match request_at(&socket_path, 1, RpcOperation::Describe).await? {
            RpcValue::Descriptor(descriptor) => descriptor,
            _ => anyhow::bail!("PCP runtime returned an unexpected describe response"),
        };
        anyhow::ensure!(
            !descriptor.identity_id.trim().is_empty(),
            "PCP runtime descriptor has no identityId"
        );
        Ok(Self {
            socket_path,
            descriptor,
            next_request_id: Arc::new(AtomicU64::new(2)),
        })
    }

    pub async fn connect_expected(
        socket_path: impl AsRef<Path>,
        expected_principal_id: &str,
    ) -> Result<Self> {
        let client = Self::connect(socket_path).await?;
        anyhow::ensure!(
            client.access().principal.principal_id == expected_principal_id,
            "PCP runtime principal mismatch: expected {expected_principal_id}, received {}",
            client.access().principal.principal_id
        );
        Ok(client)
    }

    pub fn server_pid(&self) -> u32 {
        self.descriptor.server_pid
    }

    pub fn server_started_at_unix_ms(&self) -> u64 {
        self.descriptor.server_started_at_unix_ms
    }

    async fn request(&self, operation: RpcOperation) -> Result<RpcValue> {
        let id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        request_at(&self.socket_path, id, operation).await
    }
}

async fn request_at(socket_path: &Path, id: u64, operation: RpcOperation) -> Result<RpcValue> {
    tokio::time::timeout(RPC_TIMEOUT, exchange(socket_path, id, operation))
        .await
        .with_context(|| format!("PCP runtime request timed out at {}", socket_path.display()))?
}

async fn exchange(socket_path: &Path, id: u64, operation: RpcOperation) -> Result<RpcValue> {
    let mut stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| format!("connect PCP runtime {}", socket_path.display()))?;
    write_frame(&mut stream, &RpcRequest { id, operation }).await?;
    let response = read_frame::<RpcResponse>(&mut stream)
        .await?
        .context("PCP runtime closed before responding")?;
    anyhow::ensure!(response.id == id, "PCP runtime response id mismatch");
    match response.outcome {
        RpcOutcome::Ok(value) => Ok(*value),
        RpcOutcome::Error { message } => anyhow::bail!("PCP runtime: {message}"),
    }
}

fn unexpected(operation: &str) -> anyhow::Error {
    anyhow::anyhow!("PCP runtime returned an unexpected response for {operation}")
}

#[async_trait]
impl PcpTenantApi for RemotePcpClient {
    fn identity_id(&self) -> &str {
        &self.descriptor.identity_id
    }

    fn capabilities(&self) -> Capabilities {
        self.descriptor.capabilities.clone()
    }

    fn access(&self) -> &AccessSession {
        &self.descriptor.access
    }

    async fn list_scopes(
        &self,
        requested_scopes: Vec<String>,
        query: Option<String>,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<(Vec<Scope>, Option<String>)> {
        match self
            .request(RpcOperation::ListScopes {
                requested_scopes,
                query,
                limit,
                cursor,
            })
            .await?
        {
            RpcValue::Scopes {
                scopes,
                next_cursor,
            } => Ok((scopes, next_cursor)),
            _ => Err(unexpected("list_scopes")),
        }
    }

    async fn search_pages(&self, request: SearchPagesRequest) -> Result<SearchResult> {
        match self.request(RpcOperation::SearchPages(request)).await? {
            RpcValue::SearchResult(value) => Ok(value),
            _ => Err(unexpected("search_pages")),
        }
    }

    async fn browse_index(
        &self,
        scopes: Vec<String>,
        excluded_page_kinds: Vec<String>,
        order: pcp_core::BrowseIndexOrder,
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
    ) -> Result<SearchResult> {
        match self
            .request(RpcOperation::BrowseIndex {
                scopes,
                excluded_page_kinds,
                order,
                limit,
                cursor,
                max_chars,
            })
            .await?
        {
            RpcValue::SearchResult(value) => Ok(value),
            _ => Err(unexpected("browse_index")),
        }
    }

    async fn browse_content_pages(
        &self,
        scopes: Vec<String>,
        query: Option<String>,
        order: pcp_core::BrowseIndexOrder,
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
    ) -> Result<pcp_client::ContentLibraryResult> {
        match self
            .request(RpcOperation::BrowseContentPages {
                scopes,
                query,
                order,
                limit,
                cursor,
                max_chars,
            })
            .await?
        {
            RpcValue::ContentLibraryResult(value) => Ok(value),
            _ => Err(unexpected("browse_content_pages")),
        }
    }

    async fn content_library_summary(
        &self,
        requested_scopes: Vec<String>,
    ) -> Result<pcp_client::ContentLibrarySummary> {
        match self
            .request(RpcOperation::ContentLibrarySummary { requested_scopes })
            .await?
        {
            RpcValue::ContentLibrarySummary(value) => Ok(value),
            _ => Err(unexpected("content_library_summary")),
        }
    }

    async fn read_pages(&self, request: ReadPagesRequest) -> Result<Vec<ReadPage>> {
        match self.request(RpcOperation::ReadPages(request)).await? {
            RpcValue::Pages(value) => Ok(value),
            _ => Err(unexpected("read_pages")),
        }
    }

    async fn ingest_page(&self, request: IngestPageRequest) -> Result<WriteResult> {
        match self.request(RpcOperation::IngestPage(request)).await? {
            RpcValue::WriteResult(value) => Ok(value),
            _ => Err(unexpected("ingest_page")),
        }
    }
}

#[async_trait]
impl PcpApi for RemotePcpClient {
    async fn integrity_check(&self) -> Result<String> {
        match self.request(RpcOperation::IntegrityCheck).await? {
            RpcValue::Integrity(value) => Ok(value),
            _ => Err(unexpected("integrity_check")),
        }
    }

    async fn create_scope(&self, request: CreateScopeRequest) -> Result<()> {
        match self.request(RpcOperation::CreateScope(request)).await? {
            RpcValue::Unit => Ok(()),
            _ => Err(unexpected("create_scope")),
        }
    }

    async fn current_revision_id(&self, page_id: String) -> Result<String> {
        match self
            .request(RpcOperation::CurrentRevisionId { page_id })
            .await?
        {
            RpcValue::RevisionId(value) => Ok(value),
            _ => Err(unexpected("current_revision_id")),
        }
    }

    async fn page_count(&self, requested_scopes: Vec<String>) -> Result<u64> {
        match self
            .request(RpcOperation::PageCount { requested_scopes })
            .await?
        {
            RpcValue::PageCount(value) => Ok(value),
            _ => Err(unexpected("page_count")),
        }
    }

    async fn content_char_count(&self, requested_scopes: Vec<String>) -> Result<usize> {
        match self
            .request(RpcOperation::ContentCharCount { requested_scopes })
            .await?
        {
            RpcValue::ContentCharCount(value) => {
                usize::try_from(value).context("PCP content character count exceeds usize")
            }
            _ => Err(unexpected("content_char_count")),
        }
    }

    async fn plan_revision_retention(
        &self,
        request: PlanRevisionRetentionRequest,
    ) -> Result<RevisionRetentionPlan> {
        match self
            .request(RpcOperation::PlanRevisionRetention(request))
            .await?
        {
            RpcValue::RevisionRetentionPlan(value) => Ok(value),
            _ => Err(unexpected("plan_revision_retention")),
        }
    }

    async fn collect_revision_retention(
        &self,
        request: CollectRevisionRetentionRequest,
    ) -> Result<RevisionCollectionResult> {
        match self
            .request(RpcOperation::CollectRevisionRetention(request))
            .await?
        {
            RpcValue::RevisionCollectionResult(value) => Ok(value),
            _ => Err(unexpected("collect_revision_retention")),
        }
    }

    async fn put_revision_retention_lease(
        &self,
        request: PutRevisionRetentionLeaseRequest,
    ) -> Result<RevisionRetentionLease> {
        match self
            .request(RpcOperation::PutRevisionRetentionLease(request))
            .await?
        {
            RpcValue::RevisionRetentionLease(value) => Ok(value),
            _ => Err(unexpected("put_revision_retention_lease")),
        }
    }

    async fn active_revision_retention_leases(
        &self,
        requested_scopes: Vec<String>,
        limit: u32,
    ) -> Result<Vec<RevisionRetentionLease>> {
        match self
            .request(RpcOperation::ActiveRevisionRetentionLeases {
                requested_scopes,
                limit,
            })
            .await?
        {
            RpcValue::RevisionRetentionLeases(value) => Ok(value),
            _ => Err(unexpected("active_revision_retention_leases")),
        }
    }

    async fn write_page(&self, request: WritePageRequest) -> Result<WriteResult> {
        match self.request(RpcOperation::WritePage(request)).await? {
            RpcValue::WriteResult(value) => Ok(value),
            _ => Err(unexpected("write_page")),
        }
    }

    async fn revise_page(&self, request: RevisePageRequest) -> Result<WriteResult> {
        match self.request(RpcOperation::RevisePage(request)).await? {
            RpcValue::WriteResult(value) => Ok(value),
            _ => Err(unexpected("revise_page")),
        }
    }

    async fn pack_pages(&self, request: PackPagesRequest) -> Result<WriteResult> {
        match self.request(RpcOperation::PackPages(request)).await? {
            RpcValue::WriteResult(value) => Ok(value),
            _ => Err(unexpected("pack_pages")),
        }
    }

    async fn unpack_page(&self, request: UnpackPageRequest) -> Result<UnpackPageResult> {
        match self.request(RpcOperation::UnpackPage(request)).await? {
            RpcValue::UnpackPageResult(value) => Ok(value),
            _ => Err(unexpected("unpack_page")),
        }
    }

    async fn link_pages(&self, request: LinkPagesRequest) -> Result<Relation> {
        match self.request(RpcOperation::LinkPages(request)).await? {
            RpcValue::Relation(value) => Ok(value),
            _ => Err(unexpected("link_pages")),
        }
    }

    async fn write_summary(&self, request: WriteSummaryRequest) -> Result<WriteSummaryResult> {
        match self.request(RpcOperation::WriteSummary(request)).await? {
            RpcValue::SummaryResult(value) => Ok(value),
            _ => Err(unexpected("write_summary")),
        }
    }

    async fn next_summary_candidate(
        &self,
        minimum_chars: usize,
        excluded_page_kinds: Vec<String>,
    ) -> Result<Option<String>> {
        match self
            .request(RpcOperation::NextSummaryCandidate {
                minimum_chars,
                excluded_page_kinds,
            })
            .await?
        {
            RpcValue::SummaryCandidate(value) => Ok(value),
            _ => Err(unexpected("next_summary_candidate")),
        }
    }

    async fn mark_summary_assessed(
        &self,
        target_revision_id: String,
        outcome: String,
        tool_or_model: Option<String>,
    ) -> Result<()> {
        match self
            .request(RpcOperation::MarkSummaryAssessed {
                target_revision_id,
                outcome,
                tool_or_model,
            })
            .await?
        {
            RpcValue::Unit => Ok(()),
            _ => Err(unexpected("mark_summary_assessed")),
        }
    }

    async fn assess_page_validity(
        &self,
        request: AssessPageValidityRequest,
    ) -> Result<WriteValidityResult> {
        match self
            .request(RpcOperation::AssessPageValidity(request))
            .await?
        {
            RpcValue::ValidityResult(value) => Ok(value),
            _ => Err(unexpected("assess_page_validity")),
        }
    }

    async fn tombstone_derivation_cascade(
        &self,
        root_revision_id: String,
        actor: pcp_core::Actor,
    ) -> Result<TombstoneCascadeResult> {
        match self
            .request(RpcOperation::TombstoneDerivationCascade {
                root_revision_id,
                actor,
            })
            .await?
        {
            RpcValue::TombstoneCascade(value) => Ok(value),
            _ => Err(unexpected("tombstone_derivation_cascade")),
        }
    }

    async fn durable_page_inventory(
        &self,
        excluded_page_kinds: Vec<String>,
    ) -> Result<Vec<DurablePageInventoryItem>> {
        match self
            .request(RpcOperation::DurablePageInventory {
                excluded_page_kinds,
            })
            .await?
        {
            RpcValue::Inventory(value) => Ok(value),
            _ => Err(unexpected("durable_page_inventory")),
        }
    }

    async fn access_log(
        &self,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<(Vec<AccessAuditEvent>, Option<String>)> {
        match self
            .request(RpcOperation::AccessLog { limit, cursor })
            .await?
        {
            RpcValue::AccessLog {
                events,
                next_cursor,
            } => Ok((events, next_cursor)),
            _ => Err(unexpected("access_log")),
        }
    }

    async fn health_snapshot(
        &self,
        requested_scopes: Vec<String>,
        window_hours: u32,
    ) -> Result<HealthSnapshot> {
        match self
            .request(RpcOperation::HealthSnapshot {
                requested_scopes,
                window_hours,
            })
            .await?
        {
            RpcValue::HealthSnapshot(value) => Ok(value),
            _ => Err(unexpected("health_snapshot")),
        }
    }
}
