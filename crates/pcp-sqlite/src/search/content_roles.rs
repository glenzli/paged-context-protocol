//! Content-library classification is structural, never inferred from `kind`.
//! These predicates are shared by the row query, filtered totals and retrieval.
use pcp_store::{ContentLibraryFilter, ContentPageRole};
use rusqlite::types::Value;

pub(super) fn scope_parameters(scope_count: usize) -> String {
    (1..=scope_count)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(",")
}

pub(super) fn current_topic_coverage(scope_count: usize) -> String {
    format!(
        "EXISTS (
    SELECT 1
    FROM pcp_topic_extraction_members extraction_member
    JOIN pcp_pages topic_page ON topic_page.page_id = extraction_member.topic_page_id
    JOIN pcp_revisions topic_revision ON topic_revision.revision_id = topic_page.current_revision_id
    WHERE extraction_member.source_page_id = p.page_id
      AND extraction_member.source_revision_id = r.revision_id
      AND extraction_member.topic_revision_id = topic_page.current_revision_id
      AND topic_page.lifecycle_status = 'active'
      AND topic_revision.lifecycle_status = 'active'
      AND topic_page.namespace IN ({})
)",
        scope_parameters(scope_count)
    )
}

pub(super) const CURRENT_SUMMARY: &str = "CASE WHEN summary_revision.lifecycle_status = 'active'
    AND EXISTS (SELECT 1 FROM pcp_summaries attached
        WHERE attached.summary_revision_id = summary_revision.revision_id
          AND attached.target_revision_id = r.revision_id)
    THEN summary_revision.revision_id ELSE NULL END";

pub(super) fn role_sql(scope_count: usize) -> String {
    let coverage = current_topic_coverage(scope_count);
    format!(
        "CASE WHEN EXISTS (
        SELECT 1 FROM pcp_topic_extractions extraction
        WHERE extraction.topic_revision_id = r.revision_id
    ) THEN 'condensed' WHEN {coverage}
      THEN 'covered_source' ELSE 'other' END"
    )
}

pub(super) fn append_filter(
    sql: &mut String,
    values: &mut Vec<Value>,
    filter: &ContentLibraryFilter,
    scope_count: usize,
) {
    if let Some(role) = filter.role {
        sql.push_str(&format!(" AND ({}) = ?", role_sql(scope_count)));
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
