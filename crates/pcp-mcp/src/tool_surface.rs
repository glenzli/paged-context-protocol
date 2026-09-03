//! MCP transport presentation. The reusable evidence projection lives in pcp-client.
use pcp_client::model_context::ModelContext;
use rmcp::{
    ErrorData,
    handler::server::tool::IntoCallToolResult,
    model::{CallToolResponse, CallToolResult, ContentBlock},
};
use schemars::JsonSchema;
use serde::Deserialize;

pub const STANDARD_TOOLS: &[&str] = &[
    "pcp_describe",
    "pcp_whoami",
    "pcp_list_scopes",
    "pcp_search_pages",
    "pcp_semantic_search",
    "pcp_match_intent",
    "pcp_expand_graph",
    "pcp_browse_index",
    "pcp_read_pages",
    "pcp_capture",
    "pcp_submit_feedback",
];

pub const CONTEXT_TOOLS: &[&str] = &[
    "pcp_submit_candidate",
    "pcp_publish_activity",
    "pcp_read_activity",
];

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFormat {
    #[default]
    Json,
    Text,
}

pub struct ModelReply(pub ModelContext, pub ResponseFormat);

impl IntoCallToolResult for ModelReply {
    fn into_call_tool_result(self) -> Result<CallToolResponse, ErrorData> {
        let text = match self.1 {
            ResponseFormat::Text => self.0.to_text(),
            ResponseFormat::Json => serde_json::to_string(&self.0)
                .map_err(|_| ErrorData::internal_error("serialize PCP context", None))?,
        };
        // One content block: do not mirror a large body in structuredContent.
        // JSON text works with clients that do not support structured results.
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]).into())
    }
}
