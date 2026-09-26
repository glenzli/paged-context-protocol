use super::*;
use pcp_core::{ConsolidatePagesRequest, ConsolidatedPageOutput, ConsolidationCoverage};
use pcp_store::{ContentLibraryFilter, ContentPageRole};

fn source(page: &pcp_core::WriteResult) -> PageRevisionRef {
    PageRevisionRef {
        page_id: page.page_id.clone(),
        revision_id: page.revision_id.clone(),
    }
}

async fn recall(client: &Arc<dyn PcpApi>, namespace: &str) -> Vec<String> {
    client
        .search_pages(SearchPagesRequest {
            query: String::new(),
            scopes: vec![namespace.into()],
            mode: SearchMode::Temporal,
            term_match: SearchTermMatch::All,
            projections: pcp_core::default_search_projections(),
            filters: SearchFilters::default(),
            limit: 20,
            cursor: None,
        })
        .await
        .unwrap()
        .hits
        .into_iter()
        .map(|hit| hit.page_id)
        .collect()
}

#[tokio::test]
async fn reviewed_split_is_atomic_and_only_complete_sources_leave_recall() {
    let root = std::env::temp_dir().join(format!(
        "pcp-consolidation-{}",
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
            "consolidation-test",
        ),
    );
    let namespace = "test:consolidation";
    client
        .create_scope(CreateScopeRequest {
            namespace: namespace.into(),
            display_name: "Consolidation".into(),
            description: None,
            parent_namespace: None,
        })
        .await
        .unwrap();
    let actor = Actor {
        actor_type: ActorType::Tool,
        actor_id: "operator:local".into(),
    };
    let first = client
        .write_page(write_request(
            store.identity_id(),
            namespace,
            actor.clone(),
            "Earlier plan: keep A. Separate note: issue B is unresolved.",
            "source-a",
        ))
        .await
        .unwrap();
    let second = client
        .write_page(write_request(
            store.identity_id(),
            namespace,
            actor.clone(),
            "Later plan changes A to A2; B remains unresolved.",
            "source-b",
        ))
        .await
        .unwrap();
    let request = ConsolidatePagesRequest {
        namespace: namespace.into(),
        source_pages: vec![source(&first), source(&second)],
        outputs: vec![
            ConsolidatedPageOutput {
                title: "Plan A".into(),
                content: "Earlier plan A was changed later to A2.".into(),
                source_indexes: vec![0, 1],
            },
            ConsolidatedPageOutput {
                title: "Issue B".into(),
                content: "Issue B is still unresolved across both source Pages.".into(),
                source_indexes: vec![0, 1],
            },
        ],
        coverage: vec![
            ConsolidationCoverage {
                source_index: 0,
                output_indexes: vec![0, 1],
                explanation:
                    "A and B, including the earlier timing and open question, are retained.".into(),
                complete: true,
            },
            ConsolidationCoverage {
                source_index: 1,
                output_indexes: vec![0, 1],
                explanation: "Keep this source in recall until its extra context is reviewed."
                    .into(),
                complete: false,
            },
        ],
        created_by: actor,
        idempotency_key: "split-a-b".into(),
    };
    let result = client.consolidate_pages(request.clone()).await.unwrap();
    assert!(result.created);
    assert_eq!(result.outputs.len(), 2);
    assert_eq!(result.source_only, vec![source(&first)]);
    let replay = client.consolidate_pages(request.clone()).await.unwrap();
    assert!(!replay.created);
    assert_eq!(replay.outputs, result.outputs);
    let mut repeated = request.clone();
    repeated.idempotency_key = "split-a-b-new-key".into();
    assert!(
        client
            .consolidate_pages(repeated)
            .await
            .unwrap_err()
            .to_string()
            .contains("already covers these source Revisions")
    );
    let mut changed = request.clone();
    changed.outputs[0].content.push_str(" changed");
    assert!(
        client
            .consolidate_pages(changed)
            .await
            .unwrap_err()
            .to_string()
            .contains("reused")
    );

    let ids = recall(&client, namespace).await;
    assert!(!ids.contains(&first.page_id));
    assert!(ids.contains(&second.page_id));
    let inventory = client.durable_page_inventory(Vec::new()).await.unwrap();
    assert!(
        inventory
            .iter()
            .find(|page| page.page_id == first.page_id)
            .unwrap()
            .source_only
    );
    assert!(
        !inventory
            .iter()
            .find(|page| page.page_id == second.page_id)
            .unwrap()
            .source_only
    );
    assert!(
        result
            .outputs
            .iter()
            .all(|output| ids.contains(&output.page_id))
    );
    let routed = client
        .browse_retrieval_pages(
            vec![namespace.into()],
            None,
            BrowseIndexOrder::Recent,
            20,
            None,
            32000,
        )
        .await
        .unwrap();
    let summary = client
        .content_library_summary(vec![namespace.into()])
        .await
        .unwrap();
    assert_eq!(summary.page_count, 4);
    assert_eq!(summary.source_only_page_count, Some(1));
    assert_eq!(summary.secondary_search_page_count, Some(1));
    assert_eq!(summary.condensed_page_count, Some(2));
    assert_eq!(
        routed.total_pages,
        summary.page_count - summary.source_only_page_count.unwrap()
    );
    assert!(!routed.hits.iter().any(|hit| hit.page_id == first.page_id));
    assert!(routed.hits.iter().any(|hit| hit.page_id == second.page_id));
    let related = client
        .search_pages(SearchPagesRequest {
            query: result.outputs[0].revision_id.clone(),
            scopes: vec![namespace.into()],
            mode: SearchMode::Graph,
            term_match: SearchTermMatch::All,
            projections: pcp_core::default_search_projections(),
            filters: SearchFilters::default(),
            limit: 20,
            cursor: None,
        })
        .await
        .unwrap();
    assert!(related.hits.iter().any(|hit| {
        hit.page_id == first.page_id
            && hit
                .graph_edges
                .iter()
                .any(|edge| edge.edge_kind == GraphEdgeKind::Provenance)
    }));
    let exact = client
        .read_pages(ReadPagesRequest {
            page_ids: vec![],
            revision_ids: vec![first.revision_id.clone()],
            projections: vec![Projection::Payload],
            max_chars: 64000,
        })
        .await
        .unwrap();
    assert!(
        exact[0]
            .revision
            .payload
            .as_ref()
            .unwrap()
            .content
            .contains("Earlier plan")
    );
    let library = client
        .browse_content_pages(
            vec![namespace.into()],
            None,
            BrowseIndexOrder::Recent,
            20,
            None,
            32000,
            ContentLibraryFilter {
                role: Some(ContentPageRole::CoveredSource),
                with_summary: false,
            },
        )
        .await
        .unwrap();
    assert!(library.hits.iter().any(|hit| hit.page_id == first.page_id));

    client
        .archive_page(ArchivePageRequest {
            page_id: result.outputs[1].page_id.clone(),
            expected_revision_id: result.outputs[1].revision_id.clone(),
            reason: Some("review output again".into()),
        })
        .await
        .unwrap();
    let summary = client
        .content_library_summary(vec![namespace.into()])
        .await
        .unwrap();
    assert_eq!(summary.page_count, 3);
    assert_eq!(summary.source_only_page_count, Some(0));
    assert_eq!(summary.secondary_search_page_count, Some(0));
    assert_eq!(summary.condensed_page_count, Some(1));
    assert!(recall(&client, namespace).await.contains(&first.page_id));
    assert!(
        !client
            .durable_page_inventory(Vec::new())
            .await
            .unwrap()
            .iter()
            .find(|page| page.page_id == first.page_id)
            .unwrap()
            .source_only
    );
    let before = client.page_count(vec![namespace.into()]).await.unwrap();
    let stale = client
        .consolidate_pages(ConsolidatePagesRequest {
            idempotency_key: "stale-source".into(),
            source_pages: vec![source(&first), result.outputs[1].clone()],
            ..request
        })
        .await;
    assert!(stale.is_err());
    assert_eq!(
        client.page_count(vec![namespace.into()]).await.unwrap(),
        before
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn clean_four_store_adds_consolidation_tables_without_changing_pages() {
    let root = std::env::temp_dir().join(format!(
        "pcp-consolidation-migration-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let path = root.join("store.sqlite3");
    let store = SqlitePcpStore::open(path.clone()).await.unwrap();
    let identity = store.identity_id().to_owned();
    drop(store);
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "DROP INDEX pcp_consolidation_sources_current;
         DROP TABLE pcp_consolidation_coverage;
         DROP TABLE pcp_consolidation_sources;
         DROP TABLE pcp_consolidations;
         UPDATE pcp_metadata SET value = '0.8.0-clean.4' WHERE key = 'schema_version';",
        )
        .unwrap();
    drop(connection);
    let reopened = SqlitePcpStore::open(path.clone()).await.unwrap();
    assert_eq!(reopened.identity_id(), identity);
    drop(reopened);
    let connection = Connection::open(&path).unwrap();
    let version: String = connection
        .query_row(
            "SELECT value FROM pcp_metadata WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, "0.8.0-clean.5");
    let count: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name LIKE 'pcp_consolidation%'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 3);
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}
