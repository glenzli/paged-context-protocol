use std::str::FromStr;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use pcp_core::{
    AccessAuditEvent, AccessPermission, AccessPrincipal, AccessSession, AssessPageValidityRequest,
    Capabilities, CollectRevisionRetentionRequest, ConsolidatePagesRequest, CreateScopeRequest,
    LinkPagesRequest, PlanRevisionRetentionRequest, PutRevisionRetentionLeaseRequest, ReadPage,
    ReadPagesRequest, Relation, RevisePageRequest, RevisionCollectionResult,
    RevisionRetentionLease, RevisionRetentionPlan, Scope, ScopeGrant, SearchPagesRequest,
    SearchResult, WritePageRequest, WriteResult, WriteSummaryRequest, WriteSummaryResult,
    WriteValidityResult,
};
use pcp_store::PcpStore;
pub use pcp_store::{DurablePageInventoryItem, HealthSnapshot, TombstoneCascadeResult};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessMode {
    Read,
    Audit,
    Write,
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

    fn permissions(self, allow_cross_scope_derivation: bool) -> Vec<AccessPermission> {
        let mut permissions = vec![
            AccessPermission::ListScopes,
            AccessPermission::Search,
            AccessPermission::ReadSummary,
            AccessPermission::ReadDetail,
        ];
        if matches!(self, Self::Write | Self::Admin) {
            permissions.extend([
                AccessPermission::Write,
                AccessPermission::Revise,
                AccessPermission::Summarize,
                AccessPermission::Link,
                AccessPermission::Assess,
            ]);
        }
        if matches!(self, Self::Audit | Self::Admin) {
            permissions.push(AccessPermission::Audit);
        }
        if self == Self::Admin {
            permissions.extend([
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
            "read" => Ok(Self::Read),
            "audit" => Ok(Self::Audit),
            "write" => Ok(Self::Write),
            "admin" => Ok(Self::Admin),
            other => anyhow::bail!("unsupported PCP access mode: {other}"),
        }
    }
}

/// Transport-independent capabilities exposed to a PCP consumer.
///
/// Implementations may bind an in-process Store or a server-attested remote
/// session. Callers must not depend on the deployment form.
#[async_trait]
pub trait PcpApi: Send + Sync {
    fn owner_id(&self) -> &str;
    fn capabilities(&self) -> Capabilities;
    fn access(&self) -> &AccessSession;

    async fn integrity_check(&self) -> Result<String>;
    async fn create_scope(&self, request: CreateScopeRequest) -> Result<()>;
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
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
    ) -> Result<SearchResult>;
    async fn read_pages(&self, request: ReadPagesRequest) -> Result<Vec<ReadPage>>;
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
    async fn consolidate_pages(&self, request: ConsolidatePagesRequest) -> Result<WriteResult>;
    async fn link_pages(&self, request: LinkPagesRequest) -> Result<Relation>;
    async fn write_summary(&self, request: WriteSummaryRequest) -> Result<WriteSummaryResult>;
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
}

#[derive(Clone)]
pub struct EmbeddedPcpClient {
    store: Arc<dyn PcpStore>,
    access: AccessSession,
}

impl EmbeddedPcpClient {
    pub fn new(store: Arc<dyn PcpStore>, access: AccessSession) -> Self {
        Self { store, access }
    }

    pub fn shared(store: Arc<dyn PcpStore>, access: AccessSession) -> Arc<dyn PcpApi> {
        Arc::new(Self::new(store, access))
    }
}

#[async_trait]
impl PcpApi for EmbeddedPcpClient {
    fn owner_id(&self) -> &str {
        self.store.owner_id()
    }

    fn capabilities(&self) -> Capabilities {
        self.store.capabilities()
    }

    fn access(&self) -> &AccessSession {
        &self.access
    }

    async fn integrity_check(&self) -> Result<String> {
        self.store.integrity_check().await
    }

    async fn create_scope(&self, request: CreateScopeRequest) -> Result<()> {
        self.store.create_scope(&self.access, request).await
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

    async fn read_pages(&self, request: ReadPagesRequest) -> Result<Vec<ReadPage>> {
        self.store.read_pages(&self.access, request).await
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
        self.store.write_page(&self.access, request).await
    }

    async fn revise_page(&self, request: RevisePageRequest) -> Result<WriteResult> {
        self.store.revise_page(&self.access, request).await
    }

    async fn consolidate_pages(&self, request: ConsolidatePagesRequest) -> Result<WriteResult> {
        self.store.consolidate_pages(&self.access, request).await
    }

    async fn link_pages(&self, request: LinkPagesRequest) -> Result<Relation> {
        self.store.link_pages(&self.access, request).await
    }

    async fn write_summary(&self, request: WriteSummaryRequest) -> Result<WriteSummaryResult> {
        self.store.write_summary(&self.access, request).await
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
        self.store.assess_page_validity(&self.access, request).await
    }

    async fn tombstone_derivation_cascade(
        &self,
        root_revision_id: String,
        actor: pcp_core::Actor,
    ) -> Result<TombstoneCascadeResult> {
        self.store
            .tombstone_derivation_cascade(&self.access, root_revision_id, actor)
            .await
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
}
