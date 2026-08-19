use std::collections::HashSet;

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
            let timestamp = now();
            let page_id = random_id(&transaction, "pg_")?;
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
            transaction
                .execute(
                    "UPDATE pcp_pages
                     SET current_revision_id = ?2, updated_at = ?3
                     WHERE page_id = ?1",
                    params![page_id, revision_id, timestamp],
                )
                .context("publish PCP topic Summary Page")?;
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
                created: true,
            })
        })
        .await
    }
}

fn validate_request(request: &ExtractTopicRequest) -> Result<()> {
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
