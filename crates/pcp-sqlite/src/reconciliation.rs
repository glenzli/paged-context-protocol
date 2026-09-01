use std::collections::{BTreeSet, HashSet};

use anyhow::{Context, Result};
use pcp_core::{
    Actor, ApplyReconciliationRequest, AssessPageValidityRequest, FeedbackAuthority, FeedbackKind,
    FeedbackSignal, FeedbackStatus, FeedbackSubmission, LifecycleStatus, PageMutability,
    ProvenanceEvent, ReconciliationDisposition, ReconciliationResult, SubmitFeedbackRequest,
    ValidityStanding,
};
use rusqlite::{OptionalExtension, Transaction, params, params_from_iter, types::Value};
use serde_json::json;

use crate::{
    store::SqlitePcpStore,
    validity::assess_page_validity_tx,
    write::{insert_revision, insert_revision_relation_with_basis, now, random_id},
};

const MAX_FEEDBACK_CHARS: usize = 32_000;
const MAX_FEEDBACK_TARGETS: usize = 64;
const MAX_RESPONSE_REF_CHARS: usize = 2_000;

impl SqlitePcpStore {
    pub async fn submit_feedback(
        &self,
        request: SubmitFeedbackRequest,
        actor: Actor,
        allowed_scopes: Vec<String>,
    ) -> Result<FeedbackSubmission> {
        validate_feedback(&request)?;
        let allowed_scopes = allowed_scopes.into_iter().collect::<HashSet<_>>();
        self.run("feedback submission", move |mut connection| {
            let transaction = connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .context("start PCP feedback submission")?;
            ensure_scope_access(&transaction, &request.namespace, &allowed_scopes)?;
            let challenged = normalized_revision_ids(request.challenged_revision_ids.clone());
            let used = normalized_revision_ids(request.used_revision_ids.clone());
            let evidence = normalized_revision_ids(request.evidence_revision_ids.clone());
            for revision_id in challenged.iter().chain(&used).chain(&evidence) {
                ensure_revision_access(&transaction, revision_id, &allowed_scopes)?;
            }
            if let Some(existing) = lookup_feedback_idempotency(
                &transaction,
                &actor.actor_id,
                request.external_event_id.as_deref(),
            )? {
                return Ok(existing);
            }

            let timestamp = now();
            let page_id = random_id(&transaction, "pg_")?;
            let revision_id = random_id(&transaction, "rev_")?;
            let mut provenance_inputs = challenged.clone();
            provenance_inputs.extend(used.iter().cloned());
            provenance_inputs.extend(evidence.iter().cloned());
            provenance_inputs.sort();
            provenance_inputs.dedup();
            let provenance = vec![ProvenanceEvent {
                operation: "submit_feedback".to_owned(),
                actor: actor.clone(),
                timestamp: timestamp.clone(),
                input_revision_ids: provenance_inputs,
                tool_or_model: None,
                reason: None,
            }];
            let facets = json!({
                "feedbackKind": request.kind.as_str(),
                "authority": request.authority.as_str(),
                "responseRef": request.response_ref,
            });
            transaction
                .execute(
                    "INSERT INTO pcp_pages (
                        page_id, current_revision_id, created_at, namespace,
                        kind, mutability, lifecycle_status, updated_at
                     ) VALUES (?1, NULL, ?2, ?3, 'feedback_signal', ?4, ?5, ?2)",
                    params![
                        page_id,
                        timestamp,
                        request.namespace,
                        PageMutability::Sealed.as_str(),
                        LifecycleStatus::Active.as_str(),
                    ],
                )
                .context("create PCP feedback Page")?;
            insert_revision(
                &transaction,
                &page_id,
                &revision_id,
                &request.namespace,
                LifecycleStatus::Active.as_str(),
                &timestamp,
                request.observed_at.as_deref(),
                None,
                None,
                None,
                &actor,
                Some(&request.payload),
                &request.source_refs,
                Some(&facets),
                &provenance,
            )?;
            transaction
                .execute(
                    "UPDATE pcp_pages SET current_revision_id = ?2 WHERE page_id = ?1",
                    params![page_id, revision_id],
                )
                .context("publish PCP feedback Page")?;
            transaction
                .execute(
                    "INSERT INTO pcp_feedback_signals (
                        feedback_revision_id, feedback_page_id, namespace,
                        feedback_kind, authority, response_ref, status, created_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7)",
                    params![
                        revision_id,
                        page_id,
                        request.namespace,
                        request.kind.as_str(),
                        request.authority.as_str(),
                        request.response_ref,
                        timestamp,
                    ],
                )
                .context("index PCP feedback signal")?;
            insert_targets(&transaction, &revision_id, "challenged", &challenged)?;
            insert_targets(&transaction, &revision_id, "used", &used)?;
            insert_targets(&transaction, &revision_id, "evidence", &evidence)?;
            if let Some(key) = request.external_event_id.as_deref() {
                transaction
                    .execute(
                        "INSERT INTO pcp_idempotency (
                            actor_id, operation, idempotency_key,
                            result_page_id, result_revision_id, result_relation_id, created_at
                         ) VALUES (?1, 'submit_feedback', ?2, ?3, ?4, NULL, ?5)",
                        params![actor.actor_id, key, page_id, revision_id, timestamp],
                    )
                    .context("record PCP feedback idempotency")?;
            }
            transaction
                .commit()
                .context("commit PCP feedback submission")?;
            Ok(FeedbackSubmission {
                feedback_page_id: page_id,
                feedback_revision_id: revision_id,
                created: true,
                challenged_revision_ids: challenged,
                used_revision_ids: used,
                evidence_revision_ids: evidence,
            })
        })
        .await
    }

    pub async fn pending_feedback(
        &self,
        allowed_scopes: Vec<String>,
        limit: u32,
    ) -> Result<Vec<FeedbackSignal>> {
        let mut allowed_scopes = allowed_scopes.into_iter().collect::<Vec<_>>();
        allowed_scopes.sort();
        allowed_scopes.dedup();
        self.run("pending feedback read", move |connection| {
            if allowed_scopes.is_empty() || limit == 0 {
                return Ok(Vec::new());
            }
            let placeholders = std::iter::repeat_n("?", allowed_scopes.len())
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT feedback_revision_id, feedback_page_id, namespace,
                        feedback_kind, authority, status, created_at, response_ref
                 FROM pcp_feedback_signals
                 WHERE status = 'pending' AND namespace IN ({placeholders})
                   AND EXISTS (SELECT 1 FROM pcp_pages p
                       WHERE p.page_id = feedback_page_id
                         AND p.current_revision_id = feedback_revision_id
                         AND p.lifecycle_status = 'active')
                 ORDER BY created_at ASC, feedback_revision_id ASC
                 LIMIT ?"
            );
            let mut bindings = allowed_scopes
                .iter()
                .cloned()
                .map(Value::from)
                .collect::<Vec<_>>();
            bindings.push(Value::Integer(i64::from(limit.min(1_000))));
            let mut statement = connection
                .prepare(&sql)
                .context("prepare pending PCP feedback")?;
            let rows = statement
                .query_map(params_from_iter(bindings), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, Option<String>>(7)?,
                    ))
                })
                .context("query pending PCP feedback")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("collect pending PCP feedback")?;
            let mut signals = Vec::new();
            for (
                revision_id,
                page_id,
                namespace,
                kind,
                authority,
                status,
                created_at,
                response_ref,
            ) in rows
            {
                signals.push(FeedbackSignal {
                    feedback_page_id: page_id,
                    feedback_revision_id: revision_id.clone(),
                    namespace,
                    kind: parse_feedback_kind(&kind)?,
                    authority: parse_feedback_authority(&authority)?,
                    status: parse_feedback_status(&status)?,
                    created_at,
                    response_ref,
                    challenged_revision_ids: load_pending_challenged_targets(
                        &connection,
                        &revision_id,
                    )?,
                    used_revision_ids: load_targets(&connection, &revision_id, "used")?,
                    evidence_revision_ids: load_targets(&connection, &revision_id, "evidence")?,
                });
            }
            Ok(signals)
        })
        .await
    }

    pub async fn apply_reconciliation(
        &self,
        request: ApplyReconciliationRequest,
        allowed_scopes: Vec<String>,
    ) -> Result<ReconciliationResult> {
        validate_reconciliation(&request)?;
        let allowed_scopes = allowed_scopes.into_iter().collect::<HashSet<_>>();
        self.run("feedback reconciliation", move |mut connection| {
            let transaction = connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .context("start PCP feedback reconciliation")?;
            if let Some(feedback_revision_id) = request.feedback_revision_id.as_deref() {
            transaction
                .query_row(
                    "SELECT 1 FROM pcp_feedback_signals s JOIN pcp_pages p
                     ON p.page_id = s.feedback_page_id
                     WHERE s.feedback_revision_id = ?1
                       AND p.current_revision_id = s.feedback_revision_id
                       AND p.lifecycle_status = 'active'",
                    [feedback_revision_id],
                    |_| Ok(()),
                )
                .optional()
                .context("read current PCP feedback signal")?
                .context("reconciliation review is stale: feedback changed or is no longer active")?;
            ensure_revision_access(&transaction, feedback_revision_id, &allowed_scopes)?;
            ensure_revision_access(&transaction, &request.target.revision_id, &allowed_scopes)?;
            ensure_page_revision_pair(
                &transaction,
                &request.target.page_id,
                &request.target.revision_id,
            )?;
            let (target_status, target_resolution): (String, Option<String>) = transaction
                .query_row(
                    "SELECT status, resolution_json FROM pcp_feedback_targets
                     WHERE feedback_revision_id = ?1
                       AND target_revision_id = ?2
                       AND target_role = 'challenged'",
                    params![request.feedback_revision_id, request.target.revision_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .context("reconciliation target was not challenged by this feedback")?;
            if target_status != FeedbackStatus::Pending.as_str() {
                let encoded = target_resolution
                    .context("resolved PCP feedback target is missing its reconciliation result")?;
                let mut result: ReconciliationResult =
                    serde_json::from_str(&encoded).context("decode PCP reconciliation result")?;
                result.created = false;
                return Ok(result);
            }
            for revision_id in &request.basis_revision_ids {
                ensure_revision_access(&transaction, revision_id, &allowed_scopes)?;
                if revision_id != feedback_revision_id {
                    ensure_feedback_evidence(
                        &transaction,
                        feedback_revision_id,
                        revision_id,
                    )?;
                }
            }
            anyhow::ensure!(
                request
                    .basis_revision_ids
                    .iter()
                    .any(|revision_id| revision_id == feedback_revision_id),
                "reconciliation basis must include the feedback Revision"
            );
            }
            ensure_revision_access(&transaction, &request.target.revision_id, &allowed_scopes)?;
            ensure_current_page_revision(&transaction, &request.target)?;
            for revision_id in &request.basis_revision_ids {
                ensure_revision_access(&transaction, revision_id, &allowed_scopes)?;
            }
            // Review is bound to the validity head as well as Page contents.
            let current_assessment: Option<String> = transaction.query_row(
                "SELECT p.current_revision_id FROM pcp_validity_heads h JOIN pcp_pages p
                 ON p.page_id = h.assessment_page_id JOIN pcp_validity_assessments a
                 ON a.assessment_revision_id = p.current_revision_id WHERE a.target_revision_id = ?1",
                [&request.target.revision_id], |row| row.get(0),
            ).optional()?.flatten();
            let replay = is_identical_reconciliation_replay(&transaction, &request)?;
            anyhow::ensure!(replay || current_assessment == request.expected_assessment_revision_id,
                "reconciliation review is stale: validity assessment changed; review again");

            let standing = match request.disposition {
                ReconciliationDisposition::NoSourceChange => None,
                ReconciliationDisposition::Qualified => Some(ValidityStanding::Qualified),
                ReconciliationDisposition::Disputed => Some(ValidityStanding::Disputed),
                ReconciliationDisposition::Superseded => Some(ValidityStanding::Superseded),
                ReconciliationDisposition::Retracted => Some(ValidityStanding::Retracted),
            };
            let validity = standing
                .map(|standing| {
                    assess_page_validity_tx(
                        &transaction,
                        &AssessPageValidityRequest {
                            target_page_id: request.target.page_id.clone(),
                            target_revision_id: request.target.revision_id.clone(),
                            expected_assessment_revision_id: request.expected_assessment_revision_id.clone(),
                            standing,
                            rationale: request.rationale.clone(),
                            scope: request.scope.clone(),
                            basis_revision_ids: request.basis_revision_ids.clone(),
                            created_by: request.created_by.clone(),
                            tool_or_model: request.tool_or_model.clone(),
                            idempotency_key: request
                                .idempotency_key
                                .as_ref()
                                .map(|key| format!("reconciliation:{key}")),
                        },
                        &allowed_scopes,
                    )
                })
                .transpose()?;
            let supersedes_relation = if let Some(replacement) = request.replacement.as_ref() {
                ensure_revision_access(&transaction, &replacement.revision_id, &allowed_scopes)?;
                if let Some(feedback_revision_id) = request.feedback_revision_id.as_deref() {
                    ensure_feedback_evidence(&transaction, feedback_revision_id, &replacement.revision_id)?;
                }
                ensure_current_page_revision(&transaction, replacement)?;
                anyhow::ensure!(
                    replacement.page_id != request.target.page_id,
                    "superseded reconciliation requires a different replacement Page"
                );
                let replaced: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM pcp_relations r
                     WHERE r.relation_type = 'supersedes' AND r.to_page_id = ?1
                     AND NOT EXISTS(SELECT 1 FROM pcp_relation_retractions x WHERE x.relation_id = r.relation_id))",
                    [&replacement.page_id], |row| row.get(0),
                )?;
                anyhow::ensure!(!replaced, "reconciliation review is stale: replacement Page is itself superseded");
                let validity = crate::validity::current_validity(&transaction, &replacement.revision_id)?;
                anyhow::ensure!(validity.is_none_or(|validity| !matches!(validity.standing, ValidityStanding::Retracted | ValidityStanding::Superseded)),
                    "reconciliation review is stale: replacement evidence is no longer valid");
                let mut relation_basis = request.basis_revision_ids.clone();
                relation_basis.extend([
                    request.target.revision_id.clone(),
                    replacement.revision_id.clone(),
                ]);
                relation_basis.sort();
                relation_basis.dedup();
                Some(insert_revision_relation_with_basis(
                    &transaction,
                    &replacement.revision_id,
                    "supersedes",
                    &request.target.revision_id,
                    &relation_basis,
                    &request.created_by,
                    &now(),
                )?)
            } else {
                None
            };
            let affected_revision_ids =
                dependent_revisions(&transaction, &request.target.revision_id)?;
            let result = ReconciliationResult {
                feedback_revision_id: request.feedback_revision_id.clone(),
                target: request.target.clone(),
                disposition: request.disposition.clone(),
                validity,
                supersedes_relation,
                affected_revision_ids,
                created: !replay,
            };
            if request.feedback_revision_id.is_some() {
            let resolved_at = now();
            let resolution_json = serde_json::to_string(&result)?;
            let updated = transaction
                .execute(
                    "UPDATE pcp_feedback_targets
                     SET status = 'applied', disposition = ?3,
                         resolution_json = ?4, resolved_at = ?5
                     WHERE feedback_revision_id = ?1
                       AND target_revision_id = ?2
                       AND target_role = 'challenged'
                       AND status = 'pending'",
                    params![
                        request.feedback_revision_id,
                        request.target.revision_id,
                        request.disposition.as_str(),
                        resolution_json,
                        resolved_at
                    ],
                )
                .context("resolve PCP feedback signal")?;
            anyhow::ensure!(
                updated == 1,
                "feedback target changed during reconciliation"
            );
            transaction
                .execute(
                    "UPDATE pcp_feedback_signals
                     SET status = 'applied', resolved_at = ?2
                     WHERE feedback_revision_id = ?1
                       AND status = 'pending'
                       AND NOT EXISTS (
                           SELECT 1 FROM pcp_feedback_targets
                           WHERE feedback_revision_id = ?1
                             AND target_role = 'challenged'
                             AND status = 'pending'
                       )",
                    params![request.feedback_revision_id, resolved_at],
                )
                .context("complete reconciled PCP feedback signal")?;
            }
            transaction
                .commit()
                .context("commit PCP feedback reconciliation")?;
            Ok(result)
        })
        .await
    }
}

/// Page repair and the feedback work index share one transaction. Keep original
/// signals/results for audit; only the Page's current Revision is actionable.
/// Copy resolved targets unchanged so an edit never undoes or repeats an approval.
pub(crate) fn repair_feedback_binding(
    transaction: &Transaction<'_>,
    previous_revision_id: &str,
    revision_id: &str,
    payload: Option<&pcp_core::PagePayload>,
    timestamp: &str,
) -> Result<()> {
    let exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM pcp_feedback_signals WHERE feedback_revision_id = ?1)",
        [previous_revision_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(());
    }
    let chars = payload.map_or(0, |payload| payload.content.trim().chars().count());
    anyhow::ensure!(
        (1..=MAX_FEEDBACK_CHARS).contains(&chars),
        "feedback content must contain 1-{MAX_FEEDBACK_CHARS} characters"
    );
    transaction.execute(
        "INSERT INTO pcp_feedback_signals (
            feedback_revision_id, feedback_page_id, namespace, feedback_kind,
            authority, response_ref, status, created_at, resolved_at
         ) SELECT ?2, feedback_page_id, namespace, feedback_kind,
                  authority, response_ref, status, ?3, resolved_at
           FROM pcp_feedback_signals WHERE feedback_revision_id = ?1",
        params![previous_revision_id, revision_id, timestamp],
    )?;
    transaction.execute(
        "INSERT INTO pcp_feedback_targets (
            feedback_revision_id, target_revision_id, target_page_id, target_role,
            position, status, disposition, resolution_json, resolved_at
         ) SELECT ?2, target_revision_id, target_page_id, target_role,
                  position, status, disposition, resolution_json, resolved_at
           FROM pcp_feedback_targets WHERE feedback_revision_id = ?1",
        params![previous_revision_id, revision_id],
    )?;
    Ok(())
}

// A retry may replay the same decision, never turn one approval into another
// replacement by reusing its idempotency key.
fn is_identical_reconciliation_replay(
    transaction: &Transaction<'_>,
    request: &ApplyReconciliationRequest,
) -> Result<bool> {
    let Some(key) = request.idempotency_key.as_ref() else {
        return Ok(false);
    };
    let existing = transaction.query_row(
        "SELECT i.target_revision_id, r.payload_content, r.facets_json, r.provenance_json
         FROM pcp_validity_idempotency i JOIN pcp_revisions r ON r.revision_id = i.result_assessment_id
         WHERE i.actor_id = ?1 AND i.idempotency_key = ?2",
        params![request.created_by.actor_id, format!("reconciliation:{key}")],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)),
    ).optional()?;
    let Some((target, rationale, facets, provenance)) = existing else {
        return Ok(false);
    };
    let facets: serde_json::Value = serde_json::from_str(&facets)?;
    let provenance: Vec<ProvenanceEvent> = serde_json::from_str(&provenance)?;
    let mut basis = request.basis_revision_ids.clone();
    basis.push(request.target.revision_id.clone());
    let basis = normalized_revision_ids(basis);
    let old_basis = normalized_revision_ids(
        provenance
            .iter()
            .flat_map(|event| event.input_revision_ids.iter().cloned())
            .collect(),
    );
    anyhow::ensure!(
        target == request.target.revision_id
            && rationale == request.rationale.trim()
            && facets["standing"] == request.disposition.as_str()
            && facets["scope"] == json!(request.scope)
            && old_basis == basis,
        "reconciliation idempotency key was already used for a different decision"
    );
    if let Some(replacement) = &request.replacement {
        let relation_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM pcp_relations r WHERE r.relation_type = 'supersedes'
             AND r.from_page_id = ?1 AND r.to_page_id = ?2
             AND NOT EXISTS(SELECT 1 FROM pcp_relation_retractions x WHERE x.relation_id = r.relation_id))",
            params![replacement.page_id, request.target.page_id], |row| row.get(0),
        )?;
        anyhow::ensure!(
            relation_exists,
            "reconciliation idempotency key has no matching replacement relation"
        );
    }
    Ok(true)
}

fn validate_feedback(request: &SubmitFeedbackRequest) -> Result<()> {
    anyhow::ensure!(
        !request.namespace.trim().is_empty(),
        "feedback namespace is required"
    );
    let chars = request.payload.content.trim().chars().count();
    anyhow::ensure!(
        (1..=MAX_FEEDBACK_CHARS).contains(&chars),
        "feedback content must contain 1-{MAX_FEEDBACK_CHARS} characters"
    );
    anyhow::ensure!(
        !request.challenged_revision_ids.is_empty(),
        "feedback requires at least one challenged Revision"
    );
    anyhow::ensure!(
        request.challenged_revision_ids.len()
            + request.used_revision_ids.len()
            + request.evidence_revision_ids.len()
            <= MAX_FEEDBACK_TARGETS,
        "feedback exceeds {MAX_FEEDBACK_TARGETS} Revision references"
    );
    anyhow::ensure!(
        request
            .response_ref
            .as_deref()
            .is_none_or(|value| value.chars().count() <= MAX_RESPONSE_REF_CHARS),
        "feedback responseRef exceeds {MAX_RESPONSE_REF_CHARS} characters"
    );
    Ok(())
}

fn validate_reconciliation(request: &ApplyReconciliationRequest) -> Result<()> {
    anyhow::ensure!(
        !request.basis_revision_ids.is_empty()
            && request.basis_revision_ids.len() <= MAX_FEEDBACK_TARGETS + 1,
        "reconciliation requires bounded evidence"
    );
    if let Some(replacement) = &request.replacement {
        anyhow::ensure!(
            request
                .basis_revision_ids
                .contains(&replacement.revision_id),
            "reconciliation basis must include the replacement Revision"
        );
    }
    match request.disposition {
        ReconciliationDisposition::Superseded => anyhow::ensure!(
            request.replacement.is_some(),
            "superseded reconciliation requires a replacement Revision"
        ),
        _ => anyhow::ensure!(
            request.replacement.is_none(),
            "replacement Revision is only valid for superseded reconciliation"
        ),
    }
    Ok(())
}

fn normalized_revision_ids(mut values: Vec<String>) -> Vec<String> {
    values.retain(|value| !value.trim().is_empty());
    values.sort();
    values.dedup();
    values
}

fn ensure_scope_access(
    transaction: &Transaction<'_>,
    namespace: &str,
    allowed_scopes: &HashSet<String>,
) -> Result<()> {
    anyhow::ensure!(
        allowed_scopes.contains(namespace),
        "Scope is not authorized"
    );
    transaction
        .query_row(
            "SELECT 1 FROM pcp_scopes WHERE namespace = ?1",
            [namespace],
            |_| Ok(()),
        )
        .with_context(|| format!("find PCP Scope {namespace}"))
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
    anyhow::ensure!(
        allowed_scopes.contains(&namespace),
        "Revision is outside authorized Scopes"
    );
    Ok(())
}

fn ensure_page_revision_pair(
    transaction: &Transaction<'_>,
    page_id: &str,
    revision_id: &str,
) -> Result<()> {
    let resolved: String = transaction.query_row(
        "SELECT page_id FROM pcp_revisions WHERE revision_id = ?1",
        [revision_id],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        resolved == page_id,
        "Revision does not belong to the declared Page"
    );
    Ok(())
}

fn ensure_current_page_revision(
    transaction: &Transaction<'_>,
    page: &pcp_core::PageRevisionRef,
) -> Result<()> {
    let current: Option<(String, String)> = transaction
        .query_row(
            "SELECT current_revision_id, lifecycle_status FROM pcp_pages WHERE page_id = ?1",
            [&page.page_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    anyhow::ensure!(
        current == Some((page.revision_id.clone(), "active".to_owned())),
        "reconciliation review is stale: Page changed; review again"
    );
    Ok(())
}

fn ensure_feedback_evidence(
    transaction: &Transaction<'_>,
    feedback_revision_id: &str,
    revision_id: &str,
) -> Result<()> {
    let offered = transaction
        .query_row(
            "SELECT 1 FROM pcp_feedback_targets
             WHERE feedback_revision_id = ?1 AND target_revision_id = ?2
             LIMIT 1",
            params![feedback_revision_id, revision_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    anyhow::ensure!(offered, "Revision is outside this feedback evidence set");
    Ok(())
}

fn insert_targets(
    transaction: &Transaction<'_>,
    feedback_revision_id: &str,
    role: &str,
    revision_ids: &[String],
) -> Result<()> {
    for (position, revision_id) in revision_ids.iter().enumerate() {
        let page_id: String = transaction.query_row(
            "SELECT page_id FROM pcp_revisions WHERE revision_id = ?1",
            [revision_id],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO pcp_feedback_targets (feedback_revision_id, target_revision_id, target_page_id, target_role, position) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![feedback_revision_id, revision_id, page_id, role, position as i64],
        ).context("index PCP feedback target")?;
    }
    Ok(())
}

fn load_targets(
    connection: &rusqlite::Connection,
    feedback_revision_id: &str,
    role: &str,
) -> Result<Vec<String>> {
    let mut statement = connection.prepare("SELECT target_revision_id FROM pcp_feedback_targets WHERE feedback_revision_id = ?1 AND target_role = ?2 ORDER BY position ASC")?;
    Ok(statement
        .query_map(params![feedback_revision_id, role], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_pending_challenged_targets(
    connection: &rusqlite::Connection,
    feedback_revision_id: &str,
) -> Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT target_revision_id FROM pcp_feedback_targets
         WHERE feedback_revision_id = ?1
           AND target_role = 'challenged'
           AND status = 'pending'
         ORDER BY position ASC",
    )?;
    Ok(statement
        .query_map([feedback_revision_id], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

fn lookup_feedback_idempotency(
    transaction: &Transaction<'_>,
    actor_id: &str,
    key: Option<&str>,
) -> Result<Option<FeedbackSubmission>> {
    let Some(key) = key else {
        return Ok(None);
    };
    let result = transaction.query_row(
        "SELECT result_page_id, result_revision_id FROM pcp_idempotency WHERE actor_id = ?1 AND operation = 'submit_feedback' AND idempotency_key = ?2",
        params![actor_id, key],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    ).optional()?;
    result
        .map(|(page_id, revision_id)| {
            Ok(FeedbackSubmission {
                feedback_page_id: page_id,
                feedback_revision_id: revision_id.clone(),
                created: false,
                challenged_revision_ids: load_targets(transaction, &revision_id, "challenged")?,
                used_revision_ids: load_targets(transaction, &revision_id, "used")?,
                evidence_revision_ids: load_targets(transaction, &revision_id, "evidence")?,
            })
        })
        .transpose()
}

fn dependent_revisions(
    transaction: &Transaction<'_>,
    target_revision_id: &str,
) -> Result<Vec<String>> {
    let mut affected = BTreeSet::new();
    affected.insert(target_revision_id.to_owned());
    for sql in [
        "SELECT summary_revision_id FROM pcp_summaries WHERE target_revision_id = ?1",
        "SELECT topic_revision_id FROM pcp_topic_extraction_members WHERE source_revision_id = ?1",
        "SELECT derived_revision_id FROM pcp_provenance_inputs WHERE input_revision_id = ?1",
    ] {
        let mut statement = transaction.prepare(sql)?;
        affected.extend(
            statement
                .query_map([target_revision_id], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        );
    }
    let mut statement = transaction.prepare(
        "SELECT DISTINCT page.current_revision_id FROM pcp_relations relation JOIN json_each(relation.basis_revision_ids_json) basis JOIN pcp_pages page ON page.page_id = relation.from_page_id WHERE basis.value = ?1 AND page.current_revision_id IS NOT NULL",
    )?;
    affected.extend(
        statement
            .query_map([target_revision_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?,
    );
    Ok(affected.into_iter().collect())
}

fn parse_feedback_kind(value: &str) -> Result<FeedbackKind> {
    FeedbackKind::parse(value).with_context(|| format!("invalid PCP feedback kind {value}"))
}
fn parse_feedback_authority(value: &str) -> Result<FeedbackAuthority> {
    FeedbackAuthority::parse(value)
        .with_context(|| format!("invalid PCP feedback authority {value}"))
}
fn parse_feedback_status(value: &str) -> Result<FeedbackStatus> {
    FeedbackStatus::parse(value).with_context(|| format!("invalid PCP feedback status {value}"))
}
