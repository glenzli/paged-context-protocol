use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

const MIGRATION_KEY: &str = "page_revision_model_version";
const MIGRATION_VERSION: &str = "5";

pub(crate) fn migrate(connection: &mut Connection) -> Result<()> {
    let current = connection
        .query_row(
            "SELECT value FROM pcp_metadata WHERE key = ?1",
            [MIGRATION_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .context("read PCP Page/Revision migration version")?;
    if current.as_deref() == Some(MIGRATION_VERSION) {
        return Ok(());
    }

    let transaction = connection
        .transaction()
        .context("start PCP Page/Revision migration")?;

    add_column_if_missing(&transaction, "pcp_pages", "owner_id", "TEXT")?;
    add_column_if_missing(&transaction, "pcp_pages", "namespace", "TEXT")?;
    add_column_if_missing(&transaction, "pcp_pages", "visibility", "TEXT")?;
    add_column_if_missing(&transaction, "pcp_pages", "kind", "TEXT")?;
    add_column_if_missing(&transaction, "pcp_pages", "mutability", "TEXT")?;
    add_column_if_missing(&transaction, "pcp_pages", "lifecycle_status", "TEXT")?;
    add_column_if_missing(&transaction, "pcp_pages", "updated_at", "TEXT")?;
    add_column_if_missing(
        &transaction,
        "pcp_revisions",
        "previous_revision_id",
        "TEXT",
    )?;
    add_column_if_missing(&transaction, "pcp_relations", "from_page_id", "TEXT")?;
    add_column_if_missing(&transaction, "pcp_relations", "to_page_id", "TEXT")?;
    add_column_if_missing(
        &transaction,
        "pcp_relations",
        "basis_revision_ids_json",
        "TEXT",
    )?;
    add_column_if_missing(&transaction, "pcp_summaries", "target_page_id", "TEXT")?;

    transaction
        .execute_batch(
            r#"
            UPDATE pcp_pages
            SET owner_id = COALESCE(owner_id, (
                    SELECT owner_id FROM pcp_revisions
                    WHERE revision_id = pcp_pages.current_revision_id
                )),
                namespace = COALESCE(namespace, (
                    SELECT namespace FROM pcp_revisions
                    WHERE revision_id = pcp_pages.current_revision_id
                )),
                visibility = COALESCE(visibility, (
                    SELECT visibility FROM pcp_revisions
                    WHERE revision_id = pcp_pages.current_revision_id
                )),
                kind = COALESCE(kind, NULLIF((
                    SELECT json_extract(facets_json, '$.kind') FROM pcp_revisions
                    WHERE revision_id = pcp_pages.current_revision_id
                ), ''), 'document'),
                lifecycle_status = COALESCE(lifecycle_status, (
                    SELECT lifecycle_status FROM pcp_revisions
                    WHERE revision_id = pcp_pages.current_revision_id
                ), 'active'),
                updated_at = COALESCE(updated_at, (
                    SELECT created_at FROM pcp_revisions
                    WHERE revision_id = pcp_pages.current_revision_id
                ), created_at);

            UPDATE pcp_pages
            SET mutability = CASE
                WHEN mutability IS NOT NULL THEN mutability
                WHEN (SELECT count(*) FROM pcp_revisions r
                      WHERE r.page_id = pcp_pages.page_id) > 1 THEN 'revisioned'
                WHEN kind IN (
                    'summary_projection', 'validity_assessment', 'topic_projection',
                    'symbiont_current_map', 'symbiont_open_loops', 'conversation_episode',
                    'symbiont_hunch', 'user_profile', 'user_orientation', 'project_state'
                ) THEN 'revisioned'
                ELSE 'sealed'
            END;

            CREATE TEMP TABLE pcp_validity_page_map AS
            SELECT target_page_id, assessment_page_id AS canonical_page_id,
                   assessment_id AS current_assessment_id,
                   target_revision_id
            FROM (
                SELECT target.page_id AS target_page_id,
                       assessment_revision.page_id AS assessment_page_id,
                       assessment.assessment_id,
                       assessment.target_revision_id,
                       row_number() OVER (
                           PARTITION BY target.page_id
                           ORDER BY assessment.assessed_at DESC,
                                    assessment.assessment_id DESC
                       ) AS rank
                FROM pcp_validity_assessments assessment
                JOIN pcp_revisions target
                  ON target.revision_id = assessment.target_revision_id
                JOIN pcp_revisions assessment_revision
                  ON assessment_revision.revision_id = assessment.assessment_id
            ) ranked
            WHERE rank = 1;

            UPDATE pcp_revisions
            SET page_id = (
                SELECT map.canonical_page_id
                FROM pcp_validity_assessments assessment
                JOIN pcp_revisions target
                  ON target.revision_id = assessment.target_revision_id
                JOIN pcp_validity_page_map map
                  ON map.target_page_id = target.page_id
                WHERE assessment.assessment_id = pcp_revisions.revision_id
            )
            WHERE revision_id IN (
                SELECT assessment_id FROM pcp_validity_assessments
            );

            UPDATE pcp_pages
            SET current_revision_id = (
                    SELECT map.current_assessment_id
                    FROM pcp_validity_page_map map
                    WHERE map.canonical_page_id = pcp_pages.page_id
                ),
                kind = 'validity_assessment',
                mutability = 'revisioned',
                lifecycle_status = 'active',
                updated_at = COALESCE((
                    SELECT assessment.assessed_at
                    FROM pcp_validity_page_map map
                    JOIN pcp_validity_assessments assessment
                      ON assessment.assessment_id = map.current_assessment_id
                    WHERE map.canonical_page_id = pcp_pages.page_id
                ), updated_at)
            WHERE page_id IN (
                SELECT canonical_page_id FROM pcp_validity_page_map
            );

            DELETE FROM pcp_pages
            WHERE kind = 'validity_assessment'
              AND page_id NOT IN (
                  SELECT canonical_page_id FROM pcp_validity_page_map
              );

            DROP TABLE pcp_validity_heads;
            CREATE TABLE pcp_validity_heads (
                target_page_id TEXT PRIMARY KEY REFERENCES pcp_pages(page_id),
                target_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id),
                current_assessment_id TEXT NOT NULL
                    REFERENCES pcp_validity_assessments(assessment_id)
            );
            INSERT INTO pcp_validity_heads (
                target_page_id, target_revision_id, current_assessment_id
            )
            SELECT target_page_id, target_revision_id, current_assessment_id
            FROM pcp_validity_page_map;
            DROP TABLE pcp_validity_page_map;

            UPDATE pcp_summaries
            SET target_page_id = COALESCE(target_page_id, (
                SELECT page_id FROM pcp_revisions
                WHERE revision_id = pcp_summaries.target_revision_id
            ));

            CREATE TEMP TABLE pcp_summary_page_map AS
            SELECT summary.summary_revision_id,
                   summary.target_page_id,
                   summary.summary_page_id AS old_page_id,
                   first_value(summary.summary_page_id) OVER (
                       PARTITION BY summary.target_page_id
                       ORDER BY summary.created_at DESC,
                                summary.summary_revision_id DESC
                   ) AS canonical_page_id,
                   first_value(summary.summary_revision_id) OVER (
                       PARTITION BY summary.target_page_id
                       ORDER BY summary.created_at DESC,
                                summary.summary_revision_id DESC
                   ) AS current_summary_revision_id,
                   first_value(summary.created_at) OVER (
                       PARTITION BY summary.target_page_id
                       ORDER BY summary.created_at DESC,
                                summary.summary_revision_id DESC
                   ) AS updated_at
            FROM pcp_summaries summary
            WHERE summary.target_page_id IS NOT NULL
              AND summary.summary_page_id IS NOT NULL;

            UPDATE pcp_revisions
            SET page_id = (
                SELECT map.canonical_page_id
                FROM pcp_summary_page_map map
                WHERE map.summary_revision_id = pcp_revisions.revision_id
            )
            WHERE revision_id IN (
                SELECT summary_revision_id FROM pcp_summary_page_map
            );

            UPDATE pcp_summaries
            SET summary_page_id = (
                SELECT map.canonical_page_id
                FROM pcp_summary_page_map map
                WHERE map.summary_revision_id = pcp_summaries.summary_revision_id
            )
            WHERE summary_revision_id IN (
                SELECT summary_revision_id FROM pcp_summary_page_map
            );

            UPDATE pcp_pages
            SET current_revision_id = (
                    SELECT map.current_summary_revision_id
                    FROM pcp_summary_page_map map
                    WHERE map.canonical_page_id = pcp_pages.page_id
                    LIMIT 1
                ),
                kind = 'summary_projection',
                mutability = 'revisioned',
                lifecycle_status = 'active',
                updated_at = COALESCE((
                    SELECT map.updated_at
                    FROM pcp_summary_page_map map
                    WHERE map.canonical_page_id = pcp_pages.page_id
                    LIMIT 1
                ), updated_at)
            WHERE page_id IN (
                SELECT canonical_page_id FROM pcp_summary_page_map
            );

            DELETE FROM pcp_pages
            WHERE page_id IN (
                SELECT old_page_id FROM pcp_summary_page_map
                WHERE old_page_id <> canonical_page_id
            );

            WITH ordered AS (
                SELECT revision_id,
                       lag(revision_id) OVER (
                           PARTITION BY page_id ORDER BY created_at, revision_id
                       ) AS previous_revision_id
                FROM pcp_revisions
            )
            UPDATE pcp_revisions
            SET previous_revision_id = (
                SELECT ordered.previous_revision_id FROM ordered
                WHERE ordered.revision_id = pcp_revisions.revision_id
            )
            WHERE previous_revision_id IS NULL;

            UPDATE pcp_relations
            SET from_page_id = COALESCE(from_page_id, (
                    SELECT page_id FROM pcp_revisions
                    WHERE revision_id = pcp_relations.from_revision_id
                )),
                to_page_id = COALESCE(to_page_id, (
                    SELECT page_id FROM pcp_revisions
                    WHERE revision_id = pcp_relations.to_revision_id
                )),
                basis_revision_ids_json = COALESCE(
                    basis_revision_ids_json,
                    json_array(from_revision_id, to_revision_id)
                );

            DELETE FROM pcp_relations
            WHERE relation_type = 'supersedes'
              AND from_page_id = to_page_id;

            UPDATE pcp_relations AS kept
            SET basis_revision_ids_json = (
                SELECT json_group_array(revision_id)
                FROM (
                    SELECT DISTINCT basis.value AS revision_id
                    FROM pcp_relations duplicate,
                         json_each(COALESCE(
                             duplicate.basis_revision_ids_json,
                             json_array(
                                 duplicate.from_revision_id,
                                 duplicate.to_revision_id
                             )
                         )) basis
                    WHERE duplicate.from_page_id = kept.from_page_id
                      AND duplicate.relation_type = kept.relation_type
                      AND duplicate.to_page_id = kept.to_page_id
                    ORDER BY basis.value
                )
            )
            WHERE kept.rowid = (
                SELECT min(candidate.rowid)
                FROM pcp_relations candidate
                WHERE candidate.from_page_id = kept.from_page_id
                  AND candidate.relation_type = kept.relation_type
                  AND candidate.to_page_id = kept.to_page_id
            );

            DELETE FROM pcp_relations
            WHERE rowid NOT IN (
                SELECT min(rowid)
                FROM pcp_relations
                GROUP BY from_page_id, relation_type, to_page_id
            );

            DROP TABLE pcp_refs;

            DELETE FROM pcp_revision_fts;
            INSERT INTO pcp_revision_fts (
                revision_id, page_id, namespace, payload_content, facets_text
            )
            SELECT revision.revision_id, page.page_id, page.namespace,
                   COALESCE(revision.payload_content, ''),
                   COALESCE(revision.facets_json, '')
            FROM pcp_pages page
            JOIN pcp_revisions revision
              ON revision.revision_id = page.current_revision_id;

            CREATE INDEX IF NOT EXISTS pcp_pages_scope
                ON pcp_pages(namespace, updated_at DESC);
            CREATE INDEX IF NOT EXISTS pcp_pages_kind
                ON pcp_pages(kind, lifecycle_status, updated_at DESC);
            CREATE INDEX IF NOT EXISTS pcp_relations_page_from
                ON pcp_relations(from_page_id, relation_type);
            CREATE INDEX IF NOT EXISTS pcp_relations_page_to
                ON pcp_relations(to_page_id, relation_type);

            CREATE TABLE IF NOT EXISTS pcp_page_summary_heads (
                target_page_id TEXT PRIMARY KEY REFERENCES pcp_pages(page_id),
                target_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id),
                summary_page_id TEXT NOT NULL REFERENCES pcp_pages(page_id),
                current_summary_revision_id TEXT NOT NULL
                    REFERENCES pcp_summaries(summary_revision_id),
                updated_at TEXT NOT NULL
            );

            INSERT INTO pcp_page_summary_heads (
                target_page_id, target_revision_id, summary_page_id,
                current_summary_revision_id, updated_at
            )
            SELECT ranked.target_page_id, ranked.target_revision_id,
                   ranked.summary_page_id, ranked.summary_revision_id,
                   ranked.created_at
            FROM (
                SELECT summary.*,
                       row_number() OVER (
                           PARTITION BY summary.target_page_id
                           ORDER BY summary.created_at DESC, summary.summary_revision_id DESC
                       ) AS rank
                FROM pcp_summaries summary
                WHERE summary.target_page_id IS NOT NULL
            ) ranked
            WHERE ranked.rank = 1
            ON CONFLICT(target_page_id) DO UPDATE SET
                target_revision_id = excluded.target_revision_id,
                summary_page_id = excluded.summary_page_id,
                current_summary_revision_id = excluded.current_summary_revision_id,
                updated_at = excluded.updated_at;

            DELETE FROM pcp_summary_fts;
            INSERT INTO pcp_summary_fts (
                summary_revision_id, target_revision_id, content
            )
            SELECT summary.summary_revision_id, summary.target_revision_id, summary.content
            FROM pcp_page_summary_heads head
            JOIN pcp_summaries summary
              ON summary.summary_revision_id = head.current_summary_revision_id;

            DROP TABLE pcp_summary_page_map;
            "#,
        )
        .context("upgrade PCP Page/Revision data")?;

    transaction
        .execute(
            "INSERT INTO pcp_metadata (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![MIGRATION_KEY, MIGRATION_VERSION],
        )
        .context("publish PCP Page/Revision migration")?;
    transaction
        .commit()
        .context("commit PCP Page/Revision migration")?;
    Ok(())
}

fn add_column_if_missing(
    connection: &Connection,
    table: &str,
    column: &str,
    declaration: &str,
) -> Result<()> {
    let columns = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .with_context(|| format!("inspect {table}"))?
        .query_map([], |row| row.get::<_, String>(1))
        .with_context(|| format!("query {table} columns"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .with_context(|| format!("collect {table} columns"))?;
    if columns.iter().any(|candidate| candidate == column) {
        return Ok(());
    }
    connection
        .execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {declaration}"),
            [],
        )
        .with_context(|| format!("add {table}.{column}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::migrate;
    use crate::schema;

    #[test]
    fn migration_is_idempotent() {
        let mut connection = Connection::open_in_memory().expect("open database");
        schema::initialize(&mut connection).expect("initialize schema");
        migrate(&mut connection).expect("repeat migration");
        let version: String = connection
            .query_row(
                "SELECT value FROM pcp_metadata WHERE key = 'page_revision_model_version'",
                [],
                |row| row.get(0),
            )
            .expect("migration version");
        assert_eq!(version, "5");
    }
}
