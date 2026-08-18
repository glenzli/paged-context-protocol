use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::SystemTime,
};

use anyhow::{Context, Result};
use async_trait::async_trait;
use pcp_client::{EmbeddedPcpClient, PcpApi};
use pcp_core::{
    AccessPermission, AccessPrincipal, AccessPrincipalType, AccessSession, Actor, ActorType,
    CreateScopeRequest, LifecycleStatus, LinkPagesRequest, PackPagesRequest, PageMutability,
    PagePayload, PageRevisionRef, PlanRevisionRetentionRequest, Projection, ProvenanceEvent,
    ReadPagesRequest, RetentionPolicy, RetentionProtectionReason, RevisePageRequest, SourceSpan,
    WritePageRequest,
};
use pcp_sqlite::SqlitePcpStore;
use pcp_store::PcpStore;

use super::{
    AnalyzeMaintenancePacksRequest, AnalyzeMaintenanceRelationRequest,
    AnalyzeMaintenanceSummariesRequest, AnalyzeMaintenanceSummaryRequest,
    ApplyMaintenancePackRequest, ApplyMaintenanceRelationRequest, ApplyMaintenanceSummaryRequest,
    CommandSemanticWorker, MaintenanceAutomationState, MaintenanceConfig, MaintenanceMode,
    MaintenanceRunAudit, MaintenanceWorkerConfig, MaintenanceWorkerRequest,
    MaintenanceWorkerResponse, PackingCandidateGroup, PackingMaintenanceConfig,
    RelationCandidatePage, RelationMaintenanceConfig, RetentionMaintenanceConfig,
    RetentionMilestone, RuntimeMaintainer, SemanticMaintenanceWorker, SummaryMaintenanceConfig,
    WriteTriggeredMaintenanceConfig, worker::PackingCandidatePage,
};

struct FakeWorker {
    responses: Mutex<VecDeque<MaintenanceWorkerResponse>>,
    requests: Mutex<Vec<MaintenanceWorkerRequest>>,
}

#[test]
fn packing_is_opt_in_and_selection_wire_contains_only_semantic_inputs() {
    let config = PackingMaintenanceConfig::default();
    assert!(!config.enabled);
    assert_eq!(config.analysis_window_pages, 64);
    assert_eq!(config.routing_chars_per_page, 1_200);
    let wire = serde_json::to_value(MaintenanceWorkerRequest::SelectPacking {
        pages: vec![PackingCandidatePage {
            page_id: "pg_1".to_owned(),
            created_at: "2026-08-15T08:00:00Z".to_owned(),
            observed_at: None,
            routing_text: "one bounded source excerpt".to_owned(),
            packed: false,
        }],
        excluded_candidate_sets: Vec::new(),
    })
    .expect("serialize packing selection request");

    assert_eq!(
        wire,
        serde_json::json!({
            "operation": "select_packing",
            "pages": [{
                "pageId": "pg_1",
                "createdAt": "2026-08-15T08:00:00Z",
                "routingText": "one bounded source excerpt"
            }],
            "excluded_candidate_sets": []
        })
    );
}

#[test]
fn packing_analysis_wire_batches_mechanical_groups_without_commit_authority() {
    let wire = serde_json::to_value(MaintenanceWorkerRequest::AnalyzePacking {
        groups: vec![PackingCandidateGroup {
            group_id: "mpg_one".to_owned(),
            pages: vec![PackingCandidatePage {
                page_id: "pg_1".to_owned(),
                created_at: "2026-08-16T08:00:00Z".to_owned(),
                observed_at: None,
                routing_text: "bounded semantic input".to_owned(),
                packed: false,
            }],
        }],
        max_pages_per_candidate: 8,
    })
    .expect("serialize packing analysis request");

    assert_eq!(
        wire,
        serde_json::json!({
            "operation": "analyze_packing",
            "groups": [{
                "groupId": "mpg_one",
                "pages": [{
                    "pageId": "pg_1",
                    "createdAt": "2026-08-16T08:00:00Z",
                    "routingText": "bounded semantic input"
                }]
            }],
            "max_pages_per_candidate": 8
        })
    );
}

#[test]
fn single_summary_wire_keeps_page_identity_out_of_the_response_contract() {
    let wire = serde_json::to_value(MaintenanceWorkerRequest::SummarizePage {
        page: Box::new(super::MaintenanceDetailPage {
            page_id: "pg_1".to_owned(),
            revision_id: "rev_1".to_owned(),
            namespace: "project:one".to_owned(),
            created_at: "2026-08-16T08:00:00Z".to_owned(),
            observed_at: None,
            media_type: Some("text/markdown".to_owned()),
            content: Some("bounded source content".to_owned()),
            summary: None,
            facets: None,
            source_refs: Vec::new(),
            relations: Vec::new(),
        }),
    })
    .expect("serialize single-page Summary request");

    assert_eq!(
        wire,
        serde_json::json!({
            "operation": "summarize_page",
            "page": {
                "pageId": "pg_1",
                "revisionId": "rev_1",
                "namespace": "project:one",
                "createdAt": "2026-08-16T08:00:00Z",
                "mediaType": "text/markdown",
                "content": "bounded source content",
                "sourceRefs": [],
                "relations": []
            }
        })
    );
}

#[test]
fn relation_is_opt_in_and_selection_wire_omits_commit_authority() {
    assert!(!RelationMaintenanceConfig::default().enabled);
    let wire = serde_json::to_value(MaintenanceWorkerRequest::SelectRelation {
        pages: vec![RelationCandidatePage {
            page_id: "pg_1".to_owned(),
            namespace: "project:one".to_owned(),
            kind: "document".to_owned(),
            created_at: "2026-08-15T08:00:00Z".to_owned(),
            observed_at: None,
            routing_text: "one bounded routing excerpt".to_owned(),
            facets: None,
            relation_types: vec!["summarizes".to_owned()],
        }],
        excluded_page_pairs: vec![["pg_1".to_owned(), "pg_2".to_owned()]],
    })
    .expect("serialize relation selection request");

    assert_eq!(
        wire,
        serde_json::json!({
            "operation": "select_relation",
            "pages": [{
                "pageId": "pg_1",
                "namespace": "project:one",
                "kind": "document",
                "createdAt": "2026-08-15T08:00:00Z",
                "routingText": "one bounded routing excerpt",
                "relationTypes": ["summarizes"]
            }],
            "excluded_page_pairs": [["pg_1", "pg_2"]]
        })
    );
    assert_eq!(
        serde_json::to_value(MaintenanceWorkerResponse::Relate {
            page_ids: ["pg_1".to_owned(), "pg_2".to_owned()],
        })
        .expect("serialize relation decision"),
        serde_json::json!({
            "decision": "relate",
            "page_ids": ["pg_1", "pg_2"]
        })
    );
}

impl FakeWorker {
    fn new(responses: Vec<MaintenanceWorkerResponse>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn request_count(&self) -> usize {
        self.requests.lock().expect("fake worker requests").len()
    }
}

#[async_trait]
impl SemanticMaintenanceWorker for FakeWorker {
    async fn evaluate(
        &self,
        request: MaintenanceWorkerRequest,
    ) -> Result<MaintenanceWorkerResponse> {
        self.requests
            .lock()
            .expect("fake worker requests")
            .push(request);
        self.responses
            .lock()
            .expect("fake worker responses")
            .pop_front()
            .context("missing fake maintenance worker response")
    }
}

#[tokio::test]
async fn maintainer_writes_only_the_summary_selected_by_the_worker() {
    let fixture = Fixture::open("summary").await;
    let written = fixture
        .client
        .write_page(fixture.page(
            &"A durable observation with detail. ".repeat(180),
            "summary:1",
        ))
        .await
        .expect("write Summary candidate");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::WriteSummary {
            content: "A bounded routing Summary chosen by the semantic worker.".to_owned(),
        },
    ]));
    let mut config = fixture.config();
    config.packing.enabled = false;
    let mut maintainer =
        RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);

    let report = maintainer
        .run_once()
        .await
        .expect("run Summary maintenance");

    assert_eq!(report.worker_calls, 1);
    assert_eq!(report.summaries_written, 1);
    assert_eq!(worker.request_count(), 1);
    let read = fixture
        .client
        .read_pages(ReadPagesRequest {
            page_ids: Vec::new(),
            revision_ids: vec![written.revision_id],
            projections: vec![Projection::Manifest, Projection::Summary],
            max_chars: 2_000,
        })
        .await
        .expect("read summarized Page");
    assert_eq!(
        read[0]
            .summary
            .as_ref()
            .map(|summary| summary.content.as_str()),
        Some("A bounded routing Summary chosen by the semantic worker.")
    );
    fixture.close().await;
}

#[tokio::test]
async fn maintainer_refreshes_a_summary_after_its_target_page_is_revised() {
    let fixture = Fixture::open("summary-refresh").await;
    let written = fixture
        .client
        .write_page(fixture.page(
            &"A durable observation with initial detail. ".repeat(180),
            "summary-refresh:1",
        ))
        .await
        .expect("write initial Summary candidate");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::WriteSummary {
            content: "Summary of the initial Page Revision.".to_owned(),
        },
        MaintenanceWorkerResponse::WriteSummary {
            content: "Summary refreshed for the current Page Revision.".to_owned(),
        },
    ]));
    let mut config = fixture.config();
    config.packing.enabled = false;
    config.relation.enabled = false;
    let mut maintainer =
        RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);

    let initial_report = maintainer.run_once().await.expect("write initial Summary");
    assert_eq!(initial_report.summaries_written, 1);

    let revised = fixture
        .client
        .revise_page(RevisePageRequest {
            page_id: written.page_id.clone(),
            expected_revision_id: written.revision_id,
            created_by: Actor {
                actor_type: ActorType::Model,
                actor_id: "model:maintenance-test".to_owned(),
            },
            lifecycle_status: LifecycleStatus::Active,
            observed_at: None,
            valid_from: None,
            valid_to: None,
            payload: Some(PagePayload {
                media_type: "text/markdown".to_owned(),
                content: "The durable observation now has materially revised detail. ".repeat(180),
            }),
            source_refs: Vec::new(),
            facets: None,
            provenance: Vec::new(),
            initial_relations: Vec::new(),
            idempotency_key: Some("summary-refresh:2".to_owned()),
        })
        .await
        .expect("revise summarized Page");
    fixture
        .client
        .write_page(fixture.page(
            &"A newer unsummarized Page that must wait behind stale Summary repair. ".repeat(180),
            "summary-refresh:newer-unsummarized",
        ))
        .await
        .expect("write newer unsummarized Page");

    let refresh_report = maintainer.run_once().await.expect("refresh stale Summary");
    assert_eq!(refresh_report.summaries_written, 1);
    assert_eq!(worker.request_count(), 2);

    let current = fixture
        .client
        .read_pages(ReadPagesRequest {
            page_ids: Vec::new(),
            revision_ids: vec![revised.revision_id],
            projections: vec![Projection::Manifest, Projection::Summary],
            max_chars: 2_000,
        })
        .await
        .expect("read refreshed Summary");
    assert_eq!(
        current[0]
            .summary
            .as_ref()
            .map(|summary| summary.content.as_str()),
        Some("Summary refreshed for the current Page Revision.")
    );
    fixture.close().await;
}

#[tokio::test]
async fn maintainer_defers_an_oversized_worker_summary_without_writing_it() {
    let fixture = Fixture::open("oversized-summary").await;
    let written = fixture
        .client
        .write_page(fixture.page(
            &"A durable observation with detail. ".repeat(180),
            "oversized-summary:1",
        ))
        .await
        .expect("write oversized Summary candidate");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::WriteSummary {
            content: "x".repeat(1_201),
        },
    ]));
    let mut config = fixture.config();
    config.packing.enabled = false;
    let mut maintainer =
        RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);

    let report = maintainer
        .run_once()
        .await
        .expect("defer oversized worker Summary");

    assert_eq!(report.worker_calls, 1);
    assert_eq!(report.summaries_written, 0);
    assert_eq!(report.deferred, 1);
    let read = fixture
        .client
        .read_pages(ReadPagesRequest {
            page_ids: Vec::new(),
            revision_ids: vec![written.revision_id],
            projections: vec![Projection::Manifest, Projection::Summary],
            max_chars: 2_000,
        })
        .await
        .expect("read Page after oversized worker Summary");
    assert!(read[0].summary.is_none());
    fixture.close().await;
}

#[tokio::test]
async fn observe_mode_records_a_summary_proposal_without_mutating_the_page() {
    let fixture = Fixture::open("observe-summary").await;
    let written = fixture
        .client
        .write_page(fixture.page(
            &"A durable observation with detail. ".repeat(180),
            "observe-summary:1",
        ))
        .await
        .expect("write observed Summary candidate");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::WriteSummary {
            content: "A proposed routing Summary that must not be committed.".to_owned(),
        },
    ]));
    let mut config = fixture.config();
    config.mode = MaintenanceMode::Observe;
    config.packing.enabled = false;
    let mut maintainer =
        RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);

    let first = maintainer
        .run_once()
        .await
        .expect("observe Summary maintenance");
    let second = maintainer
        .run_once()
        .await
        .expect("respect observed Summary cooldown");

    assert_eq!(first.summaries_proposed, 1);
    assert_eq!(first.summaries_written, 0);
    assert_eq!(second.worker_calls, 0);
    assert_eq!(worker.request_count(), 1);
    let read = fixture
        .client
        .read_pages(ReadPagesRequest {
            page_ids: Vec::new(),
            revision_ids: vec![written.revision_id],
            projections: vec![Projection::Manifest, Projection::Summary],
            max_chars: 2_000,
        })
        .await
        .expect("read observed Page");
    assert!(read[0].summary.is_none());
    fixture.close().await;
}

#[tokio::test]
async fn packed_conversation_page_can_receive_a_summary() {
    let fixture = Fixture::open("packed-summary").await;
    let first = fixture
        .client
        .write_page(fixture.sealed_event(
            &"A continuous high-density discussion segment. ".repeat(100),
            "packed-summary:1",
            1,
        ))
        .await
        .expect("write first packed Summary source");
    let second = fixture
        .client
        .write_page(fixture.sealed_event(
            &"The same discussion continues with durable detail. ".repeat(100),
            "packed-summary:2",
            2,
        ))
        .await
        .expect("write second packed Summary source");
    let packed = fixture
        .client
        .pack_pages(PackPagesRequest {
            pages: vec![
                PageRevisionRef {
                    page_id: first.page_id,
                    revision_id: first.revision_id,
                },
                PageRevisionRef {
                    page_id: second.page_id,
                    revision_id: second.revision_id,
                },
            ],
            idempotency_key: Some("packed-summary:pack".to_owned()),
        })
        .await
        .expect("pack conversation Summary source");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::WriteSummary {
            content: "A summary of the packed continuous discussion.".to_owned(),
        },
    ]));
    let mut config = fixture.config();
    config.packing.enabled = false;
    config.relation.enabled = false;
    let mut maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker, config);

    let report = maintainer
        .run_once()
        .await
        .expect("summarize packed conversation Page");

    assert_eq!(report.summaries_written, 1);
    let page = fixture
        .client
        .read_pages(ReadPagesRequest {
            page_ids: vec![packed.page_id],
            revision_ids: Vec::new(),
            projections: vec![Projection::Summary],
            max_chars: 2_000,
        })
        .await
        .expect("read packed Summary Page");
    assert_eq!(
        page[0]
            .summary
            .as_ref()
            .map(|summary| summary.content.as_str()),
        Some("A summary of the packed continuous discussion.")
    );
    fixture.close().await;
}

#[tokio::test]
async fn manual_relation_review_includes_adjacent_packed_conversation_pages() {
    let fixture = Fixture::open("packed-relation-review").await;
    let first = fixture
        .client
        .write_page(fixture.sealed_event(
            "The user asks what OET means in this durable conversation.",
            "packed-relation-review:1",
            1,
        ))
        .await
        .expect("write first OET event");
    let second = fixture
        .client
        .write_page(fixture.sealed_event(
            "The assistant explains that OET needs a precise definition before use.",
            "packed-relation-review:2",
            2,
        ))
        .await
        .expect("write second OET event");
    let first_pack = fixture
        .client
        .pack_pages(PackPagesRequest {
            pages: vec![first, second]
                .into_iter()
                .map(|page| PageRevisionRef {
                    page_id: page.page_id,
                    revision_id: page.revision_id,
                })
                .collect(),
            idempotency_key: Some("packed-relation-review:first-pack".to_owned()),
        })
        .await
        .expect("pack first OET episode");
    let third = fixture
        .client
        .write_page(fixture.sealed_event(
            "The OET definition distinguishes its strict core from later research conjectures.",
            "packed-relation-review:3",
            3,
        ))
        .await
        .expect("write third OET event");
    let fourth = fixture
        .client
        .write_page(fixture.sealed_event(
            "That OET maturity boundary determines how future context should be recalled.",
            "packed-relation-review:4",
            4,
        ))
        .await
        .expect("write fourth OET event");
    let second_pack = fixture
        .client
        .pack_pages(PackPagesRequest {
            pages: vec![third, fourth]
                .into_iter()
                .map(|page| PageRevisionRef {
                    page_id: page.page_id,
                    revision_id: page.revision_id,
                })
                .collect(),
            idempotency_key: Some("packed-relation-review:second-pack".to_owned()),
        })
        .await
        .expect("pack second OET episode");
    let worker = Arc::new(FakeWorker::new(vec![MaintenanceWorkerResponse::Relate {
        page_ids: [first_pack.page_id.clone(), second_pack.page_id.clone()],
    }]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = true;
    config.relation.enabled = true;
    let maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);

    let scan = maintainer
        .scan_maintenance_work()
        .await
        .expect("scan packed relation review candidates");
    let group = scan
        .relation
        .groups
        .iter()
        .find(|group| group.page_count >= 2)
        .expect("packed conversation Pages remain relation eligible");
    let analysis = maintainer
        .analyze_relation_candidate(AnalyzeMaintenanceRelationRequest {
            scan_id: scan.relation.scan_id,
            group_id: group.group_id.clone(),
        })
        .await
        .expect("analyze packed relation candidate");
    let candidate = analysis.candidate.expect("packed relation proposal");
    let proposed_page_ids = candidate
        .pages
        .iter()
        .map(|page| page.page_id.as_str())
        .collect::<Vec<_>>();
    assert!(proposed_page_ids.contains(&first_pack.page_id.as_str()));
    assert!(proposed_page_ids.contains(&second_pack.page_id.as_str()));
    assert_eq!(worker.request_count(), 1);
    fixture.close().await;
}

#[tokio::test]
async fn reviewed_summary_is_a_revision_bound_proposal_before_it_is_written() {
    let fixture = Fixture::open("reviewed-summary").await;
    let written = fixture
        .client
        .write_page(fixture.page(
            &"A durable Page whose Summary must be proposed before it is written. ".repeat(100),
            "reviewed-summary:1",
        ))
        .await
        .expect("write reviewable Summary Page");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::WriteSummary {
            content: "A bounded Summary proposed for explicit operator review.".to_owned(),
        },
    ]));
    let mut config = fixture.config();
    config.packing.enabled = false;
    config.relation.enabled = false;
    let maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker, config);

    let analysis = maintainer
        .analyze_summary_candidate(AnalyzeMaintenanceSummaryRequest {
            page_id: written.page_id.clone(),
            revision_id: written.revision_id.clone(),
        })
        .await
        .expect("analyze Summary review");
    let candidate = analysis.candidate.expect("Summary proposal");
    let before = fixture
        .client
        .read_pages(ReadPagesRequest {
            page_ids: vec![written.page_id.clone()],
            revision_ids: Vec::new(),
            projections: vec![Projection::Manifest, Projection::Summary],
            max_chars: 2_000,
        })
        .await
        .expect("read unmodified Summary Page");
    assert!(before[0].summary.is_none());

    let request = ApplyMaintenanceSummaryRequest {
        candidate_id: candidate.candidate_id.clone(),
        page_id: candidate.page_id.clone(),
        revision_id: candidate.revision_id.clone(),
        expected_summary_revision_id: candidate.expected_summary_revision_id.clone(),
        content: candidate.content.clone(),
    };
    maintainer
        .apply_summary_candidate(request.clone())
        .await
        .expect("apply reviewed Summary");
    let after = fixture
        .client
        .read_pages(ReadPagesRequest {
            page_ids: vec![written.page_id],
            revision_ids: Vec::new(),
            projections: vec![Projection::Manifest, Projection::Summary],
            max_chars: 2_000,
        })
        .await
        .expect("read written Summary");
    assert_eq!(
        after[0]
            .summary
            .as_ref()
            .map(|summary| summary.content.as_str()),
        Some(candidate.content.as_str())
    );
    assert!(maintainer.apply_summary_candidate(request).await.is_err());
    fixture.close().await;
}

#[tokio::test]
async fn reviewed_summaries_send_one_page_per_worker_call_before_any_write() {
    let fixture = Fixture::open("reviewed-summary-batch").await;
    let first = fixture
        .client
        .write_page(fixture.page(
            &"The first durable routing Page needs a concise Summary. ".repeat(100),
            "reviewed-summary-batch:1",
        ))
        .await
        .expect("write first reviewable Summary Page");
    let second = fixture
        .client
        .write_page(fixture.page(
            &"The second durable routing Page needs a concise Summary. ".repeat(100),
            "reviewed-summary-batch:2",
        ))
        .await
        .expect("write second reviewable Summary Page");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::WriteSummary {
            content: "First concise routing Summary.".to_owned(),
        },
        MaintenanceWorkerResponse::WriteSummary {
            content: "Second concise routing Summary.".to_owned(),
        },
    ]));
    let mut config = fixture.config();
    config.packing.enabled = false;
    config.relation.enabled = false;
    let maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);
    let scan = maintainer
        .scan_maintenance_work()
        .await
        .expect("scan reviewable Summary Pages");

    let analysis = maintainer
        .analyze_summary_candidates(AnalyzeMaintenanceSummariesRequest {
            scan_id: scan.summary.scan_id,
            pages: vec![
                PageRevisionRef {
                    page_id: first.page_id.clone(),
                    revision_id: first.revision_id.clone(),
                },
                PageRevisionRef {
                    page_id: second.page_id.clone(),
                    revision_id: second.revision_id.clone(),
                },
            ],
        })
        .await
        .expect("analyze batch Summary review");

    assert_eq!(scan.summary.estimated_model_calls, 2);
    assert_eq!(analysis.worker_calls, 2);
    assert_eq!(analysis.candidates.len(), 2);
    let requests = worker
        .requests
        .lock()
        .expect("recorded page-local Summary requests");
    assert_eq!(requests.len(), 2);
    let requested_page_ids = requests
        .iter()
        .map(|request| {
            let MaintenanceWorkerRequest::SummarizePage { page } = request else {
                panic!("expected page-local Summary request");
            };
            page.page_id.as_str()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        requested_page_ids,
        vec![first.page_id.as_str(), second.page_id.as_str()]
    );
    drop(requests);
    let pages = fixture
        .client
        .read_pages(ReadPagesRequest {
            page_ids: vec![first.page_id, second.page_id],
            revision_ids: Vec::new(),
            projections: vec![Projection::Summary],
            max_chars: 2_000,
        })
        .await
        .expect("read unmodified batch Summary Pages");
    assert!(pages.iter().all(|page| page.summary.is_none()));
    fixture.close().await;
}

#[tokio::test]
async fn deferred_summary_page_does_not_drop_other_page_proposals() {
    let fixture = Fixture::open("incomplete-summary-batch").await;
    let first = fixture
        .client
        .write_page(fixture.page(
            &"The first durable routing Page needs a concise Summary. ".repeat(100),
            "incomplete-summary-batch:1",
        ))
        .await
        .expect("write first reviewable Summary Page");
    let second = fixture
        .client
        .write_page(fixture.page(
            &"The second durable routing Page needs a concise Summary. ".repeat(100),
            "incomplete-summary-batch:2",
        ))
        .await
        .expect("write second reviewable Summary Page");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::WriteSummary {
            content: "First routing Summary is still reviewable.".to_owned(),
        },
        MaintenanceWorkerResponse::Defer,
    ]));
    let mut config = fixture.config();
    config.packing.enabled = false;
    config.relation.enabled = false;
    let maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker, config);
    let scan = maintainer
        .scan_maintenance_work()
        .await
        .expect("scan reviewable Summary Pages");

    let analysis = maintainer
        .analyze_summary_candidates(AnalyzeMaintenanceSummariesRequest {
            scan_id: scan.summary.scan_id,
            pages: vec![
                PageRevisionRef {
                    page_id: first.page_id,
                    revision_id: first.revision_id,
                },
                PageRevisionRef {
                    page_id: second.page_id,
                    revision_id: second.revision_id,
                },
            ],
        })
        .await
        .expect("analyze independently reviewable Summary Pages");

    assert_eq!(analysis.worker_calls, 2);
    assert_eq!(analysis.candidates.len(), 1);
    assert_eq!(analysis.deferred_pages, 1);
    assert_eq!(analysis.no_candidate_pages, 0);
    assert!(analysis.issues.is_empty());
    fixture.close().await;
}

#[tokio::test]
async fn reviewed_summary_keeps_a_language_shift_reviewable() {
    let fixture = Fixture::open("summary-language-quality-gate").await;
    let written = fixture
        .client
        .write_page(fixture.page(
            &"这是关于 OpenAI 与 GPT-5.6 的中文长期讨论，必须保留原文语言。".repeat(220),
            "summary-language-quality-gate:1",
        ))
        .await
        .expect("write Chinese reviewable Summary Page");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::WriteSummary {
            content: "This English summary changes the language of the supplied Page.".to_owned(),
        },
    ]));
    let mut config = fixture.config();
    config.packing.enabled = false;
    config.relation.enabled = false;
    let maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker, config);
    let scan = maintainer
        .scan_maintenance_work()
        .await
        .expect("scan Chinese Summary Page");

    let analysis = maintainer
        .analyze_summary_candidates(AnalyzeMaintenanceSummariesRequest {
            scan_id: scan.summary.scan_id,
            pages: vec![PageRevisionRef {
                page_id: written.page_id,
                revision_id: written.revision_id,
            }],
        })
        .await
        .expect("analyze Chinese Summary Page");

    assert_eq!(analysis.candidates.len(), 1);
    assert_eq!(
        analysis.candidates[0].content,
        "This English summary changes the language of the supplied Page."
    );
    assert_eq!(analysis.deferred_pages, 0);
    assert!(analysis.issues.is_empty());
    fixture.close().await;
}

#[tokio::test]
async fn reviewed_summary_normalizes_a_near_miss_source_identifier() {
    let fixture = Fixture::open("summary-identifier-quality-gate").await;
    let written = fixture
        .client
        .write_page(fixture.page(
            &"OpenAI published a durable English source Page for exact identifier preservation. "
                .repeat(180),
            "summary-identifier-quality-gate:1",
        ))
        .await
        .expect("write identifier reviewable Summary Page");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::WriteSummary {
            content: "Openerai published the durable source discussed by this Page.".to_owned(),
        },
    ]));
    let mut config = fixture.config();
    config.packing.enabled = false;
    config.relation.enabled = false;
    let maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker, config);
    let scan = maintainer
        .scan_maintenance_work()
        .await
        .expect("scan identifier Summary Page");

    let analysis = maintainer
        .analyze_summary_candidates(AnalyzeMaintenanceSummariesRequest {
            scan_id: scan.summary.scan_id,
            pages: vec![PageRevisionRef {
                page_id: written.page_id,
                revision_id: written.revision_id,
            }],
        })
        .await
        .expect("analyze identifier Summary Page");

    assert_eq!(analysis.candidates.len(), 1);
    assert_eq!(
        analysis.candidates[0].content,
        "OpenAI published the durable source discussed by this Page."
    );
    assert_eq!(analysis.deferred_pages, 0);
    assert!(analysis.issues.is_empty());
    fixture.close().await;
}

#[tokio::test]
async fn maintenance_scan_surfaces_separate_pack_summary_and_relation_work() {
    let fixture = Fixture::open("phase-scan").await;
    fixture
        .client
        .write_page(fixture.sealed_event(
            "The first source event belongs to one continuous discussion.",
            "phase-scan:event:1",
            1,
        ))
        .await
        .expect("write first Pack candidate Page");
    fixture
        .client
        .write_page(fixture.sealed_event(
            "The second source event continues that same discussion.",
            "phase-scan:event:2",
            2,
        ))
        .await
        .expect("write second Pack candidate Page");
    let long_conversation = fixture
        .client
        .write_page(fixture.sealed_event(
            &"A long conversation Page needs a concise routing Summary. ".repeat(100),
            "phase-scan:event:3",
            3,
        ))
        .await
        .expect("write long conversation Page");
    fixture
        .client
        .write_page(fixture.page(
            "First independent Page available for relation review.",
            "phase-scan:relation:1",
        ))
        .await
        .expect("write first relation Page");
    fixture
        .client
        .write_page(fixture.page(
            "Second independent Page available for relation review.",
            "phase-scan:relation:2",
        ))
        .await
        .expect("write second relation Page");

    let worker = Arc::new(FakeWorker::new(Vec::new()));
    let mut config = fixture.config();
    config.packing.enabled = true;
    config.relation.enabled = true;
    let maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);

    let scan = maintainer
        .scan_maintenance_work()
        .await
        .expect("scan phase work without invoking a worker");

    assert!(scan.packing.candidate_group_count >= 1);
    assert!(
        scan.summary
            .pages
            .iter()
            .any(|page| page.page_id == long_conversation.page_id)
    );
    assert!(scan.relation.candidate_group_count >= 1);
    assert_eq!(worker.request_count(), 0);
    fixture.close().await;
}

#[tokio::test]
async fn adjacent_packs_require_model_approval_before_becoming_merge_proposals() {
    let fixture = Fixture::open("deterministic-pack-merge").await;
    let mut leaves = Vec::new();
    for sequence in 1..=4 {
        leaves.push(
            fixture
                .client
                .write_page(fixture.sealed_event(
                    &format!("OET discussion episode entry {sequence}."),
                    &format!("deterministic-pack-merge:{sequence}"),
                    sequence,
                ))
                .await
                .expect("write merge source Page"),
        );
    }
    let first = fixture
        .client
        .pack_pages(PackPagesRequest {
            pages: leaves[..2]
                .iter()
                .map(|page| PageRevisionRef {
                    page_id: page.page_id.clone(),
                    revision_id: page.revision_id.clone(),
                })
                .collect(),
            idempotency_key: Some("deterministic-pack-merge:first".to_owned()),
        })
        .await
        .expect("pack first OET episode");
    let second = fixture
        .client
        .pack_pages(PackPagesRequest {
            pages: leaves[2..]
                .iter()
                .map(|page| PageRevisionRef {
                    page_id: page.page_id.clone(),
                    revision_id: page.revision_id.clone(),
                })
                .collect(),
            idempotency_key: Some("deterministic-pack-merge:second".to_owned()),
        })
        .await
        .expect("pack second OET episode");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::PackingCandidates {
            candidates: vec![vec![first.page_id.clone(), second.page_id.clone()]],
        },
    ]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = true;
    config.relation.enabled = false;
    config.retention.enabled = false;
    let maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);

    let scan = maintainer
        .scan_packing_candidates()
        .await
        .expect("scan adjacent Packs");
    assert_eq!(scan.candidate_group_count, 1);
    let analysis = maintainer
        .analyze_packing_candidates(AnalyzeMaintenancePacksRequest {
            scan_id: scan.scan_id,
            batch_index: 0,
        })
        .await
        .expect("create a model-approved Pack merge proposal");

    assert_eq!(analysis.worker_calls, 1);
    assert_eq!(analysis.candidates.len(), 1);
    assert_eq!(
        analysis.candidates[0]
            .pages
            .iter()
            .map(|page| page.page_id.as_str())
            .collect::<Vec<_>>(),
        vec![first.page_id.as_str(), second.page_id.as_str()]
    );
    assert_eq!(worker.request_count(), 1);
    fixture.close().await;
}

#[tokio::test]
async fn semantic_retention_job_leases_only_worker_selected_revisions() {
    let fixture = Fixture::open("retention").await;
    let selected = fixture
        .client
        .write_page(fixture.page(
            "A durable decision that should survive ordinary Revision pruning.",
            "retention:1",
        ))
        .await
        .expect("write retention candidate");
    let current = fixture
        .client
        .revise_page(RevisePageRequest {
            page_id: selected.page_id.clone(),
            expected_revision_id: selected.revision_id.clone(),
            created_by: Actor {
                actor_type: ActorType::Model,
                actor_id: "model:maintenance-test".to_owned(),
            },
            lifecycle_status: LifecycleStatus::Active,
            observed_at: None,
            valid_from: None,
            valid_to: None,
            payload: Some(PagePayload {
                media_type: "text/markdown".to_owned(),
                content: "The current state after that decision.".to_owned(),
            }),
            source_refs: Vec::new(),
            facets: None,
            provenance: Vec::new(),
            initial_relations: Vec::new(),
            idempotency_key: Some("retention:2".to_owned()),
        })
        .await
        .expect("revise retention candidate Page");
    let worker = Arc::new(FakeWorker::new(vec![MaintenanceWorkerResponse::Retain {
        milestones: vec![RetentionMilestone {
            revision_id: selected.revision_id.clone(),
            reason: "Records a durable decision boundary".to_owned(),
        }],
    }]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = false;
    config.retention.enabled = true;
    config.retention.write_leases = true;
    config.retention.minimum_age_days = 0;
    config.retention.keep_recent_revisions_per_page = 0;
    let access = config.access_session(&fixture.identity_id);
    assert!(access.allows(&fixture.namespace, AccessPermission::Audit));
    assert!(access.allows(&fixture.namespace, AccessPermission::Write));
    let mut maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker, config);

    let report = maintainer
        .run_once()
        .await
        .expect("run semantic retention maintenance");

    assert_eq!(report.retention_leases_written, 1);
    let leases = fixture
        .client
        .active_revision_retention_leases(vec![fixture.namespace.clone()], 10)
        .await
        .expect("read active retention leases");
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0].revision_id, selected.revision_id);
    assert_ne!(leases[0].revision_id, current.revision_id);
    let plan = fixture
        .client
        .plan_revision_retention(PlanRevisionRetentionRequest {
            scopes: vec![fixture.namespace.clone()],
            policy: RetentionPolicy {
                minimum_age_days: 0,
                keep_recent_revisions_per_page: 0,
                sample_limit: 20,
            },
        })
        .await
        .expect("plan retention with semantic lease");
    assert_eq!(plan.active_retention_leases, 1);
    assert!(plan.protection_reasons.iter().any(|count| {
        count.reason == RetentionProtectionReason::ExplicitLease && count.revisions == 1
    }));
    fixture.close().await;
}

#[tokio::test]
async fn observe_mode_retention_can_plan_without_receiving_write_access() {
    let fixture = Fixture::open("retention-observe-access").await;
    let mut config = fixture.config();
    config.mode = MaintenanceMode::Observe;
    config.retention.enabled = true;
    config.retention.write_leases = true;

    let access = config.access_session(&fixture.identity_id);
    let client = EmbeddedPcpClient::shared(Arc::clone(&fixture.store), access.clone());

    assert!(access.allows(&fixture.namespace, AccessPermission::Audit));
    assert!(!access.allows(&fixture.namespace, AccessPermission::Write));
    assert!(!config.writes_retention_leases());
    let plan = client
        .plan_revision_retention(PlanRevisionRetentionRequest {
            scopes: vec![fixture.namespace.clone()],
            policy: RetentionPolicy::default(),
        })
        .await
        .expect("observe-mode retention can plan");
    assert_eq!(plan.scanned_pages, 0);
    fixture.close().await;
}

#[tokio::test]
async fn maintainer_packs_only_the_ordered_candidate_selected_by_the_worker() {
    let fixture = Fixture::open("packing").await;
    let first = fixture
        .client
        .write_page(fixture.sealed_event(
            "The runtime keeps one durable and auditable state.",
            "packing:1",
            1,
        ))
        .await
        .expect("write first packing Page");
    let second = fixture
        .client
        .write_page(fixture.sealed_event(
            "The same runtime state remains durable across restarts.",
            "packing:2",
            2,
        ))
        .await
        .expect("write second packing Page");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::Candidate {
            page_ids: vec![first.page_id.clone(), second.page_id.clone()],
        },
    ]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = true;
    let mut maintainer =
        RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);

    let report = maintainer
        .run_once()
        .await
        .expect("run packing maintenance");

    assert_eq!(report.worker_calls, 1);
    assert_eq!(report.packs_committed, 1);
    assert_eq!(worker.request_count(), 1);
    assert!(
        fixture
            .client
            .current_revision_id(first.page_id)
            .await
            .is_err()
    );
    assert!(
        fixture
            .client
            .current_revision_id(second.page_id)
            .await
            .is_err()
    );
    assert_eq!(
        fixture
            .client
            .page_count(Vec::new())
            .await
            .expect("count packed Pages"),
        1
    );
    let packed = fixture
        .client
        .durable_page_inventory(Vec::new())
        .await
        .expect("read packed inventory")
        .into_iter()
        .find(|page| page.media_type.as_deref() == Some(pcp_core::PACKED_PAGE_MEDIA_TYPE))
        .expect("packed Page inventory item");
    assert_eq!(packed.mutability, PageMutability::Revisioned);
    let third = fixture
        .client
        .write_page(fixture.sealed_event(
            "The continuous runtime discussion adds another detail.",
            "packing:3",
            3,
        ))
        .await
        .expect("write third packing Page");
    let fourth = fixture
        .client
        .write_page(fixture.sealed_event(
            "The same discussion remains continuous and auditable.",
            "packing:4",
            4,
        ))
        .await
        .expect("write fourth packing Page");
    let extension_worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::Candidate {
            page_ids: vec![
                packed.page_id.clone(),
                third.page_id.clone(),
                fourth.page_id.clone(),
            ],
        },
    ]));
    let mut extension_config = fixture.config();
    extension_config.summary.enabled = false;
    extension_config.packing.enabled = true;
    let mut extension_maintainer = RuntimeMaintainer::for_test(
        fixture.client.clone(),
        extension_worker.clone(),
        extension_config,
    );
    let extension_report = extension_maintainer
        .run_once()
        .await
        .expect("extend packed Page through maintenance");

    assert_eq!(extension_report.packs_committed, 1);
    assert_ne!(
        fixture
            .client
            .current_revision_id(packed.page_id.clone())
            .await
            .expect("read extended packed head"),
        packed.revision_id
    );
    assert_eq!(
        fixture
            .client
            .page_count(Vec::new())
            .await
            .expect("count extended packed Pages"),
        1
    );
    {
        let requests = extension_worker
            .requests
            .lock()
            .expect("fake worker requests");
        let MaintenanceWorkerRequest::SelectPacking { pages, .. } = &requests[0] else {
            panic!("expected packing selection request");
        };
        let anchor = pages
            .iter()
            .find(|page| page.page_id == packed.page_id)
            .expect("packed anchor routing page");
        assert!(anchor.routing_text.contains("Packed range boundary"));
        assert!(!anchor.routing_text.starts_with('{'));
    }
    fixture.close().await;
}

#[tokio::test]
async fn packing_scan_is_read_only_and_selected_apply_revalidates_exact_revisions() {
    let fixture = Fixture::open("packing-scan").await;
    let first = fixture
        .client
        .write_page(fixture.sealed_event(
            "The same bounded topic starts here.",
            "packing-scan:1",
            1,
        ))
        .await
        .expect("write first scan Page");
    let second = fixture
        .client
        .write_page(fixture.sealed_event(
            "The bounded topic continues without a semantic break.",
            "packing-scan:2",
            2,
        ))
        .await
        .expect("write second scan Page");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::PackingCandidates {
            candidates: vec![vec![first.page_id.clone(), second.page_id.clone()]],
        },
    ]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = true;
    config.relation.enabled = false;
    config.retention.enabled = false;
    let maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);

    let scan = maintainer
        .scan_packing_candidates()
        .await
        .expect("scan packing candidates");

    assert_eq!(scan.inspected_pages, 2);
    assert_eq!(scan.eligible_pages, 2);
    assert_eq!(scan.candidate_group_count, 1);
    assert_eq!(scan.estimated_model_calls, 1);
    assert_eq!(worker.request_count(), 0);
    assert_eq!(
        fixture
            .client
            .page_count(Vec::new())
            .await
            .expect("count Pages after scan"),
        2
    );
    let analysis = maintainer
        .analyze_packing_candidates(AnalyzeMaintenancePacksRequest {
            scan_id: scan.scan_id,
            batch_index: 0,
        })
        .await
        .expect("analyze packing candidates");
    assert_eq!(analysis.worker_calls, 1);
    assert_eq!(analysis.batch_index, 0);
    assert_eq!(analysis.batch_count, 1);
    assert_eq!(analysis.analyzed_group_count, 1);
    assert!(analysis.issue.is_none());
    assert_eq!(worker.request_count(), 1);
    assert_eq!(analysis.candidates.len(), 1);
    let candidate = &analysis.candidates[0];
    assert_eq!(candidate.source_span.start, 1);
    assert_eq!(candidate.source_span.end, 2);
    assert_eq!(candidate.resulting_entry_count, 2);
    assert!(!candidate.extends_existing_pack);

    maintainer
        .apply_pack_candidate(ApplyMaintenancePackRequest {
            candidate_id: candidate.candidate_id.clone(),
            pages: candidate
                .pages
                .iter()
                .map(|page| PageRevisionRef {
                    page_id: page.page_id.clone(),
                    revision_id: page.revision_id.clone(),
                })
                .collect(),
        })
        .await
        .expect("apply selected packing candidate");
    assert_eq!(
        fixture
            .client
            .page_count(Vec::new())
            .await
            .expect("count Pages after apply"),
        1
    );
    fixture.close().await;
}

#[tokio::test]
async fn packing_analysis_can_select_across_the_final_pack_size_boundary() {
    let fixture = Fixture::open("packing-analysis-window").await;
    let mut pages = Vec::new();
    for sequence in 1..=10 {
        pages.push(
            fixture
                .client
                .write_page(fixture.sealed_event(
                    &format!("One coherent episode, step {sequence}."),
                    &format!("packing-analysis-window:{sequence}"),
                    sequence,
                ))
                .await
                .expect("write analysis-window Page"),
        );
    }
    let selected_ids = vec![pages[7].page_id.clone(), pages[8].page_id.clone()];
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::PackingCandidates {
            candidates: vec![selected_ids.clone()],
        },
    ]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = true;
    config.packing.max_pages = 8;
    config.packing.analysis_window_pages = 32;
    config.relation.enabled = false;
    config.retention.enabled = false;
    let maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);

    let scan = maintainer
        .scan_packing_candidates()
        .await
        .expect("scan a window larger than one final Pack");
    assert_eq!(scan.eligible_pages, 10);
    assert_eq!(scan.candidate_group_count, 1);

    let analysis = maintainer
        .analyze_packing_candidates(AnalyzeMaintenancePacksRequest {
            scan_id: scan.scan_id,
            batch_index: 0,
        })
        .await
        .expect("select across the former eight-Page analysis boundary");
    assert!(analysis.issue.is_none());
    assert_eq!(analysis.candidates.len(), 1);
    assert_eq!(
        analysis.candidates[0]
            .pages
            .iter()
            .map(|page| page.page_id.clone())
            .collect::<Vec<_>>(),
        selected_ids
    );
    let requests = worker.requests.lock().expect("fake worker requests");
    let MaintenanceWorkerRequest::AnalyzePacking { groups, .. } = &requests[0] else {
        panic!("expected packing analysis request");
    };
    assert_eq!(groups[0].pages.len(), 10);
    drop(requests);
    fixture.close().await;
}

#[tokio::test]
async fn analyzed_pack_remains_applicable_after_another_pack_shifts_analysis_windows() {
    let fixture = Fixture::open("packing-apply-window-shift").await;
    let mut pages = Vec::new();
    for sequence in 1..=42 {
        pages.push(
            fixture
                .client
                .write_page(fixture.sealed_event(
                    &format!("One continuous source episode, entry {sequence}."),
                    &format!("packing-apply-window-shift:{sequence}"),
                    sequence,
                ))
                .await
                .expect("write window-shift Page"),
        );
    }
    let first_ids = pages[0..8]
        .iter()
        .map(|page| page.page_id.clone())
        .collect::<Vec<_>>();
    let second_ids = pages[38..42]
        .iter()
        .map(|page| page.page_id.clone())
        .collect::<Vec<_>>();
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::PackingCandidates {
            candidates: vec![first_ids],
        },
        MaintenanceWorkerResponse::PackingCandidates {
            candidates: vec![second_ids],
        },
    ]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = true;
    config.packing.max_pages = 8;
    config.packing.analysis_window_pages = 32;
    config.relation.enabled = false;
    config.retention.enabled = false;
    let maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker, config);

    let scan = maintainer
        .scan_packing_candidates()
        .await
        .expect("scan two analysis windows");
    assert_eq!(scan.candidate_group_count, 2);
    let first_analysis = maintainer
        .analyze_packing_candidates(AnalyzeMaintenancePacksRequest {
            scan_id: scan.scan_id.clone(),
            batch_index: 0,
        })
        .await
        .expect("analyze the first candidate window before it shifts");
    let second_analysis = maintainer
        .analyze_packing_candidates(AnalyzeMaintenancePacksRequest {
            scan_id: scan.scan_id,
            batch_index: 1,
        })
        .await
        .expect("analyze the second candidate window before it shifts");
    let analysis = first_analysis
        .candidates
        .into_iter()
        .chain(second_analysis.candidates)
        .collect::<Vec<_>>();
    assert_eq!(analysis.len(), 2);

    for candidate in &analysis {
        maintainer
            .apply_pack_candidate(ApplyMaintenancePackRequest {
                candidate_id: candidate.candidate_id.clone(),
                pages: candidate
                    .pages
                    .iter()
                    .map(|page| PageRevisionRef {
                        page_id: page.page_id.clone(),
                        revision_id: page.revision_id.clone(),
                    })
                    .collect(),
            })
            .await
            .expect("apply candidate independently of shifted analysis windows");
    }
    assert_eq!(
        fixture
            .client
            .page_count(Vec::new())
            .await
            .expect("count Pages after both Packs"),
        32
    );
    fixture.close().await;
}

#[tokio::test]
async fn packing_analysis_uses_one_group_per_worker_call() {
    let fixture = Fixture::open("packing-analysis-batch").await;
    let mut a1 = fixture.sealed_event("Topic A starts.", "packing-analysis:a1", 1);
    let mut a2 = fixture.sealed_event("Topic A continues.", "packing-analysis:a2", 2);
    a1.source_span.as_mut().expect("source span").stream_id = "conversation:a".to_owned();
    a2.source_span.as_mut().expect("source span").stream_id = "conversation:a".to_owned();
    fixture.client.write_page(a1).await.expect("write A1");
    fixture.client.write_page(a2).await.expect("write A2");
    let mut b1 = fixture.sealed_event("Topic B starts.", "packing-analysis:b1", 1);
    let mut b2 = fixture.sealed_event("Topic B continues.", "packing-analysis:b2", 2);
    b1.source_span.as_mut().expect("source span").stream_id = "conversation:b".to_owned();
    b2.source_span.as_mut().expect("source span").stream_id = "conversation:b".to_owned();
    fixture.client.write_page(b1).await.expect("write B1");
    fixture.client.write_page(b2).await.expect("write B2");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::NoCandidate,
        MaintenanceWorkerResponse::NoCandidate,
    ]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = true;
    config.relation.enabled = false;
    config.retention.enabled = false;
    let maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);

    let scan = maintainer
        .scan_packing_candidates()
        .await
        .expect("scan all packing groups");
    assert_eq!(scan.candidate_group_count, 2);
    assert_eq!(scan.estimated_model_calls, 2);
    assert_eq!(worker.request_count(), 0);

    let first_analysis = maintainer
        .analyze_packing_candidates(AnalyzeMaintenancePacksRequest {
            scan_id: scan.scan_id.clone(),
            batch_index: 0,
        })
        .await
        .expect("analyze the first packing group");
    assert_eq!(first_analysis.worker_calls, 1);
    assert_eq!(first_analysis.batch_count, 2);
    assert_eq!(first_analysis.analyzed_group_count, 1);
    assert!(first_analysis.issue.is_none());
    assert_eq!(first_analysis.no_candidate_groups, 1);
    assert!(first_analysis.candidates.is_empty());

    let second_analysis = maintainer
        .analyze_packing_candidates(AnalyzeMaintenancePacksRequest {
            scan_id: scan.scan_id,
            batch_index: 1,
        })
        .await
        .expect("analyze the second packing group");
    assert_eq!(second_analysis.worker_calls, 1);
    assert_eq!(second_analysis.batch_count, 2);
    assert_eq!(second_analysis.analyzed_group_count, 1);
    assert!(second_analysis.issue.is_none());
    assert_eq!(second_analysis.no_candidate_groups, 1);
    assert!(second_analysis.candidates.is_empty());
    let requests = worker.requests.lock().expect("fake worker requests");
    assert_eq!(requests.len(), 2);
    for request in requests.iter() {
        let MaintenanceWorkerRequest::AnalyzePacking { groups, .. } = request else {
            panic!("expected packing analysis request");
        };
        assert_eq!(groups.len(), 1);
    }
    drop(requests);
    fixture.close().await;
}

#[tokio::test]
async fn packing_analysis_defers_an_invalid_model_selection_without_failing_the_batch() {
    let fixture = Fixture::open("packing-analysis-invalid-selection").await;
    fixture
        .client
        .write_page(fixture.sealed_event(
            "A bounded topic starts here.",
            "packing-analysis-invalid:1",
            1,
        ))
        .await
        .expect("write first Page");
    fixture
        .client
        .write_page(fixture.sealed_event(
            "The same bounded topic continues.",
            "packing-analysis-invalid:2",
            2,
        ))
        .await
        .expect("write second Page");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::PackingCandidates {
            candidates: vec![vec![
                "pg_not_supplied".to_owned(),
                "pg_also_missing".to_owned(),
            ]],
        },
    ]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = true;
    config.relation.enabled = false;
    config.retention.enabled = false;
    let maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker, config);
    let scan = maintainer
        .scan_packing_candidates()
        .await
        .expect("scan packing candidates");

    let analysis = maintainer
        .analyze_packing_candidates(AnalyzeMaintenancePacksRequest {
            scan_id: scan.scan_id,
            batch_index: 0,
        })
        .await
        .expect("invalid model output becomes a batch issue");

    assert!(analysis.candidates.is_empty());
    assert_eq!(analysis.worker_calls, 1);
    assert_eq!(analysis.overlap_retries, 0);
    assert_eq!(analysis.deferred_groups, 1);
    assert_eq!(analysis.no_candidate_groups, 0);
    let issue = analysis.issue.expect("invalid selection issue");
    assert_eq!(issue.code, "invalid_model_selection");
    assert!(issue.message.contains("outside one supplied group"));
    fixture.close().await;
}

#[tokio::test]
async fn packing_analysis_repairs_overlapping_candidates_once() {
    let fixture = Fixture::open("packing-analysis-overlap-repair").await;
    let first = fixture
        .client
        .write_page(fixture.sealed_event(
            "A coherent episode starts here.",
            "packing-analysis-overlap:1",
            1,
        ))
        .await
        .expect("write first Page");
    let second = fixture
        .client
        .write_page(fixture.sealed_event(
            "The same episode adds its key qualification.",
            "packing-analysis-overlap:2",
            2,
        ))
        .await
        .expect("write second Page");
    let third = fixture
        .client
        .write_page(fixture.sealed_event(
            "The episode reaches its bounded conclusion.",
            "packing-analysis-overlap:3",
            3,
        ))
        .await
        .expect("write third Page");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::PackingCandidates {
            candidates: vec![
                vec![first.page_id.clone(), second.page_id.clone()],
                vec![second.page_id.clone(), third.page_id.clone()],
            ],
        },
        MaintenanceWorkerResponse::PackingCandidates {
            candidates: vec![vec![
                first.page_id.clone(),
                second.page_id.clone(),
                third.page_id.clone(),
            ]],
        },
    ]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = true;
    config.relation.enabled = false;
    config.retention.enabled = false;
    let maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);
    let scan = maintainer
        .scan_packing_candidates()
        .await
        .expect("scan packing candidates");

    let analysis = maintainer
        .analyze_packing_candidates(AnalyzeMaintenancePacksRequest {
            scan_id: scan.scan_id,
            batch_index: 0,
        })
        .await
        .expect("repair overlapping model candidates");

    assert_eq!(analysis.worker_calls, 2);
    assert_eq!(analysis.overlap_retries, 1);
    assert_eq!(analysis.deferred_groups, 0);
    assert!(analysis.issue.is_none());
    assert_eq!(analysis.candidates.len(), 1);
    assert_eq!(
        analysis.candidates[0]
            .pages
            .iter()
            .map(|page| page.page_id.clone())
            .collect::<Vec<_>>(),
        vec![first.page_id, second.page_id, third.page_id]
    );
    assert_eq!(worker.request_count(), 2);
    fixture.close().await;
}

#[tokio::test]
async fn packing_scan_keeps_pages_connected_by_provenance_eligible() {
    let fixture = Fixture::open("packing-provenance-protection").await;
    let first = fixture
        .client
        .write_page(fixture.sealed_event(
            "A source event that remains independently referenced.",
            "packing-provenance-protection:1",
            1,
        ))
        .await
        .expect("write provenance source");
    let mut derived = fixture.sealed_event(
        "A derived event with explicit provenance.",
        "packing-provenance-protection:2",
        2,
    );
    derived.provenance = vec![ProvenanceEvent {
        operation: "derive".to_owned(),
        actor: Actor {
            actor_type: ActorType::Model,
            actor_id: "model:maintenance-test".to_owned(),
        },
        timestamp: "2026-08-16T00:00:00Z".to_owned(),
        input_revision_ids: vec![first.revision_id],
        tool_or_model: Some("test-model".to_owned()),
    }];
    fixture
        .client
        .write_page(derived)
        .await
        .expect("write provenance-derived Page");
    fixture
        .client
        .write_page(fixture.sealed_event(
            "A trailing unreferenced event cannot form a Pack alone.",
            "packing-provenance-protection:3",
            3,
        ))
        .await
        .expect("write trailing Page");
    let worker = Arc::new(FakeWorker::new(Vec::new()));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = true;
    config.relation.enabled = false;
    config.retention.enabled = false;
    let maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);

    let scan = maintainer
        .scan_packing_candidates()
        .await
        .expect("scan with provenance protection");

    assert_eq!(scan.inspected_pages, 3);
    assert_eq!(scan.candidate_group_count, 1);
    assert_eq!(scan.eligible_pages, 3);
    assert_eq!(worker.request_count(), 0);
    fixture.close().await;
}

#[tokio::test]
async fn bounded_cycle_reloads_inventory_and_commits_multiple_pack_windows() {
    let fixture = Fixture::open("packing-converge").await;
    let mut a1 = fixture.sealed_event("Topic A starts.", "packing-converge:a1", 1);
    let mut a2 = fixture.sealed_event("Topic A continues.", "packing-converge:a2", 2);
    a1.source_span.as_mut().expect("source span").stream_id = "conversation:a".to_owned();
    a2.source_span.as_mut().expect("source span").stream_id = "conversation:a".to_owned();
    let a1 = fixture.client.write_page(a1).await.expect("write A1");
    let a2 = fixture.client.write_page(a2).await.expect("write A2");
    let mut b1 = fixture.sealed_event("Topic B starts.", "packing-converge:b1", 1);
    let mut b2 = fixture.sealed_event("Topic B continues.", "packing-converge:b2", 2);
    b1.source_span.as_mut().expect("source span").stream_id = "conversation:b".to_owned();
    b2.source_span.as_mut().expect("source span").stream_id = "conversation:b".to_owned();
    let b1 = fixture.client.write_page(b1).await.expect("write B1");
    let b2 = fixture.client.write_page(b2).await.expect("write B2");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::Candidate {
            page_ids: vec![b1.page_id, b2.page_id],
        },
        MaintenanceWorkerResponse::Candidate {
            page_ids: vec![a1.page_id, a2.page_id],
        },
    ]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = true;
    config.relation.enabled = false;
    config.retention.enabled = false;
    config.max_jobs_per_cycle = 2;
    let mut maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker, config);

    let report = maintainer
        .run_bounded_cycle()
        .await
        .expect("run bounded maintenance convergence");

    assert_eq!(report.worker_calls, 2);
    assert_eq!(report.packs_committed, 2);
    assert_eq!(
        fixture
            .client
            .page_count(Vec::new())
            .await
            .expect("count converged packed Pages"),
        2
    );
    fixture.close().await;
}

#[tokio::test]
async fn observe_mode_records_a_packing_proposal_without_replacing_pages() {
    let fixture = Fixture::open("observe-packing").await;
    let first = fixture
        .client
        .write_page(fixture.sealed_event("One durable runtime state.", "observe-pack:1", 1))
        .await
        .expect("write first observed Page");
    let second = fixture
        .client
        .write_page(fixture.sealed_event(
            "The same durable runtime state after restart.",
            "observe-pack:2",
            2,
        ))
        .await
        .expect("write second observed Page");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::Candidate {
            page_ids: vec![first.page_id.clone(), second.page_id.clone()],
        },
        // Observe mode still exhausts the Pack pass before moving on.
        MaintenanceWorkerResponse::NoCandidate,
    ]));
    let mut config = fixture.config();
    config.mode = MaintenanceMode::Observe;
    config.summary.enabled = false;
    config.packing.enabled = true;
    let mut maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker, config);

    let report = maintainer
        .run_once()
        .await
        .expect("observe packing maintenance");

    assert_eq!(report.packs_proposed, 1);
    assert_eq!(report.packs_committed, 0);
    assert_eq!(
        fixture
            .client
            .current_revision_id(first.page_id)
            .await
            .expect("read first observed head"),
        first.revision_id
    );
    assert_eq!(
        fixture
            .client
            .current_revision_id(second.page_id)
            .await
            .expect("read second observed head"),
        second.revision_id
    );
    fixture.close().await;
}

#[tokio::test]
async fn invalid_packing_selection_defers_and_cools_down_the_window() {
    let fixture = Fixture::open("invalid-packing-selection").await;
    let first = fixture
        .client
        .write_page(fixture.sealed_event(
            "First contiguous packing candidate.",
            "invalid-pack:1",
            1,
        ))
        .await
        .expect("write first packing candidate");
    fixture
        .client
        .write_page(fixture.sealed_event(
            "Second contiguous packing candidate.",
            "invalid-pack:2",
            2,
        ))
        .await
        .expect("write second packing candidate");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::Candidate {
            page_ids: vec![first.page_id.clone(), "pg_not_offered".to_owned()],
        },
    ]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = true;
    let mut maintainer =
        RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);

    let report = maintainer
        .run_once()
        .await
        .expect("defer invalid packing selection");
    assert_eq!(report.worker_calls, 1);
    assert_eq!(report.packs_committed, 0);
    assert_eq!(report.deferred, 1);

    let second_cycle = maintainer
        .run_once()
        .await
        .expect("skip cooled down packing window");
    assert_eq!(second_cycle.worker_calls, 0);
    assert_eq!(worker.request_count(), 1);
    assert_eq!(
        fixture
            .client
            .current_revision_id(first.page_id)
            .await
            .expect("read unchanged packing candidate"),
        first.revision_id
    );
    fixture.close().await;
}

#[tokio::test]
async fn relation_runs_after_packing_against_the_refreshed_inventory() {
    let fixture = Fixture::open("pack-before-relation").await;
    let first = fixture
        .client
        .write_page(fixture.sealed_event(
            "First contiguous stream Page.",
            "pack-before-relation:1",
            1,
        ))
        .await
        .expect("write first stream Page");
    let second = fixture
        .client
        .write_page(fixture.sealed_event(
            "Second contiguous stream Page.",
            "pack-before-relation:2",
            2,
        ))
        .await
        .expect("write second stream Page");
    let document_a = fixture
        .client
        .write_page(fixture.page(
            "First durable relation document.",
            "pack-before-relation:document-a",
        ))
        .await
        .expect("write first relation document");
    let document_b = fixture
        .client
        .write_page(fixture.page(
            "Second durable relation document.",
            "pack-before-relation:document-b",
        ))
        .await
        .expect("write second relation document");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::Candidate {
            page_ids: vec![first.page_id, second.page_id],
        },
        MaintenanceWorkerResponse::Relate {
            page_ids: [document_a.page_id, document_b.page_id],
        },
    ]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = true;
    config.relation.enabled = true;
    let mut maintainer =
        RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);

    let report = maintainer
        .run_once()
        .await
        .expect("run ordered Pack and Relation maintenance");
    assert_eq!(report.worker_calls, 2);
    assert_eq!(report.packs_committed, 1);
    assert_eq!(report.relations_committed, 1);
    assert_eq!(worker.request_count(), 2);
    fixture.close().await;
}

#[tokio::test]
async fn relation_maintains_a_source_page_without_a_pack_window() {
    let fixture = Fixture::open("relation-singleton-source-page").await;
    let source_page = fixture
        .client
        .write_page(fixture.sealed_event(
            "A one-page source event that cannot form a Pack by itself.",
            "relation-singleton-source-page:1",
            1,
        ))
        .await
        .expect("write singleton source Page");
    let document = fixture
        .client
        .write_page(fixture.page(
            "A durable document that directly explains the source event.",
            "relation-singleton-source-page:document",
        ))
        .await
        .expect("write durable relation Page");
    let worker = Arc::new(FakeWorker::new(vec![MaintenanceWorkerResponse::Relate {
        page_ids: [source_page.page_id.clone(), document.page_id.clone()],
    }]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = true;
    config.relation.enabled = true;
    config.retention.enabled = false;
    let mut maintainer =
        RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);

    let report = maintainer
        .run_once()
        .await
        .expect("route an unpackable source Page to relation maintenance");

    assert_eq!(report.worker_calls, 1);
    assert_eq!(report.packs_committed, 0);
    assert_eq!(report.relations_committed, 1);
    let requests = worker.requests.lock().expect("read relation request");
    let MaintenanceWorkerRequest::SelectRelation { pages, .. } = &requests[0] else {
        panic!("expected relation selection for the unpackable source Page");
    };
    assert!(pages.iter().any(|page| page.page_id == source_page.page_id));
    fixture.close().await;
}

#[tokio::test]
async fn relation_maintains_source_pages_after_packing_declines_their_window() {
    let fixture = Fixture::open("relation-after-pack-decline").await;
    let first = fixture
        .client
        .write_page(fixture.sealed_event(
            "A first source event in an otherwise packable local episode.",
            "relation-after-pack-decline:1",
            1,
        ))
        .await
        .expect("write first source Page");
    fixture
        .client
        .write_page(fixture.sealed_event(
            "A second source event in an otherwise packable local episode.",
            "relation-after-pack-decline:2",
            2,
        ))
        .await
        .expect("write second source Page");
    let document = fixture
        .client
        .write_page(fixture.page(
            "A durable document with the stable interpretation of the first source event.",
            "relation-after-pack-decline:document",
        ))
        .await
        .expect("write durable relation Page");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::NoCandidate,
        MaintenanceWorkerResponse::Relate {
            page_ids: [first.page_id.clone(), document.page_id.clone()],
        },
    ]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = true;
    config.relation.enabled = true;
    config.retention.enabled = false;
    let mut maintainer =
        RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);

    let report = maintainer
        .run_once()
        .await
        .expect("route the declined Pack window to relation maintenance");
    assert_eq!(report.worker_calls, 2);
    assert_eq!(report.packs_committed, 0);
    assert_eq!(report.relations_committed, 1);
    let requests = worker.requests.lock().expect("read maintenance requests");
    let MaintenanceWorkerRequest::SelectRelation { pages, .. } = &requests[1] else {
        panic!("expected relation selection after the Pack worker declined");
    };
    assert!(pages.iter().any(|page| page.page_id == first.page_id));
    fixture.close().await;
}

#[tokio::test]
async fn maintainer_links_only_an_offered_pair_with_runtime_owned_relation_semantics() {
    let fixture = Fixture::open("relation").await;
    let first = fixture
        .client
        .write_page(fixture.page(
            "PCP keeps durable context outside the active model window.",
            "relation:1",
        ))
        .await
        .expect("write first relation candidate");
    let second = fixture
        .client
        .write_page(fixture.page(
            "The runtime recalls durable context into a bounded active window.",
            "relation:2",
        ))
        .await
        .expect("write second relation candidate");
    let worker = Arc::new(FakeWorker::new(vec![MaintenanceWorkerResponse::Relate {
        page_ids: [second.page_id.clone(), first.page_id.clone()],
    }]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = false;
    config.relation.enabled = true;
    let mut maintainer =
        RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);

    let report = maintainer
        .run_once()
        .await
        .expect("run relation maintenance");

    assert_eq!(report.worker_calls, 1);
    assert_eq!(report.relations_committed, 1);
    assert_eq!(report.relations_proposed, 0);
    let pages = fixture
        .client
        .read_pages(ReadPagesRequest {
            page_ids: vec![first.page_id.clone()],
            revision_ids: Vec::new(),
            projections: vec![Projection::Manifest, Projection::Relations],
            max_chars: 1,
        })
        .await
        .expect("read maintained relation");
    let relation = pages[0]
        .relations
        .iter()
        .find(|relation| relation.relation_type == "related_to")
        .expect("related_to relation");
    let mut expected_basis = vec![first.revision_id.clone(), second.revision_id.clone()];
    expected_basis.sort();
    assert_eq!(relation.basis_revision_ids, expected_basis);
    assert_eq!(relation.created_by.actor_id, "model:maintenance-test");

    let second_cycle = maintainer
        .run_once()
        .await
        .expect("skip the relation window after selecting one durable edge");
    assert_eq!(second_cycle.worker_calls, 0);
    assert_eq!(second_cycle.relations_committed, 0);
    assert_eq!(second_cycle.deferred, 0);
    assert_eq!(worker.request_count(), 1);
    fixture.close().await;
}

#[tokio::test]
async fn reviewed_relation_is_target_bound_and_applies_only_once() {
    let fixture = Fixture::open("reviewed-relation").await;
    let first = fixture
        .client
        .write_page(fixture.page(
            "PCP keeps durable context outside the active model window.",
            "reviewed-relation:1",
        ))
        .await
        .expect("write reviewed relation source");
    let second = fixture
        .client
        .write_page(fixture.page(
            "The runtime restores relevant durable context into a bounded active window.",
            "reviewed-relation:2",
        ))
        .await
        .expect("write reviewed relation target");
    let worker = Arc::new(FakeWorker::new(vec![MaintenanceWorkerResponse::Relate {
        page_ids: [first.page_id.clone(), second.page_id.clone()],
    }]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = false;
    config.relation.enabled = true;
    let maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker, config);
    let scan = maintainer
        .scan_maintenance_work()
        .await
        .expect("scan relation review windows");
    let group = scan
        .relation
        .groups
        .first()
        .expect("one relation review window");

    let analysis = maintainer
        .analyze_relation_candidate(AnalyzeMaintenanceRelationRequest {
            scan_id: scan.relation.scan_id,
            group_id: group.group_id.clone(),
        })
        .await
        .expect("analyze relation review");
    let candidate = analysis.candidate.expect("relation proposal");
    assert!(
        candidate
            .pages
            .iter()
            .any(|page| page.page_id == first.page_id)
    );
    let request = ApplyMaintenanceRelationRequest {
        candidate_id: candidate.candidate_id,
        pages: candidate.pages.map(|page| PageRevisionRef {
            page_id: page.page_id,
            revision_id: page.revision_id,
        }),
    };
    maintainer
        .apply_relation_candidate(request.clone())
        .await
        .expect("apply reviewed relation");
    let pages = fixture
        .client
        .read_pages(ReadPagesRequest {
            page_ids: vec![first.page_id],
            revision_ids: Vec::new(),
            projections: vec![Projection::Manifest, Projection::Relations],
            max_chars: 1,
        })
        .await
        .expect("read applied relation");
    assert!(pages[0].relations.iter().any(|relation| {
        relation.relation_type == "related_to"
            && (relation.to_page_id == second.page_id || relation.from_page_id == second.page_id)
    }));
    assert!(maintainer.apply_relation_candidate(request).await.is_err());
    fixture.close().await;
}

#[tokio::test]
async fn observe_relation_records_a_proposal_without_linking_pages() {
    let fixture = Fixture::open("observe-relation").await;
    let first = fixture
        .client
        .write_page(fixture.page("A durable protocol decision.", "observe-relation:1"))
        .await
        .expect("write first observed relation candidate");
    let second = fixture
        .client
        .write_page(fixture.page(
            "The implementation follows that durable protocol decision.",
            "observe-relation:2",
        ))
        .await
        .expect("write second observed relation candidate");
    let worker = Arc::new(FakeWorker::new(vec![MaintenanceWorkerResponse::Relate {
        page_ids: [first.page_id.clone(), second.page_id.clone()],
    }]));
    let mut config = fixture.config();
    config.mode = MaintenanceMode::Observe;
    config.summary.enabled = false;
    config.packing.enabled = false;
    config.relation.enabled = true;
    let mut maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker, config);

    let report = maintainer
        .run_once()
        .await
        .expect("observe relation maintenance");

    assert_eq!(report.relations_proposed, 1);
    assert_eq!(report.relations_committed, 0);
    let pages = fixture
        .client
        .read_pages(ReadPagesRequest {
            page_ids: vec![first.page_id],
            revision_ids: Vec::new(),
            projections: vec![Projection::Manifest, Projection::Relations],
            max_chars: 1,
        })
        .await
        .expect("read observed relation candidate");
    assert!(pages[0].relations.is_empty());
    fixture.close().await;
}

#[tokio::test]
async fn scheduled_relation_waits_for_review_before_it_is_asserted() {
    let fixture = Fixture::open("scheduled-relation-review").await;
    let first = fixture
        .client
        .write_page(fixture.sealed_event("First stable source event.", "scheduled-relation:1", 1))
        .await
        .expect("write first source event");
    let second = fixture
        .client
        .write_page(fixture.sealed_event("Second stable source event.", "scheduled-relation:2", 2))
        .await
        .expect("write second source event");
    let worker = Arc::new(FakeWorker::new(vec![MaintenanceWorkerResponse::Relate {
        page_ids: [first.page_id.clone(), second.page_id.clone()],
    }]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = false;
    config.relation.enabled = true;
    config.max_jobs_per_cycle = 1;
    config.write_trigger.min_new_pages = 1;
    config.write_trigger.quiet_period_seconds = 0;
    config.write_trigger.max_wait_seconds = 0;
    let state_path = config.state_path.clone();
    let status_config = config.clone();
    let mut maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker, config);

    let baseline = maintainer
        .run_scheduled_cycle()
        .await
        .expect("establish write watermark");
    assert_eq!(baseline.worker_calls, 0);
    fixture
        .client
        .write_page(fixture.sealed_event(
            "Third event makes the stream ready.",
            "scheduled-relation:3",
            3,
        ))
        .await
        .expect("write triggering source event");

    let report = maintainer
        .run_scheduled_cycle()
        .await
        .expect("schedule conservative relation review");
    assert_eq!(report.relations_proposed, 1);
    assert_eq!(report.relations_committed, 0);
    let automation = RuntimeMaintainer::automation_status(&status_config)
        .await
        .expect("read persisted automatic maintenance state");
    assert!(matches!(
        automation.state,
        MaintenanceAutomationState::Waiting
    ));
    assert_eq!(
        automation
            .last_report
            .expect("scheduled report")
            .relations_proposed,
        1
    );
    assert_eq!(automation.pending_relation_review_count, 1);
    let reviews = maintainer.pending_relation_reviews();
    assert_eq!(reviews.len(), 1);
    let reviewed_page_ids = reviews[0]
        .pages
        .iter()
        .map(|page| page.page_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        reviewed_page_ids,
        std::collections::BTreeSet::from([first.page_id.as_str(), second.page_id.as_str()])
    );
    assert!(!reviews[0].pages[0].preview.is_empty());
    let restored_ledger = super::ledger::MaintenanceLedger::load(&state_path)
        .await
        .expect("reload persisted review queue");
    assert_eq!(restored_ledger.relation_reviews().len(), 1);
    let pages = fixture
        .client
        .read_pages(ReadPagesRequest {
            page_ids: vec![first.page_id.clone()],
            revision_ids: Vec::new(),
            projections: vec![Projection::Manifest, Projection::Relations],
            max_chars: 1,
        })
        .await
        .expect("read scheduled relation source");
    assert!(pages[0].relations.is_empty());
    let relation = maintainer
        .approve_relation_review(&reviews[0].candidate_id)
        .await
        .expect("approve scheduled relation");
    assert_eq!(relation.relation_type, "related_to");
    assert!(maintainer.pending_relation_reviews().is_empty());
    let pages = fixture
        .client
        .read_pages(ReadPagesRequest {
            page_ids: vec![first.page_id],
            revision_ids: Vec::new(),
            projections: vec![Projection::Manifest, Projection::Relations],
            max_chars: 1,
        })
        .await
        .expect("read approved relation source");
    assert_eq!(pages[0].relations.len(), 1);
    fixture.close().await;
}

#[tokio::test]
async fn scheduled_low_risk_pack_boundary_relation_is_asserted_automatically() {
    let fixture = Fixture::open("scheduled-low-risk-pack-relation").await;
    let mut leaves = Vec::new();
    for sequence in 1..=4 {
        leaves.push(
            fixture
                .client
                .write_page(fixture.sealed_event(
                    &format!("OET continuity evidence entry {sequence}."),
                    &format!("scheduled-low-risk-pack-relation:{sequence}"),
                    sequence,
                ))
                .await
                .expect("write Pack boundary source"),
        );
    }
    let first_pack = fixture
        .client
        .pack_pages(PackPagesRequest {
            pages: leaves[..2]
                .iter()
                .map(|page| PageRevisionRef {
                    page_id: page.page_id.clone(),
                    revision_id: page.revision_id.clone(),
                })
                .collect(),
            idempotency_key: Some("scheduled-low-risk-pack-relation:first".to_owned()),
        })
        .await
        .expect("pack first OET boundary");
    let second_pack = fixture
        .client
        .pack_pages(PackPagesRequest {
            pages: leaves[2..]
                .iter()
                .map(|page| PageRevisionRef {
                    page_id: page.page_id.clone(),
                    revision_id: page.revision_id.clone(),
                })
                .collect(),
            idempotency_key: Some("scheduled-low-risk-pack-relation:second".to_owned()),
        })
        .await
        .expect("pack second OET boundary");
    let worker = Arc::new(FakeWorker::new(vec![MaintenanceWorkerResponse::Relate {
        page_ids: [first_pack.page_id.clone(), second_pack.page_id.clone()],
    }]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = false;
    config.relation.enabled = true;
    config.max_jobs_per_cycle = 1;
    config.write_trigger.min_new_pages = 1;
    config.write_trigger.quiet_period_seconds = 0;
    config.write_trigger.max_wait_seconds = 0;
    let mut maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker, config);

    maintainer
        .run_scheduled_cycle()
        .await
        .expect("establish Pack-boundary write watermark");
    fixture
        .client
        .write_page(fixture.sealed_event(
            "A later OET write makes the source stream eligible.",
            "scheduled-low-risk-pack-relation:trigger",
            5,
        ))
        .await
        .expect("write triggering source event");

    let report = maintainer
        .run_scheduled_cycle()
        .await
        .expect("run low-risk automatic relation");
    assert_eq!(report.relations_committed, 1);
    assert_eq!(report.relations_proposed, 0);
    assert!(maintainer.pending_relation_reviews().is_empty());
    let pages = fixture
        .client
        .read_pages(ReadPagesRequest {
            page_ids: vec![first_pack.page_id.clone(), second_pack.page_id.clone()],
            revision_ids: Vec::new(),
            projections: vec![Projection::Relations],
            max_chars: 1,
        })
        .await
        .expect("read asserted automatic relation");
    assert!(pages.iter().any(|page| {
        let counterpart = if page.page.page_id == first_pack.page_id {
            &second_pack.page_id
        } else {
            &first_pack.page_id
        };
        page.relations.iter().any(|relation| {
            relation.relation_type == "related_to" && relation.to_page_id == *counterpart
        })
    }));
    fixture.close().await;
}

#[tokio::test]
async fn automatic_cycle_exhausts_packing_before_entering_semantic_maintenance() {
    let fixture = Fixture::open("automatic-pack-first").await;
    let mut pages = Vec::new();
    for sequence in 1..=4 {
        pages.push(
            fixture
                .client
                .write_page(fixture.sealed_event(
                    &format!("PCP Pack-first source event {sequence}."),
                    &format!("automatic-pack-first:{sequence}"),
                    sequence,
                ))
                .await
                .expect("write Pack-first source"),
        );
    }
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::Candidate {
            page_ids: pages[..2].iter().map(|page| page.page_id.clone()).collect(),
        },
        MaintenanceWorkerResponse::NoCandidate,
    ]));
    let mut config = fixture.config();
    config.summary.minimum_chars = 1;
    config.packing.enabled = true;
    config.relation.enabled = false;
    config.max_jobs_per_cycle = 2;
    let mut maintainer =
        RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);

    let report = maintainer
        .run_once()
        .await
        .expect("run Pack-first automatic maintenance");
    assert_eq!(report.packs_committed, 1);
    assert_eq!(report.summaries_written, 0);
    assert!(
        worker
            .requests
            .lock()
            .expect("inspect worker requests")
            .iter()
            .all(|request| matches!(request, MaintenanceWorkerRequest::SelectPacking { .. }))
    );
    fixture.close().await;
}

#[tokio::test]
async fn scheduled_write_trigger_applies_a_stable_large_page_summary() {
    let fixture = Fixture::open("scheduled-summary").await;
    let baseline = fixture
        .client
        .write_page(fixture.page(
            "Existing Page establishes the watermark.",
            "scheduled-summary:1",
        ))
        .await
        .expect("write baseline Page");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::WriteSummary {
            content: "这是一段稳定页面的自动摘要。".to_owned(),
        },
    ]));
    let mut config = fixture.config();
    config.packing.enabled = false;
    config.relation.enabled = false;
    config.summary.minimum_chars = 1;
    config.write_trigger.min_new_pages = 1;
    config.write_trigger.quiet_period_seconds = 0;
    config.write_trigger.max_wait_seconds = 0;
    let mut maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker, config);
    maintainer
        .run_scheduled_cycle()
        .await
        .expect("establish write watermark");
    let written = fixture
        .client
        .write_page(
            fixture.page(
                "这是达到稳定窗口后应自动写入摘要的长页面。"
                    .repeat(24)
                    .as_str(),
                "scheduled-summary:2",
            ),
        )
        .await
        .expect("write triggering Page");

    let report = maintainer
        .run_scheduled_cycle()
        .await
        .expect("run scheduled Summary maintenance");
    assert_eq!(report.summaries_written, 1);
    let pages = fixture
        .client
        .read_pages(ReadPagesRequest {
            page_ids: vec![written.page_id],
            revision_ids: Vec::new(),
            projections: vec![Projection::Manifest, Projection::Summary],
            max_chars: 1,
        })
        .await
        .expect("read scheduled Summary result");
    assert!(pages[0].summary.is_some());
    let _ = baseline;
    fixture.close().await;
}

#[tokio::test]
async fn relation_selection_excludes_pairs_already_connected_by_a_path() {
    let fixture = Fixture::open("relation-path-exclusion").await;
    let first = fixture
        .client
        .write_page(fixture.page("First durable topic Page.", "relation-path:1"))
        .await
        .expect("write first relation Page");
    let second = fixture
        .client
        .write_page(fixture.page("Second durable topic Page.", "relation-path:2"))
        .await
        .expect("write second relation Page");
    let third = fixture
        .client
        .write_page(fixture.page("Third durable topic Page.", "relation-path:3"))
        .await
        .expect("write third relation Page");
    for (index, (from, to)) in [(&first, &second), (&second, &third)]
        .into_iter()
        .enumerate()
    {
        fixture
            .client
            .link_pages(LinkPagesRequest {
                from_page_id: from.page_id.clone(),
                relation_type: "related_to".to_owned(),
                to_page_id: to.page_id.clone(),
                basis_revision_ids: vec![from.revision_id.clone(), to.revision_id.clone()],
                created_by: Actor {
                    actor_type: ActorType::Tool,
                    actor_id: "tool:relation-path-test".to_owned(),
                },
                idempotency_key: Some(format!("relation-path:{index}")),
            })
            .await
            .expect("link relation path");
    }
    let worker = Arc::new(FakeWorker::new(vec![MaintenanceWorkerResponse::Relate {
        page_ids: [first.page_id.clone(), third.page_id.clone()],
    }]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = false;
    config.relation.enabled = true;
    let mut maintainer =
        RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);

    let report = maintainer
        .run_once()
        .await
        .expect("defer transitively connected relation pair");
    assert_eq!(report.worker_calls, 1);
    assert_eq!(report.relations_committed, 0);
    assert_eq!(report.deferred, 1);

    let requests = worker.requests.lock().expect("fake worker requests");
    let MaintenanceWorkerRequest::SelectRelation {
        excluded_page_pairs,
        ..
    } = &requests[0]
    else {
        panic!("expected relation selection request");
    };
    let mut expected_pair = [first.page_id, third.page_id];
    expected_pair.sort();
    assert!(excluded_page_pairs.contains(&expected_pair));
    drop(requests);
    fixture.close().await;
}

#[tokio::test]
async fn relation_window_reads_existing_pairs_in_store_sized_batches() {
    let fixture = Fixture::open("relation-read-batches").await;
    for index in 0..24 {
        fixture
            .client
            .write_page(fixture.page(
                &format!("Durable relation candidate {index}."),
                &format!("relation-read-batches:{index}"),
            ))
            .await
            .expect("write relation batch candidate");
    }
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::NoCandidate,
    ]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = false;
    config.relation.enabled = true;
    config.relation.candidate_window = 24;
    let mut maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker, config);

    let report = maintainer
        .run_once()
        .await
        .expect("read relation window in bounded Store batches");

    assert_eq!(report.worker_calls, 1);
    fixture.close().await;
}

#[tokio::test]
async fn maintainer_rejects_a_relation_page_outside_the_offered_window() {
    let fixture = Fixture::open("relation-outside-window").await;
    fixture
        .client
        .write_page(fixture.page("First offered Page.", "relation-window:1"))
        .await
        .expect("write first offered Page");
    let second = fixture
        .client
        .write_page(fixture.page("Second offered Page.", "relation-window:2"))
        .await
        .expect("write second offered Page");
    let worker = Arc::new(FakeWorker::new(vec![MaintenanceWorkerResponse::Relate {
        page_ids: [second.page_id, "pg_not_offered".to_owned()],
    }]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = false;
    config.relation.enabled = true;
    let mut maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker, config);

    let error = maintainer
        .run_once()
        .await
        .expect_err("reject relation outside offered window");

    assert!(format!("{error:#}").contains("invalid relation pair"));
    fixture.close().await;
}

#[tokio::test]
async fn maintainer_does_not_resend_an_unchanged_empty_candidate_window() {
    let fixture = Fixture::open("empty-window").await;
    fixture
        .client
        .write_page(fixture.sealed_event("One durable Page.", "empty:1", 1))
        .await
        .expect("write first inventory Page");
    fixture
        .client
        .write_page(fixture.sealed_event("Another durable Page.", "empty:2", 2))
        .await
        .expect("write second inventory Page");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::NoCandidate,
    ]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    config.packing.enabled = true;
    let mut maintainer =
        RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);

    let first = maintainer
        .run_once()
        .await
        .expect("inspect candidate window");
    let second = maintainer
        .run_once()
        .await
        .expect("skip unchanged candidate window");

    assert_eq!(first.worker_calls, 1);
    assert_eq!(second.worker_calls, 0);
    assert_eq!(worker.request_count(), 1);
    fixture.close().await;
}

#[tokio::test]
async fn operator_observe_once_runs_a_worker_without_persisting_scheduler_state() {
    let fixture = Fixture::open("operator-observe").await;
    fixture
        .client
        .write_page(fixture.sealed_event("One durable Page.", "operator-observe:1", 1))
        .await
        .expect("write first inventory Page");
    fixture
        .client
        .write_page(fixture.sealed_event("Another durable Page.", "operator-observe:2", 2))
        .await
        .expect("write second inventory Page");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::NoCandidate,
    ]));
    let mut config = fixture.config();
    config.mode = MaintenanceMode::Observe;
    config.summary.enabled = false;
    config.packing.enabled = true;
    let state_path = config.state_path.clone();
    let mut maintainer = RuntimeMaintainer::load_operator_observe_once(
        fixture.client.clone(),
        worker.clone(),
        config,
    )
    .await
    .expect("load operator observe maintenance");

    let report = maintainer
        .run_operator_observe_once()
        .await
        .expect("run operator observe maintenance");

    assert_eq!(report.worker_calls, 1);
    assert_eq!(worker.request_count(), 1);
    assert!(!state_path.exists());
    fixture.close().await;
}

#[tokio::test]
async fn operator_audit_records_only_worker_operation_and_decision() {
    let audit = MaintenanceRunAudit::queued_with_limits("apply", 7, "integration-smoke".to_owned());
    let worker = audit.worker(Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::NoCandidate,
    ])));
    worker
        .evaluate(MaintenanceWorkerRequest::SelectPacking {
            pages: Vec::new(),
            excluded_candidate_sets: Vec::new(),
        })
        .await
        .expect("evaluate audited worker");
    let record = audit.complete(Default::default());
    let serialized = serde_json::to_string(&record).expect("serialize maintenance audit");

    assert!(serialized.contains("queued"));
    assert!(serialized.contains("worker_started"));
    assert!(serialized.contains("no_candidate"));
    assert_eq!(record.mode, "apply");
    assert_eq!(record.max_jobs, 7);
}

struct Fixture {
    root: PathBuf,
    identity_id: String,
    namespace: String,
    store: Arc<dyn PcpStore>,
    client: Arc<dyn PcpApi>,
}

impl Fixture {
    async fn open(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("pcp-maintenance-{label}-{nonce}"));
        std::fs::create_dir_all(&root).expect("create maintenance fixture directory");
        let store = Arc::new(
            SqlitePcpStore::open(root.join("context.sqlite3"))
                .await
                .expect("open maintenance Store"),
        );
        let identity_id = store.identity_id().to_owned();
        let namespace = format!("project:maintenance-{label}");
        let store: Arc<dyn PcpStore> = store;
        let client = EmbeddedPcpClient::shared(
            Arc::clone(&store),
            AccessSession::full_control(
                AccessPrincipal {
                    principal_id: "service:maintenance-test".to_owned(),
                    principal_type: AccessPrincipalType::Service,
                    display_name: None,
                },
                "session:maintenance-test",
                vec![namespace.clone()],
            ),
        );
        client
            .create_scope(CreateScopeRequest {
                namespace: namespace.clone(),
                display_name: "Maintenance test".to_owned(),
                description: None,
                parent_namespace: None,
            })
            .await
            .expect("create maintenance Scope");
        Self {
            root,
            identity_id,
            namespace,
            store,
            client,
        }
    }

    fn page(&self, content: &str, idempotency_key: &str) -> WritePageRequest {
        WritePageRequest {
            namespace: self.namespace.clone(),
            lifecycle_status: LifecycleStatus::Active,
            kind: "document".to_owned(),
            mutability: PageMutability::Revisioned,
            created_by: Actor {
                actor_type: ActorType::Model,
                actor_id: "model:maintenance-test".to_owned(),
            },
            observed_at: None,
            source_span: None,
            valid_from: None,
            valid_to: None,
            payload: Some(PagePayload {
                media_type: "text/markdown".to_owned(),
                content: content.to_owned(),
            }),
            source_refs: Vec::new(),
            facets: None,
            provenance: Vec::new(),
            initial_relations: Vec::new(),
            idempotency_key: Some(idempotency_key.to_owned()),
        }
    }

    fn sealed_event(
        &self,
        content: &str,
        idempotency_key: &str,
        sequence: u64,
    ) -> WritePageRequest {
        let mut page = self.page(content, idempotency_key);
        page.kind = "conversation_event".to_owned();
        page.mutability = PageMutability::Sealed;
        page.source_span = Some(SourceSpan {
            stream_id: "conversation:maintenance-test".to_owned(),
            start: sequence,
            end: sequence,
        });
        page
    }

    fn config(&self) -> MaintenanceConfig {
        MaintenanceConfig {
            enabled: true,
            mode: MaintenanceMode::Apply,
            state_path: self.root.join("maintenance.json"),
            allowed_scopes: vec![self.namespace.clone()],
            interval_seconds: 60,
            initial_delay_seconds: 0,
            write_trigger: WriteTriggeredMaintenanceConfig::default(),
            max_jobs_per_cycle: 2,
            principal_id: "service:maintenance-test".to_owned(),
            principal_name: "Maintenance test".to_owned(),
            worker: MaintenanceWorkerConfig::Command {
                program: PathBuf::from("/bin/false"),
                args: Vec::new(),
                timeout_seconds: 1,
                actor_id: "model:maintenance-test".to_owned(),
                actor_type: "model".to_owned(),
            },
            summary: SummaryMaintenanceConfig::default(),
            packing: PackingMaintenanceConfig::default(),
            relation: RelationMaintenanceConfig::default(),
            retention: RetentionMaintenanceConfig::default(),
        }
    }

    async fn close(self) {
        drop(self.client);
        let _ = tokio::fs::remove_dir_all(self.root).await;
    }
}

#[test]
fn command_worker_is_constructible_from_validated_runtime_configuration() {
    let _ = CommandSemanticWorker::new(
        PathBuf::from("/bin/false"),
        Vec::new(),
        std::time::Duration::from_secs(1),
    );
}

#[tokio::test]
async fn command_worker_closes_stdin_before_waiting_for_the_child() {
    let worker = CommandSemanticWorker::new(
        PathBuf::from("/bin/cat"),
        Vec::new(),
        std::time::Duration::from_secs(1),
    );
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        worker.evaluate(MaintenanceWorkerRequest::SelectPacking {
            pages: Vec::new(),
            excluded_candidate_sets: Vec::new(),
        }),
    )
    .await
    .expect("command worker must observe EOF and exit");

    assert!(
        result.is_err(),
        "cat echoes a request, not a valid response"
    );
    assert!(!result.unwrap_err().to_string().contains("timed out"));
}
