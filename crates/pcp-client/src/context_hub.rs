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
    /// Optional handoff metadata. Omission preserves a stored value on update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ActivityStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_step: Option<String>,
    /// A grouping relation within this client's Scope, never merged evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_topic_key: Option<String>,
    #[serde(default)]
    pub expected_version: Option<u64>,
    /// Hours from the server's receipt time, 1..=168; omission means 48 hours.
    #[serde(default)]
    pub ttl_hours: Option<u32>,
}

/// Publisher-declared state. Missing state on a legacy card remains unknown.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityStatus {
    Discussing,
    InProgress,
    WaitingUser,
    WaitingDependency,
    Completed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivityQuery {
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub query: Option<String>,
    /// A snapshot token local to this consumer conversation and query.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Independent continuation token returned as nextCursor. Repeat the query
    /// unchanged; permission or visible snapshot changes require a fresh first page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    /// Include this client's cards, including other windows sharing its identity.
    /// Explicit false requests only other clients' cards.
    #[serde(default = "include_same_client")]
    pub include_own: bool,
}

fn include_same_client() -> bool {
    true
}

impl Default for ActivityQuery {
    fn default() -> Self {
        Self {
            scopes: Vec::new(),
            query: None,
            cursor: None,
            page_cursor: None,
            limit: None,
            include_own: include_same_client(),
        }
    }
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
    /// Native host evidence; uses the same opt-in, quota, review and wakeup path.
    SubmitExperience(crate::experience::ExperienceCandidate),
    PublishActivity(ActivityInput),
    ReadActivity(ActivityQuery),
    Inspect,
    /// Lightweight operator read views; the complete legacy Inspect is retained.
    InspectActivity,
    InspectPolicies,
    InspectInbox,
    InspectCandidate {
        candidate_id: String,
    },
    InspectSynthesis {
        synthesis_id: String,
    },
    SetPolicy(ClientContextPolicy),
    Review(CandidateReview),
    /// Queue bounded organization; no memory is approved by this operation.
    OrganizeCandidates,
    ReviewSynthesis(SynthesisReview),
    StopSynthesis(SynthesisStop),
    SetAutomaticReview {
        enabled: bool,
    },
    UndoAutomaticOutput {
        synthesis_id: String,
        version: u64,
        output_index: usize,
    },
    RemoveActivity {
        card_id: String,
        version: u64,
    },
}

/// An operator-reviewed output, with its own evidence subset.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SynthesisOutput {
    pub candidate_ids: Vec<String>,
    pub title: String,
    pub content: String,
    /// create, update, or represented
    pub action: String,
    #[serde(default)]
    pub target_revision_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SynthesisReview {
    pub synthesis_id: String,
    pub version: u64,
    pub outputs: Vec<SynthesisOutput>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SynthesisStop {
    pub synthesis_id: String,
    pub version: u64,
}

/// Implemented by Runtime. The bound session, never model arguments, identifies
/// the caller. The service additionally enforces per-client opt-in and Scope ACL.
#[async_trait]
pub trait ContextHubService: Send + Sync {
    async fn execute(&self, access: &AccessSession, request: ContextHubRequest) -> Result<Value>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn omitted_activity_metadata_keeps_the_legacy_wire_shape() {
        let legacy = json!({"scope":"a","topicKey":"work","summary":"current understanding",
            "expectedVersion":null,"ttlHours":null});
        let input: ActivityInput = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(serde_json::to_value(input).unwrap(), legacy);
        assert_eq!(
            serde_json::to_value(ActivityQuery::default()).unwrap(),
            json!({
                "scopes":[],"query":null,"cursor":null,"limit":null,"includeOwn":true
            })
        );
    }
}
