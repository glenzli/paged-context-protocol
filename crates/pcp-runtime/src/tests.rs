use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::SystemTime,
};

use pcp_client::{EmbeddedPcpClient, PcpApi, PcpTenantApi};
use pcp_core::{
    AccessPermission, AccessPrincipal, AccessPrincipalType, AccessSession, Actor, ActorType,
    CreateScopeRequest, IngestPageRequest, LifecycleStatus, PackPagesRequest, PageMutability,
    PagePayload, PageRevisionRef, PlanRevisionRetentionRequest, Projection, ReadPagesRequest,
    RetentionPolicy, SearchFilters, SearchMode, SearchPagesRequest, SearchTermMatch, SourceSpan,
    WritePageRequest,
};
use pcp_rpc::{RemotePcpClient, RuntimeEndpoint, serve_unix, serve_unix_endpoints};
use pcp_sqlite::SqlitePcpStore;
use pcp_store::PcpStore;

use crate::RuntimeConfig;

#[test]
fn runtime_config_resolves_paths_and_identity_scope_placeholders() {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("pcp-runtime-config-{nonce}"));
    std::fs::create_dir_all(&root).expect("create config test directory");
    let path = root.join("runtime.toml");
    std::fs::write(
        &path,
        r#"
store_path = "data/context.sqlite3"

[[endpoints]]
socket_path = "run/symbiont.sock"
client_id = "host:symbiont-d"
client_type = "host"
access_mode = "admin"
allowed_scopes = ["user:{identity_id}", "project:symbiont-d"]
allow_cross_scope_derivation = true

[maintenance]
enabled = true
mode = "apply"
state_path = "data/maintenance.json"
allowed_scopes = ["user:{identity_id}", "project:symbiont-d"]
interval_seconds = 300
max_interval_seconds = 3600
initial_delay_seconds = 45
max_jobs_per_cycle = 2

[maintenance.worker]
provider = "command"
program = "bin/semantic-worker"
actor_id = "model:maintenance-test"

[maintenance.summary]
minimum_chars = 6000

[maintenance.packing]
enabled = true
max_pages = 6
"#,
    )
    .expect("write runtime config");

    let config = RuntimeConfig::load(&path).expect("load runtime config");
    assert_eq!(config.store_path, root.join("data/context.sqlite3"));
    assert_eq!(
        config.endpoints[0].socket_path,
        root.join("run/symbiont.sock")
    );
    let maintenance = config.maintenance.as_ref().expect("maintenance config");
    assert_eq!(maintenance.mode, crate::MaintenanceMode::Apply);
    assert_eq!(maintenance.max_interval_seconds, 3600);
    assert_eq!(maintenance.initial_delay_seconds, 45);
    assert_eq!(maintenance.state_path, root.join("data/maintenance.json"));
    let mut invalid_schedule = maintenance.clone();
    invalid_schedule.max_interval_seconds = invalid_schedule.interval_seconds - 1;
    assert!(invalid_schedule.validate().is_err());
    let crate::MaintenanceWorkerConfig::Command { program, .. } = &maintenance.worker else {
        panic!("expected command maintenance worker");
    };
    assert_eq!(program, &root.join("bin/semantic-worker"));
    let maintenance_access = maintenance.access_session("owner-test");
    assert!(maintenance_access.allows("user:owner-test", AccessPermission::Write));
    assert!(maintenance_access.allows("user:owner-test", AccessPermission::Collect));
    assert!(!maintenance_access.allows("user:owner-test", AccessPermission::ManageScope));
    assert!(!maintenance_access.allows("user:owner-test", AccessPermission::Audit));
    let access = config.endpoints[0]
        .access_session("owner-test", 0)
        .expect("build endpoint access session");
    assert!(access.allows("user:owner-test", AccessPermission::Write));
    assert!(access.allows("project:symbiont-d", AccessPermission::DeriveAcrossScopes));

    let mut invalid = config;
    invalid.endpoints.push(invalid.endpoints[0].clone());
    assert!(invalid.validate().is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn maintenance_defaults_to_read_only_observation() {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("pcp-runtime-observe-{nonce}"));
    std::fs::create_dir_all(&root).expect("create observe config directory");
    let path = root.join("runtime.toml");
    std::fs::write(
        &path,
        r#"
store_path = "data/context.sqlite3"

[[endpoints]]
socket_path = "data/pcp.sock"
client_id = "host:test"
client_type = "host"
client_name = "test"
access_mode = "read"
allowed_scopes = ["user:{identity_id}"]

[maintenance]
enabled = true
state_path = "data/maintenance.json"
allowed_scopes = ["user:{identity_id}"]

[maintenance.worker]
provider = "command"
program = "/bin/false"
actor_id = "model:maintenance-test"
"#,
    )
    .expect("write observing maintenance config");
    let config = RuntimeConfig::load(&path).expect("parse observing maintenance config");
    let maintenance = config.maintenance.expect("maintenance config");
    assert_eq!(maintenance.mode, crate::MaintenanceMode::Observe);
    let access = maintenance.access_session("owner-test");
    assert!(access.allows("user:owner-test", AccessPermission::ReadDetail));
    assert!(access.allows("user:owner-test", AccessPermission::Search));
    assert!(!access.allows("user:owner-test", AccessPermission::Write));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn runtime_config_resolves_infer_runtime_credential_path() {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("pcp-runtime-infer-config-{nonce}"));
    std::fs::create_dir_all(&root).expect("create runtime config root");
    let path = root.join("runtime.toml");
    std::fs::write(
        &path,
        r#"
store_path = "data/context.sqlite3"

[[endpoints]]
socket_path = "run/client.sock"
client_id = "host:test"
client_type = "host"
access_mode = "read"
allowed_scopes = ["user:{identity_id}"]

[maintenance]
enabled = true
state_path = "data/maintenance.json"
allowed_scopes = ["user:{identity_id}"]

[maintenance.worker]
provider = "infer_runtime"
credential_file = "secrets/pcp-runtime.token"
timeout_seconds = 90
actor_id = "model:infer-runtime-maintenance"
actor_type = "model"
"#,
    )
    .expect("write Infer Runtime worker config");

    let config = RuntimeConfig::load(&path).expect("load Infer Runtime worker config");
    let maintenance = config.maintenance.expect("maintenance config");
    let crate::MaintenanceWorkerConfig::InferRuntime {
        credential_file,
        timeout_seconds,
        summary_deployment_id,
        reasoning_deployment_id,
        relation_deployment_id,
        escalation_deployment_id,
        escalation_operations,
        ..
    } = maintenance.worker
    else {
        panic!("expected Infer Runtime maintenance worker");
    };
    assert_eq!(credential_file, root.join("secrets/pcp-runtime.token"));
    assert_eq!(timeout_seconds, 90);
    assert_eq!(summary_deployment_id, "codex_gpt_5_6_luna");
    assert_eq!(reasoning_deployment_id, "codex_gpt_5_6_luna");
    assert_eq!(relation_deployment_id, None);
    assert_eq!(escalation_deployment_id, None);
    assert_eq!(
        escalation_operations,
        vec![
            "select_packing",
            "analyze_packing",
            "select_relation",
            "extract_topic",
            "assess_archive",
        ]
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn broker_isolates_multiple_principals_on_one_store() {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("pcp-broker-{nonce}"));
    std::fs::create_dir_all(&root).expect("create broker test directory");
    let suffix = nonce % 1_000_000_000;
    let socket_a = std::path::PathBuf::from("/tmp").join(format!("pcp-a-{suffix}.sock"));
    let socket_b = std::path::PathBuf::from("/tmp").join(format!("pcp-b-{suffix}.sock"));
    let store = Arc::new(
        SqlitePcpStore::open(root.join("context.sqlite3"))
            .await
            .expect("open broker store"),
    );
    let identity_id = store.identity_id().to_owned();
    let store: Arc<dyn PcpStore> = store;
    let client_a = EmbeddedPcpClient::shared(
        Arc::clone(&store),
        AccessSession::full_control(
            AccessPrincipal {
                principal_id: "client:broker-a".to_owned(),
                principal_type: AccessPrincipalType::Service,
                display_name: None,
            },
            "session:broker-a",
            vec!["project:broker-a".to_owned()],
        ),
    );
    let client_b = EmbeddedPcpClient::shared(
        store,
        AccessSession::full_control(
            AccessPrincipal {
                principal_id: "client:broker-b".to_owned(),
                principal_type: AccessPrincipalType::Service,
                display_name: None,
            },
            "session:broker-b",
            vec!["project:broker-b".to_owned()],
        ),
    );
    let server = tokio::spawn(serve_unix_endpoints(vec![
        RuntimeEndpoint {
            socket_path: socket_a.clone(),
            client: client_a,
            query_service: None,
        },
        RuntimeEndpoint {
            socket_path: socket_b.clone(),
            client: client_b,
            query_service: None,
        },
    ]));
    let remote_a = connect_when_ready(&socket_a).await;
    let remote_b = connect_when_ready(&socket_b).await;
    assert_eq!(remote_a.access().principal.principal_id, "client:broker-a");
    assert_eq!(remote_b.access().principal.principal_id, "client:broker-b");

    for (client, namespace, label) in [
        (&remote_a, "project:broker-a", "Broker A"),
        (&remote_b, "project:broker-b", "Broker B"),
    ] {
        client
            .create_scope(CreateScopeRequest {
                namespace: namespace.to_owned(),
                display_name: label.to_owned(),
                description: None,
                parent_namespace: None,
            })
            .await
            .expect("create broker scope");
    }
    remote_a
        .write_page(test_page(
            &identity_id,
            "project:broker-a",
            "Only broker A can read this signal.",
            "broker:a:page",
        ))
        .await
        .expect("write broker A page");
    assert_eq!(remote_a.page_count(Vec::new()).await.expect("count A"), 1);
    assert_eq!(remote_b.page_count(Vec::new()).await.expect("count B"), 0);
    assert!(
        remote_b
            .search_pages(SearchPagesRequest {
                query: "broker A signal".to_owned(),
                scopes: vec!["project:broker-a".to_owned()],
                mode: SearchMode::Text,
                term_match: SearchTermMatch::All,
                projections: vec![Projection::Payload],
                filters: SearchFilters::default(),
                limit: 10,
                cursor: None,
            })
            .await
            .is_err()
    );

    server.abort();
    let _ = server.await;
    let _ = tokio::fs::remove_file(socket_a).await;
    let _ = tokio::fs::remove_file(socket_b).await;
    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn remote_client_uses_the_runtime_bound_access_session() {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("pcp-runtime-{nonce}"));
    std::fs::create_dir_all(&root).expect("create runtime test directory");
    let socket_path = std::path::PathBuf::from("/tmp").join(format!("pcp-runtime-{nonce}.sock"));
    let store = Arc::new(
        SqlitePcpStore::open(root.join("context.sqlite3"))
            .await
            .expect("open PCP store"),
    );
    let identity_id = store.identity_id().to_owned();
    let namespace = "project:runtime-test".to_owned();
    let access = AccessSession::full_control(
        AccessPrincipal {
            principal_id: "host:runtime-test".to_owned(),
            principal_type: AccessPrincipalType::Host,
            display_name: Some("runtime test".to_owned()),
        },
        "session:runtime-test",
        vec![namespace.clone()],
    );
    let store: Arc<dyn PcpStore> = store;
    let observed_writes = Arc::new(AtomicUsize::new(0));
    let observer_counter = Arc::clone(&observed_writes);
    let embedded: Arc<dyn PcpApi> = Arc::new(
        EmbeddedPcpClient::new(store, access).with_successful_write_observer(Arc::new(move || {
            observer_counter.fetch_add(1, Ordering::SeqCst);
        })),
    );
    let server_path = socket_path.clone();
    let mut server = tokio::spawn(async move { serve_unix(server_path, embedded).await });
    let remote = tokio::select! {
        result = &mut server => panic!("PCP runtime stopped before connect: {result:?}"),
        remote = connect_when_ready(&socket_path) => remote,
    };
    assert!(
        RemotePcpClient::connect_expected(&socket_path, "host:not-this-endpoint")
            .await
            .is_err()
    );
    assert_eq!(remote.identity_id(), identity_id);
    assert_eq!(remote.access().principal.principal_id, "host:runtime-test");
    remote
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Runtime test".to_owned(),
            description: None,
            parent_namespace: None,
        })
        .await
        .expect("create authorized scope");
    assert!(
        remote
            .create_scope(CreateScopeRequest {
                namespace: "project:not-authorized".to_owned(),
                display_name: "Denied".to_owned(),
                description: None,
                parent_namespace: None,
            })
            .await
            .is_err()
    );
    let written = remote
        .write_page(WritePageRequest {
            namespace: namespace.clone(),
            lifecycle_status: LifecycleStatus::Active,
            kind: "document".to_owned(),
            mutability: PageMutability::Sealed,
            created_by: Actor {
                actor_type: ActorType::Model,
                actor_id: "model:runtime-test".to_owned(),
            },
            observed_at: None,
            source_span: Some(SourceSpan {
                stream_id: "runtime-test".to_owned(),
                start: 1,
                end: 1,
            }),
            valid_from: None,
            valid_to: None,
            payload: Some(PagePayload {
                media_type: "text/markdown".to_owned(),
                content: "Remote PCP preserves the server bound principal.".to_owned(),
            }),
            source_refs: Vec::new(),
            facets: None,
            provenance: Vec::new(),
            initial_relations: Vec::new(),
            idempotency_key: Some("runtime:test:page".to_owned()),
        })
        .await
        .expect("write through remote client");
    assert_eq!(observed_writes.load(Ordering::SeqCst), 1);
    let found = remote
        .search_pages(SearchPagesRequest {
            query: "server bound principal".to_owned(),
            scopes: vec![namespace.clone()],
            mode: SearchMode::Text,
            term_match: SearchTermMatch::All,
            projections: vec![Projection::Payload],
            filters: SearchFilters::default(),
            limit: 10,
            cursor: None,
        })
        .await
        .expect("search through remote client");
    assert_eq!(found.hits.len(), 1);
    assert_eq!(found.hits[0].revision_id, written.revision_id);
    assert_eq!(observed_writes.load(Ordering::SeqCst), 1);
    let mut second_request = test_page(
        &identity_id,
        &namespace,
        "The same durable state is available through a second Page.",
        "runtime:test:page:second",
    );
    second_request.mutability = PageMutability::Sealed;
    second_request.source_span = Some(SourceSpan {
        stream_id: "runtime-test".to_owned(),
        start: 2,
        end: 2,
    });
    let second = remote
        .write_page(second_request)
        .await
        .expect("write second remote Page");
    assert_eq!(remote.page_count(Vec::new()).await.expect("count pages"), 2);
    let packed = remote
        .pack_pages(PackPagesRequest {
            pages: vec![
                PageRevisionRef {
                    page_id: written.page_id.clone(),
                    revision_id: written.revision_id.clone(),
                },
                PageRevisionRef {
                    page_id: second.page_id.clone(),
                    revision_id: second.revision_id.clone(),
                },
            ],
            idempotency_key: Some("runtime:test:packing".to_owned()),
        })
        .await
        .expect("pack through remote client");
    assert_eq!(remote.page_count(Vec::new()).await.expect("count pages"), 1);
    assert!(remote.current_revision_id(written.page_id).await.is_err());
    assert!(remote.current_revision_id(second.page_id).await.is_err());
    assert_eq!(
        remote
            .current_revision_id(packed.page_id)
            .await
            .expect("resolve packed Page"),
        packed.revision_id
    );
    let health = remote
        .health_snapshot(Vec::new(), 24)
        .await
        .expect("read health through remote client");
    assert_eq!(health.storage.current_pages, 1);
    assert_eq!(health.packing.runs, 1);
    assert_eq!(health.packing.input_pages, 2);
    assert!(health.recall.searches >= 1);
    let retention = remote
        .plan_revision_retention(PlanRevisionRetentionRequest {
            scopes: vec![namespace],
            policy: RetentionPolicy {
                minimum_age_days: 0,
                keep_recent_revisions_per_page: 0,
                sample_limit: 10,
            },
        })
        .await
        .expect("plan retention through remote client");
    assert_eq!(retention.scanned_pages, 1);
    assert_eq!(retention.scanned_revisions, 1);
    let (audit, _) = remote.access_log(50, None).await.expect("read access log");
    assert!(audit.iter().all(|event| {
        event.principal.principal_id == "host:runtime-test"
            || event.principal.principal_id == "system:pcp"
    }));

    let ingested = remote
        .ingest_page(IngestPageRequest {
            namespace: "project:runtime-test".to_owned(),
            kind: "conversation_event".to_owned(),
            observed_at: Some("2026-08-15T00:00:00Z".to_owned()),
            source_span: Some(SourceSpan {
                stream_id: "conversation-main".to_owned(),
                start: 1,
                end: 1,
            }),
            payload: Some(PagePayload {
                media_type: "text/plain".to_owned(),
                content: "Producer fields are supplied by the authenticated Runtime.".to_owned(),
            }),
            source_refs: Vec::new(),
            based_on_revision_ids: vec![packed.revision_id.clone()],
            facets: None,
            external_event_id: Some("runtime:test:ingest".to_owned()),
        })
        .await
        .expect("ingest through simplified API");
    let ingested_page = remote
        .read_pages(ReadPagesRequest {
            page_ids: vec![ingested.page_id],
            revision_ids: Vec::new(),
            projections: vec![
                Projection::Manifest,
                Projection::Payload,
                Projection::Provenance,
            ],
            max_chars: 1_024,
        })
        .await
        .expect("read ingested Page")
        .pop()
        .expect("ingested Page exists");
    assert_eq!(ingested_page.page.mutability, PageMutability::Sealed);
    assert_eq!(
        ingested_page.revision.created_by.actor_id,
        "host:runtime-test"
    );
    let provenance = ingested_page
        .revision
        .provenance
        .first()
        .expect("Runtime-authenticated ingest provenance");
    assert_eq!(provenance.operation, "ingest");
    assert_eq!(provenance.actor.actor_id, "host:runtime-test");
    assert_eq!(provenance.timestamp, ingested_page.revision.created_at);
    assert_eq!(
        provenance.input_revision_ids,
        vec![packed.revision_id.clone()]
    );

    server.abort();
    let _ = server.await;
    let _ = tokio::fs::remove_dir_all(root).await;
}

async fn connect_when_ready(socket_path: &std::path::Path) -> RemotePcpClient {
    let mut last_error = None;
    for _ in 0..100 {
        match RemotePcpClient::connect(socket_path).await {
            Ok(client) => return client,
            Err(error) => last_error = Some(error),
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("PCP runtime did not become ready: {last_error:?}");
}

fn test_page(
    _identity_id: &str,
    namespace: &str,
    content: &str,
    idempotency_key: &str,
) -> WritePageRequest {
    WritePageRequest {
        namespace: namespace.to_owned(),
        lifecycle_status: LifecycleStatus::Active,
        kind: "document".to_owned(),
        mutability: PageMutability::Revisioned,
        created_by: Actor {
            actor_type: ActorType::Model,
            actor_id: "model:runtime-test".to_owned(),
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
