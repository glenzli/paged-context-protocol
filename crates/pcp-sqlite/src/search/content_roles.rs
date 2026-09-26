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

/// A Topic is a first-pass route only while its current Revision is usable
/// and visible to the reader. This does not change the source's lifecycle.
pub(super) fn current_topic_route(scope_count: usize) -> String {
    format!(
        "EXISTS (
    SELECT 1 FROM pcp_topic_extraction_members member
    JOIN pcp_pages topic ON topic.page_id = member.topic_page_id
    JOIN pcp_revisions topic_revision ON topic_revision.revision_id = topic.current_revision_id
    WHERE member.source_page_id = p.page_id
      AND member.source_revision_id = r.revision_id
      AND member.topic_revision_id = topic.current_revision_id
      AND topic.lifecycle_status = 'active'
      AND topic_revision.lifecycle_status = 'active'
      AND topic.namespace IN ({})
      AND NOT EXISTS (
          SELECT 1 FROM pcp_relations newer
          WHERE newer.relation_type = 'supersedes'
            AND newer.to_page_id = topic.page_id
            AND NOT EXISTS (
                SELECT 1 FROM pcp_relation_retractions retraction
                WHERE retraction.relation_id = newer.relation_id
            )
      )
      AND NOT EXISTS (
          SELECT 1 FROM pcp_validity_heads head
          JOIN pcp_pages assessment_page ON assessment_page.page_id = head.assessment_page_id
          JOIN pcp_validity_assessments assessment
            ON assessment.assessment_revision_id = assessment_page.current_revision_id
          JOIN pcp_revisions validity_revision
            ON validity_revision.revision_id = assessment.assessment_revision_id
          WHERE head.target_page_id = topic.page_id
            AND assessment.target_revision_id = topic_revision.revision_id
            AND json_extract(validity_revision.facets_json, '$.standing') = 'retracted'
      )
)",
        scope_parameters(scope_count)
    )
}

/// A reviewed source-only decision remains effective only while every
/// canonical output carrying that source is still a usable current head.
pub(super) const CURRENT_CONSOLIDATION_COVERAGE: &str = "EXISTS (
    SELECT 1 FROM pcp_consolidation_sources source
    WHERE source.source_page_id = p.page_id
      AND source.source_revision_id = r.revision_id
      AND source.source_only = 1
      AND EXISTS (
          SELECT 1 FROM pcp_consolidation_coverage coverage
          WHERE coverage.consolidation_id = source.consolidation_id
            AND coverage.source_revision_id = source.source_revision_id
      )
      AND NOT EXISTS (
          SELECT 1 FROM pcp_consolidation_coverage coverage
          JOIN pcp_pages output ON output.page_id = coverage.output_page_id
          JOIN pcp_revisions output_revision ON output_revision.revision_id = coverage.output_revision_id
          WHERE coverage.consolidation_id = source.consolidation_id
            AND coverage.source_revision_id = source.source_revision_id
            AND (output.current_revision_id != coverage.output_revision_id
                 OR output.lifecycle_status != 'active'
                 OR output_revision.lifecycle_status != 'active'
                 OR EXISTS (
                     SELECT 1 FROM pcp_relations newer
                     WHERE newer.relation_type = 'supersedes'
                       AND newer.to_page_id = output.page_id
                       AND NOT EXISTS (
                           SELECT 1 FROM pcp_relation_retractions retraction
                           WHERE retraction.relation_id = newer.relation_id
                       )
                 )
                 OR EXISTS (
                     SELECT 1 FROM pcp_validity_heads validity_head
                     JOIN pcp_pages validity_page ON validity_page.page_id = validity_head.assessment_page_id
                     JOIN pcp_validity_assessments assessment
                       ON assessment.assessment_revision_id = validity_page.current_revision_id
                     JOIN pcp_revisions validity_revision
                       ON validity_revision.revision_id = assessment.assessment_revision_id
                     WHERE validity_head.target_page_id = output.page_id
                       AND assessment.target_revision_id = output_revision.revision_id
                       AND json_extract(validity_revision.facets_json, '$.standing') = 'retracted'
                 ))
      )
)";

pub(super) const CURRENT_SUMMARY: &str = "CASE WHEN summary_revision.lifecycle_status = 'active'
    AND EXISTS (SELECT 1 FROM pcp_summaries attached
        WHERE attached.summary_revision_id = summary_revision.revision_id
          AND attached.target_revision_id = r.revision_id)
    THEN summary_revision.revision_id ELSE NULL END";

pub(super) fn role_sql(scope_count: usize) -> String {
    let topic_route = current_topic_route(scope_count);
    format!(
        "CASE WHEN {CURRENT_CONSOLIDATION_COVERAGE} OR {topic_route}
      THEN 'covered_source' WHEN EXISTS (
        SELECT 1 FROM pcp_topic_extractions extraction
        WHERE extraction.topic_revision_id = r.revision_id
    ) OR EXISTS (
        SELECT 1 FROM pcp_consolidation_coverage canonical
        WHERE canonical.output_revision_id = r.revision_id
    ) THEN 'condensed' ELSE 'other' END"
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
