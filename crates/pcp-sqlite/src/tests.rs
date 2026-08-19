use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use chrono::{Duration, SecondsFormat, Utc};
use pcp_client::{EmbeddedPcpClient, PcpApi};
use pcp_core::{
    AccessDecision, AccessPermission, AccessPrincipal, AccessPrincipalType, AccessSession, Actor,
    ActorType, AssessPageValidityRequest, BrowseIndexOrder, CollectRevisionRetentionRequest,
    CreateScopeRequest, ExtractTopicRequest, GraphEdgeDirection, GraphEdgeKind, LifecycleStatus,
    LinkPagesRequest, PackPagesRequest, PageMutability, PagePayload, PageRevisionRef,
    PlanRevisionRetentionRequest, Projection, ProvenanceEvent, PutRevisionRetentionLeaseRequest,
    QueryAuditEvent, QueryAuditMethod, ReadPagesRequest, RetentionPolicy,
    RetentionProtectionReason, RevisePageRequest, RouterTokenUsage, ScopeGrant, SearchFilters,
    SearchMode, SearchPagesRequest, SearchTermMatch, SourceRef, SourceSpan, UnpackPageRequest,
    ValidityStanding, WritePageRequest, WriteSummaryRequest,
};
use pcp_store::PcpStore;
use rusqlite::{Connection, params};
use serde_json::json;

use super::SqlitePcpStore;

#[tokio::test]
async fn stores_minimal_external_source_references() {
    let root = std::env::temp_dir().join(format!(
        "pcp-sqlite-media-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let store = SqlitePcpStore::open(root.join("pcp.sqlite3"))
        .await
        .expect("open media store");
    let identity_id = store.identity_id().to_owned();
    let namespace = "project:media".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Media".to_owned(),
            description: None,
            parent_namespace: None,
        })
        .await
        .expect("create media scope");

    let source = SourceRef {
        provider_id: "tenant:photos".to_owned(),
        locator: "opaque-photo-42".to_owned(),
        media_type: Some("image/jpeg".to_owned()),
        content_digest: Some("sha256:abc123".to_owned()),
    };
    let mut request = write_request(
        &identity_id,
        &namespace,
        Actor {
            actor_type: ActorType::Tool,
            actor_id: "tenant:photos".to_owned(),
        },
        "A searchable interpretation of the externally held image.",
        "media:valid",
    );
    request.kind = "media_representation".to_owned();
    request.source_refs = vec![source.clone()];
    let written = store
        .write_page(request, vec![namespace.clone()])
        .await
        .expect("write media representation");
    let read = store
        .read_pages(
            ReadPagesRequest {
                page_ids: vec![written.page_id],
                revision_ids: Vec::new(),
                projections: vec![Projection::Manifest, Projection::Sources],
                max_chars: 1_024,
            },
            vec![namespace.clone()],
        )
        .await
        .expect("read media representation")
        .pop()
        .expect("media Page exists");
    assert_eq!(read.revision.source_refs[0].provider_id, "tenant:photos");
    assert_eq!(read.revision.source_refs[0].locator, "opaque-photo-42");

    let mut malformed = source;
    malformed.locator.clear();
    let mut rejected = write_request(
        &identity_id,
        &namespace,
        Actor {
            actor_type: ActorType::Tool,
            actor_id: "tenant:photos".to_owned(),
        },
        "This media reference has no provider locator.",
        "media:invalid",
    );
    rejected.source_refs = vec![malformed];
    assert!(store.write_page(rejected, vec![namespace]).await.is_err());

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn topic_extraction_preserves_sources_but_routes_retrieval_through_topic_page() {
    let root = std::env::temp_dir().join(format!(
        "pcp-sqlite-topic-extraction-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let store = SqlitePcpStore::open(root.join("pcp.sqlite3"))
        .await
        .expect("open topic Store");
    let identity_id = store.identity_id().to_owned();
    let namespace = "conversation:topic-extraction".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Topic extraction".to_owned(),
            description: None,
            parent_namespace: None,
        })
        .await
        .expect("create topic scope");
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "model:topic-extraction".to_owned(),
    };
    let first = store
        .write_page(
            write_request(
                &identity_id,
                &namespace,
                actor.clone(),
                "The first portion of a long topic establishes its definition.",
                "topic:first",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write first topic source");
    let second = store
        .write_page(
            write_request(
                &identity_id,
                &namespace,
                actor.clone(),
                "The second portion records the decision and unresolved boundary.",
                "topic:second",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write second topic source");
    let unrelated = store
        .write_page(
            write_request(
                &identity_id,
                &namespace,
                actor.clone(),
                "An independent topic remains directly discoverable.",
                "topic:unrelated",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write unrelated Page");

    let topic = store
        .extract_topic(
            ExtractTopicRequest {
                source_pages: vec![
                    PageRevisionRef {
                        page_id: first.page_id.clone(),
                        revision_id: first.revision_id.clone(),
                    },
                    PageRevisionRef {
                        page_id: second.page_id.clone(),
                        revision_id: second.revision_id.clone(),
                    },
                ],
                title: "A long-lived topic".to_owned(),
                content: "This front-door topic summary joins the definition, decision, and unresolved boundary.".to_owned(),
                created_by: actor,
                tool_or_model: Some("topic-extraction-test".to_owned()),
                provenance: Vec::new(),
                idempotency_key: Some("topic:extract".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("extract topic");

    let library = store
        .browse_content_pages(
            vec![namespace.clone()],
            None,
            BrowseIndexOrder::Recent,
            10,
            None,
            10_000,
        )
        .await
        .expect("browse complete content library");
    assert!(library.hits.iter().any(|hit| hit.page_id == first.page_id));
    assert!(library.hits.iter().any(|hit| hit.page_id == second.page_id));
    assert!(
        library
            .hits
            .iter()
            .any(|hit| hit.page_id == unrelated.page_id)
    );
    assert!(library.hits.iter().any(|hit| hit.page_id == topic.page_id));

    let retrieval = store
        .browse_retrieval_pages(
            vec![namespace.clone()],
            None,
            BrowseIndexOrder::Recent,
            10,
            None,
            10_000,
        )
        .await
        .expect("browse default retrieval surface");
    assert!(
        retrieval
            .hits
            .iter()
            .any(|hit| hit.page_id == topic.page_id)
    );
    assert!(
        retrieval
            .hits
            .iter()
            .any(|hit| hit.page_id == unrelated.page_id)
    );
    assert!(
        !retrieval
            .hits
            .iter()
            .any(|hit| hit.page_id == first.page_id)
    );
    assert!(
        !retrieval
            .hits
            .iter()
            .any(|hit| hit.page_id == second.page_id)
    );

    let sources = store
        .read_pages(
            ReadPagesRequest {
                page_ids: Vec::new(),
                revision_ids: vec![first.revision_id, second.revision_id],
                projections: vec![Projection::Payload],
                max_chars: 1_000,
            },
            vec![namespace],
        )
        .await
        .expect("read retained exact source revisions");
    assert_eq!(sources.len(), 2);

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn rejects_pre_v08_stores_instead_of_migrating_them() {
    let root = std::env::temp_dir().join(format!(
        "pcp-sqlite-old-schema-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create old Store fixture");
    let path = root.join("pcp.sqlite3");
    Connection::open(&path)
        .expect("open old Store fixture")
        .execute("CREATE TABLE pcp_pages (page_id TEXT PRIMARY KEY)", [])
        .expect("seed old Store fixture");

    let error = SqlitePcpStore::open(path)
        .await
        .err()
        .expect("reject old Store");
    assert!(error.to_string().contains("requires a new Store"));

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn migrates_clean_association_store_to_topic_extraction_schema() {
    let root = std::env::temp_dir().join(format!(
        "pcp-sqlite-topic-schema-migration-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let path = root.join("pcp.sqlite3");
    let store = SqlitePcpStore::open(path.clone())
        .await
        .expect("open current topic Store");
    drop(store);

    let connection = Connection::open(&path).expect("open topic migration fixture");
    connection
        .execute_batch(
            "
            DROP INDEX pcp_topic_extraction_members_source;
            DROP TABLE pcp_topic_extraction_members;
            DROP TABLE pcp_topic_extractions;
            UPDATE pcp_metadata
            SET value = '0.8.0-clean.1'
            WHERE key = 'schema_version';
            ",
        )
        .expect("downgrade topic migration fixture");
    drop(connection);

    let reopened = SqlitePcpStore::open(path.clone())
        .await
        .expect("migrate topic extraction schema");
    drop(reopened);
    let connection = Connection::open(&path).expect("inspect migrated topic Store");
    let version: String = connection
        .query_row(
            "SELECT value FROM pcp_metadata WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .expect("read topic Store schema version");
    assert_eq!(version, "0.8.0-clean.2");
    let table_count: u32 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name IN
                ('pcp_topic_extractions', 'pcp_topic_extraction_members')",
            [],
            |row| row.get(0),
        )
        .expect("count migrated topic tables");
    assert_eq!(table_count, 2);

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn migrates_draft_store_to_minimal_clean_schema() {
    let root = std::env::temp_dir().join(format!(
        "pcp-sqlite-clean-migration-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let path = root.join("pcp.sqlite3");
    let store = SqlitePcpStore::open(path.clone())
        .await
        .expect("open clean fixture Store");
    let identity_id = store.identity_id().to_owned();
    let namespace = "conversation:clean-migration".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Clean migration".to_owned(),
            description: None,
            parent_namespace: None,
        })
        .await
        .expect("create migration Scope");
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "model:clean-migration".to_owned(),
    };
    let page = store
        .write_page(
            write_request(
                &identity_id,
                &namespace,
                actor.clone(),
                "One canonical payload.",
                "clean-migration:page",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write migration Page");
    let summary = store
        .write_summary(
            WriteSummaryRequest {
                target_page_id: page.page_id.clone(),
                target_revision_id: page.revision_id.clone(),
                expected_summary_revision_id: None,
                content: "A compact routing summary.".to_owned(),
                created_by: actor,
                tool_or_model: Some("test-model".to_owned()),
                provenance: Vec::new(),
                idempotency_key: Some("clean-migration:summary".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("write migration Summary");
    drop(store);

    let connection = Connection::open(&path).expect("open draft fixture");
    connection
        .execute_batch(
            r#"
            PRAGMA foreign_keys = OFF;
            DROP TABLE pcp_validity_heads;
            DROP TABLE pcp_validity_assessments;
            CREATE TABLE pcp_validity_assessments (
                assessment_id TEXT PRIMARY KEY,
                previous_assessment_id TEXT,
                target_revision_id TEXT NOT NULL,
                standing TEXT NOT NULL,
                rationale TEXT NOT NULL,
                scope TEXT,
                assessed_at TEXT NOT NULL,
                actor_type TEXT NOT NULL,
                actor_id TEXT NOT NULL,
                tool_or_model TEXT,
                basis_revision_ids_json TEXT NOT NULL
            );
            CREATE TABLE pcp_validity_heads (
                target_page_id TEXT PRIMARY KEY,
                target_revision_id TEXT NOT NULL,
                current_assessment_id TEXT NOT NULL
            );
            UPDATE pcp_metadata
            SET value = '0.8.0-draft'
            WHERE key = 'schema_version';
            INSERT INTO pcp_access_log (
                event_id, occurred_at, principal_json, session_id, operation,
                scopes_json, decision, detail, telemetry_json
            ) VALUES (
                'evt_draft', '2026-08-16T00:00:00.000Z', '{}', 'session:draft',
                'search_pages', '[]', 'allowed', NULL, NULL
            );
            "#,
        )
        .expect("downgrade clean fixture to draft table shape");
    connection
        .execute(
            "UPDATE pcp_revisions
             SET facets_json = ?2, source_refs_json = ?3
             WHERE revision_id = ?1",
            params![
                page.revision_id,
                json!({
                    "kind": "document",
                    "contentParts": [{"type": "markdown", "text": "One canonical payload."}]
                })
                .to_string(),
                json!([{
                    "providerId": "legacy_markdown_memory",
                    "locator": "file:///obsolete/memory.md"
                }])
                .to_string()
            ],
        )
        .expect("seed redundant draft content");
    drop(connection);

    let reopened = SqlitePcpStore::open(path.clone())
        .await
        .expect("migrate draft Store");
    let connection = Connection::open(&path).expect("inspect migrated Store");
    let version: String = connection
        .query_row(
            "SELECT value FROM pcp_metadata WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .expect("read migrated version");
    assert_eq!(version, "0.8.0-clean.2");
    for (table, removed_column) in [
        ("pcp_relations", "from_revision_id"),
        ("pcp_summaries", "content"),
        ("pcp_validity_assessments", "assessment_id"),
    ] {
        let count: i64 = connection
            .query_row(
                "SELECT count(*) FROM pragma_table_info(?1) WHERE name = ?2",
                params![table, removed_column],
                |row| row.get(0),
            )
            .expect("inspect clean schema columns");
        assert_eq!(count, 0, "{table}.{removed_column} survived cleanup");
    }
    let (facets, sources): (Option<String>, String) = connection
        .query_row(
            "SELECT facets_json, source_refs_json FROM pcp_revisions WHERE revision_id = ?1",
            [&page.revision_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read normalized migration Page");
    assert!(facets.is_none());
    assert_eq!(sources, "[]");
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM pcp_access_log", [], |row| row
                .get::<_, i64>(0))
            .expect("count migrated access log"),
        0
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or(0),
        0
    );
    drop(connection);

    let projected = reopened
        .read_pages(
            ReadPagesRequest {
                page_ids: vec![page.page_id],
                revision_ids: Vec::new(),
                projections: vec![Projection::Summary],
                max_chars: 1_000,
            },
            vec![namespace],
        )
        .await
        .expect("read migrated Summary");
    assert_eq!(
        projected[0]
            .summary
            .as_ref()
            .map(|value| value.summary_revision_id.as_str()),
        Some(summary.summary_revision_id.as_str())
    );

    drop(reopened);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn migrates_clean_store_associations_without_erasing_exact_inputs() {
    let root = std::env::temp_dir().join(format!(
        "pcp-sqlite-association-cleanup-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let path = root.join("pcp.sqlite3");
    let store = SqlitePcpStore::open(path.clone())
        .await
        .expect("open association cleanup Store");
    let identity_id = store.identity_id().to_owned();
    let namespace = "conversation:association-cleanup".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Association cleanup".to_owned(),
            description: None,
            parent_namespace: None,
        })
        .await
        .expect("create association cleanup Scope");
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "codex:symbiont-d".to_owned(),
    };
    let source = store
        .write_page(
            write_request(
                &identity_id,
                &namespace,
                actor.clone(),
                "One exact source.",
                "association-cleanup:source",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write exact source");
    let mut autonomous = write_request(
        &identity_id,
        &namespace,
        actor.clone(),
        "Autonomous output with legacy context exposure.",
        "association-cleanup:autonomous",
    );
    autonomous.kind = "conversation_event".to_owned();
    autonomous.facets = Some(json!({"messageMetadata": {"origin": "autonomous"}}));
    autonomous.provenance = vec![ProvenanceEvent {
        operation: "ingest".to_owned(),
        actor: actor.clone(),
        timestamp: "2026-08-15T00:00:00Z".to_owned(),
        input_revision_ids: vec![source.revision_id.clone()],
        tool_or_model: Some("gpt-5.6-terra".to_owned()),
    }];
    let autonomous = store
        .write_page(autonomous, vec![namespace.clone()])
        .await
        .expect("write legacy autonomous Page");
    let mut exact = write_request(
        &identity_id,
        &namespace,
        actor.clone(),
        "Interactive output with one exact input.",
        "association-cleanup:exact",
    );
    exact.kind = "conversation_event".to_owned();
    exact.provenance = vec![ProvenanceEvent {
        operation: "ingest".to_owned(),
        actor: actor.clone(),
        timestamp: "2026-08-15T00:00:01Z".to_owned(),
        input_revision_ids: vec![source.revision_id.clone()],
        tool_or_model: Some("gpt-5.6-luna".to_owned()),
    }];
    let exact = store
        .write_page(exact, vec![namespace.clone()])
        .await
        .expect("write exact-input Page");
    store
        .link_pages(
            LinkPagesRequest {
                from_page_id: autonomous.page_id.clone(),
                relation_type: "references".to_owned(),
                to_page_id: source.page_id.clone(),
                basis_revision_ids: vec![
                    autonomous.revision_id.clone(),
                    source.revision_id.clone(),
                ],
                created_by: actor.clone(),
                idempotency_key: Some("association-cleanup:reference".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("write exact reference");
    let redundant = store
        .link_pages(
            LinkPagesRequest {
                from_page_id: autonomous.page_id.clone(),
                relation_type: "related_to".to_owned(),
                to_page_id: source.page_id.clone(),
                basis_revision_ids: vec![
                    autonomous.revision_id.clone(),
                    source.revision_id.clone(),
                ],
                created_by: actor,
                idempotency_key: Some("association-cleanup:redundant".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("write redundant semantic relation");
    drop(store);

    let connection = Connection::open(&path).expect("open legacy clean Store fixture");
    connection
        .execute(
            "UPDATE pcp_metadata SET value = '0.8.0-clean' WHERE key = 'schema_version'",
            [],
        )
        .expect("downgrade association cleanup fixture version");
    drop(connection);

    let reopened = SqlitePcpStore::open(path.clone())
        .await
        .expect("migrate clean Store associations");
    let pages = reopened
        .read_pages(
            ReadPagesRequest {
                page_ids: vec![autonomous.page_id.clone(), exact.page_id.clone()],
                revision_ids: Vec::new(),
                projections: vec![Projection::Provenance, Projection::Relations],
                max_chars: 4_000,
            },
            vec![namespace],
        )
        .await
        .expect("read cleaned associations");
    let autonomous_page = pages
        .iter()
        .find(|page| page.page.page_id == autonomous.page_id)
        .expect("autonomous Page remains");
    let exact_page = pages
        .iter()
        .find(|page| page.page.page_id == exact.page_id)
        .expect("exact-input Page remains");
    assert!(
        autonomous_page.revision.provenance[0]
            .input_revision_ids
            .is_empty()
    );
    assert_eq!(
        exact_page.revision.provenance[0].input_revision_ids,
        vec![source.revision_id]
    );
    assert!(autonomous_page.relations.iter().any(|relation| {
        relation.relation_type == "references" && relation.to_page_id == source.page_id
    }));
    assert!(
        !autonomous_page
            .relations
            .iter()
            .any(|relation| relation.relation_id == redundant.relation_id)
    );

    let connection = Connection::open(&path).expect("inspect association cleanup migration");
    assert_eq!(
        connection
            .query_row(
                "SELECT value FROM pcp_metadata WHERE key = 'schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("read cleaned Store version"),
        "0.8.0-clean.2"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT reason FROM pcp_relation_retractions WHERE relation_id = ?1",
                [&redundant.relation_id],
                |row| row.get::<_, String>(0),
            )
            .expect("read redundant relation retraction"),
        "redundant_with_references_relation"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM pcp_provenance_inputs WHERE derived_revision_id = ?1",
                [&autonomous.revision_id],
                |row| row.get::<_, i64>(0),
            )
            .expect("count cleaned provenance inputs"),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM pcp_provenance_inputs WHERE derived_revision_id = ?1",
                [&exact.revision_id],
                |row| row.get::<_, i64>(0),
            )
            .expect("count preserved exact provenance inputs"),
        1
    );
    drop(connection);
    drop(reopened);
    let _ = std::fs::remove_dir_all(root);
}

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
    let identity_id = store.identity_id().to_owned();
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
                namespace: namespace.clone(),
                display_name: namespace.clone(),
                description: None,
                parent_namespace: None,
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
            &identity_id,
            &scope_a,
            actor.clone(),
            "Visible only to client A.",
            "access:a",
        ))
        .await
        .expect("write Scope A page");
    let page_b = admin
        .write_page(write_request(
            &identity_id,
            &scope_b,
            actor.clone(),
            "Private beta launch detail.",
            "access:b",
        ))
        .await
        .expect("write Scope B page");
    let relation = admin
        .link_pages(LinkPagesRequest {
            from_page_id: page_a.page_id.clone(),
            relation_type: "related_to".to_owned(),
            to_page_id: page_b.page_id.clone(),
            basis_revision_ids: vec![page_a.revision_id.clone(), page_b.revision_id.clone()],
            created_by: actor.clone(),
            idempotency_key: Some("access:cross-link".to_owned()),
        })
        .await
        .expect("link across authorized Scopes");
    let reverse = admin
        .link_pages(LinkPagesRequest {
            from_page_id: page_b.page_id.clone(),
            relation_type: "related_to".to_owned(),
            to_page_id: page_a.page_id.clone(),
            basis_revision_ids: vec![page_b.revision_id.clone(), page_a.revision_id.clone()],
            created_by: actor.clone(),
            idempotency_key: None,
        })
        .await
        .expect("coalesce symmetric relation");
    assert_eq!(relation.relation_id, reverse.relation_id);
    assert!(relation.from_page_id < relation.to_page_id);

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
                page_ids: Vec::new(),
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
            page_ids: Vec::new(),
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
    assert!(
        client_a
            .plan_revision_retention(PlanRevisionRetentionRequest {
                scopes: vec![scope_a.clone()],
                policy: RetentionPolicy::default(),
            })
            .await
            .is_err()
    );
    let retention = admin
        .plan_revision_retention(PlanRevisionRetentionRequest {
            scopes: vec![scope_a.clone()],
            policy: RetentionPolicy::default(),
        })
        .await
        .expect("audit-authorized retention plan");
    assert_eq!(retention.scanned_pages, 1);
    let lease_request = PutRevisionRetentionLeaseRequest {
        namespace: scope_a.clone(),
        revision_id: page_a.revision_id.clone(),
        reason: "Explicit test milestone".to_owned(),
        expires_at: (Utc::now() + Duration::days(30)).to_rfc3339_opts(SecondsFormat::Millis, true),
        idempotency_key: "access-test:retention".to_owned(),
    };
    assert!(
        client_a
            .put_revision_retention_lease(lease_request.clone())
            .await
            .is_err()
    );
    let lease = admin
        .put_revision_retention_lease(lease_request)
        .await
        .expect("write-authorized retention lease");
    assert_eq!(lease.revision_id, page_a.revision_id);
    assert!(
        client_a
            .active_revision_retention_leases(vec![scope_a.clone()], 10)
            .await
            .is_err()
    );
    assert_eq!(
        admin
            .active_revision_retention_leases(vec![scope_a.clone()], 10)
            .await
            .expect("audit retention leases")
            .len(),
        1
    );

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
        &identity_id,
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
        &identity_id,
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
            page_ids: Vec::new(),
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
async fn retracted_relations_do_not_affect_reads_graphs_or_effective_pages() {
    let root = std::env::temp_dir().join(format!(
        "pcp-sqlite-retracted-relations-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let path = root.join("pcp.sqlite3");
    let store = SqlitePcpStore::open(path.clone())
        .await
        .expect("open relation Store");
    let identity_id = store.identity_id().to_owned();
    let namespace = "project:retracted-relations".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Retracted relations".to_owned(),
            description: None,
            parent_namespace: None,
        })
        .await
        .expect("create relation Scope");
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "model:relations".to_owned(),
    };
    let older = store
        .write_page(
            write_request(
                &identity_id,
                &namespace,
                actor.clone(),
                "Older visible evidence.",
                "relations:older",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write older Page");
    let newer = store
        .write_page(
            write_request(
                &identity_id,
                &namespace,
                actor.clone(),
                "Newer visible evidence.",
                "relations:newer",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write newer Page");
    let relation = store
        .link_pages(
            LinkPagesRequest {
                from_page_id: newer.page_id.clone(),
                relation_type: "supersedes".to_owned(),
                to_page_id: older.page_id.clone(),
                basis_revision_ids: vec![newer.revision_id.clone(), older.revision_id.clone()],
                created_by: actor,
                idempotency_key: None,
            },
            vec![namespace.clone()],
        )
        .await
        .expect("link superseding Pages");
    assert_eq!(
        store
            .page_count(vec![namespace.clone()])
            .await
            .expect("count effective Pages"),
        1
    );

    Connection::open(&path)
        .expect("open relation fixture")
        .execute(
            "INSERT INTO pcp_relation_retractions (
                relation_id, retracted_actor_type, retracted_actor_id,
                retracted_at, reason
             ) VALUES (?1, 'system', 'test', '2026-08-16T00:00:00.000Z', 'test')",
            [&relation.relation_id],
        )
        .expect("retract relation");
    assert_eq!(
        store
            .page_count(vec![namespace.clone()])
            .await
            .expect("count Pages after retraction"),
        2
    );
    let read = store
        .read_pages(
            ReadPagesRequest {
                page_ids: vec![older.page_id.clone()],
                revision_ids: Vec::new(),
                projections: vec![Projection::Relations],
                max_chars: 1_000,
            },
            vec![namespace.clone()],
        )
        .await
        .expect("read Page after relation retraction");
    assert!(read[0].relations.is_empty());
    let graph = store
        .search_pages(SearchPagesRequest {
            query: older.revision_id,
            scopes: vec![namespace.clone()],
            mode: SearchMode::Graph,
            term_match: SearchTermMatch::All,
            projections: Vec::new(),
            filters: SearchFilters::default(),
            limit: 10,
            cursor: None,
        })
        .await
        .expect("graph search after relation retraction");
    assert!(graph.hits.is_empty());
    let filtered = store
        .search_pages(SearchPagesRequest {
            query: "Older visible evidence".to_owned(),
            scopes: vec![namespace.clone()],
            mode: SearchMode::Exact,
            term_match: SearchTermMatch::All,
            projections: Vec::new(),
            filters: SearchFilters {
                relation_types: vec!["supersedes".to_owned()],
                ..SearchFilters::default()
            },
            limit: 10,
            cursor: None,
        })
        .await
        .expect("relation-filtered search after retraction");
    assert!(filtered.hits.is_empty());

    drop(store);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn aggregates_privacy_preserving_runtime_health() {
    let root = std::env::temp_dir().join(format!(
        "pcp-health-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let store = Arc::new(
        SqlitePcpStore::open(root.join("pcp.sqlite3"))
            .await
            .expect("open health store"),
    );
    let identity_id = store.identity_id().to_owned();
    let namespace = "project:health".to_owned();
    let admin = pcp_client(
        Arc::clone(&store),
        AccessSession::full_control(
            principal("host:health", AccessPrincipalType::Host),
            "session:health",
            vec![namespace.clone()],
        ),
    );
    admin
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Health project".to_owned(),
            description: None,
            parent_namespace: None,
        })
        .await
        .expect("create health Scope");
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "model:health".to_owned(),
    };
    let mut first_request = write_request(
        &identity_id,
        &namespace,
        actor.clone(),
        "Alpha signal remains current.",
        "health:first",
    );
    first_request.mutability = PageMutability::Sealed;
    first_request.source_span = Some(SourceSpan {
        stream_id: "health-stream".to_owned(),
        start: 1,
        end: 1,
    });
    let first = admin
        .write_page(first_request)
        .await
        .expect("write first health Page");
    let mut second_request = write_request(
        &identity_id,
        &namespace,
        actor,
        "Alpha signal is repeated with more words.",
        "health:second",
    );
    second_request.mutability = PageMutability::Sealed;
    second_request.source_span = Some(SourceSpan {
        stream_id: "health-stream".to_owned(),
        start: 2,
        end: 2,
    });
    let second = admin
        .write_page(second_request)
        .await
        .expect("write second health Page");
    let hit = admin
        .search_pages(SearchPagesRequest {
            query: "Alpha signal".to_owned(),
            scopes: vec![namespace.clone()],
            mode: SearchMode::Text,
            term_match: SearchTermMatch::All,
            projections: vec![Projection::Manifest, Projection::Payload],
            filters: SearchFilters::default(),
            limit: 10,
            cursor: None,
        })
        .await
        .expect("search health hit");
    assert_eq!(hit.hits.len(), 2);
    let miss = admin
        .search_pages(SearchPagesRequest {
            query: "never-present-token".to_owned(),
            scopes: vec![namespace.clone()],
            mode: SearchMode::Text,
            term_match: SearchTermMatch::All,
            projections: vec![Projection::Manifest, Projection::Payload],
            filters: SearchFilters::default(),
            limit: 10,
            cursor: None,
        })
        .await
        .expect("search health miss");
    assert!(miss.hits.is_empty());
    admin
        .read_pages(ReadPagesRequest {
            page_ids: Vec::new(),
            revision_ids: vec![first.revision_id.clone()],
            projections: vec![Projection::Manifest, Projection::Summary],
            max_chars: 2_000,
        })
        .await
        .expect("read Summary route");
    admin
        .read_pages(ReadPagesRequest {
            page_ids: Vec::new(),
            revision_ids: vec![second.revision_id.clone()],
            projections: vec![Projection::Payload],
            max_chars: 2_000,
        })
        .await
        .expect("read detail route");
    admin
        .pack_pages(PackPagesRequest {
            pages: vec![
                PageRevisionRef {
                    page_id: first.page_id.clone(),
                    revision_id: first.revision_id.clone(),
                },
                PageRevisionRef {
                    page_id: second.page_id,
                    revision_id: second.revision_id,
                },
            ],
            idempotency_key: Some("health:pack".to_owned()),
        })
        .await
        .expect("pack health Pages");
    let legacy_scope = namespace.clone();
    store
        .run("seed legacy health event", move |connection| {
            connection.execute(
                "
                INSERT INTO pcp_access_log (
                    event_id, occurred_at, principal_json, session_id,
                    operation, scopes_json, decision, detail, telemetry_json
                ) VALUES (
                    'acc_legacy_health', ?1,
                    '{\"principalId\":\"host:legacy\",\"principalType\":\"host\"}',
                    'session:legacy', 'search_pages', ?2, 'allowed', NULL, NULL
                )
                ",
                rusqlite::params![
                    chrono::Utc::now().to_rfc3339(),
                    serde_json::to_string(&vec![legacy_scope])?,
                ],
            )?;
            Ok(())
        })
        .await
        .expect("seed legacy access event without telemetry");

    let health = admin
        .health_snapshot(vec![namespace.clone()], 24)
        .await
        .expect("read PCP health snapshot");
    assert_eq!(health.storage.current_pages, 1);
    assert_eq!(health.storage.pages, 1);
    assert_eq!(health.storage.revisions, 1);
    assert_eq!(health.storage.historical_revisions, 0);
    assert_eq!(health.storage.sealed_pages, 0);
    assert_eq!(health.storage.revisioned_pages, 1);
    assert_eq!(health.recall.searches, 2);
    assert_eq!(health.recall.zero_result_searches, 1);
    assert_eq!(health.recall.returned_pages, 2);
    assert_eq!(health.recall.summary_reads, 1);
    assert_eq!(health.recall.detail_reads, 1);
    assert_eq!(health.packing.runs, 1);
    assert_eq!(health.packing.input_pages, 2);
    assert_eq!(health.packing.net_page_reduction, 1);
    assert_eq!(health.graph.relations, 0);
    assert!(health.activity.calls >= 7);
    assert_eq!(health.activity.measured_calls + 1, health.activity.calls);

    let (events, _) = admin
        .access_log(100, None)
        .await
        .expect("read telemetry audit");
    let search_event = events
        .iter()
        .find(|event| event.operation == "search_pages" && event.telemetry.is_some())
        .expect("search telemetry event");
    let telemetry = search_event.telemetry.as_ref().expect("search telemetry");
    assert_eq!(telemetry.input_count, Some(1));
    assert_eq!(telemetry.output_count, Some(0));
    assert_eq!(telemetry.projections, vec!["manifest", "payload"]);
    assert!(search_event.detail.is_none());

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
    let identity_id = store.identity_id().to_owned();
    let namespace = "conversation:test".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Test conversation".to_owned(),
            description: None,
            parent_namespace: None,
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
                &identity_id,
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
                &identity_id,
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
                    provider_id: "test_file".to_owned(),
                    locator: "file:///tmp/pcp-source.md#L1-L3".to_owned(),
                    media_type: Some("text/markdown".to_owned()),
                    content_digest: None,
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
    assert_eq!(derived_from_source.hits[0].graph_edges.len(), 1);
    assert_eq!(
        derived_from_source.hits[0].graph_edges[0].edge_kind,
        GraphEdgeKind::Provenance
    );
    assert_eq!(
        derived_from_source.hits[0].graph_edges[0].direction,
        GraphEdgeDirection::Incoming
    );
    assert_eq!(
        derived_from_source.hits[0].graph_edges[0].basis_revision_ids,
        vec![revised.revision_id.clone(), first.revision_id.clone()]
    );

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
    assert_eq!(source_from_derived.hits[0].graph_edges.len(), 1);
    assert_eq!(
        source_from_derived.hits[0].graph_edges[0].direction,
        GraphEdgeDirection::Outgoing
    );

    let second = store
        .write_page(
            write_request(
                &identity_id,
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
                from_page_id: second.page_id.clone(),
                relation_type: "depends_on".to_owned(),
                to_page_id: revised.page_id.clone(),
                basis_revision_ids: vec![second.revision_id.clone(), revised.revision_id.clone()],
                created_by: actor.clone(),
                idempotency_key: Some("link:first".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("link pages");

    let revised_second = store
        .revise_page(
            RevisePageRequest {
                page_id: second.page_id.clone(),
                expected_revision_id: second.revision_id.clone(),
                created_by: actor.clone(),
                lifecycle_status: LifecycleStatus::Active,
                observed_at: None,
                valid_from: None,
                valid_to: None,
                payload: Some(PagePayload {
                    media_type: "text/markdown".to_owned(),
                    content: "The related open question is now more precise.".to_owned(),
                }),
                source_refs: Vec::new(),
                facets: None,
                provenance: vec![ProvenanceEvent {
                    operation: "revise".to_owned(),
                    actor: actor.clone(),
                    timestamp: "2026-07-29T00:05:00Z".to_owned(),
                    input_revision_ids: vec![second.revision_id.clone()],
                    tool_or_model: Some("test".to_owned()),
                }],
                initial_relations: Vec::new(),
                idempotency_key: Some("revision:second".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("revise linked Page");

    let graph_from_old_revision = store
        .search_pages(SearchPagesRequest {
            query: second.revision_id.clone(),
            scopes: vec![namespace.clone()],
            mode: SearchMode::Graph,
            term_match: SearchTermMatch::All,
            projections: pcp_core::default_search_projections(),
            filters: SearchFilters {
                relation_types: vec!["depends_on".to_owned()],
                ..SearchFilters::default()
            },
            limit: 10,
            cursor: None,
        })
        .await
        .expect("search Page relation from historical Revision");
    assert_eq!(graph_from_old_revision.hits.len(), 1);
    assert_eq!(
        graph_from_old_revision.hits[0].revision_id,
        revised.revision_id
    );

    let graph_to_revised_page = store
        .search_pages(SearchPagesRequest {
            query: revised.revision_id.clone(),
            scopes: vec![namespace.clone()],
            mode: SearchMode::Graph,
            term_match: SearchTermMatch::All,
            projections: pcp_core::default_search_projections(),
            filters: SearchFilters {
                relation_types: vec!["depends_on".to_owned()],
                ..SearchFilters::default()
            },
            limit: 10,
            cursor: None,
        })
        .await
        .expect("follow Page relation to current head");
    assert_eq!(graph_to_revised_page.hits.len(), 1);
    assert_eq!(
        graph_to_revised_page.hits[0].revision_id,
        revised_second.revision_id
    );

    let relation_filtered = store
        .search_pages(SearchPagesRequest {
            query: String::new(),
            scopes: vec![namespace.clone()],
            mode: SearchMode::Temporal,
            term_match: SearchTermMatch::All,
            projections: pcp_core::default_search_projections(),
            filters: SearchFilters {
                relation_types: vec!["depends_on".to_owned()],
                ..SearchFilters::default()
            },
            limit: 10,
            cursor: None,
        })
        .await
        .expect("filter current Pages by stable relation");
    assert_eq!(relation_filtered.hits.len(), 2);
    assert!(
        relation_filtered
            .hits
            .iter()
            .any(|hit| { hit.revision_id == revised_second.revision_id })
    );
    assert!(
        relation_filtered
            .hits
            .iter()
            .all(|hit| { hit.revision_id != second.revision_id })
    );

    let read = store
        .read_pages(
            ReadPagesRequest {
                page_ids: Vec::new(),
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
    assert_eq!(read[0].relations.len(), 1);
    assert!(
        read[0]
            .relations
            .iter()
            .all(|relation| relation.relation_type != "supersedes")
    );
    assert!(read[0].revision.source_refs.is_empty());
    assert!(read[0].revision.provenance.is_empty());
    let lean_json = serde_json::to_value(&read[0]).expect("serialize lean projection");
    assert!(lean_json["page"].get("sourceRefs").is_none());
    assert!(lean_json["page"].get("provenance").is_none());

    let traced = store
        .read_pages(
            ReadPagesRequest {
                page_ids: Vec::new(),
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
        &identity_id,
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
    let identity_id = store.identity_id().to_owned();
    let namespace = "conversation:summary".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Summary conversation".to_owned(),
            description: None,
            parent_namespace: None,
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
                &identity_id,
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
        &identity_id,
        &namespace,
        actor.clone(),
        &"rolling current map ".repeat(20),
        "summary:operational",
    );
    operational.kind = "runtime_context".to_owned();
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
                target_page_id: page.page_id.clone(),
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
                target_page_id: page.page_id.clone(),
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
        .browse_index(
            vec![namespace.clone()],
            Vec::new(),
            BrowseIndexOrder::Recent,
            10,
            None,
            8_000,
        )
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
                page_ids: Vec::new(),
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
                page_ids: Vec::new(),
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
    assert!(summary_detail[0].revision.facets.is_none());
    assert!(summary_detail[0].relations.iter().any(|relation| {
        relation.relation_type == "summarizes" && relation.to_page_id == page.page_id
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
                target_page_id: page.page_id.clone(),
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
    assert_eq!(revised.summary_page_id, summary.summary_page_id);
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
                target_page_id: page.page_id.clone(),
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
    let identity_id = store.identity_id().to_owned();
    let namespace = "project:dag".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "DAG project".to_owned(),
            description: None,
            parent_namespace: None,
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
                &identity_id,
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
                &identity_id,
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
                from_page_id: second.page_id.clone(),
                relation_type: "aggregates".to_owned(),
                to_page_id: first.page_id.clone(),
                basis_revision_ids: vec![second.revision_id.clone(), first.revision_id.clone()],
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
                from_page_id: first.page_id,
                relation_type: "derived_from".to_owned(),
                to_page_id: second.page_id,
                basis_revision_ids: vec![first.revision_id, second.revision_id],
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
    let identity_id = store.identity_id().to_owned();
    let namespace = "conversation:retract".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Retraction test".to_owned(),
            description: None,
            parent_namespace: None,
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
                &identity_id,
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
                &identity_id,
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
        &identity_id,
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
                &identity_id,
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

    let retracted = store
        .read_pages(
            ReadPagesRequest {
                page_ids: vec![source.page_id.clone(), derived.page_id.clone()],
                revision_ids: Vec::new(),
                projections: vec![Projection::Manifest, Projection::Facets],
                max_chars: 1_000,
            },
            vec![namespace.clone()],
        )
        .await
        .expect("read retracted Page manifests");
    assert_eq!(retracted.len(), 2);
    assert!(retracted.iter().all(|page| {
        page.page.kind == "tombstone"
            && page.page.lifecycle_status == LifecycleStatus::Tombstoned
            && page.revision.lifecycle_status == LifecycleStatus::Tombstoned
    }));

    let restored_revision = store
        .current_revision_id(durable.page_id.clone(), vec![namespace.clone()])
        .await
        .expect("read restored head");
    let restored = store
        .read_pages(
            ReadPagesRequest {
                page_ids: Vec::new(),
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
    let restored_manifest = store
        .read_pages(
            ReadPagesRequest {
                page_ids: vec![durable.page_id.clone()],
                revision_ids: Vec::new(),
                projections: vec![Projection::Manifest],
                max_chars: 1_000,
            },
            vec![namespace.clone()],
        )
        .await
        .expect("read restored Page manifest");
    let restored_manifest = &restored_manifest[0].page;
    assert_eq!(restored_manifest.kind, "document");
    assert_eq!(restored_manifest.lifecycle_status, LifecycleStatus::Active);
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
    let identity_id = store.identity_id().to_owned();
    let namespace = "conversation:validity".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Validity conversation".to_owned(),
            description: None,
            parent_namespace: None,
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
                &identity_id,
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
                &identity_id,
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
                target_page_id: target.page_id.clone(),
                target_revision_id: target.revision_id.clone(),
                expected_assessment_revision_id: None,
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
                page_ids: Vec::new(),
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
    assert_eq!(
        validity.assessment_revision_id,
        first.assessment_revision_id
    );
    assert_eq!(
        validity.basis_revision_ids,
        vec![evidence.revision_id.clone()]
    );

    let revised = store
        .assess_page_validity(
            AssessPageValidityRequest {
                target_page_id: target.page_id.clone(),
                target_revision_id: target.revision_id.clone(),
                expected_assessment_revision_id: Some(first.assessment_revision_id.clone()),
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
    assert_eq!(revised.assessment_page_id, first.assessment_page_id);
    assert_ne!(revised.assessment_revision_id, first.assessment_revision_id);
    let latest = store
        .read_pages(
            ReadPagesRequest {
                page_ids: Vec::new(),
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
            .and_then(|value| value.previous_assessment_revision_id.as_deref()),
        Some(first.assessment_revision_id.as_str())
    );
    assert_eq!(
        latest[0].validity.as_ref().map(|value| &value.standing),
        Some(&ValidityStanding::Superseded)
    );
    assert_eq!(
        latest[0]
            .validity
            .as_ref()
            .map(|value| value.assessment_revision_id.as_str()),
        Some(revised.assessment_revision_id.as_str())
    );
    assert_eq!(latest[0].validity_history.len(), 1);
    assert_eq!(
        latest[0].validity_history[0].assessment_revision_id,
        first.assessment_revision_id
    );

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn browse_index_orders_current_heads_by_inventory_property() {
    let root = std::env::temp_dir().join(format!(
        "pcp-browse-index-order-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let store = SqlitePcpStore::open(root.join("pcp.sqlite3"))
        .await
        .expect("open browse index Store");
    let identity_id = store.identity_id().to_owned();
    let namespace = "conversation:inventory-order".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Inventory order".to_owned(),
            description: None,
            parent_namespace: None,
        })
        .await
        .expect("create scope");
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "model:inventory-test".to_owned(),
    };

    let mut first_request = write_request(
        &identity_id,
        &namespace,
        actor.clone(),
        "First current Page.",
        "inventory-order:first",
    );
    first_request.source_span = Some(SourceSpan {
        stream_id: "host:inventory-order".to_owned(),
        start: 10,
        end: 10,
    });
    let first = store
        .write_page(first_request, vec![namespace.clone()])
        .await
        .expect("write first Page");

    let mut second_request = write_request(
        &identity_id,
        &namespace,
        actor.clone(),
        "Second current Page.",
        "inventory-order:second",
    );
    second_request.source_span = Some(SourceSpan {
        stream_id: "host:inventory-order".to_owned(),
        start: 20,
        end: 20,
    });
    let second = store
        .write_page(second_request, vec![namespace.clone()])
        .await
        .expect("write second Page");

    let mut large_request = write_request(
        &identity_id,
        &namespace,
        actor.clone(),
        &"Largest current Page. ".repeat(300),
        "inventory-order:largest",
    );
    large_request.source_span = Some(SourceSpan {
        stream_id: "host:inventory-order".to_owned(),
        start: 30,
        end: 30,
    });
    let large = store
        .write_page(large_request, vec![namespace.clone()])
        .await
        .expect("write largest Page");

    for (target, key) in [
        (&second, "inventory-order:first-second"),
        (&large, "inventory-order:first-large"),
    ] {
        store
            .link_pages(
                LinkPagesRequest {
                    from_page_id: first.page_id.clone(),
                    relation_type: "related_to".to_owned(),
                    to_page_id: target.page_id.clone(),
                    basis_revision_ids: vec![first.revision_id.clone(), target.revision_id.clone()],
                    created_by: actor.clone(),
                    idempotency_key: Some(key.to_owned()),
                },
                vec![namespace.clone()],
            )
            .await
            .expect("link direct relation");
    }

    let by_connections = store
        .browse_index(
            vec![namespace.clone()],
            Vec::new(),
            BrowseIndexOrder::MostConnected,
            10,
            None,
            8_000,
        )
        .await
        .expect("browse most connected");
    assert_eq!(by_connections.hits[0].page_id, first.page_id);

    let by_fewest_connections = store
        .browse_index(
            vec![namespace.clone()],
            Vec::new(),
            BrowseIndexOrder::LeastConnected,
            10,
            None,
            8_000,
        )
        .await
        .expect("browse least connected");
    assert_ne!(by_fewest_connections.hits[0].page_id, first.page_id);

    let by_size = store
        .browse_index(
            vec![namespace.clone()],
            Vec::new(),
            BrowseIndexOrder::Largest,
            10,
            None,
            8_000,
        )
        .await
        .expect("browse largest");
    assert_eq!(by_size.hits[0].page_id, large.page_id);

    let by_source = store
        .browse_index(
            vec![namespace.clone()],
            Vec::new(),
            BrowseIndexOrder::SourceOrder,
            10,
            None,
            8_000,
        )
        .await
        .expect("browse source order");
    assert_eq!(
        by_source
            .hits
            .iter()
            .map(|hit| hit.page_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            first.page_id.as_str(),
            second.page_id.as_str(),
            large.page_id.as_str()
        ]
    );

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn content_library_excludes_attached_summaries_without_restricting_page_kind() {
    let root = std::env::temp_dir().join(format!(
        "pcp-content-library-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let store = SqlitePcpStore::open(root.join("pcp.sqlite3"))
        .await
        .expect("open content library Store");
    let identity_id = store.identity_id().to_owned();
    let namespace = "conversation:content-library".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Content library".to_owned(),
            description: None,
            parent_namespace: None,
        })
        .await
        .expect("create Scope");
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "model:content-library-test".to_owned(),
    };

    let mut source_request = write_request(
        &identity_id,
        &namespace,
        actor.clone(),
        "Original source text.",
        "content-library:source",
    );
    source_request.kind = "summary_projection".to_owned();
    let source = store
        .write_page(source_request, vec![namespace.clone()])
        .await
        .expect("write source Page");

    let summary = store
        .write_summary(
            WriteSummaryRequest {
                target_page_id: source.page_id.clone(),
                target_revision_id: source.revision_id.clone(),
                expected_summary_revision_id: None,
                content: "Routing phrase distinct from source.".to_owned(),
                created_by: actor,
                tool_or_model: Some("test:content-library".to_owned()),
                provenance: Vec::new(),
                idempotency_key: Some("content-library:summary".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("write attached summary");

    let browse = store
        .browse_content_pages(
            vec![namespace.clone()],
            Some("routing phrase".to_owned()),
            BrowseIndexOrder::Recent,
            10,
            None,
            8_000,
        )
        .await
        .expect("browse content library");
    assert_eq!(browse.total_pages, 1);
    assert_eq!(browse.hits.len(), 1);
    assert_eq!(browse.hits[0].page_id, source.page_id);
    assert_eq!(browse.hits[0].kind, "summary_projection");
    assert_eq!(
        browse.hits[0].summary_revision_id.as_deref(),
        Some(summary.summary_revision_id.as_str())
    );

    let summary = store
        .content_library_summary(vec![namespace.clone()])
        .await
        .expect("summarize content library");
    assert_eq!(summary.page_count, 1);
    assert_eq!(summary.scopes.len(), 1);
    assert_eq!(summary.scopes[0].namespace, namespace);
    assert_eq!(
        summary.content_chars,
        "Original source text.".chars().count() as u64
    );

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn query_audit_summarizes_router_usage_without_query_text_or_page_content() {
    let root = std::env::temp_dir().join(format!(
        "pcp-query-audit-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let store = Arc::new(
        SqlitePcpStore::open(root.join("pcp.sqlite3"))
            .await
            .expect("open Store"),
    );
    let namespace = "project:query-audit".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Query audit".to_owned(),
            description: None,
            parent_namespace: None,
        })
        .await
        .expect("create query audit scope");
    let principal = principal("operator:query-audit", AccessPrincipalType::Service);
    store
        .record_runtime_query_audit(QueryAuditEvent {
            event_id: "qa_fixture".to_owned(),
            occurred_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            principal: principal.clone(),
            session_id: "session:query-audit".to_owned(),
            method: QueryAuditMethod::MatchIntent,
            effort: Some(pcp_core::IntentEffort::High),
            scopes: vec![namespace.clone()],
            decision: AccessDecision::Allowed,
            duration_ms: 240,
            anchor_count: 3,
            related_count: 1,
            context_chars: 1_200,
            semantic_indexed_count: Some(42),
            semantic_embedded_count: Some(0),
            router_rounds: Some(2),
            router_usage: Some(RouterTokenUsage {
                reported_responses: 2,
                input_tokens: 100,
                output_tokens: 25,
                total_tokens: 125,
                ..RouterTokenUsage::default()
            }),
            failure_kind: None,
        })
        .await
        .expect("record Runtime query audit");
    let client = pcp_client(
        Arc::clone(&store),
        AccessSession::new(
            principal,
            "session:query-audit",
            vec![ScopeGrant {
                namespace: namespace.clone(),
                permissions: vec![AccessPermission::Audit],
            }],
        ),
    );
    let summary = client
        .query_audit_summary(vec![namespace.clone()], 24)
        .await
        .expect("read query audit summary");
    assert_eq!(summary.calls, 1);
    assert_eq!(summary.match_intent.calls, 1);
    assert_eq!(summary.match_intent.context_chars, 1_200);
    assert_eq!(summary.router_usage.total_tokens, 125);
    assert_eq!(summary.recent_events.len(), 1);
    assert!(summary.recent_events[0].failure_kind.is_none());
    assert!(summary.recent_events[0].scopes.contains(&namespace));

    let _ = std::fs::remove_dir_all(root);
}

fn write_request(
    _identity_id: &str,
    namespace: &str,
    actor: Actor,
    content: &str,
    idempotency_key: &str,
) -> WritePageRequest {
    WritePageRequest {
        namespace: namespace.to_owned(),
        lifecycle_status: LifecycleStatus::Active,
        kind: "document".to_owned(),
        mutability: PageMutability::Revisioned,
        created_by: actor,
        observed_at: None,
        source_span: None,
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
async fn text_search_boosts_directly_related_lexical_pages() {
    let root = std::env::temp_dir().join(format!(
        "pcp-text-relation-ranking-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let store = SqlitePcpStore::open(root.join("pcp.sqlite3"))
        .await
        .expect("open store");
    let identity_id = store.identity_id().to_owned();
    let namespace = "project:text-relation-ranking".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Text relation ranking".to_owned(),
            description: None,
            parent_namespace: None,
        })
        .await
        .expect("create scope");
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "model:text-relation-ranking".to_owned(),
    };
    let anchor = store
        .write_page(
            write_request(
                &identity_id,
                &namespace,
                actor.clone(),
                "PCP structure anchor evidence.",
                "text-relation-ranking:anchor",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write anchor Page");
    let supported = store
        .write_page(
            write_request(
                &identity_id,
                &namespace,
                actor.clone(),
                "PCP structure supported evidence.",
                "text-relation-ranking:supported",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write supported Page");
    let isolated = store
        .write_page(
            write_request(
                &identity_id,
                &namespace,
                actor.clone(),
                "PCP structure isolated evidence.",
                "text-relation-ranking:isolated",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write isolated Page");
    store
        .link_pages(
            LinkPagesRequest {
                from_page_id: anchor.page_id.clone(),
                relation_type: "related_to".to_owned(),
                to_page_id: supported.page_id.clone(),
                basis_revision_ids: vec![anchor.revision_id.clone(), supported.revision_id.clone()],
                created_by: actor,
                idempotency_key: Some("text-relation-ranking:related".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("link lexical Pages");

    let result = store
        .search_pages(SearchPagesRequest {
            query: "PCP structure".to_owned(),
            scopes: vec![namespace.clone()],
            mode: SearchMode::Text,
            term_match: SearchTermMatch::All,
            projections: vec![Projection::Payload],
            filters: SearchFilters::default(),
            limit: 10,
            cursor: None,
        })
        .await
        .expect("search lexical Pages");
    let page_ids = result
        .hits
        .iter()
        .map(|hit| hit.page_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(page_ids.len(), 3);
    assert!(
        page_ids[..2].contains(&anchor.page_id.as_str())
            && page_ids[..2].contains(&supported.page_id.as_str())
    );
    assert_eq!(page_ids[2], isolated.page_id);

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn plans_revision_retention_from_roots_without_deleting_history() {
    let root = std::env::temp_dir().join(format!(
        "pcp-retention-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let store = SqlitePcpStore::open(root.join("pcp.sqlite3"))
        .await
        .expect("open store");
    let identity_id = store.identity_id().to_owned();
    let namespace = "project:retention".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Retention planning".to_owned(),
            description: None,
            parent_namespace: None,
        })
        .await
        .expect("create Scope");
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "model:retention".to_owned(),
    };
    let first = store
        .write_page(
            write_request(
                &identity_id,
                &namespace,
                actor.clone(),
                "Revision zero.",
                "retention:revision:0",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write revisioned Page");
    let mut revisions = vec![first.clone()];
    let mut current = first;
    for index in 1..4 {
        current = store
            .revise_page(
                RevisePageRequest {
                    page_id: current.page_id.clone(),
                    expected_revision_id: current.revision_id.clone(),
                    created_by: actor.clone(),
                    lifecycle_status: LifecycleStatus::Active,
                    observed_at: None,
                    valid_from: None,
                    valid_to: None,
                    payload: Some(PagePayload {
                        media_type: "text/markdown".to_owned(),
                        content: format!("Revision {index}."),
                    }),
                    source_refs: Vec::new(),
                    facets: None,
                    provenance: Vec::new(),
                    initial_relations: Vec::new(),
                    idempotency_key: Some(format!("retention:revision:{index}")),
                },
                vec![namespace.clone()],
            )
            .await
            .expect("revise Page");
        revisions.push(current.clone());
    }
    let anchor = store
        .write_page(
            write_request(
                &identity_id,
                &namespace,
                actor.clone(),
                "Stable relation anchor.",
                "retention:anchor",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write relation anchor");
    store
        .link_pages(
            LinkPagesRequest {
                from_page_id: anchor.page_id,
                relation_type: "depends_on".to_owned(),
                to_page_id: current.page_id,
                basis_revision_ids: vec![revisions[0].revision_id.clone()],
                created_by: actor,
                idempotency_key: Some("retention:relation".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("link exact historical basis");

    let plan = store
        .plan_revision_retention(PlanRevisionRetentionRequest {
            scopes: vec![namespace],
            policy: RetentionPolicy {
                minimum_age_days: 0,
                keep_recent_revisions_per_page: 2,
                sample_limit: 20,
            },
        })
        .await
        .expect("plan Revision retention");

    assert_eq!(plan.scanned_pages, 2);
    assert_eq!(plan.scanned_revisions, 5);
    assert_eq!(plan.candidate_revisions, 1);
    assert_eq!(plan.candidates[0].revision_id, revisions[1].revision_id);
    assert!(plan.candidate_estimated_bytes > 0);
    assert_eq!(plan.past_window_idempotency_records, 5);
    assert!(plan.protection_reasons.iter().any(|count| {
        count.reason == RetentionProtectionReason::RelationBasis && count.revisions == 3
    }));
    assert!(plan.protected_samples.iter().any(|sample| {
        sample.revision_id == revisions[0].revision_id
            && sample
                .reasons
                .contains(&RetentionProtectionReason::RelationBasis)
    }));

    store
        .put_revision_retention_lease(
            "service:retention-test".to_owned(),
            PutRevisionRetentionLeaseRequest {
                namespace: "project:retention".to_owned(),
                revision_id: revisions[1].revision_id.clone(),
                reason: "Preserve a semantic milestone while it remains useful".to_owned(),
                expires_at: (Utc::now() + Duration::days(90))
                    .to_rfc3339_opts(SecondsFormat::Millis, true),
                idempotency_key: "retention:lease:milestone".to_owned(),
            },
        )
        .await
        .expect("write explicit retention lease");
    let leased_plan = store
        .plan_revision_retention(PlanRevisionRetentionRequest {
            scopes: vec!["project:retention".to_owned()],
            policy: RetentionPolicy {
                minimum_age_days: 0,
                keep_recent_revisions_per_page: 2,
                sample_limit: 20,
            },
        })
        .await
        .expect("plan Revision retention with semantic lease");
    assert_eq!(leased_plan.active_retention_leases, 1);
    assert_eq!(leased_plan.candidate_revisions, 0);
    assert!(leased_plan.protected_samples.iter().any(|sample| {
        sample.revision_id == revisions[1].revision_id
            && sample
                .reasons
                .contains(&RetentionProtectionReason::ExplicitLease)
    }));

    let history = store
        .read_pages(
            ReadPagesRequest {
                page_ids: vec![revisions[0].page_id.clone()],
                revision_ids: Vec::new(),
                projections: vec![Projection::History],
                max_chars: 8_000,
            },
            vec!["project:retention".to_owned()],
        )
        .await
        .expect("retention plan leaves history untouched");
    assert_eq!(history[0].history.len(), 4);

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn collects_only_replanned_revision_candidates_and_preserves_a_compact_ledger() {
    let root = std::env::temp_dir().join(format!(
        "pcp-retention-collection-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let path = root.join("pcp.sqlite3");
    let store = SqlitePcpStore::open(path.clone())
        .await
        .expect("open store");
    let identity_id = store.identity_id().to_owned();
    let namespace = "project:retention-collection".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Retention collection".to_owned(),
            description: None,
            parent_namespace: None,
        })
        .await
        .expect("create Scope");
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "model:retention-collection".to_owned(),
    };
    let first = store
        .write_page(
            write_request(
                &identity_id,
                &namespace,
                actor.clone(),
                "Initial maintained state.",
                "retention-collection:0",
            ),
            vec![namespace.clone()],
        )
        .await
        .expect("write maintained Page");
    let mut current = first;
    for index in 1..5 {
        current = store
            .revise_page(
                RevisePageRequest {
                    page_id: current.page_id.clone(),
                    expected_revision_id: current.revision_id.clone(),
                    created_by: actor.clone(),
                    lifecycle_status: LifecycleStatus::Active,
                    observed_at: None,
                    valid_from: None,
                    valid_to: None,
                    payload: Some(PagePayload {
                        media_type: "text/markdown".to_owned(),
                        content: format!("Maintained state {index}."),
                    }),
                    source_refs: Vec::new(),
                    facets: None,
                    provenance: Vec::new(),
                    initial_relations: Vec::new(),
                    idempotency_key: Some(format!("retention-collection:{index}")),
                },
                vec![namespace.clone()],
            )
            .await
            .expect("revise maintained Page");
    }
    let policy = RetentionPolicy {
        minimum_age_days: 0,
        keep_recent_revisions_per_page: 2,
        sample_limit: 20,
    };
    let plan = store
        .plan_revision_retention(PlanRevisionRetentionRequest {
            scopes: vec![namespace.clone()],
            policy: policy.clone(),
        })
        .await
        .expect("plan collection");
    assert_eq!(plan.candidate_revisions, 3);
    let revision_ids = plan
        .candidates
        .iter()
        .map(|candidate| candidate.revision_id.clone())
        .collect::<Vec<_>>();
    let result = store
        .collect_revision_retention(
            "operator:retention-test".to_owned(),
            CollectRevisionRetentionRequest {
                scopes: vec![namespace.clone()],
                policy,
                revision_ids: revision_ids.clone(),
            },
        )
        .await
        .expect("collect confirmed Revisions");
    assert_eq!(result.collected_revisions, 3);
    assert_eq!(result.collected_pages, 1);
    assert!(result.reclaimed_estimated_bytes > 0);

    let history = store
        .read_pages(
            ReadPagesRequest {
                page_ids: vec![current.page_id],
                revision_ids: Vec::new(),
                projections: vec![Projection::History],
                max_chars: 8_000,
            },
            vec![namespace],
        )
        .await
        .expect("read compacted history");
    assert_eq!(history[0].history.len(), 2);
    assert_eq!(
        store
            .integrity_check()
            .await
            .expect("check compacted store"),
        "ok"
    );
    drop(store);

    let connection = Connection::open(path).expect("open collection ledger");
    let ledger_entries: i64 = connection
        .query_row("SELECT count(*) FROM pcp_revision_collections", [], |row| {
            row.get(0)
        })
        .expect("count collection ledger");
    assert_eq!(ledger_entries, 3);
    for revision_id in revision_ids {
        let retained_payload: i64 = connection
            .query_row(
                "SELECT count(*) FROM pcp_revisions WHERE revision_id = ?1",
                [revision_id],
                |row| row.get(0),
            )
            .expect("check collected Revision payload");
        assert_eq!(retained_payload, 0);
    }

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn packs_contiguous_unreferenced_sealed_pages_without_rewriting_content() {
    let root = std::env::temp_dir().join(format!(
        "pcp-packing-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let path = root.join("pcp.sqlite3");
    let store = SqlitePcpStore::open(path.clone())
        .await
        .expect("open store");
    let identity_id = store.identity_id().to_owned();
    let namespace = "project:packing".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Packing project".to_owned(),
            description: None,
            parent_namespace: None,
        })
        .await
        .expect("create scope");
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "host:packing".to_owned(),
    };
    let mut pages = Vec::new();
    for (index, content) in [
        "The runtime keeps durable state.",
        "Durable state survives task restarts.",
        "The same runtime state remains auditable.",
    ]
    .into_iter()
    .enumerate()
    {
        let mut request = write_request(
            &identity_id,
            &namespace,
            actor.clone(),
            content,
            &format!("packing:{index}"),
        );
        request.kind = "conversation_event".to_owned();
        request.mutability = PageMutability::Sealed;
        request.source_span = Some(SourceSpan {
            stream_id: "conversation:main".to_owned(),
            start: index as u64,
            end: index as u64,
        });
        pages.push(
            store
                .write_page(request, vec![namespace.clone()])
                .await
                .expect("write packing input"),
        );
    }
    let mut mismatched_request = write_request(
        &identity_id,
        &namespace,
        actor.clone(),
        "A source document that must remain independent evidence.",
        "packing:mismatched-kind",
    );
    mismatched_request.kind = "source_document".to_owned();
    mismatched_request.mutability = PageMutability::Sealed;
    mismatched_request.source_span = Some(SourceSpan {
        stream_id: "conversation:main".to_owned(),
        start: 3,
        end: 3,
    });
    let mismatched = store
        .write_page(mismatched_request, vec![namespace.clone()])
        .await
        .expect("write semantically distinct packing input");
    let mismatch_error = store
        .pack_pages(
            PackPagesRequest {
                pages: vec![
                    PageRevisionRef {
                        page_id: pages[0].page_id.clone(),
                        revision_id: pages[0].revision_id.clone(),
                    },
                    PageRevisionRef {
                        page_id: mismatched.page_id,
                        revision_id: mismatched.revision_id,
                    },
                ],
                idempotency_key: Some("packing:mismatched-apply".to_owned()),
            },
            actor.clone(),
            vec![namespace.clone()],
        )
        .await
        .expect_err("reject packing across semantic roles");
    assert!(
        format!("{mismatch_error:#}").contains("share Scope and kind"),
        "unexpected packing error: {mismatch_error:#}"
    );
    let request = PackPagesRequest {
        pages: pages
            .iter()
            .map(|page| PageRevisionRef {
                page_id: page.page_id.clone(),
                revision_id: page.revision_id.clone(),
            })
            .collect(),
        idempotency_key: Some("packing:apply".to_owned()),
    };
    let packed = store
        .pack_pages(request.clone(), actor.clone(), vec![namespace.clone()])
        .await
        .expect("pack Pages");
    assert!(!pages.iter().any(|page| page.page_id == packed.page_id));
    assert_eq!(
        store
            .page_count(vec![namespace.clone()])
            .await
            .expect("count effective Pages"),
        2
    );
    let read = store
        .read_pages(
            ReadPagesRequest {
                page_ids: Vec::new(),
                revision_ids: vec![packed.revision_id.clone()],
                projections: vec![Projection::Payload],
                max_chars: 8_000,
            },
            vec![namespace.clone()],
        )
        .await
        .expect("read packed Page")
        .remove(0);
    assert_eq!(read.page.mutability, PageMutability::Revisioned);
    let payload = read.revision.payload.expect("packed payload");
    assert_eq!(payload.media_type, pcp_core::PACKED_PAGE_MEDIA_TYPE);
    let packed_json: serde_json::Value =
        serde_json::from_str(&payload.content).expect("decode packed payload");
    assert_eq!(
        packed_json
            .as_object()
            .map(|object| object.keys().cloned().collect::<Vec<_>>()),
        Some(vec!["entries".to_owned()])
    );
    assert_eq!(packed_json["entries"].as_array().map(Vec::len), Some(3));
    assert_eq!(
        packed_json["entries"][0]["provenance"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        read.revision.source_span.as_ref().map(|span| span.start),
        Some(0)
    );
    assert_eq!(
        read.revision.source_span.as_ref().map(|span| span.end),
        Some(2)
    );
    let old_read_error = store
        .read_pages(
            ReadPagesRequest {
                page_ids: Vec::new(),
                revision_ids: vec![pages[0].revision_id.clone()],
                projections: vec![Projection::Payload],
                max_chars: 8_000,
            },
            vec![namespace.clone()],
        )
        .await
        .expect_err("old packed Revision must not remain readable");
    assert!(format!("{old_read_error:#}").contains("was packed into Page"));
    let old_page_error = store
        .current_revision_id(pages[0].page_id.clone(), vec![namespace.clone()])
        .await
        .expect_err("old packed Page must not look like an unknown ID");
    assert!(format!("{old_page_error:#}").contains("was packed into Page"));

    let replay = store
        .pack_pages(request.clone(), actor, vec![namespace.clone()])
        .await
        .expect("replay idempotent packing");
    assert_eq!(replay.revision_id, packed.revision_id);
    assert!(!replay.created);

    drop(store);
    let reopened = SqlitePcpStore::open(path)
        .await
        .expect("reopen packed store");
    assert_eq!(
        reopened
            .page_count(vec![namespace])
            .await
            .expect("count effective Pages after reopen"),
        2
    );

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn unpack_restores_lossless_leaves_and_retracts_the_ambiguous_pack_relations() {
    let root = std::env::temp_dir().join(format!(
        "pcp-unpacking-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let store = SqlitePcpStore::open(root.join("pcp.sqlite3"))
        .await
        .expect("open store");
    let identity_id = store.identity_id().to_owned();
    let namespace = "conversation:repair".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Repair".to_owned(),
            description: None,
            parent_namespace: None,
        })
        .await
        .expect("create scope");
    let actor = Actor {
        actor_type: ActorType::Tool,
        actor_id: "tool:test".to_owned(),
    };
    let mut leaves = Vec::new();
    for index in 0..3 {
        let mut request = write_request(
            &identity_id,
            &namespace,
            actor.clone(),
            &format!("topic {index}"),
            &format!("unpack:{index}"),
        );
        request.kind = "conversation_event".to_owned();
        request.mutability = PageMutability::Sealed;
        request.source_span = Some(SourceSpan {
            stream_id: "conversation:repair".to_owned(),
            start: index,
            end: index,
        });
        leaves.push(
            store
                .write_page(request, vec![namespace.clone()])
                .await
                .expect("write source leaf"),
        );
    }
    let packed = store
        .pack_pages(
            PackPagesRequest {
                pages: leaves
                    .iter()
                    .map(|leaf| PageRevisionRef {
                        page_id: leaf.page_id.clone(),
                        revision_id: leaf.revision_id.clone(),
                    })
                    .collect(),
                idempotency_key: None,
            },
            actor.clone(),
            vec![namespace.clone()],
        )
        .await
        .expect("pack source leaves");
    let mut external_request = write_request(
        &identity_id,
        &namespace,
        actor.clone(),
        "external relation target",
        "unpack:external",
    );
    external_request.kind = "conversation_event".to_owned();
    external_request.mutability = PageMutability::Sealed;
    external_request.source_span = Some(SourceSpan {
        stream_id: "conversation:repair".to_owned(),
        start: 3,
        end: 3,
    });
    let external = store
        .write_page(external_request, vec![namespace.clone()])
        .await
        .expect("write external Page");
    let relation = store
        .link_pages(
            LinkPagesRequest {
                from_page_id: packed.page_id.clone(),
                relation_type: "related_to".to_owned(),
                to_page_id: external.page_id,
                basis_revision_ids: vec![packed.revision_id.clone()],
                created_by: actor.clone(),
                idempotency_key: None,
            },
            vec![namespace.clone()],
        )
        .await
        .expect("link packed Page");
    let unpacked = store
        .unpack_page(
            UnpackPageRequest {
                page_id: packed.page_id.clone(),
                expected_revision_id: packed.revision_id,
                idempotency_key: None,
            },
            actor,
            vec![namespace.clone()],
        )
        .await
        .expect("unpack Page");
    assert_eq!(unpacked.restored_pages.len(), leaves.len());
    assert!(
        unpacked
            .restored_pages
            .iter()
            .zip(&leaves)
            .all(|(restored, leaf)| restored.page_id == leaf.page_id
                && restored.revision_id == leaf.revision_id)
    );
    assert_eq!(unpacked.retracted_relation_ids, vec![relation.relation_id]);
    let restored = store
        .read_pages(
            ReadPagesRequest {
                page_ids: leaves.iter().map(|leaf| leaf.page_id.clone()).collect(),
                revision_ids: Vec::new(),
                projections: vec![Projection::Payload],
                max_chars: 1_024,
            },
            vec![namespace.clone()],
        )
        .await
        .expect("read restored leaves");
    assert_eq!(restored.len(), leaves.len());
    let retired = store
        .read_pages(
            ReadPagesRequest {
                page_ids: vec![packed.page_id],
                revision_ids: Vec::new(),
                projections: vec![Projection::Facets],
                max_chars: 1_024,
            },
            vec![namespace.clone()],
        )
        .await
        .expect("read retired pack")
        .remove(0);
    assert_eq!(retired.page.lifecycle_status, LifecycleStatus::Tombstoned);
    assert_eq!(
        store
            .page_count(vec![namespace])
            .await
            .expect("count active leaves after unpack"),
        4
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn extends_one_packed_page_as_a_flat_revision_without_changing_its_identity() {
    let root = std::env::temp_dir().join(format!(
        "pcp-packing-extension-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let store = SqlitePcpStore::open(root.join("pcp.sqlite3"))
        .await
        .expect("open store");
    let identity_id = store.identity_id().to_owned();
    let namespace = "project:packing-extension".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Packing extension".to_owned(),
            description: None,
            parent_namespace: None,
        })
        .await
        .expect("create scope");
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "host:packing-extension".to_owned(),
    };
    let mut leaves = Vec::new();
    for index in 0_u64..6 {
        let mut request = write_request(
            &identity_id,
            &namespace,
            actor.clone(),
            &format!("Continuous discussion event {index}."),
            &format!("packing-extension:{index}"),
        );
        request.kind = "conversation_event".to_owned();
        request.mutability = PageMutability::Sealed;
        request.source_span = Some(SourceSpan {
            stream_id: "conversation:main".to_owned(),
            start: index,
            end: index,
        });
        leaves.push(
            store
                .write_page(request, vec![namespace.clone()])
                .await
                .expect("write packing extension leaf"),
        );
    }
    let first_request = PackPagesRequest {
        pages: leaves[2..4]
            .iter()
            .map(|page| PageRevisionRef {
                page_id: page.page_id.clone(),
                revision_id: page.revision_id.clone(),
            })
            .collect(),
        idempotency_key: Some("packing-extension:first".to_owned()),
    };
    let packed = store
        .pack_pages(
            first_request.clone(),
            actor.clone(),
            vec![namespace.clone()],
        )
        .await
        .expect("create initial packed Page");

    let mut reference_request = write_request(
        &identity_id,
        &namespace,
        actor.clone(),
        "A stable topic referenced by the packed conversation.",
        "packing-extension:reference",
    );
    reference_request.kind = "topic".to_owned();
    let reference = store
        .write_page(reference_request, vec![namespace.clone()])
        .await
        .expect("write related topic");
    store
        .link_pages(
            LinkPagesRequest {
                from_page_id: packed.page_id.clone(),
                relation_type: "related_to".to_owned(),
                to_page_id: reference.page_id,
                basis_revision_ids: vec![packed.revision_id.clone(), reference.revision_id],
                created_by: actor.clone(),
                idempotency_key: Some("packing-extension:relation".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("relate packed Page before extension");
    store
        .write_summary(
            WriteSummaryRequest {
                target_page_id: packed.page_id.clone(),
                target_revision_id: packed.revision_id.clone(),
                expected_summary_revision_id: None,
                content: "A continuous discussion about one durable runtime topic.".to_owned(),
                created_by: actor.clone(),
                tool_or_model: Some("model:packing-test".to_owned()),
                provenance: Vec::new(),
                idempotency_key: Some("packing-extension:summary".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("summarize packed Page before extension");

    let extended = store
        .pack_pages(
            PackPagesRequest {
                pages: leaves[..2]
                    .iter()
                    .map(|page| PageRevisionRef {
                        page_id: page.page_id.clone(),
                        revision_id: page.revision_id.clone(),
                    })
                    .chain(std::iter::once(PageRevisionRef {
                        page_id: packed.page_id.clone(),
                        revision_id: packed.revision_id.clone(),
                    }))
                    .collect(),
                idempotency_key: Some("packing-extension:second".to_owned()),
            },
            actor.clone(),
            vec![namespace.clone()],
        )
        .await
        .expect("prepend to packed Page");
    assert_eq!(extended.page_id, packed.page_id);
    assert_ne!(extended.revision_id, packed.revision_id);

    let packed_page_id = extended.page_id.clone();
    let current_packed_revision_id = extended.revision_id.clone();
    let member_positions = store
        .run("read extended packing membership", move |connection| {
            let mut statement = connection.prepare(
                "
                SELECT position, packed_revision_id
                FROM pcp_page_packs
                WHERE packed_page_id = ?1
                ORDER BY position
                ",
            )?;
            let rows = statement
                .query_map([packed_page_id], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
        .expect("read extended packing membership");
    assert_eq!(
        member_positions,
        (0_i64..4)
            .map(|position| (position, current_packed_revision_id.clone()))
            .collect::<Vec<_>>()
    );

    let current = store
        .read_pages(
            ReadPagesRequest {
                page_ids: vec![packed.page_id.clone()],
                revision_ids: Vec::new(),
                projections: vec![
                    Projection::Payload,
                    Projection::Relations,
                    Projection::Summary,
                    Projection::History,
                ],
                max_chars: 32_000,
            },
            vec![namespace.clone()],
        )
        .await
        .expect("read extended packed Page")
        .remove(0);
    assert_eq!(current.page.mutability, PageMutability::Revisioned);
    assert_eq!(
        current.revision.previous_revision_id.as_deref(),
        Some(packed.revision_id.as_str())
    );
    assert!(
        current
            .relations
            .iter()
            .any(|relation| relation.relation_type == "related_to")
    );
    assert_eq!(
        current
            .summary
            .as_ref()
            .map(|summary| summary.target_revision_id.as_str()),
        Some(packed.revision_id.as_str())
    );
    assert!(current.history.contains(&packed.revision_id));
    assert!(current.history.contains(&extended.revision_id));
    let current_payload = current.revision.payload.expect("extended packed payload");
    let current_json: serde_json::Value =
        serde_json::from_str(&current_payload.content).expect("decode extended packed payload");
    let entries = current_json["entries"]
        .as_array()
        .expect("packed entries array");
    assert_eq!(entries.len(), 4);
    assert!(entries.iter().all(|entry| {
        entry["payload"]["mediaType"].as_str() != Some(pcp_core::PACKED_PAGE_MEDIA_TYPE)
    }));
    let inventory_anchor = store
        .durable_page_inventory(vec![namespace.clone()], Vec::new())
        .await
        .expect("read packing inventory")
        .into_iter()
        .find(|page| page.page_id == packed.page_id)
        .expect("packed Page in inventory");
    assert!(inventory_anchor.snippet.contains("Packed range boundary"));
    assert!(
        inventory_anchor
            .snippet
            .contains("Continuous discussion event 0")
    );
    assert!(
        inventory_anchor
            .snippet
            .contains("Continuous discussion event 3")
    );
    let generic_revision_error = store
        .revise_page(
            RevisePageRequest {
                page_id: packed.page_id.clone(),
                expected_revision_id: extended.revision_id.clone(),
                created_by: actor.clone(),
                lifecycle_status: LifecycleStatus::Active,
                observed_at: None,
                valid_from: None,
                valid_to: None,
                payload: Some(PagePayload {
                    media_type: "text/markdown".to_owned(),
                    content: "This must not replace the packed container.".to_owned(),
                }),
                source_refs: Vec::new(),
                facets: None,
                provenance: Vec::new(),
                initial_relations: Vec::new(),
                idempotency_key: Some("packing-extension:generic-revision".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect_err("reject generic packed Page revision");
    assert!(format!("{generic_revision_error:#}").contains("only be revised by pack_pages"));

    let old = store
        .read_pages(
            ReadPagesRequest {
                page_ids: Vec::new(),
                revision_ids: vec![packed.revision_id.clone()],
                projections: vec![Projection::Payload],
                max_chars: 16_000,
            },
            vec![namespace.clone()],
        )
        .await
        .expect("read previous packed Revision")
        .remove(0);
    let old_payload: serde_json::Value = serde_json::from_str(
        &old.revision
            .payload
            .expect("previous packed payload")
            .content,
    )
    .expect("decode previous packed payload");
    assert_eq!(old_payload["entries"].as_array().map(Vec::len), Some(2));

    let initial_replay = store
        .pack_pages(first_request, actor.clone(), vec![namespace.clone()])
        .await
        .expect("replay initial packing after extension");
    assert_eq!(initial_replay.page_id, packed.page_id);
    assert_eq!(initial_replay.revision_id, packed.revision_id);
    assert!(!initial_replay.created);

    let second_packed = store
        .pack_pages(
            PackPagesRequest {
                pages: leaves[4..]
                    .iter()
                    .map(|page| PageRevisionRef {
                        page_id: page.page_id.clone(),
                        revision_id: page.revision_id.clone(),
                    })
                    .collect(),
                idempotency_key: Some("packing-extension:third".to_owned()),
            },
            actor.clone(),
            vec![namespace.clone()],
        )
        .await
        .expect("create second packed Page");
    let merged = store
        .pack_pages(
            PackPagesRequest {
                pages: vec![
                    PageRevisionRef {
                        page_id: extended.page_id.clone(),
                        revision_id: extended.revision_id.clone(),
                    },
                    PageRevisionRef {
                        page_id: second_packed.page_id.clone(),
                        revision_id: second_packed.revision_id.clone(),
                    },
                ],
                idempotency_key: Some("packing-extension:two-anchors".to_owned()),
            },
            actor,
            vec![namespace.clone()],
        )
        .await
        .expect("merge two contiguous packed anchors");
    assert_eq!(merged.page_id, extended.page_id);
    assert_ne!(merged.revision_id, extended.revision_id);

    let merged_page_id = merged.page_id.clone();
    let merged_revision_id = merged.revision_id.clone();
    let merged_members = store
        .run("read merged packed membership", move |connection| {
            let mut statement = connection.prepare(
                "SELECT position, packed_revision_id FROM pcp_page_packs WHERE packed_page_id = ?1 ORDER BY position",
            )?;
            statement
                .query_map([merged_page_id], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(Into::into)
        })
        .await
        .expect("read merged packed membership");
    assert_eq!(
        merged_members,
        (0_i64..6)
            .map(|position| (position, merged_revision_id.clone()))
            .collect::<Vec<_>>()
    );
    let active_pages = store
        .durable_page_inventory(vec![namespace], Vec::new())
        .await
        .expect("read current Pages after pack merge");
    assert!(
        active_pages
            .iter()
            .any(|page| page.page_id == merged.page_id)
    );
    assert!(
        !active_pages
            .iter()
            .any(|page| page.page_id == second_packed.page_id)
    );

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn packing_rejects_source_gaps_and_folds_internal_relations() {
    let root = std::env::temp_dir().join(format!(
        "pcp-packing-guards-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let store = SqlitePcpStore::open(root.join("pcp.sqlite3"))
        .await
        .expect("open store");
    let identity_id = store.identity_id().to_owned();
    let namespace = "project:packing-guards".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Packing guards".to_owned(),
            description: None,
            parent_namespace: None,
        })
        .await
        .expect("create scope");
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "host:packing-guards".to_owned(),
    };
    let mut pages = Vec::new();
    for (position, span) in [0_u64, 2, 1].into_iter().enumerate() {
        let mut request = write_request(
            &identity_id,
            &namespace,
            actor.clone(),
            &format!("Source event {position}"),
            &format!("packing-guards:{position}"),
        );
        request.kind = "conversation_event".to_owned();
        request.mutability = PageMutability::Sealed;
        request.source_span = Some(SourceSpan {
            stream_id: "conversation:main".to_owned(),
            start: span,
            end: span,
        });
        pages.push(
            store
                .write_page(request, vec![namespace.clone()])
                .await
                .expect("write packing guard Page"),
        );
    }

    let gap_error = store
        .pack_pages(
            PackPagesRequest {
                pages: pages[..2]
                    .iter()
                    .map(|page| PageRevisionRef {
                        page_id: page.page_id.clone(),
                        revision_id: page.revision_id.clone(),
                    })
                    .collect(),
                idempotency_key: Some("packing-guards:gap".to_owned()),
            },
            actor.clone(),
            vec![namespace.clone()],
        )
        .await
        .expect_err("reject a source-span gap");
    assert!(format!("{gap_error:#}").contains("contiguous and ordered"));

    store
        .link_pages(
            LinkPagesRequest {
                from_page_id: pages[0].page_id.clone(),
                relation_type: "related_to".to_owned(),
                to_page_id: pages[2].page_id.clone(),
                basis_revision_ids: vec![
                    pages[0].revision_id.clone(),
                    pages[2].revision_id.clone(),
                ],
                created_by: actor.clone(),
                idempotency_key: Some("packing-guards:relation".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("link packing guard Pages");
    store
        .link_pages(
            LinkPagesRequest {
                from_page_id: pages[2].page_id.clone(),
                relation_type: "depends_on".to_owned(),
                to_page_id: pages[1].page_id.clone(),
                basis_revision_ids: vec![
                    pages[2].revision_id.clone(),
                    pages[1].revision_id.clone(),
                ],
                created_by: actor.clone(),
                idempotency_key: Some("packing-guards:external-relation".to_owned()),
            },
            vec![namespace.clone()],
        )
        .await
        .expect("link packing input to external Page");
    let mut derived_request = write_request(
        &identity_id,
        &namespace,
        actor.clone(),
        "A later result derived from one event in the packed range.",
        "packing-guards:derived",
    );
    derived_request.kind = "conversation_event".to_owned();
    derived_request.mutability = PageMutability::Sealed;
    derived_request.source_span = Some(SourceSpan {
        stream_id: "conversation:other".to_owned(),
        start: 0,
        end: 0,
    });
    derived_request.provenance = vec![ProvenanceEvent {
        operation: "derive".to_owned(),
        actor: actor.clone(),
        timestamp: "2026-08-16T00:00:00Z".to_owned(),
        input_revision_ids: vec![pages[2].revision_id.clone()],
        tool_or_model: Some("test-model".to_owned()),
    }];
    let derived = store
        .write_page(derived_request, vec![namespace.clone()])
        .await
        .expect("write externally derived Page");
    let inventory = store
        .durable_page_inventory(vec![namespace.clone()], Vec::new())
        .await
        .expect("read packing protection inventory");
    for transferable in [&pages[0], &pages[2]] {
        assert!(
            !inventory
                .iter()
                .find(|item| item.page_id == transferable.page_id)
                .expect("referenced Page in inventory")
                .packing_protected
        );
    }
    assert!(
        !inventory
            .iter()
            .find(|item| item.page_id == pages[1].page_id)
            .expect("unreferenced Page in inventory")
            .packing_protected
    );
    let packed = store
        .pack_pages(
            PackPagesRequest {
                pages: [pages[0].clone(), pages[2].clone()]
                    .into_iter()
                    .map(|page| PageRevisionRef {
                        page_id: page.page_id,
                        revision_id: page.revision_id,
                    })
                    .collect(),
                idempotency_key: Some("packing-guards:referenced".to_owned()),
            },
            actor,
            vec![namespace.clone()],
        )
        .await
        .expect("pack relation-connected inputs");
    let packed_page_id = packed.page_id.clone();
    let packed_revision_id = packed.revision_id.clone();
    let external_page_id = pages[1].page_id.clone();
    let external_revision_id = pages[1].revision_id.clone();
    let derived_revision_id = derived.revision_id.clone();
    let (relations, provenance) = store
        .run("read rewritten packing references", move |connection| {
            let relations = {
                let mut statement = connection
                    .prepare(
                        "SELECT from_page_id, to_page_id, basis_revision_ids_json
                         FROM pcp_relations
                         WHERE from_page_id = ?1 OR to_page_id = ?1",
                    )
                    .context("prepare packed relations")?;
                statement
                    .query_map([&packed_page_id], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    })
                    .context("query packed relations")?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .context("collect packed relations")?
            };
            let provenance = connection
                .query_row(
                    "SELECT input_revision_id FROM pcp_provenance_inputs
                     WHERE derived_revision_id = ?1",
                    [&derived_revision_id],
                    |row| row.get::<_, String>(0),
                )
                .context("read rewritten packed provenance")?;
            Ok((relations, provenance))
        })
        .await
        .expect("read rewritten packing references");
    assert_eq!(relations.len(), 1);
    assert!(
        (relations[0].0 == packed.page_id && relations[0].1 == external_page_id)
            || (relations[0].1 == packed.page_id && relations[0].0 == external_page_id)
    );
    let basis = serde_json::from_str::<Vec<String>>(&relations[0].2)
        .expect("decode rewritten relation basis");
    assert!(basis.contains(&packed_revision_id));
    assert!(basis.contains(&external_revision_id));
    assert_eq!(provenance, packed.revision_id);

    let _ = std::fs::remove_dir_all(root);
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
    let identity_id = store.identity_id().to_owned();
    let namespace = "project:inventory".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Inventory project".to_owned(),
            description: None,
            parent_namespace: None,
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
                &identity_id,
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
                target_page_id: durable.page_id.clone(),
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
        &identity_id,
        &namespace,
        actor,
        "A raw chat message must not become a reconciliation candidate.",
        "inventory:conversation",
    );
    conversation.kind = "conversation_event".to_owned();
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

#[tokio::test]
async fn durable_inventory_does_not_drop_pages_after_the_first_hundred() {
    let root = std::env::temp_dir().join(format!(
        "pcp-inventory-complete-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let store = SqlitePcpStore::open(root.join("pcp.sqlite3"))
        .await
        .expect("open store");
    let identity_id = store.identity_id().to_owned();
    let namespace = "project:inventory-complete".to_owned();
    store
        .create_scope(CreateScopeRequest {
            namespace: namespace.clone(),
            display_name: "Complete inventory project".to_owned(),
            description: None,
            parent_namespace: None,
        })
        .await
        .expect("create scope");
    let actor = Actor {
        actor_type: ActorType::Model,
        actor_id: "model:inventory-complete".to_owned(),
    };
    for index in 0..105 {
        store
            .write_page(
                write_request(
                    &identity_id,
                    &namespace,
                    actor.clone(),
                    &format!("Durable inventory Page {index}."),
                    &format!("inventory-complete:{index}"),
                ),
                vec![namespace.clone()],
            )
            .await
            .expect("write inventory Page");
    }

    let inventory = store
        .durable_page_inventory(vec![namespace], Vec::new())
        .await
        .expect("read complete durable inventory");
    assert_eq!(inventory.len(), 105);

    let _ = std::fs::remove_dir_all(root);
}
