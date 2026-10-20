use super::topic_convergence::verification;
use super::*;
use crate::maintenance::{MaintenanceReviewStatus, VerificationVerdict};
use pcp_core::{ConsolidatedPageOutput, ConsolidationCoverage};

#[tokio::test]
async fn fusion_native_permission_requires_explicit_apply_opt_in() {
    let f = Fixture::open("fusion-permission").await;
    let mut c = f.config();
    assert!(
        !c.access_session(&f.identity_id)
            .allows(&f.namespace, AccessPermission::ManageLifecycle)
    );
    c.reconciliation.auto_apply_project_consolidation = true;
    assert!(
        c.access_session(&f.identity_id)
            .allows(&f.namespace, AccessPermission::ManageLifecycle)
    );
    c.mode = MaintenanceMode::Observe;
    assert!(
        !c.access_session(&f.identity_id)
            .allows(&f.namespace, AccessPermission::ManageLifecycle)
    );
    f.close().await;
}

#[tokio::test]
#[ignore = "Uses the operator-configured real inference provider and shared review budget"]
async fn configured_model_verifies_real_fusion_and_native_rollback_in_an_isolated_store() {
    let path =
        std::env::var("PCP_MAINTENANCE_VALIDATION_CONFIG").expect("explicit validation config");
    let configured = crate::RuntimeConfig::load(path)
        .unwrap()
        .maintenance
        .unwrap();
    let f = Fixture::open("real-model-fusion-rollback").await;
    let staged = fusion(&f, Arc::new(FakeWorker::new(vec![plan()])), true).await;
    let id = staged.pending_reviews()[0].candidate_id.clone();
    let mut config = staged.config.clone();
    config.worker = configured.worker;
    let worker = crate::maintenance::build_semantic_worker(&config.worker).unwrap();
    let client = EmbeddedPcpClient::shared(f.store.clone(), config.access_session(&f.identity_id));
    let mut real = RuntimeMaintainer::load(client, worker, config)
        .await
        .unwrap();
    let inventory = f.client.durable_page_inventory(vec![]).await.unwrap();
    let mut report = MaintenanceCycleReport::default();
    assert!(
        real.resume_budget_review(&inventory, &mut report)
            .await
            .unwrap()
    );
    let item = real.review_item(&id).unwrap();
    println!(
        "real model review: {}",
        serde_json::to_string(&serde_json::json!({"report":report,"review":item})).unwrap()
    );
    assert_eq!(
        report.consolidations_written, 1,
        "real independent review must pass before publication"
    );
    assert_eq!(item.status, MaintenanceReviewStatus::Accepted);
    assert_eq!(
        f.client
            .durable_page_inventory(vec![])
            .await
            .unwrap()
            .iter()
            .filter(|p| p.source_only)
            .count(),
        2
    );
    real.rollback_consolidation_review(&id, f.client.as_ref())
        .await
        .unwrap();
    let restored = f.client.durable_page_inventory(vec![]).await.unwrap();
    assert_eq!(restored.len(), 2);
    assert!(restored.iter().all(|p| !p.source_only));
    println!("real model fusion and native rollback verified; original source heads retained");
    f.close().await;
}

async fn fusion(f: &Fixture, worker: Arc<FakeWorker>, auto: bool) -> RuntimeMaintainer {
    let a = f
        .client
        .write_page(f.page("The project used plan A; issue B remains open.", "auto:a"))
        .await
        .unwrap();
    let mut b = f.page(
        "The project now uses plan A2; issue B remains open.",
        "auto:b",
    );
    b.provenance = vec![ProvenanceEvent {
        operation: "derive".into(),
        actor: Actor {
            actor_type: ActorType::Tool,
            actor_id: "test".into(),
        },
        timestamp: "2026-10-20T00:00:00Z".into(),
        input_revision_ids: vec![a.revision_id.clone()],
        tool_or_model: None,
        reason: None,
    }];
    f.client.write_page(b).await.unwrap();
    let mut c = f.config();
    c.summary.enabled = false;
    c.packing.enabled = false;
    c.relation.enabled = false;
    c.retention.enabled = false;
    c.reconciliation.discover_updates = true;
    c.reconciliation.auto_apply_project_consolidation = auto;
    let mut m = RuntimeMaintainer::for_test(f.client.clone(), worker, c);
    m.run_once().await.unwrap();
    m
}

fn plan() -> MaintenanceWorkerResponse {
    MaintenanceWorkerResponse::ConsolidatePages {
        rationale: "One project decision with its retained unresolved issue.".into(),
        outputs: vec![ConsolidatedPageOutput {
            title: "Project plan and open issue".into(),
            content:
                "The project previously used plan A, then changed to A2. Issue B remains open."
                    .into(),
            source_indexes: vec![0, 1],
        }],
        coverage: (0..2)
            .map(|source_index| ConsolidationCoverage {
                source_index,
                output_indexes: vec![0],
                explanation: "The old and new plans and the unresolved issue are retained.".into(),
                complete: true,
            })
            .collect(),
    }
}

#[tokio::test]
async fn fusion_is_independently_reviewed_then_reversible_without_source_edits() {
    let f = Fixture::open("auto-fusion").await;
    let worker = Arc::new(FakeWorker::new(vec![
        plan(),
        verification(VerificationVerdict::Approve),
    ]));
    let mut m = fusion(&f, worker.clone(), true).await;
    let id = m.pending_reviews()[0].candidate_id.clone();
    let inventory = f.client.durable_page_inventory(vec![]).await.unwrap();
    let original = inventory
        .iter()
        .map(|p| (p.page_id.clone(), p.revision_id.clone()))
        .collect::<Vec<_>>();
    let mut report = MaintenanceCycleReport::default();
    assert!(
        m.resume_budget_review(&inventory, &mut report)
            .await
            .unwrap()
    );
    assert_eq!(report.consolidations_written, 1);
    assert_eq!(worker.request_count(), 2);
    let MaintenanceWorkerRequest::VerifyMaintenance {
        kind,
        pages,
        content,
        ..
    } = &worker.requests()[1]
    else {
        panic!("missing independent review")
    };
    assert_eq!(kind, "consolidation");
    assert_eq!(pages.len(), 2);
    assert!(content.contains("coverage"));
    assert!(
        pages
            .iter()
            .all(|p| p.content.as_ref().unwrap().contains("issue B"))
    );
    let after = f.client.durable_page_inventory(vec![]).await.unwrap();
    assert_eq!(after.iter().filter(|p| p.source_only).count(), 2);
    let covered_sources = after.iter().filter(|p| p.source_only).collect::<Vec<_>>();
    assert!(
        crate::maintenance::update_discovery::update_pair_is_covered(
            covered_sources[0],
            covered_sources[1]
        )
    );
    for (page, revision) in &original {
        assert!(
            after
                .iter()
                .any(|p| &p.page_id == page && &p.revision_id == revision)
        );
    }
    let item = m.review_item(&id).unwrap();
    assert_eq!(item.status, MaintenanceReviewStatus::Accepted);
    let MaintenanceReviewPayload::Reconciliation(c) = item.payload else {
        panic!()
    };
    assert_eq!(c.consolidation_result.unwrap().outputs.len(), 1);
    m.rollback_consolidation_review(&id, f.client.as_ref())
        .await
        .unwrap();
    m.rollback_consolidation_review(&id, f.client.as_ref())
        .await
        .unwrap();
    let restored = f.client.durable_page_inventory(vec![]).await.unwrap();
    assert!(original.iter().all(|(page, revision)| {
        restored
            .iter()
            .any(|p| &p.page_id == page && &p.revision_id == revision && !p.source_only)
    }));
    f.close().await;
}

#[tokio::test]
async fn automatic_review_runs_with_application_disabled_and_stops_after_approval() {
    let f = Fixture::open("review-without-apply").await;
    let worker = Arc::new(FakeWorker::new(vec![
        plan(),
        verification(VerificationVerdict::Approve),
    ]));
    let mut m = fusion(&f, worker.clone(), false).await;
    let inventory = f.client.durable_page_inventory(vec![]).await.unwrap();
    assert_eq!(
        m.routed_reviews(&m.config).await.unwrap()[0]
            .queue
            .as_ref()
            .unwrap()
            .state,
        "automatic"
    );
    let mut report = MaintenanceCycleReport::default();
    m.resume_budget_review(&inventory, &mut report)
        .await
        .unwrap();
    assert_eq!(worker.request_count(), 2);
    assert_eq!(report.consolidations_written, 0);
    assert_eq!(
        m.routed_reviews(&m.config).await.unwrap()[0]
            .queue
            .as_ref()
            .unwrap()
            .state,
        "paused"
    );
    assert!(
        !m.resume_budget_review(&inventory, &mut report)
            .await
            .unwrap()
    );
    assert_eq!(worker.request_count(), 2);
    assert_eq!(f.client.page_count(vec![]).await.unwrap(), 2);
    f.close().await;
}

#[tokio::test]
async fn incomplete_coverage_and_reviewer_repairs_never_publish_automatically() {
    for partial in [false, true] {
        let f = Fixture::open("fusion-incomplete").await;
        let mut p = plan();
        if partial {
            if let MaintenanceWorkerResponse::ConsolidatePages { coverage, .. } = &mut p {
                coverage[0].complete = false;
            }
        }
        let mut approval = verification(VerificationVerdict::Approve);
        if let MaintenanceWorkerResponse::VerifyMaintenance { assessment } = &mut approval {
            assessment.revision = Some(crate::maintenance::VerifiedMaintenanceRevision {
                title: "Unreviewed".into(),
                content: "A changed plan was not independently verified".into(),
            });
        }
        let worker = Arc::new(FakeWorker::new(vec![p, approval]));
        let mut m = fusion(&f, worker.clone(), true).await;
        let inventory = f.client.durable_page_inventory(vec![]).await.unwrap();
        let mut report = MaintenanceCycleReport::default();
        m.resume_budget_review(&inventory, &mut report)
            .await
            .unwrap();
        assert_eq!(report.consolidations_written, 0);
        assert_eq!(f.client.page_count(vec![]).await.unwrap(), 2);
        assert_eq!(
            m.routed_reviews(&m.config).await.unwrap()[0]
                .queue
                .as_ref()
                .unwrap()
                .state,
            "human"
        );
        assert_eq!(worker.request_count(), if partial { 1 } else { 2 });
        f.close().await;
    }
}

#[tokio::test]
async fn store_rechecks_pending_feedback_and_live_standing_inside_fusion_transaction() {
    let f = Fixture::open("fusion-contested").await;
    let worker = Arc::new(FakeWorker::new(vec![
        plan(),
        verification(VerificationVerdict::Approve),
    ]));
    let mut m = fusion(&f, worker, true).await;
    let item = m.pending_reviews()[0].clone();
    let MaintenanceReviewPayload::Reconciliation(c) = item.payload else {
        panic!()
    };
    let feedback = f
        .client
        .submit_feedback(SubmitFeedbackRequest {
            namespace: f.namespace.clone(),
            kind: FeedbackKind::Correction,
            authority: FeedbackAuthority::SubjectOwner,
            payload: PagePayload {
                media_type: "text/plain".into(),
                content: "The plan still needs a user decision.".into(),
            },
            observed_at: None,
            source_refs: vec![],
            challenged_revision_ids: vec![c.target.revision_id.clone()],
            used_revision_ids: vec![],
            evidence_revision_ids: vec![],
            response_ref: None,
            external_event_id: None,
        })
        .await
        .unwrap();
    let inventory = f.client.durable_page_inventory(vec![]).await.unwrap();
    let mut report = MaintenanceCycleReport::default();
    m.resume_budget_review(&inventory, &mut report)
        .await
        .unwrap();
    assert_eq!(report.consolidations_written, 0);
    assert!(
        f.client
            .pending_feedback(vec![], 32)
            .await
            .unwrap()
            .iter()
            .any(|s| s.feedback_revision_id == feedback.feedback_revision_id)
    );
    assert_eq!(
        m.routed_reviews(&m.config).await.unwrap()[0]
            .queue
            .as_ref()
            .unwrap()
            .state,
        "human"
    );
    assert!(
        !f.client
            .durable_page_inventory(vec![])
            .await
            .unwrap()
            .iter()
            .any(|p| p.source_only)
    );
    f.close().await;
}

#[tokio::test]
async fn committed_fusion_recovers_lost_ledger_receipt_without_reinference_or_duplicate_pages() {
    let f = Fixture::open("fusion-recovery").await;
    let worker = Arc::new(FakeWorker::new(vec![
        plan(),
        verification(VerificationVerdict::Approve),
    ]));
    let mut m = fusion(&f, worker.clone(), true).await;
    let mut pending = m.pending_reviews()[0].clone();
    let inventory = f.client.durable_page_inventory(vec![]).await.unwrap();
    m.resume_budget_review(&inventory, &mut MaintenanceCycleReport::default())
        .await
        .unwrap();
    let MaintenanceReviewPayload::Reconciliation(c) = &mut pending.payload else {
        panic!()
    };
    let MaintenanceWorkerResponse::VerifyMaintenance { mut assessment } =
        verification(VerificationVerdict::Approve)
    else {
        panic!()
    };
    assessment.review_state = Some("publishing".into());
    c.verification = Some(assessment);
    std::fs::write(
        &m.config.state_path,
        serde_json::to_vec(
            &serde_json::json!({"reviewItems":{pending.candidate_id.clone():pending}}),
        )
        .unwrap(),
    )
    .unwrap();
    let mut recovered = RuntimeMaintainer::load(f.client.clone(), worker.clone(), m.config.clone())
        .await
        .unwrap();
    let inventory = f.client.durable_page_inventory(vec![]).await.unwrap();
    recovered
        .resume_budget_review(&inventory, &mut MaintenanceCycleReport::default())
        .await
        .unwrap();
    assert!(recovered.pending_reviews().is_empty());
    assert_eq!(worker.request_count(), 2);
    assert_eq!(f.client.page_count(vec![]).await.unwrap(), 3);
    f.close().await;
}
