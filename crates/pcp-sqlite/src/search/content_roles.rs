//! Content-library classification is structural, never inferred from `kind`.
//! These predicates are shared by the row query, filtered totals and retrieval.
use pcp_store::{ContentLibraryFilter, ContentPageRole};
use rusqlite::types::Value;

pub(super) const CURRENT_TOPIC_COVERAGE: &str = "EXISTS (
    SELECT 1
    FROM pcp_topic_extraction_members extraction_member
    JOIN pcp_pages topic_page ON topic_page.page_id = extraction_member.topic_page_id
    JOIN pcp_revisions topic_revision ON topic_revision.revision_id = topic_page.current_revision_id
    WHERE extraction_member.source_page_id = p.page_id
      AND extraction_member.source_revision_id = r.revision_id
      AND extraction_member.topic_revision_id = topic_page.current_revision_id
      AND topic_page.lifecycle_status = 'active'
      AND topic_revision.lifecycle_status = 'active'
)";

pub(super) const CURRENT_SUMMARY: &str = "CASE WHEN summary_revision.lifecycle_status = 'active'
    AND EXISTS (SELECT 1 FROM pcp_summaries attached
        WHERE attached.summary_revision_id = summary_revision.revision_id
          AND attached.target_revision_id = r.revision_id)
    THEN summary_revision.revision_id ELSE NULL END";

pub(super) fn role_sql() -> String {
    format!(
        "CASE WHEN EXISTS (
        SELECT 1 FROM pcp_topic_extractions extraction
        WHERE extraction.topic_revision_id = r.revision_id
    ) THEN 'condensed' WHEN {CURRENT_TOPIC_COVERAGE}
      THEN 'covered_source' ELSE 'other' END"
    )
}

pub(super) fn append_filter(
    sql: &mut String,
    values: &mut Vec<Value>,
    filter: &ContentLibraryFilter,
) {
    if let Some(role) = filter.role {
        sql.push_str(&format!(" AND ({}) = ?", role_sql()));
        values.push(Value::Text(
            match role {
                ContentPageRole::Condensed => "condensed",
                ContentPageRole::CoveredSource => "covered_source",
                ContentPageRole::Other => "other",
            }
            .into(),
        ));
    }
    if filter.with_summary {
        sql.push_str(&format!(" AND ({CURRENT_SUMMARY}) IS NOT NULL"));
    }
}
