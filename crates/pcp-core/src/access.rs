use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, Deserialize, Eq, Hash, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum AccessPermission {
    ListScopes,
    Search,
    ReadSummary,
    ReadDetail,
    Write,
    Revise,
    Summarize,
    Link,
    Assess,
    Retract,
    ManageScope,
    Audit,
    Observe,
    Collect,
    DeriveAcrossScopes,
}

impl AccessPermission {
    pub const ALL: [Self; 15] = [
        Self::ListScopes,
        Self::Search,
        Self::ReadSummary,
        Self::ReadDetail,
        Self::Write,
        Self::Revise,
        Self::Summarize,
        Self::Link,
        Self::Assess,
        Self::Retract,
        Self::ManageScope,
        Self::Audit,
        Self::Observe,
        Self::Collect,
        Self::DeriveAcrossScopes,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ListScopes => "list_scopes",
            Self::Search => "search",
            Self::ReadSummary => "read_summary",
            Self::ReadDetail => "read_detail",
            Self::Write => "write",
            Self::Revise => "revise",
            Self::Summarize => "summarize",
            Self::Link => "link",
            Self::Assess => "assess",
            Self::Retract => "retract",
            Self::ManageScope => "manage_scope",
            Self::Audit => "audit",
            Self::Observe => "observe",
            Self::Collect => "collect",
            Self::DeriveAcrossScopes => "derive_across_scopes",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessPrincipalType {
    Host,
    ModelClient,
    Cli,
    Service,
}

impl AccessPrincipalType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::ModelClient => "model_client",
            Self::Cli => "cli",
            Self::Service => "service",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "host" => Some(Self::Host),
            "model_client" => Some(Self::ModelClient),
            "cli" => Some(Self::Cli),
            "service" => Some(Self::Service),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessPrincipal {
    pub principal_id: String,
    pub principal_type: AccessPrincipalType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeGrant {
    pub namespace: String,
    pub permissions: Vec<AccessPermission>,
}

impl ScopeGrant {
    pub fn allows(&self, permission: AccessPermission) -> bool {
        self.permissions.contains(&permission)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessSession {
    pub principal: AccessPrincipal,
    pub session_id: String,
    pub grants: Vec<ScopeGrant>,
}

impl AccessSession {
    pub fn new(
        principal: AccessPrincipal,
        session_id: impl Into<String>,
        grants: Vec<ScopeGrant>,
    ) -> Self {
        Self {
            principal,
            session_id: session_id.into(),
            grants,
        }
    }

    pub fn allows(&self, namespace: &str, permission: AccessPermission) -> bool {
        self.grants
            .iter()
            .any(|grant| grant.namespace == namespace && grant.allows(permission))
    }

    pub fn scopes_with_permissions(&self, permissions: &[AccessPermission]) -> Vec<String> {
        let mut scopes = self
            .grants
            .iter()
            .map(|grant| grant.namespace.clone())
            .collect::<Vec<_>>();
        scopes.sort();
        scopes.dedup();
        scopes.retain(|namespace| {
            permissions
                .iter()
                .all(|permission| self.allows(namespace, *permission))
        });
        scopes
    }

    pub fn full_control(
        principal: AccessPrincipal,
        session_id: impl Into<String>,
        namespaces: impl IntoIterator<Item = String>,
    ) -> Self {
        Self::new(
            principal,
            session_id,
            namespaces
                .into_iter()
                .map(|namespace| ScopeGrant {
                    namespace,
                    permissions: AccessPermission::ALL.to_vec(),
                })
                .collect(),
        )
    }

    pub fn read_only(
        principal: AccessPrincipal,
        session_id: impl Into<String>,
        namespaces: impl IntoIterator<Item = String>,
    ) -> Self {
        let permissions = vec![
            AccessPermission::ListScopes,
            AccessPermission::Search,
            AccessPermission::ReadSummary,
            AccessPermission::ReadDetail,
        ];
        Self::new(
            principal,
            session_id,
            namespaces
                .into_iter()
                .map(|namespace| ScopeGrant {
                    namespace,
                    permissions: permissions.clone(),
                })
                .collect(),
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessDecision {
    Allowed,
    Denied,
    Failed,
}

impl AccessDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Denied => "denied",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "allowed" => Some(Self::Allowed),
            "denied" => Some(Self::Denied),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessAuditEvent {
    pub event_id: String,
    pub occurred_at: String,
    pub principal: AccessPrincipal,
    pub session_id: String,
    pub operation: String,
    pub scopes: Vec<String>,
    pub decision: AccessDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<OperationTelemetry>,
}

/// Privacy-preserving operational measurements attached to an access event.
///
/// Query text and Page content are deliberately excluded. Counts and projection
/// names are sufficient for runtime health analysis without duplicating memory.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationTelemetry {
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projections: Vec<String>,
}
