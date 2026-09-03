//! Thin MCP adapters for optional Runtime staging; no direct Store fallback.
use pcp_client::{
    PcpApi,
    context_hub::{ActivityInput, ActivityQuery, CandidateInput, ContextHubRequest},
};
use pcp_core::SourceRef;
use rmcp::{
    ErrorData,
    handler::server::tool::IntoCallToolResult,
    model::{CallToolResponse, CallToolResult, ContentBlock},
};
use schemars::JsonSchema;
use serde::Deserialize;

pub struct HubReply(pub serde_json::Value);
impl IntoCallToolResult for HubReply {
    fn into_call_tool_result(self) -> Result<CallToolResponse, ErrorData> {
        Ok(CallToolResult::success(vec![ContentBlock::text(self.0.to_string())]).into())
    }
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

pub async fn submit(client: &dyn PcpApi, p: CandidateParams) -> Result<HubReply, ErrorData> {
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
    )
    .await
}
pub async fn publish(client: &dyn PcpApi, p: ActivityParams) -> Result<HubReply, ErrorData> {
    invoke(
        client,
        ContextHubRequest::PublishActivity(ActivityInput {
            scope: p.scope,
            topic_key: p.topic_key,
            summary: p.summary,
            expected_version: p.expected_version,
            ttl_hours: p.ttl_hours,
        }),
    )
    .await
}
pub async fn read(client: &dyn PcpApi, p: ActivityReadParams) -> Result<HubReply, ErrorData> {
    invoke(
        client,
        ContextHubRequest::ReadActivity(ActivityQuery {
            scopes: p.scopes,
            query: p.query,
            cursor: p.cursor,
            limit: p.limit,
            include_own: p.include_own,
        }),
    )
    .await
}
async fn invoke(client: &dyn PcpApi, request: ContextHubRequest) -> Result<HubReply, ErrorData> {
    client
        .context_hub(request)
        .await
        .map(HubReply)
        .map_err(|e| ErrorData::invalid_request(format!("{e:#}"), None))
}
