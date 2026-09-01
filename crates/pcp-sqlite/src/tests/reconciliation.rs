use super::{SqlitePcpStore, pcp_client, principal, write_request};
use pcp_client::{AccessMode, PcpApi};
use pcp_core::{
    AccessPrincipalType, Actor, ActorType, ApplyReconciliationRequest, CreateScopeRequest,
    FeedbackAuthority, FeedbackKind, PagePayload, PageRevisionRef, Projection, ReadPagesRequest,
    ReconciliationDisposition, RevisePageRequest, SubmitFeedbackRequest, ValidityStanding,
};
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture {
    store: Arc<SqlitePcpStore>,
    admin: Arc<dyn PcpApi>,
    tenant: Arc<dyn PcpApi>,
    actor: Actor,
}
impl Fixture {
    async fn open() -> Self {
        let root = std::env::temp_dir().join(format!(
            "pcp-cross-feedback-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = Arc::new(
            SqlitePcpStore::open(root.join("store.sqlite3"))
                .await
                .unwrap(),
        );
        let admin = pcp_client(
            store.clone(),
            AccessMode::Admin.store_wide_session(
                principal("operator", AccessPrincipalType::Service),
                "admin",
                Vec::new(),
                true,
            ),
        );
        for scope in ["symbiont", "codex", "private"] {
            admin
                .create_scope(CreateScopeRequest {
                    namespace: scope.into(),
                    display_name: scope.into(),
                    description: None,
                    parent_namespace: None,
                })
                .await
                .unwrap();
        }
        let mut access = AccessMode::Contribute.session(
            principal("codex", AccessPrincipalType::ModelClient),
            "tenant",
            vec!["codex".into()],
            false,
        );
        access.grants.extend(
            AccessMode::Read
                .session(
                    principal("codex", AccessPrincipalType::ModelClient),
                    "read",
                    vec!["symbiont".into()],
                    false,
                )
                .grants,
        );
        let tenant = pcp_client(store.clone(), access);
        Self {
            store,
            admin,
            tenant,
            actor: Actor {
                actor_type: ActorType::Tool,
                actor_id: "operator".into(),
            },
        }
    }
    async fn page(&self, scope: &str, text: &str) -> pcp_core::WriteResult {
        self.store.flush_access_audit().await.unwrap();
        self.store
            .write_page(
                write_request(
                    self.store.identity_id(),
                    scope,
                    self.actor.clone(),
                    text,
                    text,
                ),
                vec![scope.into()],
            )
            .await
            .unwrap()
    }
    fn feedback(&self, old: &str, new: &str) -> SubmitFeedbackRequest {
        SubmitFeedbackRequest {
            namespace: "codex".into(),
            kind: FeedbackKind::Correction,
            authority: FeedbackAuthority::SubjectOwner,
            payload: PagePayload {
                media_type: "text/plain".into(),
                content: "用户确认用新事实纠正旧说明".into(),
            },
            observed_at: None,
            source_refs: Vec::new(),
            challenged_revision_ids: vec![old.into()],
            used_revision_ids: Vec::new(),
            evidence_revision_ids: vec![new.into()],
            response_ref: None,
            external_event_id: Some("correction:1".into()),
        }
    }
    fn update(
        &self,
        old: &pcp_core::WriteResult,
        new: &pcp_core::WriteResult,
        feedback: Option<String>,
    ) -> ApplyReconciliationRequest {
        let mut basis = vec![old.revision_id.clone(), new.revision_id.clone()];
        basis.extend(feedback.iter().cloned());
        ApplyReconciliationRequest {
            feedback_revision_id: feedback,
            expected_assessment_revision_id: None,
            target: PageRevisionRef {
                page_id: old.page_id.clone(),
                revision_id: old.revision_id.clone(),
            },
            disposition: ReconciliationDisposition::Superseded,
            rationale: "旧结论由新证据替代".into(),
            scope: None,
            replacement: Some(PageRevisionRef {
                page_id: new.page_id.clone(),
                revision_id: new.revision_id.clone(),
            }),
            basis_revision_ids: basis,
            created_by: self.actor.clone(),
            tool_or_model: None,
            idempotency_key: Some("approved:1".into()),
        }
    }
    async fn read(&self, old: &pcp_core::WriteResult) -> pcp_core::ReadPage {
        self.admin
            .read_pages(ReadPagesRequest {
                page_ids: Vec::new(),
                revision_ids: vec![old.revision_id.clone()],
                projections: vec![
                    Projection::Manifest,
                    Projection::Payload,
                    Projection::Validity,
                    Projection::Relations,
                ],
                max_chars: 8000,
            })
            .await
            .unwrap()
            .remove(0)
    }
}

#[tokio::test]
async fn cross_scope_feedback_records_new_evidence_without_write_or_derive_authority() {
    let f = Fixture::open().await;
    let old = f.page("symbiont", "旧事实").await;
    let new = f.page("codex", "修正后的事实").await;
    let submission = f
        .tenant
        .submit_feedback(f.feedback(&old.revision_id, &new.revision_id))
        .await
        .unwrap();
    assert_eq!(
        submission.evidence_revision_ids,
        vec![new.revision_id.clone()]
    );
    assert!(submission.used_revision_ids.is_empty());
    assert!(f.read(&old).await.validity.is_none());
    let pending = f.admin.pending_feedback(Vec::new(), 10).await.unwrap();
    assert_eq!(
        pending[0].evidence_revision_ids,
        submission.evidence_revision_ids
    );
    let request = f.update(&old, &new, Some(submission.feedback_revision_id));
    assert!(
        f.tenant
            .apply_reconciliation(request.clone())
            .await
            .is_err()
    );
    assert!(f.read(&old).await.validity.is_none());
    let result = f.admin.apply_reconciliation(request.clone()).await.unwrap();
    assert!(result.supersedes_relation.is_some());
    assert_eq!(
        f.read(&old).await.validity.unwrap().standing,
        ValidityStanding::Superseded
    );
    assert!(!f.admin.apply_reconciliation(request).await.unwrap().created);
}

#[tokio::test]
async fn feedback_cannot_read_hidden_evidence_or_write_into_the_challenged_scope() {
    let f = Fixture::open().await;
    let old = f.page("symbiont", "old").await;
    let hidden = f.page("private", "private evidence").await;
    assert!(
        f.tenant
            .submit_feedback(f.feedback(&old.revision_id, &hidden.revision_id))
            .await
            .is_err()
    );
    let mut wrong_scope = f.feedback(&old.revision_id, &old.revision_id);
    wrong_scope.namespace = "symbiont".into();
    assert!(f.tenant.submit_feedback(wrong_scope).await.is_err());
    assert!(
        f.admin
            .pending_feedback(Vec::new(), 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn discovered_update_is_atomic_and_rejects_changed_replacement() {
    let f = Fixture::open().await;
    let old = f.page("symbiont", "old assertion").await;
    let new = f.page("codex", "new assertion").await;
    let request = f.update(&old, &new, None);
    f.admin
        .revise_page(RevisePageRequest {
            page_id: new.page_id.clone(),
            expected_revision_id: new.revision_id.clone(),
            lifecycle_status: pcp_core::LifecycleStatus::Active,
            payload: Some(PagePayload {
                media_type: "text/plain".into(),
                content: "changed after review".into(),
            }),
            source_refs: Vec::new(),
            facets: None,
            provenance: Vec::new(),
            created_by: f.actor.clone(),
            observed_at: None,
            valid_from: None,
            valid_to: None,
            initial_relations: Vec::new(),
            idempotency_key: None,
        })
        .await
        .unwrap();
    let error = f.admin.apply_reconciliation(request).await.unwrap_err();
    assert!(error.to_string().contains("stale"));
    assert!(
        f.read(&old).await.validity.is_none(),
        "failed replacement must roll back validity"
    );
}

#[tokio::test]
async fn discovered_update_needs_operator_and_creates_no_fake_feedback() {
    let f = Fixture::open().await;
    let old = f.page("symbiont", "old state").await;
    let new = f.page("codex", "current state").await;
    let request = f.update(&old, &new, None);
    assert!(
        f.tenant
            .apply_reconciliation(request.clone())
            .await
            .is_err()
    );
    let result = f.admin.apply_reconciliation(request).await.unwrap();
    assert!(result.feedback_revision_id.is_none());
    assert!(
        f.admin
            .pending_feedback(Vec::new(), 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        f.read(&old).await.revision.payload.unwrap().content,
        "old state"
    );
}

#[tokio::test]
async fn discovered_update_retry_cannot_change_the_approved_decision() {
    let f = Fixture::open().await;
    let old = f.page("symbiont", "old state").await;
    let new = f.page("codex", "new state").await;
    let request = f.update(&old, &new, None);
    f.admin.apply_reconciliation(request.clone()).await.unwrap();
    assert!(
        !f.admin
            .apply_reconciliation(request.clone())
            .await
            .unwrap()
            .created
    );
    let other = f.page("codex", "different replacement").await;
    let changed = f.update(&old, &other, None);
    assert!(
        f.admin
            .apply_reconciliation(changed)
            .await
            .unwrap_err()
            .to_string()
            .contains("idempotency")
    );
    let mut changed = request;
    changed.disposition = ReconciliationDisposition::Retracted;
    changed.replacement = None;
    assert!(f.admin.apply_reconciliation(changed).await.is_err());
    assert_eq!(
        f.read(&old).await.validity.unwrap().standing,
        ValidityStanding::Superseded
    );
}

#[tokio::test]
async fn reviewed_update_cannot_overwrite_a_newer_validity_decision() {
    let f = Fixture::open().await;
    let old = f.page("symbiont", "old state").await;
    let new = f.page("codex", "new state").await;
    let pending = f.update(&old, &new, None);
    let mut other = pending.clone();
    other.disposition = ReconciliationDisposition::Disputed;
    other.replacement = None;
    other.idempotency_key = Some("separate-review".into());
    let applied = f.admin.apply_reconciliation(other).await.unwrap();
    assert!(
        f.admin
            .apply_reconciliation(pending.clone())
            .await
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
    assert_eq!(
        f.read(&old).await.validity.unwrap().standing,
        ValidityStanding::Disputed
    );
    let mut reviewed_again = pending;
    reviewed_again.expected_assessment_revision_id =
        Some(applied.validity.unwrap().assessment_revision_id);
    f.admin.apply_reconciliation(reviewed_again).await.unwrap();
    assert_eq!(
        f.read(&old).await.validity.unwrap().standing,
        ValidityStanding::Superseded
    );
}

#[tokio::test]
async fn older_revision_assessment_does_not_block_reviewing_the_new_head() {
    let f = Fixture::open().await;
    let old = f.page("symbiont", "old state").await;
    let new = f.page("codex", "new state").await;
    let mut earlier = f.update(&old, &new, None);
    earlier.disposition = ReconciliationDisposition::Disputed;
    earlier.replacement = None;
    f.admin.apply_reconciliation(earlier).await.unwrap();
    let revised = f
        .admin
        .revise_page(RevisePageRequest {
            page_id: old.page_id.clone(),
            expected_revision_id: old.revision_id,
            lifecycle_status: pcp_core::LifecycleStatus::Active,
            payload: Some(PagePayload {
                media_type: "text/plain".into(),
                content: "updated old state".into(),
            }),
            source_refs: Vec::new(),
            facets: None,
            provenance: Vec::new(),
            created_by: f.actor.clone(),
            observed_at: None,
            valid_from: None,
            valid_to: None,
            initial_relations: Vec::new(),
            idempotency_key: None,
        })
        .await
        .unwrap();
    assert!(f.read(&revised).await.validity.is_none());
    let mut reviewed = f.update(&revised, &new, None);
    reviewed.idempotency_key = Some("new-head-review".into());
    f.admin.apply_reconciliation(reviewed).await.unwrap();
}
