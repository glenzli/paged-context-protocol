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
