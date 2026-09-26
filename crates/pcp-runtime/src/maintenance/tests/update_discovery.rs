use super::super::{
    MaintenanceReviewPayload, MaintenanceWorkerRequest, MaintenanceWorkerResponse,
    RuntimeMaintainer,
};
use super::{FakeWorker, Fixture};
use pcp_client::{AccessMode, EmbeddedPcpClient};
use pcp_core::{
    AccessPrincipal, AccessPrincipalType, Actor, ActorType, ConsolidatedPageOutput,
    ConsolidationCoverage, CreateScopeRequest, FeedbackAuthority, FeedbackKind, PagePayload,
    Projection, ProvenanceEvent, ReadPagesRequest, ReconciliationDisposition,
    SubmitFeedbackRequest, ValidityStanding,
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

#[tokio::test]
async fn overlapping_pages_propose_reviewed_fusion_without_changing_validity() {
    let f = fixture().await;
    let old = f
        .client
        .write_page(f.page(
            "The workflow uses plan A. Issue B remains open.",
            "fusion:old",
        ))
        .await
        .unwrap();
    let mut new = f.page(
        "The workflow changed plan A to A2. Issue B remains open.",
        "fusion:new",
    );
    new.provenance = vec![ProvenanceEvent {
        operation: "derive".into(),
        actor: Actor {
            actor_type: ActorType::Tool,
            actor_id: "reviewer".into(),
        },
        timestamp: "2026-09-02T00:00:00Z".into(),
        input_revision_ids: vec![old.revision_id.clone()],
        tool_or_model: None,
        reason: None,
    }];
    let new = f.client.write_page(new).await.unwrap();
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::ConsolidatePages {
            rationale: "The two Pages overlap, but the second changes only plan A.".into(),
            outputs: vec![
                ConsolidatedPageOutput {
                    title: "Plan A".into(),
                    content: "The workflow formerly used A and later changed to A2.".into(),
                    source_indexes: vec![0, 1],
                },
                ConsolidatedPageOutput {
                    title: "Issue B".into(),
                    content: "Issue B remains open in both observations.".into(),
                    source_indexes: vec![0, 1],
                },
            ],
            coverage: vec![
                ConsolidationCoverage {
                    source_index: 0,
                    output_indexes: vec![0, 1],
                    explanation: "Both its plan and open issue are preserved.".into(),
                    complete: true,
                },
                ConsolidationCoverage {
                    source_index: 1,
                    output_indexes: vec![0, 1],
                    explanation: "Both its plan change and open issue are preserved.".into(),
                    complete: true,
                },
            ],
        },
    ]));
    let mut config = f.config();
    config.summary.enabled = false;
    config.packing.enabled = false;
    config.relation.enabled = false;
    config.retention.enabled = false;
    config.reconciliation.discover_updates = true;
    let mut maintainer = RuntimeMaintainer::for_test(f.client.clone(), worker, config);
    let report = maintainer.run_once().await.unwrap();
    assert_eq!(report.reconciliations_proposed, 1);
    let reviews = maintainer.pending_reviews();
    assert_eq!(reviews.len(), 1);
    let MaintenanceReviewPayload::Reconciliation(candidate) = &reviews[0].payload else {
        panic!("wrong review type")
    };
    assert!(candidate.suggested_consolidation.is_some());
    assert_eq!(
        candidate.disposition,
        ReconciliationDisposition::NoSourceChange
    );
    assert_eq!(candidate.target.revision_id, old.revision_id);
    assert_eq!(candidate.evidence[0].revision_id, new.revision_id);
    assert!(
        maintainer
            .approve_reconciliation_review(&reviews[0].candidate_id)
            .await
            .unwrap_err()
            .to_string()
            .contains("fusion draft")
    );
    assert_eq!(f.client.page_count(vec![]).await.unwrap(), 2);
    f.close().await;
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
async fn discovered_qualification_requires_and_applies_the_reviewers_boundary() {
    let f = fixture().await;
    let old = f
        .client
        .write_page(f.page("A proposed general mechanism.", "qualified:old"))
        .await
        .unwrap();
    let mut new = f.page(
        "One observed workflow supports a narrower claim.",
        "qualified:new",
    );
    new.provenance = vec![ProvenanceEvent {
        operation: "derive".into(),
        actor: Actor {
            actor_type: ActorType::Tool,
            actor_id: "reviewer".into(),
        },
        timestamp: "2026-09-02T00:00:00Z".into(),
        input_revision_ids: vec![old.revision_id.clone()],
        tool_or_model: None,
        reason: None,
    }];
    let new = f.client.write_page(new).await.unwrap();
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::ReconcileFeedback {
            target_revision_id: old.revision_id.clone(),
            disposition: ReconciliationDisposition::Qualified,
            rationale: "A narrower claim may be appropriate.".into(),
            scope: None,
            replacement_revision_id: None,
        },
    ]));
    let mut config = f.config();
    config.summary.enabled = false;
    config.packing.enabled = false;
    config.relation.enabled = false;
    config.retention.enabled = false;
    config.reconciliation.discover_updates = true;
    let mut maintainer = RuntimeMaintainer::for_test(f.client.clone(), worker, config);
    assert_eq!(
        maintainer
            .run_once()
            .await
            .unwrap()
            .reconciliations_proposed,
        1
    );
    let review = maintainer.pending_reviews().remove(0);
    let MaintenanceReviewPayload::Reconciliation(candidate) = &review.payload else {
        panic!("wrong review type")
    };
    assert_eq!(candidate.evidence[0].revision_id, new.revision_id);
    assert!(candidate.scope.is_none());
    assert!(
        maintainer
            .approve_reconciliation_review(&review.candidate_id)
            .await
            .is_err()
    );

    let request = ReadPagesRequest {
        page_ids: Vec::new(),
        revision_ids: vec![old.revision_id],
        projections: vec![Projection::Validity],
        max_chars: 4000,
    };
    assert!(
        f.client.read_pages(request.clone()).await.unwrap()[0]
            .validity
            .is_none()
    );
    maintainer
        .approve_reconciliation_review_with_qualification(
            &review.candidate_id,
            " Only for the observed workflow. ".into(),
            " The broader mechanism remains unverified. ".into(),
        )
        .await
        .unwrap();
    let validity = f
        .client
        .read_pages(request)
        .await
        .unwrap()
        .remove(0)
        .validity
        .unwrap();
    assert_eq!(validity.standing, ValidityStanding::Qualified);
    assert_eq!(
        validity.scope.as_deref(),
        Some("Only for the observed workflow.")
    );
    assert_eq!(
        validity.rationale,
        "The broader mechanism remains unverified."
    );
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

#[tokio::test]
async fn covered_source_skips_own_output_but_remains_available_for_new_evidence() {
    let f = fixture().await;
    let old = f
        .client
        .write_page(f.page(
            "PCP source evidence retains the original workflow boundary.",
            "covered-loop:old",
        ))
        .await
        .unwrap();
    let second = f
        .client
        .write_page(f.page(
            "PCP source evidence adds the workflow validation boundary.",
            "covered-loop:second",
        ))
        .await
        .unwrap();
    let published = f.client.consolidate_pages(pcp_core::ConsolidatePagesRequest {
        namespace: f.namespace.clone(),
        source_pages: vec![
            pcp_core::PageRevisionRef { page_id: old.page_id.clone(), revision_id: old.revision_id.clone() },
            pcp_core::PageRevisionRef { page_id: second.page_id.clone(), revision_id: second.revision_id.clone() },
        ],
        outputs: vec![ConsolidatedPageOutput {
            title: "Workflow boundaries".into(),
            content: "PCP source evidence retains the original workflow boundary and adds the validation boundary.".into(),
            source_indexes: vec![0, 1],
        }],
        coverage: (0..2).map(|source_index| ConsolidationCoverage {
            source_index,
            output_indexes: vec![0],
            explanation: "Both complete source boundaries are retained.".into(),
            complete: true,
        }).collect(),
        created_by: Actor { actor_type: ActorType::Tool, actor_id: "operator".into() },
        idempotency_key: "covered-loop:publish".into(),
    }).await.unwrap();
    let output = &published.outputs[0];
    let mut independent = f.page(
        "PCP source evidence now adds a separate workflow authorization condition.",
        "covered-loop:independent",
    );
    independent.provenance = vec![ProvenanceEvent {
        operation: "derive".into(),
        actor: Actor {
            actor_type: ActorType::Tool,
            actor_id: "independent-observer".into(),
        },
        timestamp: "2026-09-26T00:00:00Z".into(),
        input_revision_ids: vec![old.revision_id.clone()],
        tool_or_model: None,
        reason: None,
    }];
    let independent = f.client.write_page(independent).await.unwrap();

    let partial = f.client.consolidate_pages(pcp_core::ConsolidatePagesRequest {
        namespace: f.namespace.clone(),
        source_pages: vec![
            pcp_core::PageRevisionRef { page_id: old.page_id.clone(), revision_id: old.revision_id.clone() },
            pcp_core::PageRevisionRef { page_id: independent.page_id.clone(), revision_id: independent.revision_id.clone() },
        ],
        outputs: vec![ConsolidatedPageOutput {
            title: "Separate authorization condition".into(),
            content: "PCP workflow authorization now has a separate condition; this output does not preserve the original boundary.".into(),
            source_indexes: vec![0, 1],
        }],
        coverage: vec![
            ConsolidationCoverage { source_index: 0, output_indexes: vec![0], explanation: "The original boundary is not fully preserved here.".into(), complete: false },
            ConsolidationCoverage { source_index: 1, output_indexes: vec![0], explanation: "The independent authorization observation is fully preserved.".into(), complete: true },
        ],
        created_by: Actor { actor_type: ActorType::Tool, actor_id: "operator".into() },
        idempotency_key: "covered-loop:partial-output".into(),
    }).await.unwrap();
    let inventory = f.client.durable_page_inventory(vec![]).await.unwrap();
    let source = inventory.iter().find(|p| p.page_id == old.page_id).unwrap();
    let canonical = inventory
        .iter()
        .find(|p| p.page_id == output.page_id)
        .unwrap();
    assert!(source.source_only);
    assert_eq!(
        source.consolidation_covering_revision_ids,
        vec![output.revision_id.clone()]
    );
    let partial_output = inventory
        .iter()
        .find(|p| p.page_id == partial.outputs[0].page_id)
        .unwrap();
    assert!(!super::super::update_discovery::is_covered_source_output_pair(source, partial_output));
    assert!(super::super::update_discovery::is_covered_source_output_pair(source, canonical));
    let pairs = super::super::update_discovery::candidate_pairs(&inventory);
    assert!(!pairs.iter().any(|(a, b)| {
        [old.page_id.as_str(), output.page_id.as_str()]
            .iter()
            .all(|id| a.page_id == *id || b.page_id == *id)
    }));
    assert!(
        pairs
            .iter()
            .any(|(a, b)| { a.page_id == old.page_id && b.page_id == independent.page_id })
    );
    assert!(
        pairs
            .iter()
            .any(|(a, b)| { a.page_id == old.page_id && b.page_id == partial.outputs[0].page_id })
    );
    // Source-only routing does not archive sources or make them unreadable.
    let read = f
        .client
        .read_pages(ReadPagesRequest {
            page_ids: vec![old.page_id.clone()],
            revision_ids: vec![],
            projections: vec![Projection::Payload],
            max_chars: 4000,
        })
        .await
        .unwrap();
    assert!(
        read[0]
            .revision
            .payload
            .as_ref()
            .unwrap()
            .content
            .contains("original workflow")
    );
    // Loss of canonical coverage must not permanently suppress the source.
    f.client
        .archive_page(pcp_core::ArchivePageRequest {
            page_id: output.page_id.clone(),
            expected_revision_id: output.revision_id.clone(),
            reason: Some("Coverage no longer current".into()),
        })
        .await
        .unwrap();
    let restored = f.client.durable_page_inventory(vec![]).await.unwrap();
    let source = restored.iter().find(|p| p.page_id == old.page_id).unwrap();
    assert!(!source.source_only);
    assert!(source.consolidation_covering_revision_ids.is_empty());
    assert!(!super::super::update_discovery::is_covered_source_output_pair(source, canonical));
    f.close().await;
}
