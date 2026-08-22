use std::collections::{BTreeMap, BTreeSet, HashSet};

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, SecondsFormat, Timelike, Utc};
use pcp_core::{AccessDecision, AccessPrincipal, OperationTelemetry};
use pcp_store::{
    ActivityHealth, GraphHealth, HealthSnapshot, HealthTimelineBucket, NamedCount, OperationHealth,
    PackingHealth, RecallHealth, ScopeHealth, StorageHealth,
};
use rusqlite::{params_from_iter, types::Value as SqlValue};

use crate::SqlitePcpStore;

const LONG_PAGE_CHARS: u64 = 4_000;
const MIN_WINDOW_HOURS: u32 = 1;
const MAX_WINDOW_HOURS: u32 = 24 * 90;
const OBSERVABILITY_OPERATIONS: &[&str] = &[
    "access_log",
    "content_char_count",
    "durable_inventory",
    "health_snapshot",
    "list_scopes",
    "page_count",
    "query_audit_summary",
];

#[derive(Debug)]
struct HealthEvent {
    occurred_at: String,
    principal: AccessPrincipal,
    operation: String,
    scopes: Vec<String>,
    decision: AccessDecision,
    telemetry: Option<OperationTelemetry>,
}

#[derive(Default)]
struct OperationAccumulator {
    calls: u64,
    measured_calls: u64,
    failures: u64,
    input_count: u64,
    output_count: u64,
    output_bytes: u64,
    durations: Vec<u64>,
}

impl SqlitePcpStore {
    pub async fn health_snapshot(
        &self,
        allowed_scopes: Vec<String>,
        window_hours: u32,
    ) -> Result<HealthSnapshot> {
        self.flush_access_audit().await?;
        let window_hours = window_hours.clamp(MIN_WINDOW_HOURS, MAX_WINDOW_HOURS);
        let generated = Utc::now();
        let window_started = generated - Duration::hours(i64::from(window_hours));
        let generated_at = generated.to_rfc3339_opts(SecondsFormat::Millis, true);
        let window_started_at = window_started.to_rfc3339_opts(SecondsFormat::Millis, true);
        let allowed_scopes = allowed_scopes.into_iter().collect::<BTreeSet<_>>();
        if allowed_scopes.is_empty() {
            return Ok(HealthSnapshot {
                generated_at,
                window_started_at,
                window_hours,
                storage: StorageHealth::default(),
                activity: ActivityHealth::default(),
                recall: RecallHealth::default(),
                packing: PackingHealth::default(),
                graph: GraphHealth::default(),
                model_usage: pcp_store::RuntimeModelUsageHealth::default(),
                operations: Vec::new(),
                scopes: Vec::new(),
                timeline: Vec::new(),
            });
        }

        self.run("health snapshot", move |connection| {
            let mut scopes = storage_by_scope(&connection, &allowed_scopes, &window_started_at)?;
            let model_usage = SqlitePcpStore::runtime_usage_health(
                &connection,
                &allowed_scopes,
                &window_started_at,
            )?;
            let (relations, isolated_current_pages, relation_types) =
                graph_health(&connection, &allowed_scopes)?;
            let events = operation_events(&connection, &allowed_scopes, &window_started_at)?;
            let mut storage = StorageHealth::default();
            for scope in scopes.values() {
                storage.current_pages += scope.current_pages;
                storage.pages += scope.pages;
                storage.revisions += scope.revisions;
                storage.content_chars += scope.content_chars;
            }
            storage.historical_revisions = storage.revisions.saturating_sub(storage.pages);
            let (sealed_pages, revisioned_pages) = mutability_health(&connection, &allowed_scopes)?;
            storage.sealed_pages = sealed_pages;
            storage.revisioned_pages = revisioned_pages;
            let (created, long_pages, summarized_long_pages) =
                current_page_details(&connection, &allowed_scopes, &window_started_at)?;
            storage.current_pages_created = created;
            storage.long_pages = long_pages;
            storage.summarized_long_pages = summarized_long_pages;

            let mut activity = ActivityHealth::default();
            let mut recall = RecallHealth::default();
            let mut packing = PackingHealth::default();
            let mut operations = BTreeMap::<String, OperationAccumulator>::new();
            let mut principals = HashSet::new();
            let mut activity_durations = Vec::new();
            let mut timeline = BTreeMap::<String, HealthTimelineBucket>::new();

            for event in events {
                if OBSERVABILITY_OPERATIONS.contains(&event.operation.as_str()) {
                    continue;
                }
                activity.calls += 1;
                principals.insert(event.principal.principal_id);
                match event.decision {
                    AccessDecision::Allowed => activity.allowed += 1,
                    AccessDecision::Denied => activity.denied += 1,
                    AccessDecision::Failed => activity.failed += 1,
                }
                let failed = event.decision != AccessDecision::Allowed;
                let telemetry = event.telemetry.as_ref();
                if let Some(telemetry) = telemetry {
                    activity.measured_calls += 1;
                    activity_durations.push(telemetry.duration_ms);
                }

                let operation = operations.entry(event.operation.clone()).or_default();
                operation.calls += 1;
                operation.failures += u64::from(failed);
                if let Some(telemetry) = telemetry {
                    operation.measured_calls += 1;
                    operation.input_count += telemetry.input_count.unwrap_or(0);
                    operation.output_count += telemetry.output_count.unwrap_or(0);
                    operation.output_bytes += telemetry.output_bytes.unwrap_or(0);
                    operation.durations.push(telemetry.duration_ms);
                }

                if telemetry.is_some()
                    && matches!(event.operation.as_str(), "search_pages" | "browse_index")
                {
                    recall.searches += 1;
                    let returned = telemetry.and_then(|value| value.output_count).unwrap_or(0);
                    recall.returned_pages += returned;
                    if event.decision == AccessDecision::Allowed && returned == 0 {
                        recall.zero_result_searches += 1;
                    }
                }
                if event.operation == "read_pages" && telemetry.is_some() {
                    recall.pages_read +=
                        telemetry.and_then(|value| value.output_count).unwrap_or(0);
                    if telemetry.is_some_and(is_detail_read) {
                        recall.detail_reads += 1;
                    } else {
                        recall.summary_reads += 1;
                    }
                }
                if event.operation == "pack_pages"
                    && event.decision == AccessDecision::Allowed
                    && telemetry.is_some()
                {
                    let inputs = telemetry.and_then(|value| value.input_count).unwrap_or(0);
                    packing.runs += 1;
                    packing.input_pages += inputs;
                    packing.net_page_reduction += inputs.saturating_sub(1);
                }

                for namespace in &event.scopes {
                    let Some(scope) = scopes.get_mut(namespace) else {
                        continue;
                    };
                    scope.calls += 1;
                    scope.failures += u64::from(failed);
                    scope.searches += u64::from(matches!(
                        event.operation.as_str(),
                        "search_pages" | "browse_index"
                    ));
                    scope.writes += u64::from(is_write_operation(&event.operation));
                    scope.packs += u64::from(
                        event.operation == "pack_pages"
                            && event.decision == AccessDecision::Allowed,
                    );
                }

                let bucket_name = timeline_bucket(&event.occurred_at, window_hours);
                let bucket =
                    timeline
                        .entry(bucket_name.clone())
                        .or_insert_with(|| HealthTimelineBucket {
                            bucket: bucket_name,
                            ..HealthTimelineBucket::default()
                        });
                bucket.calls += 1;
                bucket.searches += u64::from(matches!(
                    event.operation.as_str(),
                    "search_pages" | "browse_index"
                ));
                bucket.writes += u64::from(is_write_operation(&event.operation));
                bucket.failures += u64::from(failed);
            }

            activity.principals = principals.len().try_into().unwrap_or(u64::MAX);
            activity.p50_duration_ms = percentile(&mut activity_durations, 50);
            activity.p95_duration_ms = percentile(&mut activity_durations, 95);
            let operations = operations
                .into_iter()
                .map(|(name, mut value)| OperationHealth {
                    operation: name,
                    calls: value.calls,
                    measured_calls: value.measured_calls,
                    failures: value.failures,
                    input_count: value.input_count,
                    output_count: value.output_count,
                    output_bytes: value.output_bytes,
                    p50_duration_ms: percentile(&mut value.durations, 50),
                    p95_duration_ms: percentile(&mut value.durations, 95),
                })
                .collect();
            let average_relations_per_page = if storage.pages == 0 {
                0.0
            } else {
                relations as f64 / storage.pages as f64
            };

            Ok(HealthSnapshot {
                generated_at,
                window_started_at,
                window_hours,
                storage,
                activity,
                recall,
                packing,
                graph: GraphHealth {
                    relations,
                    isolated_current_pages,
                    average_relations_per_page,
                    relation_types,
                },
                model_usage,
                operations,
                scopes: scopes.into_values().collect(),
                timeline: timeline.into_values().collect(),
            })
        })
        .await
    }
}

fn storage_by_scope(
    connection: &rusqlite::Connection,
    allowed_scopes: &BTreeSet<String>,
    _window_started_at: &str,
) -> Result<BTreeMap<String, ScopeHealth>> {
    let mut scopes = allowed_scopes
        .iter()
        .map(|namespace| {
            (
                namespace.clone(),
                ScopeHealth {
                    namespace: namespace.clone(),
                    ..ScopeHealth::default()
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let placeholders = placeholders(allowed_scopes.len());
    let values = scope_values(allowed_scopes);
    let current_sql = format!(
        "
        SELECT page.namespace, count(*),
               sum(length(COALESCE(revision.payload_content, '')))
        FROM pcp_pages page
        JOIN pcp_revisions revision ON revision.revision_id = page.current_revision_id
        WHERE page.namespace IN ({placeholders})
          AND page.lifecycle_status = 'active'
        GROUP BY page.namespace
        "
    );
    let mut statement = connection
        .prepare(&current_sql)
        .context("prepare PCP current Page health")?;
    let rows = statement
        .query_map(params_from_iter(values.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? as u64,
                row.get::<_, i64>(2)? as u64,
            ))
        })
        .context("query PCP current Page health")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collect PCP current Page health")?;
    for (namespace, current_pages, content_chars) in rows {
        if let Some(scope) = scopes.get_mut(&namespace) {
            scope.current_pages = current_pages;
            scope.content_chars = content_chars;
        }
    }

    let page_sql = format!(
        "SELECT namespace, count(*) FROM pcp_pages WHERE namespace IN ({placeholders}) GROUP BY namespace"
    );
    let mut statement = connection
        .prepare(&page_sql)
        .context("prepare PCP Page health")?;
    let rows = statement
        .query_map(params_from_iter(values.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
        })
        .context("query PCP Page health")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collect PCP Page health")?;
    for (namespace, pages) in rows {
        if let Some(scope) = scopes.get_mut(&namespace) {
            scope.pages = pages;
        }
    }

    let revision_sql = format!(
        "SELECT namespace, count(*) FROM pcp_revisions WHERE namespace IN ({placeholders}) GROUP BY namespace"
    );
    let mut statement = connection
        .prepare(&revision_sql)
        .context("prepare PCP Revision health")?;
    let rows = statement
        .query_map(params_from_iter(values.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
        })
        .context("query PCP Revision health")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collect PCP Revision health")?;
    for (namespace, revisions) in rows {
        if let Some(scope) = scopes.get_mut(&namespace) {
            scope.revisions = revisions;
        }
    }
    Ok(scopes)
}

fn current_page_details(
    connection: &rusqlite::Connection,
    allowed_scopes: &BTreeSet<String>,
    window_started_at: &str,
) -> Result<(u64, u64, u64)> {
    let placeholders = placeholders(allowed_scopes.len());
    let mut values = scope_values(allowed_scopes);
    values.push(SqlValue::Text(window_started_at.to_owned()));
    values.push(SqlValue::Integer(LONG_PAGE_CHARS as i64));
    let sql = format!(
        "
        SELECT
            sum(CASE WHEN revision.created_at >= ?{} THEN 1 ELSE 0 END),
            sum(CASE WHEN revision.payload_media_type LIKE 'text/%'
                          AND length(COALESCE(revision.payload_content, '')) >= ?{}
                          AND page.kind <> 'summary_projection'
                     THEN 1 ELSE 0 END),
            sum(CASE WHEN revision.payload_media_type LIKE 'text/%'
                          AND length(COALESCE(revision.payload_content, '')) >= ?{}
                          AND page.kind <> 'summary_projection'
                          AND EXISTS (
                              SELECT 1 FROM pcp_page_summary_heads summary
                              WHERE summary.target_page_id = page.page_id
                          )
                     THEN 1 ELSE 0 END)
        FROM pcp_pages page
        JOIN pcp_revisions revision ON revision.revision_id = page.current_revision_id
        WHERE page.namespace IN ({placeholders})
          AND page.lifecycle_status = 'active'
        ",
        allowed_scopes.len() + 1,
        allowed_scopes.len() + 2,
        allowed_scopes.len() + 2,
    );
    connection
        .query_row(&sql, params_from_iter(values.iter()), |row| {
            Ok((
                row.get::<_, Option<i64>>(0)?.unwrap_or(0) as u64,
                row.get::<_, Option<i64>>(1)?.unwrap_or(0) as u64,
                row.get::<_, Option<i64>>(2)?.unwrap_or(0) as u64,
            ))
        })
        .context("query PCP current Page details")
}

fn mutability_health(
    connection: &rusqlite::Connection,
    allowed_scopes: &BTreeSet<String>,
) -> Result<(u64, u64)> {
    let sql = format!(
        "SELECT
             sum(CASE WHEN mutability = 'sealed' THEN 1 ELSE 0 END),
             sum(CASE WHEN mutability = 'revisioned' THEN 1 ELSE 0 END)
         FROM pcp_pages WHERE namespace IN ({})",
        placeholders(allowed_scopes.len())
    );
    let values = scope_values(allowed_scopes);
    connection
        .query_row(&sql, params_from_iter(values.iter()), |row| {
            Ok((
                row.get::<_, Option<i64>>(0)?.unwrap_or(0) as u64,
                row.get::<_, Option<i64>>(1)?.unwrap_or(0) as u64,
            ))
        })
        .context("query PCP Page mutability health")
}

fn graph_health(
    connection: &rusqlite::Connection,
    allowed_scopes: &BTreeSet<String>,
) -> Result<(u64, u64, Vec<NamedCount>)> {
    let placeholders = placeholders(allowed_scopes.len());
    let values = scope_values(allowed_scopes);
    let relation_sql = format!(
        "
        SELECT relation.relation_type, count(*)
        FROM pcp_relations relation
        JOIN pcp_pages source ON source.page_id = relation.from_page_id
        LEFT JOIN pcp_relation_retractions retraction
          ON retraction.relation_id = relation.relation_id
        WHERE source.namespace IN ({placeholders})
          AND retraction.relation_id IS NULL
        GROUP BY relation.relation_type
        ORDER BY count(*) DESC, relation.relation_type
        "
    );
    let mut statement = connection
        .prepare(&relation_sql)
        .context("prepare PCP relation health")?;
    let relation_types = statement
        .query_map(params_from_iter(values.iter()), |row| {
            Ok(NamedCount {
                name: row.get(0)?,
                count: row.get::<_, i64>(1)? as u64,
            })
        })
        .context("query PCP relation health")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collect PCP relation health")?;
    let relations = relation_types.iter().map(|entry| entry.count).sum();

    let isolated_sql = format!(
        "
        SELECT count(*) FROM (
            SELECT page.page_id
            FROM pcp_pages page
            WHERE page.namespace IN ({placeholders})
              AND page.lifecycle_status = 'active'
              AND NOT EXISTS (
                  SELECT 1 FROM pcp_relations relation
                  LEFT JOIN pcp_relation_retractions retraction
                    ON retraction.relation_id = relation.relation_id
                  WHERE (relation.from_page_id = page.page_id
                         OR relation.to_page_id = page.page_id)
                    AND retraction.relation_id IS NULL
              )
        )
        "
    );
    let isolated = connection
        .query_row(&isolated_sql, params_from_iter(values.iter()), |row| {
            Ok(row.get::<_, i64>(0)? as u64)
        })
        .context("query isolated PCP Pages")?;
    Ok((relations, isolated, relation_types))
}

fn operation_events(
    connection: &rusqlite::Connection,
    allowed_scopes: &BTreeSet<String>,
    window_started_at: &str,
) -> Result<Vec<HealthEvent>> {
    let mut statement = connection
        .prepare(
            "
            SELECT occurred_at, principal_json, operation, scopes_json,
                   decision, telemetry_json
            FROM pcp_access_log
            WHERE occurred_at >= ?1
            ORDER BY occurred_at, event_id
            ",
        )
        .context("prepare PCP health event query")?;
    let rows = statement
        .query_map([window_started_at], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .context("query PCP health events")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collect PCP health events")?;
    Ok(rows
        .into_iter()
        .filter_map(
            |(occurred_at, principal, operation, scopes, decision, telemetry)| {
                let mut scopes = serde_json::from_str::<Vec<String>>(&scopes).ok()?;
                scopes.retain(|scope| allowed_scopes.contains(scope));
                if scopes.is_empty() {
                    return None;
                }
                Some(HealthEvent {
                    occurred_at,
                    principal: serde_json::from_str(&principal).ok()?,
                    operation,
                    scopes,
                    decision: AccessDecision::parse(&decision)?,
                    telemetry: telemetry
                        .as_deref()
                        .and_then(|value| serde_json::from_str(value).ok()),
                })
            },
        )
        .collect())
}

fn is_detail_read(telemetry: &OperationTelemetry) -> bool {
    telemetry.projections.iter().any(|projection| {
        matches!(
            projection.as_str(),
            "payload" | "sources" | "provenance" | "relations" | "facets" | "history"
        )
    })
}

fn is_write_operation(operation: &str) -> bool {
    matches!(
        operation,
        "assess_validity"
            | "pack_pages"
            | "create_scope"
            | "link_pages"
            | "mark_summary_assessed"
            | "retract"
            | "revise_page"
            | "write_page"
            | "write_summary"
    )
}

fn percentile(values: &mut [u64], percentile: usize) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let index = ((values.len() - 1) * percentile).div_ceil(100);
    values.get(index).copied()
}

fn timeline_bucket(value: &str, window_hours: u32) -> String {
    let Ok(timestamp) = DateTime::parse_from_rfc3339(value) else {
        return value.to_owned();
    };
    let timestamp = timestamp.with_timezone(&Utc);
    if window_hours <= 48 {
        timestamp
            .with_minute(0)
            .and_then(|value| value.with_second(0))
            .and_then(|value| value.with_nanosecond(0))
            .unwrap_or(timestamp)
            .to_rfc3339_opts(SecondsFormat::Secs, true)
    } else {
        timestamp.format("%Y-%m-%d").to_string()
    }
}

fn placeholders(count: usize) -> String {
    (1..=count)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn scope_values(scopes: &BTreeSet<String>) -> Vec<SqlValue> {
    scopes.iter().cloned().map(SqlValue::Text).collect()
}

#[cfg(test)]
mod tests {
    use super::{percentile, timeline_bucket};

    #[test]
    fn percentile_is_stable_for_small_samples() {
        assert_eq!(percentile(&mut [], 95), None);
        assert_eq!(percentile(&mut [8], 95), Some(8));
        assert_eq!(percentile(&mut [1, 2, 3, 4, 5], 50), Some(3));
        assert_eq!(percentile(&mut [1, 2, 3, 4, 5], 95), Some(5));
    }

    #[test]
    fn timeline_uses_hours_for_short_windows_and_days_for_long_windows() {
        assert_eq!(
            timeline_bucket("2026-08-04T12:34:56Z", 24),
            "2026-08-04T12:00:00Z"
        );
        assert_eq!(timeline_bucket("2026-08-04T12:34:56Z", 168), "2026-08-04");
    }
}
