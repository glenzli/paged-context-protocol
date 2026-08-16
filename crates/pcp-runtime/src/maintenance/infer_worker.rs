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
    summary_deployment_id: String,
    reasoning_deployment_id: String,
    relation_deployment_id: Option<String>,
}

impl InferRuntimeSemanticWorker {
    pub fn new(
        credential_file: PathBuf,
        timeout: Duration,
        summary_deployment_id: String,
        reasoning_deployment_id: String,
        relation_deployment_id: Option<String>,
    ) -> Result<Self> {
        let client = Client::builder()
            .credential_file(credential_file)
            .build()
            .context("build Infer Runtime client for PCP maintenance")?;
        Ok(Self {
            client,
            timeout,
            summary_deployment_id,
            reasoning_deployment_id,
            relation_deployment_id,
        })
    }

    async fn evaluate_inner(
        &self,
        request: &MaintenanceWorkerRequest,
    ) -> Result<MaintenanceWorkerResponse> {
        let infer_request = infer_request(
            request,
            self.timeout,
            &self.summary_deployment_id,
            &self.reasoning_deployment_id,
            self.relation_deployment_id.as_deref(),
        )?;
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
    summary_deployment_id: &str,
    reasoning_deployment_id: &str,
    relation_deployment_id: Option<&str>,
) -> Result<ResponsesRequest> {
    let payload =
        serde_json::to_string(request).context("encode PCP maintenance inference input")?;
    let deadline_ms = timeout.as_millis().clamp(1, u128::from(u64::MAX));
    let mut metadata = BTreeMap::from([
        ("infer.priority".to_owned(), "background".to_owned()),
        ("infer.max_cost_usd".to_owned(), "0".to_owned()),
        ("infer.fallback".to_owned(), "none".to_owned()),
        ("infer.deadline_ms".to_owned(), deadline_ms.to_string()),
    ]);
    let reasoning = match request {
        MaintenanceWorkerRequest::SummarizePage { .. } => {
            metadata.insert(
                "infer.deployment_ids".to_owned(),
                summary_deployment_id.to_owned(),
            );
            metadata.insert(
                "infer.capability_floor".to_owned(),
                "foundational".to_owned(),
            );
            None
        }
        MaintenanceWorkerRequest::SelectPacking { .. }
        | MaintenanceWorkerRequest::AnalyzePacking { .. }
        | MaintenanceWorkerRequest::SelectRelation { .. }
        | MaintenanceWorkerRequest::SelectRetentionMilestones { .. } => {
            let deployment_id = match request {
                MaintenanceWorkerRequest::SelectRelation { .. } => {
                    relation_deployment_id.unwrap_or(reasoning_deployment_id)
                }
                _ => reasoning_deployment_id,
            };
            metadata.insert("infer.deployment_ids".to_owned(), deployment_id.to_owned());
            metadata.insert("infer.placement".to_owned(), "cloud_only".to_owned());
            metadata.insert("infer.prefer".to_owned(), "cloud".to_owned());
            metadata.insert(
                "infer.provider_access_class".to_owned(),
                "subscription".to_owned(),
            );
            metadata.insert("infer.capability_floor".to_owned(), "advanced".to_owned());
            Some(serde_json::json!({"effort": "medium"}))
        }
    };
    Ok(ResponsesRequest {
        model: intent_for(request).to_owned(),
        input: Value::String(payload),
        instructions: Some(Value::String(instructions_for(request).to_owned())),
        stream: false,
        background: false,
        metadata,
        tools: Vec::new(),
        reasoning,
        max_output_tokens: None,
    })
}

fn intent_for(request: &MaintenanceWorkerRequest) -> &'static str {
    match request {
        MaintenanceWorkerRequest::SummarizePage { .. } => "language.respond",
        MaintenanceWorkerRequest::SelectPacking { .. }
        | MaintenanceWorkerRequest::AnalyzePacking { .. }
        | MaintenanceWorkerRequest::SelectRelation { .. }
        | MaintenanceWorkerRequest::SelectRetentionMilestones { .. } => "reasoning.solve",
    }
}

fn instructions_for(request: &MaintenanceWorkerRequest) -> &'static str {
    match request {
        MaintenanceWorkerRequest::SummarizePage { .. } => {
            "Return exactly one JSON object and no markdown. Use either {\"decision\":\"write_summary\",\"content\":\"...\"}, {\"decision\":\"no_candidate\"}, or {\"decision\":\"defer\"}. Write a concise routing summary of 1-800 Unicode characters. Preserve qualified claims and do not invent facts."
        }
        MaintenanceWorkerRequest::SelectPacking { .. } => {
            "Return exactly one JSON object and no markdown. Use either {\"decision\":\"candidate\",\"page_ids\":[\"pg_...\",\"pg_...\"]}, {\"decision\":\"no_candidate\"}, or {\"decision\":\"defer\"}. Select a maximal useful ordered contiguous segment for lossless physical packing. Pages need not state the same fact: keep a coherent local episode together, including its question, answer, correction, qualification, or short reasoning transition. Split only at a clear independent subject or event boundary. A candidate may include at most one Page marked packed=true."
        }
        MaintenanceWorkerRequest::AnalyzePacking { .. } => {
            "Return exactly one JSON object and no markdown. Use either {\"decision\":\"packing_candidates\",\"candidates\":[[\"pg_...\",\"pg_...\"]]}, {\"decision\":\"no_candidate\"}, or {\"decision\":\"defer\"}. Analyze every supplied group and return every maximal useful ordered, contiguous, non-overlapping segment for lossless physical packing, with at least two Pages and no more than max_pages_per_candidate. Pages need not state the same fact: keep coherent local episodes together, including questions, answers, corrections, qualifications, and short reasoning transitions. Split only at a clear independent subject or event boundary; temporal adjacency alone is not enough, but do not require semantic equivalence. Never combine Pages from different groups, and include at most one Page marked packed=true in each candidate."
        }
        MaintenanceWorkerRequest::SelectRelation { .. } => {
            "Return exactly one JSON object and no markdown. Use either {\"decision\":\"relate\",\"page_ids\":[\"pg_...\",\"pg_...\"]}, {\"decision\":\"no_candidate\"}, or {\"decision\":\"defer\"}. Select exactly two supplied Pages only when each directly helps understand, verify, or act on the same stable subject or evidence chain, and never select a pair listed in excluded_page_pairs. Temporal adjacency, shared Scope, co-retrieval, lexical similarity, broad analogy, or merely both discussing AI, tools, infrastructure, harnesses, runtimes, or workspaces are insufficient. Return no_candidate when no pair meets this bar."
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
    use crate::maintenance::{MaintenanceDetailPage, RelationCandidatePage, RetentionMilestone};

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
    fn reasoning_request_is_fixed_to_named_luna_without_fallback() {
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
            excluded_page_pairs: Vec::new(),
        };
        let infer = infer_request(
            &request,
            Duration::from_secs(12),
            "ollama_qwen3_5_4b",
            "codex_gpt_5_6_luna",
            None,
        )
        .expect("build Infer maintenance request");

        assert_eq!(infer.model, "reasoning.solve");
        assert!(!infer.background);
        assert!(!infer.stream);
        assert!(infer.tools.is_empty());
        assert_eq!(infer.metadata["infer.deployment_ids"], "codex_gpt_5_6_luna");
        assert_eq!(infer.metadata["infer.placement"], "cloud_only");
        assert_eq!(infer.metadata["infer.prefer"], "cloud");
        assert_eq!(
            infer.metadata["infer.provider_access_class"],
            "subscription"
        );
        assert_eq!(infer.metadata["infer.capability_floor"], "advanced");
        assert!(!infer.metadata.contains_key("infer.offline_required"));
        assert_eq!(infer.metadata["infer.fallback"], "none");
        assert_eq!(infer.metadata["infer.max_cost_usd"], "0");
        assert_eq!(infer.metadata["infer.deadline_ms"], "12000");
        assert_eq!(
            infer.reasoning,
            Some(serde_json::json!({"effort": "medium"}))
        );
        assert_eq!(infer.max_output_tokens, None);
    }

    #[test]
    fn summary_request_uses_prompt_bound_without_provider_token_constraint() {
        let request = MaintenanceWorkerRequest::SummarizePage {
            page: Box::new(MaintenanceDetailPage {
                page_id: "pg_1".to_owned(),
                revision_id: "rev_1".to_owned(),
                namespace: "project:one".to_owned(),
                created_at: "2026-08-15T00:00:00Z".to_owned(),
                observed_at: None,
                media_type: Some("text/markdown".to_owned()),
                content: Some("bounded text".to_owned()),
                summary: None,
                facets: None,
                source_refs: Vec::new(),
                relations: Vec::new(),
            }),
        };
        let infer = infer_request(
            &request,
            Duration::from_secs(12),
            "ollama_qwen3_5_4b",
            "codex_gpt_5_6_luna",
            None,
        )
        .expect("build Infer summary request");

        assert_eq!(infer.model, "language.respond");
        assert_eq!(infer.metadata["infer.deployment_ids"], "ollama_qwen3_5_4b");
        assert!(!infer.metadata.contains_key("infer.placement"));
        assert!(!infer.metadata.contains_key("infer.prefer"));
        assert!(!infer.metadata.contains_key("infer.offline_required"));
        assert!(!infer.metadata.contains_key("infer.provider_access_class"));
        assert_eq!(infer.metadata["infer.capability_floor"], "foundational");
        assert_eq!(infer.metadata["infer.fallback"], "none");
        assert_eq!(infer.reasoning, None);
        assert_eq!(infer.max_output_tokens, None);
        assert!(
            infer
                .instructions
                .as_ref()
                .and_then(Value::as_str)
                .is_some_and(|instructions| instructions.contains("1-800 Unicode characters"))
        );
    }

    #[test]
    fn relation_request_can_use_an_explicit_terra_escalation() {
        let request = MaintenanceWorkerRequest::SelectRelation {
            pages: Vec::new(),
            excluded_page_pairs: Vec::new(),
        };
        let infer = infer_request(
            &request,
            Duration::from_secs(12),
            "ollama_qwen3_5_4b",
            "codex_gpt_5_6_luna",
            Some("codex_gpt_5_6_terra"),
        )
        .expect("build Infer relation escalation request");

        assert_eq!(
            infer.metadata["infer.deployment_ids"],
            "codex_gpt_5_6_terra"
        );
        assert_eq!(infer.metadata["infer.fallback"], "none");
        assert_eq!(
            infer.reasoning,
            Some(serde_json::json!({"effort": "medium"}))
        );
        assert_eq!(infer.max_output_tokens, None);
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
