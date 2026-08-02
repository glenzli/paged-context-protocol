use anyhow::{Context, Result};
use rusqlite::Connection;

pub(crate) fn initialize(connection: &mut Connection) -> Result<()> {
    connection
        .execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS pcp_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS pcp_scopes (
                namespace TEXT PRIMARY KEY,
                owner_id TEXT NOT NULL,
                scope_type TEXT NOT NULL,
                display_name TEXT NOT NULL,
                description TEXT,
                parent_namespace TEXT,
                visibility TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS pcp_pages (
                page_id TEXT PRIMARY KEY,
                current_revision_id TEXT,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS pcp_revisions (
                revision_id TEXT PRIMARY KEY,
                page_id TEXT NOT NULL REFERENCES pcp_pages(page_id),
                owner_id TEXT NOT NULL,
                namespace TEXT NOT NULL REFERENCES pcp_scopes(namespace),
                visibility TEXT NOT NULL,
                lifecycle_status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                observed_at TEXT,
                valid_from TEXT,
                valid_to TEXT,
                actor_type TEXT NOT NULL,
                actor_id TEXT NOT NULL,
                payload_media_type TEXT,
                payload_content TEXT,
                source_refs_json TEXT NOT NULL,
                facets_json TEXT,
                provenance_json TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS pcp_relations (
                relation_id TEXT PRIMARY KEY,
                from_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id),
                relation_type TEXT NOT NULL,
                to_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id),
                actor_type TEXT NOT NULL,
                actor_id TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS pcp_provenance_inputs (
                derived_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id),
                input_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id),
                created_at TEXT NOT NULL,
                PRIMARY KEY (derived_revision_id, input_revision_id)
            );

            CREATE TABLE IF NOT EXISTS pcp_summaries (
                summary_revision_id TEXT PRIMARY KEY,
                target_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id),
                summary_page_id TEXT REFERENCES pcp_pages(page_id),
                previous_summary_revision_id TEXT REFERENCES pcp_summaries(summary_revision_id),
                content TEXT NOT NULL,
                actor_type TEXT NOT NULL,
                actor_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                tool_or_model TEXT,
                provenance_json TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS pcp_summary_heads (
                target_revision_id TEXT PRIMARY KEY REFERENCES pcp_revisions(revision_id),
                current_summary_revision_id TEXT NOT NULL
                    REFERENCES pcp_summaries(summary_revision_id)
            );

            CREATE TABLE IF NOT EXISTS pcp_summary_idempotency (
                actor_id TEXT NOT NULL,
                idempotency_key TEXT NOT NULL,
                target_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id),
                result_summary_revision_id TEXT NOT NULL
                    REFERENCES pcp_summaries(summary_revision_id),
                created_at TEXT NOT NULL,
                PRIMARY KEY (actor_id, idempotency_key)
            );

            CREATE TABLE IF NOT EXISTS pcp_summary_assessments (
                target_revision_id TEXT PRIMARY KEY REFERENCES pcp_revisions(revision_id),
                policy_version TEXT NOT NULL,
                outcome TEXT NOT NULL,
                assessed_at TEXT NOT NULL,
                tool_or_model TEXT
            );

            CREATE TABLE IF NOT EXISTS pcp_validity_assessments (
                assessment_id TEXT PRIMARY KEY,
                previous_assessment_id TEXT
                    REFERENCES pcp_validity_assessments(assessment_id),
                target_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id),
                standing TEXT NOT NULL,
                rationale TEXT NOT NULL,
                scope TEXT,
                assessed_at TEXT NOT NULL,
                actor_type TEXT NOT NULL,
                actor_id TEXT NOT NULL,
                tool_or_model TEXT,
                basis_revision_ids_json TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS pcp_validity_heads (
                target_revision_id TEXT PRIMARY KEY REFERENCES pcp_revisions(revision_id),
                current_assessment_id TEXT NOT NULL
                    REFERENCES pcp_validity_assessments(assessment_id)
            );

            CREATE TABLE IF NOT EXISTS pcp_validity_idempotency (
                actor_id TEXT NOT NULL,
                idempotency_key TEXT NOT NULL,
                target_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id),
                result_assessment_id TEXT NOT NULL
                    REFERENCES pcp_validity_assessments(assessment_id),
                created_at TEXT NOT NULL,
                PRIMARY KEY (actor_id, idempotency_key)
            );

            CREATE TABLE IF NOT EXISTS pcp_idempotency (
                actor_id TEXT NOT NULL,
                operation TEXT NOT NULL,
                idempotency_key TEXT NOT NULL,
                result_page_id TEXT,
                result_revision_id TEXT,
                result_relation_id TEXT,
                created_at TEXT NOT NULL,
                PRIMARY KEY (actor_id, operation, idempotency_key)
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS pcp_revision_fts USING fts5(
                revision_id UNINDEXED,
                page_id UNINDEXED,
                namespace UNINDEXED,
                payload_content,
                facets_text,
                tokenize = 'unicode61'
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS pcp_summary_fts USING fts5(
                summary_revision_id UNINDEXED,
                target_revision_id UNINDEXED,
                content,
                tokenize = 'unicode61'
            );

            CREATE INDEX IF NOT EXISTS pcp_revisions_page
                ON pcp_revisions(page_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS pcp_revisions_namespace
                ON pcp_revisions(namespace, created_at DESC);
            CREATE INDEX IF NOT EXISTS pcp_relations_from
                ON pcp_relations(from_revision_id, relation_type);
            CREATE INDEX IF NOT EXISTS pcp_relations_to
                ON pcp_relations(to_revision_id, relation_type);
            CREATE INDEX IF NOT EXISTS pcp_provenance_inputs_input
                ON pcp_provenance_inputs(input_revision_id, derived_revision_id);
            CREATE INDEX IF NOT EXISTS pcp_summaries_target
                ON pcp_summaries(target_revision_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS pcp_summary_assessments_policy
                ON pcp_summary_assessments(policy_version, assessed_at DESC);
            CREATE INDEX IF NOT EXISTS pcp_validity_target
                ON pcp_validity_assessments(target_revision_id, assessed_at DESC);
            CREATE INDEX IF NOT EXISTS pcp_validity_standing
                ON pcp_validity_assessments(standing, assessed_at DESC);

            INSERT OR IGNORE INTO pcp_metadata (key, value)
            VALUES ('provenance_input_index_version', '0');

            INSERT OR IGNORE INTO pcp_provenance_inputs (
                derived_revision_id, input_revision_id, created_at
            )
            SELECT revision.revision_id, CAST(input.value AS TEXT), revision.created_at
            FROM pcp_revisions revision
            CROSS JOIN json_each(revision.provenance_json) event
            CROSS JOIN json_each(event.value, '$.inputRevisionIds') input
            JOIN pcp_revisions source
              ON source.revision_id = CAST(input.value AS TEXT)
            WHERE input.type = 'text'
              AND (
                SELECT value FROM pcp_metadata
                WHERE key = 'provenance_input_index_version'
              ) = '0';

            UPDATE pcp_metadata
            SET value = '1'
            WHERE key = 'provenance_input_index_version' AND value = '0';
            "#,
        )
        .context("initialize PCP schema")?;

    crate::summary_migration::migrate(connection)?;

    connection
        .execute(
            "
            INSERT OR IGNORE INTO pcp_metadata (key, value)
            VALUES ('owner_id', 'usr_' || lower(hex(randomblob(16))))
            ",
            [],
        )
        .context("initialize PCP owner identity")?;
    Ok(())
}

pub(crate) fn owner_id(connection: &Connection) -> Result<String> {
    connection
        .query_row(
            "SELECT value FROM pcp_metadata WHERE key = 'owner_id'",
            [],
            |row| row.get(0),
        )
        .context("read PCP owner identity")
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::initialize;

    #[test]
    fn migrates_legacy_summary_sidecars_into_the_page_dag() {
        let mut connection = Connection::open_in_memory().expect("open legacy database");
        connection
            .execute_batch(
                r#"
                CREATE TABLE pcp_scopes (
                    namespace TEXT PRIMARY KEY, owner_id TEXT NOT NULL,
                    scope_type TEXT NOT NULL, display_name TEXT NOT NULL,
                    description TEXT, parent_namespace TEXT, visibility TEXT NOT NULL,
                    created_at TEXT NOT NULL, updated_at TEXT NOT NULL
                );
                CREATE TABLE pcp_pages (
                    page_id TEXT PRIMARY KEY, current_revision_id TEXT, created_at TEXT NOT NULL
                );
                CREATE TABLE pcp_revisions (
                    revision_id TEXT PRIMARY KEY,
                    page_id TEXT NOT NULL REFERENCES pcp_pages(page_id),
                    owner_id TEXT NOT NULL, namespace TEXT NOT NULL REFERENCES pcp_scopes(namespace),
                    visibility TEXT NOT NULL, lifecycle_status TEXT NOT NULL,
                    created_at TEXT NOT NULL, observed_at TEXT, valid_from TEXT, valid_to TEXT,
                    actor_type TEXT NOT NULL, actor_id TEXT NOT NULL,
                    payload_media_type TEXT, payload_content TEXT,
                    source_refs_json TEXT NOT NULL, facets_json TEXT,
                    provenance_json TEXT NOT NULL
                );
                CREATE TABLE pcp_summaries (
                    summary_revision_id TEXT PRIMARY KEY,
                    target_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id),
                    previous_summary_revision_id TEXT REFERENCES pcp_summaries(summary_revision_id),
                    content TEXT NOT NULL, actor_type TEXT NOT NULL, actor_id TEXT NOT NULL,
                    created_at TEXT NOT NULL, tool_or_model TEXT, provenance_json TEXT NOT NULL
                );
                CREATE TABLE pcp_summary_heads (
                    target_revision_id TEXT PRIMARY KEY REFERENCES pcp_revisions(revision_id),
                    current_summary_revision_id TEXT NOT NULL REFERENCES pcp_summaries(summary_revision_id)
                );

                INSERT INTO pcp_scopes VALUES (
                    'conversation:test', 'usr_test', 'conversation', 'Test', NULL, NULL,
                    'private', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'
                );
                INSERT INTO pcp_pages VALUES ('pg_target', 'rev_target', '2026-01-01T00:00:00Z');
                INSERT INTO pcp_revisions VALUES (
                    'rev_target', 'pg_target', 'usr_test', 'conversation:test', 'private',
                    'active', '2026-01-01T00:00:00Z', NULL, NULL, NULL,
                    'user', 'usr_test', 'text/markdown', 'Long target detail', '[]',
                    '{"kind":"conversation_event"}', '[]'
                );
                INSERT INTO pcp_summaries VALUES (
                    'sumrev_legacy', 'rev_target', NULL, 'Compact legacy route',
                    'model', 'model:test', '2026-01-02T00:00:00Z', 'small-model',
                    '[{"operation":"summarize","actor":{"actorType":"model","actorId":"model:test"},"timestamp":"2026-01-02T00:00:00Z","inputRevisionIds":["rev_target"],"toolOrModel":"small-model"}]'
                );
                INSERT INTO pcp_summary_heads VALUES ('rev_target', 'sumrev_legacy');
                "#,
            )
            .expect("seed legacy Summary sidecar");

        initialize(&mut connection).expect("migrate legacy Summary");
        initialize(&mut connection).expect("repeat migration idempotently");

        let summary_page_id: String = connection
            .query_row(
                "SELECT summary_page_id FROM pcp_summaries WHERE summary_revision_id = 'sumrev_legacy'",
                [],
                |row| row.get(0),
            )
            .expect("read migrated Summary Page id");
        let current_revision_id: String = connection
            .query_row(
                "SELECT current_revision_id FROM pcp_pages WHERE page_id = ?1",
                [&summary_page_id],
                |row| row.get(0),
            )
            .expect("read migrated Summary Page");
        assert_eq!(current_revision_id, "sumrev_legacy");
        let summarizes_edges: i64 = connection
            .query_row(
                "SELECT count(*) FROM pcp_relations WHERE from_revision_id = 'sumrev_legacy' AND relation_type = 'summarizes' AND to_revision_id = 'rev_target'",
                [],
                |row| row.get(0),
            )
            .expect("count migrated summarizes edge");
        assert_eq!(summarizes_edges, 1);
        let provenance_edges: i64 = connection
            .query_row(
                "SELECT count(*) FROM pcp_provenance_inputs WHERE derived_revision_id = 'sumrev_legacy' AND input_revision_id = 'rev_target'",
                [],
                |row| row.get(0),
            )
            .expect("count migrated provenance edge");
        assert_eq!(provenance_edges, 1);
    }
}
