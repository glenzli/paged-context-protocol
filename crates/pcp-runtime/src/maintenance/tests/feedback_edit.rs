use super::super::ledger::{MaintenanceLedger, MaintenanceWakeReason};
use super::super::{
    MaintenanceConfig, MaintenanceReviewPayload, MaintenanceReviewStatus, MaintenanceWorkerRequest,
    MaintenanceWorkerResponse, RuntimeMaintainer, SemanticMaintenanceWorker,
};
use super::{FakeWorker, Fixture};
use pcp_core::{
    FeedbackAuthority, FeedbackKind, FeedbackSubmission, PagePayload, Projection, ReadPagesRequest,
    ReconciliationDisposition, RepairPageRequest, SubmitFeedbackRequest,
};
use std::sync::Arc;

fn config(f: &Fixture) -> MaintenanceConfig {
    let mut config = f.config();
    config.summary.enabled = false;
    config.packing.enabled = false;
    config.relation.enabled = false;
    config.retention.enabled = false;
    config.max_jobs_per_cycle = 1;
    config
}

fn repair(signal: &FeedbackSubmission) -> RepairPageRequest {
    RepairPageRequest {
        page_id: signal.feedback_page_id.clone(),
        expected_revision_id: signal.feedback_revision_id.clone(),
        payload: Some(PagePayload {
            media_type: "text/plain".into(),
            content: "Corrected feedback without extra instructions.".into(),
        }),
        source_refs: Vec::new(),
        facets: None,
        based_on_revision_ids: Vec::new(),
        reason: "Operator correction".into(),
        tool_or_model: Some("pcp-console".into()),
        idempotency_key: None,
    }
}

async fn feedback(f: &Fixture) -> (FeedbackSubmission, MaintenanceWorkerResponse) {
    let target = f
        .client
        .write_page(f.page("Original icon description.", "old-icon"))
        .await
        .unwrap();
    let signal = f
        .client
        .submit_feedback(SubmitFeedbackRequest {
            namespace: f.namespace.clone(),
            kind: FeedbackKind::Correction,
            authority: FeedbackAuthority::SubjectOwner,
            payload: PagePayload {
                media_type: "text/plain".into(),
                content: "Correction with extra instructions.".into(),
            },
            observed_at: None,
            source_refs: Vec::new(),
            challenged_revision_ids: vec![target.revision_id.clone()],
            used_revision_ids: Vec::new(),
            evidence_revision_ids: Vec::new(),
            response_ref: None,
            external_event_id: None,
        })
        .await
        .unwrap();
    let response = MaintenanceWorkerResponse::ReconcileFeedback {
        target_revision_id: target.revision_id,
        disposition: ReconciliationDisposition::Retracted,
        rationale: "Original claim is withdrawn.".into(),
        scope: None,
        replacement_revision_id: None,
    };
    (signal, response)
}

#[tokio::test]
async fn feedback_edit_invalidates_persisted_and_snoozed_reviews_without_inference() {
    for snoozed in [false, true] {
        let f = Fixture::open("feedback-edit-inbox").await;
        let (signal, response) = feedback(&f).await;
        let worker = Arc::new(FakeWorker::new(vec![response.clone(), response]));
        let mut maintainer =
            RuntimeMaintainer::for_test(f.client.clone(), worker.clone(), config(&f));
        maintainer.run_once().await.unwrap();
        let candidate_id = maintainer.pending_reviews()[0].candidate_id.clone();
        if snoozed {
            maintainer
                .resolve_review(&candidate_id, MaintenanceReviewStatus::Deferred)
                .await
                .unwrap();
        }
        let edited = f.client.repair_page(repair(&signal)).await.unwrap();
        // Console loads the same persisted ledger; opening the inbox must not
        // require a model call or leave stale staged approvals actionable.
        let mut refreshed = RuntimeMaintainer::load(f.client.clone(), worker.clone(), config(&f))
            .await
            .unwrap();
        assert!(refreshed.pending_reviews().is_empty());
        assert_eq!(
            refreshed.review_item(&candidate_id).unwrap().status,
            MaintenanceReviewStatus::Stale
        );
        assert_eq!(worker.request_count(), 1);
        // Even an old, already-open maintainer is stopped at the Store boundary.
        let error = maintainer
            .approve_reconciliation_review(&candidate_id)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("stale"));
        // New feedback identity bypasses the old analysis retry timer.
        let report = refreshed.run_once().await.unwrap();
        assert_eq!(report.reconciliations_proposed, 1);
        let reviews = refreshed.pending_reviews();
        assert_eq!(reviews.len(), 1);
        assert_ne!(reviews[0].candidate_id, candidate_id);
        let MaintenanceReviewPayload::Reconciliation(candidate) = &reviews[0].payload else {
            panic!("wrong review kind")
        };
        assert_eq!(
            candidate.signal.as_ref().unwrap().feedback_revision_id,
            edited.revision_id
        );
        assert_eq!(
            candidate.feedback.as_ref().unwrap().content.as_deref(),
            Some("Corrected feedback without extra instructions.")
        );
        let requests = worker.requests();
        let MaintenanceWorkerRequest::ReconcileFeedback { feedback, .. } = &requests[1] else {
            panic!("wrong request")
        };
        assert_eq!(feedback.revision_id, edited.revision_id);
        refreshed
            .approve_reconciliation_review(&reviews[0].candidate_id)
            .await
            .unwrap();
        f.close().await;
    }
}

struct EditingWorker {
    client: Arc<dyn pcp_client::PcpApi>,
    edit: RepairPageRequest,
    response: MaintenanceWorkerResponse,
}

#[async_trait::async_trait]
impl SemanticMaintenanceWorker for EditingWorker {
    async fn evaluate(
        &self,
        _: MaintenanceWorkerRequest,
    ) -> anyhow::Result<MaintenanceWorkerResponse> {
        self.client.repair_page(self.edit.clone()).await?;
        Ok(self.response.clone())
    }
}

#[tokio::test]
async fn feedback_edited_during_inference_is_neither_queued_nor_auto_applied() {
    // Cover both reviewed high-impact and unattended low-impact outcomes.
    for disposition in [
        ReconciliationDisposition::Retracted,
        ReconciliationDisposition::Disputed,
    ] {
        let f = Fixture::open("feedback-edit-race").await;
        let (signal, mut response) = feedback(&f).await;
        if let MaintenanceWorkerResponse::ReconcileFeedback {
            disposition: decision,
            ..
        } = &mut response
        {
            *decision = disposition;
        }
        let worker = Arc::new(EditingWorker {
            client: f.client.clone(),
            edit: repair(&signal),
            response,
        });
        let mut maintainer = RuntimeMaintainer::for_test(f.client.clone(), worker, config(&f));
        let report = maintainer.run_once().await.unwrap();
        assert_eq!(report.worker_calls, 1);
        assert_eq!(report.reconciliations_proposed, 0);
        assert_eq!(report.reconciliations_committed, 0);
        assert!(maintainer.pending_reviews().is_empty());
        let pending = f.client.pending_feedback(Vec::new(), 10).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_ne!(pending[0].feedback_revision_id, signal.feedback_revision_id);
        let target = f
            .client
            .read_pages(ReadPagesRequest {
                page_ids: Vec::new(),
                revision_ids: signal.challenged_revision_ids,
                projections: vec![Projection::Validity],
                max_chars: 4_000,
            })
            .await
            .unwrap();
        assert!(target[0].validity.is_none());
        f.close().await;
    }
}

#[tokio::test]
async fn feedback_inbox_refresh_preserves_concurrent_scheduler_state() {
    let f = Fixture::open("feedback-edit-cadence").await;
    let (signal, response) = feedback(&f).await;
    let worker = Arc::new(FakeWorker::new(vec![response]));
    let config = config(&f);
    let mut maintainer = RuntimeMaintainer::for_test(f.client.clone(), worker, config.clone());
    maintainer.run_once().await.unwrap();
    let candidate_id = maintainer.pending_reviews()[0].candidate_id.clone();
    // The Console has an older ledger snapshot while a scheduler begins work.
    let mut concurrent = MaintenanceLedger::load(&config.state_path).await.unwrap();
    concurrent.start_scheduled_cycle(MaintenanceWakeReason::ExternalWrite);
    concurrent.record("concurrent-work".into(), "running", 900);
    concurrent.save(&config.state_path).await.unwrap();
    let before: serde_json::Value =
        serde_json::from_slice(&tokio::fs::read(&config.state_path).await.unwrap()).unwrap();
    f.client.repair_page(repair(&signal)).await.unwrap();
    maintainer.refresh_feedback_reviews().await.unwrap();
    let after: serde_json::Value =
        serde_json::from_slice(&tokio::fs::read(&config.state_path).await.unwrap()).unwrap();
    assert_eq!(before["scheduler"], after["scheduler"]);
    assert_eq!(before["entries"], after["entries"]);
    assert_eq!(before["writeTrigger"], after["writeTrigger"]);
    assert_eq!(after["reviewItems"][&candidate_id]["status"], "stale");
    f.close().await;
}
