use super::super::{
    MaintenanceReviewPayload, MaintenanceWorkerRequest, MaintenanceWorkerResponse,
    RuntimeMaintainer,
};
use super::{FakeWorker, Fixture};
use pcp_client::{AccessMode, EmbeddedPcpClient};
use pcp_core::{
    AccessPrincipal, AccessPrincipalType, Actor, ActorType, CreateScopeRequest, FeedbackAuthority,
    FeedbackKind, PagePayload, Projection, ProvenanceEvent, ReadPagesRequest,
    ReconciliationDisposition, SubmitFeedbackRequest, ValidityStanding,
};
use std::sync::Arc;

struct ConcurrentAssessmentWorker {
    client: Arc<dyn pcp_client::PcpApi>,
}

#[async_trait::async_trait]
impl super::super::SemanticMaintenanceWorker for ConcurrentAssessmentWorker {
    async fn evaluate(
        &self,
        request: MaintenanceWorkerRequest,
    ) -> anyhow::Result<MaintenanceWorkerResponse> {
        let MaintenanceWorkerRequest::ReconcileFeedback {
            targets, signal, ..
        } = request
        else {
            anyhow::bail!("unexpected operation")
        };
        let target = targets
            .iter()
            .find(|page| signal.challenged_revision_ids.contains(&page.revision_id))
            .unwrap();
        // Simulate an independent operator decision while inference is in flight.
        self.client
            .assess_page_validity(pcp_core::AssessPageValidityRequest {
                target_page_id: target.page_id.clone(),
                target_revision_id: target.revision_id.clone(),
                expected_assessment_revision_id: None,
                standing: ValidityStanding::Live,
                rationale: "New independent review".into(),
                scope: None,
                basis_revision_ids: vec![target.revision_id.clone()],
                created_by: Actor {
                    actor_type: ActorType::Tool,
                    actor_id: "operator".into(),
                },
                tool_or_model: None,
                idempotency_key: None,
            })
            .await?;
        Ok(MaintenanceWorkerResponse::ReconcileFeedback {
            target_revision_id: target.revision_id.clone(),
            disposition: ReconciliationDisposition::Disputed,
            rationale: "Older in-flight analysis".into(),
            scope: None,
            replacement_revision_id: None,
        })
    }
}

#[tokio::test]
async fn validity_changed_during_inference_requires_a_fresh_review() {
    let f = fixture().await;
    let old = f
        .client
        .write_page(f.page("Claim being reviewed.", "concurrent:old"))
        .await
        .unwrap();
    f.client
        .submit_feedback(SubmitFeedbackRequest {
            namespace: "codex".into(),
            kind: FeedbackKind::Challenge,
            authority: FeedbackAuthority::SubjectOwner,
            payload: PagePayload {
                media_type: "text/plain".into(),
                content: "Please review".into(),
            },
            observed_at: None,
            source_refs: Vec::new(),
            challenged_revision_ids: vec![old.revision_id.clone()],
            used_revision_ids: Vec::new(),
            evidence_revision_ids: Vec::new(),
            response_ref: None,
            external_event_id: None,
        })
        .await
        .unwrap();
    let mut config = f.config();
    config.summary.enabled = false;
    config.packing.enabled = false;
    config.relation.enabled = false;
    config.retention.enabled = false;
    config.allowed_scopes.push("codex".into());
    let worker = Arc::new(ConcurrentAssessmentWorker {
        client: f.client.clone(),
    });
    let mut maintainer = RuntimeMaintainer::for_test(f.client.clone(), worker, config);
    maintainer.run_once().await.unwrap();
    let reviews = maintainer.pending_reviews();
    assert_eq!(reviews.len(), 1);
    let error = maintainer
        .approve_reconciliation_review(&reviews[0].candidate_id)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("stale"));
    assert!(maintainer.pending_reviews().is_empty());
    let pages = f
        .client
        .read_pages(ReadPagesRequest {
            page_ids: Vec::new(),
            revision_ids: vec![old.revision_id],
            projections: vec![Projection::Validity],
            max_chars: 4000,
        })
        .await
        .unwrap();
    assert_eq!(
        pages[0].validity.as_ref().unwrap().standing,
        ValidityStanding::Live
    );
    f.close().await;
}

async fn fixture() -> Fixture {
    let mut f = Fixture::open("update-discovery").await;
    f.client = EmbeddedPcpClient::shared(
        f.store.clone(),
        AccessMode::Admin.store_wide_session(
            AccessPrincipal {
                principal_id: "service:test".into(),
                principal_type: AccessPrincipalType::Service,
                display_name: None,
            },
            "session:update",
            Vec::new(),
            true,
        ),
    );
    f.client
        .create_scope(CreateScopeRequest {
            namespace: "codex".into(),
            display_name: "Codex".into(),
            description: None,
            parent_namespace: None,
        })
        .await
        .unwrap();
    f
}

#[tokio::test]
async fn ordinary_cross_scope_update_requires_review_and_preserves_source_history() {
    assert_update_review(false).await;
}

#[tokio::test]
async fn provenance_only_guides_discovery_and_never_asserts_replacement() {
    assert_update_review(true).await;
}

async fn assert_update_review(with_provenance: bool) {
    let f = fixture().await;
    let old = f
        .client
        .write_page(f.page(
            "PCP updates originally require restarting the client.",
            "update:old",
        ))
        .await
        .unwrap();
    let mut new = f.page(
        "PCP updates now refresh the client without restarting it.",
        "update:new",
    );
    new.namespace = "codex".into();
    if with_provenance {
        new.provenance = vec![ProvenanceEvent {
            operation: "derive".into(),
            actor: Actor {
                actor_type: ActorType::Tool,
                actor_id: "codex".into(),
            },
            timestamp: "2026-09-02T00:00:00Z".into(),
            input_revision_ids: vec![old.revision_id.clone()],
            tool_or_model: None,
            reason: None,
        }];
    } else {
        // Store timestamps have millisecond precision; keep fixture chronology unambiguous.
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    let new = f.client.write_page(new).await.unwrap();
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::ReconcileFeedback {
            target_revision_id: old.revision_id.clone(),
            disposition: ReconciliationDisposition::Superseded,
            rationale: "新证据完整替代旧限制".into(),
            scope: None,
            replacement_revision_id: Some(new.revision_id.clone()),
        },
    ]));
    let mut config = f.config();
    config.summary.enabled = false;
    config.packing.enabled = false;
    config.relation.enabled = false;
    config.retention.enabled = false;
    config.reconciliation.discover_updates = true;
    config.allowed_scopes.push("codex".into());
    let mut maintainer = RuntimeMaintainer::for_test(f.client.clone(), worker.clone(), config);
    let report = maintainer.run_once().await.unwrap();
    assert_eq!(report.reconciliations_proposed, 1);
    assert_eq!(report.reconciliations_committed, 0);
    assert!(
        f.client
            .pending_feedback(Vec::new(), 10)
            .await
            .unwrap()
            .is_empty()
    );
    let read_request = ReadPagesRequest {
        page_ids: Vec::new(),
        revision_ids: vec![old.revision_id.clone()],
        projections: vec![
            Projection::Manifest,
            Projection::Validity,
            Projection::Payload,
        ],
        max_chars: 8000,
    };
    assert!(
        f.client.read_pages(read_request.clone()).await.unwrap()[0]
            .validity
            .is_none()
    );
    let pending = maintainer.pending_reviews();
    let MaintenanceReviewPayload::Reconciliation(candidate) = &pending[0].payload else {
        panic!("wrong review type")
    };
    assert!(candidate.signal.is_none());
    assert_eq!(candidate.evidence[0].revision_id, new.revision_id);
    assert!(matches!(
        worker.requests.lock().unwrap()[0],
        MaintenanceWorkerRequest::ReviewUpdate { .. }
    ));
    let result = maintainer
        .approve_reconciliation_review(&pending[0].candidate_id)
        .await
        .unwrap();
    assert!(result.feedback_revision_id.is_none());
    let read = f.client.read_pages(read_request).await.unwrap();
    assert_eq!(
        read[0].validity.as_ref().unwrap().standing,
        ValidityStanding::Superseded
    );
    assert!(
        read[0]
            .revision
            .payload
            .as_ref()
            .unwrap()
            .content
            .contains("originally")
    );
    assert!(maintainer.pending_reviews().is_empty());
    f.close().await;
}

#[tokio::test]
async fn new_feedback_evidence_is_offered_separately_and_cross_scope_dispute_waits_for_review() {
    let f = fixture().await;
    let old = f
        .client
        .write_page(f.page("An earlier claim.", "feedback:old"))
        .await
        .unwrap();
    let mut new = f.page("New contradictory evidence.", "feedback:new");
    new.namespace = "codex".into();
    let new = f.client.write_page(new).await.unwrap();
    f.client
        .submit_feedback(SubmitFeedbackRequest {
            namespace: "codex".into(),
            kind: FeedbackKind::Correction,
            authority: FeedbackAuthority::SubjectOwner,
            payload: PagePayload {
                media_type: "text/plain".into(),
                content: "用户明确纠正".into(),
            },
            observed_at: None,
            source_refs: Vec::new(),
            challenged_revision_ids: vec![old.revision_id.clone()],
            used_revision_ids: Vec::new(),
            evidence_revision_ids: vec![new.revision_id.clone()],
            response_ref: None,
            external_event_id: None,
        })
        .await
        .unwrap();
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::ReconcileFeedback {
            target_revision_id: old.revision_id.clone(),
            disposition: ReconciliationDisposition::Disputed,
            rationale: "证据相互冲突".into(),
            scope: None,
            replacement_revision_id: None,
        },
    ]));
    let mut config = f.config();
    config.summary.enabled = false;
    config.packing.enabled = false;
    config.relation.enabled = false;
    config.retention.enabled = false;
    config.allowed_scopes.push("codex".into());
    let mut maintainer = RuntimeMaintainer::for_test(f.client.clone(), worker.clone(), config);
    let report = maintainer.run_once().await.unwrap();
    assert_eq!(report.reconciliations_committed, 0);
    assert_eq!(report.reconciliations_proposed, 1);
    let requests = worker.requests.lock().unwrap();
    let MaintenanceWorkerRequest::ReconcileFeedback {
        signal, targets, ..
    } = &requests[0]
    else {
        panic!("wrong operation")
    };
    assert!(signal.used_revision_ids.is_empty());
    assert_eq!(signal.evidence_revision_ids, vec![new.revision_id.clone()]);
    assert!(
        targets
            .iter()
            .any(|page| page.revision_id == new.revision_id)
    );
    drop(requests);
    f.close().await;
}
