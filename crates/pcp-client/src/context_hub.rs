//! Optional Runtime facilities, not part of the durable Page/Store protocol.
use anyhow::Result;
use async_trait::async_trait;
use pcp_core::{AccessSession, SourceRef};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClientContextPolicy {
    pub client_id: String,
    #[serde(default)]
    pub submit_candidates: bool,
    #[serde(default)]
    pub publish_activity: bool,
    #[serde(default)]
    pub read_activity: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidateInput {
    pub scope: String,
    pub event_id: String,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub source_refs: Vec<SourceRef>,
    #[serde(default)]
    pub based_on_revision_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivityInput {
    pub scope: String,
    pub topic_key: String,
    pub summary: String,
    #[serde(default)]
    pub expected_version: Option<u64>,
    /// Hours from the server's receipt time, 1..=168; omission means 48 hours.
    #[serde(default)]
    pub ttl_hours: Option<u32>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivityQuery {
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub query: Option<String>,
    /// A snapshot token local to this consumer conversation and query.
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub include_own: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidateVersion {
    pub candidate_id: String,
    pub version: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CandidateAction {
    Promote,
    Represented,
    Defer,
    Reject,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidateReview {
    pub candidates: Vec<CandidateVersion>,
    pub action: CandidateAction,
    /// Operator-reviewed content; required for promotion, including a merged group.
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    /// Exact existing Revision when the material is already represented.
    #[serde(default)]
    pub target_revision_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "operation", content = "params", rename_all = "snake_case")]
pub enum ContextHubRequest {
    SubmitCandidate(CandidateInput),
    PublishActivity(ActivityInput),
    ReadActivity(ActivityQuery),
    Inspect,
    SetPolicy(ClientContextPolicy),
    Review(CandidateReview),
    RemoveActivity { card_id: String, version: u64 },
}

/// Implemented by Runtime. The bound session, never model arguments, identifies
/// the caller. The service additionally enforces per-client opt-in and Scope ACL.
#[async_trait]
pub trait ContextHubService: Send + Sync {
    async fn execute(&self, access: &AccessSession, request: ContextHubRequest) -> Result<Value>;
}
