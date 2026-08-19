use std::collections::BTreeSet;

use anyhow::{Context, Result};
use chrono::{Duration, SecondsFormat, Utc};
use pcp_core::{AccessDecision, QueryAuditEvent, QueryAuditMethod, RouterTokenUsage};
use pcp_store::{QueryAuditMethodHealth, QueryAuditSummary};
use rusqlite::params;

use crate::SqlitePcpStore;

const MIN_WINDOW_HOURS: u32 = 1;
const MAX_WINDOW_HOURS: u32 = 24 * 90;
const RECENT_EVENT_LIMIT: usize = 12;
const QUERY_AUDIT_RETENTION_DAYS: i64 = 90;

#[derive(Default)]
struct MethodAccumulator {
    calls: u64,
    allowed: u64,
    failed: u64,
    anchors: u64,
    related_contexts: u64,
    context_chars: u64,
    durations: Vec<u64>,
}

impl MethodAccumulator {
    fn observe(&mut self, event: &QueryAuditEvent) {
        self.calls += 1;
        self.allowed += u64::from(event.decision == AccessDecision::Allowed);
        self.failed += u64::from(event.decision != AccessDecision::Allowed);
        self.anchors = self.anchors.saturating_add(event.anchor_count);
        self.related_contexts = self.related_contexts.saturating_add(event.related_count);
        self.context_chars = self.context_chars.saturating_add(event.context_chars);
        self.durations.push(event.duration_ms);
    }

    fn finish(mut self) -> QueryAuditMethodHealth {
        QueryAuditMethodHealth {
            calls: self.calls,
            allowed: self.allowed,
            failed: self.failed,
            anchors: self.anchors,
            related_contexts: self.related_contexts,
            context_chars: self.context_chars,
            p50_duration_ms: percentile(&mut self.durations, 50),
            p95_duration_ms: percentile(&mut self.durations, 95),
        }
    }
}

impl SqlitePcpStore {
    pub(crate) async fn record_runtime_query_audit(&self, event: QueryAuditEvent) -> Result<()> {
        let retention_cutoff = (Utc::now() - Duration::days(QUERY_AUDIT_RETENTION_DAYS))
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        self.run("record Runtime query audit", move |connection| {
            connection
                .execute(
                    "DELETE FROM pcp_query_audit WHERE occurred_at < ?1",
                    [retention_cutoff],
                )
                .context("prune expired Runtime query audit")?;
            connection
                .execute(
                    "
                    INSERT INTO pcp_query_audit (
                        event_id, occurred_at, principal_json, session_id, method, effort,
                        scopes_json, decision, duration_ms, anchor_count, related_count,
                        context_chars, semantic_indexed_count, semantic_embedded_count,
                        router_rounds, router_usage_json, failure_kind
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17
                    )
                    ",
                    params![
                        event.event_id,
                        event.occurred_at,
                        serde_json::to_string(&event.principal)?,
                        event.session_id,
                        query_method_name(event.method),
                        event.effort.map(intent_effort_name),
                        serde_json::to_string(&event.scopes)?,
                        event.decision.as_str(),
                        i64::try_from(event.duration_ms).unwrap_or(i64::MAX),
                        i64::try_from(event.anchor_count).unwrap_or(i64::MAX),
                        i64::try_from(event.related_count).unwrap_or(i64::MAX),
                        i64::try_from(event.context_chars).unwrap_or(i64::MAX),
                        event
                            .semantic_indexed_count
                            .map(|value| i64::try_from(value).unwrap_or(i64::MAX)),
                        event
                            .semantic_embedded_count
                            .map(|value| i64::try_from(value).unwrap_or(i64::MAX)),
                        event
                            .router_rounds
                            .map(|value| i64::try_from(value).unwrap_or(i64::MAX)),
                        event
                            .router_usage
                            .map(|usage| serde_json::to_string(&usage))
                            .transpose()?,
                        event.failure_kind,
                    ],
                )
                .context("insert Runtime query audit")?;
            Ok(())
        })
        .await
    }

    pub(crate) async fn query_audit_summary(
        &self,
        allowed_scopes: Vec<String>,
        window_hours: u32,
    ) -> Result<QueryAuditSummary> {
        let window_hours = window_hours.clamp(MIN_WINDOW_HOURS, MAX_WINDOW_HOURS);
        let generated = Utc::now();
        let window_started = generated - Duration::hours(i64::from(window_hours));
        let generated_at = generated.to_rfc3339_opts(SecondsFormat::Millis, true);
        let window_started_at = window_started.to_rfc3339_opts(SecondsFormat::Millis, true);
        let allowed_scopes = allowed_scopes.into_iter().collect::<BTreeSet<_>>();
        if allowed_scopes.is_empty() {
            return Ok(empty_summary(generated_at, window_started_at, window_hours));
        }

        self.run("query audit summary", move |connection| {
            let mut statement = connection
                .prepare(
                    "
                    SELECT event_id, occurred_at, principal_json, session_id, method, effort,
                           scopes_json, decision, duration_ms, anchor_count, related_count,
                           context_chars, semantic_indexed_count, semantic_embedded_count,
                           router_rounds, router_usage_json, failure_kind
                    FROM pcp_query_audit
                    WHERE occurred_at >= ?1
                    ORDER BY occurred_at DESC, event_id DESC
                    ",
                )
                .context("prepare query audit summary")?;
            let rows = statement
                .query_map([window_started_at.clone()], row_to_event)
                .context("query Runtime query audit")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("collect Runtime query audit")?;

            let mut semantic_search = MethodAccumulator::default();
            let mut match_intent = MethodAccumulator::default();
            let mut router_usage = RouterTokenUsage::default();
            let mut calls = 0_u64;
            let mut allowed = 0_u64;
            let mut failed = 0_u64;
            let mut recent_events = Vec::new();
            for mut event in rows {
                event.scopes.retain(|scope| allowed_scopes.contains(scope));
                if event.scopes.is_empty() {
                    continue;
                }
                calls += 1;
                allowed += u64::from(event.decision == AccessDecision::Allowed);
                failed += u64::from(event.decision != AccessDecision::Allowed);
                match event.method {
                    QueryAuditMethod::SemanticSearch => semantic_search.observe(&event),
                    QueryAuditMethod::MatchIntent => match_intent.observe(&event),
                }
                if let Some(usage) = event.router_usage.as_ref() {
                    add_usage(&mut router_usage, usage);
                }
                if recent_events.len() < RECENT_EVENT_LIMIT {
                    recent_events.push(event);
                }
            }
            Ok(QueryAuditSummary {
                generated_at,
                window_started_at,
                window_hours,
                calls,
                allowed,
                failed,
                semantic_search: semantic_search.finish(),
                match_intent: match_intent.finish(),
                router_usage,
                recent_events,
            })
        })
        .await
    }
}

fn empty_summary(
    generated_at: String,
    window_started_at: String,
    window_hours: u32,
) -> QueryAuditSummary {
    QueryAuditSummary {
        generated_at,
        window_started_at,
        window_hours,
        calls: 0,
        allowed: 0,
        failed: 0,
        semantic_search: QueryAuditMethodHealth::default(),
        match_intent: QueryAuditMethodHealth::default(),
        router_usage: RouterTokenUsage::default(),
        recent_events: Vec::new(),
    }
}

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<QueryAuditEvent> {
    let method = row.get::<_, String>(4)?;
    let effort = row.get::<_, Option<String>>(5)?;
    let decision = row.get::<_, String>(7)?;
    let router_usage = row.get::<_, Option<String>>(15)?;
    let scopes = row.get::<_, String>(6)?;
    Ok(QueryAuditEvent {
        event_id: row.get(0)?,
        occurred_at: row.get(1)?,
        principal: serde_json::from_str(&row.get::<_, String>(2)?).map_err(json_error)?,
        session_id: row.get(3)?,
        method: parse_method(&method).ok_or_else(|| invalid_data("invalid query audit method"))?,
        effort: effort.as_deref().map(parse_intent_effort).transpose()?,
        scopes: serde_json::from_str(&scopes).map_err(json_error)?,
        decision: AccessDecision::parse(&decision)
            .ok_or_else(|| invalid_data("invalid query audit decision"))?,
        duration_ms: row.get::<_, i64>(8)?.max(0) as u64,
        anchor_count: row.get::<_, i64>(9)?.max(0) as u64,
        related_count: row.get::<_, i64>(10)?.max(0) as u64,
        context_chars: row.get::<_, i64>(11)?.max(0) as u64,
        semantic_indexed_count: row
            .get::<_, Option<i64>>(12)?
            .map(|value| value.max(0) as u64),
        semantic_embedded_count: row
            .get::<_, Option<i64>>(13)?
            .map(|value| value.max(0) as u64),
        router_rounds: row
            .get::<_, Option<i64>>(14)?
            .map(|value| value.max(0) as u64),
        router_usage: router_usage
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(json_error)?,
        failure_kind: row.get(16)?,
    })
}

fn query_method_name(method: QueryAuditMethod) -> &'static str {
    match method {
        QueryAuditMethod::SemanticSearch => "semantic_search",
        QueryAuditMethod::MatchIntent => "match_intent",
    }
}

fn parse_method(value: &str) -> Option<QueryAuditMethod> {
    match value {
        "semantic_search" => Some(QueryAuditMethod::SemanticSearch),
        "match_intent" => Some(QueryAuditMethod::MatchIntent),
        _ => None,
    }
}

fn intent_effort_name(value: pcp_core::IntentEffort) -> &'static str {
    match value {
        pcp_core::IntentEffort::Low => "low",
        pcp_core::IntentEffort::Medium => "medium",
        pcp_core::IntentEffort::High => "high",
    }
}

fn parse_intent_effort(value: &str) -> Result<pcp_core::IntentEffort, rusqlite::Error> {
    match value {
        "low" => Ok(pcp_core::IntentEffort::Low),
        "medium" => Ok(pcp_core::IntentEffort::Medium),
        "high" => Ok(pcp_core::IntentEffort::High),
        _ => Err(invalid_data("invalid query audit effort")),
    }
}

fn invalid_data(message: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message,
        )),
    )
}

fn json_error(error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

fn add_usage(total: &mut RouterTokenUsage, value: &RouterTokenUsage) {
    total.reported_responses = total
        .reported_responses
        .saturating_add(value.reported_responses);
    total.unreported_responses = total
        .unreported_responses
        .saturating_add(value.unreported_responses);
    total.input_tokens = total.input_tokens.saturating_add(value.input_tokens);
    total.output_tokens = total.output_tokens.saturating_add(value.output_tokens);
    total.total_tokens = total.total_tokens.saturating_add(value.total_tokens);
    total.cached_input_tokens = total
        .cached_input_tokens
        .saturating_add(value.cached_input_tokens);
    total.reasoning_tokens = total
        .reasoning_tokens
        .saturating_add(value.reasoning_tokens);
}

fn percentile(values: &mut [u64], percentile: usize) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let index = ((values.len() - 1) * percentile) / 100;
    values.get(index).copied()
}
