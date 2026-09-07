//! Filtered operator audit queries; authorization is resolved by the adapter.
use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use pcp_core::{
    AccessAuditEvent, AccessClientSummary, AccessDecision, AccessLogQuery, AccessLogResult,
    AccessOperationSummary,
};
use rusqlite::{Connection, Row, params_from_iter, types::Value};
use serde::{Deserialize, Serialize};

use crate::SqlitePcpStore;

#[derive(Deserialize, Serialize)]
struct Cursor {
    at: String,
    id: String,
}

fn timestamp(value: &str) -> Result<String> {
    Ok(DateTime::parse_from_rfc3339(value)
        .context("audit time must be RFC3339")?
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Millis, true))
}

impl SqlitePcpStore {
    pub(crate) async fn query_access_audit(
        &self,
        scopes: Vec<String>,
        mut query: AccessLogQuery,
    ) -> Result<AccessLogResult> {
        self.flush_access_audit().await?;
        query.since = query.since.as_deref().map(timestamp).transpose()?;
        query.until = query.until.as_deref().map(timestamp).transpose()?;
        anyhow::ensure!(
            query
                .since
                .as_ref()
                .zip(query.until.as_ref())
                .is_none_or(|(a, b)| a <= b),
            "audit time range is reversed"
        );
        let cursor: Option<Cursor> = query
            .cursor
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .context("invalid audit cursor")?;
        if scopes.is_empty() {
            return Ok(AccessLogResult::default());
        }
        self.run("filtered access audit", move |connection| {
            query_audit(connection, scopes, query, cursor)
        })
        .await
    }
}

fn query_audit(
    mut connection: Connection,
    scopes: Vec<String>,
    query: AccessLogQuery,
    cursor: Option<Cursor>,
) -> Result<AccessLogResult> {
    // Keep counts and rows in one read snapshot. Filter authorization before
    // aggregation, then project each returned event onto the same scopes.
    let tx = connection.transaction()?;
    let mut filter = "EXISTS (SELECT 1 FROM json_each(scopes_json) scope
        WHERE scope.value IN (SELECT value FROM json_each(?)))"
        .to_string();
    let mut values = vec![Value::Text(serde_json::to_string(&scopes)?)];
    if !query.include_health_checks {
        filter.push_str(" AND operation <> 'health_snapshot'");
    }
    for (clause, value) in [
        (" AND occurred_at >= ?", query.since),
        (" AND occurred_at <= ?", query.until),
        (
            " AND operation = ?",
            query.operation.filter(|s| !s.is_empty()),
        ),
    ] {
        if let Some(value) = value {
            filter.push_str(clause);
            values.push(Value::Text(value));
        }
    }
    // Client selection filters the timeline, not the client selector itself.
    // SQLite's single MAX aggregate selects principal_json from a latest row.
    let clients = {
        let mut stmt = tx.prepare(&format!(
            "SELECT principal_json, COUNT(*), MAX(occurred_at)
             FROM pcp_access_log WHERE {filter}
             GROUP BY json_extract(principal_json, '$.principalId')
             ORDER BY MAX(occurred_at) DESC, json_extract(principal_json, '$.principalId')"
        ))?;
        let rows = stmt
            .query_map(params_from_iter(&values), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(principal, event_count, last_access_at)| {
                Ok(AccessClientSummary {
                    principal: serde_json::from_str(&principal)?,
                    event_count,
                    last_access_at,
                })
            })
            .collect::<Result<Vec<_>>>()?
    };
    if let Some(principal) = query.principal_id.filter(|s| !s.is_empty()) {
        filter.push_str(" AND json_extract(principal_json, '$.principalId') = ?");
        values.push(Value::Text(principal));
    }
    let total_events = tx.query_row(
        &format!("SELECT COUNT(*) FROM pcp_access_log WHERE {filter}"),
        params_from_iter(&values),
        |row| row.get::<_, i64>(0),
    )? as u64;
    let operations = {
        let mut stmt = tx.prepare(&format!(
            "SELECT operation, COUNT(*) FROM pcp_access_log WHERE {filter}
             GROUP BY operation ORDER BY COUNT(*) DESC, operation"
        ))?;
        stmt.query_map(params_from_iter(&values), |row| {
            Ok(AccessOperationSummary {
                operation: row.get(0)?,
                event_count: row.get::<_, i64>(1)? as u64,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    // Keyset pagination stays stable when newer events arrive between requests.
    if let Some(cursor) = cursor {
        filter.push_str(" AND (occurred_at < ? OR (occurred_at = ? AND event_id < ?))");
        values.extend([
            Value::Text(cursor.at.clone()),
            Value::Text(cursor.at),
            Value::Text(cursor.id),
        ]);
    }
    let limit = query.limit.unwrap_or(50).clamp(1, 100) as usize;
    values.push(Value::Integer((limit + 1) as i64));
    let mut events = {
        let mut stmt = tx.prepare(&format!(
            "SELECT event_id, occurred_at, principal_json, session_id, operation,
                    scopes_json, decision, detail, telemetry_json
             FROM pcp_access_log WHERE {filter}
             ORDER BY occurred_at DESC, event_id DESC LIMIT ?"
        ))?;
        stmt.query_map(params_from_iter(&values), read_event)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for event in &mut events {
        event.scopes.retain(|scope| scopes.contains(scope));
    }
    let more = events.len() > limit;
    events.truncate(limit);
    let next_cursor = if more {
        events
            .last()
            .map(|event| {
                serde_json::to_string(&Cursor {
                    at: event.occurred_at.clone(),
                    id: event.event_id.clone(),
                })
            })
            .transpose()?
    } else {
        None
    };
    Ok(AccessLogResult {
        events,
        next_cursor,
        total_events,
        clients,
        operations,
    })
}

fn read_event(row: &Row<'_>) -> rusqlite::Result<AccessAuditEvent> {
    fn decode<T: serde::de::DeserializeOwned>(row: &Row<'_>, column: usize) -> rusqlite::Result<T> {
        let value: String = row.get(column)?;
        serde_json::from_str(&value).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                column,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
    }
    let decision: String = row.get(6)?;
    let telemetry: Option<String> = row.get(8)?;
    Ok(AccessAuditEvent {
        event_id: row.get(0)?,
        occurred_at: row.get(1)?,
        principal: decode(row, 2)?,
        session_id: row.get(3)?,
        operation: row.get(4)?,
        scopes: decode(row, 5)?,
        decision: AccessDecision::parse(&decision).ok_or_else(|| {
            rusqlite::Error::InvalidColumnType(6, "decision".into(), rusqlite::types::Type::Text)
        })?,
        detail: row.get(7)?,
        telemetry: telemetry.map(|_| decode(row, 8)).transpose()?,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use pcp_core::{
        AccessPermission, AccessPrincipal, AccessPrincipalType, AccessSession, ScopeGrant,
    };
    use pcp_store::PcpStore;
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn access(audit: bool) -> AccessSession {
        AccessSession::new(
            AccessPrincipal {
                principal_id: "test:auditor".into(),
                principal_type: AccessPrincipalType::Service,
                display_name: None,
            },
            "test:session",
            vec![ScopeGrant {
                namespace: "a".into(),
                permissions: if audit {
                    vec![AccessPermission::Audit]
                } else {
                    vec![AccessPermission::ReadDetail]
                },
            }],
        )
    }
    async fn fixture() -> (SqlitePcpStore, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "pcp-audit-query-{}-{}-{}.sqlite",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = SqlitePcpStore::open(path.clone()).await.unwrap();
        store.run("seed audit", |c| {
            for (id,at,client,op,scopes) in [
                ("01","2026-09-01T01:00:00.000Z","client:a","search_pages",r#"["a"]"#),
                ("02","2026-09-01T02:00:00.000Z","client:b","read_pages",r#"["a","b"]"#),
                ("03","2026-09-01T02:00:00.000Z","client:a","read_pages",r#"["a"]"#),
                ("04","2026-09-01T03:00:00.000Z","hidden:client","read_pages",r#"["b"]"#),
                ("05","2026-09-01T04:00:00.000Z","service:observer","health_snapshot",r#"["a"]"#),
            ] {
                c.execute("INSERT INTO pcp_access_log (event_id,occurred_at,principal_json,session_id,operation,scopes_json,decision) VALUES (?1,?2,?3,'session:one',?4,?5,'allowed')",rusqlite::params![id,at,serde_json::json!({"principalId":client,"principalType":"service","displayName":client}).to_string(),op,scopes])?;
            }
            Ok(())
        }).await.unwrap();
        (store, path)
    }
    #[tokio::test]
    async fn audit_query_filters_before_counts_and_projects_authorized_scopes() {
        let (store, path) = fixture().await;
        assert!(
            store
                .query_access_log(&access(false), AccessLogQuery::default())
                .await
                .is_err()
        );
        let result = store
            .query_access_log(&access(true), AccessLogQuery::default())
            .await
            .unwrap();
        assert_eq!(result.total_events, 3);
        assert_eq!(result.clients.len(), 2);
        assert!(result.events.iter().all(|e| e.scopes == ["a"]));
        assert_eq!(
            result.operations.iter().map(|o| o.event_count).sum::<u64>(),
            3
        );
        let selected = store
            .query_access_log(
                &access(true),
                AccessLogQuery {
                    principal_id: Some("client:a".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(selected.total_events, 2);
        assert_eq!(selected.clients.len(), 2);
        assert!(
            selected
                .events
                .iter()
                .all(|e| e.principal.principal_id == "client:a")
        );
        let health = store
            .query_access_log(
                &access(true),
                AccessLogQuery {
                    include_health_checks: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(health.total_events, 4);
        drop(store);
        let _ = std::fs::remove_file(path);
    }
    #[tokio::test]
    async fn audit_query_time_operation_and_empty_match_are_exact() {
        let (store, path) = fixture().await;
        let q = AccessLogQuery {
            operation: Some("read_pages".into()),
            since: Some("2026-09-01T10:00:00+08:00".into()),
            until: Some("2026-09-01T02:00:00Z".into()),
            ..Default::default()
        };
        let result = store.query_access_log(&access(true), q).await.unwrap();
        assert_eq!(result.total_events, 2);
        let result = store
            .query_access_log(
                &access(true),
                AccessLogQuery {
                    principal_id: Some("' OR 1=1 --".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(result.total_events, 0);
        assert!(
            store
                .query_access_log(
                    &access(true),
                    AccessLogQuery {
                        since: Some("yesterday".into()),
                        ..Default::default()
                    }
                )
                .await
                .is_err()
        );
        assert!(
            store
                .query_access_log(
                    &access(true),
                    AccessLogQuery {
                        cursor: Some("bad".into()),
                        ..Default::default()
                    }
                )
                .await
                .is_err()
        );
        drop(store);
        let _ = std::fs::remove_file(path);
    }
    #[tokio::test]
    async fn audit_query_cursor_survives_new_events_and_timestamp_ties() {
        let (store, path) = fixture().await;
        let first = store
            .query_access_log(
                &access(true),
                AccessLogQuery {
                    limit: Some(1),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(first.events[0].event_id, "03");
        assert_eq!(first.total_events, 3);
        store.run("new audit event",|c|{c.execute("INSERT INTO pcp_access_log SELECT '06','2026-09-02T02:00:00.000Z',principal_json,session_id,operation,scopes_json,decision,detail,telemetry_json FROM pcp_access_log WHERE event_id='01'",[])?;Ok(())}).await.unwrap();
        let next = store
            .query_access_log(
                &access(true),
                AccessLogQuery {
                    limit: Some(1),
                    cursor: first.next_cursor,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(next.events[0].event_id, "02");
        assert_eq!(next.total_events, 4);
        let last = store
            .query_access_log(
                &access(true),
                AccessLogQuery {
                    limit: Some(1),
                    cursor: next.next_cursor,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(last.events[0].event_id, "01");
        assert!(last.next_cursor.is_none());
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
