use super::*;
use pcp_core::{DeletePageRequest, ReadPage};

async fn fixture() -> (std::path::PathBuf, Arc<SqlitePcpStore>, Arc<dyn PcpApi>) {
    let root = std::env::temp_dir().join(format!(
        "pcp-direct-actions-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let store = Arc::new(
        SqlitePcpStore::open(root.join("store.sqlite3"))
            .await
            .unwrap(),
    );
    let client = pcp_client(
        Arc::clone(&store),
        AccessSession::store_wide_full_control(
            principal("operator:local", AccessPrincipalType::Service),
            "direct-actions",
        ),
    );
    client
        .create_scope(CreateScopeRequest {
            namespace: "test:pages".into(),
            display_name: "Pages".into(),
            description: None,
            parent_namespace: None,
        })
        .await
        .unwrap();
    (root, store, client)
}

async fn read(
    client: &Arc<dyn PcpApi>,
    page_ids: Vec<String>,
    revision_ids: Vec<String>,
) -> Vec<ReadPage> {
    client
        .read_pages(ReadPagesRequest {
            page_ids,
            revision_ids,
            projections: vec![
                Projection::Manifest,
                Projection::Payload,
                Projection::Summary,
                Projection::Sources,
                Projection::Facets,
                Projection::Relations,
                Projection::Provenance,
            ],
            max_chars: 64_000,
        })
        .await
        .unwrap()
}

fn document(content: &str, key: &str) -> WritePageRequest {
    write_request(
        "",
        "test:pages",
        Actor {
            actor_id: "operator:local".into(),
            actor_type: ActorType::Tool,
        },
        content,
        key,
    )
}

#[tokio::test]
async fn direct_delete_checks_authority_and_revision_without_cascading() {
    let (root, store, client) = fixture().await;
    let mut request = document("delete-only-marker", "root");
    request.mutability = PageMutability::Sealed;
    let original = client.write_page(request).await.unwrap();
    let mut child = document("independent downstream interpretation", "child");
    child.provenance = vec![ProvenanceEvent {
        operation: "derive".into(),
        actor: child.created_by.clone(),
        timestamp: Utc::now().to_rfc3339(),
        input_revision_ids: vec![original.revision_id.clone()],
        tool_or_model: None,
        reason: None,
    }];
    let child = client.write_page(child).await.unwrap();
    client
        .link_pages(LinkPagesRequest {
            idempotency_key: None,
            from_page_id: child.page_id.clone(),
            relation_type: "related_to".into(),
            to_page_id: original.page_id.clone(),
            basis_revision_ids: vec![child.revision_id.clone(), original.revision_id.clone()],
            created_by: Actor {
                actor_id: "operator:local".into(),
                actor_type: ActorType::Tool,
            },
        })
        .await
        .unwrap();
    let request = DeletePageRequest {
        page_id: original.page_id.clone(),
        expected_revision_id: original.revision_id.clone(),
        reason: Some("User requested replacement".into()),
        idempotency_key: Some("delete-once".into()),
    };
    for access in [
        AccessMode::Read.session(
            principal("read", AccessPrincipalType::Host),
            "read",
            vec!["test:pages".into()],
            false,
        ),
        AccessSession::full_control(
            principal("other", AccessPrincipalType::Service),
            "other",
            vec!["test:other".into()],
        ),
        AccessSession::store_wide_full_control(
            principal("model", AccessPrincipalType::ModelClient),
            "model",
        ),
    ] {
        assert!(
            pcp_client(Arc::clone(&store), access)
                .delete_page(request.clone())
                .await
                .is_err()
        );
    }
    let mut stale = request.clone();
    stale.expected_revision_id = "rev_stale".into();
    assert!(client.delete_page(stale).await.is_err());
    assert_eq!(client.page_count(vec![]).await.unwrap(), 2);
    let deleted = client.delete_page(request.clone()).await.unwrap();
    let retry = client.delete_page(request).await.unwrap();
    assert_eq!(deleted.revision_id, retry.revision_id);
    assert!(!retry.created);
    let pages = read(
        &client,
        vec![original.page_id.clone(), child.page_id.clone()],
        vec![],
    )
    .await;
    let retired = pages
        .iter()
        .find(|p| p.page.page_id == original.page_id)
        .unwrap();
    assert_eq!(retired.page.lifecycle_status, LifecycleStatus::Tombstoned);
    assert!(retired.revision.payload.is_none());
    assert!(retired.relations.is_empty());
    let survivor = pages
        .iter()
        .find(|p| p.page.page_id == child.page_id)
        .unwrap();
    assert_eq!(survivor.revision.revision_id, child.revision_id);
    assert_eq!(survivor.page.lifecycle_status, LifecycleStatus::Active);
    assert!(survivor.relations.is_empty());
    let history = read(&client, vec![], vec![original.revision_id.clone()]).await;
    assert_eq!(
        history[0].revision.payload.as_ref().unwrap().content,
        "delete-only-marker"
    );
    assert_eq!(client.page_count(vec![]).await.unwrap(), 1);
    let result = client
        .search_pages(SearchPagesRequest {
            term_match: SearchTermMatch::All,
            query: "delete-only-marker".into(),
            scopes: vec![],
            mode: SearchMode::Exact,
            projections: vec![Projection::Payload],
            filters: SearchFilters::default(),
            limit: 10,
            cursor: None,
        })
        .await
        .unwrap();
    assert!(result.hits.is_empty());
    let replacement = client
        .write_page(document("delete-only-marker", "replacement"))
        .await
        .unwrap();
    assert_ne!(replacement.page_id, original.page_id);
    assert_eq!(client.integrity_check().await.unwrap(), "ok");
    drop(client);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn direct_summary_edit_keeps_projection_binding_and_delete_detaches_it() {
    let (root, store, client) = fixture().await;
    let target = client
        .write_page(document("Original source remains untouched", "source"))
        .await
        .unwrap();
    let summary = client
        .write_summary(WriteSummaryRequest {
            target_page_id: target.page_id.clone(),
            target_revision_id: target.revision_id.clone(),
            expected_summary_revision_id: None,
            content: "Incorrect summary".into(),
            created_by: Actor {
                actor_id: "operator:local".into(),
                actor_type: ActorType::Tool,
            },
            tool_or_model: None,
            provenance: vec![],
            idempotency_key: None,
        })
        .await
        .unwrap();
    let page = read(&client, vec![summary.summary_page_id.clone()], vec![])
        .await
        .remove(0);
    let repair = RepairPageRequest {
        page_id: summary.summary_page_id.clone(),
        expected_revision_id: summary.summary_revision_id.clone(),
        payload: Some(PagePayload {
            media_type: "text/markdown".into(),
            content: "Corrected summary marker".into(),
        }),
        source_refs: page.revision.source_refs,
        facets: page.revision.facets,
        based_on_revision_ids: vec![],
        reason: "Edited in PCP Console".into(),
        tool_or_model: None,
        idempotency_key: None,
    };
    let mut oversized = repair.clone();
    oversized.payload.as_mut().unwrap().content = "x".repeat(1201);
    assert!(client.repair_page(oversized).await.is_err());
    let fixed = client.repair_page(repair).await.unwrap();
    let source = read(&client, vec![target.page_id.clone()], vec![])
        .await
        .remove(0);
    assert_eq!(source.revision.revision_id, target.revision_id);
    assert_eq!(
        source.summary.as_ref().unwrap().summary_revision_id,
        fixed.revision_id
    );
    assert_eq!(source.summary.unwrap().content, "Corrected summary marker");
    let old = read(&client, vec![], vec![summary.summary_revision_id])
        .await
        .remove(0);
    assert_eq!(old.revision.payload.unwrap().content, "Incorrect summary");
    client
        .delete_page(DeletePageRequest {
            page_id: fixed.page_id,
            expected_revision_id: fixed.revision_id,
            reason: None,
            idempotency_key: None,
        })
        .await
        .unwrap();
    let source = read(&client, vec![target.page_id], vec![]).await.remove(0);
    assert!(source.summary.is_none());
    assert_eq!(source.revision.revision_id, target.revision_id);
    assert_eq!(client.integrity_check().await.unwrap(), "ok");
    drop(client);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
