use std::str::FromStr;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use pcp_core::{
    AccessAuditEvent, AccessPermission, AccessPrincipal, AccessSession, AssessPageValidityRequest,
    Capabilities, CollectRevisionRetentionRequest, CreateScopeRequest, IngestPageRequest,
    LinkPagesRequest, PackPagesRequest, PlanRevisionRetentionRequest,
    PutRevisionRetentionLeaseRequest, ReadPage, ReadPagesRequest, Relation, RevisePageRequest,
    RevisionCollectionResult, RevisionRetentionLease, RevisionRetentionPlan, Scope, ScopeGrant,
    SearchPagesRequest, SearchResult, WritePageRequest, WriteResult, WriteSummaryRequest,
    WriteSummaryResult, WriteValidityResult,
};
use pcp_store::PcpStore;
pub use pcp_store::{DurablePageInventoryItem, HealthSnapshot, TombstoneCascadeResult};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessMode {
    Observe,
    Read,
    Audit,
    Contribute,
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
            "observe" => Ok(Self::Observe),
            "read" => Ok(Self::Read),
            "audit" => Ok(Self::Audit),
            "contribute" => Ok(Self::Contribute),
            "write" => Ok(Self::Write),
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
    async fn ingest_page(&self, request: IngestPageRequest) -> Result<WriteResult>;
}

/// Privileged Runtime, maintainer, and operator surface.
///
/// Ordinary tenants should be typed against [`PcpTenantApi`]. This superset
/// retains maintenance operations needed by Runtime-owned policy and local
/// administrative tools.
#[async_trait]
pub trait PcpApi: PcpTenantApi {
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
    async fn pack_pages(&self, request: PackPagesRequest) -> Result<WriteResult>;
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

    pub fn tenant_shared(store: Arc<dyn PcpStore>, access: AccessSession) -> Arc<dyn PcpTenantApi> {
        Arc::new(Self::new(store, access))
    }
}

#[async_trait]
impl PcpTenantApi for EmbeddedPcpClient {
    fn identity_id(&self) -> &str {
        self.store.identity_id()
    }

    fn capabilities(&self) -> Capabilities {
        self.store.capabilities()
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

    async fn ingest_page(&self, request: IngestPageRequest) -> Result<WriteResult> {
        self.store.ingest_page(&self.access, request).await
    }
}

#[async_trait]
impl PcpApi for EmbeddedPcpClient {
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
        self.store.write_page(&self.access, request).await
    }

    async fn revise_page(&self, request: RevisePageRequest) -> Result<WriteResult> {
        self.store.revise_page(&self.access, request).await
    }

    async fn pack_pages(&self, request: PackPagesRequest) -> Result<WriteResult> {
        self.store.pack_pages(&self.access, request).await
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
}
