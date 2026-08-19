use std::collections::{BTreeSet, HashMap, HashSet};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Map, Value, json};

const PACKED_PAGE_MEDIA_TYPE: &str = "application/vnd.pcp.packed-page+json";

struct RevisionRow {
    revision_id: String,
    page_id: String,
    kind: String,
    payload_media_type: Option<String>,
    payload_content: Option<String>,
    source_refs_json: String,
    facets_json: Option<String>,
    provenance_json: String,
    observed_at: Option<String>,
    created_at: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ReferenceCandidate {
    from_page_id: String,
    from_revision_id: String,
    to_page_id: String,
    to_revision_id: String,
    created_at: String,
}

#[derive(Clone)]
struct ImageTarget {
    page_id: String,
    revision_id: String,
}

pub(crate) fn migrate_draft_to_clean(
    connection: &mut Connection,
    target_version: &str,
) -> Result<()> {
    connection
        .execute_batch("PRAGMA foreign_keys = OFF;")
        .context("disable PCP foreign keys for clean Store migration")?;
    let migration = (|| -> Result<()> {
        let transaction = connection
            .transaction()
            .context("start PCP clean Store migration")?;
        rebuild_contract_tables(&transaction)?;
        let references = normalize_revision_content(&transaction)?;
        clean_legacy_context_exposure(&transaction)?;
        rebuild_provenance_inputs(&transaction)?;
        insert_reference_relations(&transaction, references)?;
        retract_redundant_related_relations(&transaction)?;
        rebuild_search_indexes(&transaction)?;
        transaction
            .execute("DELETE FROM pcp_access_log", [])
            .context("discard historical PCP access telemetry")?;
        transaction
            .execute(
                "UPDATE pcp_metadata SET value = ?1 WHERE key = 'schema_version'",
                [target_version],
            )
            .context("publish PCP clean Store schema version")?;
        transaction
            .commit()
            .context("commit PCP clean Store migration")?;
        Ok(())
    })();
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .context("restore PCP foreign key enforcement")?;
    migration?;
    let violation: Option<String> = connection
        .query_row("PRAGMA foreign_key_check", [], |row| row.get(0))
        .optional()
        .context("validate PCP clean Store foreign keys")?;
    anyhow::ensure!(
        violation.is_none(),
        "PCP clean Store migration left a foreign key violation in {}",
        violation.unwrap_or_default()
    );
    Ok(())
}

pub(crate) fn migrate_clean_associations(
    connection: &mut Connection,
    target_version: &str,
) -> Result<()> {
    let transaction = connection
        .transaction()
        .context("start PCP association cleanup migration")?;
    clean_legacy_context_exposure(&transaction)?;
    rebuild_provenance_inputs(&transaction)?;
    retract_redundant_related_relations(&transaction)?;
    transaction
        .execute(
            "UPDATE pcp_metadata SET value = ?1 WHERE key = 'schema_version'",
            [target_version],
        )
        .context("publish PCP association cleanup schema version")?;
    transaction
        .commit()
        .context("commit PCP association cleanup migration")?;
    Ok(())
}

pub(crate) fn migrate_clean_topic_extractions(
    connection: &mut Connection,
    target_version: &str,
) -> Result<()> {
    let transaction = connection
        .transaction()
        .context("start PCP topic extraction schema migration")?;
    transaction
        .execute_batch(
            "
            CREATE TABLE IF NOT EXISTS pcp_topic_extractions (
                topic_revision_id TEXT PRIMARY KEY REFERENCES pcp_revisions(revision_id),
                topic_page_id TEXT NOT NULL REFERENCES pcp_pages(page_id),
                namespace TEXT NOT NULL REFERENCES pcp_scopes(namespace),
                title TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS pcp_topic_extraction_members (
                topic_revision_id TEXT NOT NULL REFERENCES pcp_topic_extractions(topic_revision_id),
                topic_page_id TEXT NOT NULL REFERENCES pcp_pages(page_id),
                source_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id),
                source_page_id TEXT NOT NULL REFERENCES pcp_pages(page_id),
                position INTEGER NOT NULL,
                PRIMARY KEY (topic_revision_id, source_revision_id),
                UNIQUE (topic_revision_id, position)
            );
            CREATE INDEX IF NOT EXISTS pcp_topic_extraction_members_source
                ON pcp_topic_extraction_members(source_page_id, source_revision_id);
            ",
        )
        .context("create PCP topic extraction tables")?;
    transaction
        .execute(
            "UPDATE pcp_metadata SET value = ?1 WHERE key = 'schema_version'",
            [target_version],
        )
        .context("publish PCP topic extraction schema version")?;
    transaction
        .commit()
        .context("commit PCP topic extraction schema migration")?;
    Ok(())
}

fn clean_legacy_context_exposure(transaction: &Transaction<'_>) -> Result<()> {
    let mut statement = transaction
        .prepare(
            "SELECT revision.revision_id, revision.actor_type, revision.actor_id,
                    revision.payload_media_type, revision.payload_content,
                    revision.facets_json, revision.provenance_json
             FROM pcp_revisions revision
             JOIN pcp_pages page ON page.page_id = revision.page_id
             WHERE page.kind = 'conversation_event'",
        )
        .context("prepare legacy context exposure cleanup")?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
            ))
        })
        .context("query legacy context exposure cleanup")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collect legacy context exposure cleanup")?;
    drop(statement);

    for (revision_id, actor_type, actor_id, media_type, content, facets, provenance) in rows {
        let mut payload = content;
        let mut provenance = parse_json(&provenance, Value::Array(Vec::new()));
        let facets = facets
            .as_deref()
            .map(|encoded| parse_json(encoded, Value::Null));
        let mut changed =
            strip_legacy_context_inputs(&mut provenance, facets.as_ref(), &actor_type, &actor_id);

        if media_type.as_deref() == Some(PACKED_PAGE_MEDIA_TYPE)
            && let Some(content) = payload.as_mut()
            && let Ok(mut packed) = serde_json::from_str::<Value>(content)
            && strip_packed_legacy_context_inputs(&mut packed)
        {
            *content =
                serde_json::to_string(&packed).context("encode cleaned PCP packed payload")?;
            changed = true;
        }

        if changed {
            transaction
                .execute(
                    "UPDATE pcp_revisions
                     SET payload_content = ?2, provenance_json = ?3
                     WHERE revision_id = ?1",
                    params![
                        revision_id,
                        payload,
                        serde_json::to_string(&provenance)
                            .context("encode cleaned PCP provenance")?
                    ],
                )
                .context("write cleaned PCP provenance")?;
        }
    }
    Ok(())
}

fn strip_packed_legacy_context_inputs(packed: &mut Value) -> bool {
    let Some(entries) = packed.get_mut("entries").and_then(Value::as_array_mut) else {
        return false;
    };
    let mut changed = false;
    for entry in entries {
        let actor_type = entry
            .pointer("/createdBy/actorType")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let actor_id = entry
            .pointer("/createdBy/actorId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let facets = entry.get("facets").cloned();
        if let Some(provenance) = entry.get_mut("provenance") {
            changed |=
                strip_legacy_context_inputs(provenance, facets.as_ref(), &actor_type, &actor_id);
        }
    }
    changed
}

fn strip_legacy_context_inputs(
    provenance: &mut Value,
    facets: Option<&Value>,
    actor_type: &str,
    actor_id: &str,
) -> bool {
    if actor_type != "model"
        || actor_id != "codex:symbiont-d"
        || facets
            .and_then(|value| value.pointer("/messageMetadata/origin"))
            .and_then(Value::as_str)
            != Some("autonomous")
    {
        return false;
    }
    let Some(events) = provenance.as_array_mut() else {
        return false;
    };
    let mut changed = false;
    for event in events.iter_mut().filter_map(Value::as_object_mut) {
        if event.get("operation").and_then(Value::as_str) != Some("ingest") {
            continue;
        }
        let has_inputs = event
            .get("inputRevisionIds")
            .and_then(Value::as_array)
            .is_some_and(|inputs| !inputs.is_empty());
        if has_inputs {
            event.insert("inputRevisionIds".to_owned(), Value::Array(Vec::new()));
            changed = true;
        }
    }
    changed
}

fn retract_redundant_related_relations(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute(
            r#"
            INSERT OR IGNORE INTO pcp_relation_retractions (
                relation_id, retracted_actor_type, retracted_actor_id,
                retracted_at, reason
            )
            SELECT related.relation_id, 'system', 'pcp:migration',
                   strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                   'redundant_with_references_relation'
            FROM pcp_relations related
            WHERE related.relation_type = 'related_to'
              AND NOT EXISTS (
                  SELECT 1 FROM pcp_relation_retractions existing
                  WHERE existing.relation_id = related.relation_id
              )
              AND EXISTS (
                  SELECT 1
                  FROM pcp_relations reference
                  WHERE reference.relation_type = 'references'
                    AND (
                        (reference.from_page_id = related.from_page_id
                         AND reference.to_page_id = related.to_page_id)
                        OR
                        (reference.from_page_id = related.to_page_id
                         AND reference.to_page_id = related.from_page_id)
                    )
                    AND NOT EXISTS (
                        SELECT 1 FROM pcp_relation_retractions retracted_reference
                        WHERE retracted_reference.relation_id = reference.relation_id
                    )
              )
            "#,
            [],
        )
        .context("retract related_to relations duplicated by references")?;
    Ok(())
}

fn rebuild_contract_tables(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute_batch(
            r#"
            ALTER TABLE pcp_relation_retractions RENAME TO pcp_relation_retractions_draft;
            ALTER TABLE pcp_relations RENAME TO pcp_relations_draft;

            CREATE TABLE pcp_relations (
                relation_id TEXT PRIMARY KEY,
                relation_type TEXT NOT NULL,
                actor_type TEXT NOT NULL,
                actor_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                from_page_id TEXT NOT NULL REFERENCES pcp_pages(page_id),
                to_page_id TEXT NOT NULL REFERENCES pcp_pages(page_id),
                basis_revision_ids_json TEXT NOT NULL
            );
            CREATE TABLE pcp_relation_retractions (
                relation_id TEXT PRIMARY KEY REFERENCES pcp_relations(relation_id),
                retracted_actor_type TEXT NOT NULL,
                retracted_actor_id TEXT NOT NULL,
                retracted_at TEXT NOT NULL,
                reason TEXT NOT NULL
            );
            INSERT INTO pcp_relations (
                relation_id, relation_type, actor_type, actor_id, created_at,
                from_page_id, to_page_id, basis_revision_ids_json
            )
            SELECT relation_id, relation_type, actor_type, actor_id, created_at,
                   from_page_id, to_page_id, basis_revision_ids_json
            FROM pcp_relations_draft;
            INSERT INTO pcp_relation_retractions (
                relation_id, retracted_actor_type, retracted_actor_id,
                retracted_at, reason
            )
            SELECT relation_id, retracted_actor_type, retracted_actor_id,
                   retracted_at, reason
            FROM pcp_relation_retractions_draft;
            DROP TABLE pcp_relation_retractions_draft;
            DROP TABLE pcp_relations_draft;

            ALTER TABLE pcp_summary_idempotency RENAME TO pcp_summary_idempotency_draft;
            ALTER TABLE pcp_page_summary_heads RENAME TO pcp_page_summary_heads_draft;
            ALTER TABLE pcp_summaries RENAME TO pcp_summaries_draft;

            CREATE TABLE pcp_summaries (
                summary_revision_id TEXT PRIMARY KEY REFERENCES pcp_revisions(revision_id),
                target_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id)
            );
            CREATE TABLE pcp_page_summary_heads (
                target_page_id TEXT PRIMARY KEY REFERENCES pcp_pages(page_id),
                summary_page_id TEXT NOT NULL UNIQUE REFERENCES pcp_pages(page_id)
            );
            CREATE TABLE pcp_summary_idempotency (
                actor_id TEXT NOT NULL,
                idempotency_key TEXT NOT NULL,
                target_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id),
                result_summary_revision_id TEXT NOT NULL
                    REFERENCES pcp_summaries(summary_revision_id),
                created_at TEXT NOT NULL,
                PRIMARY KEY (actor_id, idempotency_key)
            );
            INSERT INTO pcp_summaries (summary_revision_id, target_revision_id)
            SELECT summary_revision_id, target_revision_id FROM pcp_summaries_draft;
            INSERT INTO pcp_page_summary_heads (target_page_id, summary_page_id)
            SELECT target_page_id, summary_page_id FROM pcp_page_summary_heads_draft;
            INSERT INTO pcp_summary_idempotency (
                actor_id, idempotency_key, target_revision_id,
                result_summary_revision_id, created_at
            )
            SELECT actor_id, idempotency_key, target_revision_id,
                   result_summary_revision_id, created_at
            FROM pcp_summary_idempotency_draft;
            DROP TABLE pcp_summary_idempotency_draft;
            DROP TABLE pcp_page_summary_heads_draft;
            DROP TABLE pcp_summaries_draft;

            ALTER TABLE pcp_validity_idempotency RENAME TO pcp_validity_idempotency_draft;
            ALTER TABLE pcp_validity_heads RENAME TO pcp_validity_heads_draft;
            ALTER TABLE pcp_validity_assessments RENAME TO pcp_validity_assessments_draft;

            CREATE TABLE pcp_validity_assessments (
                assessment_revision_id TEXT PRIMARY KEY REFERENCES pcp_revisions(revision_id),
                target_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id)
            );
            CREATE TABLE pcp_validity_heads (
                target_page_id TEXT PRIMARY KEY REFERENCES pcp_pages(page_id),
                assessment_page_id TEXT NOT NULL UNIQUE REFERENCES pcp_pages(page_id)
            );
            CREATE TABLE pcp_validity_idempotency (
                actor_id TEXT NOT NULL,
                idempotency_key TEXT NOT NULL,
                target_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id),
                result_assessment_id TEXT NOT NULL
                    REFERENCES pcp_validity_assessments(assessment_revision_id),
                created_at TEXT NOT NULL,
                PRIMARY KEY (actor_id, idempotency_key)
            );
            INSERT INTO pcp_validity_assessments (
                assessment_revision_id, target_revision_id
            )
            SELECT assessment_id, target_revision_id FROM pcp_validity_assessments_draft;
            INSERT INTO pcp_validity_heads (target_page_id, assessment_page_id)
            SELECT head.target_page_id, revision.page_id
            FROM pcp_validity_heads_draft head
            JOIN pcp_revisions revision
              ON revision.revision_id = head.current_assessment_id;
            INSERT INTO pcp_validity_idempotency (
                actor_id, idempotency_key, target_revision_id,
                result_assessment_id, created_at
            )
            SELECT actor_id, idempotency_key, target_revision_id,
                   result_assessment_id, created_at
            FROM pcp_validity_idempotency_draft;
            DROP TABLE pcp_validity_idempotency_draft;
            DROP TABLE pcp_validity_heads_draft;
            DROP TABLE pcp_validity_assessments_draft;
            "#,
        )
        .context("rebuild minimal PCP projection and Relation tables")?;
    Ok(())
}

fn normalize_revision_content(transaction: &Transaction<'_>) -> Result<Vec<ReferenceCandidate>> {
    let revision_to_page = load_revision_pages(transaction)?;
    let page_heads = load_page_heads(transaction)?;
    let valid_revisions = revision_to_page.keys().cloned().collect::<HashSet<_>>();
    let packed_replacements = load_packed_replacements(transaction)?;
    let image_targets = load_image_targets(transaction, &page_heads)?;
    let rows = load_revision_rows(transaction)?;
    let mut references = HashSet::new();

    for row in rows {
        let mut payload_content = row.payload_content.clone();
        let mut source_refs = parse_json(&row.source_refs_json, Value::Array(Vec::new()));
        let mut facets = row
            .facets_json
            .as_deref()
            .map(|encoded| parse_json(encoded, Value::Null));
        let mut provenance = parse_json(&row.provenance_json, Value::Array(Vec::new()));
        let mut observed_at = row.observed_at.clone();

        normalize_source_refs(&mut source_refs);
        normalize_provenance(
            &mut provenance,
            &row.revision_id,
            &valid_revisions,
            &packed_replacements,
            true,
        );

        if row.payload_media_type.as_deref() == Some(PACKED_PAGE_MEDIA_TYPE) {
            if let Some(content) = payload_content.as_mut() {
                if let Ok(mut packed) = serde_json::from_str::<Value>(content) {
                    normalize_packed_payload(
                        &mut packed,
                        &row,
                        &valid_revisions,
                        &packed_replacements,
                        &image_targets,
                        &revision_to_page,
                        &page_heads,
                        &mut references,
                    );
                    *content = serde_json::to_string(&packed)
                        .context("encode normalized PCP packed payload")?;
                }
            }
        } else {
            normalize_payload_and_facets(
                &row,
                &mut payload_content,
                &mut facets,
                &mut source_refs,
                &mut observed_at,
                &image_targets,
                &revision_to_page,
                &page_heads,
                &mut references,
            );
        }

        let source_refs_json =
            serde_json::to_string(&source_refs).context("encode normalized PCP SourceRefs")?;
        let facets_json = facets
            .filter(|value| !is_empty_json(value))
            .map(|value| serde_json::to_string(&value))
            .transpose()
            .context("encode normalized PCP facets")?;
        let provenance_json =
            serde_json::to_string(&provenance).context("encode normalized PCP provenance")?;
        transaction
            .execute(
                "UPDATE pcp_revisions
                 SET payload_content = ?2, source_refs_json = ?3,
                     facets_json = ?4, provenance_json = ?5, observed_at = ?6
                 WHERE revision_id = ?1",
                params![
                    row.revision_id,
                    payload_content,
                    source_refs_json,
                    facets_json,
                    provenance_json,
                    observed_at
                ],
            )
            .context("write normalized PCP Revision")?;
    }
    Ok(references.into_iter().collect())
}

fn normalize_payload_and_facets(
    row: &RevisionRow,
    payload_content: &mut Option<String>,
    facets: &mut Option<Value>,
    source_refs: &mut Value,
    observed_at: &mut Option<String>,
    image_targets: &HashMap<String, ImageTarget>,
    revision_to_page: &HashMap<String, String>,
    page_heads: &HashMap<String, String>,
    references: &mut HashSet<ReferenceCandidate>,
) {
    match row.kind.as_str() {
        "summary_projection" => *facets = None,
        "validity_assessment" => retain_object_keys(facets, &["standing", "scope"]),
        "image_asset" => {
            normalize_image_payload(payload_content, source_refs, facets);
        }
        "external_signal" => {
            normalize_external_signal(payload_content, facets, observed_at);
        }
        _ => {
            remove_facet_key(facets, "kind");
            normalize_conversation_facets(
                payload_content.as_deref(),
                facets,
                &row.page_id,
                &row.revision_id,
                &row.created_at,
                image_targets,
                revision_to_page,
                page_heads,
                references,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn normalize_packed_payload(
    packed: &mut Value,
    outer: &RevisionRow,
    valid_revisions: &HashSet<String>,
    packed_replacements: &HashMap<String, String>,
    image_targets: &HashMap<String, ImageTarget>,
    revision_to_page: &HashMap<String, String>,
    page_heads: &HashMap<String, String>,
    references: &mut HashSet<ReferenceCandidate>,
) {
    let Some(entries) = packed.get_mut("entries").and_then(Value::as_array_mut) else {
        return;
    };
    for entry in entries {
        let entry_revision_id = entry
            .get("revisionId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let payload_text = entry
            .get("payload")
            .and_then(Value::as_object)
            .and_then(|payload| payload.get("content"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(source_refs) = entry.get_mut("sourceRefs") {
            normalize_source_refs(source_refs);
        }
        if let Some(provenance) = entry.get_mut("provenance") {
            normalize_provenance(
                provenance,
                &entry_revision_id,
                valid_revisions,
                packed_replacements,
                false,
            );
        }
        if let Some(facets) = entry.get_mut("facets") {
            let mut owned = Some(facets.take());
            remove_facet_key(&mut owned, "kind");
            normalize_conversation_facets(
                payload_text.as_deref(),
                &mut owned,
                &outer.page_id,
                &outer.revision_id,
                &outer.created_at,
                image_targets,
                revision_to_page,
                page_heads,
                references,
            );
            *facets = owned.unwrap_or(Value::Null);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn normalize_conversation_facets(
    payload_text: Option<&str>,
    facets: &mut Option<Value>,
    owner_page_id: &str,
    owner_revision_id: &str,
    created_at: &str,
    image_targets: &HashMap<String, ImageTarget>,
    revision_to_page: &HashMap<String, String>,
    page_heads: &HashMap<String, String>,
    references: &mut HashSet<ReferenceCandidate>,
) {
    let Some(object) = facets.as_mut().and_then(Value::as_object_mut) else {
        return;
    };
    if let Some(metadata) = object.get_mut("messageMetadata") {
        normalize_message_metadata(metadata);
        if is_empty_json(metadata) {
            object.remove("messageMetadata");
        }
    }
    if let Some(parts) = object.get_mut("contentParts").and_then(Value::as_array_mut) {
        let mut normalized = Vec::new();
        for mut part in std::mem::take(parts) {
            let part_type = part.get("type").and_then(Value::as_str).unwrap_or_default();
            match part_type {
                "markdown" if part.get("text").and_then(Value::as_str) == payload_text => {}
                "image" => {
                    if let Some(target) = image_target_for_part(&part, image_targets) {
                        normalized.push(json!({"type": "image", "pageId": target.page_id}));
                        references.insert(ReferenceCandidate {
                            from_page_id: owner_page_id.to_owned(),
                            from_revision_id: owner_revision_id.to_owned(),
                            to_page_id: target.page_id.clone(),
                            to_revision_id: target.revision_id.clone(),
                            created_at: created_at.to_owned(),
                        });
                    } else {
                        normalized.push(part);
                    }
                }
                "quote" => normalized.push(minimal_quote(&part)),
                "externalInput" | "external_input" => {
                    if let Some(target) =
                        reference_target_for_part(&part, revision_to_page, page_heads)
                    {
                        normalized.push(json!({
                            "type": "externalInput",
                            "pageId": target.0
                        }));
                        references.insert(ReferenceCandidate {
                            from_page_id: owner_page_id.to_owned(),
                            from_revision_id: owner_revision_id.to_owned(),
                            to_page_id: target.0,
                            to_revision_id: target.1,
                            created_at: created_at.to_owned(),
                        });
                    } else {
                        strip_external_input(&mut part);
                        normalized.push(part);
                    }
                }
                _ => normalized.push(part),
            }
        }
        *parts = normalized;
        if parts.is_empty() {
            object.remove("contentParts");
        }
    }
}

fn normalize_message_metadata(value: &mut Value) {
    let Some(metadata) = value.as_object_mut() else {
        return;
    };
    metadata.remove("traceId");
    if metadata
        .get("origin")
        .and_then(Value::as_str)
        .is_some_and(|origin| matches!(origin, "conversation" | "interactive" | "default"))
    {
        metadata.remove("origin");
    }
    if metadata.get("pcpToolCalls") == metadata.get("toolCalls") {
        metadata.remove("pcpToolCalls");
    }
    let single_run_totals =
        if let Some(runs) = metadata.get_mut("runs").and_then(Value::as_array_mut) {
            for run in runs.iter_mut() {
                if let Some(run) = run.as_object_mut() {
                    run.remove("displayName");
                    run.remove("lane");
                }
            }
            if runs.len() == 1 {
                let run = runs[0].as_object();
                Some((
                    run.and_then(|value| value.get("durationMs")).cloned(),
                    run.and_then(|value| value.get("totalTokens")).cloned(),
                ))
            } else {
                None
            }
        } else {
            None
        };
    if let Some((duration, tokens)) = single_run_totals {
        if metadata.get("durationMs") == duration.as_ref() {
            metadata.remove("durationMs");
        }
        if metadata.get("totalTokens") == tokens.as_ref() {
            metadata.remove("totalTokens");
        }
    }
}

fn normalize_image_payload(
    payload_content: &mut Option<String>,
    source_refs: &mut Value,
    facets: &mut Option<Value>,
) {
    let Some(content) = payload_content.as_ref() else {
        *facets = None;
        return;
    };
    let Ok(payload) = serde_json::from_str::<Value>(content) else {
        *facets = None;
        return;
    };
    let digest = payload
        .get("sha256")
        .and_then(Value::as_str)
        .map(|value| format!("sha256:{value}"));
    let media_type = payload
        .get("mimeType")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if let Some(first) = source_refs.as_array_mut().and_then(|refs| refs.first_mut()) {
        if let Some(reference) = first.as_object_mut() {
            if !reference.contains_key("mediaType") {
                if let Some(media_type) = media_type {
                    reference.insert("mediaType".to_owned(), Value::String(media_type));
                }
            }
            if !reference.contains_key("contentDigest") {
                if let Some(digest) = digest {
                    reference.insert("contentDigest".to_owned(), Value::String(digest));
                }
            }
        }
    }
    let mut minimal = Map::new();
    if let Some(object) = payload.as_object() {
        for key in ["filename", "mimeType", "byteSize", "width", "height"] {
            if let Some(value) = object.get(key).filter(|value| !value.is_null()) {
                minimal.insert(key.to_owned(), value.clone());
            }
        }
    }
    *payload_content = Some(Value::Object(minimal).to_string());
    *facets = None;
}

fn normalize_external_signal(
    payload_content: &mut Option<String>,
    facets: &mut Option<Value>,
    observed_at: &mut Option<String>,
) {
    retain_object_keys(facets, &["source_class"]);
    let Some(content) = payload_content.as_ref() else {
        return;
    };
    let Ok(payload) = serde_json::from_str::<Value>(content) else {
        return;
    };
    if observed_at.is_none() {
        *observed_at = payload
            .get("observed_at")
            .and_then(Value::as_str)
            .map(str::to_owned);
    }
    let mut minimal = Map::new();
    if let Some(object) = payload.as_object() {
        for key in [
            "title",
            "summary",
            "content",
            "event_at",
            "qualification_note",
            "review_reason",
            "related_signal_ids",
        ] {
            if let Some(value) = object.get(key).filter(|value| !is_empty_json(value)) {
                minimal.insert(key.to_owned(), value.clone());
            }
        }
        if object.get("received_text") != object.get("content") {
            if let Some(value) = object
                .get("received_text")
                .filter(|value| !is_empty_json(value))
            {
                minimal.insert("received_text".to_owned(), value.clone());
            }
        }
    }
    *payload_content = Some(Value::Object(minimal).to_string());
}

fn normalize_source_refs(value: &mut Value) {
    let Some(references) = value.as_array_mut() else {
        *value = Value::Array(Vec::new());
        return;
    };
    references.retain(|reference| {
        reference.get("providerId").and_then(Value::as_str) != Some("legacy_markdown_memory")
    });
}

fn normalize_provenance(
    provenance: &mut Value,
    self_revision_id: &str,
    valid_revisions: &HashSet<String>,
    packed_replacements: &HashMap<String, String>,
    map_packed: bool,
) {
    let Some(events) = provenance.as_array_mut() else {
        *provenance = Value::Array(Vec::new());
        return;
    };
    for event in events {
        let Some(object) = event.as_object_mut() else {
            continue;
        };
        let mut input_ids = Vec::new();
        for key in ["inputRevisionIds", "inputPageIds"] {
            if let Some(values) = object
                .remove(key)
                .and_then(|value| value.as_array().cloned())
            {
                input_ids.extend(
                    values
                        .into_iter()
                        .filter_map(|value| value.as_str().map(str::to_owned)),
                );
            }
        }
        let mut normalized = BTreeSet::new();
        for revision_id in input_ids {
            let resolved = if valid_revisions.contains(&revision_id) {
                Some(revision_id)
            } else if map_packed {
                packed_replacements.get(&revision_id).cloned()
            } else {
                None
            };
            if let Some(resolved) = resolved.filter(|value| value != self_revision_id) {
                normalized.insert(resolved);
            }
        }
        object.insert(
            "inputRevisionIds".to_owned(),
            Value::Array(normalized.into_iter().map(Value::String).collect()),
        );
    }
}

fn rebuild_provenance_inputs(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute("DELETE FROM pcp_provenance_inputs", [])
        .context("clear PCP provenance input index")?;
    let mut statement = transaction
        .prepare("SELECT revision_id, created_at, provenance_json FROM pcp_revisions")
        .context("prepare PCP provenance input rebuild")?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .context("query PCP provenance input rebuild")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collect PCP provenance input rebuild")?;
    drop(statement);
    for (derived_revision_id, created_at, encoded) in rows {
        let events = parse_json(&encoded, Value::Array(Vec::new()));
        let Some(events) = events.as_array() else {
            continue;
        };
        for input_revision_id in events
            .iter()
            .filter_map(Value::as_object)
            .flat_map(|event| {
                event
                    .get("inputRevisionIds")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .filter_map(Value::as_str)
        {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO pcp_provenance_inputs (
                        derived_revision_id, input_revision_id, created_at
                     ) VALUES (?1, ?2, ?3)",
                    params![derived_revision_id, input_revision_id, created_at],
                )
                .context("rebuild PCP provenance input")?;
        }
    }
    Ok(())
}

fn insert_reference_relations(
    transaction: &Transaction<'_>,
    references: Vec<ReferenceCandidate>,
) -> Result<()> {
    for reference in references {
        if reference.from_page_id == reference.to_page_id {
            continue;
        }
        let mut basis = vec![reference.from_revision_id, reference.to_revision_id];
        basis.sort();
        basis.dedup();
        let basis_json = serde_json::to_string(&basis)?;
        transaction
            .execute(
                "INSERT INTO pcp_relations (
                    relation_id, relation_type, actor_type, actor_id, created_at,
                    from_page_id, to_page_id, basis_revision_ids_json
                 )
                 SELECT 'rel_' || lower(hex(randomblob(16))), 'references',
                        'system', 'pcp:migration', ?1, ?2, ?3, ?4
                 WHERE NOT EXISTS (
                     SELECT 1 FROM pcp_relations relation
                     WHERE relation.from_page_id = ?2
                       AND relation.relation_type = 'references'
                       AND relation.to_page_id = ?3
                       AND NOT EXISTS (
                           SELECT 1 FROM pcp_relation_retractions retraction
                           WHERE retraction.relation_id = relation.relation_id
                       )
                 )",
                params![
                    reference.created_at,
                    reference.from_page_id,
                    reference.to_page_id,
                    basis_json
                ],
            )
            .context("materialize PCP content reference relation")?;
    }
    Ok(())
}

fn rebuild_search_indexes(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute_batch(
            r#"
            DELETE FROM pcp_revision_fts;
            INSERT INTO pcp_revision_fts (
                revision_id, page_id, namespace, payload_content, facets_text
            )
            SELECT revision_id, page_id, namespace,
                   COALESCE(payload_content, ''), COALESCE(facets_json, '')
            FROM pcp_revisions;
            DELETE FROM pcp_summary_fts;
            INSERT INTO pcp_summary_fts (
                summary_revision_id, target_revision_id, content
            )
            SELECT summary.summary_revision_id, summary.target_revision_id,
                   COALESCE(revision.payload_content, '')
            FROM pcp_page_summary_heads head
            JOIN pcp_pages page ON page.page_id = head.summary_page_id
            JOIN pcp_summaries summary
              ON summary.summary_revision_id = page.current_revision_id
            JOIN pcp_revisions revision
              ON revision.revision_id = summary.summary_revision_id;
            "#,
        )
        .context("rebuild PCP clean Store search indexes")?;
    Ok(())
}

fn load_revision_rows(transaction: &Transaction<'_>) -> Result<Vec<RevisionRow>> {
    let mut statement = transaction
        .prepare(
            "SELECT revision.revision_id, revision.page_id, page.kind,
                    revision.payload_media_type, revision.payload_content,
                    revision.source_refs_json, revision.facets_json,
                    revision.provenance_json, revision.observed_at,
                    revision.created_at
             FROM pcp_revisions revision
             JOIN pcp_pages page ON page.page_id = revision.page_id",
        )
        .context("prepare PCP Revision cleanup inventory")?;
    statement
        .query_map([], |row| {
            Ok(RevisionRow {
                revision_id: row.get(0)?,
                page_id: row.get(1)?,
                kind: row.get(2)?,
                payload_media_type: row.get(3)?,
                payload_content: row.get(4)?,
                source_refs_json: row.get(5)?,
                facets_json: row.get(6)?,
                provenance_json: row.get(7)?,
                observed_at: row.get(8)?,
                created_at: row.get(9)?,
            })
        })
        .context("query PCP Revision cleanup inventory")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collect PCP Revision cleanup inventory")
}

fn load_revision_pages(transaction: &Transaction<'_>) -> Result<HashMap<String, String>> {
    let mut statement = transaction.prepare("SELECT revision_id, page_id FROM pcp_revisions")?;
    Ok(statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<HashMap<_, _>>>()?)
}

fn load_page_heads(transaction: &Transaction<'_>) -> Result<HashMap<String, String>> {
    let mut statement = transaction.prepare(
        "SELECT page_id, current_revision_id FROM pcp_pages
         WHERE current_revision_id IS NOT NULL",
    )?;
    Ok(statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<HashMap<_, _>>>()?)
}

fn load_packed_replacements(transaction: &Transaction<'_>) -> Result<HashMap<String, String>> {
    let mut statement =
        transaction.prepare("SELECT source_revision_id, packed_revision_id FROM pcp_page_packs")?;
    Ok(statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<HashMap<_, _>>>()?)
}

fn load_image_targets(
    transaction: &Transaction<'_>,
    page_heads: &HashMap<String, String>,
) -> Result<HashMap<String, ImageTarget>> {
    let mut statement = transaction.prepare(
        "SELECT page.page_id, revision.payload_content, revision.facets_json,
                revision.source_refs_json
         FROM pcp_pages page
         JOIN pcp_revisions revision ON revision.revision_id = page.current_revision_id
         WHERE page.kind = 'image_asset'",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut targets = HashMap::new();
    for (page_id, payload, facets, refs) in rows {
        let payload = payload
            .as_deref()
            .map(|value| parse_json(value, Value::Null));
        let facets = facets
            .as_deref()
            .map(|value| parse_json(value, Value::Null));
        let refs = parse_json(&refs, Value::Array(Vec::new()));
        let sha = payload
            .as_ref()
            .and_then(|value| value.get("sha256"))
            .and_then(Value::as_str)
            .or_else(|| {
                facets
                    .as_ref()
                    .and_then(|value| value.get("sha256"))
                    .and_then(Value::as_str)
            })
            .map(normalize_digest)
            .or_else(|| {
                refs.as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|reference| reference.get("contentDigest"))
                    .filter_map(Value::as_str)
                    .next()
                    .map(normalize_digest)
            });
        if let (Some(sha), Some(revision_id)) = (sha, page_heads.get(&page_id)) {
            targets.insert(
                sha,
                ImageTarget {
                    page_id,
                    revision_id: revision_id.clone(),
                },
            );
        }
    }
    Ok(targets)
}

fn image_target_for_part<'a>(
    part: &Value,
    targets: &'a HashMap<String, ImageTarget>,
) -> Option<&'a ImageTarget> {
    let asset = part.get("asset").unwrap_or(part);
    ["sha256", "assetId", "contentDigest"]
        .into_iter()
        .filter_map(|key| asset.get(key))
        .filter_map(Value::as_str)
        .map(|value| {
            let digest = value
                .strip_prefix("img_")
                .unwrap_or(value)
                .split('.')
                .next()
                .unwrap_or(value);
            normalize_digest(digest)
        })
        .find_map(|digest| targets.get(&digest))
}

fn reference_target_for_part(
    part: &Value,
    revision_to_page: &HashMap<String, String>,
    page_heads: &HashMap<String, String>,
) -> Option<(String, String)> {
    let part = part.get("input").unwrap_or(part);
    if let Some(page_id) = part.get("pageId").and_then(Value::as_str) {
        return page_heads
            .get(page_id)
            .map(|revision_id| (page_id.to_owned(), revision_id.clone()));
    }
    let revision_id = part.get("sourceRevisionId").and_then(Value::as_str)?;
    let page_id = revision_to_page.get(revision_id)?;
    let head = page_heads.get(page_id)?;
    Some((page_id.clone(), head.clone()))
}

fn minimal_quote(part: &Value) -> Value {
    let mut minimal = Map::new();
    minimal.insert("type".to_owned(), Value::String("quote".to_owned()));
    let part = part.get("quote").unwrap_or(part);
    if let Some(object) = part.as_object() {
        for key in ["text", "sourceRole", "sourceAt", "truncated"] {
            if let Some(value) = object.get(key).filter(|value| !is_empty_json(value)) {
                minimal.insert(key.to_owned(), value.clone());
            }
        }
    }
    Value::Object(minimal)
}

fn strip_external_input(part: &mut Value) {
    if let Some(input) = part.get_mut("input") {
        let mut minimal = json!({"type": "externalInput"});
        if let (Some(target), Some(source)) = (minimal.as_object_mut(), input.as_object()) {
            for key in ["title", "excerpt"] {
                if let Some(value) = source.get(key).filter(|value| !is_empty_json(value)) {
                    target.insert(key.to_owned(), value.clone());
                }
            }
        }
        *part = minimal;
        return;
    }
    let Some(object) = part.as_object_mut() else {
        return;
    };
    object.remove("sourceRevisionId");
    object.remove("sourceSha256");
    object.remove("sourceStart");
    object.remove("sourceEnd");
    object.retain(|key, value| {
        matches!(key.as_str(), "type" | "title" | "excerpt" | "text") && !is_empty_json(value)
    });
}

fn retain_object_keys(value: &mut Option<Value>, keys: &[&str]) {
    let Some(object) = value.as_mut().and_then(Value::as_object_mut) else {
        *value = None;
        return;
    };
    object.retain(|key, entry| keys.contains(&key.as_str()) && !is_empty_json(entry));
    if object.is_empty() {
        *value = None;
    }
}

fn remove_facet_key(value: &mut Option<Value>, key: &str) {
    if let Some(object) = value.as_mut().and_then(Value::as_object_mut) {
        object.remove(key);
        if object.is_empty() {
            *value = None;
        }
    }
}

fn parse_json(encoded: &str, fallback: Value) -> Value {
    serde_json::from_str(encoded).unwrap_or(fallback)
}

fn normalize_digest(value: &str) -> String {
    value
        .strip_prefix("sha256:")
        .unwrap_or(value)
        .to_ascii_lowercase()
}

fn is_empty_json(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(value) => value.trim().is_empty(),
        Value::Array(values) => values.is_empty(),
        Value::Object(values) => values.is_empty(),
        _ => false,
    }
}
