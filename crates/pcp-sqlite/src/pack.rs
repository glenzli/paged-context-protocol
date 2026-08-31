use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use pcp_core::{
    Actor, ActorType, LifecycleStatus, PACKED_PAGE_MEDIA_TYPE, PackPagesRequest, PagePayload,
    PageRevisionRef, ProvenanceEvent, SourceRef, SourceSpan, UnpackPageRequest, WriteResult,
};
use pcp_store::UnpackPageResult;
use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    SqlitePcpStore,
    retract::retract_incident_relations,
    write::{insert_revision, lookup_write_idempotency, now, random_id, record_idempotency},
};

const MAX_PACK_INPUTS: usize = 64;
const MAX_PACK_ENTRY_GAP_SECONDS: i64 = 15 * 60;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PackedPayload {
    entries: Vec<PackedEntry>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PackedEntry {
    page_id: String,
    revision_id: String,
    source_span: SourceSpan,
    created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    valid_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    valid_to: Option<String>,
    created_by: Actor,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<PagePayload>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    source_refs: Vec<SourceRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    facets: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    provenance: Vec<ProvenanceEvent>,
}

enum PackInputContent {
    Leaf(PackedEntry),
    Anchor(Vec<PackedEntry>),
}

struct PackInput {
    exact: PageRevisionRef,
    namespace: String,
    kind: String,
    source_span: SourceSpan,
    content: PackInputContent,
}

impl PackInput {
    fn is_anchor(&self) -> bool {
        matches!(self.content, PackInputContent::Anchor(_))
    }
}

impl SqlitePcpStore {
    pub async fn unpack_page(
        &self,
        request: UnpackPageRequest,
        actor: Actor,
        allowed_scopes: Vec<String>,
    ) -> Result<UnpackPageResult> {
        let allowed_scopes = allowed_scopes.into_iter().collect::<HashSet<_>>();
        self.run("Page unpacking", move |mut connection| {
            let transaction = connection.transaction().context("start PCP Page unpacking")?;
            transaction
                .execute_batch("PRAGMA defer_foreign_keys = ON")
                .context("defer PCP unpack foreign-key checks")?;
            let (namespace, kind, payload_content) = transaction
                .query_row(
                    "SELECT page.namespace, page.kind, revision.payload_content
                     FROM pcp_pages page
                     JOIN pcp_revisions revision ON revision.revision_id = page.current_revision_id
                     WHERE page.page_id = ?1
                       AND page.current_revision_id = ?2
                       AND page.lifecycle_status = 'active'
                       AND revision.payload_media_type = ?3",
                    params![
                        request.page_id,
                        request.expected_revision_id,
                        PACKED_PAGE_MEDIA_TYPE,
                    ],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()
                .context("find exact current packed PCP Page")?
                .context("PCP unpack requires an exact active packed Page")?;
            anyhow::ensure!(
                allowed_scopes.contains(&namespace),
                "PCP unpack input is outside the authorized Scopes"
            );
            let packed: PackedPayload = serde_json::from_str(&payload_content)
                .context("decode PCP packed Page for unpack")?;
            anyhow::ensure!(
                packed.entries.len() >= 2,
                "PCP unpack requires at least two packed entries"
            );
            let source_span = SourceSpan {
                stream_id: packed.entries[0].source_span.stream_id.clone(),
                start: packed.entries[0].source_span.start,
                end: packed
                    .entries
                    .last()
                    .expect("packed entry count was validated")
                    .source_span
                    .end,
            };
            ensure_flat_entries(&packed.entries, &source_span)?;
            for entry in &packed.entries {
                let exists: bool = transaction
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM pcp_pages WHERE page_id = ?1)",
                        [&entry.page_id],
                        |row| row.get(0),
                    )
                    .context("check restored PCP Page identity")?;
                anyhow::ensure!(
                    !exists,
                    "PCP unpack cannot overwrite existing source Page {}",
                    entry.page_id
                );
            }

            for entry in &packed.entries {
                transaction
                    .execute(
                        "INSERT INTO pcp_pages (
                            page_id, current_revision_id, created_at, namespace,
                            kind, mutability, lifecycle_status, updated_at
                         ) VALUES (?1, NULL, ?2, ?3, ?4, 'sealed', 'active', ?5)",
                        params![entry.page_id, entry.created_at, namespace, kind, now()],
                    )
                    .context("restore packed PCP Page identity")?;
            }
            let restored_revision_ids = packed
                .entries
                .iter()
                .map(|entry| entry.revision_id.as_str())
                .collect::<HashSet<_>>();
            for entry in &packed.entries {
                // A historical Pack can carry provenance references to Pages
                // that were independently collected or superseded before this
                // repair. Keep the signed event, but do not recreate a broken
                // provenance edge merely because the Pack happened to retain
                // its old identifier string.
                let mut provenance = entry.provenance.clone();
                for event in &mut provenance {
                    let input_revision_ids = std::mem::take(&mut event.input_revision_ids);
                    for revision_id in input_revision_ids {
                        let exists = restored_revision_ids.contains(revision_id.as_str())
                            || transaction
                                .query_row(
                                    "SELECT EXISTS(SELECT 1 FROM pcp_revisions WHERE revision_id = ?1)",
                                    [&revision_id],
                                    |row| row.get::<_, bool>(0),
                                )
                                .context("check restored provenance input")?;
                        if exists {
                            event.input_revision_ids.push(revision_id);
                        }
                    }
                }
                insert_revision(
                    &transaction,
                    &entry.page_id,
                    &entry.revision_id,
                    &namespace,
                    LifecycleStatus::Active.as_str(),
                    &entry.created_at,
                    entry.observed_at.as_deref(),
                    Some(&entry.source_span),
                    entry.valid_from.as_deref(),
                    entry.valid_to.as_deref(),
                    &entry.created_by,
                    entry.payload.as_ref(),
                    &entry.source_refs,
                    entry.facets.as_ref(),
                    &provenance,
                )?;
                transaction
                    .execute(
                        "UPDATE pcp_pages SET current_revision_id = ?2 WHERE page_id = ?1",
                        params![entry.page_id, entry.revision_id],
                    )
                    .context("publish restored PCP Page")?;
            }
            transaction
                .execute(
                    "DELETE FROM pcp_page_packs WHERE packed_page_id = ?1",
                    [&request.page_id],
                )
                .context("remove unpacked PCP Page memberships")?;

            let timestamp = now();
            let retracted_relation_ids = retract_incident_relations(
                &transaction,
                &[request.page_id.clone()],
                &actor,
                &timestamp,
                "packed topic was manually split; old relation endpoint is ambiguous",
            )?;
            let tombstone_revision_id = random_id(&transaction, "rev_")?;
            insert_revision(
                &transaction,
                &request.page_id,
                &tombstone_revision_id,
                &namespace,
                LifecycleStatus::Tombstoned.as_str(),
                &timestamp,
                Some(&timestamp),
                None,
                None,
                None,
                &actor,
                None,
                &[],
                Some(&serde_json::json!({
                    "kind": "tombstone",
                    "unpackedRevisionId": request.expected_revision_id,
                })),
                &[ProvenanceEvent {
                    operation: "unpack".to_owned(),
                    actor: actor.clone(),
                    timestamp: timestamp.clone(),
                    input_revision_ids: vec![request.expected_revision_id.clone()],
                    tool_or_model: Some(actor.actor_id.clone()),
                    reason: None,
                }],
            )?;
            let published = transaction
                .execute(
                    "UPDATE pcp_pages
                     SET current_revision_id = ?2, kind = 'tombstone', lifecycle_status = 'tombstoned', updated_at = ?3
                     WHERE page_id = ?1 AND current_revision_id = ?4",
                    params![
                        request.page_id,
                        tombstone_revision_id,
                        timestamp,
                        request.expected_revision_id,
                    ],
                )
                .context("retire unpacked PCP Page")?;
            anyhow::ensure!(published == 1, "packed PCP Page changed during unpack");
            let restored_pages = packed
                .entries
                .into_iter()
                .map(|entry| PageRevisionRef {
                    page_id: entry.page_id,
                    revision_id: entry.revision_id,
                })
                .collect();
            transaction.commit().context("commit PCP Page unpacking")?;
            Ok(UnpackPageResult {
                retired_page_id: request.page_id,
                retired_revision_id: tombstone_revision_id,
                restored_pages,
                retracted_relation_ids,
            })
        })
        .await
    }

    pub async fn pack_pages(
        &self,
        request: PackPagesRequest,
        actor: Actor,
        allowed_scopes: Vec<String>,
    ) -> Result<WriteResult> {
        anyhow::ensure!(
            (2..=MAX_PACK_INPUTS).contains(&request.pages.len()),
            "PCP packing requires 2-{MAX_PACK_INPUTS} Pages"
        );
        let allowed_scopes = allowed_scopes.into_iter().collect::<HashSet<_>>();
        self.run("Page packing", move |mut connection| {
            let transaction = connection.transaction().context("start PCP Page packing")?;
            if let Some(existing) = lookup_write_idempotency(
                &transaction,
                &actor.actor_id,
                "pack_pages",
                request.idempotency_key.as_deref(),
            )? {
                return Ok(existing);
            }

            let mut seen_pages = HashSet::new();
            let mut seen_revisions = HashSet::new();
            let mut inputs = Vec::with_capacity(request.pages.len());
            for exact in &request.pages {
                anyhow::ensure!(
                    seen_pages.insert(exact.page_id.clone())
                        && seen_revisions.insert(exact.revision_id.clone()),
                    "PCP packing inputs must be unique"
                );
                inputs.push(read_pack_input(&transaction, exact)?);
            }

            let anchor_count = inputs.iter().filter(|input| input.is_anchor()).count();
            anyhow::ensure!(
                anchor_count <= 2,
                "PCP packing accepts at most two existing packed Pages"
            );
            let namespace = inputs[0].namespace.clone();
            let kind = inputs[0].kind.clone();
            anyhow::ensure!(
                allowed_scopes.contains(&namespace),
                "PCP packing input is outside the authorized Scopes"
            );
            anyhow::ensure!(
                inputs
                    .iter()
                    .all(|input| input.namespace == namespace && input.kind == kind),
                "PCP packing inputs must share Scope and kind"
            );
            for input in &inputs {
                anyhow::ensure!(
                    allowed_scopes.contains(&input.namespace),
                    "PCP packing input is outside the authorized Scopes"
                );
                if !input.is_anchor() {
                    ensure_packable_leaf(&transaction, &input.exact.revision_id)?;
                }
            }

            ensure_contiguous_spans(inputs.iter().map(|input| &input.source_span))?;
            let source_span = SourceSpan {
                stream_id: inputs[0].source_span.stream_id.clone(),
                start: inputs[0].source_span.start,
                end: inputs
                    .last()
                    .expect("packing input count was validated")
                    .source_span
                    .end,
            };
            let anchors = inputs
                .iter()
                .filter_map(|input| match &input.content {
                    PackInputContent::Anchor(entries) => Some((input.exact.clone(), entries.len())),
                    PackInputContent::Leaf(_) => None,
                })
                .collect::<Vec<_>>();
            let anchor = anchors.first().map(|(exact, _)| exact.clone());
            let leaf_refs = inputs
                .iter()
                .filter(|input| !input.is_anchor())
                .map(|input| input.exact.clone())
                .collect::<Vec<_>>();
            let mut entries = Vec::new();
            for input in inputs {
                match input.content {
                    PackInputContent::Leaf(entry) => entries.push(entry),
                    PackInputContent::Anchor(anchor_entries) => entries.extend(anchor_entries),
                }
            }
            ensure_flat_entries(&entries, &source_span)?;
            ensure_continuous_entry_times(&entries)?;
            let observed_at = entries.last().and_then(|entry| entry.observed_at.clone());
            let entry_positions = entries
                .iter()
                .enumerate()
                .map(|(position, entry)| (entry.page_id.clone(), position as i64))
                .collect::<HashMap<_, _>>();
            let payload = PagePayload {
                media_type: PACKED_PAGE_MEDIA_TYPE.to_owned(),
                content: serde_json::to_string(&PackedPayload { entries })
                    .context("encode PCP packed Page")?,
            };
            anyhow::ensure!(
                payload.content.chars().count() <= crate::store::MAX_PAGE_CHARS,
                "PCP packed payload exceeds the Page character limit"
            );

            let timestamp = now();
            let packed_revision_id = random_id(&transaction, "rev_")?;
            let packed_page_id = if let Some(anchor) = &anchor {
                anchor.page_id.clone()
            } else {
                let packed_page_id = random_id(&transaction, "pg_")?;
                transaction
                    .execute(
                        "
                        INSERT INTO pcp_pages (
                            page_id, current_revision_id, created_at, namespace,
                            kind, mutability, lifecycle_status, updated_at
                        ) VALUES (?1, NULL, ?2, ?3, ?4, 'revisioned', 'active', ?2)
                        ",
                        params![packed_page_id, timestamp, namespace, kind],
                    )
                    .context("create packed PCP Page")?;
                packed_page_id
            };
            insert_revision(
                &transaction,
                &packed_page_id,
                &packed_revision_id,
                &namespace,
                LifecycleStatus::Active.as_str(),
                &timestamp,
                observed_at.as_deref(),
                Some(&source_span),
                None,
                None,
                &actor,
                Some(&payload),
                &[],
                None,
                &[],
            )?;
            let published = if let Some(anchor) = &anchor {
                transaction
                    .execute(
                        "
                        UPDATE pcp_pages
                        SET current_revision_id = ?2, lifecycle_status = 'active', updated_at = ?3
                        WHERE page_id = ?1 AND current_revision_id = ?4
                        ",
                        params![
                            packed_page_id,
                            packed_revision_id,
                            timestamp,
                            anchor.revision_id,
                        ],
                    )
                    .context("publish extended packed PCP Page")?
            } else {
                transaction
                    .execute(
                        "UPDATE pcp_pages SET current_revision_id = ?2 WHERE page_id = ?1",
                        params![packed_page_id, packed_revision_id],
                    )
                    .context("publish packed PCP Page")?
            };
            anyhow::ensure!(
                published == 1,
                "packed PCP Page head changed during packing"
            );

            for (anchor, entry_count) in &anchors {
                let mut updated_members = 0;
                for (source_page_id, position) in &entry_positions {
                    updated_members += transaction
                        .execute(
                            "
                            UPDATE pcp_page_packs
                            SET packed_page_id = ?3, packed_revision_id = ?4, position = ?5
                            WHERE source_page_id = ?1 AND packed_page_id = ?2
                            ",
                            params![source_page_id, anchor.page_id, packed_page_id, packed_revision_id, position],
                        )
                        .context("reindex existing packed PCP Page input")?;
                }
                anyhow::ensure!(
                    updated_members == *entry_count,
                    "packed PCP Page membership changed during repacking"
                );
            }

            for exact in &leaf_refs {
                let position = entry_positions[&exact.page_id];
                transaction
                    .execute(
                        "
                        INSERT INTO pcp_page_packs (
                            source_page_id, source_revision_id, namespace,
                            packed_page_id, packed_revision_id, position, packed_at
                        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                        ",
                        params![
                            exact.page_id,
                            exact.revision_id,
                            namespace,
                            packed_page_id,
                            packed_revision_id,
                            position,
                            timestamp,
                        ],
                    )
                    .context("record packed PCP Page input")?;
                transaction
                    .execute(
                        "
                        UPDATE pcp_idempotency
                        SET result_page_id = ?3, result_revision_id = ?4
                        WHERE result_page_id = ?1 OR result_revision_id = ?2
                        ",
                        params![
                            exact.page_id,
                            exact.revision_id,
                            packed_page_id,
                            packed_revision_id,
                        ],
                    )
                    .context("redirect PCP ingest idempotency to packed Page")?;
                transaction
                    .execute(
                        "DELETE FROM pcp_revision_fts WHERE revision_id = ?1",
                        [&exact.revision_id],
                    )
                    .context("remove packed input from PCP search index")?;
            }
            // When two Pack Pages are merged, the first remains the active
            // anchor and the other remains readable as superseded history. Its
            // members now resolve to the new anchor above; keeping the old Page
            // avoids rewriting its existing summaries and asserted relations as
            // though they described the larger, newly merged episode.
            for (absorbed, _) in anchors.iter().skip(1) {
                transaction
                    .execute(
                        "UPDATE pcp_pages SET lifecycle_status = 'superseded', updated_at = ?2 WHERE page_id = ?1",
                        params![absorbed.page_id, timestamp],
                    )
                    .context("supersede absorbed packed PCP Page")?;
                transaction
                    .execute(
                        "DELETE FROM pcp_revision_fts WHERE revision_id = ?1",
                        [&absorbed.revision_id],
                    )
                    .context("remove absorbed packed Page from PCP search index")?;
            }
            rewrite_pack_references(
                &transaction,
                &leaf_refs,
                &packed_page_id,
                &packed_revision_id,
            )?;
            for exact in &leaf_refs {
                transaction
                    .execute(
                        "DELETE FROM pcp_revisions WHERE revision_id = ?1",
                        [&exact.revision_id],
                    )
                    .context("remove packed PCP Revision")?;
                transaction
                    .execute("DELETE FROM pcp_pages WHERE page_id = ?1", [&exact.page_id])
                    .context("remove packed PCP Page")?;
            }
            record_idempotency(
                &transaction,
                &actor.actor_id,
                "pack_pages",
                request.idempotency_key.as_deref(),
                Some(&packed_page_id),
                Some(&packed_revision_id),
                None,
                &timestamp,
            )?;
            transaction.commit().context("commit PCP Page packing")?;
            Ok(WriteResult {
                page_id: packed_page_id,
                revision_id: packed_revision_id,
                created: true,
            })
        })
        .await
    }
}

fn read_pack_input(
    transaction: &Transaction<'_>,
    exact: &pcp_core::PageRevisionRef,
) -> Result<PackInput> {
    transaction
        .query_row(
            "
            SELECT page.namespace, page.kind, page.mutability, page.lifecycle_status,
                   page.current_revision_id, revision.previous_revision_id,
                   revision.created_at, revision.observed_at, revision.source_span_json,
                   revision.valid_from, revision.valid_to, revision.actor_type,
                   revision.actor_id, revision.payload_media_type, revision.payload_content,
                   revision.source_refs_json, revision.facets_json, revision.provenance_json
            FROM pcp_pages page
            JOIN pcp_revisions revision ON revision.page_id = page.page_id
            WHERE page.page_id = ?1 AND revision.revision_id = ?2
            ",
            params![exact.page_id, exact.revision_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, String>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                    row.get::<_, String>(15)?,
                    row.get::<_, Option<String>>(16)?,
                    row.get::<_, String>(17)?,
                ))
            },
        )
        .with_context(|| format!("find exact PCP Page {}", exact.page_id))
        .and_then(
            |(
                namespace,
                kind,
                mutability,
                lifecycle,
                current_revision_id,
                previous_revision_id,
                created_at,
                observed_at,
                source_span_json,
                valid_from,
                valid_to,
                actor_type,
                actor_id,
                payload_media_type,
                payload_content,
                source_refs_json,
                facets_json,
                provenance_json,
            )| {
                anyhow::ensure!(
                    lifecycle == "active",
                    "PCP packing accepts only active Pages"
                );
                anyhow::ensure!(
                    current_revision_id == exact.revision_id,
                    "PCP packing requires each exact current Revision"
                );
                let source_span = source_span_json
                    .context("PCP packing input has no sourceSpan")
                    .and_then(|value| {
                        serde_json::from_str(&value).context("decode PCP packing sourceSpan")
                    })?;
                let content = if payload_media_type.as_deref() == Some(PACKED_PAGE_MEDIA_TYPE) {
                    anyhow::ensure!(
                        mutability == "revisioned",
                        "PCP packed Pages must be revisioned"
                    );
                    let payload_content =
                        payload_content.context("PCP packed Page has no payload")?;
                    let packed: PackedPayload = serde_json::from_str(&payload_content)
                        .context("decode existing packed PCP Page")?;
                    ensure_flat_entries(&packed.entries, &source_span)?;
                    PackInputContent::Anchor(packed.entries)
                } else {
                    anyhow::ensure!(
                        mutability == "sealed",
                        "PCP packing accepts only sealed leaves or one packed Page"
                    );
                    anyhow::ensure!(
                        previous_revision_id.is_none(),
                        "PCP packing accepts only single-Revision sealed leaves"
                    );
                    let provenance = serde_json::from_str::<Vec<ProvenanceEvent>>(&provenance_json)
                        .context("decode PCP packing input provenance")?;
                    let actor_type = ActorType::parse(&actor_type)
                        .with_context(|| format!("unknown PCP actor type {actor_type}"))?;
                    let payload =
                        payload_media_type
                            .zip(payload_content)
                            .map(|(media_type, content)| PagePayload {
                                media_type,
                                content,
                            });
                    PackInputContent::Leaf(PackedEntry {
                        page_id: exact.page_id.clone(),
                        revision_id: exact.revision_id.clone(),
                        source_span: source_span.clone(),
                        created_at,
                        observed_at,
                        valid_from,
                        valid_to,
                        created_by: Actor {
                            actor_type,
                            actor_id,
                        },
                        payload,
                        source_refs: serde_json::from_str(&source_refs_json)
                            .context("decode PCP packing sourceRefs")?,
                        facets: facets_json
                            .map(|value| serde_json::from_str(&value))
                            .transpose()
                            .context("decode PCP packing facets")?,
                        provenance,
                    })
                };
                Ok(PackInput {
                    exact: exact.clone(),
                    namespace,
                    kind,
                    source_span,
                    content,
                })
            },
        )
}

fn ensure_contiguous_spans<'a>(spans: impl Iterator<Item = &'a SourceSpan>) -> Result<()> {
    let spans = spans.collect::<Vec<_>>();
    for pair in spans.windows(2) {
        anyhow::ensure!(
            pair[0].stream_id == pair[1].stream_id
                && pair[0].end.checked_add(1) == Some(pair[1].start),
            "PCP packing inputs must be contiguous and ordered in one source stream"
        );
    }
    Ok(())
}

fn ensure_flat_entries(entries: &[PackedEntry], outer_span: &SourceSpan) -> Result<()> {
    anyhow::ensure!(entries.len() >= 2, "PCP packed Page has too few entries");
    let mut page_ids = HashSet::new();
    let mut revision_ids = HashSet::new();
    for entry in entries {
        anyhow::ensure!(
            page_ids.insert(&entry.page_id) && revision_ids.insert(&entry.revision_id),
            "PCP packed Page contains duplicate entries"
        );
        anyhow::ensure!(
            entry
                .payload
                .as_ref()
                .map(|payload| payload.media_type.as_str())
                != Some(PACKED_PAGE_MEDIA_TYPE),
            "PCP packed payload entries must remain flat"
        );
    }
    ensure_contiguous_spans(entries.iter().map(|entry| &entry.source_span))?;
    let first = &entries[0].source_span;
    let last = &entries
        .last()
        .expect("packed entry count was validated")
        .source_span;
    anyhow::ensure!(
        outer_span.stream_id == first.stream_id
            && outer_span.start == first.start
            && outer_span.end == last.end,
        "PCP packed Page sourceSpan does not match its flat entries"
    );
    Ok(())
}

fn ensure_continuous_entry_times(entries: &[PackedEntry]) -> Result<()> {
    let entry_times = entries
        .iter()
        .map(|entry| {
            let timestamp = entry.observed_at.as_deref().unwrap_or(&entry.created_at);
            DateTime::parse_from_rfc3339(timestamp)
                .map(|value| value.with_timezone(&Utc))
                .with_context(|| format!("decode PCP packed entry timestamp for {}", entry.page_id))
        })
        .collect::<Result<Vec<_>>>()?;
    for pair in entry_times.windows(2) {
        let gap = pair[1].signed_duration_since(pair[0]).num_seconds();
        anyhow::ensure!(
            (0..=MAX_PACK_ENTRY_GAP_SECONDS).contains(&gap),
            "PCP packing entries must form one time-continuous episode (maximum gap {MAX_PACK_ENTRY_GAP_SECONDS}s)"
        );
    }
    Ok(())
}

fn ensure_packable_leaf(transaction: &Transaction<'_>, revision_id: &str) -> Result<()> {
    let referenced: bool = transaction
        .query_row(
            "
            SELECT EXISTS (
                SELECT 1 FROM pcp_summaries
                WHERE target_revision_id = ?1 OR summary_revision_id = ?1
                UNION ALL
                SELECT 1 FROM pcp_summary_assessments WHERE target_revision_id = ?1
                UNION ALL
                SELECT 1 FROM pcp_validity_assessments
                WHERE target_revision_id = ?1 OR assessment_revision_id = ?1
                UNION ALL
                SELECT 1 FROM pcp_revision_retention_leases WHERE revision_id = ?1
            )
            ",
            [revision_id],
            |row| row.get(0),
        )
        .context("check PCP packing identity pins")?;
    anyhow::ensure!(
        !referenced,
        "PCP packing input is explicitly retained by Summary, Validity, or retention policy"
    );
    Ok(())
}

fn rewrite_pack_references(
    transaction: &Transaction<'_>,
    leaf_refs: &[PageRevisionRef],
    packed_page_id: &str,
    packed_revision_id: &str,
) -> Result<()> {
    let leaf_page_ids = leaf_refs
        .iter()
        .map(|exact| exact.page_id.as_str())
        .collect::<HashSet<_>>();
    let leaf_revision_ids = leaf_refs
        .iter()
        .map(|exact| exact.revision_id.as_str())
        .collect::<HashSet<_>>();

    let relations = {
        let mut statement = transaction
            .prepare(
                "SELECT relation_id, relation_type, from_page_id, to_page_id,
                        basis_revision_ids_json
                 FROM pcp_relations",
            )
            .context("prepare PCP relation packing rewrites")?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .context("query PCP relation packing rewrites")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("collect PCP relation packing rewrites")?
    };
    for (relation_id, relation_type, from_page_id, to_page_id, basis_json) in relations {
        let endpoint_changed = leaf_page_ids.contains(from_page_id.as_str())
            || leaf_page_ids.contains(to_page_id.as_str());
        let mut basis = serde_json::from_str::<Vec<String>>(&basis_json)
            .context("decode PCP relation basis for packing")?;
        let mut basis_changed = false;
        for revision_id in &mut basis {
            if leaf_revision_ids.contains(revision_id.as_str()) {
                *revision_id = packed_revision_id.to_owned();
                basis_changed = true;
            }
        }
        if !endpoint_changed && !basis_changed {
            continue;
        }
        basis.sort();
        basis.dedup();
        let mut rewritten_from = if leaf_page_ids.contains(from_page_id.as_str()) {
            packed_page_id.to_owned()
        } else {
            from_page_id
        };
        let mut rewritten_to = if leaf_page_ids.contains(to_page_id.as_str()) {
            packed_page_id.to_owned()
        } else {
            to_page_id
        };
        if rewritten_from == rewritten_to {
            transaction
                .execute(
                    "DELETE FROM pcp_relation_retractions WHERE relation_id = ?1",
                    [&relation_id],
                )
                .context("remove internal packed relation retraction")?;
            transaction
                .execute(
                    "DELETE FROM pcp_relations WHERE relation_id = ?1",
                    [&relation_id],
                )
                .context("remove internal packed relation")?;
            transaction
                .execute(
                    "DELETE FROM pcp_idempotency
                     WHERE operation = 'link_pages' AND result_relation_id = ?1",
                    [&relation_id],
                )
                .context("remove internal packed relation idempotency")?;
            continue;
        }
        if relation_type == "related_to" && rewritten_from > rewritten_to {
            std::mem::swap(&mut rewritten_from, &mut rewritten_to);
        }
        let basis_json = serde_json::to_string(&basis).context("encode packed relation basis")?;
        transaction
            .execute(
                "UPDATE pcp_relations
                 SET from_page_id = ?2, to_page_id = ?3, basis_revision_ids_json = ?4
                 WHERE relation_id = ?1",
                params![relation_id, rewritten_from, rewritten_to, basis_json],
            )
            .context("rewrite external PCP relation for packed Page")?;
    }

    let provenance = {
        let mut statement = transaction
            .prepare(
                "SELECT derived_revision_id, input_revision_id, created_at
                 FROM pcp_provenance_inputs",
            )
            .context("prepare PCP provenance packing rewrites")?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .context("query PCP provenance packing rewrites")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("collect PCP provenance packing rewrites")?
    };
    for (derived_revision_id, input_revision_id, created_at) in provenance {
        let derived_changed = leaf_revision_ids.contains(derived_revision_id.as_str());
        let input_changed = leaf_revision_ids.contains(input_revision_id.as_str());
        if !derived_changed && !input_changed {
            continue;
        }
        let rewritten_derived = if derived_changed {
            packed_revision_id
        } else {
            derived_revision_id.as_str()
        };
        let rewritten_input = if input_changed {
            packed_revision_id
        } else {
            input_revision_id.as_str()
        };
        if rewritten_derived != rewritten_input {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO pcp_provenance_inputs (
                        derived_revision_id, input_revision_id, created_at
                     ) VALUES (?1, ?2, ?3)",
                    params![rewritten_derived, rewritten_input, created_at],
                )
                .context("rewrite external PCP provenance for packed Revision")?;
        }
        transaction
            .execute(
                "DELETE FROM pcp_provenance_inputs
                 WHERE derived_revision_id = ?1 AND input_revision_id = ?2",
                params![derived_revision_id, input_revision_id],
            )
            .context("remove superseded PCP provenance input")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use pcp_core::{Actor, ActorType, SourceSpan};

    use super::{PackedEntry, ensure_continuous_entry_times};

    fn entry(id: &str, observed_at: &str) -> PackedEntry {
        PackedEntry {
            page_id: format!("pg_{id}"),
            revision_id: format!("rev_{id}"),
            source_span: SourceSpan {
                stream_id: "conversation:test".to_owned(),
                start: 0,
                end: 0,
            },
            created_at: observed_at.to_owned(),
            observed_at: Some(observed_at.to_owned()),
            valid_from: None,
            valid_to: None,
            created_by: Actor {
                actor_type: ActorType::User,
                actor_id: "test".to_owned(),
            },
            payload: None,
            source_refs: Vec::new(),
            facets: None,
            provenance: Vec::new(),
        }
    }

    #[test]
    fn packed_episode_uses_every_entry_timestamp_not_container_boundaries() {
        let error = ensure_continuous_entry_times(&[
            entry("first", "2026-08-19T00:00:00Z"),
            entry("middle", "2026-08-19T00:05:00Z"),
            entry("last", "2026-08-19T00:25:01Z"),
        ])
        .expect_err("a gap within an existing Pack must prevent merging it");

        assert!(format!("{error:#}").contains("time-continuous episode"));
    }
}
