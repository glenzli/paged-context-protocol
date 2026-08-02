use std::collections::HashSet;

use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use pcp_core::{
    Actor, CreateScopeRequest, LinkPagesRequest, PagePayload, ProvenanceEvent, Relation,
    RevisePageRequest, SourceRef, WritePageRequest, WriteResult,
};
use rusqlite::{OptionalExtension, Transaction, params};

use crate::store::{MAX_PAGE_CHARS, SqlitePcpStore};

const MAX_PROVENANCE_INPUTS_PER_EVENT: usize = 256;

impl SqlitePcpStore {
    pub async fn create_scope(&self, request: CreateScopeRequest) -> Result<()> {
        validate_scope(&request)?;
        self.run("scope write", move |connection| {
            let now = now();
            connection
                .execute(
                    "
                    INSERT INTO pcp_scopes (
                        namespace, owner_id, scope_type, display_name, description,
                        parent_namespace, visibility, created_at, updated_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
                    ON CONFLICT(namespace) DO UPDATE SET
                        display_name = excluded.display_name,
                        description = excluded.description,
                        parent_namespace = excluded.parent_namespace,
                        updated_at = excluded.updated_at
                    WHERE pcp_scopes.owner_id = excluded.owner_id
                      AND pcp_scopes.scope_type = excluded.scope_type
                      AND pcp_scopes.visibility = excluded.visibility
                    ",
                    params![
                        request.namespace,
                        request.owner_id,
                        request.scope_type,
                        request.display_name,
                        request.description,
                        request.parent_namespace,
                        request.visibility,
                        now,
                    ],
                )
                .context("create or update PCP scope")?;
            Ok(())
        })
        .await
    }

    pub async fn write_page(
        &self,
        request: WritePageRequest,
        allowed_scopes: Vec<String>,
    ) -> Result<WriteResult> {
        validate_document(request.payload.as_ref(), &request.source_refs)?;
        let allowed_scopes = scope_set(allowed_scopes);
        self.run("page write", move |mut connection| {
            let transaction = connection.transaction().context("start PCP page write")?;
            ensure_scope_access(
                &transaction,
                &request.namespace,
                &request.owner_id,
                &allowed_scopes,
            )?;

            if let Some(existing) = lookup_write_idempotency(
                &transaction,
                &request.created_by.actor_id,
                "write_page",
                request.idempotency_key.as_deref(),
            )? {
                return Ok(existing);
            }

            for relation in &request.initial_relations {
                ensure_revision_access(&transaction, &relation.to_revision_id, &allowed_scopes)?;
            }

            let timestamp = now();
            let page_id = random_id(&transaction, "pg_")?;
            let revision_id = random_id(&transaction, "rev_")?;
            let provenance = complete_provenance(
                request.provenance,
                "write",
                &request.created_by,
                &timestamp,
                Vec::new(),
            )?;
            ensure_provenance_access(&transaction, &provenance, &allowed_scopes)?;

            transaction
                .execute(
                    "
                    INSERT INTO pcp_pages (page_id, current_revision_id, created_at)
                    VALUES (?1, NULL, ?2)
                    ",
                    params![page_id, timestamp],
                )
                .context("create PCP page")?;
            insert_revision(
                &transaction,
                &page_id,
                &revision_id,
                &request.owner_id,
                &request.namespace,
                &request.visibility,
                request.lifecycle_status.as_str(),
                &timestamp,
                request.observed_at.as_deref(),
                request.valid_from.as_deref(),
                request.valid_to.as_deref(),
                &request.created_by,
                request.payload.as_ref(),
                &request.source_refs,
                request.facets.as_ref(),
                &provenance,
            )?;
            transaction
                .execute(
                    "UPDATE pcp_pages SET current_revision_id = ?2 WHERE page_id = ?1",
                    params![page_id, revision_id],
                )
                .context("publish PCP page revision")?;

            for relation in request.initial_relations {
                insert_relation(
                    &transaction,
                    &revision_id,
                    &relation.relation_type,
                    &relation.to_revision_id,
                    &request.created_by,
                    &timestamp,
                )?;
            }
            record_idempotency(
                &transaction,
                &request.created_by.actor_id,
                "write_page",
                request.idempotency_key.as_deref(),
                Some(&page_id),
                Some(&revision_id),
                None,
                &timestamp,
            )?;
            transaction.commit().context("commit PCP page write")?;
            Ok(WriteResult {
                page_id,
                revision_id,
                created: true,
            })
        })
        .await
    }

    pub async fn revise_page(
        &self,
        request: RevisePageRequest,
        allowed_scopes: Vec<String>,
    ) -> Result<WriteResult> {
        validate_document(request.payload.as_ref(), &request.source_refs)?;
        let allowed_scopes = scope_set(allowed_scopes);
        self.run("page revision", move |mut connection| {
            let transaction = connection.transaction().context("start PCP revision")?;
            if let Some(existing) = lookup_write_idempotency(
                &transaction,
                &request.created_by.actor_id,
                "revise_page",
                request.idempotency_key.as_deref(),
            )? {
                return Ok(existing);
            }

            for relation in &request.initial_relations {
                ensure_revision_access(&transaction, &relation.to_revision_id, &allowed_scopes)?;
            }

            let (owner_id, namespace, visibility, current_revision_id): (
                String,
                String,
                String,
                String,
            ) = transaction
                .query_row(
                    "
                    SELECT r.owner_id, r.namespace, r.visibility, p.current_revision_id
                    FROM pcp_pages p
                    JOIN pcp_revisions r ON r.revision_id = p.current_revision_id
                    WHERE p.page_id = ?1
                    ",
                    [&request.page_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .context("find current PCP page revision")?;
            if !allowed_scopes.contains(&namespace) {
                anyhow::bail!("page is outside the authorized PCP scopes");
            }
            if current_revision_id != request.expected_revision_id {
                anyhow::bail!(
                    "revision conflict: expected {}, current revision is {}",
                    request.expected_revision_id,
                    current_revision_id
                );
            }

            let timestamp = now();
            let revision_id = random_id(&transaction, "rev_")?;
            let provenance = complete_provenance(
                request.provenance,
                "revise",
                &request.created_by,
                &timestamp,
                vec![request.expected_revision_id.clone()],
            )?;
            ensure_provenance_access(&transaction, &provenance, &allowed_scopes)?;
            insert_revision(
                &transaction,
                &request.page_id,
                &revision_id,
                &owner_id,
                &namespace,
                &visibility,
                request.lifecycle_status.as_str(),
                &timestamp,
                request.observed_at.as_deref(),
                request.valid_from.as_deref(),
                request.valid_to.as_deref(),
                &request.created_by,
                request.payload.as_ref(),
                &request.source_refs,
                request.facets.as_ref(),
                &provenance,
            )?;
            for relation in request.initial_relations {
                insert_relation(
                    &transaction,
                    &revision_id,
                    &relation.relation_type,
                    &relation.to_revision_id,
                    &request.created_by,
                    &timestamp,
                )?;
            }
            transaction
                .execute(
                    "
                    UPDATE pcp_pages
                    SET current_revision_id = ?2
                    WHERE page_id = ?1 AND current_revision_id = ?3
                    ",
                    params![request.page_id, revision_id, request.expected_revision_id],
                )
                .context("publish revised PCP page")?;
            record_idempotency(
                &transaction,
                &request.created_by.actor_id,
                "revise_page",
                request.idempotency_key.as_deref(),
                Some(&request.page_id),
                Some(&revision_id),
                None,
                &timestamp,
            )?;
            transaction.commit().context("commit PCP revision")?;
            Ok(WriteResult {
                page_id: request.page_id,
                revision_id,
                created: true,
            })
        })
        .await
    }

    pub async fn link_pages(
        &self,
        request: LinkPagesRequest,
        allowed_scopes: Vec<String>,
    ) -> Result<Relation> {
        if request.relation_type.trim().is_empty() || request.relation_type.len() > 80 {
            anyhow::bail!("relation type must contain 1-80 characters");
        }
        let allowed_scopes = scope_set(allowed_scopes);
        self.run("page link", move |mut connection| {
            let transaction = connection
                .transaction()
                .context("start PCP relation write")?;
            ensure_revision_access(&transaction, &request.from_revision_id, &allowed_scopes)?;
            ensure_revision_access(&transaction, &request.to_revision_id, &allowed_scopes)?;

            if let Some(key) = request.idempotency_key.as_deref()
                && let Some(relation_id) = transaction
                    .query_row(
                        "
                        SELECT result_relation_id
                        FROM pcp_idempotency
                        WHERE actor_id = ?1 AND operation = 'link_pages'
                          AND idempotency_key = ?2
                        ",
                        params![request.created_by.actor_id, key],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .context("look up PCP relation idempotency")?
            {
                return read_relation(&transaction, &relation_id);
            }

            let timestamp = now();
            let relation = insert_relation(
                &transaction,
                &request.from_revision_id,
                &request.relation_type,
                &request.to_revision_id,
                &request.created_by,
                &timestamp,
            )?;
            record_idempotency(
                &transaction,
                &request.created_by.actor_id,
                "link_pages",
                request.idempotency_key.as_deref(),
                None,
                None,
                Some(&relation.relation_id),
                &timestamp,
            )?;
            transaction.commit().context("commit PCP relation")?;
            Ok(relation)
        })
        .await
    }
}

fn validate_scope(request: &CreateScopeRequest) -> Result<()> {
    if request.namespace.trim().is_empty() || request.namespace.len() > 200 {
        anyhow::bail!("scope namespace must contain 1-200 characters");
    }
    if request.owner_id.trim().is_empty() || request.scope_type.trim().is_empty() {
        anyhow::bail!("scope owner and type cannot be empty");
    }
    if request.display_name.trim().is_empty() || request.visibility.trim().is_empty() {
        anyhow::bail!("scope display name and visibility cannot be empty");
    }
    Ok(())
}

fn validate_document(payload: Option<&PagePayload>, source_refs: &[SourceRef]) -> Result<()> {
    if payload.is_none() && source_refs.is_empty() {
        anyhow::bail!("a PCP page requires a payload or source reference");
    }
    if let Some(payload) = payload {
        if payload.media_type.trim().is_empty() {
            anyhow::bail!("PCP payload media type cannot be empty");
        }
        if payload.content.chars().count() > MAX_PAGE_CHARS {
            anyhow::bail!("PCP payload exceeds {MAX_PAGE_CHARS} characters");
        }
    }
    Ok(())
}

fn scope_set(scopes: Vec<String>) -> HashSet<String> {
    scopes.into_iter().collect()
}

fn ensure_scope_access(
    transaction: &Transaction<'_>,
    namespace: &str,
    owner_id: &str,
    allowed_scopes: &HashSet<String>,
) -> Result<()> {
    if !allowed_scopes.contains(namespace) {
        anyhow::bail!("scope is outside the authorized PCP scope set");
    }
    let stored_owner: String = transaction
        .query_row(
            "SELECT owner_id FROM pcp_scopes WHERE namespace = ?1",
            [namespace],
            |row| row.get(0),
        )
        .with_context(|| format!("find PCP scope {namespace}"))?;
    if stored_owner != owner_id {
        anyhow::bail!("PCP scope owner does not match the write request");
    }
    Ok(())
}

fn ensure_revision_access(
    transaction: &Transaction<'_>,
    revision_id: &str,
    allowed_scopes: &HashSet<String>,
) -> Result<()> {
    let namespace: String = transaction
        .query_row(
            "SELECT namespace FROM pcp_revisions WHERE revision_id = ?1",
            [revision_id],
            |row| row.get(0),
        )
        .with_context(|| format!("find PCP revision {revision_id}"))?;
    if !allowed_scopes.contains(&namespace) {
        anyhow::bail!("revision is outside the authorized PCP scopes");
    }
    Ok(())
}

fn ensure_provenance_access(
    transaction: &Transaction<'_>,
    provenance: &[ProvenanceEvent],
    allowed_scopes: &HashSet<String>,
) -> Result<()> {
    let mut checked = HashSet::new();
    for revision_id in provenance
        .iter()
        .flat_map(|event| event.input_revision_ids.iter())
    {
        if checked.insert(revision_id) {
            ensure_revision_access(transaction, revision_id, allowed_scopes)?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_revision(
    transaction: &Transaction<'_>,
    page_id: &str,
    revision_id: &str,
    owner_id: &str,
    namespace: &str,
    visibility: &str,
    lifecycle_status: &str,
    created_at: &str,
    observed_at: Option<&str>,
    valid_from: Option<&str>,
    valid_to: Option<&str>,
    actor: &Actor,
    payload: Option<&PagePayload>,
    source_refs: &[SourceRef],
    facets: Option<&serde_json::Value>,
    provenance: &[ProvenanceEvent],
) -> Result<()> {
    let source_refs_json = serde_json::to_string(source_refs).context("encode PCP source refs")?;
    let facets_json = facets
        .map(serde_json::to_string)
        .transpose()
        .context("encode PCP facets")?;
    let provenance_json = serde_json::to_string(provenance).context("encode PCP provenance")?;
    transaction
        .execute(
            "
            INSERT INTO pcp_revisions (
                revision_id, page_id, owner_id, namespace, visibility,
                lifecycle_status, created_at, observed_at, valid_from, valid_to,
                actor_type, actor_id, payload_media_type, payload_content,
                source_refs_json, facets_json, provenance_json
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                ?13, ?14, ?15, ?16, ?17
            )
            ",
            params![
                revision_id,
                page_id,
                owner_id,
                namespace,
                visibility,
                lifecycle_status,
                created_at,
                observed_at,
                valid_from,
                valid_to,
                actor.actor_type.as_str(),
                actor.actor_id,
                payload.map(|value| value.media_type.as_str()),
                payload.map(|value| value.content.as_str()),
                source_refs_json,
                facets_json,
                provenance_json,
            ],
        )
        .context("insert PCP revision")?;
    for input_revision_id in provenance
        .iter()
        .flat_map(|event| event.input_revision_ids.iter())
    {
        transaction
            .execute(
                "
                INSERT OR IGNORE INTO pcp_provenance_inputs (
                    derived_revision_id, input_revision_id, created_at
                ) VALUES (?1, ?2, ?3)
                ",
                params![revision_id, input_revision_id, created_at],
            )
            .context("index PCP provenance input")?;
    }
    transaction
        .execute(
            "
            INSERT INTO pcp_revision_fts (
                revision_id, page_id, namespace, payload_content, facets_text
            ) VALUES (?1, ?2, ?3, ?4, ?5)
            ",
            params![
                revision_id,
                page_id,
                namespace,
                payload.map(|value| value.content.as_str()).unwrap_or(""),
                facets_json.as_deref().unwrap_or(""),
            ],
        )
        .context("index PCP revision")?;
    Ok(())
}

pub(crate) fn insert_relation(
    transaction: &Transaction<'_>,
    from_revision_id: &str,
    relation_type: &str,
    to_revision_id: &str,
    actor: &Actor,
    created_at: &str,
) -> Result<Relation> {
    if relation_type.trim().is_empty() || relation_type.len() > 80 {
        anyhow::bail!("relation type must contain 1-80 characters");
    }
    ensure_acyclic_derivation_relation(
        transaction,
        from_revision_id,
        relation_type.trim(),
        to_revision_id,
    )?;
    let relation_id = random_id(transaction, "rel_")?;
    transaction
        .execute(
            "
            INSERT INTO pcp_relations (
                relation_id, from_revision_id, relation_type, to_revision_id,
                actor_type, actor_id, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ",
            params![
                relation_id,
                from_revision_id,
                relation_type,
                to_revision_id,
                actor.actor_type.as_str(),
                actor.actor_id,
                created_at,
            ],
        )
        .context("insert PCP relation")?;
    Ok(Relation {
        relation_id,
        from_revision_id: from_revision_id.to_owned(),
        relation_type: relation_type.to_owned(),
        to_revision_id: to_revision_id.to_owned(),
        created_by: actor.clone(),
        created_at: created_at.to_owned(),
    })
}

fn ensure_acyclic_derivation_relation(
    transaction: &Transaction<'_>,
    from_revision_id: &str,
    relation_type: &str,
    to_revision_id: &str,
) -> Result<()> {
    if !matches!(relation_type, "aggregates" | "derived_from" | "summarizes") {
        return Ok(());
    }
    if from_revision_id == to_revision_id {
        anyhow::bail!("derivation relations cannot point to the same Revision");
    }
    let creates_cycle: bool = transaction
        .query_row(
            "
            WITH RECURSIVE derivation_edges (from_revision_id, to_revision_id) AS (
                SELECT from_revision_id, to_revision_id
                FROM pcp_relations
                WHERE relation_type IN ('aggregates', 'derived_from', 'summarizes')
                UNION
                SELECT derived_revision_id, input_revision_id
                FROM pcp_provenance_inputs
            ),
            reachable (revision_id) AS (
                SELECT ?2
                UNION
                SELECT edge.to_revision_id
                FROM reachable
                JOIN derivation_edges edge
                  ON edge.from_revision_id = reachable.revision_id
            )
            SELECT EXISTS (
                SELECT 1 FROM reachable WHERE revision_id = ?1
            )
            ",
            params![from_revision_id, to_revision_id],
            |row| row.get(0),
        )
        .context("validate PCP derivation DAG")?;
    if creates_cycle {
        anyhow::bail!("relation would introduce a cycle in the PCP derivation DAG");
    }
    Ok(())
}

fn read_relation(transaction: &Transaction<'_>, relation_id: &str) -> Result<Relation> {
    transaction
        .query_row(
            "
            SELECT relation_id, from_revision_id, relation_type, to_revision_id,
                   actor_type, actor_id, created_at
            FROM pcp_relations
            WHERE relation_id = ?1
            ",
            [relation_id],
            |row| {
                let actor_type_text: String = row.get(4)?;
                let actor_type = pcp_core::ActorType::parse(&actor_type_text).ok_or_else(|| {
                    rusqlite::Error::InvalidColumnType(
                        4,
                        "actor_type".to_owned(),
                        rusqlite::types::Type::Text,
                    )
                })?;
                Ok(Relation {
                    relation_id: row.get(0)?,
                    from_revision_id: row.get(1)?,
                    relation_type: row.get(2)?,
                    to_revision_id: row.get(3)?,
                    created_by: Actor {
                        actor_type,
                        actor_id: row.get(5)?,
                    },
                    created_at: row.get(6)?,
                })
            },
        )
        .context("read idempotent PCP relation")
}

fn lookup_write_idempotency(
    transaction: &Transaction<'_>,
    actor_id: &str,
    operation: &str,
    key: Option<&str>,
) -> Result<Option<WriteResult>> {
    let Some(key) = key else {
        return Ok(None);
    };
    transaction
        .query_row(
            "
            SELECT result_page_id, result_revision_id
            FROM pcp_idempotency
            WHERE actor_id = ?1 AND operation = ?2 AND idempotency_key = ?3
            ",
            params![actor_id, operation, key],
            |row| {
                Ok(WriteResult {
                    page_id: row.get(0)?,
                    revision_id: row.get(1)?,
                    created: false,
                })
            },
        )
        .optional()
        .context("look up PCP write idempotency")
}

#[allow(clippy::too_many_arguments)]
fn record_idempotency(
    transaction: &Transaction<'_>,
    actor_id: &str,
    operation: &str,
    key: Option<&str>,
    page_id: Option<&str>,
    revision_id: Option<&str>,
    relation_id: Option<&str>,
    created_at: &str,
) -> Result<()> {
    let Some(key) = key else {
        return Ok(());
    };
    transaction
        .execute(
            "
            INSERT INTO pcp_idempotency (
                actor_id, operation, idempotency_key, result_page_id,
                result_revision_id, result_relation_id, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ",
            params![
                actor_id,
                operation,
                key,
                page_id,
                revision_id,
                relation_id,
                created_at,
            ],
        )
        .context("record PCP idempotency key")?;
    Ok(())
}

fn complete_provenance(
    mut provenance: Vec<ProvenanceEvent>,
    operation: &str,
    actor: &Actor,
    timestamp: &str,
    input_revision_ids: Vec<String>,
) -> Result<Vec<ProvenanceEvent>> {
    if provenance.is_empty() {
        provenance.push(ProvenanceEvent {
            operation: operation.to_owned(),
            actor: actor.clone(),
            timestamp: timestamp.to_owned(),
            input_revision_ids,
            tool_or_model: None,
        });
    }
    for event in &mut provenance {
        if event
            .input_revision_ids
            .iter()
            .any(|revision_id| revision_id.trim().is_empty())
        {
            anyhow::bail!("provenance input revision ids cannot be empty");
        }
        event.input_revision_ids.sort();
        event.input_revision_ids.dedup();
        if event.input_revision_ids.len() > MAX_PROVENANCE_INPUTS_PER_EVENT {
            anyhow::bail!(
                "provenance event exceeds {MAX_PROVENANCE_INPUTS_PER_EVENT} input revisions"
            );
        }
    }
    Ok(provenance)
}

pub(crate) fn random_id(transaction: &Transaction<'_>, prefix: &str) -> Result<String> {
    let random: String = transaction
        .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))
        .context("generate PCP identifier")?;
    Ok(format!("{prefix}{random}"))
}

pub(crate) fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
