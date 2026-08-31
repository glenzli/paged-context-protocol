use std::collections::{BTreeMap, BTreeSet, HashSet};

use anyhow::{Context, Result};
use pcp_core::{ExtractTopicRequest, PagePayload, PageRevisionRef, WriteResult};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::json;

use crate::{
    store::SqlitePcpStore,
    write::{
        complete_provenance, ensure_provenance_access, insert_relation, insert_revision,
        lookup_write_idempotency, now, random_id, record_idempotency,
    },
};

pub const MAX_TOPIC_EXTRACTION_SOURCES: usize = 64;
pub const MAX_TOPIC_TITLE_CHARS: usize = 240;
pub const MAX_TOPIC_CONTENT_CHARS: usize = 6_000;

impl SqlitePcpStore {
    pub async fn extract_topic(
        &self,
        request: ExtractTopicRequest,
        allowed_scopes: Vec<String>,
    ) -> Result<WriteResult> {
        validate_request(&request)?;
        let allowed_scopes = allowed_scopes.into_iter().collect::<HashSet<_>>();
        self.run("topic extraction", move |mut connection| {
            let transaction = connection
                .transaction()
                .context("start PCP topic extraction")?;
            if let Some(existing) = lookup_write_idempotency(
                &transaction,
                &request.created_by.actor_id,
                "extract_topic",
                request.idempotency_key.as_deref(),
            )? {
                return Ok(existing);
            }

            let namespace = validate_sources(&transaction, &request.source_pages, &allowed_scopes)?;
            let refresh_target = resolve_refresh_target(
                &transaction,
                request.target_topic.as_ref(),
                &request.source_pages,
                &namespace,
                &allowed_scopes,
            )?;
            let timestamp = now();
            let page_id = match refresh_target.as_ref() {
                Some(target) => target.page_id.clone(),
                None => random_id(&transaction, "pg_")?,
            };
            let revision_id = random_id(&transaction, "rev_")?;
            let source_revision_ids = request
                .source_pages
                .iter()
                .map(|source| source.revision_id.clone())
                .collect::<Vec<_>>();
            let provenance = complete_provenance(
                request.provenance,
                "extract_topic",
                &request.created_by,
                &timestamp,
                source_revision_ids.clone(),
            )?;
            ensure_provenance_access(&transaction, &provenance, &allowed_scopes)?;

            if refresh_target.is_none() {
                transaction
                    .execute(
                        "INSERT INTO pcp_pages (
                            page_id, current_revision_id, created_at, namespace,
                            kind, mutability, lifecycle_status, updated_at
                        ) VALUES (?1, NULL, ?2, ?3, 'topic_summary',
                                  'revisioned', 'active', ?2)",
                        params![page_id, timestamp, namespace],
                    )
                    .context("create PCP topic Summary Page")?;
            }
            let payload = PagePayload {
                media_type: "text/markdown".to_owned(),
                content: request.content.trim().to_owned(),
            };
            let facets = json!({
                "topicTitle": request.title.trim(),
                "routingTier": "front",
                "sourcePageCount": request.source_pages.len(),
            });
            insert_revision(
                &transaction,
                &page_id,
                &revision_id,
                &namespace,
                "active",
                &timestamp,
                None,
                None,
                None,
                None,
                &request.created_by,
                Some(&payload),
                &[],
                Some(&facets),
                &provenance,
            )?;
            let published = match refresh_target.as_ref() {
                Some(target) => transaction.execute(
                    "UPDATE pcp_pages
                     SET current_revision_id = ?2, updated_at = ?4
                     WHERE page_id = ?1 AND current_revision_id = ?3",
                    params![page_id, revision_id, target.revision_id, timestamp],
                ),
                None => transaction.execute(
                    "UPDATE pcp_pages
                     SET current_revision_id = ?2, updated_at = ?3
                     WHERE page_id = ?1 AND current_revision_id IS NULL",
                    params![page_id, revision_id, timestamp],
                ),
            }
            .context("publish PCP topic Summary Page")?;
            anyhow::ensure!(
                published == 1,
                "Topic Page changed while publishing extraction"
            );
            transaction
                .execute(
                    "INSERT INTO pcp_topic_extractions (
                        topic_revision_id, topic_page_id, namespace, title, created_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        revision_id,
                        page_id,
                        namespace,
                        request.title.trim(),
                        timestamp
                    ],
                )
                .context("record PCP topic extraction")?;

            if refresh_target.is_some() {
                retract_topic_source_relations(
                    &transaction,
                    &page_id,
                    &request.created_by,
                    &timestamp,
                )?;
            }
            for (position, source) in request.source_pages.iter().enumerate() {
                transaction
                    .execute(
                        "INSERT INTO pcp_topic_extraction_members (
                            topic_revision_id, topic_page_id, source_revision_id,
                            source_page_id, position
                         ) VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            revision_id,
                            page_id,
                            source.revision_id,
                            source.page_id,
                            i64::try_from(position).context("topic source position exceeds i64")?,
                        ],
                    )
                    .context("record PCP topic extraction source")?;
                insert_relation(
                    &transaction,
                    &revision_id,
                    "summarizes",
                    &source.revision_id,
                    &request.created_by,
                    &timestamp,
                )?;
            }
            record_idempotency(
                &transaction,
                &request.created_by.actor_id,
                "extract_topic",
                request.idempotency_key.as_deref(),
                Some(&page_id),
                Some(&revision_id),
                None,
                &timestamp,
            )?;
            transaction
                .commit()
                .context("commit PCP topic extraction")?;
            Ok(WriteResult {
                page_id,
                revision_id,
                created: refresh_target.is_none(),
            })
        })
        .await
    }
}

#[derive(Clone)]
struct TopicRefreshTarget {
    page_id: String,
    revision_id: String,
}

fn resolve_refresh_target(
    transaction: &Transaction<'_>,
    requested_target: Option<&PageRevisionRef>,
    sources: &[PageRevisionRef],
    namespace: &str,
    allowed_scopes: &HashSet<String>,
) -> Result<Option<TopicRefreshTarget>> {
    let requested_source_ids = sources
        .iter()
        .map(|source| source.page_id.clone())
        .collect::<BTreeSet<_>>();
    let topics = current_topic_source_sets(transaction, namespace)?;
    if let Some(target) = requested_target {
        let topic_sources = topics
            .get(&target.page_id)
            .context("Topic refresh target is unavailable or superseded")?;
        anyhow::ensure!(
            topic_sources.revision_id == target.revision_id,
            "Topic refresh target is stale"
        );
        anyhow::ensure!(
            allowed_scopes.contains(namespace),
            "Topic refresh target is outside the authorized PCP scopes"
        );
        let shared = topic_sources
            .source_page_ids
            .intersection(&requested_source_ids)
            .count();
        let union = topic_sources
            .source_page_ids
            .union(&requested_source_ids)
            .count();
        anyhow::ensure!(
            shared >= 2 && shared.saturating_mul(2) >= union,
            "Topic refresh target does not substantially overlap the selected source Pages"
        );
        return Ok(Some(TopicRefreshTarget {
            page_id: target.page_id.clone(),
            revision_id: target.revision_id.clone(),
        }));
    }

    anyhow::ensure!(
        !topics
            .values()
            .any(|topic| topic.source_page_ids == requested_source_ids),
        "an active Topic already has the same logical source Pages; refresh that Topic instead"
    );
    Ok(None)
}

struct CurrentTopicSources {
    revision_id: String,
    source_page_ids: BTreeSet<String>,
}

fn current_topic_source_sets(
    transaction: &Transaction<'_>,
    namespace: &str,
) -> Result<BTreeMap<String, CurrentTopicSources>> {
    let mut statement = transaction
        .prepare(
            "SELECT page.page_id, page.current_revision_id, member.source_page_id
             FROM pcp_pages page
             JOIN pcp_topic_extractions extraction
               ON extraction.topic_revision_id = page.current_revision_id
             JOIN pcp_topic_extraction_members member
               ON member.topic_revision_id = extraction.topic_revision_id
             WHERE page.namespace = ?1
               AND page.kind = 'topic_summary'
               AND page.lifecycle_status = 'active'
               AND NOT EXISTS (
                   SELECT 1
                   FROM pcp_relations superseding
                   WHERE superseding.relation_type = 'supersedes'
                     AND superseding.to_page_id = page.page_id
                     AND NOT EXISTS (
                         SELECT 1 FROM pcp_relation_retractions retraction
                         WHERE retraction.relation_id = superseding.relation_id
                     )
               )
             ORDER BY page.page_id, member.position",
        )
        .context("prepare current Topic source lookup")?;
    let rows = statement
        .query_map([namespace], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .context("query current Topic sources")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collect current Topic sources")?;
    drop(statement);

    let mut result = BTreeMap::<String, CurrentTopicSources>::new();
    for (page_id, revision_id, source_page_id) in rows {
        let entry = result
            .entry(page_id)
            .or_insert_with(|| CurrentTopicSources {
                revision_id,
                source_page_ids: BTreeSet::new(),
            });
        entry.source_page_ids.insert(source_page_id);
    }
    Ok(result)
}

fn retract_topic_source_relations(
    transaction: &Transaction<'_>,
    topic_page_id: &str,
    actor: &pcp_core::Actor,
    timestamp: &str,
) -> Result<()> {
    let mut statement = transaction
        .prepare(
            "SELECT relation.relation_id
             FROM pcp_relations relation
             WHERE relation.from_page_id = ?1
               AND relation.relation_type = 'summarizes'
               AND NOT EXISTS (
                   SELECT 1 FROM pcp_relation_retractions retraction
                   WHERE retraction.relation_id = relation.relation_id
               )",
        )
        .context("prepare Topic source relation refresh")?;
    let relation_ids = statement
        .query_map([topic_page_id], |row| row.get::<_, String>(0))
        .context("query current Topic source relations")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collect current Topic source relations")?;
    drop(statement);
    for relation_id in relation_ids {
        transaction
            .execute(
                "INSERT INTO pcp_relation_retractions (
                    relation_id, retracted_actor_type, retracted_actor_id,
                    retracted_at, reason
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    relation_id,
                    actor.actor_type.as_str(),
                    actor.actor_id,
                    timestamp,
                    "Topic source membership was refreshed",
                ],
            )
            .context("retract previous Topic source relation")?;
    }
    Ok(())
}

fn validate_request(request: &ExtractTopicRequest) -> Result<()> {
    if let Some(target) = request.target_topic.as_ref() {
        anyhow::ensure!(
            !target.page_id.trim().is_empty() && !target.revision_id.trim().is_empty(),
            "Topic refresh target requires exact Page and Revision IDs"
        );
    }
    anyhow::ensure!(
        (2..=MAX_TOPIC_EXTRACTION_SOURCES).contains(&request.source_pages.len()),
        "PCP topic extraction requires 2-{MAX_TOPIC_EXTRACTION_SOURCES} source Pages"
    );
    let mut source_pages = HashSet::new();
    let mut source_revisions = HashSet::new();
    for source in &request.source_pages {
        anyhow::ensure!(
            !source.page_id.trim().is_empty() && !source.revision_id.trim().is_empty(),
            "PCP topic extraction sources require exact Page and Revision IDs"
        );
        anyhow::ensure!(
            source_pages.insert(source.page_id.as_str())
                && source_revisions.insert(source.revision_id.as_str()),
            "PCP topic extraction source Pages and Revisions must be unique"
        );
    }
    let title_len = request.title.trim().chars().count();
    anyhow::ensure!(
        (1..=MAX_TOPIC_TITLE_CHARS).contains(&title_len),
        "PCP topic extraction title must contain 1-{MAX_TOPIC_TITLE_CHARS} characters"
    );
    let content_len = request.content.trim().chars().count();
    anyhow::ensure!(
        (1..=MAX_TOPIC_CONTENT_CHARS).contains(&content_len),
        "PCP topic extraction content must contain 1-{MAX_TOPIC_CONTENT_CHARS} characters"
    );
    Ok(())
}

fn validate_sources(
    transaction: &Transaction<'_>,
    sources: &[PageRevisionRef],
    allowed_scopes: &HashSet<String>,
) -> Result<String> {
    let mut namespace = None;
    for source in sources {
        let (resolved_page_id, resolved_namespace, lifecycle_status, kind): (
            String,
            String,
            String,
            String,
        ) = transaction
            .query_row(
                "SELECT revision.page_id, revision.namespace, revision.lifecycle_status, page.kind
                 FROM pcp_revisions revision
                 JOIN pcp_pages page ON page.page_id = revision.page_id
                 WHERE revision.revision_id = ?1
                   AND page.current_revision_id = revision.revision_id",
                [&source.revision_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .context("read PCP topic extraction source")?
            .ok_or_else(|| anyhow::anyhow!("topic extraction source must be a current Revision"))?;
        anyhow::ensure!(
            resolved_page_id == source.page_id,
            "topic extraction source Revision does not belong to its Page"
        );
        anyhow::ensure!(
            lifecycle_status == "active",
            "topic extraction sources must be active"
        );
        anyhow::ensure!(
            kind != "topic_summary",
            "topic extraction cannot use another topic Summary as a source"
        );
        anyhow::ensure!(
            allowed_scopes.contains(&resolved_namespace),
            "topic extraction source is outside the authorized PCP scopes"
        );
        match &namespace {
            Some(existing) if existing != &resolved_namespace => {
                anyhow::bail!("topic extraction sources must belong to one Scope")
            }
            Some(_) => {}
            None => namespace = Some(resolved_namespace),
        }
    }
    namespace.context("topic extraction requires sources")
}
