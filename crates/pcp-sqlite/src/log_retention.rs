//! Retention for disposable operational observations, never Page evidence.
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{Duration as ChronoDuration, SecondsFormat, Utc};
use rusqlite::{Connection, params};

const MIB: u64 = 1024 * 1024;
pub(crate) const DELETE_BATCH: usize = 5_000;

#[derive(Clone, Copy, Debug)]
pub(crate) struct LogPolicy {
    pub(crate) table: &'static str,
    pub(crate) predicate: &'static str,
    pub(crate) retention: Duration,
    pub(crate) max_events: usize,
    pub(crate) max_bytes: u64,
    pub(crate) bytes_sql: &'static str,
}

const ACCESS_BYTES: &str = "128 + length(CAST(event_id AS BLOB))
    + length(CAST(occurred_at AS BLOB)) + length(CAST(decision AS BLOB))
    + length(CAST(principal_json AS BLOB))
    + length(CAST(session_id AS BLOB)) + length(CAST(operation AS BLOB))
    + length(CAST(scopes_json AS BLOB)) + COALESCE(length(CAST(detail AS BLOB)), 0)
    + COALESCE(length(CAST(telemetry_json AS BLOB)), 0)";

pub(crate) const ACCESS_SUCCESS: LogPolicy = LogPolicy {
    table: "pcp_access_log",
    predicate: "decision = 'allowed'",
    retention: Duration::from_secs(7 * 86_400),
    max_events: 10_000,
    max_bytes: 8 * MIB,
    bytes_sql: ACCESS_BYTES,
};

pub(crate) const ACCESS_FAILURE: LogPolicy = LogPolicy {
    predicate: "decision <> 'allowed'",
    retention: Duration::from_secs(30 * 86_400),
    max_events: 2_000,
    max_bytes: 4 * MIB,
    ..ACCESS_SUCCESS
};

pub(crate) const QUERY: LogPolicy = LogPolicy {
    table: "pcp_query_audit",
    predicate: "1",
    retention: Duration::from_secs(7 * 86_400),
    max_events: 2_000,
    max_bytes: 4 * MIB,
    bytes_sql: "192 + length(CAST(principal_json AS BLOB))
        + length(CAST(session_id AS BLOB)) + length(CAST(scopes_json AS BLOB))
        + COALESCE(length(CAST(router_usage_json AS BLOB)), 0)
        + COALESCE(length(CAST(failure_kind AS BLOB)), 0)
        + length(CAST(occurred_at AS BLOB))
        + length(CAST(event_id AS BLOB))",
};

pub(crate) const MODEL_USAGE: LogPolicy = LogPolicy {
    table: "pcp_runtime_usage",
    predicate: "1",
    retention: Duration::from_secs(90 * 86_400),
    max_events: 50_000,
    max_bytes: 32 * MIB,
    bytes_sql: "128 + length(CAST(principal_json AS BLOB))
        + length(CAST(session_id AS BLOB)) + length(CAST(scopes_json AS BLOB))
        + length(CAST(source AS BLOB)) + length(CAST(operation AS BLOB))
        + COALESCE(length(CAST(usage_json AS BLOB)), 0)
        + COALESCE(length(CAST(failure_kind AS BLOB)), 0)
        + length(CAST(occurred_at AS BLOB))
        + length(CAST(event_id AS BLOB))",
};

/// Find the first excess event using a bounded, indexed newest-first scan.
/// Count and UTF-8 byte budgets are independent; a single oversized event
/// cannot evade a row-count limit. No event JSON is deserialized here.
pub(crate) fn prune(
    connection: &Connection,
    policy: LogPolicy,
    delete_limit: usize,
) -> Result<usize> {
    let cutoff = (Utc::now() - ChronoDuration::from_std(policy.retention)?)
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let boundary = {
        let mut query = connection.prepare(&format!(
            "SELECT occurred_at, event_id, {} FROM {} WHERE {}
             ORDER BY occurred_at DESC, event_id DESC LIMIT ?1",
            policy.bytes_sql, policy.table, policy.predicate,
        ))?;
        let mut rows = query.query([i64::try_from(policy.max_events.saturating_add(1))?])?;
        let mut bytes = 0_u64;
        let mut count = 0;
        let mut boundary = None;
        while let Some(row) = rows.next()? {
            count += 1;
            bytes = bytes.saturating_add(row.get::<_, i64>(2)?.max(0) as u64);
            if count > policy.max_events || bytes > policy.max_bytes {
                boundary = Some((row.get::<_, String>(0)?, row.get::<_, String>(1)?));
                break;
            }
        }
        boundary.unwrap_or_default()
    };
    connection
        .execute(
            &format!(
                "DELETE FROM {} WHERE rowid IN (
                    SELECT rowid FROM {} WHERE {} AND
                    (occurred_at < ?1 OR (occurred_at, event_id) <= (?2, ?3))
                    ORDER BY occurred_at ASC, event_id ASC LIMIT ?4
                )",
                policy.table, policy.table, policy.predicate,
            ),
            params![cutoff, boundary.0, boundary.1, i64::try_from(delete_limit)?],
        )
        .with_context(|| format!("bound {} operational history", policy.table))
}

/// A committed append starts from a bounded history. Drain its excess before
/// commit, so bursts and unusually large records cannot accumulate between sweeps.
pub(crate) fn prune_after_append(connection: &Connection, policy: LogPolicy) -> Result<()> {
    while prune(connection, policy, DELETE_BATCH)? == DELETE_BATCH {}
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE pcp_access_log (
            event_id TEXT, occurred_at TEXT, decision TEXT, principal_json TEXT,
            session_id TEXT, operation TEXT, scopes_json TEXT, detail TEXT,
            telemetry_json TEXT);
            CREATE INDEX audit_time ON pcp_access_log(occurred_at DESC,event_id DESC);
            CREATE TABLE memories (content TEXT);
            INSERT INTO memories VALUES ('durable source evidence');",
        )
        .unwrap();
        c
    }

    fn insert(c: &Connection, id: &str, at: &str, decision: &str, detail: &str) {
        c.execute(
            "INSERT INTO pcp_access_log VALUES (?1,?2,?3,'{}','s','read_pages','[]',?4,NULL)",
            params![id, at, decision, detail],
        )
        .unwrap();
    }

    fn ids(c: &Connection) -> Vec<String> {
        c.prepare("SELECT event_id FROM pcp_access_log ORDER BY event_id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn success_and_error_budgets_expire_independently_without_touching_memories() {
        let c = fixture();
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let old =
            (Utc::now() - ChronoDuration::days(10)).to_rfc3339_opts(SecondsFormat::Millis, true);
        let expired =
            (Utc::now() - ChronoDuration::days(31)).to_rfc3339_opts(SecondsFormat::Millis, true);
        for id in ["a", "b", "c"] {
            insert(&c, id, &now, "allowed", "");
        }
        insert(&c, "old-success", &old, "allowed", "");
        insert(&c, "old-denial", &old, "denied", "");
        insert(&c, "expired-failure", &expired, "failed", "");
        prune_after_append(
            &c,
            LogPolicy {
                max_events: 2,
                ..ACCESS_SUCCESS
            },
        )
        .unwrap();
        prune_after_append(&c, ACCESS_FAILURE).unwrap();
        assert_eq!(ids(&c), vec!["b", "c", "old-denial"]);
        assert_eq!(
            c.query_row("SELECT content FROM memories", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "durable source evidence"
        );
    }

    #[test]
    fn byte_budget_uses_utf8_bytes_and_preserves_newest_events() {
        let c = fixture();
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        for id in ["a", "b", "c"] {
            insert(&c, id, &now, "failed", &"界".repeat(100));
        }
        let bytes = c
            .query_row(
                &format!("SELECT {} FROM pcp_access_log LIMIT 1", ACCESS_BYTES),
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap() as u64;
        prune_after_append(
            &c,
            LogPolicy {
                max_bytes: bytes * 2,
                ..ACCESS_FAILURE
            },
        )
        .unwrap();
        assert_eq!(ids(&c), vec!["b", "c"]);
        insert(&c, "z", &now, "failed", &"x".repeat(10_000));
        prune_after_append(
            &c,
            LogPolicy {
                max_bytes: bytes * 2,
                ..ACCESS_FAILURE
            },
        )
        .unwrap();
        assert!(ids(&c).is_empty());
    }

    #[test]
    fn legacy_cleanup_has_a_bounded_delete_batch() {
        let c = fixture();
        for n in 0..10 {
            insert(
                &c,
                &n.to_string(),
                "2000-01-01T00:00:00.000Z",
                "allowed",
                "",
            );
        }
        assert_eq!(prune(&c, ACCESS_SUCCESS, 3).unwrap(), 3);
        assert_eq!(ids(&c).len(), 7);
        prune_after_append(&c, ACCESS_SUCCESS).unwrap();
        assert!(ids(&c).is_empty());
    }

    #[tokio::test]
    async fn real_query_and_usage_appends_enforce_their_own_caps() {
        use pcp_core::{
            AccessDecision, AccessPrincipal, AccessPrincipalType, ModelTokenUsage, QueryAuditEvent,
            QueryAuditMethod, RuntimeUsageEvent,
        };
        let root = std::env::temp_dir().join(format!(
            "pcp-log-contract-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let store = crate::SqlitePcpStore::open(root.join("pcp.sqlite3"))
            .await
            .unwrap();
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let at = now.clone();
        store.run("seed operational overflow",move |c| {
            c.execute("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<2001)
                INSERT INTO pcp_query_audit (event_id,occurred_at,principal_json,session_id,method,scopes_json,decision,duration_ms,anchor_count,related_count,context_chars)
                SELECT printf('seed_%06d',i),?1,'{}','s','semantic_search','[]','allowed',0,0,0,0 FROM n",[&at])?;
            c.execute("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<50001)
                INSERT INTO pcp_runtime_usage (event_id,occurred_at,principal_json,session_id,source,operation,scopes_json,duration_ms)
                SELECT printf('seed_%06d',i),?1,'{}','s','test','test','[]',0 FROM n",[&at])?;
            Ok(())
        }).await.unwrap();
        let principal = AccessPrincipal {
            principal_id: "model:test".into(),
            principal_type: AccessPrincipalType::ModelClient,
            display_name: None,
        };
        store
            .record_runtime_query_audit(QueryAuditEvent {
                event_id: "zz_new_query".into(),
                occurred_at: now.clone(),
                principal: principal.clone(),
                session_id: "s".into(),
                method: QueryAuditMethod::SemanticSearch,
                effort: None,
                scopes: vec!["project:test".into()],
                decision: AccessDecision::Allowed,
                duration_ms: 10,
                anchor_count: 1,
                related_count: 0,
                context_chars: 100,
                semantic_indexed_count: Some(1),
                semantic_embedded_count: Some(0),
                router_rounds: None,
                router_usage: None,
                failure_kind: None,
            })
            .await
            .unwrap();
        store
            .record_runtime_usage(RuntimeUsageEvent {
                event_id: "zz_new_usage".into(),
                occurred_at: now,
                principal,
                session_id: "s".into(),
                source: "automatic_maintenance".into(),
                operation: "test".into(),
                scopes: vec!["project:test".into()],
                duration_ms: 10,
                usage: Some(ModelTokenUsage {
                    reported_responses: 1,
                    input_tokens: 100,
                    output_tokens: 25,
                    total_tokens: 125,
                    ..Default::default()
                }),
                failure_kind: None,
            })
            .await
            .unwrap();
        store.run("verify append caps",|c| {
            assert_eq!(c.query_row("SELECT count(*) FROM pcp_query_audit",[],|r|r.get::<_,i64>(0))?,QUERY.max_events as i64);
            assert_eq!(c.query_row("SELECT count(*) FROM pcp_runtime_usage",[],|r|r.get::<_,i64>(0))?,MODEL_USAGE.max_events as i64);
            assert_eq!(c.query_row("SELECT json_extract(usage_json,'$.totalTokens') FROM pcp_runtime_usage WHERE event_id='zz_new_usage'",[],|r|r.get::<_,i64>(0))?,125);
            Ok(())
        }).await.unwrap();
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
}
