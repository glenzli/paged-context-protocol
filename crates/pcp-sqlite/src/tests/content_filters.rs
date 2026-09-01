use super::*;
use pcp_core::WriteResult;
use pcp_store::{ContentLibraryFilter, ContentLibraryResult, ContentPageRole};

async fn browse(
    client: &Arc<dyn PcpApi>,
    role: Option<ContentPageRole>,
    with_summary: bool,
    query: Option<&str>,
    limit: u32,
    cursor: Option<String>,
) -> ContentLibraryResult {
    client
        .browse_content_pages(
            vec!["test:content-roles".into()],
            query.map(str::to_owned),
            BrowseIndexOrder::Oldest,
            limit,
            cursor,
            32_000,
            ContentLibraryFilter { role, with_summary },
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn content_roles_filter_before_pagination_and_follow_exact_revisions() {
    let root = std::env::temp_dir().join(format!(
        "pcp-content-roles-{}",
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
            "content-roles",
        ),
    );
    let namespace = "test:content-roles";
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "model:role-tests".into(),
    };
    for scope in [namespace, "test:unrelated"] {
        client
            .create_scope(CreateScopeRequest {
                namespace: scope.into(),
                display_name: scope.into(),
                parent_namespace: None,
                description: None,
            })
            .await
            .unwrap();
    }
    let mut pages: Vec<WriteResult> = Vec::new();
    for (key, kind, content) in [
        ("alpha", "document", "source alpha"),
        ("beta", "document", "source beta"),
        (
            "fake",
            "topic_summary",
            "Ordinary tenant Page, not an extraction",
        ),
        (
            "attached",
            "document",
            "Page with an attached routing summary",
        ),
    ] {
        let mut request =
            write_request(store.identity_id(), namespace, actor.clone(), content, key);
        request.kind = kind.into();
        pages.push(
            store
                .write_page(request, vec![namespace.into()])
                .await
                .unwrap(),
        );
    }
    store
        .write_page(
            write_request(
                store.identity_id(),
                "test:unrelated",
                actor.clone(),
                "source alpha",
                "outside",
            ),
            vec!["test:unrelated".into()],
        )
        .await
        .unwrap();
    let summary = store
        .write_summary(
            WriteSummaryRequest {
                target_page_id: pages[3].page_id.clone(),
                target_revision_id: pages[3].revision_id.clone(),
                expected_summary_revision_id: None,
                content: "Unique routing summary".into(),
                created_by: actor.clone(),
                tool_or_model: None,
                provenance: vec![],
                idempotency_key: None,
            },
            vec![namespace.into()],
        )
        .await
        .unwrap();
    let topic = store
        .extract_topic(
            ExtractTopicRequest {
                target_topic: None,
                source_pages: pages[..2]
                    .iter()
                    .map(|p| PageRevisionRef {
                        page_id: p.page_id.clone(),
                        revision_id: p.revision_id.clone(),
                    })
                    .collect(),
                title: "Condensed topic".into(),
                content: "Both source decisions in one summary".into(),
                created_by: actor.clone(),
                tool_or_model: None,
                provenance: vec![],
                idempotency_key: None,
            },
            vec![namespace.into()],
        )
        .await
        .unwrap();

    let all = browse(&client, None, false, None, 20, None).await;
    assert_eq!(all.total_pages, 5); // No attached-summary duplicate and no other scope.
    assert_eq!(all.page_roles[&pages[2].page_id], ContentPageRole::Other);
    assert_eq!(all.page_roles[&topic.page_id], ContentPageRole::Condensed);
    let condensed = browse(
        &client,
        Some(ContentPageRole::Condensed),
        false,
        None,
        20,
        None,
    )
    .await;
    assert_eq!(condensed.total_pages, 1);
    assert_eq!(condensed.hits[0].page_id, topic.page_id);
    let first = browse(
        &client,
        Some(ContentPageRole::CoveredSource),
        false,
        None,
        1,
        None,
    )
    .await;
    let second = browse(
        &client,
        Some(ContentPageRole::CoveredSource),
        false,
        None,
        1,
        first.next_cursor.clone(),
    )
    .await;
    assert_eq!(first.total_pages, 2);
    assert_eq!(second.total_pages, 2);
    assert_eq!(first.hits.len(), 1);
    assert_eq!(second.hits.len(), 1);
    assert_ne!(first.hits[0].page_id, second.hits[0].page_id);
    assert!(second.next_cursor.is_none());
    let text = browse(
        &client,
        Some(ContentPageRole::CoveredSource),
        false,
        Some("alpha"),
        20,
        None,
    )
    .await;
    assert_eq!(text.total_pages, 1);
    assert_eq!(text.hits[0].page_id, pages[0].page_id);
    let attached = browse(
        &client,
        Some(ContentPageRole::Other),
        true,
        Some("Unique routing"),
        20,
        None,
    )
    .await;
    assert_eq!(attached.total_pages, 1);
    assert_eq!(attached.hits[0].page_id, pages[3].page_id);
    assert_eq!(
        attached.hits[0].summary_revision_id.as_deref(),
        Some(summary.summary_revision_id.as_str())
    );
    let empty = browse(
        &client,
        Some(ContentPageRole::Condensed),
        true,
        None,
        20,
        None,
    )
    .await;
    assert_eq!(empty.total_pages, 0);
    assert!(empty.hits.is_empty());

    // Updating a source does not imply the old Topic covers the new Revision.
    for index in [0, 3] {
        store
            .revise_page(
                RevisePageRequest {
                    page_id: pages[index].page_id.clone(),
                    expected_revision_id: pages[index].revision_id.clone(),
                    created_by: actor.clone(),
                    lifecycle_status: LifecycleStatus::Active,
                    observed_at: None,
                    valid_from: None,
                    valid_to: None,
                    payload: Some(PagePayload {
                        media_type: "text/plain".into(),
                        content: "Updated content".into(),
                    }),
                    source_refs: vec![],
                    facets: None,
                    provenance: vec![],
                    initial_relations: vec![],
                    idempotency_key: None,
                },
                vec![namespace.into()],
            )
            .await
            .unwrap();
    }
    let covered = browse(
        &client,
        Some(ContentPageRole::CoveredSource),
        false,
        None,
        20,
        None,
    )
    .await;
    assert_eq!(covered.total_pages, 1);
    assert_eq!(covered.hits[0].page_id, pages[1].page_id);
    assert_eq!(
        browse(&client, None, true, None, 20, None)
            .await
            .total_pages,
        0
    );
    let current = browse(&client, None, false, None, 20, None).await;
    assert!(
        current
            .hits
            .iter()
            .find(|p| p.page_id == pages[3].page_id)
            .unwrap()
            .summary_revision_id
            .is_none()
    );

    store
        .archive_page(
            ArchivePageRequest {
                page_id: topic.page_id,
                expected_revision_id: topic.revision_id,
                reason: None,
            },
            actor,
            vec![namespace.into()],
        )
        .await
        .unwrap();
    assert_eq!(
        browse(
            &client,
            Some(ContentPageRole::Condensed),
            false,
            None,
            20,
            None
        )
        .await
        .total_pages,
        0
    );
    assert_eq!(
        browse(
            &client,
            Some(ContentPageRole::CoveredSource),
            false,
            None,
            20,
            None
        )
        .await
        .total_pages,
        0
    );
    assert_eq!(
        browse(&client, Some(ContentPageRole::Other), false, None, 20, None)
            .await
            .total_pages,
        4
    );
    drop(client);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
