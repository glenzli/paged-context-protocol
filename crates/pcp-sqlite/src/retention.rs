use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use pcp_core::{
    PlanRevisionRetentionRequest, ProtectedRevisionSample, RetentionProtectionReason,
    RetentionReasonCount, RevisionRetentionCandidate, RevisionRetentionPlan,
};
use rusqlite::{Connection, params_from_iter, types::Value as SqlValue};

use crate::store::SqlitePcpStore;

const MAX_KEEP_RECENT_REVISIONS: u32 = 1_000;
const MAX_SAMPLE_LIMIT: u32 = 500;
const MAX_MINIMUM_AGE_DAYS: u32 = 36_500;

#[derive(Clone)]
struct RevisionRecord {
    page_id: String,
    revision_id: String,
    previous_revision_id: Option<String>,
    namespace: String,
    kind: String,
    created_at: String,
    current_revision_id: String,
    sealed: bool,
    estimated_bytes: u64,
}

impl SqlitePcpStore {
    pub async fn plan_revision_retention(
        &self,
        mut request: PlanRevisionRetentionRequest,
    ) -> Result<RevisionRetentionPlan> {
        if request.scopes.is_empty() {
            anyhow::bail!("PCP retention planning requires at least one authorized Scope");
        }
        request.policy.minimum_age_days = request.policy.minimum_age_days.min(MAX_MINIMUM_AGE_DAYS);
        request.policy.keep_recent_revisions_per_page = request
            .policy
            .keep_recent_revisions_per_page
            .min(MAX_KEEP_RECENT_REVISIONS);
        request.policy.sample_limit = request.policy.sample_limit.min(MAX_SAMPLE_LIMIT);
        request.scopes.sort();
        request.scopes.dedup();

        self.run("Revision retention plan", move |connection| {
            plan_retention(&connection, request)
        })
        .await
    }
}

fn plan_retention(
    connection: &Connection,
    request: PlanRevisionRetentionRequest,
) -> Result<RevisionRetentionPlan> {
    let generated = Utc::now();
    let cutoff = generated - Duration::days(i64::from(request.policy.minimum_age_days));
    let generated_at = generated.to_rfc3339_opts(SecondsFormat::Millis, true);
    let cutoff_at = cutoff.to_rfc3339_opts(SecondsFormat::Millis, true);
    let records = load_revisions(connection, &request.scopes)?;
    let record_indexes = records
        .iter()
        .enumerate()
        .map(|(index, record)| (record.revision_id.clone(), index))
        .collect::<HashMap<_, _>>();
    let mut protections = BTreeMap::<String, BTreeSet<RetentionProtectionReason>>::new();

    let mut page_offsets = HashMap::<String, u32>::new();
    for record in &records {
        let offset = page_offsets.entry(record.page_id.clone()).or_default();
        if record.revision_id == record.current_revision_id {
            protect(
                &mut protections,
                &record.revision_id,
                RetentionProtectionReason::CurrentHead,
            );
        }
        if record.sealed {
            protect(
                &mut protections,
                &record.revision_id,
                RetentionProtectionReason::SealedEvidence,
            );
        }
        if *offset < request.policy.keep_recent_revisions_per_page {
            protect(
                &mut protections,
                &record.revision_id,
                RetentionProtectionReason::RecentRevisionWindow,
            );
        }
        match DateTime::parse_from_rfc3339(&record.created_at) {
            Ok(created_at) if created_at.with_timezone(&Utc) > cutoff => protect(
                &mut protections,
                &record.revision_id,
                RetentionProtectionReason::MinimumAgeWindow,
            ),
            Ok(_) => {}
            Err(_) => protect(
                &mut protections,
                &record.revision_id,
                RetentionProtectionReason::InvalidTimestamp,
            ),
        }
        *offset = offset.saturating_add(1);
    }

    protect_relation_basis(connection, &record_indexes, &mut protections)?;
    protect_projection_heads(connection, &record_indexes, &mut protections)?;
    let active_retention_leases =
        protect_retention_leases(connection, &record_indexes, &mut protections, &generated_at)?;
    let expired_idempotency_records =
        protect_live_idempotency(connection, &record_indexes, &mut protections, cutoff)?;
    close_over_provenance(connection, &record_indexes, &mut protections)?;

    let scanned_pages = records
        .iter()
        .map(|record| record.page_id.as_str())
        .collect::<HashSet<_>>()
        .len() as u64;
    let mut candidates = records
        .iter()
        .filter(|record| !protections.contains_key(&record.revision_id))
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.page_id.cmp(&right.page_id))
            .then_with(|| left.revision_id.cmp(&right.revision_id))
    });
    let candidate_revisions = candidates.len() as u64;
    let candidate_pages = candidates
        .iter()
        .map(|record| record.page_id.as_str())
        .collect::<HashSet<_>>()
        .len() as u64;
    let candidate_estimated_bytes = candidates.iter().map(|record| record.estimated_bytes).sum();
    let sample_limit = request.policy.sample_limit as usize;
    let candidate_samples = candidates
        .iter()
        .take(sample_limit)
        .map(candidate_sample)
        .collect::<Vec<_>>();

    let mut protected = records
        .iter()
        .filter_map(|record| {
            let reasons = protections.get(&record.revision_id)?;
            (record.revision_id != record.current_revision_id)
                .then(|| (record, reasons.iter().copied().collect::<Vec<_>>()))
        })
        .collect::<Vec<_>>();
    protected.sort_by(|(left, _), (right, _)| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.page_id.cmp(&right.page_id))
            .then_with(|| left.revision_id.cmp(&right.revision_id))
    });
    let protected_samples = protected
        .iter()
        .take(sample_limit)
        .map(|(record, reasons)| ProtectedRevisionSample {
            page_id: record.page_id.clone(),
            revision_id: record.revision_id.clone(),
            namespace: record.namespace.clone(),
            kind: record.kind.clone(),
            created_at: record.created_at.clone(),
            estimated_bytes: record.estimated_bytes,
            reasons: reasons.clone(),
        })
        .collect::<Vec<_>>();

    let mut reason_counts = BTreeMap::<RetentionProtectionReason, u64>::new();
    for reasons in protections.values() {
        for reason in reasons {
            *reason_counts.entry(*reason).or_default() += 1;
        }
    }

    Ok(RevisionRetentionPlan {
        generated_at,
        cutoff_at,
        scopes: request.scopes,
        policy: request.policy,
        scanned_pages,
        scanned_revisions: records.len() as u64,
        protected_revisions: records.len().saturating_sub(candidates.len()) as u64,
        candidate_revisions,
        candidate_pages,
        candidate_estimated_bytes,
        expired_idempotency_records,
        active_retention_leases,
        protection_reasons: reason_counts
            .into_iter()
            .map(|(reason, revisions)| RetentionReasonCount { reason, revisions })
            .collect(),
        candidates: candidate_samples,
        protected_samples,
        candidates_truncated: candidates.len() > sample_limit,
        protected_samples_truncated: protected.len() > sample_limit,
    })
}

fn protect_retention_leases(
    connection: &Connection,
    records: &HashMap<String, usize>,
    protections: &mut BTreeMap<String, BTreeSet<RetentionProtectionReason>>,
    generated_at: &str,
) -> Result<u64> {
    let mut statement = connection
        .prepare(
            "SELECT revision_id FROM pcp_revision_retention_leases
             WHERE expires_at > ?1",
        )
        .context("prepare PCP active retention lease scan")?;
    let revision_ids = statement
        .query_map([generated_at], |row| row.get::<_, String>(0))
        .context("query PCP active retention leases")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collect PCP active retention leases")?;
    let mut active = 0_u64;
    for revision_id in revision_ids {
        if records.contains_key(&revision_id) {
            active = active.saturating_add(1);
            protect(
                protections,
                &revision_id,
                RetentionProtectionReason::ExplicitLease,
            );
        }
    }
    Ok(active)
}

fn load_revisions(connection: &Connection, scopes: &[String]) -> Result<Vec<RevisionRecord>> {
    let mut sql = String::from(
        "SELECT revision.page_id, revision.revision_id,
                revision.previous_revision_id, revision.namespace, page.kind,
                revision.created_at, page.current_revision_id, page.mutability,
                COALESCE(length(CAST(revision.payload_content AS BLOB)), 0)
                  + COALESCE(length(CAST(revision.source_refs_json AS BLOB)), 0)
                  + COALESCE(length(CAST(revision.facets_json AS BLOB)), 0)
                  + COALESCE(length(CAST(revision.provenance_json AS BLOB)), 0)
         FROM pcp_revisions revision
         JOIN pcp_pages page ON page.page_id = revision.page_id
         WHERE revision.namespace IN (",
    );
    push_placeholders(&mut sql, scopes.len());
    sql.push_str(
        ") ORDER BY revision.page_id, revision.created_at DESC, revision.revision_id DESC",
    );
    let values = scopes
        .iter()
        .cloned()
        .map(SqlValue::Text)
        .collect::<Vec<_>>();
    let mut statement = connection
        .prepare(&sql)
        .context("prepare PCP Revision retention inventory")?;
    statement
        .query_map(params_from_iter(values.iter()), |row| {
            Ok(RevisionRecord {
                page_id: row.get(0)?,
                revision_id: row.get(1)?,
                previous_revision_id: row.get(2)?,
                namespace: row.get(3)?,
                kind: row.get(4)?,
                created_at: row.get(5)?,
                current_revision_id: row.get(6)?,
                sealed: row.get::<_, String>(7)? == "sealed",
                estimated_bytes: row.get::<_, i64>(8)?.max(0) as u64,
            })
        })
        .context("query PCP Revision retention inventory")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collect PCP Revision retention inventory")
}

fn protect_relation_basis(
    connection: &Connection,
    records: &HashMap<String, usize>,
    protections: &mut BTreeMap<String, BTreeSet<RetentionProtectionReason>>,
) -> Result<()> {
    let mut statement = connection
        .prepare("SELECT COALESCE(basis_revision_ids_json, '[]') FROM pcp_relations")
        .context("prepare PCP Relation basis retention scan")?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .context("query PCP Relation basis retention scan")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collect PCP Relation basis retention scan")?;
    for encoded in rows {
        for revision_id in serde_json::from_str::<Vec<String>>(&encoded)
            .context("decode PCP Relation basis during retention planning")?
        {
            if records.contains_key(&revision_id) {
                protect(
                    protections,
                    &revision_id,
                    RetentionProtectionReason::RelationBasis,
                );
            }
        }
    }
    Ok(())
}

fn protect_projection_heads(
    connection: &Connection,
    records: &HashMap<String, usize>,
    protections: &mut BTreeMap<String, BTreeSet<RetentionProtectionReason>>,
) -> Result<()> {
    for sql in [
        "SELECT target_revision_id, current_summary_revision_id FROM pcp_page_summary_heads",
        "SELECT target_revision_id, current_assessment_id FROM pcp_validity_heads",
    ] {
        let mut statement = connection
            .prepare(sql)
            .context("prepare PCP projection-head retention scan")?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .context("query PCP projection-head retention scan")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("collect PCP projection-head retention scan")?;
        for (target_revision_id, projection_revision_id) in rows {
            for revision_id in [target_revision_id, projection_revision_id] {
                if records.contains_key(&revision_id) {
                    protect(
                        protections,
                        &revision_id,
                        RetentionProtectionReason::ProjectionHead,
                    );
                }
            }
        }
    }
    Ok(())
}

fn protect_live_idempotency(
    connection: &Connection,
    records: &HashMap<String, usize>,
    protections: &mut BTreeMap<String, BTreeSet<RetentionProtectionReason>>,
    cutoff: DateTime<Utc>,
) -> Result<u64> {
    let queries = [
        "SELECT created_at, result_revision_id, NULL FROM pcp_idempotency
         WHERE result_revision_id IS NOT NULL",
        "SELECT created_at, target_revision_id, result_summary_revision_id
         FROM pcp_summary_idempotency",
        "SELECT created_at, target_revision_id, result_assessment_id
         FROM pcp_validity_idempotency",
    ];
    let mut expired = 0_u64;
    for sql in queries {
        let mut statement = connection
            .prepare(sql)
            .context("prepare PCP idempotency retention scan")?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .context("query PCP idempotency retention scan")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("collect PCP idempotency retention scan")?;
        for (created_at, first_revision_id, second_revision_id) in rows {
            let live = DateTime::parse_from_rfc3339(&created_at)
                .map(|value| value.with_timezone(&Utc) > cutoff)
                .unwrap_or(true);
            if !live {
                expired = expired.saturating_add(1);
                continue;
            }
            for revision_id in [first_revision_id, second_revision_id]
                .into_iter()
                .flatten()
            {
                if records.contains_key(&revision_id) {
                    protect(
                        protections,
                        &revision_id,
                        RetentionProtectionReason::IdempotencyWindow,
                    );
                }
            }
        }
    }
    Ok(expired)
}

fn close_over_provenance(
    connection: &Connection,
    records: &HashMap<String, usize>,
    protections: &mut BTreeMap<String, BTreeSet<RetentionProtectionReason>>,
) -> Result<()> {
    let mut statement = connection
        .prepare(
            "SELECT provenance.derived_revision_id, provenance.input_revision_id,
                    derived.page_id, input.page_id
             FROM pcp_provenance_inputs provenance
             JOIN pcp_revisions derived
               ON derived.revision_id = provenance.derived_revision_id
             JOIN pcp_revisions input
               ON input.revision_id = provenance.input_revision_id
             WHERE derived.page_id <> input.page_id",
        )
        .context("prepare PCP provenance retention graph")?;
    let edges = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .context("query PCP provenance retention graph")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collect PCP provenance retention graph")?;
    let mut inputs_by_derived = HashMap::<String, Vec<String>>::new();
    for (derived_revision_id, input_revision_id) in edges {
        if !records.contains_key(&input_revision_id) {
            continue;
        }
        if !records.contains_key(&derived_revision_id) {
            protect(
                protections,
                &input_revision_id,
                RetentionProtectionReason::ProvenanceDependency,
            );
            continue;
        }
        inputs_by_derived
            .entry(derived_revision_id)
            .or_default()
            .push(input_revision_id);
    }

    let mut queue = protections.keys().cloned().collect::<VecDeque<_>>();
    let mut visited = HashSet::new();
    while let Some(derived_revision_id) = queue.pop_front() {
        if !visited.insert(derived_revision_id.clone()) {
            continue;
        }
        let Some(inputs) = inputs_by_derived.get(&derived_revision_id) else {
            continue;
        };
        for input_revision_id in inputs {
            let inserted = protections
                .entry(input_revision_id.clone())
                .or_default()
                .insert(RetentionProtectionReason::ProvenanceDependency);
            if inserted {
                queue.push_back(input_revision_id.clone());
            }
        }
    }
    Ok(())
}

fn protect(
    protections: &mut BTreeMap<String, BTreeSet<RetentionProtectionReason>>,
    revision_id: &str,
    reason: RetentionProtectionReason,
) {
    protections
        .entry(revision_id.to_owned())
        .or_default()
        .insert(reason);
}

fn candidate_sample(record: &RevisionRecord) -> RevisionRetentionCandidate {
    RevisionRetentionCandidate {
        page_id: record.page_id.clone(),
        revision_id: record.revision_id.clone(),
        namespace: record.namespace.clone(),
        kind: record.kind.clone(),
        created_at: record.created_at.clone(),
        previous_revision_id: record.previous_revision_id.clone(),
        estimated_bytes: record.estimated_bytes,
    }
}

fn push_placeholders(sql: &mut String, count: usize) {
    for index in 0..count {
        if index > 0 {
            sql.push(',');
        }
        sql.push('?');
    }
}
