use anyhow::{Context, Result};
use pcp_core::{Actor, ActorType, PagePayload, ProvenanceEvent};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

use crate::write::{insert_relation, insert_revision};

const MIGRATION_KEY: &str = "immutable_page_model_version";
const MIGRATION_VERSION: &str = "1";

pub(crate) fn migrate(connection: &mut Connection) -> Result<()> {
    connection
        .execute_batch(
            "
            CREATE TABLE IF NOT EXISTS pcp_refs (
                ref_id TEXT PRIMARY KEY,
                namespace TEXT NOT NULL REFERENCES pcp_scopes(namespace),
                head_page_id TEXT NOT NULL REFERENCES pcp_revisions(revision_id),
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS pcp_refs_head
                ON pcp_refs(head_page_id);

            INSERT OR IGNORE INTO pcp_metadata (key, value)
            VALUES ('immutable_page_model_version', '0');
            ",
        )
        .context("initialize immutable PCP Page support")?;

    let version: String = connection
        .query_row(
            "SELECT value FROM pcp_metadata WHERE key = ?1",
            [MIGRATION_KEY],
            |row| row.get(0),
        )
        .context("read immutable PCP Page migration version")?;
    if version == MIGRATION_VERSION {
        return Ok(());
    }
    if version != "0" {
        anyhow::bail!("unsupported immutable PCP Page migration version: {version}");
    }

    let transaction = connection
        .transaction()
        .context("start immutable PCP Page migration")?;

    transaction
        .execute(
            "
            INSERT OR IGNORE INTO pcp_refs (
                ref_id, namespace, head_page_id, created_at, updated_at
            )
            SELECT page.page_id, revision.namespace, page.current_revision_id,
                   page.created_at, revision.created_at
            FROM pcp_pages page
            JOIN pcp_revisions revision
              ON revision.revision_id = page.current_revision_id
            ",
            [],
        )
        .context("migrate legacy PCP Page heads into Refs")?;

    backfill_supersedes_relations(&transaction)?;
    materialize_validity_pages(&transaction)?;

    transaction
        .execute(
            "UPDATE pcp_metadata SET value = ?2 WHERE key = ?1",
            params![MIGRATION_KEY, MIGRATION_VERSION],
        )
        .context("publish immutable PCP Page migration")?;
    transaction
        .commit()
        .context("commit immutable PCP Page migration")?;
    Ok(())
}

fn backfill_supersedes_relations(transaction: &rusqlite::Transaction<'_>) -> Result<()> {
    transaction
        .execute(
            "
            WITH ordered AS (
                SELECT revision_id, page_id, actor_type, actor_id, created_at,
                       lag(revision_id) OVER (
                           PARTITION BY page_id
                           ORDER BY created_at, revision_id
                       ) AS previous_page_id
                FROM pcp_revisions
            )
            INSERT INTO pcp_relations (
                relation_id, from_revision_id, relation_type, to_revision_id,
                actor_type, actor_id, created_at
            )
            SELECT 'rel_' || lower(hex(randomblob(16))), revision_id, 'supersedes',
                   previous_page_id, actor_type, actor_id, created_at
            FROM ordered
            WHERE previous_page_id IS NOT NULL
              AND NOT EXISTS (
                  SELECT 1
                  FROM pcp_relations relation
                  WHERE relation.from_revision_id = ordered.revision_id
                    AND relation.relation_type = 'supersedes'
                    AND relation.to_revision_id = ordered.previous_page_id
              )
            ",
            [],
        )
        .context("backfill immutable PCP Page supersedes relations")?;
    Ok(())
}

#[derive(Debug)]
struct LegacyValidity {
    assessment_id: String,
    previous_assessment_id: Option<String>,
    target_page_id: String,
    standing: String,
    rationale: String,
    scope: Option<String>,
    assessed_at: String,
    actor: Actor,
    tool_or_model: Option<String>,
    basis_page_ids: Vec<String>,
    owner_id: String,
    namespace: String,
    visibility: String,
}

fn materialize_validity_pages(transaction: &rusqlite::Transaction<'_>) -> Result<()> {
    let assessments = {
        let mut statement = transaction
            .prepare(
                "
                SELECT assessment.assessment_id, assessment.previous_assessment_id,
                       assessment.target_revision_id, assessment.standing,
                       assessment.rationale, assessment.scope, assessment.assessed_at,
                       assessment.actor_type, assessment.actor_id,
                       assessment.tool_or_model, assessment.basis_revision_ids_json,
                       target.owner_id, target.namespace, target.visibility
                FROM pcp_validity_assessments assessment
                JOIN pcp_revisions target
                  ON target.revision_id = assessment.target_revision_id
                ORDER BY assessment.assessed_at, assessment.assessment_id
                ",
            )
            .context("prepare legacy PCP validity migration")?;
        let rows = statement
            .query_map([], |row| {
                let actor_type: String = row.get(7)?;
                let basis_json: String = row.get(10)?;
                Ok(LegacyValidity {
                    assessment_id: row.get(0)?,
                    previous_assessment_id: row.get(1)?,
                    target_page_id: row.get(2)?,
                    standing: row.get(3)?,
                    rationale: row.get(4)?,
                    scope: row.get(5)?,
                    assessed_at: row.get(6)?,
                    actor: Actor {
                        actor_type: ActorType::parse(&actor_type).unwrap_or(ActorType::System),
                        actor_id: row.get(8)?,
                    },
                    tool_or_model: row.get(9)?,
                    basis_page_ids: serde_json::from_str(&basis_json).unwrap_or_default(),
                    owner_id: row.get(11)?,
                    namespace: row.get(12)?,
                    visibility: row.get(13)?,
                })
            })
            .context("query legacy PCP validity assessments")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("collect legacy PCP validity assessments")?;
        rows
    };

    for assessment in assessments {
        let already_materialized = transaction
            .query_row(
                "SELECT 1 FROM pcp_revisions WHERE revision_id = ?1",
                [&assessment.assessment_id],
                |_| Ok(()),
            )
            .optional()
            .context("check materialized PCP validity Page")?
            .is_some();
        if already_materialized {
            continue;
        }

        let physical_page_id = format!("pg_assessment_{}", assessment.assessment_id);
        transaction
            .execute(
                "
                INSERT INTO pcp_pages (page_id, current_revision_id, created_at)
                VALUES (?1, NULL, ?2)
                ",
                params![physical_page_id, assessment.assessed_at],
            )
            .context("create immutable PCP validity Page")?;

        let mut inputs = assessment.basis_page_ids.clone();
        inputs.push(assessment.target_page_id.clone());
        inputs.sort();
        inputs.dedup();
        let provenance = vec![ProvenanceEvent {
            operation: "assess".to_owned(),
            actor: assessment.actor.clone(),
            timestamp: assessment.assessed_at.clone(),
            input_revision_ids: inputs,
            tool_or_model: assessment.tool_or_model.clone(),
        }];
        let payload = PagePayload {
            media_type: "text/markdown".to_owned(),
            content: assessment.rationale.clone(),
        };
        let facets = json!({
            "kind": "validity_assessment",
            "standing": assessment.standing,
            "scope": assessment.scope,
            "targetPageId": assessment.target_page_id,
        });
        insert_revision(
            transaction,
            &physical_page_id,
            &assessment.assessment_id,
            &assessment.owner_id,
            &assessment.namespace,
            &assessment.visibility,
            "active",
            &assessment.assessed_at,
            Some(&assessment.assessed_at),
            None,
            None,
            &assessment.actor,
            Some(&payload),
            &[],
            Some(&facets),
            &provenance,
        )?;
        transaction
            .execute(
                "UPDATE pcp_pages SET current_revision_id = ?2 WHERE page_id = ?1",
                params![physical_page_id, assessment.assessment_id],
            )
            .context("publish immutable PCP validity Page")?;

        link_if_missing(
            transaction,
            &assessment.assessment_id,
            "assesses",
            &assessment.target_page_id,
            &assessment.actor,
            &assessment.assessed_at,
        )?;
        for basis_page_id in assessment.basis_page_ids {
            link_if_missing(
                transaction,
                &assessment.assessment_id,
                "derived_from",
                &basis_page_id,
                &assessment.actor,
                &assessment.assessed_at,
            )?;
        }
        if let Some(previous) = assessment.previous_assessment_id {
            link_if_missing(
                transaction,
                &assessment.assessment_id,
                "supersedes",
                &previous,
                &assessment.actor,
                &assessment.assessed_at,
            )?;
        }
    }
    Ok(())
}

fn link_if_missing(
    transaction: &rusqlite::Transaction<'_>,
    from_page_id: &str,
    relation_type: &str,
    to_page_id: &str,
    actor: &Actor,
    created_at: &str,
) -> Result<()> {
    let exists = transaction
        .query_row(
            "
            SELECT 1 FROM pcp_relations
            WHERE from_revision_id = ?1 AND relation_type = ?2 AND to_revision_id = ?3
            ",
            params![from_page_id, relation_type, to_page_id],
            |_| Ok(()),
        )
        .optional()
        .context("check migrated PCP relation")?
        .is_some();
    if !exists {
        insert_relation(
            transaction,
            from_page_id,
            relation_type,
            to_page_id,
            actor,
            created_at,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::migrate;
    use crate::schema;

    #[test]
    fn migration_is_idempotent_on_an_empty_store() {
        let mut connection = Connection::open_in_memory().expect("open database");
        schema::initialize(&mut connection).expect("initialize schema");
        migrate(&mut connection).expect("repeat immutable migration");
        let version: String = connection
            .query_row(
                "SELECT value FROM pcp_metadata WHERE key = 'immutable_page_model_version'",
                [],
                |row| row.get(0),
            )
            .expect("migration version");
        assert_eq!(version, "1");
    }
}
