use anyhow::Result;
use async_trait::async_trait;
use pcp_core::{
    AccessAuditEvent, AccessDecision, AccessPermission, AccessSession, AssessPageValidityRequest,
    Capabilities, CreateScopeRequest, LinkPagesRequest, Projection, ProvenanceEvent, ReadPage,
    ReadPagesRequest, Relation, RevisePageRequest, Scope, SearchPagesRequest, SearchResult,
    WritePageRequest, WriteResult, WriteSummaryRequest, WriteSummaryResult, WriteValidityResult,
};
use pcp_store::{DurablePageInventoryItem, PcpStore, TombstoneCascadeResult};

use crate::{
    SqlitePcpStore,
    access::{authorize_exact, authorize_scopes},
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
            return complete(self, access, "create_scope", scopes, Err(error), true).await;
        }
        let result = SqlitePcpStore::create_scope(self, request).await;
        complete(self, access, "create_scope", scopes, result, false).await
    }

    async fn list_scopes(
        &self,
        access: &AccessSession,
        requested_scopes: Vec<String>,
        query: Option<String>,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<(Vec<Scope>, Option<String>)> {
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
                    )
                    .await;
                }
            };
        let result = SqlitePcpStore::list_scopes(self, scopes.clone(), query, limit, cursor).await;
        complete(self, access, "list_scopes", scopes, result, false).await
    }

    async fn search_pages(
        &self,
        access: &AccessSession,
        mut request: SearchPagesRequest,
    ) -> Result<SearchResult> {
        if request.projections.is_empty() {
            request.projections = pcp_core::default_search_projections();
        }
        let permissions = search_permissions(&request.projections);
        let requested = request.scopes.clone();
        let scopes = match authorize_scopes(access, &permissions, &requested) {
            Ok(scopes) => scopes,
            Err(error) => {
                return complete(self, access, "search_pages", requested, Err(error), true).await;
            }
        };
        request.scopes = scopes.clone();
        let result = SqlitePcpStore::search_pages(self, request).await;
        complete(self, access, "search_pages", scopes, result, false).await
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
        complete(self, access, "browse_index", scopes, result, false).await
    }

    async fn read_pages(
        &self,
        access: &AccessSession,
        request: ReadPagesRequest,
    ) -> Result<Vec<ReadPage>> {
        let permission = if request.projections.iter().any(is_detail_projection) {
            AccessPermission::ReadDetail
        } else {
            AccessPermission::ReadSummary
        };
        let scopes = match authorize_scopes(access, &[permission], &[]) {
            Ok(scopes) => scopes,
            Err(error) => {
                return complete(self, access, "read_pages", Vec::new(), Err(error), true).await;
            }
        };
        let result = SqlitePcpStore::read_pages(self, request, scopes.clone()).await;
        complete(self, access, "read_pages", scopes, result, false).await
    }

    async fn current_revision_id(&self, access: &AccessSession, page_id: String) -> Result<String> {
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
                )
                .await;
            }
        };
        let result = SqlitePcpStore::current_revision_id(self, page_id, scopes.clone()).await;
        complete(self, access, "current_revision", scopes, result, false).await
    }

    async fn page_count(
        &self,
        access: &AccessSession,
        requested_scopes: Vec<String>,
    ) -> Result<u64> {
        let scopes = match authorize_scopes(access, &[AccessPermission::Search], &requested_scopes)
        {
            Ok(scopes) => scopes,
            Err(error) => {
                return complete(self, access, "page_count", Vec::new(), Err(error), true).await;
            }
        };
        let result = SqlitePcpStore::page_count(self, scopes.clone()).await;
        complete(self, access, "page_count", scopes, result, false).await
    }

    async fn content_char_count(
        &self,
        access: &AccessSession,
        requested_scopes: Vec<String>,
    ) -> Result<usize> {
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
                    )
                    .await;
                }
            };
        let result = SqlitePcpStore::content_char_count(self, scopes.clone()).await;
        complete(self, access, "content_char_count", scopes, result, false).await
    }

    async fn write_page(
        &self,
        access: &AccessSession,
        request: WritePageRequest,
    ) -> Result<WriteResult> {
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
                    .revision_namespaces(
                        request
                            .initial_relations
                            .iter()
                            .map(|relation| relation.to_revision_id.clone())
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
            return complete(self, access, "write_page", audit_scopes, Err(error), true).await;
        }
        let result = SqlitePcpStore::write_page(self, request, audit_scopes.clone()).await;
        complete(self, access, "write_page", audit_scopes, result, false).await
    }

    async fn revise_page(
        &self,
        access: &AccessSession,
        request: RevisePageRequest,
    ) -> Result<WriteResult> {
        let target_scope = match self.page_namespace(request.page_id.clone()).await {
            Ok(scope) => scope,
            Err(error) => {
                return complete(self, access, "revise_page", Vec::new(), Err(error), false).await;
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
                    .revision_namespaces(
                        request
                            .initial_relations
                            .iter()
                            .map(|relation| relation.to_revision_id.clone())
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
            return complete(self, access, "revise_page", audit_scopes, Err(error), true).await;
        }
        let result = SqlitePcpStore::revise_page(self, request, audit_scopes.clone()).await;
        complete(self, access, "revise_page", audit_scopes, result, false).await
    }

    async fn link_pages(
        &self,
        access: &AccessSession,
        request: LinkPagesRequest,
    ) -> Result<Relation> {
        let scopes = match self
            .revision_namespaces(vec![
                request.from_revision_id.clone(),
                request.to_revision_id.clone(),
            ])
            .await
        {
            Ok(scopes) => scopes,
            Err(error) => {
                return complete(self, access, "link_pages", Vec::new(), Err(error), false).await;
            }
        };
        for scope in &scopes {
            if let Err(error) = authorize_exact(access, scope, AccessPermission::Link) {
                return complete(self, access, "link_pages", scopes, Err(error), true).await;
            }
        }
        let result = SqlitePcpStore::link_pages(self, request, scopes.clone()).await;
        complete(self, access, "link_pages", scopes, result, false).await
    }

    async fn write_summary(
        &self,
        access: &AccessSession,
        request: WriteSummaryRequest,
    ) -> Result<WriteSummaryResult> {
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
                return complete(self, access, "write_summary", Vec::new(), Err(error), false)
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
            return complete(self, access, "write_summary", scopes, Err(error), true).await;
        }
        let result = SqlitePcpStore::write_summary(self, request, scopes.clone()).await;
        complete(self, access, "write_summary", scopes, result, false).await
    }

    async fn next_summary_candidate(
        &self,
        access: &AccessSession,
        minimum_chars: usize,
        excluded_page_kinds: Vec<String>,
    ) -> Result<Option<String>> {
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
        complete(self, access, "mark_summary_assessed", scopes, result, false).await
    }

    async fn assess_page_validity(
        &self,
        access: &AccessSession,
        request: AssessPageValidityRequest,
    ) -> Result<WriteValidityResult> {
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
            return complete(self, access, "assess_validity", scopes, Err(error), true).await;
        }
        let result = SqlitePcpStore::assess_page_validity(self, request, scopes.clone()).await;
        complete(self, access, "assess_validity", scopes, result, false).await
    }

    async fn tombstone_derivation_cascade(
        &self,
        access: &AccessSession,
        root_revision_id: String,
        actor: pcp_core::Actor,
    ) -> Result<TombstoneCascadeResult> {
        let scopes = match authorize_scopes(access, &[AccessPermission::Retract], &[]) {
            Ok(scopes) => scopes,
            Err(error) => {
                return complete(self, access, "retract", Vec::new(), Err(error), true).await;
            }
        };
        let result = SqlitePcpStore::tombstone_derivation_cascade(
            self,
            root_revision_id,
            actor,
            scopes.clone(),
        )
        .await;
        complete(self, access, "retract", scopes, result, false).await
    }

    async fn durable_page_inventory(
        &self,
        access: &AccessSession,
        excluded_page_kinds: Vec<String>,
    ) -> Result<Vec<DurablePageInventoryItem>> {
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
                )
                .await;
            }
        };
        let result =
            SqlitePcpStore::durable_page_inventory(self, scopes.clone(), excluded_page_kinds).await;
        complete(self, access, "durable_inventory", scopes, result, false).await
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
) -> Result<T> {
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
    let audit = store
        .record_access(access, operation, &scopes, decision, detail)
        .await;
    match (result, audit) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}
