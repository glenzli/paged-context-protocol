use std::collections::HashSet;

use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use pcp_core::{
    ActorType, PagePayload, PageSummary, ProvenanceEvent, WriteSummaryRequest, WriteSummaryResult,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::json;

use crate::{
    store::SqlitePcpStore,
    write::{insert_relation, insert_revision},
};

pub(crate) const MAX_SUMMARY_CHARS: usize = 1_200;
pub const SUMMARY_POLICY_VERSION: &str = "sparse-index-v2";

impl SqlitePcpStore {
    pub async fn write_summary(
        &self,
        request: WriteSummaryRequest,
        allowed_scopes: Vec<String>,
    ) -> Result<WriteSummaryResult> {
        validate_summary(&request)?;
        let allowed_scopes = allowed_scopes.into_iter().collect::<HashSet<_>>();
        self.run("summary write", move |mut connection| {
            let transaction = connection
                .transaction()
                .context("start PCP summary write")?;
            ensure_revision_access(&transaction, &request.target_revision_id, &allowed_scopes)?;

            if let Some(existing) = lookup_idempotency(
                &transaction,
                &request.created_by.actor_id,
                request.idempotency_key.as_deref(),
            )? {
                if existing.target_revision_id != request.target_revision_id {
                    anyhow::bail!("summary idempotency key was already used for another Revision");
                }
                return Ok(existing);
            }

            let current = current_summary_identity(&transaction, &request.target_revision_id)?;
            let current_revision_id = current.as_ref().map(|(revision_id, _)| revision_id);
            match (current_revision_id, &request.expected_summary_revision_id) {
                (None, None) => {}
                (Some(current), Some(expected)) if current == expected => {}
                (None, Some(expected)) => {
                    anyhow::bail!(
                        "summary conflict: expected {expected}, but the Revision has no Summary"
                    );
                }
                (Some(current), expected) => {
                    anyhow::bail!(
                        "summary conflict: expected {}, current Summary Revision is {current}",
                        expected.as_deref().unwrap_or("none")
                    );
                }
            }

            let timestamp = now();
            let summary_revision_id = random_id(&transaction, "rev_")?;
            let summary_page_id = match current.as_ref() {
                Some((_, page_id)) => page_id.clone(),
                None => {
                    let page_id = random_id(&transaction, "pg_")?;
                    transaction
                        .execute(
                            "INSERT INTO pcp_pages (page_id, current_revision_id, created_at) VALUES (?1, NULL, ?2)",
                            params![page_id, timestamp],
                        )
                        .context("create PCP Summary Page")?;
                    page_id
                }
            };
            let provenance = complete_provenance(
                request.provenance,
                &request.target_revision_id,
                &request.created_by,
                request.tool_or_model.as_deref(),
                &timestamp,
            );
            ensure_provenance_access(&transaction, &provenance, &allowed_scopes)?;
            let provenance_json =
                serde_json::to_string(&provenance).context("encode PCP Summary provenance")?;
            let (owner_id, namespace, visibility): (String, String, String) = transaction
                .query_row(
                    "SELECT owner_id, namespace, visibility FROM pcp_revisions WHERE revision_id = ?1",
                    [&request.target_revision_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .context("read PCP Summary target metadata")?;
            let content = request.content.trim().to_owned();
            let payload = PagePayload {
                media_type: "text/markdown".to_owned(),
                content: content.clone(),
            };
            let facets = json!({
                "kind": "summary_projection",
                "targetRevisionId": request.target_revision_id,
            });
            insert_revision(
                &transaction,
                &summary_page_id,
                &summary_revision_id,
                &owner_id,
                &namespace,
                &visibility,
                "active",
                &timestamp,
                None,
                None,
                None,
                &request.created_by,
                Some(&payload),
                &[],
                Some(&facets),
                &provenance,
            )?;
            insert_relation(
                &transaction,
                &summary_revision_id,
                "summarizes",
                &request.target_revision_id,
                &request.created_by,
                &timestamp,
            )?;

            transaction
                .execute(
                    "
                    INSERT INTO pcp_summaries (
                        summary_revision_id, target_revision_id, summary_page_id,
                        previous_summary_revision_id, content, actor_type, actor_id,
                        created_at, tool_or_model, provenance_json
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                    ",
                    params![
                        summary_revision_id,
                        request.target_revision_id,
                        summary_page_id,
                        current_revision_id,
                        content,
                        request.created_by.actor_type.as_str(),
                        request.created_by.actor_id,
                        timestamp,
                        request.tool_or_model,
                        provenance_json,
                    ],
                )
                .context("insert PCP Summary Revision")?;
            transaction
                .execute(
                    "
                    INSERT INTO pcp_summary_heads (
                        target_revision_id, current_summary_revision_id
                    ) VALUES (?1, ?2)
                    ON CONFLICT(target_revision_id) DO UPDATE SET
                        current_summary_revision_id = excluded.current_summary_revision_id
                    ",
                    params![request.target_revision_id, summary_revision_id],
                )
                .context("publish PCP Summary Revision")?;
            transaction
                .execute(
                    "UPDATE pcp_pages SET current_revision_id = ?2 WHERE page_id = ?1",
                    params![summary_page_id, summary_revision_id],
                )
                .context("publish PCP Summary Page Revision")?;
            transaction
                .execute(
                    "
                    INSERT INTO pcp_summary_fts (
                        summary_revision_id, target_revision_id, content
                    ) VALUES (?1, ?2, ?3)
                    ",
                    params![
                        summary_revision_id,
                        request.target_revision_id,
                        request.content.trim()
                    ],
                )
                .context("index PCP Summary Revision")?;
            if let Some(key) = request.idempotency_key.as_deref() {
                transaction
                    .execute(
                        "
                        INSERT INTO pcp_summary_idempotency (
                            actor_id, idempotency_key, target_revision_id,
                            result_summary_revision_id, created_at
                        ) VALUES (?1, ?2, ?3, ?4, ?5)
                        ",
                        params![
                            request.created_by.actor_id,
                            key,
                            request.target_revision_id,
                            summary_revision_id,
                            timestamp
                        ],
                    )
                    .context("record PCP Summary idempotency")?;
            }
            transaction.commit().context("commit PCP Summary write")?;
            Ok(WriteSummaryResult {
                target_revision_id: request.target_revision_id,
                summary_page_id,
                summary_revision_id,
                created: true,
            })
        })
        .await
    }

    pub async fn next_summary_candidate(
        &self,
        allowed_scopes: Vec<String>,
        minimum_chars: usize,
        excluded_page_kinds: Vec<String>,
    ) -> Result<Option<String>> {
        if allowed_scopes.is_empty() {
            return Ok(None);
        }
        self.run("summary candidate discovery", move |connection| {
            let mut values = vec![rusqlite::types::Value::Text(
                SUMMARY_POLICY_VERSION.to_owned(),
            )];
            values.extend(
                allowed_scopes
                    .iter()
                    .cloned()
                    .map(rusqlite::types::Value::Text),
            );
            values.push(rusqlite::types::Value::Integer(minimum_chars as i64));
            let placeholders = (0..allowed_scopes.len())
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(",");
            let mut sql = format!(
                "
                SELECT r.revision_id
                FROM pcp_pages page
                JOIN pcp_revisions r ON r.revision_id = page.current_revision_id
                LEFT JOIN pcp_summary_heads summary
                  ON summary.target_revision_id = r.revision_id
                LEFT JOIN pcp_summary_assessments assessment
                  ON assessment.target_revision_id = r.revision_id
                 AND assessment.policy_version = ?
                WHERE r.namespace IN ({placeholders})
                  AND r.lifecycle_status = 'active'
                  AND r.payload_media_type LIKE 'text/%'
                  AND length(COALESCE(r.payload_content, '')) >= ?
                "
            );
            if !excluded_page_kinds.is_empty() {
                let placeholders = (0..excluded_page_kinds.len())
                    .map(|_| "?")
                    .collect::<Vec<_>>()
                    .join(",");
                sql.push_str(&format!(
                    " AND COALESCE(json_extract(r.facets_json, '$.kind'), '') NOT IN ({placeholders})"
                ));
                values.extend(
                    excluded_page_kinds
                        .into_iter()
                        .map(rusqlite::types::Value::Text),
                );
            }
            sql.push_str(
                " AND summary.target_revision_id IS NULL
                  AND assessment.target_revision_id IS NULL
                  ORDER BY COALESCE(r.observed_at, r.created_at) DESC, r.revision_id DESC
                  LIMIT 1",
            );
            connection
                .query_row(&sql, rusqlite::params_from_iter(values.iter()), |row| {
                    row.get(0)
                })
                .optional()
                .context("find PCP Summary candidate")
        })
        .await
    }

    pub async fn mark_summary_assessed(
        &self,
        target_revision_id: String,
        outcome: String,
        tool_or_model: Option<String>,
        allowed_scopes: Vec<String>,
    ) -> Result<()> {
        let allowed_scopes = allowed_scopes.into_iter().collect::<HashSet<_>>();
        self.run("summary assessment write", move |mut connection| {
            let transaction = connection
                .transaction()
                .context("start PCP Summary assessment")?;
            ensure_revision_access(&transaction, &target_revision_id, &allowed_scopes)?;
            transaction
                .execute(
                    "
                    INSERT INTO pcp_summary_assessments (
                        target_revision_id, policy_version, outcome, assessed_at,
                        tool_or_model
                    ) VALUES (?1, ?2, ?3, ?4, ?5)
                    ON CONFLICT(target_revision_id) DO UPDATE SET
                        policy_version = excluded.policy_version,
                        outcome = excluded.outcome,
                        assessed_at = excluded.assessed_at,
                        tool_or_model = excluded.tool_or_model
                    ",
                    params![
                        target_revision_id,
                        SUMMARY_POLICY_VERSION,
                        outcome,
                        now(),
                        tool_or_model
                    ],
                )
                .context("record PCP Summary assessment")?;
            transaction
                .commit()
                .context("commit PCP Summary assessment")
        })
        .await
    }
}

pub(crate) fn current_summary(
    connection: &Connection,
    target_revision_id: &str,
) -> Result<Option<PageSummary>> {
    connection
        .query_row(
            "
            SELECT s.summary_page_id, s.summary_revision_id, s.target_revision_id, s.content,
                   s.actor_type, s.actor_id, s.created_at, s.tool_or_model,
                   s.provenance_json
            FROM pcp_summary_heads h
            JOIN pcp_summaries s
              ON s.summary_revision_id = h.current_summary_revision_id
            WHERE h.target_revision_id = ?1
            ",
            [target_revision_id],
            summary_from_row,
        )
        .optional()
        .context("read current PCP Summary")
}

fn validate_summary(request: &WriteSummaryRequest) -> Result<()> {
    let length = request.content.trim().chars().count();
    if length == 0 || length > MAX_SUMMARY_CHARS {
        anyhow::bail!("PCP Summary must contain 1-{MAX_SUMMARY_CHARS} characters");
    }
    if request.target_revision_id.trim().is_empty() {
        anyhow::bail!("PCP Summary requires a target Revision");
    }
    Ok(())
}

fn current_summary_identity(
    transaction: &Transaction<'_>,
    target_revision_id: &str,
) -> Result<Option<(String, String)>> {
    transaction
        .query_row(
            "
            SELECT h.current_summary_revision_id, s.summary_page_id
            FROM pcp_summary_heads h
            JOIN pcp_summaries s
              ON s.summary_revision_id = h.current_summary_revision_id
            WHERE h.target_revision_id = ?1
            ",
            [target_revision_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .context("read current PCP Summary Revision")
}

fn lookup_idempotency(
    transaction: &Transaction<'_>,
    actor_id: &str,
    idempotency_key: Option<&str>,
) -> Result<Option<WriteSummaryResult>> {
    let Some(key) = idempotency_key else {
        return Ok(None);
    };
    transaction
        .query_row(
            "
            SELECT i.target_revision_id, s.summary_page_id, i.result_summary_revision_id
            FROM pcp_summary_idempotency i
            JOIN pcp_summaries s
              ON s.summary_revision_id = i.result_summary_revision_id
            WHERE i.actor_id = ?1 AND i.idempotency_key = ?2
            ",
            params![actor_id, key],
            |row| {
                Ok(WriteSummaryResult {
                    target_revision_id: row.get(0)?,
                    summary_page_id: row.get(1)?,
                    summary_revision_id: row.get(2)?,
                    created: false,
                })
            },
        )
        .optional()
        .context("look up PCP Summary idempotency")
}

fn complete_provenance(
    mut provenance: Vec<ProvenanceEvent>,
    target_revision_id: &str,
    actor: &pcp_core::Actor,
    tool_or_model: Option<&str>,
    timestamp: &str,
) -> Vec<ProvenanceEvent> {
    let includes_target = provenance.iter().any(|event| {
        event
            .input_revision_ids
            .iter()
            .any(|id| id == target_revision_id)
    });
    if !includes_target {
        provenance.push(ProvenanceEvent {
            operation: "summarize".to_owned(),
            actor: actor.clone(),
            timestamp: timestamp.to_owned(),
            input_revision_ids: vec![target_revision_id.to_owned()],
            tool_or_model: tool_or_model.map(str::to_owned),
        });
    }
    provenance
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

fn ensure_provenance_access(
    transaction: &Transaction<'_>,
    provenance: &[ProvenanceEvent],
    allowed_scopes: &HashSet<String>,
) -> Result<()> {
    let mut checked = HashSet::new();
    for revision_id in provenance
        .iter()
        .flat_map(|event| event.input_revision_ids.iter())
    {
        if checked.insert(revision_id) {
            ensure_revision_access(transaction, revision_id, allowed_scopes)?;
        }
    }
    Ok(())
}

fn summary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PageSummary> {
    let actor_type_text: String = row.get(4)?;
    let actor_type = ActorType::parse(&actor_type_text).ok_or_else(|| {
        rusqlite::Error::InvalidColumnType(4, "actor_type".to_owned(), rusqlite::types::Type::Text)
    })?;
    let provenance_json: String = row.get(8)?;
    let provenance = serde_json::from_str(&provenance_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(PageSummary {
        summary_page_id: row.get(0)?,
        summary_revision_id: row.get(1)?,
        target_revision_id: row.get(2)?,
        content: row.get(3)?,
        created_by: pcp_core::Actor {
            actor_type,
            actor_id: row.get(5)?,
        },
        created_at: row.get(6)?,
        tool_or_model: row.get(7)?,
        provenance,
    })
}

fn random_id(transaction: &Transaction<'_>, prefix: &str) -> Result<String> {
    let value: String = transaction
        .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))
        .context("generate PCP Summary identity")?;
    Ok(format!("{prefix}{value}"))
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
