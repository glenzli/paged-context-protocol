use std::{path::PathBuf, process::Stdio, time::Duration};

use anyhow::{Context, Result};
use async_trait::async_trait;
use pcp_core::{ReadPage, SourceRef};
use pcp_store::DurablePageInventoryItem;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{io::AsyncWriteExt, process::Command, time::timeout};

const MAX_WORKER_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_WORKER_REQUEST_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaintenanceWorkerRequest {
    SummarizePage {
        page: Box<MaintenanceDetailPage>,
    },
    SelectPacking {
        pages: Vec<PackingCandidatePage>,
        excluded_candidate_sets: Vec<Vec<String>>,
    },
    SelectRelation {
        pages: Vec<RelationCandidatePage>,
    },
    SelectRetentionMilestones {
        pages: Vec<MaintenanceRoutingPage>,
        max_revisions: usize,
        lease_days: u32,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackingCandidatePage {
    pub page_id: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
    pub routing_text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationCandidatePage {
    pub page_id: String,
    pub namespace: String,
    pub kind: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
    pub routing_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facets: Option<Value>,
    #[serde(default)]
    pub relation_types: Vec<String>,
}

impl PackingCandidatePage {
    pub(crate) fn from_inventory(item: &DurablePageInventoryItem, routing_chars: usize) -> Self {
        let semantic_text = if item.media_type.as_deref() == Some(pcp_core::PACKED_PAGE_MEDIA_TYPE)
        {
            item.snippet.as_str()
        } else {
            item.summary.as_deref().unwrap_or(&item.snippet)
        };
        let routing_text = semantic_text.chars().take(routing_chars).collect();
        Self {
            page_id: item.page_id.clone(),
            created_at: item.created_at.clone(),
            observed_at: item.observed_at.clone(),
            routing_text,
        }
    }
}

impl RelationCandidatePage {
    pub(crate) fn from_inventory(item: &DurablePageInventoryItem, routing_chars: usize) -> Self {
        let routing_text = item
            .summary
            .as_deref()
            .unwrap_or(&item.snippet)
            .chars()
            .take(routing_chars)
            .collect();
        Self {
            page_id: item.page_id.clone(),
            namespace: item.namespace.clone(),
            kind: item.kind.clone(),
            created_at: item.created_at.clone(),
            observed_at: item.observed_at.clone(),
            routing_text,
            facets: item.facets.clone(),
            relation_types: item.relation_types.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaintenanceWorkerResponse {
    WriteSummary { content: String },
    Candidate { page_ids: Vec<String> },
    Relate { page_ids: [String; 2] },
    Retain { milestones: Vec<RetentionMilestone> },
    NoCandidate,
    Defer,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetentionMilestone {
    pub revision_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceRoutingPage {
    pub page_id: String,
    pub revision_id: String,
    pub namespace: String,
    pub kind: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    pub content_chars: u64,
    pub routing_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facets: Option<Value>,
    #[serde(default)]
    pub relation_types: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceDetailPage {
    pub page_id: String,
    pub revision_id: String,
    pub namespace: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facets: Option<Value>,
    #[serde(default)]
    pub source_refs: Vec<SourceRef>,
    #[serde(default)]
    pub relations: Vec<MaintenanceRelation>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceRelation {
    pub relation_type: String,
    pub from_page_id: String,
    pub to_page_id: String,
}

impl MaintenanceRoutingPage {
    pub(crate) fn from_detail(
        item: MaintenanceDetailPage,
        kind: String,
        routing_chars: usize,
    ) -> Self {
        let routing_text = item
            .summary
            .as_deref()
            .or(item.content.as_deref())
            .unwrap_or_default()
            .chars()
            .take(routing_chars)
            .collect();
        let content_chars = item
            .content
            .as_deref()
            .map(|content| content.chars().count() as u64)
            .unwrap_or_default();
        let mut relation_types = item
            .relations
            .iter()
            .map(|relation| relation.relation_type.clone())
            .collect::<Vec<_>>();
        relation_types.sort();
        relation_types.dedup();
        Self {
            page_id: item.page_id,
            revision_id: item.revision_id,
            namespace: item.namespace,
            kind,
            created_at: item.created_at,
            observed_at: item.observed_at,
            media_type: item.media_type,
            content_chars,
            routing_text,
            facets: item.facets,
            relation_types,
        }
    }
}

impl From<ReadPage> for MaintenanceDetailPage {
    fn from(page: ReadPage) -> Self {
        let revision = page.revision;
        let (media_type, content) = revision
            .payload
            .map(|payload| (Some(payload.media_type), Some(payload.content)))
            .unwrap_or_default();
        Self {
            page_id: revision.page_id,
            revision_id: revision.revision_id,
            namespace: revision.namespace,
            created_at: revision.created_at,
            observed_at: revision.observed_at,
            media_type,
            content,
            summary: page.summary.map(|summary| summary.content),
            facets: revision.facets,
            source_refs: revision.source_refs,
            relations: page
                .relations
                .into_iter()
                .map(|relation| MaintenanceRelation {
                    relation_type: relation.relation_type,
                    from_page_id: relation.from_page_id,
                    to_page_id: relation.to_page_id,
                })
                .collect(),
        }
    }
}

#[async_trait]
pub trait SemanticMaintenanceWorker: Send + Sync {
    async fn evaluate(
        &self,
        request: MaintenanceWorkerRequest,
    ) -> Result<MaintenanceWorkerResponse>;
}

pub struct CommandSemanticWorker {
    program: PathBuf,
    args: Vec<String>,
    timeout: Duration,
}

impl CommandSemanticWorker {
    pub fn new(program: PathBuf, args: Vec<String>, timeout: Duration) -> Self {
        Self {
            program,
            args,
            timeout,
        }
    }
}

#[async_trait]
impl SemanticMaintenanceWorker for CommandSemanticWorker {
    async fn evaluate(
        &self,
        request: MaintenanceWorkerRequest,
    ) -> Result<MaintenanceWorkerResponse> {
        let payload = serde_json::to_vec(&request).context("encode PCP maintenance request")?;
        anyhow::ensure!(
            payload.len() <= MAX_WORKER_REQUEST_BYTES,
            "PCP semantic maintenance worker request exceeds {MAX_WORKER_REQUEST_BYTES} bytes"
        );
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| {
                format!(
                    "start PCP semantic maintenance worker {}",
                    self.program.display()
                )
            })?;
        let mut stdin = child
            .stdin
            .take()
            .context("open PCP maintenance worker stdin")?;
        let interaction = async move {
            stdin
                .write_all(&payload)
                .await
                .context("write PCP maintenance worker request")?;
            stdin
                .flush()
                .await
                .context("flush PCP maintenance worker stdin")?;
            drop(stdin);
            child
                .wait_with_output()
                .await
                .context("wait for PCP maintenance worker")
        };
        let output = timeout(self.timeout, interaction)
            .await
            .context("PCP semantic maintenance worker timed out")??;
        anyhow::ensure!(
            output.status.success(),
            "PCP semantic maintenance worker failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        anyhow::ensure!(
            output.stdout.len() <= MAX_WORKER_RESPONSE_BYTES,
            "PCP semantic maintenance worker response exceeds {MAX_WORKER_RESPONSE_BYTES} bytes"
        );
        serde_json::from_slice(&output.stdout).context("decode PCP maintenance worker response")
    }
}
