use super::*;
use pcp_client::AccessMode;

fn operator(f: &Fixture) -> Arc<dyn PcpApi> {
    EmbeddedPcpClient::shared(
        f.store.clone(),
        AccessMode::Admin.store_wide_session(
            AccessPrincipal {
                principal_id: "operator".into(),
                principal_type: AccessPrincipalType::Service,
                display_name: None,
            },
            "setup",
            vec![],
            true,
        ),
    )
}

#[tokio::test]
async fn periodic_review_synthesizes_short_pages_across_existing_and_future_scopes() {
    let f = Fixture::open("periodic-all-scopes").await;
    let operator = operator(&f);
    let mut config = f.config();
    config.store_wide = true;
    config.allowed_scopes.clear();
    config.allow_cross_scope_derivation = true;
    config.periodic_review.enabled = true;
    config.summary.enabled = false;
    config.relation.enabled = false;
    config.topic.auto_apply = true;
    config.topic.target_scope = Some("user:{identity_id}".into());
    config.max_jobs_per_cycle = 1;
    let access = config.access_session(&f.identity_id);
    assert!(access.allows("scope:created-later", AccessPermission::Summarize));
    assert!(!access.allows("scope:created-later", AccessPermission::ManageScope));
    let client = EmbeddedPcpClient::shared(f.store.clone(), access);
    let personal = format!("user:{}", f.identity_id);
    // Scopes created after the maintenance client are included without a reload.
    for namespace in ["scope:created-later", &personal] {
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
    let mut sources = Vec::new();
    for i in 0..4 {
        let mut page = f.page(
            &format!(
                "OET certificate locality preserves explicit proof assumptions, boundary {i}."
            ),
            &format!("source:{i}"),
        );
        if i % 2 == 1 {
            page.namespace = "scope:created-later".into();
        }
        sources.push(operator.write_page(page).await.unwrap());
    }
    let worker = Arc::new(FakeWorker::new(vec![MaintenanceWorkerResponse::ExtractTopic {
        page_ids: sources.iter().map(|page| page.page_id.clone()).collect(),
        title: "OET certificate locality".into(),
        content: "OET proofs retain explicit assumptions. Each application uses a local certificate to delimit its supported claim. The source Pages record these proof boundaries; this Topic provides an entry point to their exact evidence.".into(),
        reason: "Four short Pages describe complementary aspects of the same proof boundary.".into(),
        refresh_topic_page_id: None,
    }, topic_convergence::verification(crate::maintenance::VerificationVerdict::Approve)]));
    let mut maintainer =
        RuntimeMaintainer::for_test(client.clone(), worker.clone(), config.clone());
    let report = maintainer.run_scheduled_cycle().await.unwrap();
    assert!(report.periodic_review);
    assert_eq!(report.inspected_pages, 5);
    assert_eq!(report.topics_written, 1);
    assert_eq!(report.worker_calls, 2);
    let inventory = client.durable_page_inventory(vec![]).await.unwrap();
    let topic = inventory
        .iter()
        .find(|page| page.kind == "topic_summary")
        .unwrap();
    assert_eq!(topic.namespace, personal);
    assert_eq!(topic.topic_source_page_ids.len(), 4);
    for page in sources {
        assert_eq!(
            client.current_revision_id(page.page_id).await.unwrap(),
            page.revision_id
        );
    }
    let next = maintainer.run_scheduled_cycle().await.unwrap();
    assert!(!next.periodic_review);
    assert_eq!(next.worker_calls, 0);
    let state = RuntimeMaintainer::automation_status(&config).await.unwrap();
    assert!(state.last_analysis_at.is_some() && state.last_content_change_at.is_some());
    assert!(state.last_periodic_review_at.is_some() && state.next_periodic_review_at.is_some());
    assert_eq!(state.recent_cycles.len(), 2);
    assert_eq!(state.recent_cycles[0].report.topics_written, 1);
    assert_eq!(worker.request_count(), 2);
    drop(maintainer);
    drop(client);
    drop(operator);
    f.close().await;
}

#[tokio::test]
async fn cross_scope_relation_is_discovered_and_reviewed_before_assertion() {
    let f = Fixture::open("cross-scope-relation").await;
    let operator = operator(&f);
    operator
        .create_scope(CreateScopeRequest {
            namespace: "other".into(),
            display_name: "Other".into(),
            description: None,
            parent_namespace: None,
        })
        .await
        .unwrap();
    let first = operator
        .write_page(f.page("OET assumptions and certificate locality.", "first"))
        .await
        .unwrap();
    let mut page = f.page("OET certificate locality in another application.", "second");
    page.namespace = "other".into();
    let second = operator.write_page(page).await.unwrap();
    let mut config = f.config();
    config.store_wide = true;
    config.allowed_scopes.clear();
    config.periodic_review.enabled = true;
    config.topic.enabled = false;
    config.summary.enabled = false;
    config.relation.enabled = true;
    config.max_jobs_per_cycle = 1;
    let worker = Arc::new(FakeWorker::new(vec![MaintenanceWorkerResponse::Relate {
        page_ids: [first.page_id.clone(), second.page_id.clone()],
        reason: "Both Pages discuss the same certificate-locality boundary.".into(),
    }]));
    let client = EmbeddedPcpClient::shared(f.store.clone(), config.access_session(&f.identity_id));
    let mut maintainer = RuntimeMaintainer::for_test(client.clone(), worker, config);
    let report = maintainer.run_scheduled_cycle().await.unwrap();
    assert_eq!(report.relations_proposed, 1);
    assert_eq!(report.relations_committed, 0);
    let review = maintainer.pending_relation_reviews().pop().unwrap();
    maintainer
        .approve_relation_review(&review.candidate_id)
        .await
        .unwrap();
    let read = client
        .read_pages(ReadPagesRequest {
            page_ids: vec![first.page_id],
            revision_ids: vec![],
            projections: vec![Projection::Relations],
            max_chars: 1000,
        })
        .await
        .unwrap();
    assert_eq!(read[0].relations.len(), 1);
    drop(maintainer);
    drop(client);
    drop(operator);
    f.close().await;
}
