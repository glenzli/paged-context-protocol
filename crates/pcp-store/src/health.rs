use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthSnapshot {
    pub generated_at: String,
    pub window_started_at: String,
    pub window_hours: u32,
    pub storage: StorageHealth,
    pub activity: ActivityHealth,
    pub recall: RecallHealth,
    pub consolidation: ConsolidationHealth,
    pub graph: GraphHealth,
    pub operations: Vec<OperationHealth>,
    pub scopes: Vec<ScopeHealth>,
    pub timeline: Vec<HealthTimelineBucket>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageHealth {
    pub current_pages: u64,
    pub pages: u64,
    pub revisions: u64,
    pub historical_revisions: u64,
    pub sealed_pages: u64,
    pub revisioned_pages: u64,
    pub content_chars: u64,
    pub current_pages_created: u64,
    pub long_pages: u64,
    pub summarized_long_pages: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityHealth {
    pub calls: u64,
    pub measured_calls: u64,
    pub allowed: u64,
    pub failed: u64,
    pub denied: u64,
    pub principals: u64,
    pub p50_duration_ms: Option<u64>,
    pub p95_duration_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecallHealth {
    pub searches: u64,
    pub zero_result_searches: u64,
    pub returned_pages: u64,
    pub summary_reads: u64,
    pub detail_reads: u64,
    pub pages_read: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsolidationHealth {
    pub runs: u64,
    pub input_pages: u64,
    pub net_page_reduction: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphHealth {
    pub relations: u64,
    pub isolated_current_pages: u64,
    pub average_relations_per_page: f64,
    pub relation_types: Vec<NamedCount>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationHealth {
    pub operation: String,
    pub calls: u64,
    pub measured_calls: u64,
    pub failures: u64,
    pub input_count: u64,
    pub output_count: u64,
    pub output_bytes: u64,
    pub p50_duration_ms: Option<u64>,
    pub p95_duration_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeHealth {
    pub namespace: String,
    pub current_pages: u64,
    pub pages: u64,
    pub revisions: u64,
    pub content_chars: u64,
    pub calls: u64,
    pub failures: u64,
    pub searches: u64,
    pub writes: u64,
    pub consolidations: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthTimelineBucket {
    pub bucket: String,
    pub calls: u64,
    pub searches: u64,
    pub writes: u64,
    pub failures: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedCount {
    pub name: String,
    pub count: u64,
}
