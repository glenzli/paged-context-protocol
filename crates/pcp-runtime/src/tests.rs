use std::{sync::Arc, time::SystemTime};

use pcp_client::{EmbeddedPcpClient, PcpApi};
use pcp_core::{
    AccessPrincipal, AccessPrincipalType, AccessSession, Actor, ActorType, CreateScopeRequest,
    LifecycleStatus, PagePayload, Projection, SearchFilters, SearchMode, SearchPagesRequest,
    SearchTermMatch, WritePageRequest,
};
use pcp_sqlite::SqlitePcpStore;
use pcp_store::PcpStore;

use crate::{RemotePcpClient, serve_unix};

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
