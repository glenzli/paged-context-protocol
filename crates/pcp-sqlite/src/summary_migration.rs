use std::collections::HashMap;

use anyhow::{Context, Result};
use pcp_core::{Actor, ActorType, PagePayload, ProvenanceEvent};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::json;

use crate::write::{insert_relation, insert_revision};

#[derive(Debug)]
struct LegacySummary {
    summary_revision_id: String,
    target_revision_id: String,
    summary_page_id: Option<String>,
    content: String,
    actor_type: String,
    actor_id: String,
    created_at: String,
    provenance_json: String,
}

pub(crate) fn migrate(connection: &mut Connection) -> Result<()> {
    ensure_summary_page_column(connection)?;
    migrate_summary_pages(connection)
}

fn ensure_summary_page_column(connection: &Connection) -> Result<()> {
    let mut statement = connection
        .prepare("PRAGMA table_info(pcp_summaries)")
        .context("inspect PCP Summary schema")?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .context("list PCP Summary columns")?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !columns.iter().any(|column| column == "summary_page_id") {
        connection
            .execute(
                "ALTER TABLE pcp_summaries ADD COLUMN summary_page_id TEXT REFERENCES pcp_pages(page_id)",
                [],
            )
            .context("add PCP Summary Page identity")?;
    }
    Ok(())
}

fn migrate_summary_pages(connection: &mut Connection) -> Result<()> {
    let transaction = connection
        .transaction()
        .context("start PCP Summary Page migration")?;
    let summaries = {
        let mut statement = transaction
            .prepare(
                "
                SELECT summary_revision_id, target_revision_id, summary_page_id,
                       content, actor_type, actor_id, created_at, provenance_json
                FROM pcp_summaries
                ORDER BY target_revision_id, created_at, summary_revision_id
                ",
            )
            .context("prepare legacy PCP Summary migration")?;
        statement
            .query_map([], |row| {
                Ok(LegacySummary {
                    summary_revision_id: row.get(0)?,
                    target_revision_id: row.get(1)?,
                    summary_page_id: row.get(2)?,
                    content: row.get(3)?,
                    actor_type: row.get(4)?,
                    actor_id: row.get(5)?,
                    created_at: row.get(6)?,
                    provenance_json: row.get(7)?,
                })
            })
            .context("read legacy PCP Summaries")?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };

    let mut pages = summaries
        .iter()
        .filter_map(|summary| {
            summary
                .summary_page_id
                .as_ref()
                .map(|page_id| (summary.target_revision_id.clone(), page_id.clone()))
        })
        .collect::<HashMap<_, _>>();
    for summary in summaries {
        if let Some(page_id) = summary.summary_page_id.as_ref() {
            pages
                .entry(summary.target_revision_id.clone())
                .or_insert_with(|| page_id.clone());
            continue;
        }
        let page_id = match pages.get(&summary.target_revision_id) {
            Some(page_id) => page_id.clone(),
            None => {
                let page_id = random_id(&transaction, "pg_")?;
                transaction
                    .execute(
                        "INSERT INTO pcp_pages (page_id, current_revision_id, created_at) VALUES (?1, NULL, ?2)",
                        params![page_id, summary.created_at],
                    )
                    .context("create migrated PCP Summary Page")?;
                pages.insert(summary.target_revision_id.clone(), page_id.clone());
                page_id
            }
        };
        let (owner_id, namespace, visibility): (String, String, String) = transaction
            .query_row(
                "SELECT owner_id, namespace, visibility FROM pcp_revisions WHERE revision_id = ?1",
                [&summary.target_revision_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .with_context(|| {
                format!(
                    "find target Revision {} for PCP Summary migration",
                    summary.target_revision_id
                )
            })?;
        let actor_type = ActorType::parse(&summary.actor_type)
            .with_context(|| format!("unknown PCP Summary actor type {}", summary.actor_type))?;
        let actor = Actor {
            actor_type,
            actor_id: summary.actor_id.clone(),
        };
        let provenance = serde_json::from_str::<Vec<ProvenanceEvent>>(&summary.provenance_json)
            .context("decode migrated PCP Summary provenance")?;
        let payload = PagePayload {
            media_type: "text/markdown".to_owned(),
            content: summary.content.clone(),
        };
        let facets = json!({
            "kind": "summary_projection",
            "targetRevisionId": summary.target_revision_id,
        });
        insert_revision(
            &transaction,
            &page_id,
            &summary.summary_revision_id,
            &owner_id,
            &namespace,
            &visibility,
            "active",
            &summary.created_at,
            None,
            None,
            None,
            &actor,
            Some(&payload),
            &[],
            Some(&facets),
            &provenance,
        )?;
        insert_relation(
            &transaction,
            &summary.summary_revision_id,
            "summarizes",
            &summary.target_revision_id,
            &actor,
            &summary.created_at,
        )?;
        transaction
            .execute(
                "UPDATE pcp_summaries SET summary_page_id = ?2 WHERE summary_revision_id = ?1",
                params![summary.summary_revision_id, page_id],
            )
            .context("attach migrated PCP Summary Page identity")?;
        let is_current = transaction
            .query_row(
                "SELECT 1 FROM pcp_summary_heads WHERE target_revision_id = ?1 AND current_summary_revision_id = ?2",
                params![summary.target_revision_id, summary.summary_revision_id],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if is_current {
            transaction
                .execute(
                    "UPDATE pcp_pages SET current_revision_id = ?2 WHERE page_id = ?1",
                    params![page_id, summary.summary_revision_id],
                )
                .context("publish migrated PCP Summary Revision")?;
        }
    }
    transaction
        .commit()
        .context("commit PCP Summary Page migration")
}

fn random_id(transaction: &Transaction<'_>, prefix: &str) -> Result<String> {
    let value: String = transaction
        .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))
        .context("generate migrated PCP identity")?;
    Ok(format!("{prefix}{value}"))
}
