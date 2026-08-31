use std::collections::HashSet;

use anyhow::{Context, Result};
use pcp_core::{
    Actor, ActorType, AssessPageValidityRequest, PagePayload, PageValidity, ProvenanceEvent,
    ValidityStanding, WriteValidityResult,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::json;

use crate::{
    store::SqlitePcpStore,
    write::{insert_relation, insert_revision, now, random_id},
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

            let resolved_target_page: String = transaction.query_row(
                "SELECT page_id FROM pcp_revisions WHERE revision_id = ?1",
                [&request.target_revision_id],
                |row| row.get(0),
            )?;
            anyhow::ensure!(
                resolved_target_page == request.target_page_id,
                "target Revision does not belong to target Page"
            );
            let current = current_assessment(&transaction, &request.target_page_id)?;
            let current_revision_id = current
                .as_ref()
                .map(|(revision_id, _)| revision_id.clone());
            match (&current_revision_id, &request.expected_assessment_revision_id) {
                (None, None) => {}
                (Some(_), None) => {}
                (Some(current), Some(expected)) if current == expected => {}
                (None, Some(expected)) => {
                    anyhow::bail!(
                        "validity conflict: expected {expected}, but the Revision has no assessment"
                    );
                }
                (Some(current), Some(expected)) => {
                    anyhow::bail!(
                        "validity conflict: expected {expected}, current assessment Page is {current}",
                    );
                }
            }

            let assessment_id = random_id(&transaction, "rev_")?;
            let physical_page_id = current
                .as_ref()
                .map(|(_, page_id)| page_id.clone())
                .unwrap_or(random_id(&transaction, "pg_")?);
            let assessed_at = now();
            let namespace: String = transaction
                .query_row(
                    "SELECT namespace FROM pcp_revisions WHERE revision_id = ?1",
                    [&request.target_revision_id],
                    |row| row.get(0),
                )
                .context("read PCP assessment target metadata")?;
            if current.is_none() {
                transaction
                    .execute(
                    "INSERT INTO pcp_pages (
                        page_id, current_revision_id, created_at, namespace,
                        kind, mutability, lifecycle_status, updated_at
                     ) VALUES (?1, NULL, ?2, ?3, 'validity_assessment',
                               'revisioned', 'active', ?2)",
                    params![physical_page_id, assessed_at, namespace],
                )
                    .context("create PCP validity Page")?;
            }
            let mut input_page_ids = request.basis_revision_ids.clone();
            input_page_ids.push(request.target_revision_id.clone());
            input_page_ids.sort();
            input_page_ids.dedup();
            let provenance = vec![ProvenanceEvent {
                operation: "assess".to_owned(),
                actor: request.created_by.clone(),
                timestamp: assessed_at.clone(),
                input_revision_ids: input_page_ids,
                tool_or_model: request.tool_or_model.clone(),
                reason: None,
            }];
            let payload = PagePayload {
                media_type: "text/markdown".to_owned(),
                content: request.rationale.trim().to_owned(),
            };
            let facets = json!({
                "standing": request.standing.as_str(),
                "scope": request.scope.clone(),
            });
            insert_revision(
                &transaction,
                &physical_page_id,
                &assessment_id,
                &namespace,
                "active",
                &assessed_at,
                Some(&assessed_at),
                None,
                None,
                None,
                &request.created_by,
                Some(&payload),
                &[],
                Some(&facets),
                &provenance,
            )?;
            if current.is_none() {
                insert_relation(
                    &transaction,
                    &assessment_id,
                    "assesses",
                    &request.target_revision_id,
                    &request.created_by,
                    &assessed_at,
                )?;
            }
            let published = transaction
                .execute(
                    "UPDATE pcp_pages
                     SET current_revision_id = ?2, updated_at = ?3
                     WHERE page_id = ?1
                       AND (current_revision_id = ?4 OR (current_revision_id IS NULL AND ?4 IS NULL))",
                    params![physical_page_id, assessment_id, assessed_at, current_revision_id],
                )
                .context("publish PCP validity Revision")?;
            anyhow::ensure!(published == 1, "validity Page changed during publication");
            transaction
                .execute(
                    "
                    INSERT INTO pcp_validity_assessments (
                        assessment_revision_id, target_revision_id
                    ) VALUES (?1, ?2)
                    ",
                    params![assessment_id, request.target_revision_id],
                )
                .context("insert PCP validity assessment")?;
            transaction
                .execute(
                    "
                    INSERT INTO pcp_validity_heads (
                        target_page_id, assessment_page_id
                    ) VALUES (?1, ?2)
                    ON CONFLICT(target_page_id) DO UPDATE SET
                        assessment_page_id = excluded.assessment_page_id
                    ",
                    params![request.target_page_id, physical_page_id],
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
                target_page_id: request.target_page_id,
                target_revision_id: request.target_revision_id,
                assessment_page_id: physical_page_id,
                assessment_revision_id: assessment_id,
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
            SELECT assessment.assessment_revision_id,
                   assessment_revision.previous_revision_id,
                   assessment.target_revision_id,
                   json_extract(assessment_revision.facets_json, '$.standing'),
                   assessment_revision.payload_content,
                   json_extract(assessment_revision.facets_json, '$.scope'),
                   assessment_revision.created_at,
                   assessment_revision.actor_type, assessment_revision.actor_id,
                   json_extract(assessment_revision.provenance_json, '$[#-1].toolOrModel'),
                   assessment_revision.provenance_json,
                   assessment_revision.page_id, target_revision.page_id
            FROM pcp_validity_heads head
            JOIN pcp_pages assessment_page
              ON assessment_page.page_id = head.assessment_page_id
            JOIN pcp_validity_assessments assessment
              ON assessment.assessment_revision_id = assessment_page.current_revision_id
            JOIN pcp_revisions assessment_revision
              ON assessment_revision.revision_id = assessment.assessment_revision_id
            JOIN pcp_revisions target_revision
              ON target_revision.revision_id = assessment.target_revision_id
            WHERE head.target_page_id = (
                SELECT page_id FROM pcp_revisions WHERE revision_id = ?1
            )
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
            SELECT assessment.assessment_revision_id,
                   assessment_revision.previous_revision_id,
                   assessment.target_revision_id,
                   json_extract(assessment_revision.facets_json, '$.standing'),
                   assessment_revision.payload_content,
                   json_extract(assessment_revision.facets_json, '$.scope'),
                   assessment_revision.created_at,
                   assessment_revision.actor_type, assessment_revision.actor_id,
                   json_extract(assessment_revision.provenance_json, '$[#-1].toolOrModel'),
                   assessment_revision.provenance_json,
                   assessment_revision.page_id, target_revision.page_id
            FROM pcp_validity_heads head
            JOIN pcp_revisions current_target
              ON current_target.page_id = head.target_page_id
             AND current_target.revision_id = ?1
            JOIN pcp_revisions assessment_revision
              ON assessment_revision.page_id = head.assessment_page_id
            JOIN pcp_validity_assessments assessment
              ON assessment.assessment_revision_id = assessment_revision.revision_id
            JOIN pcp_revisions target_revision
              ON target_revision.revision_id = assessment.target_revision_id
            JOIN pcp_pages assessment_page
              ON assessment_page.page_id = head.assessment_page_id
            WHERE assessment_revision.revision_id <> assessment_page.current_revision_id
            ORDER BY assessment_revision.created_at DESC,
                     assessment_revision.revision_id DESC
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

fn current_assessment(
    transaction: &Transaction<'_>,
    target_page_id: &str,
) -> Result<Option<(String, String)>> {
    transaction
        .query_row(
            "
            SELECT assessment_page.current_revision_id, head.assessment_page_id
            FROM pcp_validity_heads head
            JOIN pcp_pages assessment_page
              ON assessment_page.page_id = head.assessment_page_id
            WHERE head.target_page_id = ?1
            ",
            [target_page_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
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
            SELECT i.target_revision_id, i.result_assessment_id,
                   target.page_id, assessment.page_id
            FROM pcp_validity_idempotency i
            JOIN pcp_revisions target ON target.revision_id = i.target_revision_id
            JOIN pcp_revisions assessment ON assessment.revision_id = i.result_assessment_id
            WHERE i.actor_id = ?1 AND i.idempotency_key = ?2
            ",
            params![actor_id, key],
            |row| {
                Ok(WriteValidityResult {
                    target_page_id: row.get(2)?,
                    target_revision_id: row.get(0)?,
                    assessment_page_id: row.get(3)?,
                    assessment_revision_id: row.get(1)?,
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
    let provenance_json: String = row.get(10)?;
    let provenance =
        serde_json::from_str::<Vec<ProvenanceEvent>>(&provenance_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                10,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
    let target_revision_id: String = row.get(2)?;
    let mut basis_revision_ids = provenance
        .into_iter()
        .flat_map(|event| event.input_revision_ids)
        .filter(|revision_id| revision_id != &target_revision_id)
        .collect::<Vec<_>>();
    basis_revision_ids.sort();
    basis_revision_ids.dedup();
    Ok(PageValidity {
        assessment_page_id: row.get(11)?,
        assessment_revision_id: row.get(0)?,
        previous_assessment_revision_id: row.get(1)?,
        target_page_id: row.get(12)?,
        target_revision_id,
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
