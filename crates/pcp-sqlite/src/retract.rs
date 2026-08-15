use std::collections::{BTreeSet, HashSet};

use anyhow::{Context, Result};
use pcp_core::{Actor, LifecycleStatus, PageRevision, ProvenanceEvent};
use pcp_store::TombstoneCascadeResult;
use rusqlite::{OptionalExtension, params};
use serde_json::json;

use crate::{
    row::{REVISION_COLUMNS, revision_from_row},
    store::SqlitePcpStore,
    write::{insert_revision, now, random_id},
};

const MAX_CASCADE_REVISIONS: usize = 1_000;

impl SqlitePcpStore {
    pub async fn tombstone_derivation_cascade(
        &self,
        root_revision_id: String,
        actor: Actor,
        allowed_scopes: Vec<String>,
    ) -> Result<TombstoneCascadeResult> {
        let allowed_scopes = allowed_scopes.into_iter().collect::<HashSet<_>>();
        self.run("derivation cascade tombstone", move |mut connection| {
            let transaction = connection
                .transaction()
                .context("start PCP derivation retraction")?;
            let root_namespace = transaction
                .query_row(
                    "
                    SELECT current.namespace
                    FROM pcp_pages page
                    JOIN pcp_revisions current
                      ON current.revision_id = page.current_revision_id
                    WHERE current.revision_id = ?1
                      AND current.lifecycle_status <> 'tombstoned'
                    ",
                    [&root_revision_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .context("find current PCP retraction root")?
                .context("PCP retraction root is not an active current Revision")?;
            if !allowed_scopes.contains(&root_namespace) {
                anyhow::bail!("PCP retraction root is outside the authorized scopes");
            }

            let affected_revision_ids = downstream_revision_ids(&transaction, &root_revision_id)?;
            if affected_revision_ids.len() > MAX_CASCADE_REVISIONS {
                anyhow::bail!(
                    "PCP retraction exceeds the {MAX_CASCADE_REVISIONS}-Revision safety limit"
                );
            }
            let affected = affected_revision_ids
                .iter()
                .cloned()
                .collect::<HashSet<_>>();
            let page_ids = affected_page_ids(&transaction, &affected_revision_ids)?;
            let timestamp = now();
            let mut retracted_revision_ids = Vec::new();
            let mut restored_page_ids = Vec::new();
            let mut tombstone_revision_ids = Vec::new();

            for page_id in page_ids {
                let (current, current_kind) = current_revision(&transaction, &page_id)?;
                if current.lifecycle_status == LifecycleStatus::Tombstoned {
                    continue;
                }
                if !allowed_scopes.contains(&current.namespace) {
                    anyhow::bail!("PCP retraction dependency is outside the authorized scopes");
                }
                let current_revision_id = current.revision_id.clone();
                retracted_revision_ids.push(current_revision_id.clone());

                let fallback = latest_unaffected_revision(&transaction, &page_id, &affected)?;
                let next_revision_id = random_id(&transaction, "rev_")?;
                let (next_kind, next_lifecycle_status) = match fallback {
                    Some(fallback) => {
                        let next_kind = fallback
                            .facets
                            .as_ref()
                            .and_then(|facets| facets.get("kind"))
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned)
                            .unwrap_or_else(|| current_kind.clone());
                        let next_lifecycle_status = fallback.lifecycle_status.clone();
                        insert_revision(
                            &transaction,
                            &page_id,
                            &next_revision_id,
                            &current.namespace,
                            fallback.lifecycle_status.as_str(),
                            &timestamp,
                            Some(&timestamp),
                            fallback.source_span.as_ref(),
                            fallback.valid_from.as_deref(),
                            fallback.valid_to.as_deref(),
                            &actor,
                            fallback.payload.as_ref(),
                            &fallback.source_refs,
                            fallback.facets.as_ref(),
                            &[ProvenanceEvent {
                                operation: "retract_restore".to_owned(),
                                actor: actor.clone(),
                                timestamp: timestamp.clone(),
                                input_revision_ids: vec![fallback.revision_id],
                                tool_or_model: Some(actor.actor_id.clone()),
                            }],
                        )?;
                        restored_page_ids.push(page_id.clone());
                        (next_kind, next_lifecycle_status)
                    }
                    None => {
                        insert_revision(
                            &transaction,
                            &page_id,
                            &next_revision_id,
                            &current.namespace,
                            LifecycleStatus::Tombstoned.as_str(),
                            &timestamp,
                            Some(&timestamp),
                            None,
                            None,
                            None,
                            &actor,
                            None,
                            &[],
                            Some(&json!({
                                "kind": "tombstone",
                                "retractedRevisionId": current_revision_id.clone(),
                                "rootRevisionId": root_revision_id
                            })),
                            &[ProvenanceEvent {
                                operation: "retract".to_owned(),
                                actor: actor.clone(),
                                timestamp: timestamp.clone(),
                                input_revision_ids: vec![current_revision_id.clone()],
                                tool_or_model: Some(actor.actor_id.clone()),
                            }],
                        )?;
                        tombstone_revision_ids.push(next_revision_id.clone());
                        ("tombstone".to_owned(), LifecycleStatus::Tombstoned)
                    }
                };
                let published = transaction
                    .execute(
                        "
                        UPDATE pcp_pages
                        SET current_revision_id = ?2,
                            kind = ?4,
                            lifecycle_status = ?5,
                            updated_at = ?6
                        WHERE page_id = ?1 AND current_revision_id = ?3
                        ",
                        params![
                            page_id,
                            next_revision_id,
                            current_revision_id,
                            next_kind,
                            next_lifecycle_status.as_str(),
                            timestamp
                        ],
                    )
                    .context("publish PCP retraction revision")?;
                anyhow::ensure!(published == 1, "PCP Page changed during retraction");
            }

            transaction
                .commit()
                .context("commit PCP derivation retraction")?;
            Ok(TombstoneCascadeResult {
                retracted_revision_ids,
                restored_page_ids,
                tombstone_revision_ids,
            })
        })
        .await
    }
}

fn downstream_revision_ids(
    transaction: &rusqlite::Transaction<'_>,
    root_revision_id: &str,
) -> Result<Vec<String>> {
    let mut statement = transaction
        .prepare(
            "
            WITH RECURSIVE downstream (revision_id) AS (
                SELECT ?1
                UNION
                SELECT provenance.derived_revision_id
                FROM pcp_provenance_inputs provenance
                JOIN downstream
                  ON provenance.input_revision_id = downstream.revision_id
            )
            SELECT revision_id FROM downstream
            ",
        )
        .context("prepare PCP downstream traversal")?;
    statement
        .query_map([root_revision_id], |row| row.get(0))
        .context("query PCP downstream traversal")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collect PCP downstream traversal")
}

fn affected_page_ids(
    transaction: &rusqlite::Transaction<'_>,
    revision_ids: &[String],
) -> Result<Vec<String>> {
    let mut page_ids = BTreeSet::new();
    let mut statement = transaction
        .prepare("SELECT page_id FROM pcp_revisions WHERE revision_id = ?1")
        .context("prepare affected PCP Page lookup")?;
    for revision_id in revision_ids {
        page_ids.insert(
            statement
                .query_row([revision_id], |row| row.get::<_, String>(0))
                .with_context(|| format!("find Page for affected Revision {revision_id}"))?,
        );
    }
    Ok(page_ids.into_iter().collect())
}

fn current_revision(
    transaction: &rusqlite::Transaction<'_>,
    page_id: &str,
) -> Result<(PageRevision, String)> {
    let sql = format!(
        "SELECT {REVISION_COLUMNS}, page.kind
         FROM pcp_pages page
         JOIN pcp_revisions r ON r.revision_id = page.current_revision_id
         WHERE page.page_id = ?1"
    );
    transaction
        .query_row(&sql, [page_id], |row| {
            Ok((
                revision_from_row(row, true, true, true, true).map_err(to_sql_error)?,
                row.get(17)?,
            ))
        })
        .context("read current PCP Revision for retraction")
}

fn latest_unaffected_revision(
    transaction: &rusqlite::Transaction<'_>,
    page_id: &str,
    affected: &HashSet<String>,
) -> Result<Option<PageRevision>> {
    let sql = format!(
        "SELECT {REVISION_COLUMNS}
         FROM pcp_revisions r
         WHERE r.page_id = ?1
         ORDER BY r.created_at DESC, r.revision_id DESC"
    );
    let mut statement = transaction
        .prepare(&sql)
        .context("prepare PCP fallback Revision lookup")?;
    let mut rows = statement
        .query([page_id])
        .context("query PCP fallback Revisions")?;
    while let Some(row) = rows.next().context("read PCP fallback Revision")? {
        let revision = revision_from_row(row, true, true, true, true)?;
        if !affected.contains(&revision.revision_id)
            && revision.lifecycle_status != LifecycleStatus::Tombstoned
        {
            return Ok(Some(revision));
        }
    }
    Ok(None)
}

fn to_sql_error(error: anyhow::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, error.into())
}
