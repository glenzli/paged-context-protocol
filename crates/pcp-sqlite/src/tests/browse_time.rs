use super::*;

#[tokio::test]
async fn browse_time_separates_store_updates_from_source_observations() {
    let root = std::env::temp_dir().join(format!(
        "pcp-browse-time-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let path = root.join("store.sqlite3");
    let store = Arc::new(SqlitePcpStore::open(path.clone()).await.unwrap());
    let client = pcp_client(
        Arc::clone(&store),
        AccessSession::store_wide_full_control(
            principal("operator:local", AccessPrincipalType::Service),
            "browse-time",
        ),
    );
    let namespace = "test:browse-time";
    client
        .create_scope(CreateScopeRequest {
            namespace: namespace.into(),
            display_name: namespace.into(),
            description: None,
            parent_namespace: None,
        })
        .await
        .unwrap();
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "model:test".into(),
    };
    let mut pages = Vec::new();
    for (key, observed_at) in [
        ("older-write", Some("2026-09-02T12:00:00Z")),
        ("new-date-only-write", Some("2026-09-02")),
        ("unknown-observation", None),
    ] {
        let mut request = write_request(store.identity_id(), namespace, actor.clone(), key, key);
        request.observed_at = observed_at.map(str::to_owned);
        pages.push(
            store
                .write_page(request, vec![namespace.into()])
                .await
                .unwrap(),
        );
    }
    // Deterministic Store-clock fixture; never change a source observation to sort it.
    let connection = Connection::open(&path).unwrap();
    for (index, page) in pages.iter().enumerate() {
        connection
            .execute(
                "UPDATE pcp_pages SET updated_at = ? WHERE page_id = ?",
                params![format!("2026-09-02T14:48:0{}.000Z", index), page.page_id,],
            )
            .unwrap();
    }
    let browse = |order, limit, cursor| {
        client.browse_content_pages(
            vec![namespace.into()],
            None,
            order,
            limit,
            cursor,
            20_000,
            Default::default(),
        )
    };
    let first = browse(BrowseIndexOrder::Updated, 1, None).await.unwrap();
    assert_eq!(first.total_pages, 3);
    assert_eq!(first.hits[0].page_id, pages[2].page_id);
    let second = browse(BrowseIndexOrder::Updated, 1, first.next_cursor)
        .await
        .unwrap();
    assert_eq!(second.hits[0].page_id, pages[1].page_id);
    assert_eq!(second.hits[0].observed_at.as_deref(), Some("2026-09-02"));
    let third = browse(BrowseIndexOrder::Updated, 1, second.next_cursor)
        .await
        .unwrap();
    assert_eq!(third.hits[0].page_id, pages[0].page_id);
    assert!(third.next_cursor.is_none());

    let oldest = browse(BrowseIndexOrder::LeastRecentlyUpdated, 10, None)
        .await
        .unwrap();
    assert_eq!(
        oldest
            .hits
            .iter()
            .map(|hit| &hit.page_id)
            .collect::<Vec<_>>(),
        pages.iter().map(|page| &page.page_id).collect::<Vec<_>>()
    );
    let observed = browse(BrowseIndexOrder::Recent, 10, None).await.unwrap();
    let position = |page_id: &str| {
        observed
            .hits
            .iter()
            .position(|hit| hit.page_id == page_id)
            .unwrap()
    };
    assert!(position(&pages[0].page_id) < position(&pages[1].page_id));

    // Updating an old Page, not just inserting a new one, moves it to the front.
    connection
        .execute(
            "UPDATE pcp_pages SET updated_at = ? WHERE page_id = ?",
            params!["2026-09-03T10:00:00.000Z", pages[0].page_id],
        )
        .unwrap();
    let refreshed = browse(BrowseIndexOrder::Updated, 10, None).await.unwrap();
    assert_eq!(refreshed.hits[0].page_id, pages[0].page_id);
    assert_eq!(refreshed.hits[0].revision_id, pages[0].revision_id);
    assert_eq!(
        refreshed.hits[0].observed_at.as_deref(),
        Some("2026-09-02T12:00:00Z")
    );
    let index = client
        .browse_index(
            vec![namespace.into()],
            Vec::new(),
            BrowseIndexOrder::Updated,
            10,
            None,
            20_000,
        )
        .await
        .unwrap();
    assert_eq!(index.hits[0].page_id, pages[0].page_id);
    let retrieval = client
        .browse_retrieval_pages(
            vec![namespace.into()],
            None,
            BrowseIndexOrder::Updated,
            10,
            None,
            20_000,
        )
        .await
        .unwrap();
    assert_eq!(retrieval.hits[0].page_id, pages[0].page_id);
    drop(connection);
    drop(client);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
