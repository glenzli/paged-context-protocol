//! Atomic publication of reviewed canonical Pages and source recall roles.
//! A source is hidden from default recall only while every Page carrying its
//! reviewed coverage remains an active current head.
use std::collections::{BTreeSet, HashSet};

use anyhow::{Context, Result, ensure};
use pcp_core::{ConsolidatePagesRequest, ConsolidationResult, PagePayload, PageRevisionRef};
use rusqlite::{OptionalExtension, params};
use serde_json::json;

use crate::{
    store::SqlitePcpStore,
    write::{complete_provenance, insert_revision, now, random_id},
};

impl SqlitePcpStore {
    pub async fn consolidate_pages(
        &self,
        request: ConsolidatePagesRequest,
    ) -> Result<ConsolidationResult> {
        validate_request(&request)?;
        self.run("page consolidation", move |mut connection| {
            let transaction = connection
                .transaction()
                .context("start PCP consolidation")?;
            let request_json = serde_json::to_string(&request)?;
            let replay: Option<(String, String)> = transaction
                .query_row(
                    "SELECT request_json, result_json FROM pcp_consolidations
                     WHERE actor_id = ?1 AND idempotency_key = ?2",
                    params![request.created_by.actor_id, request.idempotency_key],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if let Some((original, result)) = replay {
                ensure!(
                    original == request_json,
                    "consolidation key was reused for a different plan"
                );
                let mut result: ConsolidationResult = serde_json::from_str(&result)?;
                result.created = false;
                return Ok(result);
            }

            for source in &request.source_pages {
                let current: Option<(String, String, String)> = transaction
                    .query_row(
                        "SELECT p.current_revision_id, p.namespace, p.lifecycle_status
                         FROM pcp_pages p WHERE p.page_id = ?1",
                        [source.page_id.as_str()],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()?;
                ensure!(
                    current
                        .as_ref()
                        .is_some_and(|(revision, scope, lifecycle)| revision
                            == &source.revision_id
                            && scope == &request.namespace
                            && lifecycle == "active"),
                    "consolidation source changed or is outside the reviewed Scope"
                );
                let active: String = transaction.query_row(
                    "SELECT lifecycle_status FROM pcp_revisions WHERE revision_id = ?1",
                    [source.revision_id.as_str()],
                    |row| row.get(0),
                )?;
                ensure!(
                    active == "active",
                    "consolidation source Revision is not active"
                );
            }

            let created_at = now();
            let consolidation_id = random_id(&transaction, "con_")?;
            let mut outputs = Vec::with_capacity(request.outputs.len());
            for output in &request.outputs {
                let page_id = random_id(&transaction, "pg_")?;
                let revision_id = random_id(&transaction, "rev_")?;
                transaction.execute(
                    "INSERT INTO pcp_pages (
                        page_id, current_revision_id, created_at, namespace,
                        kind, mutability, lifecycle_status, updated_at
                     ) VALUES (?1, NULL, ?2, ?3, 'consolidated', 'revisioned', 'active', ?2)",
                    params![page_id, created_at, request.namespace],
                )?;
                let source_ids = output
                    .source_indexes
                    .iter()
                    .map(|index| request.source_pages[*index].revision_id.clone())
                    .collect::<Vec<_>>();
                let provenance = complete_provenance(
                    vec![],
                    "consolidate_pages",
                    &request.created_by,
                    &created_at,
                    source_ids,
                )?;
                insert_revision(
                    &transaction,
                    &page_id,
                    &revision_id,
                    &request.namespace,
                    "active",
                    &created_at,
                    None,
                    None,
                    None,
                    None,
                    &request.created_by,
                    Some(&PagePayload {
                        media_type: "text/markdown".into(),
                        content: output.content.trim().to_owned(),
                    }),
                    &[],
                    Some(&json!({
                        "title": output.title.trim(),
                        "routingTier": "canonical",
                        "consolidationId": consolidation_id,
                    })),
                    &provenance,
                )?;
                let updated = transaction.execute(
                    "UPDATE pcp_pages SET current_revision_id = ?2
                     WHERE page_id = ?1 AND current_revision_id IS NULL",
                    params![page_id, revision_id],
                )?;
                ensure!(
                    updated == 1,
                    "consolidation output changed during publication"
                );
                outputs.push(PageRevisionRef {
                    page_id,
                    revision_id,
                });
            }

            let source_only = request
                .coverage
                .iter()
                .filter(|decision| decision.complete)
                .map(|decision| request.source_pages[decision.source_index].clone())
                .collect::<Vec<_>>();
            let result = ConsolidationResult {
                outputs,
                source_only,
                created: true,
            };
            transaction.execute(
                "INSERT INTO pcp_consolidations (
                    consolidation_id, namespace, actor_id, idempotency_key,
                    request_json, result_json, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    consolidation_id,
                    request.namespace,
                    request.created_by.actor_id,
                    request.idempotency_key,
                    request_json,
                    serde_json::to_string(&result)?,
                    created_at,
                ],
            )?;
            for decision in &request.coverage {
                let source = &request.source_pages[decision.source_index];
                transaction.execute(
                    "INSERT INTO pcp_consolidation_sources (
                        consolidation_id, source_page_id, source_revision_id,
                        source_only, explanation
                     ) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        consolidation_id,
                        source.page_id,
                        source.revision_id,
                        i64::from(decision.complete),
                        decision.explanation.trim(),
                    ],
                )?;
                for output_index in &decision.output_indexes {
                    let output = &result.outputs[*output_index];
                    transaction.execute(
                        "INSERT INTO pcp_consolidation_coverage (
                            consolidation_id, source_revision_id,
                            output_page_id, output_revision_id
                         ) VALUES (?1, ?2, ?3, ?4)",
                        params![
                            consolidation_id,
                            source.revision_id,
                            output.page_id,
                            output.revision_id,
                        ],
                    )?;
                }
            }
            transaction.commit().context("commit PCP consolidation")?;
            Ok(result)
        })
        .await
    }
}

fn validate_request(request: &ConsolidatePagesRequest) -> Result<()> {
    ensure!(
        !request.namespace.trim().is_empty(),
        "consolidation Scope is required"
    );
    ensure!(
        !request.created_by.actor_id.trim().is_empty(),
        "consolidation actor is required"
    );
    ensure!(
        !request.idempotency_key.trim().is_empty() && request.idempotency_key.len() <= 160,
        "consolidation idempotency key is required and must be at most 160 bytes"
    );
    ensure!(
        (1..=20).contains(&request.source_pages.len()),
        "consolidation needs 1..20 sources"
    );
    ensure!(
        (1..=8).contains(&request.outputs.len()),
        "consolidation needs 1..8 outputs"
    );
    ensure!(
        request.coverage.len() == request.source_pages.len(),
        "every consolidation source needs a coverage decision"
    );
    ensure!(
        request.coverage.iter().any(|decision| decision.complete),
        "consolidation must fully cover at least one source"
    );
    ensure!(
        request.source_pages.iter().all(
            |source| !source.page_id.trim().is_empty() && !source.revision_id.trim().is_empty()
        ) && request
            .source_pages
            .iter()
            .map(|source| &source.page_id)
            .collect::<HashSet<_>>()
            .len()
            == request.source_pages.len(),
        "consolidation sources must be distinct current Pages"
    );
    let mut expected = vec![BTreeSet::new(); request.source_pages.len()];
    for (output_index, output) in request.outputs.iter().enumerate() {
        ensure!(
            !output.title.trim().is_empty() && output.title.chars().count() <= 160,
            "consolidated Page title must be 1..160 characters"
        );
        ensure!(
            !output.content.trim().is_empty() && output.content.chars().count() <= 64_000,
            "consolidated Page content must be 1..64000 characters"
        );
        ensure!(
            !output.source_indexes.is_empty(),
            "each output needs source evidence"
        );
        for &source_index in &output.source_indexes {
            ensure!(
                source_index < expected.len(),
                "output refers to an unknown source"
            );
            ensure!(
                expected[source_index].insert(output_index),
                "repeated source in output"
            );
        }
    }
    let mut seen = BTreeSet::new();
    for decision in &request.coverage {
        ensure!(
            decision.source_index < expected.len() && seen.insert(decision.source_index),
            "coverage decision refers to an unknown or repeated source"
        );
        ensure!(
            !decision.explanation.trim().is_empty()
                && decision.explanation.chars().count() <= 2_000,
            "coverage explanation must be 1..2000 characters"
        );
        ensure!(
            !decision.complete || decision.explanation.trim().chars().count() >= 12,
            "complete source coverage needs a specific explanation"
        );
        let actual = decision
            .output_indexes
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        ensure!(
            actual.len() == decision.output_indexes.len()
                && actual == expected[decision.source_index],
            "coverage must list exactly the outputs derived from this source"
        );
        ensure!(
            !actual.is_empty(),
            "every source must contribute to an output"
        );
    }
    Ok(())
}
