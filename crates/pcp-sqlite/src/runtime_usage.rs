use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use chrono::{Duration, SecondsFormat, Utc};
use pcp_core::{ModelTokenUsage, RuntimeUsageEvent};
use pcp_store::{RuntimeModelUsageHealth, RuntimeModelUsageSourceHealth};
use rusqlite::params;

use crate::SqlitePcpStore;

const RUNTIME_USAGE_RETENTION_DAYS: i64 = 90;

impl SqlitePcpStore {
    pub(crate) async fn record_runtime_usage(&self, event: RuntimeUsageEvent) -> Result<()> {
        let retention_cutoff = (Utc::now() - Duration::days(RUNTIME_USAGE_RETENTION_DAYS))
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        self.run("record Runtime model usage", move |connection| {
            connection
                .execute(
                    "DELETE FROM pcp_runtime_usage WHERE occurred_at < ?1",
                    [retention_cutoff],
                )
                .context("prune expired Runtime model usage")?;
            connection
                .execute(
                    "
                    INSERT INTO pcp_runtime_usage (
                        event_id, occurred_at, principal_json, session_id, source,
                        operation, scopes_json, duration_ms, usage_json, failure_kind
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                    ",
                    params![
                        event.event_id,
                        event.occurred_at,
                        serde_json::to_string(&event.principal)?,
                        event.session_id,
                        event.source,
                        event.operation,
                        serde_json::to_string(&event.scopes)?,
                        i64::try_from(event.duration_ms).unwrap_or(i64::MAX),
                        event
                            .usage
                            .map(|usage| serde_json::to_string(&usage))
                            .transpose()?,
                        event.failure_kind,
                    ],
                )
                .context("insert Runtime model usage")?;
            Ok(())
        })
        .await
    }

    pub(crate) fn runtime_usage_health(
        connection: &rusqlite::Connection,
        allowed_scopes: &BTreeSet<String>,
        window_started_at: &str,
    ) -> Result<RuntimeModelUsageHealth> {
        let mut statement = connection
            .prepare(
                "
                SELECT source, operation, scopes_json, usage_json
                FROM pcp_runtime_usage
                WHERE occurred_at >= ?1
                ORDER BY occurred_at DESC, event_id DESC
                ",
            )
            .context("prepare Runtime model usage health")?;
        let rows = statement
            .query_map([window_started_at], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .context("query Runtime model usage health")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("collect Runtime model usage health")?;

        let mut aggregate = RuntimeModelUsageHealth::default();
        let mut sources = BTreeMap::<(String, String), RuntimeModelUsageSourceHealth>::new();
        for (source, operation, scopes_json, usage_json) in rows {
            let scopes: Vec<String> =
                serde_json::from_str(&scopes_json).context("decode Runtime model usage scopes")?;
            if !scopes.iter().any(|scope| allowed_scopes.contains(scope)) {
                continue;
            }
            aggregate.operations += 1;
            let entry = sources
                .entry((source.clone(), operation.clone()))
                .or_insert_with(|| RuntimeModelUsageSourceHealth {
                    source,
                    operation,
                    ..RuntimeModelUsageSourceHealth::default()
                });
            entry.operations += 1;
            let usage = usage_json
                .as_deref()
                .map(serde_json::from_str::<ModelTokenUsage>)
                .transpose()
                .context("decode Runtime model token usage")?
                .unwrap_or_else(|| ModelTokenUsage {
                    unreported_responses: 1,
                    ..ModelTokenUsage::default()
                });
            let calls = usage.response_count();
            aggregate.model_calls = aggregate.model_calls.saturating_add(calls);
            aggregate.reported_model_calls = aggregate
                .reported_model_calls
                .saturating_add(usage.reported_responses as u64);
            aggregate.unreported_model_calls = aggregate
                .unreported_model_calls
                .saturating_add(usage.unreported_responses as u64);
            aggregate.usage.add_assign(&usage);
            entry.model_calls = entry.model_calls.saturating_add(calls);
            entry.reported_model_calls = entry
                .reported_model_calls
                .saturating_add(usage.reported_responses as u64);
            entry.unreported_model_calls = entry
                .unreported_model_calls
                .saturating_add(usage.unreported_responses as u64);
            entry.usage.add_assign(&usage);
        }
        aggregate.sources = sources.into_values().collect();
        Ok(aggregate)
    }
}
