use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use anyhow::Result;
use chrono::{SecondsFormat, Utc};
use pcp_core::{AccessPermission, AccessPrincipal, AccessPrincipalType, AccessSession, ScopeGrant};
use pcp_store::{
    ActivityHealth, ConsolidationHealth, GraphHealth, HealthSnapshot, PcpStore, RecallHealth,
    StorageHealth,
};
use serde_json::{Value, json};
use tokio::sync::RwLock;

use super::contract::{
    IssueSeverity, MetricKind, ObserverExtensions, ObserverIssue, ObserverLinks, ObserverMetric,
    ObserverRedaction, ObserverState, ObserverStatus, PCP_OBSERVER_PROTOCOL_VERSION, PcpExtension,
    PcpIntegrity, SNAPSHOT_SCHEMA, SnapshotEnvelope, SnapshotService,
};

const SERVICE_KIND: &str = "pcp";
const SNAPSHOT_WINDOW_HOURS: u32 = 24;
const SNAPSHOT_WINDOW_SECONDS: u64 = 24 * 60 * 60;

pub(super) struct SnapshotSource {
    store: Arc<dyn PcpStore>,
    instance_id: String,
    generation: String,
    console_url: Option<String>,
    started: Instant,
    sequence: AtomicU64,
    integrity: RwLock<IntegrityObservation>,
}

impl SnapshotSource {
    pub(super) fn new(
        store: Arc<dyn PcpStore>,
        instance_id: String,
        generation: String,
        console_url: Option<String>,
    ) -> Self {
        Self {
            store,
            instance_id,
            generation,
            console_url,
            started: Instant::now(),
            sequence: AtomicU64::new(1),
            integrity: RwLock::new(IntegrityObservation::default()),
        }
    }

    pub(super) async fn refresh_integrity(&self) {
        let result = self.store.integrity_check().await;
        let state = match result {
            Ok(value) if value == "ok" => IntegrityState::Ok,
            Ok(_) | Err(_) => IntegrityState::Failed,
        };
        *self.integrity.write().await = IntegrityObservation {
            state,
            checked_at: Some(timestamp()),
        };
    }

    pub(super) async fn capture(&self) -> Result<SnapshotEnvelope> {
        let captured_at = timestamp();
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        let integrity = self.integrity.read().await.clone();
        let scopes = self.store.local_scope_names().await;
        let (scope_count, health) = match scopes {
            Ok(scopes) if scopes.is_empty() => (0, Some(empty_health_snapshot())),
            Ok(scopes) => {
                let access = observer_access(&self.generation, &scopes);
                let scope_count = scopes.len().try_into().unwrap_or(u64::MAX);
                (
                    scope_count,
                    self.store
                        .health_snapshot(&access, scopes, SNAPSHOT_WINDOW_HOURS)
                        .await
                        .ok(),
                )
            }
            Err(_) => (0, None),
        };

        let mut reason_codes = Vec::new();
        let mut issues = Vec::new();
        match integrity.state {
            IntegrityState::Pending => reason_codes.push("pcp.integrity_pending".to_owned()),
            IntegrityState::Ok => {}
            IntegrityState::Failed => {
                reason_codes.push("pcp.integrity_failed".to_owned());
                issues.push(issue(
                    "pcp.integrity_failed",
                    IssueSeverity::Critical,
                    &captured_at,
                ));
            }
        }
        if health.is_none() {
            reason_codes.push("pcp.health_snapshot_failed".to_owned());
            issues.push(issue(
                "pcp.health_snapshot_failed",
                IssueSeverity::Critical,
                &captured_at,
            ));
        }
        let state = if health.is_none() || integrity.state == IntegrityState::Failed {
            ObserverState::Degraded
        } else if integrity.state == IntegrityState::Pending {
            ObserverState::Starting
        } else {
            ObserverState::Healthy
        };

        let mut metrics = vec![metric(
            "process.uptime_seconds",
            MetricKind::Gauge,
            json!(self.started.elapsed().as_secs()),
            Some("seconds"),
            None,
        )];
        if let Some(health) = health.as_ref() {
            metrics.extend(health_metrics(health));
        }
        let headline_metrics = [
            "requests.total",
            "requests.latency.p95_ms",
            "pcp.pages.current",
            "process.uptime_seconds",
        ]
        .into_iter()
        .filter(|id| metrics.iter().any(|metric| metric.id == *id))
        .take(3)
        .map(str::to_owned)
        .collect();
        let mut sanitized_health = health;
        if let Some(health) = sanitized_health.as_mut() {
            health.scopes.clear();
        }
        let capabilities = self.store.capabilities();
        Ok(SnapshotEnvelope {
            schema: SNAPSHOT_SCHEMA.to_owned(),
            schema_version: PCP_OBSERVER_PROTOCOL_VERSION.to_owned(),
            service: SnapshotService {
                kind: SERVICE_KIND.to_owned(),
                instance_id: self.instance_id.clone(),
                generation: self.generation.clone(),
            },
            sequence,
            captured_at,
            status: ObserverStatus {
                state,
                reason_codes,
            },
            headline_metrics,
            metrics,
            issues,
            links: ObserverLinks {
                console_url: self.console_url.clone(),
            },
            extensions: ObserverExtensions {
                pcp: PcpExtension {
                    protocol_version: capabilities.protocol_version.clone(),
                    capabilities,
                    integrity: PcpIntegrity {
                        state: integrity.state.as_str().to_owned(),
                        checked_at: integrity.checked_at,
                    },
                    scope_count,
                    health: sanitized_health,
                },
            },
            redaction: ObserverRedaction {
                excluded: vec![
                    "page_content".to_owned(),
                    "query_text".to_owned(),
                    "scope_names".to_owned(),
                    "raw_audit".to_owned(),
                    "storage_paths".to_owned(),
                ],
            },
        })
    }
}

fn observer_access(generation: &str, scopes: &[String]) -> AccessSession {
    AccessSession::new(
        AccessPrincipal {
            principal_id: "service:pcp-runtime-observer".to_owned(),
            principal_type: AccessPrincipalType::Service,
            display_name: Some("PCP Runtime Observer".to_owned()),
        },
        format!("pcp-runtime-observer:{generation}"),
        scopes
            .iter()
            .map(|namespace| ScopeGrant {
                namespace: namespace.clone(),
                permissions: vec![AccessPermission::Observe],
            })
            .collect(),
    )
}

fn health_metrics(health: &HealthSnapshot) -> Vec<ObserverMetric> {
    let coverage = (health.activity.calls > 0)
        .then(|| health.activity.measured_calls as f64 / health.activity.calls as f64);
    let mut metrics = vec![
        metric(
            "requests.total",
            MetricKind::Counter,
            json!(health.activity.calls),
            Some("calls"),
            Some(SNAPSHOT_WINDOW_SECONDS),
        ),
        metric(
            "requests.failed",
            MetricKind::Counter,
            json!(health.activity.failed),
            Some("calls"),
            Some(SNAPSHOT_WINDOW_SECONDS),
        ),
        metric(
            "requests.denied",
            MetricKind::Counter,
            json!(health.activity.denied),
            Some("calls"),
            Some(SNAPSHOT_WINDOW_SECONDS),
        ),
    ];
    if let Some(p95_duration_ms) = health.activity.p95_duration_ms {
        metrics.push(metric(
            "requests.latency.p95_ms",
            MetricKind::Gauge,
            Value::from(p95_duration_ms),
            Some("milliseconds"),
            Some(SNAPSHOT_WINDOW_SECONDS),
        ));
    }
    if let Some(coverage) = coverage {
        metrics.push(metric(
            "requests.telemetry_coverage_ratio",
            MetricKind::Gauge,
            Value::from(coverage),
            Some("ratio"),
            Some(SNAPSHOT_WINDOW_SECONDS),
        ));
    }
    metrics.push(metric(
        "pcp.pages.current",
        MetricKind::Gauge,
        json!(health.storage.current_pages),
        Some("pages"),
        None,
    ));
    metrics
}

fn metric(
    id: &str,
    kind: MetricKind,
    value: Value,
    unit: Option<&str>,
    window_seconds: Option<u64>,
) -> ObserverMetric {
    ObserverMetric {
        id: id.to_owned(),
        kind,
        value,
        unit: unit.map(str::to_owned),
        window_seconds,
        dimensions: None,
    }
}

fn issue(code: &str, severity: IssueSeverity, observed_at: &str) -> ObserverIssue {
    ObserverIssue {
        code: code.to_owned(),
        severity,
        subject_id: None,
        observed_at: observed_at.to_owned(),
    }
}

#[derive(Clone, Debug, Default)]
struct IntegrityObservation {
    state: IntegrityState,
    checked_at: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum IntegrityState {
    #[default]
    Pending,
    Ok,
    Failed,
}

impl IntegrityState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ok => "ok",
            Self::Failed => "failed",
        }
    }
}

fn empty_health_snapshot() -> HealthSnapshot {
    let generated = Utc::now();
    HealthSnapshot {
        generated_at: generated.to_rfc3339_opts(SecondsFormat::Millis, true),
        window_started_at: (generated - chrono::Duration::hours(SNAPSHOT_WINDOW_HOURS.into()))
            .to_rfc3339_opts(SecondsFormat::Millis, true),
        window_hours: SNAPSHOT_WINDOW_HOURS,
        storage: StorageHealth::default(),
        activity: ActivityHealth::default(),
        recall: RecallHealth::default(),
        consolidation: ConsolidationHealth::default(),
        graph: GraphHealth::default(),
        operations: Vec::new(),
        scopes: Vec::new(),
        timeline: Vec::new(),
    }
}

fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
