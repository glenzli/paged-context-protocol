use anyhow::{Context, Result};
use pcp_core::PACKED_PAGE_MEDIA_TYPE;
use pcp_store::DurablePageInventoryItem;
use rusqlite::{params_from_iter, types::Value as SqlValue};

use crate::store::SqlitePcpStore;

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
                       page.kind, page.mutability,
                       r.created_at, r.observed_at, r.source_span_json,
                       r.payload_media_type,
                       length(COALESCE(r.payload_content, '')),
                       CASE
                           WHEN r.payload_media_type = ? THEN
                               'Packed range boundary:\nfirst: ' ||
                               substr(COALESCE(json_extract(
                                   r.payload_content,
                                   '$.entries[0].payload.content'
                               ), ''), 1, ?) ||
                               '\nlast: ' ||
                               substr(COALESCE(json_extract(
                                   r.payload_content,
                                   '$.entries[#-1].payload.content'
                               ), ''), 1, ?)
                           ELSE substr(COALESCE(r.payload_content, ''), 1, ?)
                       END,
                       r.facets_json,
                       summary.summary_revision_id, summary.target_revision_id,
                       summary_revision.payload_content,
                       COALESCE((
                           SELECT json_group_array(relation_type)
                           FROM (
                               SELECT DISTINCT relation.relation_type
                               FROM pcp_relations relation
                               WHERE (relation.from_page_id = page.page_id
                                  OR relation.to_page_id = page.page_id)
                                 AND NOT EXISTS (
                                     SELECT 1 FROM pcp_relation_retractions retraction
                                     WHERE retraction.relation_id = relation.relation_id
                                 )
                               ORDER BY relation.relation_type
                           )
                       ), '[]'),
                       EXISTS (
                           SELECT 1 FROM pcp_summaries summary_reference
                           WHERE summary_reference.target_revision_id = r.revision_id
                              OR summary_reference.summary_revision_id = r.revision_id
                           UNION ALL
                           SELECT 1 FROM pcp_summary_assessments summary_assessment
                           WHERE summary_assessment.target_revision_id = r.revision_id
                           UNION ALL
                           SELECT 1 FROM pcp_validity_assessments validity
                           WHERE validity.target_revision_id = r.revision_id
                              OR validity.assessment_revision_id = r.revision_id
                           UNION ALL
                           SELECT 1 FROM pcp_revision_retention_leases retention
                           WHERE retention.revision_id = r.revision_id
                       )
                FROM pcp_pages page
                JOIN pcp_revisions r ON r.revision_id = page.current_revision_id
                LEFT JOIN pcp_page_summary_heads summary_head
                  ON summary_head.target_page_id = page.page_id
                LEFT JOIN pcp_pages summary_page
                  ON summary_page.page_id = summary_head.summary_page_id
                LEFT JOIN pcp_summaries summary
                  ON summary.summary_revision_id = summary_page.current_revision_id
                LEFT JOIN pcp_revisions summary_revision
                  ON summary_revision.revision_id = summary.summary_revision_id
                WHERE r.namespace IN ({placeholders})
                  AND r.lifecycle_status = 'active'
                  AND page.lifecycle_status = 'active'
                "
            );
            let boundary_chars = MAX_INVENTORY_SNIPPET_CHARS / 2;
            let mut values = vec![
                SqlValue::Text(PACKED_PAGE_MEDIA_TYPE.to_owned()),
                SqlValue::Integer(boundary_chars as i64),
                SqlValue::Integer(boundary_chars as i64),
                SqlValue::Integer(MAX_INVENTORY_SNIPPET_CHARS as i64),
            ];
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
                " ORDER BY COALESCE(r.observed_at, r.created_at) DESC, r.revision_id DESC",
            );
            let mut statement = connection
                .prepare(&sql)
                .context("prepare durable PCP inventory")?;
            let items = statement
                .query_map(params_from_iter(values.iter()), |row| {
                    let mutability: String = row.get(4)?;
                    let source_span_json: Option<String> = row.get(7)?;
                    let facets_json: Option<String> = row.get(11)?;
                    let relation_types_json: String = row.get(15)?;
                    Ok(DurablePageInventoryItem {
                        page_id: row.get(0)?,
                        revision_id: row.get(1)?,
                        namespace: row.get(2)?,
                        kind: row.get(3)?,
                        mutability: pcp_core::PageMutability::parse(&mutability)
                            .unwrap_or_default(),
                        created_at: row.get(5)?,
                        observed_at: row.get(6)?,
                        source_span: source_span_json
                            .as_deref()
                            .and_then(|value| serde_json::from_str(value).ok()),
                        media_type: row.get(8)?,
                        content_chars: row.get::<_, i64>(9)? as u64,
                        snippet: row.get(10)?,
                        facets: facets_json
                            .as_deref()
                            .and_then(|value| serde_json::from_str(value).ok()),
                        summary_revision_id: row.get(12)?,
                        summary_target_revision_id: row.get(13)?,
                        summary: row.get(14)?,
                        relation_types: serde_json::from_str(&relation_types_json)
                            .unwrap_or_default(),
                        packing_protected: row.get(16)?,
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
