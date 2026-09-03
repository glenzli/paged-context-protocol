//! MCP transport presentation. The reusable evidence projection lives in pcp-client.
use std::str::FromStr;

use pcp_client::model_context::ModelContext;
use rmcp::{
    ErrorData,
    handler::server::tool::IntoCallToolResult,
    model::{CallToolResponse, CallToolResult, ContentBlock},
};
use schemars::JsonSchema;
use serde::Deserialize;

pub const CORE_TOOLS: &[&str] = &[
    "pcp_search_pages",
    "pcp_semantic_search",
    "pcp_read_pages",
    "pcp_capture",
    "pcp_submit_feedback",
];

/// Read-only discovery calls remain on interactive client surfaces. Long-lived MCP hosts can
/// retain an earlier catalog across a server restart, so removing these routes turns otherwise
/// valid capability and authorization checks into `tool not found` failures.
pub const DISCOVERY_TOOLS: &[&str] = &["pcp_describe", "pcp_whoami", "pcp_list_scopes"];

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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PcpMcpToolset {
    #[default]
    Core,
    Context,
    Standard,
    Maintenance,
}

impl PcpMcpToolset {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Context => "context",
            Self::Standard => "standard",
            Self::Maintenance => "maintenance",
        }
    }

    pub(crate) fn exposes(self, name: &str, context_available: bool) -> bool {
        if CONTEXT_TOOLS.contains(&name) {
            return context_available
                && matches!(self, Self::Context | Self::Standard | Self::Maintenance);
        }
        match self {
            Self::Core => CORE_TOOLS.contains(&name),
            Self::Context => CORE_TOOLS.contains(&name) || DISCOVERY_TOOLS.contains(&name),
            Self::Standard => STANDARD_TOOLS.contains(&name),
            Self::Maintenance => true,
        }
    }
}

impl FromStr for PcpMcpToolset {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "core" => Ok(Self::Core),
            "context" => Ok(Self::Context),
            "standard" => Ok(Self::Standard),
            "maintenance" => Ok(Self::Maintenance),
            other => Err(format!(
                "unsupported PCP_MCP_TOOLSET `{other}`; use core, context, standard, or maintenance"
            )),
        }
    }
}

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
