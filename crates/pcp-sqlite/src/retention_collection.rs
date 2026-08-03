use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use pcp_core::{
    CollectRevisionRetentionRequest, PlanRevisionRetentionRequest, RevisionCollectionResult,
};
use rusqlite::{
    OptionalExtension, Transaction, TransactionBehavior, params, params_from_iter, types::Value,
};

use crate::{retention::plan_retention, store::SqlitePcpStore};

const MAX_REVISIONS_PER_COLLECTION: usize = 500;

impl SqlitePcpStore {
    pub async fn collect_revision_retention(
        &self,
        collector_principal_id: String,
        mut request: CollectRevisionRetentionRequest,
    ) -> Result<RevisionCollectionResult> {
        request.revision_ids.sort();
        request.revision_ids.dedup();
        anyhow::ensure!(
            !request.revision_ids.is_empty(),
            "PCP Revision collection requires at least one confirmed candidate"
        );
        anyhow::ensure!(
            request.revision_ids.len() <= MAX_REVISIONS_PER_COLLECTION,
            "PCP Revision collection accepts at most {MAX_REVISIONS_PER_COLLECTION} candidates"
        );
        request.policy.sample_limit = MAX_REVISIONS_PER_COLLECTION as u32;

        self.run("Revision retention collection", move |mut connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .context("start PCP Revision collection")?;
            let plan = plan_retention(
                &transaction,
                PlanRevisionRetentionRequest {
                    scopes: request.scopes.clone(),
                    policy: request.policy.clone(),
                },
            )?;
            let candidates = plan
                .candidates
                .iter()
                .map(|candidate| (candidate.revision_id.as_str(), candidate))
                .collect::<HashMap<_, _>>();
            for revision_id in &request.revision_ids {
                anyhow::ensure!(
                    candidates.contains_key(revision_id.as_str()),
                    "Revision {revision_id} is no longer an eligible retention candidate; generate a new plan"
                );
            }

            let collected_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
            let collected_pages = request
                .revision_ids
                .iter()
                .filter_map(|revision_id| candidates.get(revision_id.as_str()))
                .map(|candidate| candidate.page_id.as_str())
                .collect::<HashSet<_>>()
                .len() as u64;
            let reclaimed_estimated_bytes = request
                .revision_ids
                .iter()
                .filter_map(|revision_id| candidates.get(revision_id.as_str()))
                .map(|candidate| candidate.estimated_bytes)
                .sum();

            for revision_id in &request.revision_ids {
                let candidate = candidates
                    .get(revision_id.as_str())
                    .context("validated collection candidate disappeared")?;
                transaction
                    .execute(
                        "
                        INSERT INTO pcp_revision_collections (
                            revision_id, page_id, namespace, kind, original_created_at,
                            previous_revision_id, collected_at, estimated_bytes,
                            collector_principal_id
                        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                        ",
                        params![
                            candidate.revision_id,
                            candidate.page_id,
                            candidate.namespace,
                            candidate.kind,
                            candidate.created_at,
                            candidate.previous_revision_id,
                            collected_at,
                            i64::try_from(candidate.estimated_bytes).unwrap_or(i64::MAX),
                            collector_principal_id,
                        ],
                    )
                    .with_context(|| {
                        format!("record collected PCP Revision {}", candidate.revision_id)
                    })?;
            }

            let past_window_idempotency_records_removed = remove_past_window_idempotency(
                &transaction,
                &request.revision_ids,
                &plan.cutoff_at,
            )?;
            let expired_retention_leases_removed = execute_for_ids(
                &transaction,
                "DELETE FROM pcp_revision_retention_leases
                 WHERE expires_at <= ?1 AND revision_id IN (",
                &collected_at,
                &request.revision_ids,
            )?;
            execute_ids_only(
                &transaction,
                "DELETE FROM pcp_summary_assessments WHERE target_revision_id IN (",
                &request.revision_ids,
            )?;
            execute_ids_only(
                &transaction,
                "DELETE FROM pcp_provenance_inputs
                 WHERE derived_revision_id IN (",
                &request.revision_ids,
            )?;
            execute_ids_only(
                &transaction,
                "DELETE FROM pcp_provenance_inputs
                 WHERE input_revision_id IN (",
                &request.revision_ids,
            )?;
            execute_ids_only(
                &transaction,
                "DELETE FROM pcp_revision_fts WHERE revision_id IN (",
                &request.revision_ids,
            )?;
            let removed = execute_ids_only(
                &transaction,
                "DELETE FROM pcp_revisions WHERE revision_id IN (",
                &request.revision_ids,
            )?;
            anyhow::ensure!(
                removed == request.revision_ids.len(),
                "PCP Revision collection removed {removed} of {} confirmed candidates",
                request.revision_ids.len()
            );
            ensure_foreign_keys(&transaction)?;
            transaction
                .commit()
                .context("commit PCP Revision collection")?;

            Ok(RevisionCollectionResult {
                collected_at,
                collected_revisions: removed as u64,
                collected_pages,
                reclaimed_estimated_bytes,
                past_window_idempotency_records_removed: past_window_idempotency_records_removed
                    as u64,
                expired_retention_leases_removed: expired_retention_leases_removed as u64,
                revision_ids: request.revision_ids,
            })
        })
        .await
    }
}

fn remove_past_window_idempotency(
    transaction: &Transaction<'_>,
    revision_ids: &[String],
    cutoff_at: &str,
) -> Result<usize> {
    let mut removed = 0;
    for (table, first_column, second_column) in [
        ("pcp_idempotency", "result_revision_id", None),
        (
            "pcp_summary_idempotency",
            "target_revision_id",
            Some("result_summary_revision_id"),
        ),
        (
            "pcp_validity_idempotency",
            "target_revision_id",
            Some("result_assessment_id"),
        ),
    ] {
        removed += execute_for_ids_with_second_column(
            transaction,
            table,
            first_column,
            second_column,
            cutoff_at,
            revision_ids,
        )?;
    }
    Ok(removed)
}

fn execute_for_ids_with_second_column(
    transaction: &Transaction<'_>,
    table: &str,
    first_column: &str,
    second_column: Option<&str>,
    leading: &str,
    revision_ids: &[String],
) -> Result<usize> {
    let first_placeholders = placeholders(revision_ids.len(), 2);
    let second_clause = second_column
        .map(|column| {
            format!(
                " OR {column} IN ({})",
                placeholders(revision_ids.len(), 2 + revision_ids.len())
            )
        })
        .unwrap_or_default();
    let sql = format!(
        "DELETE FROM {table} WHERE created_at <= ?1 AND ({first_column} IN ({first_placeholders}){second_clause})"
    );
    let mut values =
        Vec::with_capacity(1 + revision_ids.len() * if second_column.is_some() { 2 } else { 1 });
    values.push(Value::Text(leading.to_owned()));
    values.extend(revision_ids.iter().cloned().map(Value::Text));
    if second_column.is_some() {
        values.extend(revision_ids.iter().cloned().map(Value::Text));
    }
    transaction
        .execute(&sql, params_from_iter(values.iter()))
        .context("remove expired PCP idempotency records")
}

fn execute_for_ids(
    transaction: &Transaction<'_>,
    prefix: &str,
    leading: &str,
    revision_ids: &[String],
) -> Result<usize> {
    let sql = format!("{prefix}{})", placeholders(revision_ids.len(), 2));
    let mut values = Vec::with_capacity(1 + revision_ids.len());
    values.push(Value::Text(leading.to_owned()));
    values.extend(revision_ids.iter().cloned().map(Value::Text));
    transaction
        .execute(&sql, params_from_iter(values.iter()))
        .context("execute PCP Revision collection cleanup")
}

fn execute_ids_only(
    transaction: &Transaction<'_>,
    prefix: &str,
    revision_ids: &[String],
) -> Result<usize> {
    let sql = format!("{prefix}{})", placeholders(revision_ids.len(), 1));
    let values = revision_ids
        .iter()
        .cloned()
        .map(Value::Text)
        .collect::<Vec<_>>();
    transaction
        .execute(&sql, params_from_iter(values.iter()))
        .context("execute PCP Revision collection mutation")
}

fn placeholders(count: usize, first_index: usize) -> String {
    (0..count)
        .map(|offset| format!("?{}", first_index + offset))
        .collect::<Vec<_>>()
        .join(",")
}

fn ensure_foreign_keys(transaction: &Transaction<'_>) -> Result<()> {
    let violation = transaction
        .query_row("PRAGMA foreign_key_check", [], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .optional()
        .context("check PCP foreign keys after Revision collection")?;
    anyhow::ensure!(
        violation.is_none(),
        "PCP Revision collection would violate referential integrity: {violation:?}"
    );
    Ok(())
}
