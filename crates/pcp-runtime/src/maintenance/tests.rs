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
    CreateScopeRequest, LifecycleStatus, PageMutability, PagePayload, PlanRevisionRetentionRequest,
    Projection, ReadPagesRequest, RetentionPolicy, RetentionProtectionReason, RevisePageRequest,
    SourceSpan, WritePageRequest,
};
use pcp_sqlite::SqlitePcpStore;
use pcp_store::PcpStore;

use super::{
    CommandSemanticWorker, MaintenanceConfig, MaintenanceMode, MaintenanceRunAudit,
    MaintenanceWorkerConfig, MaintenanceWorkerRequest, MaintenanceWorkerResponse,
    PackingMaintenanceConfig, RelationCandidatePage, RelationMaintenanceConfig,
    RetentionMaintenanceConfig, RetentionMilestone, RuntimeMaintainer, SemanticMaintenanceWorker,
    SummaryMaintenanceConfig, worker::PackingCandidatePage,
};

struct FakeWorker {
    responses: Mutex<VecDeque<MaintenanceWorkerResponse>>,
    requests: Mutex<Vec<MaintenanceWorkerRequest>>,
}

#[test]
fn packing_is_opt_in_and_selection_wire_contains_only_semantic_inputs() {
    assert!(!PackingMaintenanceConfig::default().enabled);
    let wire = serde_json::to_value(MaintenanceWorkerRequest::SelectPacking {
        pages: vec![PackingCandidatePage {
            page_id: "pg_1".to_owned(),
            created_at: "2026-08-15T08:00:00Z".to_owned(),
            observed_at: None,
            routing_text: "one bounded source excerpt".to_owned(),
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
            }]
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

    let access = config.access_session(&fixture.identity_id);
    let client = EmbeddedPcpClient::shared(Arc::clone(&fixture.store), access.clone());

    assert!(access.allows(&fixture.namespace, AccessPermission::Audit));
    assert!(!access.allows(&fixture.namespace, AccessPermission::Write));
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
        .expect("respect relation window cooldown");
    assert_eq!(second_cycle.worker_calls, 0);
    assert_eq!(worker.request_count(), 1);
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
    let audit = MaintenanceRunAudit::queued("integration-smoke".to_owned());
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
