use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime},
};

use chrono::{DateTime, Utc};
use pcp_client::{AccessMode, EmbeddedPcpClient, PcpApi};
use pcp_core::{
    AccessPrincipal, AccessPrincipalType, AccessSession, Actor, ActorType, CreateScopeRequest,
    LifecycleStatus, PageMutability, PagePayload, PlanRevisionRetentionRequest, Projection,
    ReadPagesRequest, RetentionPolicy, SearchFilters, SearchMode, SearchPagesRequest,
    SearchTermMatch, WritePageRequest,
};
use pcp_sqlite::SqlitePcpStore;
use pcp_store::PcpStore;
use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

use super::{
    ObserverConfig, ObserverService,
    contract::{
        DISCOVERY_REGISTRATION_SCHEMA, DISCOVERY_SCHEMA_VERSION, DiscoveryRegistration,
        ERROR_SCHEMA, IssueSeverity, LOCAL_UNIX_SOCKET_BINDING, ObserverError,
        PCP_OBSERVER_PROTOCOL_ID, PCP_OBSERVER_PROTOCOL_VERSION, REQUEST_SCHEMA, SNAPSHOT_SCHEMA,
        SnapshotEnvelope,
    },
    registration::{current_uid, prepare_runtime_layout},
    service::ensure_same_user,
};

#[test]
fn issue_severity_serializes_to_the_pcp_protocol_enum() {
    assert_eq!(
        serde_json::to_value(IssueSeverity::Info).expect("serialize info severity"),
        serde_json::json!("info")
    );
    assert_eq!(
        serde_json::to_value(IssueSeverity::Warning).expect("serialize warning severity"),
        serde_json::json!("warning")
    );
    assert_eq!(
        serde_json::to_value(IssueSeverity::Critical).expect("serialize critical severity"),
        serde_json::json!("critical")
    );
}

#[test]
fn observer_peer_must_have_the_provider_effective_uid() {
    let uid = current_uid();
    assert!(ensure_same_user(uid, uid).is_ok());
    assert!(ensure_same_user(uid.wrapping_add(1), uid).is_err());
}

#[test]
fn discovery_runtime_root_must_not_be_a_symlink() {
    let target = test_root("real-root");
    let link = target.with_file_name(format!(
        "{}-link",
        target
            .file_name()
            .expect("test root name")
            .to_string_lossy()
    ));
    symlink(&target, &link).expect("create runtime root symlink");
    let config = ObserverConfig::for_test(link.clone(), "pcp-symlink-test");
    let error = prepare_runtime_layout(&config).expect_err("symlink root must be rejected");
    assert!(error.to_string().contains("not a real directory"));
    fs::remove_file(link).expect("remove runtime root symlink");
    fs::remove_dir_all(target).expect("remove runtime root target");
}

#[tokio::test]
async fn observe_access_reads_only_aggregate_health() {
    let root = test_root("permissions");
    let (store, owner_id, namespace, page_id, _) = fixture(&root).await;
    let access = AccessMode::Observe.session(
        AccessPrincipal {
            principal_id: "service:pcp-runtime-observer".to_owned(),
            principal_type: AccessPrincipalType::Service,
            display_name: None,
        },
        "observer:test",
        vec![namespace.clone()],
        false,
    );
    let store: Arc<dyn PcpStore> = store;
    let client = EmbeddedPcpClient::new(store, access);

    assert!(
        client
            .health_snapshot(vec![namespace.clone()], 24)
            .await
            .is_ok()
    );
    assert!(client.page_count(vec![namespace.clone()]).await.is_err());
    assert!(
        client
            .list_scopes(vec![namespace.clone()], None, 10, None)
            .await
            .is_err()
    );
    assert!(
        client
            .search_pages(SearchPagesRequest {
                query: "observer secret".to_owned(),
                scopes: vec![namespace.clone()],
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
    assert!(
        client
            .read_pages(ReadPagesRequest {
                page_ids: vec![page_id],
                revision_ids: Vec::new(),
                projections: vec![Projection::Payload],
                max_chars: 4_096,
            })
            .await
            .is_err()
    );
    assert!(client.access_log(10, None).await.is_err());
    assert!(
        client
            .plan_revision_retention(PlanRevisionRetentionRequest {
                scopes: vec![namespace],
                policy: RetentionPolicy::default(),
            })
            .await
            .is_err()
    );
    assert_eq!(client.owner_id(), owner_id);
    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn observer_publishes_serves_renews_and_expires_naturally() {
    let root = test_root("wire");
    let (store, _, namespace, _, secret) = fixture(&root).await;
    let config = ObserverConfig::for_test(root.clone(), "pcp-test");
    let manifest_path = config.manifest_path();
    let store: Arc<dyn PcpStore> = store;
    let mut observer = ObserverService::start(config.clone(), Arc::clone(&store))
        .await
        .expect("start observer")
        .expect("observer enabled");
    let generation = observer.generation().to_owned();
    let socket_path = observer.socket_path().to_owned();

    let manifest = read_manifest(&manifest_path);
    let manifest_json = read_json(&manifest_path);
    assert_eq!(
        object_keys(&manifest_json),
        BTreeSet::from(["lease", "offers", "schema", "schema_version", "service"])
    );
    assert_eq!(
        object_keys(&manifest_json["service"]),
        BTreeSet::from(["generation", "instance_id", "kind"])
    );
    assert_eq!(
        object_keys(&manifest_json["lease"]),
        BTreeSet::from(["expires_at", "renewed_at"])
    );
    assert_eq!(
        object_keys(&manifest_json["offers"][0]),
        BTreeSet::from(["binding", "endpoint", "protocol", "protocol_versions"])
    );
    assert_eq!(manifest.schema, DISCOVERY_REGISTRATION_SCHEMA);
    assert_eq!(manifest.schema_version, DISCOVERY_SCHEMA_VERSION);
    assert_eq!(manifest.service.kind, "pcp");
    assert_eq!(manifest.service.instance_id, "pcp-test");
    assert_eq!(manifest.service.generation, generation);
    assert_eq!(manifest.offers.len(), 1);
    assert_eq!(manifest.offers[0].protocol, PCP_OBSERVER_PROTOCOL_ID);
    assert_eq!(
        manifest.offers[0].protocol_versions,
        [PCP_OBSERVER_PROTOCOL_VERSION]
    );
    assert_eq!(manifest.offers[0].binding, LOCAL_UNIX_SOCKET_BINDING);
    assert_eq!(
        config.socket_path(&manifest.offers[0].endpoint),
        socket_path
    );
    assert!(Path::new(&manifest.offers[0].endpoint).is_relative());
    assert!(manifest.offers[0].endpoint.starts_with("sockets/"));
    assert!(!manifest_json.to_string().contains("console_url"));
    assert!(!manifest_json.to_string().contains("infra-observer"));
    assert_private_layout(&config, &manifest_path, &socket_path);
    assert_no_registration_temporary_files(&config);

    let renewed_at = parse_timestamp(&manifest.lease.renewed_at);
    let expires_at = parse_timestamp(&manifest.lease.expires_at);
    assert!(renewed_at < expires_at);
    assert!(expires_at - renewed_at <= chrono::Duration::seconds(120));

    let snapshot = request_snapshot(&socket_path).await;
    let encoded = serde_json::to_string(&snapshot).expect("encode snapshot for inspection");
    assert_eq!(snapshot.schema, SNAPSHOT_SCHEMA);
    assert_eq!(snapshot.schema_version, PCP_OBSERVER_PROTOCOL_VERSION);
    assert_eq!(snapshot.service.kind, manifest.service.kind);
    assert_eq!(snapshot.service.instance_id, manifest.service.instance_id);
    assert_eq!(snapshot.service.generation, manifest.service.generation);
    assert_eq!(
        snapshot.links.console_url.as_deref(),
        Some("http://127.0.0.1:4318/")
    );
    assert!(snapshot.headline_metrics.len() <= 3);
    assert!(
        snapshot
            .metrics
            .iter()
            .all(|metric| !metric.value.is_null())
    );
    for headline in &snapshot.headline_metrics {
        assert!(
            snapshot.metrics.iter().any(|metric| &metric.id == headline),
            "headline references missing metric {headline}"
        );
    }
    for metric_id in [
        "process.uptime_seconds",
        "requests.total",
        "requests.failed",
        "requests.denied",
        "pcp.pages.current",
    ] {
        assert!(
            snapshot.metrics.iter().any(|metric| metric.id == metric_id),
            "missing metric {metric_id}"
        );
    }
    assert_eq!(snapshot.extensions.pcp.scope_count, 1);
    assert!(snapshot.extensions.pcp.health.is_some());
    assert!(!encoded.contains(&secret));
    assert!(!encoded.contains(&namespace));
    assert_eq!(
        serde_json::to_value(&snapshot.redaction).expect("serialize redaction"),
        serde_json::json!({
            "excluded": [
                "page_content",
                "query_text",
                "scope_names",
                "raw_audit",
                "storage_paths"
            ]
        })
    );

    let second = match ObserverService::start(config.clone(), store.clone()).await {
        Ok(_) => panic!("one stable identity must have one publisher"),
        Err(error) => error,
    };
    assert!(second.to_string().contains("publication authority"));

    let error = request_invalid(&socket_path).await;
    assert_eq!(error.schema, ERROR_SCHEMA);
    assert_eq!(error.schema_version, PCP_OBSERVER_PROTOCOL_VERSION);
    assert_eq!(error.code, "invalid_request");
    let oversized = request_line(&socket_path, vec![b'x'; 4_097]).await;
    let oversized: ObserverError = decode_line(&oversized);
    assert_eq!(oversized.code, "invalid_request");

    tokio::time::sleep(Duration::from_millis(90)).await;
    let renewed = read_manifest(&manifest_path);
    assert!(
        parse_timestamp(&renewed.lease.renewed_at) > parse_timestamp(&manifest.lease.renewed_at)
    );
    assert_eq!(renewed.service.generation, manifest.service.generation);

    observer.shutdown().await.expect("stop observer");
    assert!(manifest_path.exists(), "manifest must expire naturally");
    assert!(!socket_path.exists(), "generation socket must be removed");
    let expired_manifest = fs::read(&manifest_path).expect("read retained manifest");
    sleep_until_expired(&renewed.lease.expires_at).await;
    assert_eq!(
        fs::read(&manifest_path).expect("read naturally expired manifest"),
        expired_manifest,
        "shutdown must not rewrite or remove the stable manifest"
    );

    let mut replacement = ObserverService::start(config.clone(), store)
        .await
        .expect("start replacement observer")
        .expect("replacement observer enabled");
    assert_ne!(replacement.generation(), generation);
    assert_eq!(
        read_manifest(&manifest_path).service.generation,
        replacement.generation()
    );
    replacement.shutdown().await.expect("stop replacement");
    assert!(manifest_path.exists());
    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn observer_omits_metrics_without_known_values() {
    let root = test_root("unknown-metrics");
    let store: Arc<dyn PcpStore> = Arc::new(
        SqlitePcpStore::open(root.join("context.sqlite3"))
            .await
            .expect("open empty observer test store"),
    );
    let config = ObserverConfig::for_test(root.clone(), "pcp-empty-test");
    let mut observer = ObserverService::start(config.clone(), store)
        .await
        .expect("start empty observer")
        .expect("observer enabled");
    let socket_path = observer.socket_path().to_owned();

    let snapshot = request_snapshot(&socket_path).await;
    assert!(
        snapshot
            .metrics
            .iter()
            .all(|metric| !metric.value.is_null())
    );
    for omitted in [
        "requests.latency.p95_ms",
        "requests.telemetry_coverage_ratio",
    ] {
        assert!(!snapshot.metrics.iter().any(|metric| metric.id == omitted));
        assert!(!snapshot.headline_metrics.iter().any(|id| id == omitted));
    }
    for headline in &snapshot.headline_metrics {
        assert!(snapshot.metrics.iter().any(|metric| &metric.id == headline));
    }

    observer.shutdown().await.expect("stop empty observer");
    let _ = tokio::fs::remove_dir_all(root).await;
}

async fn request_snapshot(socket_path: &Path) -> SnapshotEnvelope {
    let response = request_line(
        socket_path,
        format!(
            "{{\"schema\":\"{REQUEST_SCHEMA}\",\"schema_version\":\"{PCP_OBSERVER_PROTOCOL_VERSION}\",\"operation\":\"snapshot\"}}\n"
        )
        .into_bytes(),
    )
    .await;
    decode_line(&response)
}

async fn request_invalid(socket_path: &Path) -> ObserverError {
    let response = request_line(
        socket_path,
        format!(
            "{{\"schema\":\"{REQUEST_SCHEMA}\",\"schema_version\":\"{PCP_OBSERVER_PROTOCOL_VERSION}\",\"operation\":\"snapshot\",\"extra\":true}}\n"
        )
        .into_bytes(),
    )
    .await;
    decode_line(&response)
}

async fn request_line(socket_path: &Path, request: Vec<u8>) -> Vec<u8> {
    let mut stream = UnixStream::connect(socket_path)
        .await
        .expect("connect observer socket");
    stream
        .write_all(&request)
        .await
        .expect("write observer request");
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut response))
        .await
        .expect("observer must close after one response")
        .expect("read observer response");
    assert!(response.ends_with(b"\n"), "response must end with LF");
    assert!(response.len() <= 1024 * 1024, "response includes LF limit");
    assert_eq!(
        response.iter().filter(|byte| **byte == b'\n').count(),
        1,
        "one connection must produce one JSON line"
    );
    response
}

fn decode_line<T: serde::de::DeserializeOwned>(response: &[u8]) -> T {
    serde_json::from_slice(&response[..response.len() - 1]).expect("decode observer response")
}

fn read_manifest(path: &Path) -> DiscoveryRegistration {
    serde_json::from_slice(&fs::read(path).expect("read registration manifest"))
        .expect("decode registration manifest")
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).expect("read JSON file")).expect("decode JSON file")
}

fn object_keys(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .expect("registration is an object")
        .keys()
        .map(String::as_str)
        .collect()
}

fn assert_private_layout(config: &ObserverConfig, manifest_path: &Path, socket_path: &Path) {
    for path in [
        config.runtime_root.as_path(),
        config.registration_dir().as_path(),
        config.socket_dir().as_path(),
    ] {
        let metadata = fs::symlink_metadata(path).expect("private directory metadata");
        assert!(metadata.is_dir());
        assert!(!metadata.file_type().is_symlink());
        assert_eq!(metadata.uid(), current_uid());
        assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
    }
    let manifest = fs::symlink_metadata(manifest_path).expect("manifest metadata");
    assert!(manifest.file_type().is_file());
    assert!(!manifest.file_type().is_symlink());
    assert_eq!(manifest.uid(), current_uid());
    assert_eq!(manifest.permissions().mode() & 0o777, 0o600);
    assert!(manifest.len() <= 64 * 1024);
    let socket = fs::symlink_metadata(socket_path).expect("socket metadata");
    assert_eq!(socket.uid(), current_uid());
    assert_eq!(socket.permissions().mode() & 0o777, 0o600);
}

fn assert_no_registration_temporary_files(config: &ObserverConfig) {
    let entries = fs::read_dir(config.registration_dir()).expect("read registration directory");
    for entry in entries {
        let name = entry
            .expect("read registration entry")
            .file_name()
            .to_string_lossy()
            .into_owned();
        assert!(
            !name.ends_with(".tmp"),
            "temporary manifest remains: {name}"
        );
    }
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("parse RFC 3339 timestamp")
        .with_timezone(&Utc)
}

async fn sleep_until_expired(expires_at: &str) {
    let remaining = parse_timestamp(expires_at) - Utc::now();
    if let Ok(duration) = remaining.to_std() {
        tokio::time::sleep(duration + Duration::from_millis(20)).await;
    }
    assert!(parse_timestamp(expires_at) <= Utc::now());
}

async fn fixture(root: &Path) -> (Arc<SqlitePcpStore>, String, String, String, String) {
    let store = Arc::new(
        SqlitePcpStore::open(root.join("context.sqlite3"))
            .await
            .expect("open observer test store"),
    );
    let owner_id = store.owner_id().to_owned();
    let namespace = "project:pcp-observer-test".to_owned();
    let access = AccessSession::full_control(
        AccessPrincipal {
            principal_id: "host:observer-test".to_owned(),
            principal_type: AccessPrincipalType::Host,
            display_name: None,
        },
        "host:observer-test",
        vec![namespace.clone()],
    );
    PcpStore::create_scope(
        store.as_ref(),
        &access,
        CreateScopeRequest {
            owner_id: owner_id.clone(),
            namespace: namespace.clone(),
            display_name: "Observer test".to_owned(),
            description: None,
            parent_namespace: None,
            visibility: "private".to_owned(),
        },
    )
    .await
    .expect("create observer test Scope");
    let secret = "CONTENT_MUST_NEVER_ENTER_OBSERVER".to_owned();
    let written = PcpStore::write_page(
        store.as_ref(),
        &access,
        WritePageRequest {
            owner_id: owner_id.clone(),
            namespace: namespace.clone(),
            visibility: "private".to_owned(),
            lifecycle_status: LifecycleStatus::Active,
            kind: "document".to_owned(),
            mutability: PageMutability::Revisioned,
            created_by: Actor {
                actor_type: ActorType::Tool,
                actor_id: "host:observer-test".to_owned(),
            },
            observed_at: None,
            valid_from: None,
            valid_to: None,
            payload: Some(PagePayload {
                media_type: "text/markdown".to_owned(),
                content: secret.clone(),
            }),
            source_refs: Vec::new(),
            facets: None,
            provenance: Vec::new(),
            initial_relations: Vec::new(),
            idempotency_key: Some("observer:test:page".to_owned()),
        },
    )
    .await
    .expect("write observer test Page");
    (store, owner_id, namespace, written.page_id, secret)
}

fn test_root(label: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::path::PathBuf::from("/tmp").join(format!("pcpo-{label}-{nonce}"));
    fs::create_dir_all(&root).expect("create observer test root");
    root
}
