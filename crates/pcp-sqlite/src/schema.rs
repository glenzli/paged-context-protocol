use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};

const STORE_SCHEMA_VERSION: &str = "0.8.0-clean.3";
const LEGACY_DRAFT_SCHEMA_VERSION: &str = "0.8.0-draft";
const LEGACY_CLEAN_SCHEMA_VERSION: &str = "0.8.0-clean";
const LEGACY_CLEAN_ASSOCIATIONS_SCHEMA_VERSION: &str = "0.8.0-clean.1";
const LEGACY_CLEAN_TOPIC_EXTRACTIONS_SCHEMA_VERSION: &str = "0.8.0-clean.2";

pub(crate) fn initialize(connection: &mut Connection) -> Result<()> {
    connection
        .execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS pcp_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            ",
        )
        .context("initialize PCP metadata")?;
    let stored_version = connection
        .query_row(
            "SELECT value FROM pcp_metadata WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .context("read PCP Store schema version")?;
    match stored_version {
        Some(version) if version == STORE_SCHEMA_VERSION => {}
        Some(version) if version == LEGACY_DRAFT_SCHEMA_VERSION => {
            crate::migration::migrate_draft_to_clean(connection, STORE_SCHEMA_VERSION)?;
        }
        Some(version) if version == LEGACY_CLEAN_SCHEMA_VERSION => {
            crate::migration::migrate_clean_associations(connection, STORE_SCHEMA_VERSION)?;
        }
        Some(version) if version == LEGACY_CLEAN_ASSOCIATIONS_SCHEMA_VERSION => {
            crate::migration::migrate_clean_topic_extractions(connection, STORE_SCHEMA_VERSION)?;
        }
        Some(version) if version == LEGACY_CLEAN_TOPIC_EXTRACTIONS_SCHEMA_VERSION => {
            crate::migration::migrate_clean_content_governance(connection, STORE_SCHEMA_VERSION)?;
        }
        Some(version) => {
            anyhow::bail!("unsupported PCP Store schema {version}; expected {STORE_SCHEMA_VERSION}")
        }
        None => {
            let existing_tables: u32 = connection
                .query_row(
                    "
                    SELECT count(*)
                    FROM sqlite_master
                    WHERE type = 'table'
                      AND name LIKE 'pcp_%'
                      AND name != 'pcp_metadata'
                    ",
                    [],
                    |row| row.get(0),
                )
                .context("inspect PCP Store schema")?;
            anyhow::ensure!(
                existing_tables == 0,
                "PCP v0.8 requires a new Store; import original content instead of opening a pre-v0.8 database"
            );
            connection
                .execute(
                    "INSERT INTO pcp_metadata (key, value) VALUES ('schema_version', ?1)",
                    [STORE_SCHEMA_VERSION],
                )
                .context("record PCP Store schema version")?;
        }
    }
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
                display_name TEXT NOT NULL,
                description TEXT,
                parent_namespace TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS pcp_pages (
                page_id TEXT PRIMARY KEY,
                current_revision_id TEXT,
                created_at TEXT NOT NULL,
                namespace TEXT NOT NULL REFERENCES pcp_scopes(namespace),
                kind TEXT NOT NULL,
                mutability TEXT NOT NULL,
                lifecycle_status TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS pcp_revisions (
                revision_id TEXT PRIMARY KEY,
                page_id TEXT NOT NULL REFERENCES pcp_pages(page_id),
                namespace TEXT NOT NULL REFERENCES pcp_scopes(namespace),
                lifecycle_status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                observed_at TEXT,
                source_span_json TEXT,
                valid_from TEXT,
                valid_to TEXT,
                actor_type TEXT NOT NULL,
                actor_id TEXT NOT NULL,
                payload_media_type TEXT,
                payload_content TEXT,
                source_refs_json TEXT NOT NULL,
                facets_json TEXT,
                provenance_json TEXT NOT NULL,
                previous_revision_id TEXT
            );

            CREATE TABLE IF NOT EXISTS pcp_relations (
                relation_id TEXT PRIMARY KEY,
                relation_type TEXT NOT NULL,
                actor_type TEXT NOT NULL,
                actor_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                from_page_id TEXT NOT NULL REFERENCES pcp_pages(page_id),
                to_page_id TEXT NOT NULL REFERENCES pcp_pages(page_id),
                basis_revision_ids_json TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS pcp_relation_retractions (
                relation_id TEXT PRIMARY KEY REFERENCES pcp_relations(relation_id),
                retracted_actor_type TEXT NOT NULL,
                retracted_actor_id TEXT NOT NULL,
                retracted_at TEXT NOT NULL,
                reason TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS pcp_provenance_inputs (
                derived_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id),
                input_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id),
                created_at TEXT NOT NULL,
                PRIMARY KEY (derived_revision_id, input_revision_id)
            );

            CREATE TABLE IF NOT EXISTS pcp_summaries (
                summary_revision_id TEXT PRIMARY KEY REFERENCES pcp_revisions(revision_id),
                target_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id)
            );

            CREATE TABLE IF NOT EXISTS pcp_page_summary_heads (
                target_page_id TEXT PRIMARY KEY REFERENCES pcp_pages(page_id),
                summary_page_id TEXT NOT NULL UNIQUE REFERENCES pcp_pages(page_id)
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

            CREATE TABLE IF NOT EXISTS pcp_topic_extractions (
                topic_revision_id TEXT PRIMARY KEY REFERENCES pcp_revisions(revision_id),
                topic_page_id TEXT NOT NULL REFERENCES pcp_pages(page_id),
                namespace TEXT NOT NULL REFERENCES pcp_scopes(namespace),
                title TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS pcp_topic_extraction_members (
                topic_revision_id TEXT NOT NULL REFERENCES pcp_topic_extractions(topic_revision_id),
                topic_page_id TEXT NOT NULL REFERENCES pcp_pages(page_id),
                source_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id),
                source_page_id TEXT NOT NULL REFERENCES pcp_pages(page_id),
                position INTEGER NOT NULL,
                PRIMARY KEY (topic_revision_id, source_revision_id),
                UNIQUE (topic_revision_id, position)
            );

            CREATE TABLE IF NOT EXISTS pcp_validity_assessments (
                assessment_revision_id TEXT PRIMARY KEY REFERENCES pcp_revisions(revision_id),
                target_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id)
            );

            CREATE TABLE IF NOT EXISTS pcp_validity_heads (
                target_page_id TEXT PRIMARY KEY REFERENCES pcp_pages(page_id),
                assessment_page_id TEXT NOT NULL UNIQUE REFERENCES pcp_pages(page_id)
            );

            CREATE TABLE IF NOT EXISTS pcp_validity_idempotency (
                actor_id TEXT NOT NULL,
                idempotency_key TEXT NOT NULL,
                target_revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id),
                result_assessment_id TEXT NOT NULL
                    REFERENCES pcp_validity_assessments(assessment_revision_id),
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

            CREATE TABLE IF NOT EXISTS pcp_revision_retention_leases (
                lease_id TEXT PRIMARY KEY,
                page_id TEXT NOT NULL REFERENCES pcp_pages(page_id),
                revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id),
                namespace TEXT NOT NULL,
                holder_principal_id TEXT NOT NULL,
                reason TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                idempotency_key TEXT NOT NULL,
                UNIQUE (holder_principal_id, idempotency_key)
            );

            CREATE TABLE IF NOT EXISTS pcp_revision_collections (
                revision_id TEXT PRIMARY KEY,
                page_id TEXT NOT NULL,
                namespace TEXT NOT NULL,
                kind TEXT NOT NULL,
                original_created_at TEXT NOT NULL,
                previous_revision_id TEXT,
                collected_at TEXT NOT NULL,
                estimated_bytes INTEGER NOT NULL,
                collector_principal_id TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS pcp_page_packs (
                source_page_id TEXT PRIMARY KEY,
                source_revision_id TEXT NOT NULL UNIQUE,
                namespace TEXT NOT NULL,
                packed_page_id TEXT NOT NULL,
                packed_revision_id TEXT NOT NULL,
                position INTEGER NOT NULL,
                packed_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS pcp_access_log (
                event_id TEXT PRIMARY KEY,
                occurred_at TEXT NOT NULL,
                principal_json TEXT NOT NULL,
                session_id TEXT NOT NULL,
                operation TEXT NOT NULL,
                scopes_json TEXT NOT NULL,
                decision TEXT NOT NULL,
                detail TEXT,
                telemetry_json TEXT
            );

            CREATE TABLE IF NOT EXISTS pcp_query_audit (
                event_id TEXT PRIMARY KEY,
                occurred_at TEXT NOT NULL,
                principal_json TEXT NOT NULL,
                session_id TEXT NOT NULL,
                method TEXT NOT NULL,
                effort TEXT,
                scopes_json TEXT NOT NULL,
                decision TEXT NOT NULL,
                duration_ms INTEGER NOT NULL,
                anchor_count INTEGER NOT NULL,
                related_count INTEGER NOT NULL,
                context_chars INTEGER NOT NULL,
                semantic_indexed_count INTEGER,
                semantic_embedded_count INTEGER,
                router_rounds INTEGER,
                router_usage_json TEXT,
                failure_kind TEXT
            );

            CREATE TABLE IF NOT EXISTS pcp_runtime_usage (
                event_id TEXT PRIMARY KEY,
                occurred_at TEXT NOT NULL,
                principal_json TEXT NOT NULL,
                session_id TEXT NOT NULL,
                source TEXT NOT NULL,
                operation TEXT NOT NULL,
                scopes_json TEXT NOT NULL,
                duration_ms INTEGER NOT NULL,
                usage_json TEXT,
                failure_kind TEXT
            );

            CREATE TABLE IF NOT EXISTS pcp_page_lifecycle_events (
                event_id TEXT PRIMARY KEY,
                page_id TEXT NOT NULL REFERENCES pcp_pages(page_id),
                revision_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id),
                previous_status TEXT NOT NULL,
                next_status TEXT NOT NULL,
                actor_type TEXT NOT NULL,
                actor_id TEXT NOT NULL,
                reason TEXT,
                created_at TEXT NOT NULL
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
            CREATE INDEX IF NOT EXISTS pcp_pages_scope
                ON pcp_pages(namespace, updated_at DESC);
            CREATE INDEX IF NOT EXISTS pcp_pages_kind
                ON pcp_pages(kind, lifecycle_status, updated_at DESC);
            CREATE INDEX IF NOT EXISTS pcp_relations_page_from
                ON pcp_relations(from_page_id, relation_type);
            CREATE INDEX IF NOT EXISTS pcp_relations_page_to
                ON pcp_relations(to_page_id, relation_type);
            CREATE INDEX IF NOT EXISTS pcp_relation_retractions_time
                ON pcp_relation_retractions(retracted_at DESC, relation_id);
            CREATE INDEX IF NOT EXISTS pcp_provenance_inputs_input
                ON pcp_provenance_inputs(input_revision_id, derived_revision_id);
            CREATE INDEX IF NOT EXISTS pcp_summaries_target
                ON pcp_summaries(target_revision_id, summary_revision_id);
            CREATE INDEX IF NOT EXISTS pcp_summary_assessments_policy
                ON pcp_summary_assessments(policy_version, assessed_at DESC);
            CREATE INDEX IF NOT EXISTS pcp_topic_extraction_members_source
                ON pcp_topic_extraction_members(source_page_id, source_revision_id);
            CREATE INDEX IF NOT EXISTS pcp_validity_target
                ON pcp_validity_assessments(target_revision_id, assessment_revision_id);
            CREATE INDEX IF NOT EXISTS pcp_retention_leases_revision
                ON pcp_revision_retention_leases(revision_id, expires_at DESC);
            CREATE INDEX IF NOT EXISTS pcp_retention_leases_namespace
                ON pcp_revision_retention_leases(namespace, expires_at DESC);
            CREATE INDEX IF NOT EXISTS pcp_revision_collections_page
                ON pcp_revision_collections(page_id, collected_at DESC);
            CREATE INDEX IF NOT EXISTS pcp_page_packs_output
                ON pcp_page_packs(packed_page_id, position);
            CREATE INDEX IF NOT EXISTS pcp_validity_standing
                ON pcp_revisions(json_extract(facets_json, '$.standing'), created_at DESC)
                WHERE json_extract(facets_json, '$.standing') IS NOT NULL;
            CREATE INDEX IF NOT EXISTS pcp_access_log_time
                ON pcp_access_log(occurred_at DESC, event_id DESC);
            CREATE INDEX IF NOT EXISTS pcp_query_audit_time
                ON pcp_query_audit(occurred_at DESC, event_id DESC);
            CREATE INDEX IF NOT EXISTS pcp_runtime_usage_time
                ON pcp_runtime_usage(occurred_at DESC, event_id DESC);
            CREATE INDEX IF NOT EXISTS pcp_page_lifecycle_events_page
                ON pcp_page_lifecycle_events(page_id, created_at DESC);

            "#,
        )
        .context("initialize PCP schema")?;

    connection
        .execute(
            "
            INSERT OR IGNORE INTO pcp_metadata (key, value)
            VALUES ('identity_id', 'idn_' || lower(hex(randomblob(16))))
            ",
            [],
        )
        .context("initialize PCP Identity")?;
    Ok(())
}

pub(crate) fn identity_id(connection: &Connection) -> Result<String> {
    connection
        .query_row(
            "SELECT value FROM pcp_metadata WHERE key = 'identity_id'",
            [],
            |row| row.get(0),
        )
        .context("read PCP Identity")
}
