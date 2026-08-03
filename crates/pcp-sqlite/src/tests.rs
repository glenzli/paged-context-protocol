use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use pcp_client::{EmbeddedPcpClient, PcpApi};
use pcp_core::{
    AccessPermission, AccessPrincipal, AccessPrincipalType, AccessSession, Actor, ActorType,
    AssessPageValidityRequest, CreateScopeRequest, LifecycleStatus, LinkPagesRequest, PagePayload,
    Projection, ProvenanceEvent, ReadPagesRequest, RevisePageRequest, ScopeGrant, SearchFilters,
    SearchMode, SearchPagesRequest, SearchTermMatch, SourceRef, ValidityStanding, WritePageRequest,
    WriteSummaryRequest,
};
use pcp_store::PcpStore;
use serde_json::json;

use super::SqlitePcpStore;

#[tokio::test]
async fn isolates_clients_and_requires_explicit_cross_scope_derivation() {
    let root = std::env::temp_dir().join(format!(
        "pcp-sqlite-access-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let store = Arc::new(
        SqlitePcpStore::open(root.join("pcp.sqlite3"))
            .await
            .expect("open store"),
    );
    let owner_id = store.owner_id().to_owned();
    let scope_a = "project:access-a".to_owned();
    let scope_b = "project:access-b".to_owned();
    let admin = pcp_client(
        Arc::clone(&store),
        AccessSession::full_control(
            principal("host:access-admin", AccessPrincipalType::Host),
            "session:access-admin",
            vec![scope_a.clone(), scope_b.clone()],
        ),
    );
    for namespace in [&scope_a, &scope_b] {
        admin
            .create_scope(CreateScopeRequest {
                owner_id: owner_id.clone(),
                namespace: namespace.clone(),
                display_name: namespace.clone(),
                description: None,
                parent_namespace: None,
                visibility: "private".to_owned(),
            })
            .await
            .expect("create access test Scope");
    }
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "model:access-test".to_owned(),
    };
    let page_a = admin
        .write_page(write_request(
            &owner_id,
            &scope_a,
            actor.clone(),
            "Visible only to client A.",
            "access:a",
        ))
        .await
        .expect("write Scope A page");
    let page_b = admin
        .write_page(write_request(
            &owner_id,
            &scope_b,
            actor.clone(),
            "Private beta launch detail.",
            "access:b",
        ))
        .await
        .expect("write Scope B page");
    admin
        .link_pages(LinkPagesRequest {
            from_revision_id: page_a.revision_id.clone(),
            relation_type: "related_to".to_owned(),
            to_revision_id: page_b.revision_id.clone(),
            created_by: actor.clone(),
            idempotency_key: Some("access:cross-link".to_owned()),
        })
        .await
        .expect("link across authorized Scopes");

    let client_a = pcp_client(
        Arc::clone(&store),
        AccessSession::read_only(
            principal("client:access-a", AccessPrincipalType::ModelClient),
            "session:access-a",
            vec![scope_a.clone()],
        ),
    );
    let search = client_a
        .search_pages(SearchPagesRequest {
            query: "beta launch".to_owned(),
            scopes: Vec::new(),
            mode: SearchMode::Text,
            term_match: SearchTermMatch::All,
            projections: vec![Projection::Payload],
            filters: SearchFilters::default(),
            limit: 10,
            cursor: None,
        })
        .await
        .expect("search authorized Scope");
    assert!(search.hits.is_empty());
    assert!(
        client_a
            .read_pages(ReadPagesRequest {
                revision_ids: vec![page_b.revision_id.clone()],
                projections: vec![Projection::Payload],
                max_chars: 4_000,
            })
            .await
            .is_err()
    );
    assert!(
        client_a
            .current_revision_id(page_b.page_id.clone())
            .await
            .is_err()
    );
    let page_with_relations = client_a
        .read_pages(ReadPagesRequest {
            revision_ids: vec![page_a.revision_id.clone()],
            projections: vec![Projection::Relations],
            max_chars: 4_000,
        })
        .await
        .expect("read Scope A relations");
    assert!(page_with_relations[0].relations.is_empty());
    let graph = client_a
        .search_pages(SearchPagesRequest {
            query: page_a.revision_id.clone(),
            scopes: Vec::new(),
            mode: SearchMode::Graph,
            term_match: SearchTermMatch::All,
            projections: vec![Projection::Payload],
            filters: SearchFilters::default(),
            limit: 10,
            cursor: None,
        })
        .await
        .expect("traverse only authorized graph neighbors");
    assert!(graph.hits.is_empty());

    let summary_only_client = pcp_client(
        Arc::clone(&store),
        AccessSession::new(
            principal("client:summary-only", AccessPrincipalType::ModelClient),
            "session:summary-only",
            vec![ScopeGrant {
                namespace: scope_a.clone(),
                permissions: vec![AccessPermission::Search, AccessPermission::ReadSummary],
            }],
        ),
    );
    assert!(
        summary_only_client
            .search_pages(SearchPagesRequest {
                query: "visible".to_owned(),
                scopes: Vec::new(),
                mode: SearchMode::Text,
                term_match: SearchTermMatch::All,
                projections: Vec::new(),
                filters: SearchFilters::default(),
                limit: 10,
                cursor: None,
            })
            .await
            .is_err()
    );

    let restricted_writer = pcp_client(
        Arc::clone(&store),
        AccessSession::new(
            principal("client:restricted-writer", AccessPrincipalType::Service),
            "session:restricted-writer",
            vec![
                ScopeGrant {
                    namespace: scope_a.clone(),
                    permissions: vec![AccessPermission::ReadDetail],
                },
                ScopeGrant {
                    namespace: scope_b.clone(),
                    permissions: vec![AccessPermission::Write],
                },
            ],
        ),
    );
    let mut derived = write_request(
        &owner_id,
        &scope_b,
        actor.clone(),
        "Derived from Scope A.",
        "access:derived-denied",
    );
    derived.provenance = vec![ProvenanceEvent {
        operation: "derive".to_owned(),
        actor: actor.clone(),
        timestamp: "2026-08-03T00:00:00Z".to_owned(),
        input_revision_ids: vec![page_a.revision_id.clone()],
        tool_or_model: Some("access-test".to_owned()),
    }];
    assert!(restricted_writer.write_page(derived).await.is_err());

    let cross_scope_writer = pcp_client(
        Arc::clone(&store),
        AccessSession::new(
            principal("client:cross-scope-writer", AccessPrincipalType::Service),
            "session:cross-scope-writer",
            vec![
                ScopeGrant {
                    namespace: scope_a.clone(),
                    permissions: vec![AccessPermission::ReadDetail],
                },
                ScopeGrant {
                    namespace: scope_b.clone(),
                    permissions: vec![
                        AccessPermission::Write,
                        AccessPermission::DeriveAcrossScopes,
                    ],
                },
            ],
        ),
    );
    let mut derived = write_request(
        &owner_id,
        &scope_b,
        actor.clone(),
        "Explicitly derived from Scope A.",
        "access:derived-allowed",
    );
    derived.provenance = vec![ProvenanceEvent {
        operation: "derive".to_owned(),
        actor,
        timestamp: "2026-08-03T00:00:00Z".to_owned(),
        input_revision_ids: vec![page_a.revision_id],
        tool_or_model: Some("access-test".to_owned()),
    }];
    let derived = cross_scope_writer
        .write_page(derived)
        .await
        .expect("explicitly allow cross-Scope derivation");

    let client_b = pcp_client(
        Arc::clone(&store),
        AccessSession::read_only(
            principal("client:access-b", AccessPrincipalType::ModelClient),
            "session:access-b",
            vec![scope_b.clone()],
        ),
    );
    let derived_page = client_b
        .read_pages(ReadPagesRequest {
            revision_ids: vec![derived.revision_id],
            projections: vec![Projection::Payload, Projection::Provenance],
            max_chars: 4_000,
        })
        .await
        .expect("read explicitly declassified content");
    assert_eq!(
        derived_page[0]
            .revision
            .payload
            .as_ref()
            .map(|payload| payload.content.as_str()),
        Some("Explicitly derived from Scope A.")
    );
    assert!(
        derived_page[0]
            .revision
            .provenance
            .iter()
            .all(|event| event.input_revision_ids.is_empty())
    );

    let (events, _) = admin
        .access_log(100, None)
        .await
        .expect("read access audit");
    assert!(events.iter().any(|event| {
        event.principal.principal_id == "client:access-a" && event.operation == "read_pages"
    }));
    assert!(events.iter().any(|event| {
        event.principal.principal_id == "client:restricted-writer"
            && event.decision == pcp_core::AccessDecision::Denied
    }));
    let scope_b_auditor = pcp_client(
        Arc::clone(&store),
        AccessSession::new(
            principal("client:scope-b-auditor", AccessPrincipalType::Service),
            "session:scope-b-auditor",
            vec![ScopeGrant {
                namespace: scope_b.clone(),
                permissions: vec![AccessPermission::Audit],
            }],
        ),
    );
    let (events, _) = scope_b_auditor
        .access_log(100, None)
        .await
        .expect("read Scope B audit");
    let cross_scope_event = events
        .iter()
        .find(|event| event.principal.principal_id == "client:cross-scope-writer")
        .expect("cross-Scope write audit event");
    assert_eq!(cross_scope_event.scopes, vec![scope_b]);

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn stores_searches_revises_and_links_pages() {
    let root = std::env::temp_dir().join(format!(
        "pcp-sqlite-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let path = root.join("pcp.sqlite3");
    let store = SqlitePcpStore::open(path).await.expect("open store");
    let owner_id = store.owner_id().to_owned();
    let namespace = "conversation:test".to_owned();
    store
        .create_scope(CreateScopeRequest {
            owner_id: owner_id.clone(),
            namespace: namespace.clone(),
            display_name: "Test conversation".to_owned(),
            description: None,
            parent_namespace: None,
            visibility: "private".to_owned(),
        })
        .await
        .expect("create scope");

    let actor = Actor {
        actor_type: ActorType::User,
        actor_id: "user:test".to_owned(),
    };
    let first = store
        .write_page(
            write_request(
                &owner_id,
                &namespace,
                actor.clone(),
                "A compactness argument using finite products.",
                "event:first",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write first page");
    let duplicate = store
        .write_page(
            write_request(
                &owner_id,
                &namespace,
                actor.clone(),
                "A compactness argument using finite products.",
                "event:first",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("repeat first write");
    assert_eq!(duplicate.page_id, first.page_id);
    assert!(!duplicate.created);

    let search = store
        .search_pages(SearchPagesRequest {
            query: "compactness products".to_owned(),
            scopes: vec![namespace.clone()],
            mode: SearchMode::Text,
            term_match: SearchTermMatch::All,
            projections: pcp_core::default_search_projections(),
            filters: SearchFilters::default(),
            limit: 10,
            cursor: None,
        })
        .await
        .expect("search page");
    assert_eq!(search.hits.len(), 1);
    assert_eq!(search.hits[0].revision_id, first.revision_id);

    let strict = store
        .search_pages(SearchPagesRequest {
            query: "compactness impossible".to_owned(),
            scopes: vec![namespace.clone()],
            mode: SearchMode::Text,
            term_match: SearchTermMatch::All,
            projections: vec![Projection::Payload],
            filters: SearchFilters::default(),
            limit: 10,
            cursor: None,
        })
        .await
        .expect("strict lexical search");
    assert!(strict.hits.is_empty());
    let broad = store
        .search_pages(SearchPagesRequest {
            query: "compactness impossible".to_owned(),
            scopes: vec![namespace.clone()],
            mode: SearchMode::Text,
            term_match: SearchTermMatch::Any,
            projections: vec![Projection::Payload],
            filters: SearchFilters::default(),
            limit: 10,
            cursor: None,
        })
        .await
        .expect("broad lexical candidate search");
    assert_eq!(broad.hits.len(), 1);
    assert_eq!(broad.hits[0].revision_id, first.revision_id);

    let revised = store
        .revise_page(
            RevisePageRequest {
                page_id: first.page_id.clone(),
                expected_revision_id: first.revision_id.clone(),
                created_by: actor.clone(),
                lifecycle_status: LifecycleStatus::Active,
                observed_at: None,
                valid_from: None,
                valid_to: None,
                payload: Some(PagePayload {
                    media_type: "text/markdown".to_owned(),
                    content: "The finite-product compactness argument is now verified.".to_owned(),
                }),
                source_refs: vec![SourceRef {
                    source_type: "test_file".to_owned(),
                    uri: "file:///tmp/pcp-source.md".to_owned(),
                    locator: Some("L1-L3".to_owned()),
                    metadata: None,
                }],
                facets: None,
                provenance: vec![ProvenanceEvent {
                    operation: "revise".to_owned(),
                    actor: actor.clone(),
                    timestamp: "2026-07-29T00:00:00Z".to_owned(),
                    input_revision_ids: vec![first.revision_id.clone(), first.revision_id.clone()],
                    tool_or_model: Some("test".to_owned()),
                }],
                initial_relations: Vec::new(),
                idempotency_key: Some("revision:first".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("revise page");
    assert_ne!(revised.revision_id, first.revision_id);

    let derived_from_source = store
        .search_pages(SearchPagesRequest {
            query: first.revision_id.clone(),
            scopes: vec![namespace.clone()],
            mode: SearchMode::Graph,
            term_match: SearchTermMatch::All,
            projections: pcp_core::default_search_projections(),
            filters: SearchFilters {
                relation_types: vec!["derived_from".to_owned()],
                ..SearchFilters::default()
            },
            limit: 10,
            cursor: None,
        })
        .await
        .expect("traverse provenance from source");
    assert_eq!(derived_from_source.hits.len(), 1);
    assert_eq!(derived_from_source.hits[0].revision_id, revised.revision_id);

    let source_from_derived = store
        .search_pages(SearchPagesRequest {
            query: revised.revision_id.clone(),
            scopes: vec![namespace.clone()],
            mode: SearchMode::Graph,
            term_match: SearchTermMatch::All,
            projections: pcp_core::default_search_projections(),
            filters: SearchFilters {
                relation_types: vec!["derived_from".to_owned()],
                ..SearchFilters::default()
            },
            limit: 10,
            cursor: None,
        })
        .await
        .expect("traverse provenance from derived revision");
    assert_eq!(source_from_derived.hits.len(), 1);
    assert_eq!(source_from_derived.hits[0].revision_id, first.revision_id);

    let second = store
        .write_page(
            write_request(
                &owner_id,
                &namespace,
                actor.clone(),
                "A related open question.",
                "event:second",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write second page");
    store
        .link_pages(
            LinkPagesRequest {
                from_revision_id: second.revision_id.clone(),
                relation_type: "depends_on".to_owned(),
                to_revision_id: revised.revision_id.clone(),
                created_by: actor.clone(),
                idempotency_key: Some("link:first".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("link pages");

    let graph = store
        .search_pages(SearchPagesRequest {
            query: second.revision_id,
            scopes: vec![namespace.clone()],
            mode: SearchMode::Graph,
            term_match: SearchTermMatch::All,
            projections: pcp_core::default_search_projections(),
            filters: SearchFilters::default(),
            limit: 10,
            cursor: None,
        })
        .await
        .expect("search graph");
    assert_eq!(graph.hits.len(), 1);
    assert_eq!(graph.hits[0].revision_id, revised.revision_id);

    let read = store
        .read_pages(
            ReadPagesRequest {
                revision_ids: vec![revised.revision_id.clone()],
                projections: vec![
                    Projection::Payload,
                    Projection::Relations,
                    Projection::History,
                ],
                max_chars: 10_000,
            },
            vec![namespace.clone()],
        )
        .await
        .expect("read revised page");
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].history.len(), 2);
    assert_eq!(read[0].relations.len(), 2);
    assert!(
        read[0]
            .relations
            .iter()
            .any(|relation| relation.relation_type == "supersedes")
    );
    assert!(read[0].revision.source_refs.is_empty());
    assert!(read[0].revision.provenance.is_empty());
    let lean_json = serde_json::to_value(&read[0]).expect("serialize lean projection");
    assert!(lean_json["page"].get("sourceRefs").is_none());
    assert!(lean_json["page"].get("provenance").is_none());

    let traced = store
        .read_pages(
            ReadPagesRequest {
                revision_ids: vec![revised.revision_id],
                projections: vec![Projection::Sources, Projection::Provenance],
                max_chars: 10_000,
            },
            vec![namespace.clone()],
        )
        .await
        .expect("read source and provenance");
    assert_eq!(traced[0].revision.source_refs.len(), 1);
    assert_eq!(traced[0].revision.provenance.len(), 1);
    assert_eq!(traced[0].revision.provenance[0].input_revision_ids.len(), 1);

    let mut invalid = write_request(
        &owner_id,
        &namespace,
        actor.clone(),
        "This Page cites a missing revision.",
        "event:invalid-provenance",
    );
    invalid.provenance = vec![ProvenanceEvent {
        operation: "derive".to_owned(),
        actor,
        timestamp: "2026-07-29T00:00:00Z".to_owned(),
        input_revision_ids: vec!["rev_missing".to_owned()],
        tool_or_model: Some("test".to_owned()),
    }];
    let error = store
        .write_page(invalid, vec![namespace])
        .await
        .expect_err("reject missing provenance input");
    assert!(error.to_string().contains("find PCP revision rev_missing"));

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn backfills_the_provenance_graph_index() {
    let root = std::env::temp_dir().join(format!(
        "pcp-sqlite-backfill-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let path = root.join("pcp.sqlite3");
    let store = SqlitePcpStore::open(path.clone())
        .await
        .expect("open store");
    let owner_id = store.owner_id().to_owned();
    let namespace = "conversation:backfill".to_owned();
    store
        .create_scope(CreateScopeRequest {
            owner_id: owner_id.clone(),
            namespace: namespace.clone(),
            display_name: "Backfill conversation".to_owned(),
            description: None,
            parent_namespace: None,
            visibility: "private".to_owned(),
        })
        .await
        .expect("create scope");
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "model:test".to_owned(),
    };
    let source = store
        .write_page(
            write_request(
                &owner_id,
                &namespace,
                actor.clone(),
                "Source Page",
                "backfill:source",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write source");
    let mut derived_request = write_request(
        &owner_id,
        &namespace,
        actor.clone(),
        "Derived Page",
        "backfill:derived",
    );
    derived_request.provenance = vec![ProvenanceEvent {
        operation: "derive".to_owned(),
        actor,
        timestamp: "2026-07-29T00:00:00Z".to_owned(),
        input_revision_ids: vec![source.revision_id.clone()],
        tool_or_model: Some("test".to_owned()),
    }];
    let derived = store
        .write_page(derived_request, vec![namespace.clone()])
        .await
        .expect("write derived page");
    drop(store);

    let connection = rusqlite::Connection::open(&path).expect("open raw database");
    connection
        .execute_batch(
            "
            DROP TABLE pcp_provenance_inputs;
            DELETE FROM pcp_metadata WHERE key = 'provenance_input_index_version';
            ",
        )
        .expect("remove derived index");
    drop(connection);

    let reopened = SqlitePcpStore::open(path).await.expect("reopen store");
    let graph = reopened
        .search_pages(SearchPagesRequest {
            query: source.revision_id,
            scopes: vec![namespace],
            mode: SearchMode::Graph,
            term_match: SearchTermMatch::All,
            projections: pcp_core::default_search_projections(),
            filters: SearchFilters {
                relation_types: vec!["derived_from".to_owned()],
                ..SearchFilters::default()
            },
            limit: 10,
            cursor: None,
        })
        .await
        .expect("search backfilled graph");
    assert_eq!(graph.hits.len(), 1);
    assert_eq!(graph.hits[0].revision_id, derived.revision_id);

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn writes_searches_reads_and_revises_sparse_summaries() {
    let root = std::env::temp_dir().join(format!(
        "pcp-summary-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let store = SqlitePcpStore::open(root.join("pcp.sqlite3"))
        .await
        .expect("open store");
    let owner_id = store.owner_id().to_owned();
    let namespace = "conversation:summary".to_owned();
    store
        .create_scope(CreateScopeRequest {
            owner_id: owner_id.clone(),
            namespace: namespace.clone(),
            display_name: "Summary conversation".to_owned(),
            description: None,
            parent_namespace: None,
            visibility: "private".to_owned(),
        })
        .await
        .expect("create scope");
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "model:summary".to_owned(),
    };
    let page = store
        .write_page(
            write_request(
                &owner_id,
                &namespace,
                actor.clone(),
                "A long discussion whose routing value is not captured by its opening words.",
                "summary:page",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write page");
    let mut operational = write_request(
        &owner_id,
        &namespace,
        actor.clone(),
        &"rolling current map ".repeat(20),
        "summary:operational",
    );
    operational.facets = Some(json!({"kind": "runtime_context"}));
    store
        .write_page(operational, vec![namespace.clone()])
        .await
        .expect("write operational context page");
    assert_eq!(
        store
            .next_summary_candidate(
                vec![namespace.clone()],
                10,
                vec![
                    "runtime_context".to_owned(),
                    "summary_projection".to_owned(),
                ],
            )
            .await
            .expect("find summary candidate")
            .as_deref(),
        Some(page.revision_id.as_str())
    );

    let summary = store
        .write_summary(
            WriteSummaryRequest {
                target_revision_id: page.revision_id.clone(),
                expected_summary_revision_id: None,
                content:
                    "Resource-pool ownership and cancellation semantics for background workers."
                        .to_owned(),
                created_by: actor.clone(),
                tool_or_model: Some("small-model".to_owned()),
                provenance: Vec::new(),
                idempotency_key: Some("summary:first".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("write summary");
    let duplicate = store
        .write_summary(
            WriteSummaryRequest {
                target_revision_id: page.revision_id.clone(),
                expected_summary_revision_id: None,
                content: "Ignored duplicate content.".to_owned(),
                created_by: actor.clone(),
                tool_or_model: Some("small-model".to_owned()),
                provenance: Vec::new(),
                idempotency_key: Some("summary:first".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("repeat summary write");
    assert_eq!(duplicate.summary_revision_id, summary.summary_revision_id);
    assert_eq!(duplicate.summary_page_id, summary.summary_page_id);
    assert!(!duplicate.created);

    let search = store
        .search_pages(SearchPagesRequest {
            query: "resource pool cancellation".to_owned(),
            scopes: vec![namespace.clone()],
            mode: SearchMode::Text,
            term_match: SearchTermMatch::All,
            projections: vec![Projection::Summary],
            filters: SearchFilters::default(),
            limit: 10,
            cursor: None,
        })
        .await
        .expect("search summaries");
    assert_eq!(search.hits.len(), 1);
    assert_eq!(search.hits[0].revision_id, page.revision_id);
    assert_eq!(search.hits[0].matched_projection, "summary");
    assert_eq!(
        search.hits[0].summary_revision_id.as_deref(),
        Some(summary.summary_revision_id.as_str())
    );
    let browsed = store
        .search_pages(SearchPagesRequest {
            query: String::new(),
            scopes: vec![namespace.clone()],
            mode: SearchMode::Temporal,
            term_match: SearchTermMatch::All,
            projections: vec![Projection::Summary],
            filters: SearchFilters::default(),
            limit: 10,
            cursor: None,
        })
        .await
        .expect("browse summary index");
    assert_eq!(browsed.hits[0].revision_id, page.revision_id);

    let model_index = store
        .browse_index(vec![namespace.clone()], Vec::new(), 10, None, 8_000)
        .await
        .expect("browse model-written index");
    let indexed = model_index
        .hits
        .iter()
        .find(|hit| hit.revision_id == page.revision_id)
        .expect("summarized unclassified Page remains browseable");
    assert_eq!(indexed.matched_projection, "summary");

    let read = store
        .read_pages(
            ReadPagesRequest {
                revision_ids: vec![page.revision_id.clone()],
                projections: vec![Projection::Summary],
                max_chars: 10_000,
            },
            vec![namespace.clone()],
        )
        .await
        .expect("read summary projection");
    assert!(read[0].revision.payload.is_none());
    let projected = read[0].summary.as_ref().expect("summary projection");
    assert_eq!(projected.summary_revision_id, summary.summary_revision_id);
    assert_eq!(projected.summary_page_id, summary.summary_page_id);
    assert_eq!(
        projected.provenance[0].input_revision_ids,
        vec![page.revision_id.clone()]
    );

    let summary_detail = store
        .read_pages(
            ReadPagesRequest {
                revision_ids: vec![summary.summary_revision_id.clone()],
                projections: vec![
                    Projection::Payload,
                    Projection::Facets,
                    Projection::Provenance,
                    Projection::Relations,
                ],
                max_chars: 10_000,
            },
            vec![namespace.clone()],
        )
        .await
        .expect("read Summary as an independent Page Revision");
    assert_eq!(summary_detail[0].revision.page_id, summary.summary_page_id);
    assert_eq!(
        summary_detail[0].revision.payload.as_ref().unwrap().content,
        projected.content
    );
    assert_eq!(
        summary_detail[0].revision.facets.as_ref().unwrap()["kind"],
        "summary_projection"
    );
    assert!(summary_detail[0].relations.iter().any(|relation| {
        relation.relation_type == "summarizes" && relation.to_revision_id == page.revision_id
    }));

    let summary_graph = store
        .search_pages(SearchPagesRequest {
            query: summary.summary_revision_id.clone(),
            scopes: vec![namespace.clone()],
            mode: SearchMode::Graph,
            term_match: SearchTermMatch::All,
            projections: vec![Projection::Summary],
            filters: SearchFilters {
                relation_types: vec!["summarizes".to_owned()],
                ..SearchFilters::default()
            },
            limit: 10,
            cursor: None,
        })
        .await
        .expect("follow Summary DAG edge");
    assert_eq!(summary_graph.hits[0].revision_id, page.revision_id);

    let weak_match = store
        .search_pages(SearchPagesRequest {
            query: "resource unrelated".to_owned(),
            scopes: vec![namespace.clone()],
            mode: SearchMode::Text,
            term_match: SearchTermMatch::All,
            projections: vec![Projection::Summary],
            filters: SearchFilters::default(),
            limit: 10,
            cursor: None,
        })
        .await
        .expect("require all lexical terms");
    assert!(weak_match.hits.is_empty());

    let revised = store
        .write_summary(
            WriteSummaryRequest {
                target_revision_id: page.revision_id.clone(),
                expected_summary_revision_id: Some(summary.summary_revision_id.clone()),
                content:
                    "Background worker ownership, cancellation, and terminal publication semantics."
                        .to_owned(),
                created_by: actor.clone(),
                tool_or_model: Some("small-model".to_owned()),
                provenance: Vec::new(),
                idempotency_key: Some("summary:revision".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("revise summary");
    assert_ne!(revised.summary_revision_id, summary.summary_revision_id);
    assert_ne!(revised.summary_page_id, summary.summary_page_id);
    assert!(
        store
            .next_summary_candidate(
                vec![namespace.clone()],
                10,
                vec![
                    "runtime_context".to_owned(),
                    "summary_projection".to_owned(),
                ],
            )
            .await
            .expect("summary removes candidate")
            .is_none()
    );

    let conflict = store
        .write_summary(
            WriteSummaryRequest {
                target_revision_id: page.revision_id,
                expected_summary_revision_id: Some(summary.summary_revision_id),
                content: "Stale update.".to_owned(),
                created_by: actor,
                tool_or_model: Some("small-model".to_owned()),
                provenance: Vec::new(),
                idempotency_key: Some("summary:stale".to_owned()),
            },
            vec![namespace],
        )
        .await
        .expect_err("reject stale summary update");
    assert!(conflict.to_string().contains("summary conflict"));

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn rejects_cycles_in_the_derivation_subgraph() {
    let root = std::env::temp_dir().join(format!(
        "pcp-dag-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let store = SqlitePcpStore::open(root.join("pcp.sqlite3"))
        .await
        .expect("open store");
    let owner_id = store.owner_id().to_owned();
    let namespace = "project:dag".to_owned();
    store
        .create_scope(CreateScopeRequest {
            owner_id: owner_id.clone(),
            namespace: namespace.clone(),
            display_name: "DAG project".to_owned(),
            description: None,
            parent_namespace: None,
            visibility: "private".to_owned(),
        })
        .await
        .expect("create scope");
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "model:dag".to_owned(),
    };
    let first = store
        .write_page(
            write_request(
                &owner_id,
                &namespace,
                actor.clone(),
                "First Page",
                "dag:first",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write first");
    let second = store
        .write_page(
            write_request(
                &owner_id,
                &namespace,
                actor.clone(),
                "Second Page",
                "dag:second",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write second");
    store
        .link_pages(
            LinkPagesRequest {
                from_revision_id: second.revision_id.clone(),
                relation_type: "aggregates".to_owned(),
                to_revision_id: first.revision_id.clone(),
                created_by: actor.clone(),
                idempotency_key: Some("dag:forward".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("create forward edge");

    let cycle = store
        .link_pages(
            LinkPagesRequest {
                from_revision_id: first.revision_id,
                relation_type: "derived_from".to_owned(),
                to_revision_id: second.revision_id,
                created_by: actor,
                idempotency_key: Some("dag:cycle".to_owned()),
            },
            vec![namespace],
        )
        .await
        .expect_err("reject derivation cycle");
    assert!(cycle.to_string().contains("cycle"));

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn retracts_derived_pages_and_restores_preexisting_pages() {
    let root = std::env::temp_dir().join(format!(
        "pcp-retract-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let store = SqlitePcpStore::open(root.join("pcp.sqlite3"))
        .await
        .expect("open store");
    let owner_id = store.owner_id().to_owned();
    let namespace = "conversation:retract".to_owned();
    store
        .create_scope(CreateScopeRequest {
            owner_id: owner_id.clone(),
            namespace: namespace.clone(),
            display_name: "Retraction test".to_owned(),
            description: None,
            parent_namespace: None,
            visibility: "private".to_owned(),
        })
        .await
        .expect("create scope");
    let actor = Actor {
        actor_type: ActorType::User,
        actor_id: "user:retract".to_owned(),
    };
    let source = store
        .write_page(
            write_request(
                &owner_id,
                &namespace,
                actor.clone(),
                "Withdraw this message.",
                "retract:source",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write source");
    let durable = store
        .write_page(
            write_request(
                &owner_id,
                &namespace,
                actor.clone(),
                "Stable state.",
                "retract:durable",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write durable page");
    let revised = store
        .revise_page(
            RevisePageRequest {
                page_id: durable.page_id.clone(),
                expected_revision_id: durable.revision_id.clone(),
                created_by: actor.clone(),
                lifecycle_status: LifecycleStatus::Active,
                observed_at: None,
                valid_from: None,
                valid_to: None,
                payload: Some(PagePayload {
                    media_type: "text/markdown".to_owned(),
                    content: "State derived from the withdrawn message.".to_owned(),
                }),
                source_refs: Vec::new(),
                facets: None,
                provenance: vec![ProvenanceEvent {
                    operation: "derive".to_owned(),
                    actor: actor.clone(),
                    timestamp: "2026-07-30T00:00:00Z".to_owned(),
                    input_revision_ids: vec![source.revision_id.clone()],
                    tool_or_model: None,
                }],
                initial_relations: Vec::new(),
                idempotency_key: Some("retract:durable-revision".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("revise durable page");
    let mut derived_request = write_request(
        &owner_id,
        &namespace,
        actor.clone(),
        "A newly derived Page.",
        "retract:derived",
    );
    derived_request.provenance = vec![ProvenanceEvent {
        operation: "derive".to_owned(),
        actor: actor.clone(),
        timestamp: "2026-07-30T00:00:00Z".to_owned(),
        input_revision_ids: vec![source.revision_id.clone()],
        tool_or_model: None,
    }];
    let derived = store
        .write_page(derived_request, vec![namespace.clone()])
        .await
        .expect("write derived page");
    let unrelated = store
        .write_page(
            write_request(
                &owner_id,
                &namespace,
                actor.clone(),
                "Independent state.",
                "retract:unrelated",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write unrelated page");

    let result = store
        .tombstone_derivation_cascade(
            source.revision_id.clone(),
            Actor {
                actor_type: ActorType::System,
                actor_id: "system:retract".to_owned(),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("retract source");
    assert!(result.retracted_revision_ids.contains(&source.revision_id));
    assert!(result.retracted_revision_ids.contains(&revised.revision_id));
    assert!(result.retracted_revision_ids.contains(&derived.revision_id));
    assert_eq!(result.restored_page_ids, vec![durable.page_id.clone()]);
    assert_eq!(result.tombstone_revision_ids.len(), 2);

    let restored_revision = store
        .current_revision_id(durable.page_id, vec![namespace.clone()])
        .await
        .expect("read restored head");
    let restored = store
        .read_pages(
            ReadPagesRequest {
                revision_ids: vec![restored_revision],
                projections: vec![Projection::Payload],
                max_chars: 1_000,
            },
            vec![namespace.clone()],
        )
        .await
        .expect("read restored state");
    assert_eq!(
        restored[0].revision.payload.as_ref().unwrap().content,
        "Stable state."
    );
    let active = store
        .search_pages(SearchPagesRequest {
            query: String::new(),
            scopes: vec![namespace.clone()],
            mode: SearchMode::Temporal,
            term_match: SearchTermMatch::All,
            projections: pcp_core::default_search_projections(),
            filters: SearchFilters::default(),
            limit: 20,
            cursor: None,
        })
        .await
        .expect("list active pages");
    assert!(
        active
            .hits
            .iter()
            .any(|hit| hit.revision_id == unrelated.revision_id)
    );
    assert!(
        active
            .hits
            .iter()
            .all(|hit| hit.revision_id != source.revision_id
                && hit.revision_id != derived.revision_id)
    );

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn validity_is_revisioned_and_routes_before_detail() {
    let root = std::env::temp_dir().join(format!(
        "pcp-validity-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let store = SqlitePcpStore::open(root.join("pcp.sqlite3"))
        .await
        .expect("open store");
    let owner_id = store.owner_id().to_owned();
    let namespace = "conversation:validity".to_owned();
    store
        .create_scope(CreateScopeRequest {
            owner_id: owner_id.clone(),
            namespace: namespace.clone(),
            display_name: "Validity conversation".to_owned(),
            description: None,
            parent_namespace: None,
            visibility: "private".to_owned(),
        })
        .await
        .expect("create scope");
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "model:validity".to_owned(),
    };
    let target = store
        .write_page(
            write_request(
                &owner_id,
                &namespace,
                actor.clone(),
                "The runtime always restores every deferred intent.",
                "validity:target",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write target");
    let evidence = store
        .write_page(
            write_request(
                &owner_id,
                &namespace,
                actor.clone(),
                "The guarantee holds only for persisted and uncanceled intents.",
                "validity:evidence",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write evidence");

    let first = store
        .assess_page_validity(
            AssessPageValidityRequest {
                target_revision_id: target.revision_id.clone(),
                expected_assessment_id: None,
                standing: ValidityStanding::Qualified,
                rationale: "Later evidence narrows the unconditional claim.".to_owned(),
                scope: Some("Persisted, uncanceled intents only.".to_owned()),
                basis_revision_ids: vec![evidence.revision_id.clone()],
                created_by: actor.clone(),
                tool_or_model: Some("test-model".to_owned()),
                idempotency_key: Some("validity:first".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("assess validity");

    let search = store
        .search_pages(SearchPagesRequest {
            query: "runtime deferred intent".to_owned(),
            scopes: vec![namespace.clone()],
            mode: SearchMode::Text,
            term_match: SearchTermMatch::All,
            projections: pcp_core::default_search_projections(),
            filters: SearchFilters::default(),
            limit: 10,
            cursor: None,
        })
        .await
        .expect("search assessed page");
    let hit = search
        .hits
        .iter()
        .find(|hit| hit.revision_id == target.revision_id)
        .expect("target search hit");
    assert_eq!(
        hit.validity.as_ref().map(|value| &value.standing),
        Some(&ValidityStanding::Qualified)
    );
    assert_eq!(
        hit.validity
            .as_ref()
            .map(|value| value.basis_revision_count),
        Some(1)
    );
    let routed = store
        .read_pages(
            ReadPagesRequest {
                revision_ids: vec![target.revision_id.clone()],
                projections: vec![Projection::Manifest, Projection::Validity],
                max_chars: 1_000,
            },
            vec![namespace.clone()],
        )
        .await
        .expect("read validity without detail");
    assert!(routed[0].revision.payload.is_none());
    let validity = routed[0].validity.as_ref().expect("validity projection");
    assert_eq!(validity.assessment_id, first.assessment_id);
    assert_eq!(
        validity.basis_revision_ids,
        vec![evidence.revision_id.clone()]
    );

    let revised = store
        .assess_page_validity(
            AssessPageValidityRequest {
                target_revision_id: target.revision_id.clone(),
                expected_assessment_id: Some(first.assessment_id.clone()),
                standing: ValidityStanding::Superseded,
                rationale: "A newer durable state replaces the earlier guarantee.".to_owned(),
                scope: None,
                basis_revision_ids: vec![evidence.revision_id],
                created_by: actor,
                tool_or_model: Some("test-model".to_owned()),
                idempotency_key: Some("validity:second".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("revise validity");
    let latest = store
        .read_pages(
            ReadPagesRequest {
                revision_ids: vec![target.revision_id],
                projections: vec![Projection::Validity, Projection::History],
                max_chars: 1_000,
            },
            vec![namespace],
        )
        .await
        .expect("read revised validity");
    assert_eq!(
        latest[0]
            .validity
            .as_ref()
            .and_then(|value| value.previous_assessment_id.as_deref()),
        Some(first.assessment_id.as_str())
    );
    assert_eq!(
        latest[0].validity.as_ref().map(|value| &value.standing),
        Some(&ValidityStanding::Superseded)
    );
    assert_eq!(
        latest[0]
            .validity
            .as_ref()
            .map(|value| value.assessment_id.as_str()),
        Some(revised.assessment_id.as_str())
    );
    assert_eq!(latest[0].validity_history.len(), 1);
    assert_eq!(
        latest[0].validity_history[0].assessment_id,
        first.assessment_id
    );

    let _ = std::fs::remove_dir_all(root);
}

fn write_request(
    owner_id: &str,
    namespace: &str,
    actor: Actor,
    content: &str,
    idempotency_key: &str,
) -> WritePageRequest {
    WritePageRequest {
        owner_id: owner_id.to_owned(),
        namespace: namespace.to_owned(),
        visibility: "private".to_owned(),
        lifecycle_status: LifecycleStatus::Active,
        created_by: actor,
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

fn principal(principal_id: &str, principal_type: AccessPrincipalType) -> AccessPrincipal {
    AccessPrincipal {
        principal_id: principal_id.to_owned(),
        principal_type,
        display_name: None,
    }
}

fn pcp_client(store: Arc<SqlitePcpStore>, access: AccessSession) -> Arc<dyn PcpApi> {
    let store: Arc<dyn PcpStore> = store;
    EmbeddedPcpClient::shared(store, access)
}

#[tokio::test]
async fn durable_inventory_uses_current_heads_and_excludes_runtime_pages() {
    let root = std::env::temp_dir().join(format!(
        "pcp-inventory-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let store = SqlitePcpStore::open(root.join("pcp.sqlite3"))
        .await
        .expect("open store");
    let owner_id = store.owner_id().to_owned();
    let namespace = "project:inventory".to_owned();
    store
        .create_scope(CreateScopeRequest {
            owner_id: owner_id.clone(),
            namespace: namespace.clone(),
            display_name: "Inventory project".to_owned(),
            description: None,
            parent_namespace: None,
            visibility: "private".to_owned(),
        })
        .await
        .expect("create scope");
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "model:inventory".to_owned(),
    };
    let durable = store
        .write_page(
            write_request(
                &owner_id,
                &namespace,
                actor.clone(),
                "A durable protocol decision with enough detail to route later.",
                "inventory:durable",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write durable page");
    store
        .write_summary(
            WriteSummaryRequest {
                target_revision_id: durable.revision_id.clone(),
                expected_summary_revision_id: None,
                content: "A durable protocol decision used by future retrieval.".to_owned(),
                created_by: actor.clone(),
                tool_or_model: Some("test-model".to_owned()),
                provenance: Vec::new(),
                idempotency_key: Some("inventory:summary".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("write summary");
    let mut conversation = write_request(
        &owner_id,
        &namespace,
        actor,
        "A raw chat message must not become a reconciliation candidate.",
        "inventory:conversation",
    );
    conversation.facets = Some(json!({"kind": "conversation_event", "role": "user"}));
    store
        .write_page(conversation, vec![namespace.clone()])
        .await
        .expect("write conversation page");

    let inventory = store
        .durable_page_inventory(
            vec![namespace],
            vec![
                "conversation_event".to_owned(),
                "summary_projection".to_owned(),
            ],
        )
        .await
        .expect("read inventory");
    assert_eq!(inventory.len(), 1);
    assert_eq!(inventory[0].revision_id, durable.revision_id);
    assert_eq!(
        inventory[0].summary.as_deref(),
        Some("A durable protocol decision used by future retrieval.")
    );

    let _ = std::fs::remove_dir_all(root);
}
