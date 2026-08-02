use std::{sync::Arc, time::SystemTime};

use pcp_client::{EmbeddedPcpClient, PcpApi};
use pcp_core::{
    AccessPermission, AccessPrincipal, AccessPrincipalType, AccessSession, Actor, ActorType,
    CreateScopeRequest, LifecycleStatus, PagePayload, Projection, SearchFilters, SearchMode,
    SearchPagesRequest, SearchTermMatch, WritePageRequest,
};
use pcp_rpc::{RemotePcpClient, RuntimeEndpoint, serve_unix, serve_unix_endpoints};
use pcp_sqlite::SqlitePcpStore;
use pcp_store::PcpStore;

use crate::RuntimeConfig;

#[test]
fn runtime_config_resolves_paths_and_owner_scope_placeholders() {
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
allowed_scopes = ["user:{owner_id}", "project:symbiont-d"]
allow_cross_scope_derivation = true
"#,
    )
    .expect("write runtime config");

    let config = RuntimeConfig::load(&path).expect("load runtime config");
    assert_eq!(config.store_path, root.join("data/context.sqlite3"));
    assert_eq!(
        config.endpoints[0].socket_path,
        root.join("run/symbiont.sock")
    );
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
    let owner_id = store.owner_id().to_owned();
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
        },
        RuntimeEndpoint {
            socket_path: socket_b.clone(),
            client: client_b,
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
                owner_id: owner_id.clone(),
                namespace: namespace.to_owned(),
                scope_type: "project".to_owned(),
                display_name: label.to_owned(),
                description: None,
                parent_namespace: None,
                visibility: "private".to_owned(),
            })
            .await
            .expect("create broker scope");
    }
    remote_a
        .write_page(test_page(
            &owner_id,
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
    let socket_path = root.join("runtime.sock");
    let store = Arc::new(
        SqlitePcpStore::open(root.join("context.sqlite3"))
            .await
            .expect("open PCP store"),
    );
    let owner_id = store.owner_id().to_owned();
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
    let embedded = EmbeddedPcpClient::shared(store, access);
    let server_path = socket_path.clone();
    let server = tokio::spawn(async move { serve_unix(server_path, embedded).await });

    let remote = connect_when_ready(&socket_path).await;
    assert!(
        RemotePcpClient::connect_expected(&socket_path, "host:not-this-endpoint")
            .await
            .is_err()
    );
    assert_eq!(remote.owner_id(), owner_id);
    assert_eq!(remote.access().principal.principal_id, "host:runtime-test");
    remote
        .create_scope(CreateScopeRequest {
            owner_id: owner_id.clone(),
            namespace: namespace.clone(),
            scope_type: "project".to_owned(),
            display_name: "Runtime test".to_owned(),
            description: None,
            parent_namespace: None,
            visibility: "private".to_owned(),
        })
        .await
        .expect("create authorized scope");
    assert!(
        remote
            .create_scope(CreateScopeRequest {
                owner_id: owner_id.clone(),
                namespace: "project:not-authorized".to_owned(),
                scope_type: "project".to_owned(),
                display_name: "Denied".to_owned(),
                description: None,
                parent_namespace: None,
                visibility: "private".to_owned(),
            })
            .await
            .is_err()
    );
    let written = remote
        .write_page(WritePageRequest {
            owner_id,
            namespace: namespace.clone(),
            visibility: "private".to_owned(),
            lifecycle_status: LifecycleStatus::Active,
            created_by: Actor {
                actor_type: ActorType::Model,
                actor_id: "model:runtime-test".to_owned(),
            },
            observed_at: None,
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
    let found = remote
        .search_pages(SearchPagesRequest {
            query: "server bound principal".to_owned(),
            scopes: vec![namespace],
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
    assert_eq!(remote.page_count(Vec::new()).await.expect("count pages"), 1);
    let (audit, _) = remote.access_log(50, None).await.expect("read access log");
    assert!(audit.iter().all(|event| {
        event.principal.principal_id == "host:runtime-test"
            || event.principal.principal_id == "system:pcp"
    }));

    server.abort();
    let _ = server.await;
    let _ = tokio::fs::remove_dir_all(root).await;
}

async fn connect_when_ready(socket_path: &std::path::Path) -> RemotePcpClient {
    for _ in 0..100 {
        if let Ok(client) = RemotePcpClient::connect(socket_path).await {
            return client;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("PCP runtime did not become ready");
}

fn test_page(
    owner_id: &str,
    namespace: &str,
    content: &str,
    idempotency_key: &str,
) -> WritePageRequest {
    WritePageRequest {
        owner_id: owner_id.to_owned(),
        namespace: namespace.to_owned(),
        visibility: "private".to_owned(),
        lifecycle_status: LifecycleStatus::Active,
        created_by: Actor {
            actor_type: ActorType::Model,
            actor_id: "model:runtime-test".to_owned(),
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
