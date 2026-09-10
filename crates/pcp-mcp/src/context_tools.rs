//! Thin MCP adapters for optional Runtime staging; no direct Store fallback.
use pcp_client::{
    PcpApi,
    context_hub::{ActivityInput, ActivityQuery, CandidateInput, ContextHubRequest},
};
use pcp_core::{AccessPermission, AccessSession, SourceRef};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

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
    /// Omit when the session has exactly one Scope with ingest access; otherwise specify one.
    #[serde(default)]
    pub scope: Option<String>,
    /// Omit for a stable ID derived from this exact payload. Supply a source-event ID to distinguish identical events; reuse on retry.
    #[serde(default)]
    pub event_id: Option<String>,
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
    /// Omit when the session has exactly one Scope with ingest access; otherwise specify one.
    #[serde(default)]
    pub scope: Option<String>,
    /// Stable topic key within this client, not a new key per message.
    pub topic_key: String,
    /// Current topic goal, meaningful progress, next step or status, at most 180 characters. No transcript or instructions.
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
    /// Defaults to true for windows sharing a client identity. False excludes the whole client.
    #[serde(default)]
    pub include_own: Option<bool>,
}

pub async fn submit(client: &dyn PcpApi, p: CandidateParams) -> Result<CandidateReply, ErrorData> {
    let scope = resolve_scope(client, p.scope).await?;
    let mut input = CandidateInput {
        scope,
        event_id: String::new(),
        title: p.title,
        content: p.content,
        source_refs: p.source_refs,
        based_on_revision_ids: p.based_on_revision_ids,
    };
    input.event_id = match p.event_id {
        Some(id) => id,
        None => candidate_event_id(&input)?,
    };
    invoke(
        client,
        ContextHubRequest::SubmitCandidate(input),
        "candidate submission",
    )
    .await
}
pub async fn publish(
    client: &dyn PcpApi,
    p: ActivityParams,
) -> Result<ActivityWriteReply, ErrorData> {
    let scope = resolve_scope(client, p.scope).await?;
    invoke(
        client,
        ContextHubRequest::PublishActivity(ActivityInput {
            scope,
            topic_key: p.topic_key,
            summary: p.summary,
            expected_version: p.expected_version,
            ttl_hours: p.ttl_hours,
        }),
        "activity publication",
    )
    .await
}

async fn resolve_scope(client: &dyn PcpApi, explicit: Option<String>) -> Result<String, ErrorData> {
    if let Some(scope) = explicit {
        // Explicit destinations still go through Runtime's normal authorization checks.
        return Ok(scope);
    }
    let access = client.access_snapshot().await.map_err(|error| {
        ErrorData::invalid_request(format!("resolve staging Scope: {error:#}"), None)
    })?;
    default_scope(&access)
}

fn default_scope(access: &AccessSession) -> Result<String, ErrorData> {
    // Store-wide permission does not identify a preferred destination.
    if !access.store_permissions.contains(&AccessPermission::Ingest) {
        let scopes = access.scopes_with_permissions(&[AccessPermission::Ingest]);
        if let [scope] = scopes.as_slice() {
            return Ok(scope.clone());
        }
    }
    Err(ErrorData::invalid_params(
        "scope is required unless the live session has exactly one Scope with ingest access; use pcp_whoami to inspect grants. No state was written.",
        None,
    ))
}

fn candidate_event_id(input: &CandidateInput) -> Result<String, ErrorData> {
    // Hash exact wire values: Runtime requires byte-equivalent fields on retries.
    // A fixed domain/version keeps this identity stable across process restarts.
    let bytes = serde_json::to_vec(&(
        "pcp-mcp-candidate-v1",
        &input.scope,
        &input.title,
        &input.content,
        &input.source_refs,
        &input.based_on_revision_ids,
    ))
    .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    Ok(format!("mcp-candidate:{:x}", Sha256::digest(bytes)))
}
pub async fn read(
    client: &dyn PcpApi,
    p: ActivityReadParams,
) -> Result<ActivityReadReply, ErrorData> {
    invoke(
        client,
        ContextHubRequest::ReadActivity(p.into()),
        "activity read",
    )
    .await
}

impl From<ActivityReadParams> for ActivityQuery {
    fn from(p: ActivityReadParams) -> Self {
        Self {
            scopes: p.scopes,
            query: p.query,
            cursor: p.cursor,
            limit: p.limit,
            include_own: p.include_own.unwrap_or_else(|| Self::default().include_own),
        }
    }
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
    use pcp_core::{AccessPrincipal, AccessPrincipalType, ScopeGrant};
    use serde_json::json;

    fn session(writable: &[&str]) -> AccessSession {
        let mut grants = vec![ScopeGrant {
            namespace: "project:read-only".into(),
            permissions: vec![AccessPermission::ReadDetail],
        }];
        grants.extend(writable.iter().map(|scope| ScopeGrant {
            namespace: (*scope).into(),
            permissions: vec![AccessPermission::Ingest],
        }));
        AccessSession::new(
            AccessPrincipal {
                principal_id: "mcp:test".into(),
                principal_type: AccessPrincipalType::ModelClient,
                display_name: None,
            },
            "test-session",
            grants,
        )
    }

    #[test]
    fn staging_defaults_require_one_explicitly_granted_destination() {
        assert_eq!(default_scope(&session(&["user:one"])).unwrap(), "user:one");
        assert_eq!(
            default_scope(&session(&["user:one", "user:one"])).unwrap(),
            "user:one"
        );
        for access in [
            session(&[]),
            session(&["user:one", "project:two"]),
            session(&["user:one"]).with_store_permissions([AccessPermission::Ingest]),
        ] {
            assert!(default_scope(&access).is_err());
        }
    }

    #[test]
    fn staging_schemas_accept_minimal_and_existing_explicit_calls() {
        let candidate: CandidateParams = serde_json::from_value(json!({
            "title":"A preference", "content":"The user prefers X, provisionally."
        }))
        .unwrap();
        assert!(candidate.scope.is_none() && candidate.event_id.is_none());
        let explicit: CandidateParams = serde_json::from_value(json!({
            "scope":"user:one", "eventId":"message:7", "title":"A", "content":"B"
        }))
        .unwrap();
        assert_eq!(explicit.event_id.as_deref(), Some("message:7"));
        let activity: ActivityParams = serde_json::from_value(json!({
            "topicKey":"fix", "summary":"Testing", "expectedVersion":4
        }))
        .unwrap();
        assert!(activity.scope.is_none());
        assert_eq!(activity.expected_version, Some(4));
        for (schema, expected) in [
            (
                serde_json::to_value(schemars::schema_for!(CandidateParams)).unwrap(),
                json!(["title", "content"]),
            ),
            (
                serde_json::to_value(schemars::schema_for!(ActivityParams)).unwrap(),
                json!(["topicKey", "summary"]),
            ),
        ] {
            assert_eq!(schema["required"], expected);
        }
    }

    #[tokio::test]
    async fn adapter_dispatch_preserves_identity_evidence_versions_and_denials() {
        use pcp_client::{EmbeddedPcpClient, context_hub::ContextHubService};
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct Recorder(Mutex<Vec<ContextHubRequest>>);
        #[async_trait::async_trait]
        impl ContextHubService for Recorder {
            async fn execute(
                &self,
                _: &AccessSession,
                request: ContextHubRequest,
            ) -> anyhow::Result<serde_json::Value> {
                self.0.lock().unwrap().push(request);
                anyhow::bail!("staging disabled by operator")
            }
        }
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("pcp-staging-{nonce}"));
        let store = Arc::new(
            pcp_sqlite::SqlitePcpStore::open(root.join("context.sqlite3"))
                .await
                .unwrap(),
        );
        let recorder = Arc::new(Recorder::default());
        let client = EmbeddedPcpClient::new(store.clone(), session(&["user:one"]))
            .with_context_hub(recorder.clone());
        let payload = json!({"title":"Preference", "content":"User provisionally chose X.",
            "sourceRefs":[{"providerId":"chat", "locator":"message:7"}],
            "basedOnRevisionIds":["rev:one"]});
        for _ in 0..2 {
            let error = submit(&client, serde_json::from_value(payload.clone()).unwrap())
                .await
                .unwrap_err();
            assert!(error.message.contains("staging disabled"));
        }
        let mut explicit = payload.clone();
        explicit["scope"] = json!("project:explicit");
        explicit["eventId"] = json!("message:7");
        assert!(
            submit(&client, serde_json::from_value(explicit).unwrap())
                .await
                .is_err()
        );
        assert!(
            publish(
                &client,
                serde_json::from_value(json!({
                    "topicKey":"fix", "summary":"Testing", "expectedVersion":4
                }))
                .unwrap()
            )
            .await
            .is_err()
        );
        let ambiguous = EmbeddedPcpClient::new(store, session(&["user:one", "user:two"]))
            .with_context_hub(recorder.clone());
        assert!(
            submit(&ambiguous, serde_json::from_value(payload).unwrap())
                .await
                .is_err()
        );
        let requests = recorder.0.lock().unwrap();
        assert_eq!(
            requests.len(),
            4,
            "one dispatch per call; ambiguity must not dispatch"
        );
        let ContextHubRequest::SubmitCandidate(first) = &requests[0] else {
            panic!()
        };
        let ContextHubRequest::SubmitCandidate(retry) = &requests[1] else {
            panic!()
        };
        assert_eq!(
            serde_json::to_value(first).unwrap(),
            serde_json::to_value(retry).unwrap()
        );
        assert_eq!(first.scope, "user:one");
        assert!(first.event_id.starts_with("mcp-candidate:"));
        assert_eq!(first.source_refs[0].locator, "message:7");
        assert_eq!(first.based_on_revision_ids, ["rev:one"]);
        for field in ["scope", "content", "sourceRefs", "basedOnRevisionIds"] {
            let mut changed = serde_json::to_value(first).unwrap();
            changed[field] = match field {
                "sourceRefs" | "basedOnRevisionIds" => json!([]),
                _ => json!("changed"),
            };
            let changed: CandidateInput = serde_json::from_value(changed).unwrap();
            assert_ne!(candidate_event_id(&changed).unwrap(), first.event_id);
        }
        let ContextHubRequest::SubmitCandidate(explicit) = &requests[2] else {
            panic!()
        };
        assert_eq!(explicit.event_id, "message:7");
        assert_eq!(explicit.scope, "project:explicit");
        let ContextHubRequest::PublishActivity(activity) = &requests[3] else {
            panic!()
        };
        assert_eq!(activity.scope, "user:one");
        assert_eq!(activity.expected_version, Some(4));
        assert!(activity.ttl_hours.is_none());
        drop(requests);
        drop(client);
        drop(ambiguous);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn activity_wire_defaults_match_runtime_and_preserve_explicit_exclusion() {
        for (input, expected) in [
            (json!({}), true),
            (json!({"includeOwn":true}), true),
            (json!({"includeOwn":false}), false),
        ] {
            let runtime: ActivityQuery = serde_json::from_value(input.clone()).unwrap();
            let params: ActivityReadParams = serde_json::from_value(input).unwrap();
            let mcp: ActivityQuery = params.into();
            assert_eq!(runtime.include_own, expected);
            assert_eq!(mcp.include_own, expected);
        }
        let mcp_default: ActivityQuery = ActivityReadParams::default().into();
        assert!(mcp_default.include_own);
        assert!(ActivityQuery::default().include_own);
    }

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
