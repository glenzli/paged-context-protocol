use super::*;

#[tokio::test]
async fn cross_scope_topic_requires_explicit_authority_and_preserves_sources_and_destination() {
    let root = std::env::temp_dir().join(format!(
        "pcp-cross-topic-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let store = Arc::new(
        SqlitePcpStore::open(root.join("context.sqlite3"))
            .await
            .unwrap(),
    );
    let operator = pcp_client(
        store.clone(),
        AccessMode::Admin.store_wide_session(
            principal("operator", AccessPrincipalType::Service),
            "setup",
            vec![],
            true,
        ),
    );
    for namespace in ["a", "b", "personal"] {
        operator
            .create_scope(CreateScopeRequest {
                namespace: namespace.into(),
                display_name: namespace.into(),
                description: None,
                parent_namespace: None,
            })
            .await
            .unwrap();
    }
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "model:topic-test".into(),
    };
    let first = operator
        .write_page(write_request(
            store.identity_id(),
            "a",
            actor.clone(),
            "OET definitions retain assumptions.",
            "a",
        ))
        .await
        .unwrap();
    let second = operator
        .write_page(write_request(
            store.identity_id(),
            "b",
            actor.clone(),
            "OET applications retain certificate locality.",
            "b",
        ))
        .await
        .unwrap();
    let request = ExtractTopicRequest {
        target_namespace: Some("personal".into()), target_topic: None,
        source_pages: vec![PageRevisionRef { page_id: first.page_id.clone(), revision_id: first.revision_id.clone() }, PageRevisionRef { page_id: second.page_id.clone(), revision_id: second.revision_id.clone() }],
        title: "OET proof boundaries".into(), content: "The definition's assumptions and each application's local certificate jointly delimit the supported claim.".into(),
        created_by: actor, tool_or_model: None, provenance: vec![], idempotency_key: None,
    };
    for (scopes, cross) in [
        (vec!["a", "b", "personal"], false),
        (vec!["a", "personal"], true),
        (vec!["a", "b"], true),
    ] {
        let client = pcp_client(
            store.clone(),
            AccessMode::Write.session(
                principal("restricted", AccessPrincipalType::Service),
                "restricted",
                scopes.into_iter().map(str::to_owned).collect(),
                cross,
            ),
        );
        assert!(client.extract_topic(request.clone()).await.is_err());
    }
    assert_eq!(operator.page_count(vec![]).await.unwrap(), 2);
    let mut implicit = request.clone();
    implicit.target_namespace = None;
    assert!(
        operator
            .extract_topic(implicit)
            .await
            .unwrap_err()
            .to_string()
            .contains("targetNamespace")
    );
    let topic = operator.extract_topic(request.clone()).await.unwrap();
    // A reader of only source Scope A must not lose its recall surface just
    // because an authorized maintainer created a Topic in another Scope.
    let source_reader = pcp_client(
        store.clone(),
        AccessMode::Read.session(
            principal("source-reader", AccessPrincipalType::ModelClient),
            "source-read",
            vec!["a".into()],
            false,
        ),
    );
    let scoped = source_reader
        .browse_retrieval_pages(vec![], None, BrowseIndexOrder::Recent, 10, None, 8000)
        .await
        .unwrap();
    assert_eq!(scoped.hits.len(), 1);
    assert_eq!(scoped.hits[0].page_id, first.page_id);
    assert_eq!(
        scoped.page_roles[&first.page_id],
        pcp_store::ContentPageRole::Other
    );
    let all = operator
        .browse_retrieval_pages(vec![], None, BrowseIndexOrder::Recent, 10, None, 8000)
        .await
        .unwrap();
    assert_eq!(all.hits.len(), 1);
    assert_eq!(all.hits[0].page_id, topic.page_id);
    let read = operator
        .read_pages(ReadPagesRequest {
            page_ids: vec![topic.page_id.clone()],
            revision_ids: vec![],
            projections: vec![
                Projection::Manifest,
                Projection::Provenance,
                Projection::Relations,
                Projection::Facets,
            ],
            max_chars: 8000,
        })
        .await
        .unwrap();
    assert_eq!(read[0].revision.namespace, "personal");
    assert_eq!(read[0].relations.len(), 2);
    let inputs = read[0]
        .revision
        .provenance
        .iter()
        .flat_map(|event| event.input_revision_ids.iter())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(inputs.contains(&first.revision_id) && inputs.contains(&second.revision_id));
    assert_eq!(
        operator
            .current_revision_id(first.page_id.clone())
            .await
            .unwrap(),
        first.revision_id
    );
    assert_eq!(
        operator
            .current_revision_id(second.page_id.clone())
            .await
            .unwrap(),
        second.revision_id
    );
    assert!(
        operator
            .extract_topic(request.clone())
            .await
            .unwrap_err()
            .to_string()
            .contains("refresh")
    );
    let mut refresh = request;
    refresh.target_topic = Some(PageRevisionRef {
        page_id: topic.page_id.clone(),
        revision_id: topic.revision_id.clone(),
    });
    refresh.target_namespace = None;
    let updated = operator.extract_topic(refresh.clone()).await.unwrap();
    assert_eq!(updated.page_id, topic.page_id);
    assert!(!updated.created);
    assert!(
        operator
            .extract_topic(refresh)
            .await
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
    assert_eq!(operator.page_count(vec![]).await.unwrap(), 3);
    drop(operator);
    drop(source_reader);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
