use pcp_core::Capabilities;
use pcp_store::HealthSnapshot;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DISCOVERY_REGISTRATION_SCHEMA: &str = "infra.discovery.registration";
pub const DISCOVERY_SCHEMA_VERSION: &str = "20260812.1";
pub const LOCAL_UNIX_SOCKET_BINDING: &str = "infra.local.unix-socket";

pub const PCP_OBSERVER_PROTOCOL_ID: &str = "pcp.runtime.observer";
pub const PCP_OBSERVER_PROTOCOL_VERSION: &str = "20260810.1";
pub const REQUEST_SCHEMA: &str = "pcp.runtime.observer.request";
pub const SNAPSHOT_SCHEMA: &str = "pcp.runtime.observer.snapshot";
pub const ERROR_SCHEMA: &str = "pcp.runtime.observer.error";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DiscoveryRegistration {
    pub schema: String,
    pub schema_version: String,
    pub service: DiscoveryService,
    pub offers: Vec<DiscoveryOffer>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DiscoveryService {
    pub kind: String,
    pub instance_id: String,
    pub generation: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DiscoveryOffer {
    pub protocol: String,
    pub protocol_versions: Vec<String>,
    pub binding: String,
    pub endpoint: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotRequest {
    pub schema: String,
    pub schema_version: String,
    pub operation: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SnapshotEnvelope {
    pub schema: String,
    pub schema_version: String,
    pub service: SnapshotService,
    pub sequence: u64,
    pub captured_at: String,
    pub status: ObserverStatus,
    pub headline_metrics: Vec<String>,
    pub metrics: Vec<ObserverMetric>,
    pub issues: Vec<ObserverIssue>,
    pub links: ObserverLinks,
    pub extensions: ObserverExtensions,
    pub redaction: ObserverRedaction,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SnapshotService {
    pub kind: String,
    pub instance_id: String,
    pub generation: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ObserverStatus {
    pub state: ObserverState,
    pub reason_codes: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObserverState {
    Starting,
    Healthy,
    Degraded,
    Unavailable,
    Stopping,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ObserverMetric {
    pub id: String,
    pub kind: MetricKind,
    pub value: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    Gauge,
    Counter,
    State,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ObserverIssue {
    pub code: String,
    pub severity: IssueSeverity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<String>,
    pub observed_at: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ObserverLinks {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub console_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ObserverExtensions {
    pub pcp: PcpExtension,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PcpExtension {
    pub protocol_version: String,
    pub capabilities: Capabilities,
    pub integrity: PcpIntegrity,
    pub scope_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<HealthSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PcpIntegrity {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ObserverRedaction {
    pub excluded: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ObserverError {
    pub schema: String,
    pub schema_version: String,
    pub code: String,
    pub message: String,
}
