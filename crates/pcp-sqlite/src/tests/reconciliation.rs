use super::{SqlitePcpStore, pcp_client, principal, write_request};
use pcp_client::{AccessMode, PcpApi};
use pcp_core::{
    AccessPrincipalType, Actor, ActorType, ApplyReconciliationRequest, AssessPageValidityRequest,
    BrowseIndexOrder, CreateScopeRequest, FeedbackAuthority, FeedbackKind, PagePayload,
    PageRevisionRef, Projection, ReadPagesRequest, ReconciliationDisposition, RepairPageRequest,
    RevisePageRequest, SearchFilters, SearchMode, SearchPagesRequest, SubmitFeedbackRequest,
    ValidityStanding,
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

#[tokio::test]
async fn assessment_rationale_is_optional_and_audit_pages_are_not_knowledge() {
    let f = Fixture::open().await;
    let old = f.page("symbiont", "Old icon knowledge").await;
    let new = f.page("codex", "Current icon knowledge").await;
    let mut approval = serde_json::to_value(f.update(&old, &new, None)).unwrap();
    approval.as_object_mut().unwrap().remove("rationale");
    let result = f
        .admin
        .apply_reconciliation(serde_json::from_value(approval).unwrap())
        .await
        .unwrap();
    assert!(result.supersedes_relation.is_some());
    let validity = f.read(&old).await.validity.unwrap();
    assert_eq!(validity.standing, ValidityStanding::Superseded);
    assert!(validity.rationale.is_empty());
    assert!(validity.basis_revision_ids.contains(&new.revision_id));

    // The audit Page has no invented placeholder text, but remains readable by ID.
    let audit = f
        .admin
        .read_pages(ReadPagesRequest {
            page_ids: vec![validity.assessment_page_id.clone()],
            revision_ids: vec![],
            projections: vec![Projection::Payload, Projection::Provenance],
            max_chars: 8000,
        })
        .await
        .unwrap();
    assert!(audit[0].revision.payload.is_none());
    assert_eq!(audit[0].revision.provenance[0].actor.actor_id, "operator");

    // Existing verbose assessments must also be filtered, not just empty ones.
    let mut request = serde_json::to_value(AssessPageValidityRequest {
        target_page_id: old.page_id.clone(),
        target_revision_id: old.revision_id.clone(),
        expected_assessment_revision_id: Some(validity.assessment_revision_id.clone()),
        standing: ValidityStanding::Superseded,
        rationale: "knowledge auditneedle".into(),
        scope: None,
        basis_revision_ids: vec![new.revision_id.clone()],
        created_by: Actor {
            actor_type: ActorType::Model,
            actor_id: "model:audit".into(),
        },
        tool_or_model: None,
        idempotency_key: None,
    })
    .unwrap();
    let verbose = f
        .admin
        .assess_page_validity(serde_json::from_value(request.clone()).unwrap())
        .await
        .unwrap();
    assert_eq!(verbose.assessment_page_id, validity.assessment_page_id);

    // A tenant-chosen kind alone must never hide ordinary knowledge.
    let mut ordinary = write_request(
        f.store.identity_id(),
        "symbiont",
        f.actor.clone(),
        "knowledge named by tenant",
        "tenant-kind",
    );
    ordinary.kind = "validity_assessment".into();
    let ordinary = f
        .store
        .write_page(ordinary, vec!["symbiont".into()])
        .await
        .unwrap();
    let scopes = vec!["symbiont".into(), "codex".into()];
    let library = f
        .store
        .browse_content_pages(
            scopes.clone(),
            None,
            BrowseIndexOrder::Recent,
            1,
            None,
            32000,
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(library.total_pages, 2);
    assert_eq!(library.hits.len(), 1);
    let next = f
        .store
        .browse_content_pages(
            scopes.clone(),
            None,
            BrowseIndexOrder::Recent,
            1,
            library.next_cursor,
            32000,
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(next.hits.len(), 1);
    assert!(next.next_cursor.is_none());
    assert!(
        library
            .hits
            .iter()
            .chain(&next.hits)
            .any(|p| p.page_id == ordinary.page_id)
    );
    assert_eq!(
        f.store
            .content_library_summary(scopes.clone())
            .await
            .unwrap()
            .page_count,
        2
    );
    let retrieval = f
        .store
        .browse_retrieval_pages(
            scopes.clone(),
            None,
            BrowseIndexOrder::Recent,
            10,
            None,
            32000,
        )
        .await
        .unwrap();
    assert_eq!(retrieval.total_pages, 2);
    assert!(
        retrieval
            .hits
            .iter()
            .all(|p| p.page_id != validity.assessment_page_id)
    );
    let index = f
        .store
        .browse_index(
            scopes.clone(),
            vec![],
            BrowseIndexOrder::Recent,
            10,
            None,
            32000,
        )
        .await
        .unwrap();
    assert!(
        index
            .hits
            .iter()
            .all(|p| p.page_id != validity.assessment_page_id)
    );
    for mode in [
        SearchMode::Text,
        SearchMode::Exact,
        SearchMode::Auto,
        SearchMode::Temporal,
    ] {
        let found = f
            .store
            .search_pages(SearchPagesRequest {
                query: if mode == SearchMode::Temporal {
                    ""
                } else {
                    "knowledge"
                }
                .into(),
                scopes: scopes.clone(),
                mode,
                term_match: Default::default(),
                projections: vec![Projection::Payload],
                filters: SearchFilters::default(),
                limit: 10,
                cursor: None,
            })
            .await
            .unwrap();
        assert_eq!(found.hits.len(), 2);
        assert!(
            found
                .hits
                .iter()
                .all(|p| p.page_id != validity.assessment_page_id)
        );
    }
    // Repair the current explanation through the assessment owner. Old audit
    // text stays in history, and the replacement Relation is not duplicated.
    request.as_object_mut().unwrap().remove("rationale");
    request["expectedAssessmentRevisionId"] = verbose.assessment_revision_id.clone().into();
    let clean = f
        .admin
        .assess_page_validity(serde_json::from_value(request).unwrap())
        .await
        .unwrap();
    assert_eq!(clean.assessment_page_id, validity.assessment_page_id);
    let target = f
        .admin
        .read_pages(ReadPagesRequest {
            page_ids: vec![old.page_id],
            revision_ids: vec![],
            projections: vec![
                Projection::Validity,
                Projection::History,
                Projection::Payload,
            ],
            max_chars: 16000,
        })
        .await
        .unwrap();
    assert!(target[0].validity.as_ref().unwrap().rationale.is_empty());
    assert_eq!(target[0].validity_history.len(), 2);
    assert!(
        target[0]
            .validity_history
            .iter()
            .any(|v| v.rationale == "knowledge auditneedle")
    );
    assert!(
        target[0]
            .validity_history
            .iter()
            .any(|v| v.rationale.is_empty())
    );
    assert_eq!(
        target[0].revision.payload.as_ref().unwrap().content,
        "Old icon knowledge"
    );
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

    fn repair(&self, page: &str, revision: &str, content: &str) -> RepairPageRequest {
        RepairPageRequest {
            page_id: page.into(),
            expected_revision_id: revision.into(),
            payload: Some(PagePayload {
                media_type: "text/plain".into(),
                content: content.into(),
            }),
            source_refs: Vec::new(),
            facets: None,
            based_on_revision_ids: Vec::new(),
            reason: "Remove unrelated instructions from feedback".into(),
            tool_or_model: Some("pcp-console".into()),
            idempotency_key: None,
        }
    }
}

#[tokio::test]
async fn feedback_repair_moves_pending_work_and_rejects_old_approvals() {
    let f = Fixture::open().await;
    let old = f.page("symbiont", "Old icon description").await;
    let new = f.page("codex", "Correct icon description").await;
    let mut request = f.feedback(&old.revision_id, &new.revision_id);
    request.used_revision_ids = vec![old.revision_id.clone()];
    let signal = f.tenant.submit_feedback(request.clone()).await.unwrap();
    let stale_approval = f.update(&old, &new, Some(signal.feedback_revision_id.clone()));
    let repair = f.repair(
        &signal.feedback_page_id,
        &signal.feedback_revision_id,
        "Only the factual correction.",
    );
    assert!(f.tenant.repair_page(repair.clone()).await.is_err());
    // Failed content validation must roll back the new head and work index.
    for invalid in [String::new(), "x".repeat(32_001)] {
        let failed = f.repair(
            &signal.feedback_page_id,
            &signal.feedback_revision_id,
            &invalid,
        );
        assert!(f.admin.repair_page(failed).await.is_err());
        assert_eq!(
            f.admin
                .current_revision_id(signal.feedback_page_id.clone())
                .await
                .unwrap(),
            signal.feedback_revision_id
        );
    }
    let edited = f.admin.repair_page(repair.clone()).await.unwrap();
    assert!(f.admin.repair_page(repair).await.is_err());
    let pending = f.admin.pending_feedback(Vec::new(), 1).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].feedback_revision_id, edited.revision_id);
    assert_eq!(pending[0].feedback_page_id, signal.feedback_page_id);
    assert_eq!(
        pending[0].challenged_revision_ids,
        signal.challenged_revision_ids
    );
    assert_eq!(pending[0].used_revision_ids, signal.used_revision_ids);
    assert_eq!(
        pending[0].evidence_revision_ids,
        signal.evidence_revision_ids
    );
    assert_eq!(pending[0].authority, request.authority);
    assert_eq!(pending[0].kind, request.kind);
    assert!(
        f.admin
            .apply_reconciliation(stale_approval)
            .await
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
    assert!(f.read(&old).await.validity.is_none());
    assert_eq!(
        f.read(&edited).await.revision.payload.unwrap().content,
        "Only the factual correction."
    );
    let original = pcp_core::WriteResult {
        page_id: signal.feedback_page_id.clone(),
        revision_id: signal.feedback_revision_id.clone(),
        created: true,
    };
    assert_eq!(
        f.read(&original).await.revision.payload.unwrap().content,
        request.payload.content
    );
    // A retry of the originating MCP request is not a new feedback signal.
    assert!(!f.tenant.submit_feedback(request).await.unwrap().created);
    let edited_again = f
        .admin
        .repair_page(f.repair(&edited.page_id, &edited.revision_id, "Final wording."))
        .await
        .unwrap();
    assert_eq!(
        f.admin.pending_feedback(Vec::new(), 10).await.unwrap()[0].feedback_revision_id,
        edited_again.revision_id
    );
    assert!(
        f.admin
            .apply_reconciliation(f.update(&old, &new, Some(edited.revision_id)))
            .await
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
    let result = f
        .admin
        .apply_reconciliation(f.update(&old, &new, Some(edited_again.revision_id.clone())))
        .await
        .unwrap();
    assert!(result.created);
    assert_eq!(result.feedback_revision_id, Some(edited_again.revision_id));
    assert!(
        f.admin
            .pending_feedback(Vec::new(), 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn feedback_repair_preserves_already_applied_targets() {
    let f = Fixture::open().await;
    let first = f.page("symbiont", "First older claim").await;
    let second = f.page("symbiont", "Second older claim").await;
    let new = f.page("codex", "Current evidence").await;
    let mut request = f.feedback(&first.revision_id, &new.revision_id);
    request
        .challenged_revision_ids
        .push(second.revision_id.clone());
    let signal = f.tenant.submit_feedback(request).await.unwrap();
    let first_result = f
        .admin
        .apply_reconciliation(f.update(&first, &new, Some(signal.feedback_revision_id.clone())))
        .await
        .unwrap();
    let edited = f
        .admin
        .repair_page(f.repair(
            &signal.feedback_page_id,
            &signal.feedback_revision_id,
            "Edited second claim correction",
        ))
        .await
        .unwrap();
    let pending = f.admin.pending_feedback(Vec::new(), 10).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].challenged_revision_ids,
        vec![second.revision_id.clone()]
    );
    let replay = f
        .admin
        .apply_reconciliation(f.update(&first, &new, Some(edited.revision_id.clone())))
        .await
        .unwrap();
    assert!(!replay.created);
    assert_eq!(
        serde_json::to_value(replay.validity).unwrap(),
        serde_json::to_value(first_result.validity).unwrap()
    );
    assert_eq!(
        serde_json::to_value(replay.supersedes_relation).unwrap(),
        serde_json::to_value(first_result.supersedes_relation).unwrap()
    );
    let mut approval = f.update(&second, &new, Some(edited.revision_id.clone()));
    approval.idempotency_key = Some("approved:second".into());
    f.admin.apply_reconciliation(approval).await.unwrap();
    let final_edit = f
        .admin
        .repair_page(f.repair(
            &edited.page_id,
            &edited.revision_id,
            "Copy edit after completion",
        ))
        .await
        .unwrap();
    assert_ne!(final_edit.revision_id, edited.revision_id);
    assert!(
        f.admin
            .pending_feedback(Vec::new(), 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        f.read(&first).await.validity.unwrap().standing,
        ValidityStanding::Superseded
    );
    assert_eq!(
        f.read(&second).await.validity.unwrap().standing,
        ValidityStanding::Superseded
    );
}

#[tokio::test]
async fn deleted_feedback_is_not_actionable() {
    let f = Fixture::open().await;
    let old = f.page("symbiont", "Old claim").await;
    let new = f.page("codex", "New claim").await;
    let signal = f
        .tenant
        .submit_feedback(f.feedback(&old.revision_id, &new.revision_id))
        .await
        .unwrap();
    f.admin
        .delete_page(pcp_core::DeletePageRequest {
            page_id: signal.feedback_page_id,
            expected_revision_id: signal.feedback_revision_id.clone(),
            reason: Some("Feedback withdrawn".into()),
            idempotency_key: None,
        })
        .await
        .unwrap();
    assert!(
        f.admin
            .pending_feedback(Vec::new(), 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        f.admin
            .apply_reconciliation(f.update(&old, &new, Some(signal.feedback_revision_id)))
            .await
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
    assert!(f.read(&old).await.validity.is_none());
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
