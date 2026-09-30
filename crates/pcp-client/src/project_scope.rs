//! Optional Runtime enrollment extension, not a Store-wide administration grant.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectScopeRequest {
    /// Stable project identity, shared by branches/worktrees; not a directory basename.
    pub project_key: String,
    /// Required only when first registering this project.
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Explicit adoption of an existing project Scope already writable by this client.
    #[serde(default)]
    pub existing_scope: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectScopeResult {
    pub project_key: String,
    pub scope: String,
    pub display_name: String,
    pub created: bool,
}
