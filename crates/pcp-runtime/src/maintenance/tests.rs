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
    AccessPrincipal, AccessPrincipalType, AccessSession, Actor, ActorType, CreateScopeRequest,
    LifecycleStatus, PagePayload, Projection, ReadPagesRequest, WritePageRequest,
};
use pcp_sqlite::SqlitePcpStore;
use pcp_store::PcpStore;

use super::{
    CommandSemanticWorker, CompactionMaintenanceConfig, MaintenanceConfig, MaintenanceMode,
    MaintenanceWorkerRequest, MaintenanceWorkerResponse, RuntimeMaintainer,
    SemanticMaintenanceWorker, SummaryMaintenanceConfig, WorkerCommandConfig,
};

struct FakeWorker {
    responses: Mutex<VecDeque<MaintenanceWorkerResponse>>,
    requests: Mutex<Vec<MaintenanceWorkerRequest>>,
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
    config.compaction.enabled = false;
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
    config.compaction.enabled = false;
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
async fn maintainer_requires_selection_and_synthesis_before_atomic_consolidation() {
    let fixture = Fixture::open("consolidation").await;
    let first = fixture
        .client
        .write_page(fixture.page(
            "The runtime keeps one durable and auditable state.",
            "consolidation:1",
        ))
        .await
        .expect("write first consolidation Page");
    let second = fixture
        .client
        .write_page(fixture.page(
            "The same runtime state remains durable across restarts.",
            "consolidation:2",
        ))
        .await
        .expect("write second consolidation Page");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::Candidate {
            page_ids: vec![first.revision_id.clone(), second.revision_id.clone()],
            rationale: Some("Both Pages describe one durable runtime state.".to_owned()),
        },
        MaintenanceWorkerResponse::Consolidate {
            canonical_page_id: first.revision_id.clone(),
            content: "The runtime preserves one durable, auditable state across restarts."
                .to_owned(),
        },
    ]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    let mut maintainer =
        RuntimeMaintainer::for_test(fixture.client.clone(), worker.clone(), config);

    let report = maintainer
        .run_once()
        .await
        .expect("run consolidation maintenance");

    assert_eq!(report.worker_calls, 2);
    assert_eq!(report.consolidations_committed, 1);
    assert_eq!(worker.request_count(), 2);
    let first_head = fixture
        .client
        .current_revision_id(first.page_id)
        .await
        .expect("read canonical Ref");
    let second_head = fixture
        .client
        .current_revision_id(second.page_id)
        .await
        .expect("read redirected Ref");
    assert_eq!(first_head, second_head);
    assert_ne!(first_head, first.revision_id);
    fixture.close().await;
}

#[tokio::test]
async fn observe_mode_records_a_consolidation_proposal_without_replacing_heads() {
    let fixture = Fixture::open("observe-consolidation").await;
    let first = fixture
        .client
        .write_page(fixture.page("One durable runtime state.", "observe-merge:1"))
        .await
        .expect("write first observed Page");
    let second = fixture
        .client
        .write_page(fixture.page(
            "The same durable runtime state after restart.",
            "observe-merge:2",
        ))
        .await
        .expect("write second observed Page");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::Candidate {
            page_ids: vec![first.revision_id.clone(), second.revision_id.clone()],
            rationale: Some("Both Pages describe one state.".to_owned()),
        },
        MaintenanceWorkerResponse::Consolidate {
            canonical_page_id: first.revision_id.clone(),
            content: "A proposed replacement that must not be committed.".to_owned(),
        },
    ]));
    let mut config = fixture.config();
    config.mode = MaintenanceMode::Observe;
    config.summary.enabled = false;
    let mut maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker, config);

    let report = maintainer
        .run_once()
        .await
        .expect("observe consolidation maintenance");

    assert_eq!(report.consolidations_proposed, 1);
    assert_eq!(report.consolidations_committed, 0);
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
async fn maintainer_keeps_pages_current_when_the_worker_rejects_a_merge() {
    let fixture = Fixture::open("keep-separate").await;
    let first = fixture
        .client
        .write_page(fixture.page("A design constraint.", "separate:1"))
        .await
        .expect("write first separate Page");
    let second = fixture
        .client
        .write_page(fixture.page("An unrelated observation.", "separate:2"))
        .await
        .expect("write second separate Page");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::Candidate {
            page_ids: vec![first.revision_id.clone(), second.revision_id.clone()],
            rationale: None,
        },
        MaintenanceWorkerResponse::KeepSeparate {
            reason: Some("The Pages are only superficially similar.".to_owned()),
        },
    ]));
    let mut config = fixture.config();
    config.summary.enabled = false;
    let mut maintainer = RuntimeMaintainer::for_test(fixture.client.clone(), worker, config);

    let report = maintainer
        .run_once()
        .await
        .expect("run rejected consolidation maintenance");

    assert_eq!(report.kept_separate, 1);
    assert_eq!(report.consolidations_committed, 0);
    assert_eq!(
        fixture
            .client
            .current_revision_id(first.page_id)
            .await
            .expect("read first current Page"),
        first.revision_id
    );
    assert_eq!(
        fixture
            .client
            .current_revision_id(second.page_id)
            .await
            .expect("read second current Page"),
        second.revision_id
    );
    fixture.close().await;
}

#[tokio::test]
async fn maintainer_does_not_resend_an_unchanged_empty_candidate_window() {
    let fixture = Fixture::open("empty-window").await;
    fixture
        .client
        .write_page(fixture.page("One durable Page.", "empty:1"))
        .await
        .expect("write first inventory Page");
    fixture
        .client
        .write_page(fixture.page("Another durable Page.", "empty:2"))
        .await
        .expect("write second inventory Page");
    let worker = Arc::new(FakeWorker::new(vec![
        MaintenanceWorkerResponse::NoCandidate {
            reason: Some("Nothing should be merged.".to_owned()),
        },
    ]));
    let mut config = fixture.config();
    config.summary.enabled = false;
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

struct Fixture {
    root: PathBuf,
    owner_id: String,
    namespace: String,
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
        let owner_id = store.owner_id().to_owned();
        let namespace = format!("project:maintenance-{label}");
        let store: Arc<dyn PcpStore> = store;
        let client = EmbeddedPcpClient::shared(
            store,
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
                owner_id: owner_id.clone(),
                namespace: namespace.clone(),
                display_name: "Maintenance test".to_owned(),
                description: None,
                parent_namespace: None,
                visibility: "private".to_owned(),
            })
            .await
            .expect("create maintenance Scope");
        Self {
            root,
            owner_id,
            namespace,
            client,
        }
    }

    fn page(&self, content: &str, idempotency_key: &str) -> WritePageRequest {
        WritePageRequest {
            owner_id: self.owner_id.clone(),
            namespace: self.namespace.clone(),
            visibility: "private".to_owned(),
            lifecycle_status: LifecycleStatus::Active,
            created_by: Actor {
                actor_type: ActorType::Model,
                actor_id: "model:maintenance-test".to_owned(),
            },
            observed_at: None,
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
            worker: WorkerCommandConfig {
                program: PathBuf::from("/bin/false"),
                args: Vec::new(),
                timeout_seconds: 1,
                actor_id: "model:maintenance-test".to_owned(),
                actor_type: "model".to_owned(),
            },
            summary: SummaryMaintenanceConfig::default(),
            compaction: CompactionMaintenanceConfig::default(),
        }
    }

    async fn close(self) {
        drop(self.client);
        let _ = tokio::fs::remove_dir_all(self.root).await;
    }
}

#[test]
fn command_worker_is_constructible_from_validated_runtime_configuration() {
    let worker = WorkerCommandConfig {
        program: PathBuf::from("/bin/false"),
        args: Vec::new(),
        timeout_seconds: 1,
        actor_id: "model:test".to_owned(),
        actor_type: "model".to_owned(),
    };
    let _ = CommandSemanticWorker::new(&worker);
}

#[tokio::test]
async fn command_worker_closes_stdin_before_waiting_for_the_child() {
    let worker = CommandSemanticWorker::new(&WorkerCommandConfig {
        program: PathBuf::from("/bin/cat"),
        args: Vec::new(),
        timeout_seconds: 1,
        actor_id: "model:test".to_owned(),
        actor_type: "model".to_owned(),
    });
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        worker.evaluate(MaintenanceWorkerRequest::SelectConsolidation {
            pages: Vec::new(),
            max_pages: 2,
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
