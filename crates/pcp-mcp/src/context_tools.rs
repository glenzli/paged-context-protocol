//! Thin MCP adapters for optional Runtime staging; no direct Store fallback.
use pcp_client::{
    PcpApi,
    context_hub::{ActivityInput, ActivityQuery, CandidateInput, ContextHubRequest},
};
use pcp_core::SourceRef;
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateReply {
    candidate_id: String,
    status: String,
    created: bool,
    version: u64,
    #[serde(default)]
    result: Option<CandidateOutcome>,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
struct CandidateOutcome {
    status: String,
    #[serde(default)]
    page_id: Option<String>,
    #[serde(default)]
    revision_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityWriteReply {
    card_id: String,
    version: u64,
    changed: bool,
    expires_at: String,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityReadReply {
    items: Vec<ActivityCardReply>,
    cursor: String,
    unchanged: bool,
    #[serde(default)]
    replace: bool,
    #[serde(default)]
    truncated: bool,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActivityCardReply {
    card_id: String,
    client_id: String,
    scope: String,
    topic_key: String,
    summary: String,
    version: u64,
    updated_at: String,
    expires_at: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidateParams {
    pub scope: String,
    /// Stable source-event identity. Reuse exactly on retry, never for different text.
    pub event_id: String,
    pub title: String,
    /// At most 2,000 characters. Preserve uncertainty and attribution; not save instructions.
    pub content: String,
    #[serde(default)]
    pub source_refs: Vec<SourceRef>,
    #[serde(default)]
    pub based_on_revision_ids: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivityParams {
    pub scope: String,
    /// Stable topic key within this client, not a new key per message.
    pub topic_key: String,
    /// Optional useful cross-window update, at most 180 characters. No transcript or instructions.
    pub summary: String,
    /// Version returned by the last read/write; required to change an existing card.
    #[serde(default)]
    pub expected_version: Option<u64>,
    #[serde(default)]
    pub ttl_hours: Option<u32>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivityReadParams {
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Optional literal topic terms, not a model-powered search.
    #[serde(default)]
    pub query: Option<String>,
    /// Reuse only within this conversation and query. Not a global client watermark.
    #[serde(default)]
    pub cursor: Option<String>,
    /// At most five cards. Defaults to five.
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub include_own: bool,
}

pub async fn submit(client: &dyn PcpApi, p: CandidateParams) -> Result<CandidateReply, ErrorData> {
    invoke(
        client,
        ContextHubRequest::SubmitCandidate(CandidateInput {
            scope: p.scope,
            event_id: p.event_id,
            title: p.title,
            content: p.content,
            source_refs: p.source_refs,
            based_on_revision_ids: p.based_on_revision_ids,
        }),
        "candidate submission",
    )
    .await
}
pub async fn publish(
    client: &dyn PcpApi,
    p: ActivityParams,
) -> Result<ActivityWriteReply, ErrorData> {
    invoke(
        client,
        ContextHubRequest::PublishActivity(ActivityInput {
            scope: p.scope,
            topic_key: p.topic_key,
            summary: p.summary,
            expected_version: p.expected_version,
            ttl_hours: p.ttl_hours,
        }),
        "activity publication",
    )
    .await
}
pub async fn read(
    client: &dyn PcpApi,
    p: ActivityReadParams,
) -> Result<ActivityReadReply, ErrorData> {
    invoke(
        client,
        ContextHubRequest::ReadActivity(ActivityQuery {
            scopes: p.scopes,
            query: p.query,
            cursor: p.cursor,
            limit: p.limit,
            include_own: p.include_own,
        }),
        "activity read",
    )
    .await
}
async fn invoke<T: DeserializeOwned>(
    client: &dyn PcpApi,
    request: ContextHubRequest,
    operation: &'static str,
) -> Result<T, ErrorData> {
    let value = client
        .context_hub(request)
        .await
        .map_err(|e| ErrorData::invalid_request(format!("{e:#}"), None))?;
    serde_json::from_value(value).map_err(|error| {
        ErrorData::internal_error(format!("decode PCP {operation} result: {error}"), None)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bounded_runtime_replies_have_stable_typed_shapes() {
        let candidate: CandidateReply = serde_json::from_value(json!({
            "candidateId":"cand_1", "status":"pending", "created":true, "version":1
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(candidate).unwrap()["result"],
            json!(null)
        );

        let published: ActivityWriteReply = serde_json::from_value(json!({
            "cardId":"act_1", "version":2, "changed":true,
            "expiresAt":"2026-09-06T00:00:00Z"
        }))
        .unwrap();
        assert_eq!(serde_json::to_value(published).unwrap()["changed"], true);

        let unchanged: ActivityReadReply = serde_json::from_value(json!({
            "items":[], "cursor":"cursor_1", "unchanged":true
        }))
        .unwrap();
        let stable = serde_json::to_value(unchanged).unwrap();
        assert_eq!(stable["replace"], false);
        assert_eq!(stable["truncated"], false);
    }
}
