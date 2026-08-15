use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use async_trait::async_trait;
use infer_runtime_client::{Client, ResponsesRequest, ResponsesResult};
use serde_json::Value;
use tokio::time::{Instant, sleep, timeout};

use super::{MaintenanceWorkerRequest, MaintenanceWorkerResponse, SemanticMaintenanceWorker};

const MAX_INFER_OUTPUT_BYTES: usize = 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(250);

pub struct InferRuntimeSemanticWorker {
    client: Client,
    timeout: Duration,
    max_output_tokens: u32,
}

impl InferRuntimeSemanticWorker {
    pub fn new(
        credential_file: PathBuf,
        timeout: Duration,
        max_output_tokens: u32,
    ) -> Result<Self> {
        let client = Client::builder()
            .credential_file(credential_file)
            .build()
            .context("build Infer Runtime client for PCP maintenance")?;
        Ok(Self {
            client,
            timeout,
            max_output_tokens,
        })
    }

    async fn evaluate_inner(
        &self,
        request: &MaintenanceWorkerRequest,
    ) -> Result<MaintenanceWorkerResponse> {
        let infer_request = infer_request(request, self.timeout, self.max_output_tokens)?;
        let started = Instant::now();
        let mut response = timeout(self.timeout, self.client.create_response(&infer_request))
            .await
            .context("Infer Runtime maintenance submission timed out")?
            .context("submit PCP maintenance inference")?;

        loop {
            match response.status.as_str() {
                "completed" => return decode_response(&response),
                "queued" | "in_progress" => {}
                "failed" | "cancelled" | "incomplete" => {
                    anyhow::bail!(
                        "Infer Runtime maintenance response {} ended with status {}",
                        response.id,
                        response.status
                    );
                }
                status => anyhow::bail!(
                    "Infer Runtime maintenance response {} returned unknown status {status}",
                    response.id
                ),
            }

            let Some(remaining) = self.timeout.checked_sub(started.elapsed()) else {
                let _ = self.client.cancel_response(&response.id).await;
                anyhow::bail!("Infer Runtime maintenance response timed out");
            };
            if remaining.is_zero() {
                let _ = self.client.cancel_response(&response.id).await;
                anyhow::bail!("Infer Runtime maintenance response timed out");
            }
            sleep(POLL_INTERVAL.min(remaining)).await;
            let Some(remaining) = self.timeout.checked_sub(started.elapsed()) else {
                let _ = self.client.cancel_response(&response.id).await;
                anyhow::bail!("Infer Runtime maintenance response timed out");
            };
            response = match timeout(remaining, self.client.get_response(&response.id)).await {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => {
                    let _ = self.client.cancel_response(&response.id).await;
                    return Err(error).context("poll PCP maintenance inference");
                }
                Err(_) => {
                    let _ = self.client.cancel_response(&response.id).await;
                    anyhow::bail!("Infer Runtime maintenance response timed out");
                }
            };
        }
    }
}

#[async_trait]
impl SemanticMaintenanceWorker for InferRuntimeSemanticWorker {
    async fn evaluate(
        &self,
        request: MaintenanceWorkerRequest,
    ) -> Result<MaintenanceWorkerResponse> {
        self.evaluate_inner(&request).await
    }
}

fn infer_request(
    request: &MaintenanceWorkerRequest,
    timeout: Duration,
    max_output_tokens: u32,
) -> Result<ResponsesRequest> {
    let payload =
        serde_json::to_string(request).context("encode PCP maintenance inference input")?;
    let deadline_ms = timeout.as_millis().clamp(1, u128::from(u64::MAX));
    let metadata = BTreeMap::from([
        ("infer.priority".to_owned(), "background".to_owned()),
        ("infer.placement".to_owned(), "local_only".to_owned()),
        ("infer.prefer".to_owned(), "local".to_owned()),
        ("infer.offline_required".to_owned(), "true".to_owned()),
        ("infer.max_cost_usd".to_owned(), "0".to_owned()),
        ("infer.fallback".to_owned(), "none".to_owned()),
        ("infer.deadline_ms".to_owned(), deadline_ms.to_string()),
    ]);
    Ok(ResponsesRequest {
        model: intent_for(request).to_owned(),
        input: Value::String(payload),
        instructions: Some(Value::String(instructions_for(request).to_owned())),
        stream: false,
        background: true,
        metadata,
        tools: Vec::new(),
        reasoning: None,
        max_output_tokens: Some(max_output_tokens),
    })
}

fn intent_for(request: &MaintenanceWorkerRequest) -> &'static str {
    match request {
        MaintenanceWorkerRequest::SummarizePage { .. } => "text.summarize",
        MaintenanceWorkerRequest::SelectPacking { .. }
        | MaintenanceWorkerRequest::SelectRelation { .. }
        | MaintenanceWorkerRequest::SelectRetentionMilestones { .. } => "reasoning.solve",
    }
}

fn instructions_for(request: &MaintenanceWorkerRequest) -> &'static str {
    match request {
        MaintenanceWorkerRequest::SummarizePage { .. } => {
            "Return exactly one JSON object and no markdown. Use either {\"decision\":\"write_summary\",\"content\":\"...\"}, {\"decision\":\"no_candidate\"}, or {\"decision\":\"defer\"}. Preserve qualified claims and do not invent facts."
        }
        MaintenanceWorkerRequest::SelectPacking { .. } => {
            "Return exactly one JSON object and no markdown. Use either {\"decision\":\"candidate\",\"page_ids\":[\"pg_...\",\"pg_...\"]}, {\"decision\":\"no_candidate\"}, or {\"decision\":\"defer\"}. Select only an ordered contiguous subset supplied in the request when it is one semantically continuous topic."
        }
        MaintenanceWorkerRequest::SelectRelation { .. } => {
            "Return exactly one JSON object and no markdown. Use either {\"decision\":\"relate\",\"page_ids\":[\"pg_...\",\"pg_...\"]}, {\"decision\":\"no_candidate\"}, or {\"decision\":\"defer\"}. Select exactly two supplied Pages only for a substantive semantic connection. Temporal adjacency, shared Scope, co-retrieval, or lexical similarity alone are insufficient."
        }
        MaintenanceWorkerRequest::SelectRetentionMilestones { .. } => {
            "Return exactly one JSON object and no markdown. Use either {\"decision\":\"retain\",\"milestones\":[{\"revisionId\":\"rev_...\",\"reason\":\"...\"}]}, {\"decision\":\"no_candidate\"}, or {\"decision\":\"defer\"}. Select only supplied Revisions with durable semantic importance and stay within max_revisions."
        }
    }
}

fn decode_response(response: &ResponsesResult) -> Result<MaintenanceWorkerResponse> {
    let text = extract_output_text(response)?;
    serde_json::from_str(&text).context("decode strict PCP maintenance decision from Infer Runtime")
}

fn extract_output_text(response: &ResponsesResult) -> Result<String> {
    let mut text = String::new();
    for item in &response.output {
        if item.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let Some(content) = item.get("content").and_then(Value::as_array) else {
            continue;
        };
        for part in content {
            if part.get("type").and_then(Value::as_str) == Some("output_text")
                && let Some(value) = part.get("text").and_then(Value::as_str)
            {
                anyhow::ensure!(
                    text.len().saturating_add(value.len()) <= MAX_INFER_OUTPUT_BYTES,
                    "Infer Runtime maintenance output exceeds {MAX_INFER_OUTPUT_BYTES} bytes"
                );
                text.push_str(value);
            }
        }
    }
    anyhow::ensure!(
        !text.trim().is_empty(),
        "Infer Runtime maintenance response contains no output_text"
    );
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maintenance::{RelationCandidatePage, RetentionMilestone};

    fn result(text: &str) -> ResponsesResult {
        ResponsesResult {
            id: "resp_test".to_owned(),
            object: "response".to_owned(),
            created_at: 1,
            model: "reasoning.solve".to_owned(),
            status: "completed".to_owned(),
            output: vec![serde_json::json!({
                "type": "message",
                "content": [{"type": "output_text", "text": text}]
            })],
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn infer_request_is_fixed_to_local_background_execution() {
        let request = MaintenanceWorkerRequest::SelectRelation {
            pages: vec![RelationCandidatePage {
                page_id: "pg_1".to_owned(),
                namespace: "project:one".to_owned(),
                kind: "document".to_owned(),
                created_at: "2026-08-15T00:00:00Z".to_owned(),
                observed_at: None,
                routing_text: "bounded text".to_owned(),
                facets: None,
                relation_types: Vec::new(),
            }],
        };
        let infer = infer_request(&request, Duration::from_secs(12), 512)
            .expect("build Infer maintenance request");

        assert_eq!(infer.model, "reasoning.solve");
        assert!(infer.background);
        assert!(!infer.stream);
        assert!(infer.tools.is_empty());
        assert_eq!(infer.metadata["infer.placement"], "local_only");
        assert_eq!(infer.metadata["infer.offline_required"], "true");
        assert_eq!(infer.metadata["infer.fallback"], "none");
        assert_eq!(infer.metadata["infer.max_cost_usd"], "0");
        assert_eq!(infer.metadata["infer.deadline_ms"], "12000");
    }

    #[test]
    fn infer_output_requires_exact_json_without_markdown_fences() {
        let decoded = decode_response(&result(
            "{\"decision\":\"retain\",\"milestones\":[{\"revisionId\":\"rev_1\",\"reason\":\"durable\"}]}",
        ))
        .expect("decode strict maintenance response");
        let MaintenanceWorkerResponse::Retain { milestones } = decoded else {
            panic!("expected retain response");
        };
        assert_eq!(
            milestones[0].revision_id,
            RetentionMilestone {
                revision_id: "rev_1".to_owned(),
                reason: "durable".to_owned(),
            }
            .revision_id
        );
        assert!(decode_response(&result("```json\n{\"decision\":\"no_candidate\"}\n```")).is_err());
    }

    #[test]
    fn infer_output_rejects_unknown_decision_fields() {
        assert!(
            decode_response(&result(
                "{\"decision\":\"relate\",\"page_ids\":[\"pg_1\",\"pg_2\"],\"relation_type\":\"summarizes\"}"
            ))
            .is_err()
        );
    }
}
