//! Model-facing project registration and per-write ownership binding.
use pcp_client::{PcpApi, project_scope::ProjectScopeRequest};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnsureProjectScopeParams {
    /// Stable identity from explicit project metadata or a canonical repository identity; reuse across worktrees.
    pub project_key: String,
    /// Human-readable project name. Required on first registration.
    pub display_name: Option<String>,
    pub description: Option<String>,
    /// Only for explicitly adopting a known existing project Scope already writable by this client.
    pub existing_scope: Option<String>,
}
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectScopeReply {
    pub project_key: String,
    pub scope: String,
    pub display_name: String,
    pub created: bool,
}
pub async fn ensure(
    client: &dyn PcpApi,
    p: EnsureProjectScopeParams,
) -> Result<ProjectScopeReply, ErrorData> {
    let r = client
        .ensure_project_scope(ProjectScopeRequest {
            project_key: p.project_key,
            display_name: p.display_name,
            description: p.description,
            existing_scope: p.existing_scope,
        })
        .await
        .map_err(|e| ErrorData::invalid_request(format!("ensure project Scope: {e:#}"), None))?;
    Ok(ProjectScopeReply {
        project_key: r.project_key,
        scope: r.scope,
        display_name: r.display_name,
        created: r.created,
    })
}
pub async fn write_scope(
    client: &dyn PcpApi,
    key: Option<&str>,
    explicit: Option<&str>,
) -> Result<Option<String>, ErrorData> {
    let Some(key) = key else {
        return Ok(explicit.map(str::to_owned));
    };
    // Resolve through authenticated enrollment; existing bindings work after a process restart.
    let p = ensure(
        client,
        EnsureProjectScopeParams {
            project_key: key.to_owned(),
            display_name: None,
            description: None,
            existing_scope: None,
        },
    )
    .await?;
    if explicit.is_some_and(|scope| scope != p.scope) {
        return Err(ErrorData::invalid_params(
            "projectKey does not match the requested Scope; no content was written",
            None,
        ));
    }
    Ok(Some(p.scope))
}
