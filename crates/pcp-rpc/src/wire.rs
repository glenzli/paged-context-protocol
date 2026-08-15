use anyhow::{Context, Result};
use pcp_client::{DurablePageInventoryItem, HealthSnapshot, TombstoneCascadeResult};
use pcp_core::{
    AccessAuditEvent, AccessSession, Actor, AssessPageValidityRequest, Capabilities,
    CollectRevisionRetentionRequest, CreateScopeRequest, IngestPageRequest, LinkPagesRequest,
    PackPagesRequest, PlanRevisionRetentionRequest, PutRevisionRetentionLeaseRequest, ReadPage,
    ReadPagesRequest, Relation, RevisePageRequest, RevisionCollectionResult,
    RevisionRetentionLease, RevisionRetentionPlan, Scope, SearchPagesRequest, SearchResult,
    WritePageRequest, WriteResult, WriteSummaryRequest, WriteSummaryResult, WriteValidityResult,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

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
    BrowseIndex {
        scopes: Vec<String>,
        excluded_page_kinds: Vec<String>,
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
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
    WritePage(WritePageRequest),
    RevisePage(RevisePageRequest),
    PackPages(PackPagesRequest),
    LinkPages(LinkPagesRequest),
    WriteSummary(WriteSummaryRequest),
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
    Pages(Vec<ReadPage>),
    RevisionId(String),
    PageCount(u64),
    ContentCharCount(u64),
    RevisionRetentionPlan(RevisionRetentionPlan),
    RevisionCollectionResult(RevisionCollectionResult),
    RevisionRetentionLease(RevisionRetentionLease),
    RevisionRetentionLeases(Vec<RevisionRetentionLease>),
    WriteResult(WriteResult),
    Relation(Relation),
    SummaryResult(WriteSummaryResult),
    SummaryCandidate(Option<String>),
    ValidityResult(WriteValidityResult),
    TombstoneCascade(TombstoneCascadeResult),
    Inventory(Vec<DurablePageInventoryItem>),
    AccessLog {
        events: Vec<AccessAuditEvent>,
        next_cursor: Option<String>,
    },
    HealthSnapshot(HealthSnapshot),
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
    use pcp_core::PageRevisionRef;

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
}
