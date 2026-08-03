use anyhow::{Context, Result};
use pcp_store::DurablePageInventoryItem;
use rusqlite::{params_from_iter, types::Value as SqlValue};

use crate::store::SqlitePcpStore;

const MAX_DURABLE_INVENTORY: usize = 100;
const MAX_INVENTORY_SNIPPET_CHARS: usize = 1_600;

impl SqlitePcpStore {
    pub async fn durable_page_inventory(
        &self,
        allowed_scopes: Vec<String>,
        excluded_page_kinds: Vec<String>,
    ) -> Result<Vec<DurablePageInventoryItem>> {
        if allowed_scopes.is_empty() {
            return Ok(Vec::new());
        }
        self.run("durable page inventory", move |connection| {
            let placeholders = (0..allowed_scopes.len())
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(",");
            let mut sql = format!(
                "
                SELECT r.page_id, r.revision_id, r.namespace,
                       page.kind,
                       r.created_at, r.observed_at,
                       length(COALESCE(r.payload_content, '')),
                       substr(COALESCE(r.payload_content, ''), 1, ?),
                       r.facets_json,
                       summary.summary_revision_id, summary.content,
                       COALESCE((
                           SELECT json_group_array(relation_type)
                           FROM (
                               SELECT DISTINCT relation.relation_type
                               FROM pcp_relations relation
                               WHERE relation.from_page_id = page.page_id
                                  OR relation.to_page_id = page.page_id
                               ORDER BY relation.relation_type
                           )
                       ), '[]')
                FROM pcp_pages page
                JOIN pcp_revisions r ON r.revision_id = page.current_revision_id
                LEFT JOIN pcp_page_summary_heads summary_head
                  ON summary_head.target_page_id = page.page_id
                LEFT JOIN pcp_summaries summary
                  ON summary.summary_revision_id = summary_head.current_summary_revision_id
                WHERE r.namespace IN ({placeholders})
                  AND r.lifecycle_status = 'active'
                "
            );
            let mut values = vec![SqlValue::Integer(MAX_INVENTORY_SNIPPET_CHARS as i64)];
            values.extend(allowed_scopes.into_iter().map(SqlValue::Text));
            if !excluded_page_kinds.is_empty() {
                let placeholders = (0..excluded_page_kinds.len())
                    .map(|_| "?")
                    .collect::<Vec<_>>()
                    .join(",");
                sql.push_str(&format!(" AND page.kind NOT IN ({placeholders})"));
                values.extend(excluded_page_kinds.into_iter().map(SqlValue::Text));
            }
            sql.push_str(
                " ORDER BY COALESCE(r.observed_at, r.created_at) DESC, r.revision_id DESC
                  LIMIT ?",
            );
            values.push(SqlValue::Integer(MAX_DURABLE_INVENTORY as i64));
            let mut statement = connection
                .prepare(&sql)
                .context("prepare durable PCP inventory")?;
            let items = statement
                .query_map(params_from_iter(values.iter()), |row| {
                    let facets_json: Option<String> = row.get(8)?;
                    let relation_types_json: String = row.get(11)?;
                    Ok(DurablePageInventoryItem {
                        page_id: row.get(0)?,
                        revision_id: row.get(1)?,
                        namespace: row.get(2)?,
                        kind: row.get(3)?,
                        created_at: row.get(4)?,
                        observed_at: row.get(5)?,
                        content_chars: row.get::<_, i64>(6)? as u64,
                        snippet: row.get(7)?,
                        facets: facets_json
                            .as_deref()
                            .and_then(|value| serde_json::from_str(value).ok()),
                        summary_revision_id: row.get(9)?,
                        summary: row.get(10)?,
                        relation_types: serde_json::from_str(&relation_types_json)
                            .unwrap_or_default(),
                    })
                })
                .context("query durable PCP inventory")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("collect durable PCP inventory")?;
            Ok(items)
        })
        .await
    }
}
