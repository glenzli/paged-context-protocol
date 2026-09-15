use super::*;
use crate::maintenance::{
    MaintenanceReviewOrigin, MaintenanceReviewPayload, MaintenanceReviewStatus,
    MaintenanceVerification, VerificationVerdict,
};

pub(super) fn verification(verdict: VerificationVerdict) -> MaintenanceWorkerResponse {
    MaintenanceWorkerResponse::VerifyMaintenance {
        assessment: MaintenanceVerification {
            verdict,
            reason: "The full sources support this bounded synthesis.".into(),
            added_information: "Connects source evidence to the documented review boundary.".into(),
            preserved_boundaries:
                "Keeps attribution and the distinction between proposal and approval.".into(),
            concerns: vec![],
            revision: None,
            requires_user_input: false,
            review_state: None,
            review_steps: vec![],
        },
    }
}

async fn sources(f: &Fixture) -> Vec<pcp_store::DurablePageInventoryItem> {
    for i in 0..4 {
        f.client.write_page(f.page(&format!("PCP maintenance review preserves source provenance and explicit approval boundaries, aspect {i}."), &format!("source:{i}"))).await.unwrap();
    }
    f.client.durable_page_inventory(vec![]).await.unwrap()
}

fn config(f: &Fixture) -> MaintenanceConfig {
    let mut c = f.config();
    c.summary.enabled = false;
    c.packing.enabled = false;
    c.relation.enabled = false;
    c.retention.enabled = false;
    c.topic.auto_apply = true;
    c.max_jobs_per_cycle = 1;
    c
}

fn proposal(pages: &[pcp_store::DurablePageInventoryItem]) -> MaintenanceWorkerResponse {
    MaintenanceWorkerResponse::ExtractTopic {
        page_ids: pages.iter().map(|p| p.page_id.clone()).collect(),
        title: "PCP source provenance and review".into(),
        content: "PCP maintenance connects source provenance with explicit review. Source records remain unchanged; a proposal is separate from an approved decision. The topic provides a retrieval entry point while preserving this boundary.".into(),
        reason: "Complementary source evidence establishes one durable review boundary.".into(), refresh_topic_page_id: None,
    }
}

#[tokio::test]
async fn invalid_topic_selection_repairs_once_then_isolates_without_failing_cycle() {
    let f = Fixture::open("topic-invalid-isolation").await;
    let pages = sources(&f).await;
    let mut bad = proposal(&pages);
    if let MaintenanceWorkerResponse::ExtractTopic { page_ids, .. } = &mut bad {
        page_ids[0] = "pg_not_offered".into();
    }
    let worker = Arc::new(FakeWorker::new(vec![bad.clone(), bad]));
    let c = config(&f);
    let mut m = RuntimeMaintainer::for_test(f.client.clone(), worker.clone(), c.clone());
    let report = m.run_convergence_once(1).await.unwrap();
    assert_eq!(report.isolated_jobs, 1);
    assert_eq!(worker.request_count(), 2);
    assert!(matches!(
        &worker.requests()[1],
        MaintenanceWorkerRequest::ExtractTopic {
            correction: Some(_),
            ..
        }
    ));
    let status = RuntimeMaintainer::automation_status(&c).await.unwrap();
    assert_eq!(status.job_issues.len(), 1);
    assert!(m.pending_reviews().is_empty());
    // The same evidence window is now cooling down; no third generation call.
    m.run_convergence_once(1).await.unwrap();
    assert_eq!(worker.request_count(), 2);
    f.close().await;
}

#[tokio::test]
async fn rejected_evidence_survives_rewording_but_changed_revision_can_be_reviewed() {
    let f = Fixture::open("topic-evidence-identity").await;
    let pages = sources(&f).await;
    let worker = Arc::new(FakeWorker::new(vec![]));
    let mut m = RuntimeMaintainer::for_test(f.client.clone(), worker, config(&f));
    let candidate = m
        .topic_from_response(&pages, &pages, &[], proposal(&pages))
        .unwrap();
    let id = m.ledger.enqueue_review(
        MaintenanceReviewPayload::Topic(candidate.clone()),
        MaintenanceReviewOrigin::Manual,
        "review".into(),
        1,
        false,
    );
    m.ledger
        .resolve_review(&id, MaintenanceReviewStatus::Rejected)
        .unwrap();
    m.ledger
        .set_review_reason(&id, "no_increment: existing topic already covers it".into())
        .unwrap();
    let mut paraphrase = candidate;
    paraphrase.title = "Reworded review boundary".into();
    paraphrase.content.push_str(" Reworded.");
    assert!(m.ledger.topic_evidence_reviewed(&paraphrase));
    assert!(
        m.ledger.topic_feedback(&pages)[0]
            .reason
            .starts_with("no_increment:")
    );
    paraphrase.pages[0].revision_id = "rev_new_evidence".into();
    assert!(!m.ledger.topic_evidence_reviewed(&paraphrase));
    f.close().await;
}

#[tokio::test]
async fn topic_backpressure_prevents_generation_and_no_change_never_writes() {
    let f = Fixture::open("topic-no-increment").await;
    let pages = sources(&f).await;
    let worker = Arc::new(FakeWorker::new(vec![
        proposal(&pages),
        verification(VerificationVerdict::NoChange),
    ]));
    let mut c = config(&f);
    c.topic.max_pending_reviews = 1;
    let mut m = RuntimeMaintainer::for_test(f.client.clone(), worker.clone(), c);
    let report = m.run_convergence_once(1).await.unwrap();
    assert_eq!(report.unchanged_topics_skipped, 1);
    assert_eq!(report.topics_written, 0);
    assert!(m.pending_reviews().is_empty());
    assert_eq!(worker.request_count(), 2);
    let mut candidate = m
        .topic_from_response(&pages, &pages, &[], proposal(&pages))
        .unwrap();
    candidate.pages[0].revision_id = "different".into();
    candidate.candidate_id = "mtc_different".into();
    m.ledger.enqueue_review(
        MaintenanceReviewPayload::Topic(candidate),
        MaintenanceReviewOrigin::Manual,
        "uncertain".into(),
        1,
        false,
    );
    let mut report = crate::maintenance::MaintenanceCycleReport::default();
    assert!(
        !m.run_topic_review_job(&pages, &mut report, MaintenanceReviewOrigin::Manual)
            .await
            .unwrap()
    );
    assert!(report.topic_backlog_paused);
    assert_eq!(worker.request_count(), 2);
    f.close().await;
}

#[tokio::test]
async fn ordinary_relation_requires_an_independent_approval() {
    for (label, verdict, committed) in [
        ("approved", VerificationVerdict::Approve, 1),
        ("uncertain", VerificationVerdict::NeedsReview, 0),
    ] {
        let f = Fixture::open(&format!("relation-verified-{label}")).await;
        let pages = sources(&f).await;
        let worker = Arc::new(FakeWorker::new(vec![
            MaintenanceWorkerResponse::Relate {
                page_ids: [pages[0].page_id.clone(), pages[1].page_id.clone()],
                reason: "These sources describe complementary provenance and review boundaries."
                    .into(),
            },
            verification(verdict),
        ]));
        let mut c = config(&f);
        c.relation.enabled = true;
        c.relation.auto_apply_verified = true;
        c.topic.enabled = false;
        let mut m = RuntimeMaintainer::for_test(f.client.clone(), worker.clone(), c);
        let report = m.run_convergence_once(1).await.unwrap();
        assert_eq!(report.relations_committed, committed);
        assert_eq!(report.relations_proposed, 1 - committed);
        let requests = worker.requests();
        let MaintenanceWorkerRequest::VerifyMaintenance {
            pages: evidence,
            kind,
            ..
        } = &requests[1]
        else {
            panic!("expected independent verification")
        };
        assert_eq!(kind, "relation");
        assert_eq!(evidence.len(), 2);
        assert!(evidence.iter().all(|p| {
            p.content
                .as_deref()
                .unwrap()
                .contains("explicit approval boundaries")
        }));
        f.close().await;
    }
}

#[tokio::test]
async fn refresh_preserves_all_existing_source_references() {
    let f = Fixture::open("topic-refresh-union").await;
    let pages = sources(&f).await;
    let worker = Arc::new(FakeWorker::new(vec![]));
    let m = RuntimeMaintainer::for_test(f.client.clone(), worker, config(&f));
    let mut old = pages[0].clone();
    old.page_id = "pg_existing_topic".into();
    old.revision_id = "rev_existing_topic".into();
    old.kind = "topic_summary".into();
    old.topic_source_page_ids = pages[..3].iter().map(|p| p.page_id.clone()).collect();
    let existing = vec![crate::maintenance::worker::ExistingTopicPage {
        page_id: old.page_id.clone(),
        revision_id: old.revision_id.clone(),
        title: "Old review boundary".into(),
        routing_text: "Keep old attribution and qualifications.".into(),
        source_page_ids: old.topic_source_page_ids.clone(),
    }];
    let mut inventory = pages.clone();
    inventory.push(old);
    let mut response = proposal(&pages[2..]);
    if let MaintenanceWorkerResponse::ExtractTopic {
        refresh_topic_page_id,
        ..
    } = &mut response
    {
        *refresh_topic_page_id = Some("pg_existing_topic".into());
    }
    let candidate = m
        .topic_from_response(&inventory, &pages[2..], &existing, response)
        .unwrap();
    assert_eq!(candidate.pages.len(), 4);
    assert!(pages.iter().all(|p| {
        candidate
            .pages
            .iter()
            .any(|c| c.page_id == p.page_id && c.revision_id == p.revision_id)
    }));
    assert_eq!(
        candidate.refresh_target.unwrap().revision_id,
        "rev_existing_topic"
    );
    f.close().await;
}

#[tokio::test]
async fn verified_repair_changes_candidate_identity_and_applies_only_repaired_content() {
    let f = Fixture::open("topic-verified-repair").await;
    let pages = sources(&f).await;
    let mut assessment = verification(VerificationVerdict::Approve);
    let repaired = "The repaired retrieval entry preserves source provenance and distinguishes a proposed change from an approved decision. It explicitly retains the historical context and the user's authority over acceptance.";
    if let MaintenanceWorkerResponse::VerifyMaintenance { assessment } = &mut assessment {
        assessment.revision = Some(crate::maintenance::worker::VerifiedMaintenanceRevision {
            title: "Repaired source provenance".into(),
            content: repaired.into(),
        });
    }
    let worker = Arc::new(FakeWorker::new(vec![proposal(&pages), assessment]));
    let mut c = config(&f);
    c.topic.minimum_pages = 2;
    c.topic.minimum_total_chars = 1;
    let mut m = RuntimeMaintainer::for_test(f.client.clone(), worker, c);
    let original = m
        .topic_from_response(&pages, &pages, &[], proposal(&pages))
        .unwrap();
    let report = m.run_convergence_once(1).await.unwrap();
    assert_eq!(report.topics_written, 1);
    let inventory = f.client.durable_page_inventory(vec![]).await.unwrap();
    let topic = inventory
        .iter()
        .find(|p| p.kind == "topic_summary")
        .unwrap();
    let detail = m
        .read_detail_pages(vec![topic.revision_id.clone()], 64000)
        .await
        .unwrap();
    assert!(detail[0].content.as_ref().unwrap().contains(repaired));
    assert!(m.ledger.review_item(&original.candidate_id).is_none());
    f.close().await;
}

#[tokio::test]
async fn late_topic_application_respects_persisted_human_rejection() {
    let f = Fixture::open("topic-late-rejection").await;
    let pages = sources(&f).await;
    let mut m = RuntimeMaintainer::for_test(
        f.client.clone(),
        Arc::new(FakeWorker::new(vec![])),
        config(&f),
    );
    let candidate = m
        .topic_from_response(&pages, &pages, &[], proposal(&pages))
        .unwrap();
    m.ledger.enqueue_review(
        MaintenanceReviewPayload::Topic(candidate.clone()),
        MaintenanceReviewOrigin::Manual,
        "review".into(),
        1,
        false,
    );
    m.ledger.save(&m.config.state_path).await.unwrap();
    let mut other = RuntimeMaintainer::for_test(
        f.client.clone(),
        Arc::new(FakeWorker::new(vec![])),
        config(&f),
    );
    other
        .resolve_review(&candidate.candidate_id, MaintenanceReviewStatus::Rejected)
        .await
        .unwrap();
    let result = m
        .apply_topic_candidate(crate::maintenance::ApplyMaintenanceTopicRequest {
            candidate_id: candidate.candidate_id,
            title: candidate.title,
            content: candidate.content,
            pages: candidate
                .pages
                .into_iter()
                .map(|p| pcp_core::PageRevisionRef {
                    page_id: p.page_id,
                    revision_id: p.revision_id,
                })
                .collect(),
            refresh_target: None,
        })
        .await;
    assert!(result.unwrap_err().to_string().contains("already decided"));
    assert!(
        !f.client
            .durable_page_inventory(vec![])
            .await
            .unwrap()
            .iter()
            .any(|p| p.kind == "topic_summary")
    );
    f.close().await;
}

#[tokio::test]
async fn budget_wait_resumes_exact_proposal_and_stale_sources_skip_inference() {
    for stale in [false, true] {
        let f = Fixture::open(&format!("budget-resume-{stale}")).await;
        let pages = sources(&f).await;
        let worker = Arc::new(FakeWorker::new(if stale {
            vec![]
        } else {
            vec![verification(VerificationVerdict::Approve)]
        }));
        let mut c = config(&f);
        c.topic.minimum_pages = 2;
        c.topic.minimum_total_chars = 1;
        c.worker=serde_json::from_value(serde_json::json!({"provider":"infer_runtime","credential_file":"/tmp/unused.token","actor_id":"model:test","review_budget":{"enabled":true}})).unwrap();
        let mut m = RuntimeMaintainer::for_test(f.client.clone(), worker.clone(), c);
        let mut candidate = m
            .topic_from_response(&pages, &pages, &[], proposal(&pages))
            .unwrap();
        let MaintenanceWorkerResponse::VerifyMaintenance { mut assessment } =
            verification(VerificationVerdict::NeedsReview)
        else {
            panic!()
        };
        assessment.review_state = Some("waiting_budget".into());
        candidate.verification = Some(assessment);
        if stale {
            candidate.pages[0].revision_id = "changed-evidence".into();
        }
        let id = m.ledger.enqueue_review(
            MaintenanceReviewPayload::Topic(candidate),
            MaintenanceReviewOrigin::Automatic,
            "budget wait".into(),
            2,
            true,
        );
        m.ledger.save(&m.config.state_path).await.unwrap();
        let report = m.run_convergence_once(1).await.unwrap();
        assert_eq!(worker.request_count(), usize::from(!stale));
        assert_eq!(report.topics_written, u32::from(!stale));
        assert!(
            worker
                .requests()
                .iter()
                .all(|r| matches!(r, MaintenanceWorkerRequest::VerifyMaintenance { .. }))
        );
        assert_eq!(
            m.ledger.review_item(&id).unwrap().status,
            if stale {
                MaintenanceReviewStatus::Stale
            } else {
                MaintenanceReviewStatus::Accepted
            }
        );
        f.close().await;
    }
}

fn enable_review(c: &mut MaintenanceConfig) {
    c.worker = serde_json::from_value(serde_json::json!({"provider":"infer_runtime","credential_file":"/tmp/unused.token","actor_id":"model:test","review_budget":{"enabled":true}})).unwrap();
}

fn approved() -> MaintenanceVerification {
    let MaintenanceWorkerResponse::VerifyMaintenance { assessment } =
        verification(VerificationVerdict::Approve)
    else {
        unreachable!()
    };
    assessment
}

#[tokio::test]
async fn approved_small_topics_wait_without_human_backpressure_or_repeat_calls() {
    let f = Fixture::open("approved-small-queue").await;
    let pages = sources(&f).await;
    let worker = Arc::new(FakeWorker::new(vec![]));
    let mut c = config(&f);
    enable_review(&mut c);
    let mut m = RuntimeMaintainer::for_test(f.client.clone(), worker.clone(), c.clone());
    let mut candidate = m
        .topic_from_response(&pages, &pages, &[], proposal(&pages[..2]))
        .unwrap();
    candidate.verification = Some(approved());
    let id = m.ledger.enqueue_review(
        MaintenanceReviewPayload::Topic(candidate),
        MaintenanceReviewOrigin::Automatic,
        "approved".into(),
        2,
        false,
    );
    m.ledger.save(&c.state_path).await.unwrap();
    let queue = m.routed_reviews(&c).await.unwrap();
    let route = queue[0].queue.as_ref().unwrap();
    assert_eq!(route.state, "waiting_accumulation");
    assert_eq!(route.accumulation.as_ref().unwrap().source_pages, 2);
    assert_eq!(m.ledger.topic_pending_count(), 0);
    assert!(
        !m.resume_budget_review(&pages, &mut MaintenanceCycleReport::default())
            .await
            .unwrap()
    );
    assert_eq!(worker.request_count(), 0);
    assert_eq!(
        m.ledger.review_item(&id).unwrap().status,
        MaintenanceReviewStatus::Pending
    );
    f.close().await;
}

struct ResumeOnlyWorker(Mutex<u32>);
#[async_trait]
impl SemanticMaintenanceWorker for ResumeOnlyWorker {
    async fn evaluate(&self, _: MaintenanceWorkerRequest) -> Result<MaintenanceWorkerResponse> {
        anyhow::bail!("baseline must not be generated again")
    }
    async fn review_existing_with_usage(
        &self,
        request: MaintenanceWorkerRequest,
        baseline: MaintenanceVerification,
    ) -> Result<crate::maintenance::MaintenanceWorkerOutcome> {
        assert!(matches!(
            request,
            MaintenanceWorkerRequest::VerifyMaintenance { .. }
        ));
        assert!(baseline.requires_user_input);
        *self.0.lock().unwrap() += 1;
        Ok(crate::maintenance::MaintenanceWorkerOutcome {
            response: verification(VerificationVerdict::NoChange),
            usage: None,
            model_attempts: 1,
            escalated: true,
        })
    }
}

#[tokio::test]
async fn old_baseline_uncertainty_resumes_once_without_regenerating_it() {
    let f = Fixture::open("resume-baseline-once").await;
    let pages = sources(&f).await;
    let worker = Arc::new(ResumeOnlyWorker(Mutex::new(0)));
    let mut c = config(&f);
    enable_review(&mut c);
    let mut m = RuntimeMaintainer::for_test(f.client.clone(), worker.clone(), c.clone());
    let mut candidate = m
        .topic_from_response(&pages, &pages, &[], proposal(&pages))
        .unwrap();
    let mut v = approved();
    v.verdict = VerificationVerdict::NeedsReview;
    v.requires_user_input = true;
    candidate.verification = Some(v);
    let id = m.ledger.enqueue_review(
        MaintenanceReviewPayload::Topic(candidate),
        MaintenanceReviewOrigin::Automatic,
        "choose duplicate target".into(),
        2,
        false,
    );
    m.ledger.save(&c.state_path).await.unwrap();
    assert_eq!(
        m.routed_reviews(&c).await.unwrap()[0]
            .queue
            .as_ref()
            .unwrap()
            .state,
        "automatic"
    );
    let mut report = MaintenanceCycleReport::default();
    assert!(m.resume_budget_review(&pages, &mut report).await.unwrap());
    assert!(!m.resume_budget_review(&pages, &mut report).await.unwrap());
    assert_eq!(*worker.0.lock().unwrap(), 1);
    assert_eq!(
        m.ledger.review_item(&id).unwrap().status,
        MaintenanceReviewStatus::Rejected
    );
    assert_eq!(report.worker_calls, 1);
    f.close().await;
}

#[tokio::test]
async fn competing_pending_titles_route_comparison_without_merging_sources() {
    let f = Fixture::open("pending-topic-overlap").await;
    let pages = sources(&f).await;
    let mut c = config(&f);
    enable_review(&mut c);
    let mut m = RuntimeMaintainer::for_test(
        f.client.clone(),
        Arc::new(FakeWorker::new(vec![])),
        c.clone(),
    );
    let mut ids = vec![];
    for subset in [&pages[..2], &pages[2..]] {
        let mut candidate = m
            .topic_from_response(&pages, &pages, &[], proposal(subset))
            .unwrap();
        candidate.verification = Some(approved());
        ids.push(m.ledger.enqueue_review(
            MaintenanceReviewPayload::Topic(candidate),
            MaintenanceReviewOrigin::Automatic,
            "approved".into(),
            2,
            false,
        ));
    }
    m.ledger.save(&c.state_path).await.unwrap();
    assert!(m.refresh_maintenance_reviews().await.unwrap());
    let routed = m.routed_reviews(&c).await.unwrap();
    assert_eq!(routed.len(), 2);
    assert_eq!(
        routed
            .iter()
            .filter(|r| r.queue.as_ref().unwrap().state == "automatic")
            .count(),
        1
    );
    assert_eq!(
        routed
            .iter()
            .filter(|r| r.queue.as_ref().unwrap().state == "waiting_accumulation")
            .count(),
        1
    );
    let feedback = m.ledger.pending_topic_feedback(
        "PCP source provenance and review",
        &pages[2..]
            .iter()
            .map(|p| p.revision_id.clone())
            .collect::<Vec<_>>(),
    );
    assert_eq!(feedback.len(), 1);
    assert!(feedback[0].content.is_some());
    assert_eq!(feedback[0].source_page_ids.len(), 2);
    assert!(!m.refresh_maintenance_reviews().await.unwrap());
    assert!(
        ids.iter()
            .all(|id| m.ledger.review_item(id).unwrap().status == MaintenanceReviewStatus::Pending)
    );
    f.close().await;
}

#[tokio::test]
async fn automatic_short_draft_waits_before_spending_on_verification() {
    let f = Fixture::open("short-topic-no-verifier").await;
    let pages = sources(&f).await;
    let worker = Arc::new(FakeWorker::new(vec![proposal(&pages[..2])]));
    let mut m = RuntimeMaintainer::for_test(f.client.clone(), worker.clone(), config(&f));
    let mut report = MaintenanceCycleReport::default();
    m.run_topic_review_job(&pages, &mut report, MaintenanceReviewOrigin::Automatic)
        .await
        .unwrap();
    assert_eq!(worker.request_count(), 1);
    assert_eq!(report.deferred, 1);
    assert!(m.pending_reviews().is_empty());
    f.close().await;
}

#[tokio::test]
async fn queued_relation_resolves_existing_reverse_edge_without_duplicate() {
    let fixture = Fixture::open("review-existing-reverse-edge").await;
    let first = fixture
        .client
        .write_page(fixture.page("First durable context.", "reverse:1"))
        .await
        .unwrap();
    let second = fixture
        .client
        .write_page(fixture.page("Second durable context.", "reverse:2"))
        .await
        .unwrap();
    let worker = Arc::new(FakeWorker::new(vec![MaintenanceWorkerResponse::Relate {
        page_ids: [first.page_id.clone(), second.page_id.clone()],
        reason: "The sources share one stable subject.".into(),
    }]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = false;
    config.relation.enabled = true;
    let mut maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker, config);
    maintainer.run_once().await.unwrap();
    let pending = maintainer.pending_relation_reviews();
    assert_eq!(pending.len(), 1);
    let existing = fixture
        .client
        .link_pages(LinkPagesRequest {
            from_page_id: second.page_id.clone(),
            relation_type: "related_to".into(),
            to_page_id: first.page_id.clone(),
            basis_revision_ids: vec![first.revision_id, second.revision_id],
            created_by: Actor {
                actor_type: ActorType::Tool,
                actor_id: "tool:concurrent-operator".into(),
            },
            idempotency_key: Some("external:reverse-edge".into()),
        })
        .await
        .unwrap();
    let resolved = maintainer
        .approve_relation_review(&pending[0].candidate_id)
        .await
        .unwrap();
    assert_eq!(resolved.relation_id, existing.relation_id);
    assert!(maintainer.pending_relation_reviews().is_empty());
    let pages = fixture
        .client
        .read_pages(ReadPagesRequest {
            page_ids: vec![first.page_id],
            revision_ids: vec![],
            projections: vec![Projection::Manifest, Projection::Relations],
            max_chars: 1,
        })
        .await
        .unwrap();
    assert_eq!(
        pages[0]
            .relations
            .iter()
            .filter(|r| r.relation_type == "related_to")
            .count(),
        1
    );
    fixture.close().await;
}

#[tokio::test]
async fn scheduled_review_runs_without_new_writes_or_periodic_scan() {
    let f = Fixture::open("scheduled-baseline-once").await;
    let pages = sources(&f).await;
    let worker = Arc::new(ResumeOnlyWorker(Mutex::new(0)));
    let mut c = config(&f);
    enable_review(&mut c);
    c.periodic_review.enabled = false;
    let mut m = RuntimeMaintainer::for_test(f.client.clone(), worker.clone(), c.clone());
    let mut candidate = m
        .topic_from_response(&pages, &pages, &[], proposal(&pages))
        .unwrap();
    let mut v = approved();
    v.verdict = VerificationVerdict::NeedsReview;
    v.requires_user_input = true;
    candidate.verification = Some(v);
    let id = m.ledger.enqueue_review(
        MaintenanceReviewPayload::Topic(candidate),
        MaintenanceReviewOrigin::Automatic,
        "choose duplicate target".into(),
        2,
        false,
    );
    m.ledger.save(&c.state_path).await.unwrap();
    assert_eq!(
        m.routed_reviews(&c).await.unwrap()[0]
            .queue
            .as_ref()
            .unwrap()
            .state,
        "automatic"
    );
    assert_eq!(m.review_wake_delay(21600).await.unwrap(), 30);
    let report = m.run_scheduled_cycle().await.unwrap();
    assert_eq!(report.jobs_advanced, 1);
    assert_eq!(report.worker_calls, 1);
    assert!(!report.periodic_review);
    assert_eq!(m.review_wake_delay(21600).await.unwrap(), 21600);
    let again = m.run_scheduled_cycle().await.unwrap();
    assert_eq!(again.worker_calls, 0);
    assert_eq!(*worker.0.lock().unwrap(), 1);
    assert_eq!(
        m.ledger.review_item(&id).unwrap().status,
        MaintenanceReviewStatus::Rejected
    );
    assert_eq!(report.worker_calls, 1);
    f.close().await;
}

struct UnavailableTopicWorker(Mutex<Vec<&'static str>>);
#[async_trait]
impl SemanticMaintenanceWorker for UnavailableTopicWorker {
    async fn evaluate(
        &self,
        request: MaintenanceWorkerRequest,
    ) -> Result<MaintenanceWorkerResponse> {
        match request {
            MaintenanceWorkerRequest::ExtractTopic { .. } => {
                self.0.lock().unwrap().push("topic");
                Err(anyhow::anyhow!("upstream connection unavailable")
                    .context("Infer request failed"))
            }
            MaintenanceWorkerRequest::SelectRelation { .. } => {
                self.0.lock().unwrap().push("relation");
                Ok(MaintenanceWorkerResponse::NoCandidate)
            }
            _ => Ok(MaintenanceWorkerResponse::Defer),
        }
    }
}

#[tokio::test]
async fn topic_provider_failure_is_persisted_and_does_not_block_relation_or_repeat() {
    let f = Fixture::open("topic-provider-isolation").await;
    let pages = sources(&f).await;
    let worker = Arc::new(UnavailableTopicWorker(Mutex::new(vec![])));
    let mut c = config(&f);
    c.relation.enabled = true;
    c.max_jobs_per_cycle = 3;
    c.periodic_review.enabled = true;
    let mut m = RuntimeMaintainer::for_test(f.client.clone(), worker.clone(), c.clone());
    m.ledger.advance_semantic_turn(1); // Topic runs before Relation in this cycle.
    let report = m.run_scheduled_cycle().await.unwrap();
    assert_eq!(report.isolated_jobs, 1);
    assert_eq!(*worker.0.lock().unwrap(), vec!["topic", "relation"]);
    let status = RuntimeMaintainer::automation_status(&c).await.unwrap();
    assert!(status.last_error.is_none());
    assert_eq!(status.job_issues.len(), 1);
    assert!(
        status.job_issues[0]
            .reason
            .contains("Infer request failed: upstream connection unavailable")
    );
    assert!(m.ledger.due_job_regions(&pages).is_empty());
    m.run_scheduled_cycle().await.unwrap();
    assert_eq!(*worker.0.lock().unwrap(), vec!["topic", "relation"]);
    f.close().await;
}

struct CachedPendingWorker(Mutex<u32>);
#[async_trait]
impl SemanticMaintenanceWorker for CachedPendingWorker {
    async fn evaluate(&self, _: MaintenanceWorkerRequest) -> Result<MaintenanceWorkerResponse> {
        Ok(MaintenanceWorkerResponse::NoCandidate)
    }
    async fn review_existing_with_usage(
        &self,
        _: MaintenanceWorkerRequest,
        baseline: MaintenanceVerification,
    ) -> Result<crate::maintenance::MaintenanceWorkerOutcome> {
        *self.0.lock().unwrap() += 1;
        Ok(crate::maintenance::MaintenanceWorkerOutcome {
            response: MaintenanceWorkerResponse::VerifyMaintenance {
                assessment: baseline,
            },
            usage: None,
            model_attempts: 0,
            escalated: true,
        })
    }
}

#[tokio::test]
async fn unchanged_cached_review_cannot_consume_every_scheduled_cycle() {
    let f = Fixture::open("cached-review-convergence").await;
    let pages = sources(&f).await;
    let worker = Arc::new(CachedPendingWorker(Mutex::new(0)));
    let mut c = config(&f);
    enable_review(&mut c);
    c.periodic_review.enabled = false;
    let mut m = RuntimeMaintainer::for_test(f.client.clone(), worker.clone(), c.clone());
    let mut candidate = m
        .topic_from_response(&pages, &pages, &[], proposal(&pages))
        .unwrap();
    let mut v = approved();
    v.verdict = VerificationVerdict::NeedsReview;
    v.requires_user_input = true;
    candidate.verification = Some(v);
    let review_id = m.ledger.enqueue_review(
        MaintenanceReviewPayload::Topic(candidate),
        MaintenanceReviewOrigin::Automatic,
        "Cached uncertainty".into(),
        1,
        false,
    );
    m.ledger.save(&c.state_path).await.unwrap();
    let before = m.ledger.review_item(&review_id).unwrap();
    let first = m.run_scheduled_cycle().await.unwrap();
    assert_eq!(first.jobs_advanced, 0);
    assert_eq!(first.worker_calls, 0);
    assert_eq!(first.escalated_decisions, 0);
    let after = m.ledger.review_item(&review_id).unwrap();
    assert_eq!(before.updated_at, after.updated_at);
    assert_eq!(before.model_attempts, after.model_attempts);
    let delay = m.review_wake_delay(21600).await.unwrap();
    assert!((299..=300).contains(&delay));
    m.ledger = MaintenanceLedger::load(&c.state_path).await.unwrap();
    let second = m.run_scheduled_cycle().await.unwrap();
    assert_eq!(second.jobs_advanced, 0);
    assert_eq!(second.worker_calls, 0);
    assert_eq!(*worker.0.lock().unwrap(), 1);
    f.close().await;
}

#[tokio::test]
async fn cached_topic_does_not_hide_relation_and_both_cooldowns_survive_reload() {
    let f = Fixture::open("cached-review-fairness").await;
    let pages = sources(&f).await;
    let worker = Arc::new(CachedPendingWorker(Mutex::new(0)));
    let mut c = config(&f);
    enable_review(&mut c);
    c.relation.enabled = true;
    c.relation.auto_apply_verified = true;
    let mut m = RuntimeMaintainer::for_test(f.client.clone(), worker.clone(), c.clone());
    let mut candidate = m
        .topic_from_response(&pages, &pages, &[], proposal(&pages))
        .unwrap();
    let mut assessment = approved();
    assessment.verdict = VerificationVerdict::NeedsReview;
    assessment.requires_user_input = true;
    candidate.verification = Some(assessment.clone());
    let topic_id = m.ledger.enqueue_review(
        MaintenanceReviewPayload::Topic(candidate),
        MaintenanceReviewOrigin::Automatic,
        "Cached topic".into(),
        1,
        false,
    );
    let relation_pages = [0, 1].map(|i| super::super::ledger::MaintenanceRelationReviewPage {
        page_id: pages[i].page_id.clone(),
        revision_id: pages[i].revision_id.clone(),
        preview: pages[i].snippet.clone(),
    });
    let relation_id = m.ledger.propose_relation_review(
        pages[0].namespace.clone(),
        relation_pages,
        "Source relationship".into(),
        1,
        false,
    );
    m.ledger
        .update_relation_verification(&relation_id, assessment, 0)
        .unwrap();
    m.ledger.save(&c.state_path).await.unwrap();
    let mut report = MaintenanceCycleReport::default();
    assert!(!m.resume_budget_review(&pages, &mut report).await.unwrap());
    assert_eq!(*worker.0.lock().unwrap(), 2);
    assert_eq!(report.worker_calls, 0);
    m.ledger = MaintenanceLedger::load(&c.state_path).await.unwrap();
    for id in [topic_id, relation_id] {
        assert!((299..=300).contains(&m.ledger.retry_delay(&format!("review_resume:{id}"))));
    }
    assert!(!m.resume_budget_review(&pages, &mut report).await.unwrap());
    assert_eq!(*worker.0.lock().unwrap(), 2);
    f.close().await;
}

struct UnavailableVerificationWorker(MaintenanceWorkerResponse);
#[async_trait]
impl SemanticMaintenanceWorker for UnavailableVerificationWorker {
    async fn evaluate(
        &self,
        request: MaintenanceWorkerRequest,
    ) -> Result<MaintenanceWorkerResponse> {
        match request {
            MaintenanceWorkerRequest::ExtractTopic { .. } => Ok(self.0.clone()),
            MaintenanceWorkerRequest::VerifyMaintenance { .. } => {
                anyhow::bail!("verification provider unavailable")
            }
            _ => Ok(MaintenanceWorkerResponse::NoCandidate),
        }
    }
}

#[tokio::test]
async fn unavailable_verification_retains_pending_proposal_without_approving_or_writing() {
    let f = Fixture::open("verification-provider-isolation").await;
    let pages = sources(&f).await;
    let worker = Arc::new(UnavailableVerificationWorker(proposal(&pages)));
    let c = config(&f);
    let mut m = RuntimeMaintainer::for_test(f.client.clone(), worker, c);
    let report = m.run_convergence_once(1).await.unwrap();
    assert_eq!(report.topics_written, 0);
    assert_eq!(report.worker_calls, 2);
    let pending = m.pending_reviews();
    assert_eq!(pending.len(), 1);
    let MaintenanceReviewPayload::Topic(candidate) = &pending[0].payload else {
        panic!("topic expected")
    };
    let assessment = candidate.verification.as_ref().unwrap();
    assert!(matches!(
        assessment.verdict,
        VerificationVerdict::NeedsReview
    ));
    assert_eq!(assessment.review_state.as_deref(), Some("waiting_provider"));
    assert!(
        assessment
            .reason
            .contains("verification provider unavailable")
    );
    f.close().await;
}
