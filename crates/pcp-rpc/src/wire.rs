use anyhow::{Context, Result};
use pcp_client::{
    ContentLibraryResult, ContentLibrarySummary, DurablePageInventoryItem, HealthSnapshot,
    QueryAuditSummary, TombstoneCascadeResult, UnpackPageResult,
};
use pcp_core::{
    AccessAuditEvent, AccessSession, Actor, ApplyReconciliationRequest, ArchivePageRequest,
    AssessPageValidityRequest, BrowseIndexOrder, Capabilities, CollectRevisionRetentionRequest,
    CreateScopeRequest, ExpandGraphRequest, ExtractTopicRequest, FeedbackSignal,
    FeedbackSubmission, GraphSliceResponse, IngestPageRequest, LinkPagesRequest, PackPagesRequest,
    PageLifecycleTransitionResult, PlanRevisionRetentionRequest, PutRevisionRetentionLeaseRequest,
    ReadPage, ReadPagesRequest, ReconciliationResult, Relation, RepairPageRequest,
    RestoreArchivedPageRequest, RevisePageRequest, RevisionCollectionResult,
    RevisionRetentionLease, RevisionRetentionPlan, Scope, SearchPagesRequest, SearchResult,
    SubmitFeedbackRequest, UnpackPageRequest, WritePageRequest, WriteResult, WriteSummaryRequest,
    WriteSummaryResult, WriteValidityResult,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use pcp_core::{IntentEffort, QueryContextRequest, QueryContextResponse};

const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PcpDescriptor {
    pub identity_id: String,
    pub capabilities: Capabilities,
    pub access: AccessSession,
    #[serde(default)]
    pub server_pid: u32,
    #[serde(default)]
    pub server_started_at_unix_ms: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RpcRequest {
    pub id: u64,
    pub operation: RpcOperation,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", content = "params", rename_all = "snake_case")]
pub(crate) enum RpcOperation {
    Describe,
    IntegrityCheck,
    CreateScope(CreateScopeRequest),
    ListScopes {
        requested_scopes: Vec<String>,
        query: Option<String>,
        limit: u32,
        cursor: Option<String>,
    },
    SearchPages(SearchPagesRequest),
    ExpandGraph(ExpandGraphRequest),
    SemanticSearch(QueryContextRequest),
    MatchIntent {
        request: QueryContextRequest,
        effort: IntentEffort,
    },
    BrowseIndex {
        scopes: Vec<String>,
        excluded_page_kinds: Vec<String>,
        order: BrowseIndexOrder,
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
    },
    BrowseContentPages {
        scopes: Vec<String>,
        query: Option<String>,
        order: BrowseIndexOrder,
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
        #[serde(default)]
        filter: pcp_client::ContentLibraryFilter,
    },
    BrowseRetrievalPages {
        scopes: Vec<String>,
        query: Option<String>,
        order: BrowseIndexOrder,
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
    },
    ContentLibrarySummary {
        requested_scopes: Vec<String>,
    },
    ReadPages(ReadPagesRequest),
    CurrentRevisionId {
        page_id: String,
    },
    PageCount {
        requested_scopes: Vec<String>,
    },
    ContentCharCount {
        requested_scopes: Vec<String>,
    },
    PlanRevisionRetention(PlanRevisionRetentionRequest),
    CollectRevisionRetention(CollectRevisionRetentionRequest),
    PutRevisionRetentionLease(PutRevisionRetentionLeaseRequest),
    ActiveRevisionRetentionLeases {
        requested_scopes: Vec<String>,
        limit: u32,
    },
    IngestPage(IngestPageRequest),
    SubmitFeedback(SubmitFeedbackRequest),
    WritePage(WritePageRequest),
    RevisePage(RevisePageRequest),
    RepairPage(RepairPageRequest),
    DeletePage(pcp_core::DeletePageRequest),
    ArchivePage(ArchivePageRequest),
    RestoreArchivedPage(RestoreArchivedPageRequest),
    PackPages(PackPagesRequest),
    UnpackPage(UnpackPageRequest),
    LinkPages(LinkPagesRequest),
    WriteSummary(WriteSummaryRequest),
    ExtractTopic(ExtractTopicRequest),
    NextSummaryCandidate {
        minimum_chars: usize,
        excluded_page_kinds: Vec<String>,
    },
    MarkSummaryAssessed {
        target_revision_id: String,
        outcome: String,
        tool_or_model: Option<String>,
    },
    AssessPageValidity(AssessPageValidityRequest),
    PendingFeedback {
        requested_scopes: Vec<String>,
        limit: u32,
    },
    ApplyReconciliation(ApplyReconciliationRequest),
    TombstoneDerivationCascade {
        root_revision_id: String,
        actor: Actor,
    },
    DurablePageInventory {
        excluded_page_kinds: Vec<String>,
    },
    AccessLog {
        limit: u32,
        cursor: Option<String>,
    },
    HealthSnapshot {
        requested_scopes: Vec<String>,
        window_hours: u32,
    },
    QueryAuditSummary {
        requested_scopes: Vec<String>,
        window_hours: u32,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RpcResponse {
    pub id: u64,
    pub outcome: RpcOutcome,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
pub(crate) enum RpcOutcome {
    Ok(Box<RpcValue>),
    Error { message: String },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub(crate) enum RpcValue {
    Descriptor(PcpDescriptor),
    Integrity(String),
    Unit,
    Scopes {
        scopes: Vec<Scope>,
        next_cursor: Option<String>,
    },
    SearchResult(SearchResult),
    GraphSlice(GraphSliceResponse),
    ContextQuery(QueryContextResponse),
    ContentLibraryResult(ContentLibraryResult),
    ContentLibrarySummary(ContentLibrarySummary),
    Pages(Vec<ReadPage>),
    RevisionId(String),
    PageCount(u64),
    ContentCharCount(u64),
    RevisionRetentionPlan(RevisionRetentionPlan),
    RevisionCollectionResult(RevisionCollectionResult),
    RevisionRetentionLease(RevisionRetentionLease),
    RevisionRetentionLeases(Vec<RevisionRetentionLease>),
    WriteResult(WriteResult),
    LifecycleTransition(PageLifecycleTransitionResult),
    UnpackPageResult(UnpackPageResult),
    Relation(Relation),
    SummaryResult(WriteSummaryResult),
    TopicExtractionResult(WriteResult),
    SummaryCandidate(Option<String>),
    ValidityResult(WriteValidityResult),
    FeedbackSubmission(FeedbackSubmission),
    FeedbackSignals(Vec<FeedbackSignal>),
    ReconciliationResult(ReconciliationResult),
    TombstoneCascade(TombstoneCascadeResult),
    Inventory(Vec<DurablePageInventoryItem>),
    AccessLog {
        events: Vec<AccessAuditEvent>,
        next_cursor: Option<String>,
    },
    HealthSnapshot(HealthSnapshot),
    QueryAuditSummary(QueryAuditSummary),
}

pub(crate) async fn write_frame<T>(writer: &mut (impl AsyncWrite + Unpin), value: &T) -> Result<()>
where
    T: Serialize,
{
    let payload = serde_json::to_vec(value).context("serialize PCP RPC frame")?;
    anyhow::ensure!(
        payload.len() <= MAX_FRAME_BYTES,
        "PCP RPC frame exceeds {MAX_FRAME_BYTES} bytes"
    );
    let length = u32::try_from(payload.len()).context("encode PCP RPC frame length")?;
    writer
        .write_all(&length.to_be_bytes())
        .await
        .context("write PCP RPC frame length")?;
    writer
        .write_all(&payload)
        .await
        .context("write PCP RPC frame payload")?;
    writer.flush().await.context("flush PCP RPC frame")?;
    Ok(())
}

pub(crate) async fn read_frame<T>(reader: &mut (impl AsyncRead + Unpin)) -> Result<Option<T>>
where
    T: DeserializeOwned,
{
    let mut length = [0_u8; 4];
    match reader.read_exact(&mut length).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error).context("read PCP RPC frame length"),
    }
    let length = u32::from_be_bytes(length) as usize;
    anyhow::ensure!(
        length <= MAX_FRAME_BYTES,
        "PCP RPC frame exceeds {MAX_FRAME_BYTES} bytes"
    );
    let mut payload = vec![0_u8; length];
    reader
        .read_exact(&mut payload)
        .await
        .context("read PCP RPC frame payload")?;
    let value = serde_json::from_slice(&payload).context("decode PCP RPC frame")?;
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use pcp_core::{
        Actor, ActorType, FeedbackAuthority, FeedbackKind, PagePayload, PageRevisionRef,
        ReconciliationDisposition, RepairPageRequest,
    };

    use super::*;

    #[test]
    fn pack_pages_has_a_stable_operation_name_and_exact_input_shape() {
        let value = serde_json::to_value(RpcOperation::PackPages(PackPagesRequest {
            pages: vec![PageRevisionRef {
                page_id: "pg_anchor".to_owned(),
                revision_id: "rev_head".to_owned(),
            }],
            idempotency_key: None,
        }))
        .expect("serialize pack_pages RPC operation");

        assert_eq!(
            value,
            serde_json::json!({
                "type": "pack_pages",
                "params": {
                    "pages": [{
                        "pageId": "pg_anchor",
                        "revisionId": "rev_head"
                    }]
                }
            })
        );
    }

    #[test]
    fn repair_page_has_a_stable_operation_name_without_a_caller_supplied_actor() {
        let value = serde_json::to_value(RpcOperation::RepairPage(RepairPageRequest {
            page_id: "pg_durable".to_owned(),
            expected_revision_id: "rev_old".to_owned(),
            reason: "Restore source context".to_owned(),
            payload: Some(PagePayload {
                media_type: "text/markdown".to_owned(),
                content: "Restored context".to_owned(),
            }),
            source_refs: Vec::new(),
            facets: None,
            based_on_revision_ids: vec!["rev_evidence".to_owned()],
            tool_or_model: Some("symbiont-pcp-repair".to_owned()),
            idempotency_key: None,
        }))
        .expect("serialize repair_page RPC operation");

        assert_eq!(value["type"], "repair_page");
        assert_eq!(value["params"]["pageId"], "pg_durable");
        assert_eq!(value["params"]["expectedRevisionId"], "rev_old");
        assert_eq!(value["params"]["reason"], "Restore source context");
        assert!(value["params"].get("createdBy").is_none());
        assert!(value["params"].get("actor").is_none());
    }

    #[test]
    fn content_library_filters_keep_legacy_requests_and_roundtrip_roles() {
        let legacy = serde_json::json!({"type":"browse_content_pages","params":{
            "scopes":["user:self"],"query":null,"order":"recent","limit":20,
            "cursor":null,"max_chars":32000
        }});
        let decoded: RpcOperation = serde_json::from_value(legacy.clone()).unwrap();
        let RpcOperation::BrowseContentPages { filter, .. } = decoded else {
            panic!("wrong operation")
        };
        assert!(filter.role.is_none());
        assert!(!filter.with_summary);
        let mut filtered = legacy;
        filtered["params"]["filter"] = serde_json::json!({"role":"condensed","withSummary":true});
        let decoded: RpcOperation = serde_json::from_value(filtered).unwrap();
        let encoded = serde_json::to_value(decoded).unwrap();
        assert_eq!(encoded["params"]["filter"]["role"], "condensed");
        assert_eq!(encoded["params"]["filter"]["withSummary"], true);
        let roles = pcp_client::ContentLibraryResult {
            hits: Vec::new(),
            next_cursor: None,
            total_pages: 0,
            total_content_chars: 0,
            page_roles: std::collections::BTreeMap::from([(
                "pg_one".into(),
                pcp_client::ContentPageRole::CoveredSource,
            )]),
        };
        let encoded = serde_json::to_value(&roles).unwrap();
        assert_eq!(encoded["pageRoles"]["pg_one"], "covered_source");
        let decoded: pcp_client::ContentLibraryResult = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.page_roles, roles.page_roles);
    }

    #[test]
    fn single_page_delete_is_revision_bound_and_has_no_caller_actor() {
        let value = serde_json::to_value(RpcOperation::DeletePage(pcp_core::DeletePageRequest {
            page_id: "pg_one".into(),
            expected_revision_id: "rev_current".into(),
            reason: None,
            idempotency_key: None,
        }))
        .unwrap();
        assert_eq!(value["type"], "delete_page");
        assert_eq!(value["params"]["expectedRevisionId"], "rev_current");
        assert!(value["params"].get("actor").is_none());
        assert!(
            serde_json::from_value::<RpcOperation>(serde_json::json!({
                "type":"delete_page", "params":{"pageId":"pg_one"}
            }))
            .is_err()
        );
    }

    #[test]
    fn feedback_and_reconciliation_have_stable_exact_revision_wire_shapes() {
        let feedback = serde_json::to_value(RpcOperation::SubmitFeedback(SubmitFeedbackRequest {
            namespace: "project:one".to_owned(),
            kind: FeedbackKind::Challenge,
            authority: FeedbackAuthority::TenantAssertion,
            payload: PagePayload {
                media_type: "text/plain".to_owned(),
                content: "The recalled statement was challenged.".to_owned(),
            },
            observed_at: None,
            source_refs: Vec::new(),
            challenged_revision_ids: vec!["rev_challenged".to_owned()],
            used_revision_ids: vec!["rev_challenged".to_owned(), "rev_context".to_owned()],
            evidence_revision_ids: vec!["rev_correction".to_owned()],
            response_ref: Some("tenant:response:7".to_owned()),
            external_event_id: Some("feedback:7".to_owned()),
        }))
        .expect("serialize submit_feedback RPC operation");
        assert_eq!(feedback["type"], "submit_feedback");
        assert_eq!(
            feedback["params"]["challengedRevisionIds"],
            serde_json::json!(["rev_challenged"])
        );
        assert_eq!(
            feedback["params"]["usedRevisionIds"],
            serde_json::json!(["rev_challenged", "rev_context"])
        );
        assert_eq!(feedback["params"]["authority"], "tenant_assertion");
        assert_eq!(
            feedback["params"]["evidenceRevisionIds"],
            serde_json::json!(["rev_correction"])
        );
        let mut legacy = feedback.clone();
        legacy["params"]
            .as_object_mut()
            .unwrap()
            .remove("evidenceRevisionIds");
        let RpcOperation::SubmitFeedback(decoded) =
            serde_json::from_value::<RpcOperation>(legacy).unwrap()
        else {
            panic!("wrong feedback operation")
        };
        assert!(decoded.evidence_revision_ids.is_empty());

        let reconciliation = serde_json::to_value(RpcOperation::ApplyReconciliation(
            ApplyReconciliationRequest {
                feedback_revision_id: Some("rev_feedback".to_owned()),
                expected_assessment_revision_id: None,
                target: PageRevisionRef {
                    page_id: "pg_target".to_owned(),
                    revision_id: "rev_challenged".to_owned(),
                },
                disposition: ReconciliationDisposition::Disputed,
                rationale: "The exact claim remains contested.".to_owned(),
                scope: None,
                replacement: None,
                basis_revision_ids: vec!["rev_feedback".to_owned(), "rev_challenged".to_owned()],
                created_by: Actor {
                    actor_type: ActorType::System,
                    actor_id: "service:maintainer".to_owned(),
                },
                tool_or_model: Some("model:reconciler".to_owned()),
                idempotency_key: Some("reconcile:7".to_owned()),
            },
        ))
        .expect("serialize apply_reconciliation RPC operation");
        assert_eq!(reconciliation["type"], "apply_reconciliation");
        assert_eq!(reconciliation["params"]["target"]["pageId"], "pg_target");
        assert_eq!(reconciliation["params"]["disposition"], "disputed");
        let mut discovered = reconciliation;
        discovered["params"]
            .as_object_mut()
            .unwrap()
            .remove("feedbackRevisionId");
        let RpcOperation::ApplyReconciliation(decoded) =
            serde_json::from_value::<RpcOperation>(discovered).unwrap()
        else {
            panic!("wrong reconciliation operation")
        };
        assert!(decoded.feedback_revision_id.is_none());
    }
}
