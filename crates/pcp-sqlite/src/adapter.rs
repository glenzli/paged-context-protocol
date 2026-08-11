use std::time::Instant;

use anyhow::{Context, Result};
use async_trait::async_trait;
use pcp_core::{
    AccessAuditEvent, AccessDecision, AccessPermission, AccessSession, AssessPageValidityRequest,
    Capabilities, CollectRevisionRetentionRequest, ConsolidatePagesRequest, CreateScopeRequest,
    LinkPagesRequest, OperationTelemetry, PlanRevisionRetentionRequest, Projection,
    ProvenanceEvent, PutRevisionRetentionLeaseRequest, ReadPage, ReadPagesRequest, Relation,
    RevisePageRequest, RevisionCollectionResult, RevisionRetentionLease, RevisionRetentionPlan,
    Scope, SearchPagesRequest, SearchResult, WritePageRequest, WriteResult, WriteSummaryRequest,
    WriteSummaryResult, WriteValidityResult,
};
use pcp_store::{DurablePageInventoryItem, HealthSnapshot, PcpStore, TombstoneCascadeResult};
use serde::Serialize;
use serde_json::Value;

use crate::{
    SqlitePcpStore,
    access::{authorize_exact, authorize_scopes, authorize_scopes_any},
};

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

    async fn create_scope(
        &self,
        access: &AccessSession,
        request: CreateScopeRequest,
    ) -> Result<()> {
        let observation = OperationObservation::start();
        let mut scopes = vec![request.namespace.clone()];
        let authorization = (request.owner_id == self.owner_id())
            .then_some(())
            .ok_or_else(|| anyhow::anyhow!("ownerId does not match this PCP Store"))
            .and_then(|_| {
                authorize_exact(access, &request.namespace, AccessPermission::ManageScope)
            })
            .and_then(|_| {
                if let Some(parent) = request.parent_namespace.as_ref() {
                    authorize_exact(access, parent, AccessPermission::ManageScope)?;
                    scopes.push(parent.clone());
                }
                Ok(())
            });
        if let Err(error) = authorization {
            return complete(
                self,
                access,
                "create_scope",
                scopes,
                Err(error),
                true,
                observation,
            )
            .await;
        }
        let result = SqlitePcpStore::create_scope(self, request).await;
        complete(
            self,
            access,
            "create_scope",
            scopes,
            result,
            false,
            observation,
        )
        .await
    }

    async fn list_scopes(
        &self,
        access: &AccessSession,
        requested_scopes: Vec<String>,
        query: Option<String>,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<(Vec<Scope>, Option<String>)> {
        let observation = OperationObservation::start();
        let scopes =
            match authorize_scopes(access, &[AccessPermission::ListScopes], &requested_scopes) {
                Ok(scopes) => scopes,
                Err(error) => {
                    return complete(
                        self,
                        access,
                        "list_scopes",
                        requested_scopes,
                        Err(error),
                        true,
                        observation,
                    )
                    .await;
                }
            };
        let result = SqlitePcpStore::list_scopes(self, scopes.clone(), query, limit, cursor).await;
        complete(
            self,
            access,
            "list_scopes",
            scopes,
            result,
            false,
            observation,
        )
        .await
    }

    async fn search_pages(
        &self,
        access: &AccessSession,
        mut request: SearchPagesRequest,
    ) -> Result<SearchResult> {
        if request.projections.is_empty() {
            request.projections = pcp_core::default_search_projections();
        }
        let observation = OperationObservation::start()
            .with_input_count(1)
            .with_projections(&request.projections);
        let permissions = search_permissions(&request.projections);
        let requested = request.scopes.clone();
        let scopes = match authorize_scopes(access, &permissions, &requested) {
            Ok(scopes) => scopes,
            Err(error) => {
                return complete(
                    self,
                    access,
                    "search_pages",
                    requested,
                    Err(error),
                    true,
                    observation,
                )
                .await;
            }
        };
        request.scopes = scopes.clone();
        let result = SqlitePcpStore::search_pages(self, request).await;
        complete(
            self,
            access,
            "search_pages",
            scopes,
            result,
            false,
            observation,
        )
        .await
    }

    async fn browse_index(
        &self,
        access: &AccessSession,
        requested_scopes: Vec<String>,
        excluded_page_kinds: Vec<String>,
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
    ) -> Result<SearchResult> {
        let observation = OperationObservation::start();
        let scopes = match authorize_scopes(
            access,
            &[
                AccessPermission::Search,
                AccessPermission::ReadSummary,
                AccessPermission::ReadDetail,
            ],
            &requested_scopes,
        ) {
            Ok(scopes) => scopes,
            Err(error) => {
                return complete(
                    self,
                    access,
                    "browse_index",
                    requested_scopes,
                    Err(error),
                    true,
                    observation,
                )
                .await;
            }
        };
        let result = SqlitePcpStore::browse_index(
            self,
            scopes.clone(),
            excluded_page_kinds,
            limit,
            cursor,
            max_chars,
        )
        .await;
        complete(
            self,
            access,
            "browse_index",
            scopes,
            result,
            false,
            observation,
        )
        .await
    }

    async fn read_pages(
        &self,
        access: &AccessSession,
        request: ReadPagesRequest,
    ) -> Result<Vec<ReadPage>> {
        let observation = OperationObservation::start()
            .with_input_count(request.revision_ids.len())
            .with_projections(&request.projections);
        let permission = if request.projections.iter().any(is_detail_projection) {
            AccessPermission::ReadDetail
        } else {
            AccessPermission::ReadSummary
        };
        let scopes = match authorize_scopes(access, &[permission], &[]) {
            Ok(scopes) => scopes,
            Err(error) => {
                return complete(
                    self,
                    access,
                    "read_pages",
                    Vec::new(),
                    Err(error),
                    true,
                    observation,
                )
                .await;
            }
        };
        let result = SqlitePcpStore::read_pages(self, request, scopes.clone()).await;
        complete(
            self,
            access,
            "read_pages",
            scopes,
            result,
            false,
            observation,
        )
        .await
    }

    async fn current_revision_id(&self, access: &AccessSession, page_id: String) -> Result<String> {
        let observation = OperationObservation::start().with_input_count(1);
        let scopes = match authorize_scopes(access, &[AccessPermission::ReadSummary], &[]) {
            Ok(scopes) => scopes,
            Err(error) => {
                return complete(
                    self,
                    access,
                    "current_revision",
                    Vec::new(),
                    Err(error),
                    true,
                    observation,
                )
                .await;
            }
        };
        let result = SqlitePcpStore::current_revision_id(self, page_id, scopes.clone()).await;
        complete(
            self,
            access,
            "current_revision",
            scopes,
            result,
            false,
            observation,
        )
        .await
    }

    async fn page_count(
        &self,
        access: &AccessSession,
        requested_scopes: Vec<String>,
    ) -> Result<u64> {
        let observation = OperationObservation::start();
        let scopes = match authorize_scopes(access, &[AccessPermission::Search], &requested_scopes)
        {
            Ok(scopes) => scopes,
            Err(error) => {
                return complete(
                    self,
                    access,
                    "page_count",
                    Vec::new(),
                    Err(error),
                    true,
                    observation,
                )
                .await;
            }
        };
        let result = SqlitePcpStore::page_count(self, scopes.clone()).await;
        complete(
            self,
            access,
            "page_count",
            scopes,
            result,
            false,
            observation,
        )
        .await
    }

    async fn content_char_count(
        &self,
        access: &AccessSession,
        requested_scopes: Vec<String>,
    ) -> Result<usize> {
        let observation = OperationObservation::start();
        let scopes =
            match authorize_scopes(access, &[AccessPermission::ReadDetail], &requested_scopes) {
                Ok(scopes) => scopes,
                Err(error) => {
                    return complete(
                        self,
                        access,
                        "content_char_count",
                        Vec::new(),
                        Err(error),
                        true,
                        observation,
                    )
                    .await;
                }
            };
        let result = SqlitePcpStore::content_char_count(self, scopes.clone()).await;
        complete(
            self,
            access,
            "content_char_count",
            scopes,
            result,
            false,
            observation,
        )
        .await
    }

    async fn plan_revision_retention(
        &self,
        access: &AccessSession,
        mut request: PlanRevisionRetentionRequest,
    ) -> Result<RevisionRetentionPlan> {
        let observation = OperationObservation::start();
        let requested_scopes = request.scopes.clone();
        let scopes = match authorize_scopes(access, &[AccessPermission::Audit], &requested_scopes) {
            Ok(scopes) => scopes,
            Err(error) => {
                return complete(
                    self,
                    access,
                    "plan_revision_retention",
                    requested_scopes,
                    Err(error),
                    true,
                    observation,
                )
                .await;
            }
        };
        request.scopes = scopes.clone();
        let result = SqlitePcpStore::plan_revision_retention(self, request).await;
        complete(
            self,
            access,
            "plan_revision_retention",
            scopes,
            result,
            false,
            observation,
        )
        .await
    }

    async fn collect_revision_retention(
        &self,
        access: &AccessSession,
        mut request: CollectRevisionRetentionRequest,
    ) -> Result<RevisionCollectionResult> {
        let observation =
            OperationObservation::start().with_input_count(request.revision_ids.len());
        let requested_scopes = request.scopes.clone();
        let scopes = match authorize_scopes(access, &[AccessPermission::Collect], &requested_scopes)
        {
            Ok(scopes) => scopes,
            Err(error) => {
                return complete(
                    self,
                    access,
                    "collect_revision_retention",
                    requested_scopes,
                    Err(error),
                    true,
                    observation,
                )
                .await;
            }
        };
        request.scopes = scopes.clone();
        let result = SqlitePcpStore::collect_revision_retention(
            self,
            access.principal.principal_id.clone(),
            request,
        )
        .await;
        complete(
            self,
            access,
            "collect_revision_retention",
            scopes,
            result,
            false,
            observation,
        )
        .await
    }

    async fn put_revision_retention_lease(
        &self,
        access: &AccessSession,
        request: PutRevisionRetentionLeaseRequest,
    ) -> Result<RevisionRetentionLease> {
        let observation = OperationObservation::start().with_input_count(1);
        let scope = request.namespace.clone();
        if let Err(error) = authorize_exact(access, &scope, AccessPermission::Write) {
            return complete(
                self,
                access,
                "put_revision_retention_lease",
                vec![scope],
                Err(error),
                true,
                observation,
            )
            .await;
        }
        let result = SqlitePcpStore::put_revision_retention_lease(
            self,
            access.principal.principal_id.clone(),
            request,
        )
        .await;
        complete(
            self,
            access,
            "put_revision_retention_lease",
            vec![scope],
            result,
            false,
            observation,
        )
        .await
    }

    async fn active_revision_retention_leases(
        &self,
        access: &AccessSession,
        requested_scopes: Vec<String>,
        limit: u32,
    ) -> Result<Vec<RevisionRetentionLease>> {
        let observation = OperationObservation::start();
        let scopes = match authorize_scopes(access, &[AccessPermission::Audit], &requested_scopes) {
            Ok(scopes) => scopes,
            Err(error) => {
                return complete(
                    self,
                    access,
                    "active_revision_retention_leases",
                    requested_scopes,
                    Err(error),
                    true,
                    observation,
                )
                .await;
            }
        };
        let result =
            SqlitePcpStore::active_revision_retention_leases(self, scopes.clone(), limit).await;
        complete(
            self,
            access,
            "active_revision_retention_leases",
            scopes,
            result,
            false,
            observation,
        )
        .await
    }

    async fn write_page(
        &self,
        access: &AccessSession,
        request: WritePageRequest,
    ) -> Result<WriteResult> {
        let observation = OperationObservation::start().with_input_count(
            request.initial_relations.len()
                + request
                    .provenance
                    .iter()
                    .map(|event| event.input_revision_ids.len())
                    .sum::<usize>(),
        );
        let target_scope = request.namespace.clone();
        let mut audit_scopes = vec![target_scope.clone()];
        let authorization = async {
            if request.owner_id != self.owner_id() {
                anyhow::bail!("ownerId does not match this PCP Store");
            }
            authorize_exact(access, &target_scope, AccessPermission::Write)?;
            let provenance_scopes =
                authorize_provenance(self, access, &target_scope, &request.provenance).await?;
            extend_unique(&mut audit_scopes, provenance_scopes);
            if !request.initial_relations.is_empty() {
                authorize_exact(access, &target_scope, AccessPermission::Link)?;
                let relation_scopes = self
                    .page_namespaces(
                        request
                            .initial_relations
                            .iter()
                            .map(|relation| relation.to_page_id.clone())
                            .collect(),
                    )
                    .await?;
                for scope in &relation_scopes {
                    authorize_exact(access, scope, AccessPermission::Link)?;
                }
                extend_unique(&mut audit_scopes, relation_scopes);
            }
            Ok(())
        }
        .await;
        if let Err(error) = authorization {
            return complete(
                self,
                access,
                "write_page",
                audit_scopes,
                Err(error),
                true,
                observation,
            )
            .await;
        }
        let result = SqlitePcpStore::write_page(self, request, audit_scopes.clone()).await;
        complete(
            self,
            access,
            "write_page",
            audit_scopes,
            result,
            false,
            observation,
        )
        .await
    }

    async fn revise_page(
        &self,
        access: &AccessSession,
        request: RevisePageRequest,
    ) -> Result<WriteResult> {
        let observation = OperationObservation::start().with_input_count(
            1 + request.initial_relations.len()
                + request
                    .provenance
                    .iter()
                    .map(|event| event.input_revision_ids.len())
                    .sum::<usize>(),
        );
        let target_scope = match self.page_namespace(request.page_id.clone()).await {
            Ok(scope) => scope,
            Err(error) => {
                return complete(
                    self,
                    access,
                    "revise_page",
                    Vec::new(),
                    Err(error),
                    false,
                    observation,
                )
                .await;
            }
        };
        let mut audit_scopes = vec![target_scope.clone()];
        let authorization = async {
            authorize_exact(access, &target_scope, AccessPermission::Revise)?;
            let provenance_scopes =
                authorize_provenance(self, access, &target_scope, &request.provenance).await?;
            extend_unique(&mut audit_scopes, provenance_scopes);
            if !request.initial_relations.is_empty() {
                authorize_exact(access, &target_scope, AccessPermission::Link)?;
                let relation_scopes = self
                    .page_namespaces(
                        request
                            .initial_relations
                            .iter()
                            .map(|relation| relation.to_page_id.clone())
                            .collect(),
                    )
                    .await?;
                for scope in &relation_scopes {
                    authorize_exact(access, scope, AccessPermission::Link)?;
                }
                extend_unique(&mut audit_scopes, relation_scopes);
            }
            Ok(())
        }
        .await;
        if let Err(error) = authorization {
            return complete(
                self,
                access,
                "revise_page",
                audit_scopes,
                Err(error),
                true,
                observation,
            )
            .await;
        }
        let result = SqlitePcpStore::revise_page(self, request, audit_scopes.clone()).await;
        complete(
            self,
            access,
            "revise_page",
            audit_scopes,
            result,
            false,
            observation,
        )
        .await
    }

    async fn consolidate_pages(
        &self,
        access: &AccessSession,
        request: ConsolidatePagesRequest,
    ) -> Result<WriteResult> {
        let observation = OperationObservation::start()
            .with_input_count(request.replaced_pages.len().saturating_add(1));
        let mut target_ids = request
            .replaced_pages
            .iter()
            .map(|input| input.page_id.clone())
            .collect::<Vec<_>>();
        target_ids.push(request.canonical_page_id.clone());
        target_ids.sort();
        target_ids.dedup();
        let mut scopes = match self.page_namespaces(target_ids).await {
            Ok(scopes) => scopes,
            Err(error) => {
                return complete(
                    self,
                    access,
                    "consolidate_pages",
                    Vec::new(),
                    Err(error),
                    false,
                    observation,
                )
                .await;
            }
        };
        let authorization = async {
            for scope in &scopes {
                authorize_exact(access, scope, AccessPermission::Write)?;
                authorize_exact(access, scope, AccessPermission::Revise)?;
            }
            let target_scope = scopes
                .first()
                .cloned()
                .context("PCP consolidation has no target Scope")?;
            let provenance_scopes =
                authorize_provenance(self, access, &target_scope, &request.provenance).await?;
            extend_unique(&mut scopes, provenance_scopes);
            Ok(())
        }
        .await;
        if let Err(error) = authorization {
            return complete(
                self,
                access,
                "consolidate_pages",
                scopes,
                Err(error),
                true,
                observation,
            )
            .await;
        }
        let result = SqlitePcpStore::consolidate_pages(self, request, scopes.clone()).await;
        complete(
            self,
            access,
            "consolidate_pages",
            scopes,
            result,
            false,
            observation,
        )
        .await
    }

    async fn link_pages(
        &self,
        access: &AccessSession,
        request: LinkPagesRequest,
    ) -> Result<Relation> {
        let observation = OperationObservation::start().with_input_count(2);
        let scopes = match self
            .page_namespaces(vec![
                request.from_page_id.clone(),
                request.to_page_id.clone(),
            ])
            .await
        {
            Ok(scopes) => scopes,
            Err(error) => {
                return complete(
                    self,
                    access,
                    "link_pages",
                    Vec::new(),
                    Err(error),
                    false,
                    observation,
                )
                .await;
            }
        };
        for scope in &scopes {
            if let Err(error) = authorize_exact(access, scope, AccessPermission::Link) {
                return complete(
                    self,
                    access,
                    "link_pages",
                    scopes,
                    Err(error),
                    true,
                    observation,
                )
                .await;
            }
        }
        let result = SqlitePcpStore::link_pages(self, request, scopes.clone()).await;
        complete(
            self,
            access,
            "link_pages",
            scopes,
            result,
            false,
            observation,
        )
        .await
    }

    async fn write_summary(
        &self,
        access: &AccessSession,
        request: WriteSummaryRequest,
    ) -> Result<WriteSummaryResult> {
        let observation = OperationObservation::start().with_input_count(
            1 + request
                .provenance
                .iter()
                .map(|event| event.input_revision_ids.len())
                .sum::<usize>(),
        );
        let target_scope = match self
            .revision_namespaces(vec![request.target_revision_id.clone()])
            .await
            .and_then(|scopes| {
                scopes
                    .into_iter()
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("PCP target Revision is not available"))
            }) {
            Ok(scope) => scope,
            Err(error) => {
                return complete(
                    self,
                    access,
                    "write_summary",
                    Vec::new(),
                    Err(error),
                    false,
                    observation,
                )
                .await;
            }
        };
        let mut scopes = vec![target_scope.clone()];
        let authorization = async {
            authorize_exact(access, &target_scope, AccessPermission::Summarize)?;
            let source_scopes =
                authorize_provenance(self, access, &target_scope, &request.provenance).await?;
            extend_unique(&mut scopes, source_scopes);
            Ok(())
        }
        .await;
        if let Err(error) = authorization {
            return complete(
                self,
                access,
                "write_summary",
                scopes,
                Err(error),
                true,
                observation,
            )
            .await;
        }
        let result = SqlitePcpStore::write_summary(self, request, scopes.clone()).await;
        complete(
            self,
            access,
            "write_summary",
            scopes,
            result,
            false,
            observation,
        )
        .await
    }

    async fn next_summary_candidate(
        &self,
        access: &AccessSession,
        minimum_chars: usize,
        excluded_page_kinds: Vec<String>,
    ) -> Result<Option<String>> {
        let observation = OperationObservation::start();
        let scopes = match authorize_scopes(
            access,
            &[AccessPermission::Search, AccessPermission::ReadDetail],
            &[],
        ) {
            Ok(scopes) => scopes,
            Err(error) => {
                return complete(
                    self,
                    access,
                    "next_summary_candidate",
                    Vec::new(),
                    Err(error),
                    true,
                    observation,
                )
                .await;
            }
        };
        let result = SqlitePcpStore::next_summary_candidate(
            self,
            scopes.clone(),
            minimum_chars,
            excluded_page_kinds,
        )
        .await;
        complete(
            self,
            access,
            "next_summary_candidate",
            scopes,
            result,
            false,
            observation,
        )
        .await
    }

    async fn mark_summary_assessed(
        &self,
        access: &AccessSession,
        target_revision_id: String,
        outcome: String,
        tool_or_model: Option<String>,
    ) -> Result<()> {
        let observation = OperationObservation::start().with_input_count(1);
        let scopes = match self
            .revision_namespaces(vec![target_revision_id.clone()])
            .await
        {
            Ok(scopes) => scopes,
            Err(error) => {
                return complete(
                    self,
                    access,
                    "mark_summary_assessed",
                    Vec::new(),
                    Err(error),
                    false,
                    observation,
                )
                .await;
            }
        };
        if let Some(scope) = scopes.first()
            && let Err(error) = authorize_exact(access, scope, AccessPermission::Assess)
        {
            return complete(
                self,
                access,
                "mark_summary_assessed",
                scopes,
                Err(error),
                true,
                observation,
            )
            .await;
        }
        let result = SqlitePcpStore::mark_summary_assessed(
            self,
            target_revision_id,
            outcome,
            tool_or_model,
            scopes.clone(),
        )
        .await;
        complete(
            self,
            access,
            "mark_summary_assessed",
            scopes,
            result,
            false,
            observation,
        )
        .await
    }

    async fn assess_page_validity(
        &self,
        access: &AccessSession,
        request: AssessPageValidityRequest,
    ) -> Result<WriteValidityResult> {
        let observation =
            OperationObservation::start().with_input_count(1 + request.basis_revision_ids.len());
        let target_scope = match self
            .revision_namespaces(vec![request.target_revision_id.clone()])
            .await
            .and_then(|scopes| {
                scopes
                    .into_iter()
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("PCP target Revision is not available"))
            }) {
            Ok(scope) => scope,
            Err(error) => {
                return complete(
                    self,
                    access,
                    "assess_validity",
                    Vec::new(),
                    Err(error),
                    false,
                    observation,
                )
                .await;
            }
        };
        let mut scopes = vec![target_scope.clone()];
        let authorization = async {
            authorize_exact(access, &target_scope, AccessPermission::Assess)?;
            let basis_scopes = self
                .revision_namespaces(request.basis_revision_ids.clone())
                .await?;
            for scope in &basis_scopes {
                authorize_exact(access, scope, AccessPermission::ReadDetail)?;
            }
            enforce_cross_scope_derivation(access, &target_scope, &basis_scopes)?;
            extend_unique(&mut scopes, basis_scopes);
            Ok(())
        }
        .await;
        if let Err(error) = authorization {
            return complete(
                self,
                access,
                "assess_validity",
                scopes,
                Err(error),
                true,
                observation,
            )
            .await;
        }
        let result = SqlitePcpStore::assess_page_validity(self, request, scopes.clone()).await;
        complete(
            self,
            access,
            "assess_validity",
            scopes,
            result,
            false,
            observation,
        )
        .await
    }

    async fn tombstone_derivation_cascade(
        &self,
        access: &AccessSession,
        root_revision_id: String,
        actor: pcp_core::Actor,
    ) -> Result<TombstoneCascadeResult> {
        let observation = OperationObservation::start().with_input_count(1);
        let scopes = match authorize_scopes(access, &[AccessPermission::Retract], &[]) {
            Ok(scopes) => scopes,
            Err(error) => {
                return complete(
                    self,
                    access,
                    "retract",
                    Vec::new(),
                    Err(error),
                    true,
                    observation,
                )
                .await;
            }
        };
        let result = SqlitePcpStore::tombstone_derivation_cascade(
            self,
            root_revision_id,
            actor,
            scopes.clone(),
        )
        .await;
        complete(self, access, "retract", scopes, result, false, observation).await
    }

    async fn durable_page_inventory(
        &self,
        access: &AccessSession,
        excluded_page_kinds: Vec<String>,
    ) -> Result<Vec<DurablePageInventoryItem>> {
        let observation = OperationObservation::start();
        let scopes = match authorize_scopes(access, &[AccessPermission::ReadDetail], &[]) {
            Ok(scopes) => scopes,
            Err(error) => {
                return complete(
                    self,
                    access,
                    "durable_inventory",
                    Vec::new(),
                    Err(error),
                    true,
                    observation,
                )
                .await;
            }
        };
        let result =
            SqlitePcpStore::durable_page_inventory(self, scopes.clone(), excluded_page_kinds).await;
        complete(
            self,
            access,
            "durable_inventory",
            scopes,
            result,
            false,
            observation,
        )
        .await
    }

    async fn access_log(
        &self,
        access: &AccessSession,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<(Vec<AccessAuditEvent>, Option<String>)> {
        let scopes = authorize_scopes(access, &[AccessPermission::Audit], &[])?;
        SqlitePcpStore::read_access_log(self, scopes, limit, cursor).await
    }

    async fn health_snapshot(
        &self,
        access: &AccessSession,
        requested_scopes: Vec<String>,
        window_hours: u32,
    ) -> Result<HealthSnapshot> {
        let observation = OperationObservation::start();
        let scopes = match authorize_scopes_any(
            access,
            &[AccessPermission::Observe, AccessPermission::Audit],
            &requested_scopes,
        ) {
            Ok(scopes) => scopes,
            Err(error) => {
                return complete(
                    self,
                    access,
                    "health_snapshot",
                    requested_scopes,
                    Err(error),
                    true,
                    observation,
                )
                .await;
            }
        };
        let result = SqlitePcpStore::health_snapshot(self, scopes.clone(), window_hours).await;
        complete(
            self,
            access,
            "health_snapshot",
            scopes,
            result,
            false,
            observation,
        )
        .await
    }
}

fn search_permissions(projections: &[Projection]) -> Vec<AccessPermission> {
    let mut permissions = vec![AccessPermission::Search];
    if projections.iter().any(is_detail_projection) {
        permissions.push(AccessPermission::ReadDetail);
    } else {
        permissions.push(AccessPermission::ReadSummary);
    }
    permissions
}

fn is_detail_projection(projection: &Projection) -> bool {
    matches!(
        projection,
        Projection::Payload
            | Projection::Sources
            | Projection::Provenance
            | Projection::Relations
            | Projection::Facets
            | Projection::History
    )
}

async fn authorize_provenance(
    store: &SqlitePcpStore,
    access: &AccessSession,
    target_scope: &str,
    provenance: &[ProvenanceEvent],
) -> Result<Vec<String>> {
    let source_scopes = store
        .revision_namespaces(
            provenance
                .iter()
                .flat_map(|event| event.input_revision_ids.iter().cloned())
                .collect(),
        )
        .await?;
    for scope in &source_scopes {
        authorize_exact(access, scope, AccessPermission::ReadDetail)?;
    }
    enforce_cross_scope_derivation(access, target_scope, &source_scopes)?;
    Ok(source_scopes)
}

fn enforce_cross_scope_derivation(
    access: &AccessSession,
    target_scope: &str,
    source_scopes: &[String],
) -> Result<()> {
    if source_scopes.iter().any(|scope| scope != target_scope)
        && !access.allows(target_scope, AccessPermission::DeriveAcrossScopes)
    {
        anyhow::bail!("cross-Scope derivation requires derive_across_scopes on the target Scope");
    }
    Ok(())
}

fn extend_unique(target: &mut Vec<String>, source: Vec<String>) {
    for scope in source {
        if !target.contains(&scope) {
            target.push(scope);
        }
    }
}

async fn complete<T>(
    store: &SqlitePcpStore,
    access: &AccessSession,
    operation: &str,
    scopes: Vec<String>,
    result: Result<T>,
    denied: bool,
    observation: OperationObservation,
) -> Result<T>
where
    T: Serialize,
{
    let decision = if result.is_ok() {
        AccessDecision::Allowed
    } else if denied {
        AccessDecision::Denied
    } else {
        AccessDecision::Failed
    };
    let detail = match decision {
        AccessDecision::Denied => Some("authorization denied"),
        AccessDecision::Failed => Some("operation failed"),
        AccessDecision::Allowed => None,
    };
    let telemetry = observation.finish(result.as_ref().ok());
    let audit = store
        .record_access(
            access,
            operation,
            &scopes,
            &decision,
            detail,
            Some(&telemetry),
        )
        .await;
    if let Err(error) = audit {
        eprintln!("PCP {decision:?} access audit failed for {operation}: {error:#}");
    }
    result
}

struct OperationObservation {
    started: Instant,
    input_count: Option<u64>,
    projections: Vec<String>,
}

impl OperationObservation {
    fn start() -> Self {
        Self {
            started: Instant::now(),
            input_count: None,
            projections: Vec::new(),
        }
    }

    fn with_input_count(mut self, count: usize) -> Self {
        self.input_count = u64::try_from(count).ok();
        self
    }

    fn with_projections(mut self, projections: &[Projection]) -> Self {
        self.projections = projections
            .iter()
            .map(|projection| projection.as_str().to_owned())
            .collect();
        self
    }

    fn finish<T: Serialize>(&self, result: Option<&T>) -> OperationTelemetry {
        let output = result.and_then(|value| serde_json::to_value(value).ok());
        OperationTelemetry {
            duration_ms: self
                .started
                .elapsed()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
            input_count: self.input_count,
            output_count: output.as_ref().and_then(output_count),
            output_bytes: result
                .and_then(|value| serde_json::to_vec(value).ok())
                .and_then(|bytes| u64::try_from(bytes.len()).ok()),
            projections: self.projections.clone(),
        }
    }
}

fn output_count(value: &Value) -> Option<u64> {
    match value {
        Value::Array(values) => {
            if values.len() == 2
                && let Some(first) = values.first().and_then(Value::as_array)
            {
                return u64::try_from(first.len()).ok();
            }
            u64::try_from(values.len()).ok()
        }
        Value::Object(object) => {
            for key in ["hits", "pages", "items", "events", "scopes"] {
                if let Some(values) = object.get(key).and_then(Value::as_array) {
                    return u64::try_from(values.len()).ok();
                }
            }
            Some(1)
        }
        Value::Null => Some(0),
        Value::Bool(_) | Value::Number(_) | Value::String(_) => Some(1),
    }
}
