use std::collections::HashSet;

use anyhow::{Context, Result};
use pcp_core::{
    Actor, ActorType, AssessPageValidityRequest, PageValidity, ValidityStanding,
    WriteValidityResult,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::{
    store::SqlitePcpStore,
    write::{now, random_id},
};

const MAX_RATIONALE_CHARS: usize = 2_000;
const MAX_SCOPE_CHARS: usize = 1_000;
const MAX_BASIS_REVISIONS: usize = 100;

impl SqlitePcpStore {
    pub async fn assess_page_validity(
        &self,
        request: AssessPageValidityRequest,
        allowed_scopes: Vec<String>,
    ) -> Result<WriteValidityResult> {
        validate_assessment(&request)?;
        let allowed_scopes = allowed_scopes.into_iter().collect::<HashSet<_>>();
        self.run("validity assessment write", move |mut connection| {
            let transaction = connection
                .transaction()
                .context("start PCP validity assessment")?;
            ensure_revision_access(&transaction, &request.target_revision_id, &allowed_scopes)?;
            for revision_id in &request.basis_revision_ids {
                ensure_revision_access(&transaction, revision_id, &allowed_scopes)?;
            }

            if let Some(existing) = lookup_idempotency(
                &transaction,
                &request.created_by.actor_id,
                request.idempotency_key.as_deref(),
            )? {
                if existing.target_revision_id != request.target_revision_id {
                    anyhow::bail!("validity idempotency key was already used for another Revision");
                }
                return Ok(existing);
            }

            let current = current_assessment_id(&transaction, &request.target_revision_id)?;
            match (&current, &request.expected_assessment_id) {
                (None, None) => {}
                (Some(current), Some(expected)) if current == expected => {}
                (None, Some(expected)) => {
                    anyhow::bail!(
                        "validity conflict: expected {expected}, but the Revision has no assessment"
                    );
                }
                (Some(current), expected) => {
                    anyhow::bail!(
                        "validity conflict: expected {}, current assessment is {current}",
                        expected.as_deref().unwrap_or("none")
                    );
                }
            }

            let assessment_id = random_id(&transaction, "valid_")?;
            let assessed_at = now();
            let basis_revision_ids_json = serde_json::to_string(&request.basis_revision_ids)
                .context("encode PCP validity basis")?;
            transaction
                .execute(
                    "
                    INSERT INTO pcp_validity_assessments (
                        assessment_id, previous_assessment_id, target_revision_id,
                        standing, rationale, scope, assessed_at, actor_type, actor_id,
                        tool_or_model, basis_revision_ids_json
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                    ",
                    params![
                        assessment_id,
                        current,
                        request.target_revision_id,
                        request.standing.as_str(),
                        request.rationale.trim(),
                        request.scope.as_deref().map(str::trim),
                        assessed_at,
                        request.created_by.actor_type.as_str(),
                        request.created_by.actor_id,
                        request.tool_or_model,
                        basis_revision_ids_json,
                    ],
                )
                .context("insert PCP validity assessment")?;
            transaction
                .execute(
                    "
                    INSERT INTO pcp_validity_heads (
                        target_revision_id, current_assessment_id
                    ) VALUES (?1, ?2)
                    ON CONFLICT(target_revision_id) DO UPDATE SET
                        current_assessment_id = excluded.current_assessment_id
                    ",
                    params![request.target_revision_id, assessment_id],
                )
                .context("publish PCP validity assessment")?;
            if let Some(key) = request.idempotency_key.as_deref() {
                transaction
                    .execute(
                        "
                        INSERT INTO pcp_validity_idempotency (
                            actor_id, idempotency_key, target_revision_id,
                            result_assessment_id, created_at
                        ) VALUES (?1, ?2, ?3, ?4, ?5)
                        ",
                        params![
                            request.created_by.actor_id,
                            key,
                            request.target_revision_id,
                            assessment_id,
                            assessed_at
                        ],
                    )
                    .context("record PCP validity idempotency")?;
            }
            transaction
                .commit()
                .context("commit PCP validity assessment")?;
            Ok(WriteValidityResult {
                target_revision_id: request.target_revision_id,
                assessment_id,
                created: true,
            })
        })
        .await
    }
}

pub(crate) fn current_validity(
    connection: &Connection,
    target_revision_id: &str,
) -> Result<Option<PageValidity>> {
    connection
        .query_row(
            "
            SELECT assessment.assessment_id, assessment.previous_assessment_id,
                   assessment.target_revision_id, assessment.standing,
                   assessment.rationale, assessment.scope, assessment.assessed_at,
                   assessment.actor_type, assessment.actor_id,
                   assessment.tool_or_model, assessment.basis_revision_ids_json
            FROM pcp_validity_heads head
            JOIN pcp_validity_assessments assessment
              ON assessment.assessment_id = head.current_assessment_id
            WHERE head.target_revision_id = ?1
            ",
            [target_revision_id],
            validity_from_row,
        )
        .optional()
        .context("read current PCP validity assessment")
}

pub(crate) fn validity_history(
    connection: &Connection,
    target_revision_id: &str,
) -> Result<Vec<PageValidity>> {
    let mut statement = connection
        .prepare(
            "
            WITH RECURSIVE chain(assessment_id, depth) AS (
                SELECT current_assessment_id, 0
                FROM pcp_validity_heads
                WHERE target_revision_id = ?1
                UNION ALL
                SELECT assessment.previous_assessment_id, chain.depth + 1
                FROM chain
                JOIN pcp_validity_assessments assessment
                  ON assessment.assessment_id = chain.assessment_id
                WHERE assessment.previous_assessment_id IS NOT NULL
            )
            SELECT assessment.assessment_id, assessment.previous_assessment_id,
                   assessment.target_revision_id, assessment.standing,
                   assessment.rationale, assessment.scope, assessment.assessed_at,
                   assessment.actor_type, assessment.actor_id,
                   assessment.tool_or_model, assessment.basis_revision_ids_json
            FROM chain
            JOIN pcp_validity_assessments assessment
              ON assessment.assessment_id = chain.assessment_id
            WHERE chain.depth > 0
            ORDER BY chain.depth
            LIMIT 20
            ",
        )
        .context("prepare PCP validity history")?;
    statement
        .query_map([target_revision_id], validity_from_row)
        .context("query PCP validity history")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collect PCP validity history")
}

fn validate_assessment(request: &AssessPageValidityRequest) -> Result<()> {
    if request.target_revision_id.trim().is_empty() {
        anyhow::bail!("PCP validity assessment requires a target Revision");
    }
    let rationale_chars = request.rationale.trim().chars().count();
    if rationale_chars == 0 || rationale_chars > MAX_RATIONALE_CHARS {
        anyhow::bail!("PCP validity rationale must contain 1-{MAX_RATIONALE_CHARS} characters");
    }
    if request
        .scope
        .as_deref()
        .is_some_and(|scope| scope.trim().chars().count() > MAX_SCOPE_CHARS)
    {
        anyhow::bail!("PCP validity scope exceeds {MAX_SCOPE_CHARS} characters");
    }
    if request.basis_revision_ids.is_empty() {
        anyhow::bail!("PCP validity assessment requires exact basis Revisions");
    }
    if request.basis_revision_ids.len() > MAX_BASIS_REVISIONS {
        anyhow::bail!("PCP validity assessment exceeds {MAX_BASIS_REVISIONS} basis Revisions");
    }
    Ok(())
}

fn current_assessment_id(
    transaction: &Transaction<'_>,
    target_revision_id: &str,
) -> Result<Option<String>> {
    transaction
        .query_row(
            "
            SELECT current_assessment_id
            FROM pcp_validity_heads
            WHERE target_revision_id = ?1
            ",
            [target_revision_id],
            |row| row.get(0),
        )
        .optional()
        .context("read current PCP validity assessment id")
}

fn lookup_idempotency(
    transaction: &Transaction<'_>,
    actor_id: &str,
    idempotency_key: Option<&str>,
) -> Result<Option<WriteValidityResult>> {
    let Some(key) = idempotency_key else {
        return Ok(None);
    };
    transaction
        .query_row(
            "
            SELECT target_revision_id, result_assessment_id
            FROM pcp_validity_idempotency
            WHERE actor_id = ?1 AND idempotency_key = ?2
            ",
            params![actor_id, key],
            |row| {
                Ok(WriteValidityResult {
                    target_revision_id: row.get(0)?,
                    assessment_id: row.get(1)?,
                    created: false,
                })
            },
        )
        .optional()
        .context("look up PCP validity idempotency")
}

fn ensure_revision_access(
    transaction: &Transaction<'_>,
    revision_id: &str,
    allowed_scopes: &HashSet<String>,
) -> Result<()> {
    let namespace: String = transaction
        .query_row(
            "SELECT namespace FROM pcp_revisions WHERE revision_id = ?1",
            [revision_id],
            |row| row.get(0),
        )
        .with_context(|| format!("find PCP Revision {revision_id}"))?;
    if !allowed_scopes.contains(&namespace) {
        anyhow::bail!("Revision is outside the authorized PCP Scopes");
    }
    Ok(())
}

fn validity_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PageValidity> {
    let standing_text: String = row.get(3)?;
    let standing = ValidityStanding::parse(&standing_text).ok_or_else(|| {
        rusqlite::Error::InvalidColumnType(3, "standing".to_owned(), rusqlite::types::Type::Text)
    })?;
    let actor_type_text: String = row.get(7)?;
    let actor_type = ActorType::parse(&actor_type_text).ok_or_else(|| {
        rusqlite::Error::InvalidColumnType(7, "actor_type".to_owned(), rusqlite::types::Type::Text)
    })?;
    let basis_json: String = row.get(10)?;
    let basis_revision_ids = serde_json::from_str(&basis_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(10, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(PageValidity {
        assessment_id: row.get(0)?,
        previous_assessment_id: row.get(1)?,
        target_revision_id: row.get(2)?,
        standing,
        rationale: row.get(4)?,
        scope: row.get(5)?,
        assessed_at: row.get(6)?,
        created_by: Actor {
            actor_type,
            actor_id: row.get(8)?,
        },
        tool_or_model: row.get(9)?,
        basis_revision_ids,
    })
}
