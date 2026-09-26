//! Operator-only reclassification of a sealed Page or revisioned Topic into an existing Scope.
//! The old Revision keeps its original namespace and remains addressable.

use anyhow::{Context, Result};
use pcp_core::{Actor, PagePayload, ProvenanceEvent, SourceRef, SourceSpan, WriteResult};
use rusqlite::{OptionalExtension, params};

use crate::{
    store::SqlitePcpStore,
    write::{
        ensure_scope_access, insert_relation, insert_revision, lookup_write_idempotency, now,
        random_id, record_idempotency, scope_set,
    },
};

struct SourceHead {
    namespace: String,
    mutability: String,
    kind: String,
    lifecycle_status: String,
    observed_at: Option<String>,
    source_span_json: Option<String>,
    valid_from: Option<String>,
    valid_to: Option<String>,
    payload_media_type: Option<String>,
    payload_content: Option<String>,
    source_refs_json: String,
    facets_json: Option<String>,
}

impl SqlitePcpStore {
    /// Reclassify one exact current sealed or Topic head, preserving derivation metadata. Only local operator tooling calls this
    /// method; tenant RPC intentionally has no Scope-transfer operation.
    pub async fn transfer_page_scope(
        &self,
        page_id: String,
        expected_revision_id: String,
        expected_source_scope: String,
        target_scope: String,
        actor: Actor,
        idempotency_key: String,
        allowed_scopes: Vec<String>,
    ) -> Result<WriteResult> {
        anyhow::ensure!(
            expected_source_scope != target_scope,
            "source and destination Scopes are identical"
        );
        anyhow::ensure!(
            !idempotency_key.trim().is_empty(),
            "Scope transfer requires an idempotency key"
        );
        let allowed_scopes = scope_set(allowed_scopes);
        self.run("page Scope transfer", move |mut connection| {
            let transaction = connection.transaction().context("start PCP Scope transfer")?;
            ensure_scope_access(&transaction, &expected_source_scope, &allowed_scopes)?;
            ensure_scope_access(&transaction, &target_scope, &allowed_scopes)?;
            if let Some(existing) = lookup_write_idempotency(
                &transaction,
                &actor.actor_id,
                "transfer_page_scope",
                Some(&idempotency_key),
            )? {
                return Ok(existing);
            }
            let head = transaction
                .query_row(
                    "SELECT p.namespace, p.mutability, p.lifecycle_status,
                            r.observed_at, r.source_span_json, r.valid_from, r.valid_to,
                            r.payload_media_type, r.payload_content, r.source_refs_json,
                            r.facets_json, p.kind
                     FROM pcp_pages p JOIN pcp_revisions r ON r.revision_id = p.current_revision_id
                     WHERE p.page_id = ?1 AND p.current_revision_id = ?2",
                    params![page_id, expected_revision_id],
                    |row| {
                        Ok(SourceHead {
                            namespace: row.get(0)?,
                            mutability: row.get(1)?,
                            lifecycle_status: row.get(2)?,
                            observed_at: row.get(3)?,
                            source_span_json: row.get(4)?,
                            valid_from: row.get(5)?,
                            valid_to: row.get(6)?,
                            payload_media_type: row.get(7)?,
                            payload_content: row.get(8)?,
                            source_refs_json: row.get(9)?,
                            facets_json: row.get(10)?,
                            kind: row.get(11)?,
                        })
                    },
                )
                .optional()
                .context("read exact PCP source head")?
                .context("Page head changed or is unavailable")?;
            anyhow::ensure!(head.namespace == expected_source_scope, "Page source Scope changed");
            let topic = head.kind == "topic_summary" && head.mutability == "revisioned";
            anyhow::ensure!((head.mutability == "sealed" || topic) && head.lifecycle_status == "active", "only active sealed Pages or revisioned Topics can be transferred");
            let assessed: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM pcp_validity_heads WHERE target_page_id = ?1)",
                [&page_id],
                |row| row.get(0),
            )?;
            anyhow::ensure!(!assessed, "Page has validity history; review before Scope transfer");

            let payload = match (head.payload_media_type, head.payload_content) {
                (Some(media_type), Some(content)) => Some(PagePayload { media_type, content }),
                (None, None) => None,
                _ => anyhow::bail!("source payload is incomplete"),
            };
            let source_refs: Vec<SourceRef> = serde_json::from_str(&head.source_refs_json)
                .context("decode original source references")?;
            let source_span: Option<SourceSpan> = head.source_span_json
                .map(|value| serde_json::from_str(&value))
                .transpose()
                .context("decode original source span")?;
            let facets = head.facets_json
                .map(|value| serde_json::from_str(&value))
                .transpose()
                .context("decode original Page facets")?;
            let timestamp = now();
            let revision_id = random_id(&transaction, "rev_")?;
            let provenance = vec![ProvenanceEvent {
                operation: "transfer_scope".to_owned(),
                actor: actor.clone(),
                timestamp: timestamp.clone(),
                input_revision_ids: vec![expected_revision_id.clone()],
                tool_or_model: None,
                reason: Some(format!("Reclassify from {expected_source_scope} to {target_scope}; content and sources unchanged")),
            }];
            insert_revision(
                &transaction,
                &page_id,
                &revision_id,
                &target_scope,
                &head.lifecycle_status,
                &timestamp,
                head.observed_at.as_deref(),
                source_span.as_ref(),
                head.valid_from.as_deref(),
                head.valid_to.as_deref(),
                &actor,
                payload.as_ref(),
                &source_refs,
                facets.as_ref(),
                &provenance,
            )?;
            let changed = transaction.execute(
                "UPDATE pcp_pages SET namespace = ?1, current_revision_id = ?2, updated_at = ?3
                 WHERE page_id = ?4 AND current_revision_id = ?5 AND namespace = ?6",
                params![target_scope, revision_id, timestamp, page_id, expected_revision_id, expected_source_scope],
            )?;
            anyhow::ensure!(changed == 1, "Page head changed during Scope transfer");
            if topic {
                // Scope changes must keep the current Topic's routing and exact
                // historical source membership, not turn it into an orphan Page.
                let copied = transaction.execute(
                    "INSERT INTO pcp_topic_extractions (topic_revision_id, topic_page_id, namespace, title, created_at)
                     SELECT ?1, topic_page_id, ?2, title, ?3 FROM pcp_topic_extractions WHERE topic_revision_id = ?4 AND topic_page_id = ?5",
                    params![revision_id, target_scope, timestamp, expected_revision_id, page_id],
                )?;
                anyhow::ensure!(copied == 1, "Topic extraction metadata is unavailable");
                transaction.execute(
                    "INSERT INTO pcp_topic_extraction_members (topic_revision_id, topic_page_id, source_revision_id, source_page_id, position)
                     SELECT ?1, topic_page_id, source_revision_id, source_page_id, position
                     FROM pcp_topic_extraction_members WHERE topic_revision_id = ?2",
                    params![revision_id, expected_revision_id],
                )?;
                let sources = {
                    let mut statement = transaction.prepare(
                        "SELECT member.source_revision_id, revision.namespace
                         FROM pcp_topic_extraction_members member
                         JOIN pcp_revisions revision ON revision.revision_id = member.source_revision_id
                         WHERE member.topic_revision_id = ?1 ORDER BY member.position"
                    )?;
                    statement.query_map([&revision_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                };
                anyhow::ensure!(sources.len() >= 2, "Topic source membership is incomplete");
                for (_, namespace) in &sources {
                    ensure_scope_access(&transaction, namespace, &allowed_scopes)?;
                }
                transaction.execute(
                    "INSERT INTO pcp_relation_retractions (relation_id, retracted_actor_type, retracted_actor_id, retracted_at, reason)
                     SELECT relation_id, ?2, ?3, ?4, 'Topic Scope transferred; exact source membership preserved'
                     FROM pcp_relations relation WHERE from_page_id = ?1 AND relation_type = 'summarizes'
                     AND NOT EXISTS (SELECT 1 FROM pcp_relation_retractions retraction WHERE retraction.relation_id = relation.relation_id)",
                    params![page_id, actor.actor_type.as_str(), actor.actor_id, timestamp],
                )?;
                for (source_revision, _) in sources {
                    insert_relation(&transaction, &revision_id, "summarizes", &source_revision, &actor, &timestamp)?;
                }
            }
            record_idempotency(
                &transaction,
                &actor.actor_id,
                "transfer_page_scope",
                Some(&idempotency_key),
                Some(&page_id),
                Some(&revision_id),
                None,
                &timestamp,
            )?;
            transaction.commit().context("commit PCP Scope transfer")?;
            Ok(WriteResult { page_id, revision_id, created: true })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use pcp_core::{
        ActorType, CreateScopeRequest, LifecycleStatus, PageMutability, PagePayload, Projection,
        ReadPagesRequest, SourceRef, WritePageRequest,
    };
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn transfers_exact_sealed_head_without_changing_original_revision() {
        let root = std::env::temp_dir().join(format!(
            "pcp-scope-transfer-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = SqlitePcpStore::open(root.join("pcp.sqlite3"))
            .await
            .unwrap();
        let source = "user:transfer-test".to_owned();
        let target = "project:transfer-test".to_owned();
        for namespace in [&source, &target] {
            store
                .create_scope(CreateScopeRequest {
                    namespace: namespace.clone(),
                    display_name: namespace.clone(),
                    description: None,
                    parent_namespace: None,
                })
                .await
                .unwrap();
        }
        let actor = Actor {
            actor_type: ActorType::Tool,
            actor_id: "cli:scope-transfer-test".into(),
        };
        let original = store
            .write_page(
                WritePageRequest {
                    namespace: source.clone(),
                    lifecycle_status: LifecycleStatus::Active,
                    kind: "codex_capture".into(),
                    mutability: PageMutability::Sealed,
                    created_by: actor.clone(),
                    observed_at: Some("2026-09-24".into()),
                    source_span: None,
                    valid_from: None,
                    valid_to: None,
                    payload: Some(PagePayload {
                        media_type: "text/markdown".into(),
                        content: "# Project rule\n\nKeep this.".into(),
                    }),
                    source_refs: vec![SourceRef {
                        provider_id: "test".into(),
                        locator: "task:1".into(),
                        media_type: None,
                        content_digest: None,
                    }],
                    facets: Some(json!({"captureCategory":"durable_decision"})),
                    provenance: Vec::new(),
                    initial_relations: Vec::new(),
                    idempotency_key: Some("source:1".into()),
                },
                vec![source.clone()],
            )
            .await
            .unwrap();
        let scopes = vec![source.clone(), target.clone()];
        assert!(
            store
                .transfer_page_scope(
                    original.page_id.clone(),
                    "rev_wrong".into(),
                    source.clone(),
                    target.clone(),
                    actor.clone(),
                    "migration:wrong".into(),
                    scopes.clone(),
                )
                .await
                .is_err()
        );
        let result = store
            .transfer_page_scope(
                original.page_id.clone(),
                original.revision_id.clone(),
                source.clone(),
                target.clone(),
                actor.clone(),
                "migration:1".into(),
                scopes.clone(),
            )
            .await
            .unwrap();
        let repeated = store
            .transfer_page_scope(
                original.page_id.clone(),
                original.revision_id.clone(),
                source.clone(),
                target.clone(),
                actor,
                "migration:1".into(),
                scopes.clone(),
            )
            .await
            .unwrap();
        assert_eq!(result.revision_id, repeated.revision_id);
        let current = store
            .read_pages(
                ReadPagesRequest {
                    page_ids: vec![original.page_id.clone()],
                    revision_ids: Vec::new(),
                    projections: vec![
                        Projection::Manifest,
                        Projection::Payload,
                        Projection::Sources,
                        Projection::Facets,
                        Projection::Provenance,
                    ],
                    max_chars: 64_000,
                },
                scopes.clone(),
            )
            .await
            .unwrap()
            .remove(0);
        assert_eq!(current.page.namespace, target);
        assert_eq!(
            current.revision.previous_revision_id,
            Some(original.revision_id.clone())
        );
        assert_eq!(current.revision.observed_at.as_deref(), Some("2026-09-24"));
        assert_eq!(
            current.revision.payload.as_ref().unwrap().content,
            "# Project rule\n\nKeep this."
        );
        assert_eq!(current.revision.source_refs[0].locator, "task:1");
        assert_eq!(
            current.revision.facets.unwrap()["captureCategory"],
            "durable_decision"
        );
        assert_eq!(
            current.revision.provenance[0].input_revision_ids,
            vec![original.revision_id.clone()]
        );
        let historical = store
            .read_pages(
                ReadPagesRequest {
                    page_ids: Vec::new(),
                    revision_ids: vec![original.revision_id.clone()],
                    projections: vec![Projection::Manifest, Projection::Payload],
                    max_chars: 64_000,
                },
                vec![source],
            )
            .await
            .unwrap()
            .remove(0);
        assert_eq!(historical.revision.revision_id, original.revision_id);
        assert_eq!(
            historical.revision.payload.unwrap().content,
            "# Project rule\n\nKeep this."
        );
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
    #[tokio::test]
    async fn transfers_topic_with_exact_members_and_allows_later_refresh() {
        use pcp_core::{ExtractTopicRequest, PageRevisionRef};
        let root = std::env::temp_dir().join(format!(
            "pcp-topic-transfer-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = SqlitePcpStore::open(root.join("pcp.sqlite3"))
            .await
            .unwrap();
        let from = "user:topic-test".to_owned();
        let to = "project:topic-test".to_owned();
        let scopes = vec![from.clone(), to.clone()];
        for namespace in &scopes {
            store
                .create_scope(CreateScopeRequest {
                    namespace: namespace.clone(),
                    display_name: namespace.clone(),
                    description: None,
                    parent_namespace: None,
                })
                .await
                .unwrap();
        }
        let actor = Actor {
            actor_type: ActorType::Tool,
            actor_id: "operator:topic-transfer".into(),
        };
        let mut sources = Vec::new();
        for i in 0..2 {
            let page = store
                .write_page(
                    WritePageRequest {
                        namespace: to.clone(),
                        lifecycle_status: LifecycleStatus::Active,
                        kind: "codex_capture".into(),
                        mutability: PageMutability::Sealed,
                        created_by: actor.clone(),
                        observed_at: None,
                        source_span: None,
                        valid_from: None,
                        valid_to: None,
                        payload: Some(PagePayload {
                            media_type: "text/plain".into(),
                            content: format!("Project source {i}"),
                        }),
                        source_refs: vec![],
                        initial_relations: vec![],
                        facets: None,
                        provenance: vec![],
                        idempotency_key: None,
                    },
                    scopes.clone(),
                )
                .await
                .unwrap();
            sources.push(PageRevisionRef {
                page_id: page.page_id,
                revision_id: page.revision_id,
            });
        }
        let original = store
            .extract_topic(
                ExtractTopicRequest {
                    target_namespace: Some(from.clone()),
                    target_topic: None,
                    source_pages: sources.clone(),
                    title: "Project topic".into(),
                    content: "Project source zero and one.".into(),
                    created_by: actor.clone(),
                    tool_or_model: None,
                    provenance: vec![],
                    idempotency_key: None,
                },
                scopes.clone(),
            )
            .await
            .unwrap();
        assert!(
            store
                .transfer_page_scope(
                    original.page_id.clone(),
                    original.revision_id.clone(),
                    from.clone(),
                    to.clone(),
                    actor.clone(),
                    "denied".into(),
                    vec![to.clone()]
                )
                .await
                .is_err()
        );
        let moved = store
            .transfer_page_scope(
                original.page_id.clone(),
                original.revision_id.clone(),
                from.clone(),
                to.clone(),
                actor.clone(),
                "topic-move".into(),
                scopes.clone(),
            )
            .await
            .unwrap();
        let repeated = store
            .transfer_page_scope(
                original.page_id.clone(),
                original.revision_id.clone(),
                from.clone(),
                to.clone(),
                actor.clone(),
                "topic-move".into(),
                scopes.clone(),
            )
            .await
            .unwrap();
        assert_eq!(repeated.revision_id, moved.revision_id);
        assert!(
            store
                .transfer_page_scope(
                    original.page_id.clone(),
                    original.revision_id.clone(),
                    from.clone(),
                    to.clone(),
                    actor.clone(),
                    "stale-move".into(),
                    scopes.clone()
                )
                .await
                .is_err()
        );
        let current = store
            .read_pages(
                ReadPagesRequest {
                    page_ids: vec![original.page_id.clone()],
                    revision_ids: vec![],
                    projections: vec![
                        Projection::Manifest,
                        Projection::Payload,
                        Projection::Relations,
                    ],
                    max_chars: 8000,
                },
                scopes.clone(),
            )
            .await
            .unwrap()
            .remove(0);
        assert_eq!(current.page.namespace, to);
        assert_eq!(
            current.revision.previous_revision_id,
            Some(original.revision_id.clone())
        );
        assert_eq!(
            current.revision.payload.unwrap().content,
            "Project source zero and one."
        );
        let relations = current
            .relations
            .iter()
            .filter(|r| r.relation_type == "summarizes")
            .collect::<Vec<_>>();
        assert_eq!(relations.len(), 2);
        assert!(
            relations
                .iter()
                .all(|r| r.basis_revision_ids.contains(&moved.revision_id))
        );
        let history = store
            .read_pages(
                ReadPagesRequest {
                    page_ids: vec![],
                    revision_ids: vec![original.revision_id.clone()],
                    projections: vec![Projection::Payload],
                    max_chars: 8000,
                },
                vec![from],
            )
            .await
            .unwrap();
        assert_eq!(
            history[0].revision.payload.as_ref().unwrap().content,
            "Project source zero and one."
        );
        let refreshed = store
            .extract_topic(
                ExtractTopicRequest {
                    target_namespace: Some(to),
                    target_topic: Some(PageRevisionRef {
                        page_id: moved.page_id.clone(),
                        revision_id: moved.revision_id,
                    }),
                    source_pages: sources,
                    title: "Project topic".into(),
                    content: "Refreshed project source zero and one.".into(),
                    created_by: actor,
                    tool_or_model: None,
                    provenance: vec![],
                    idempotency_key: None,
                },
                scopes,
            )
            .await
            .unwrap();
        assert_eq!(refreshed.page_id, moved.page_id);
        assert!(!refreshed.created);
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}
