use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::Duration,
};

use anyhow::{Context, Result};
use async_trait::async_trait;
use infer_runtime_client::{Client, ResponsesRequest, ResponsesResult};
use pcp_core::{ModelTokenUsage, PACKED_PAGE_MEDIA_TYPE};
use serde_json::{Value, json};
use tokio::time::{Instant, sleep, timeout};

use super::{
    MaintenanceWorkerOutcome, MaintenanceWorkerRequest, MaintenanceWorkerResponse,
    SemanticMaintenanceWorker, worker::ArchiveWorkerDecision,
};

const MAX_INFER_OUTPUT_BYTES: usize = 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const PACKING_OVERLAP_REPAIR_INSTRUCTIONS: &str = "The previous answer was rejected because a Page appeared in more than one packing candidate. Re-evaluate the supplied groups from scratch. The candidates array must be a disjoint partition: every page_id may occur in zero or one candidate only. Before responding, verify that no identifier repeats anywhere in candidates. Omit an ambiguous segment rather than returning overlapping alternatives.";
const CHINESE_SUMMARY_REPAIR_INSTRUCTIONS: &str = "上一版摘要未满足中文输出合同。只输出替换后的摘要正文：自然语言叙述必须使用中文；技术名、产品名、模型名、版本号、URL、代码标识符和原文引号可按原样保留。不要解释，不要 JSON。";
const ENGLISH_SUMMARY_REPAIR_INSTRUCTIONS: &str = "The previous summary did not meet the English output contract. Return only a replacement summary in English prose; preserve technical names, product names, model names, versions, URLs, code identifiers, and source quotations exactly. Do not explain and do not return JSON.";
const CHINESE_RELATION_REPAIR_INSTRUCTIONS: &str = "上一版关联理由未满足中文输出合同。重新判断同一批候选，只返回规定的 JSON。若 decision 为 relate，reason 的自然语言叙述必须使用中文；技术名、产品名、模型名、版本号、URL、代码标识符和原文引号可按原样保留。不要把中文页面整体翻译成英语。";
const ENGLISH_RELATION_REPAIR_INSTRUCTIONS: &str = "The previous relation rationale did not meet the English output contract. Re-evaluate the same candidates and return only the required JSON. With decision=relate, write reason in English prose while preserving technical names, product names, versions, URLs, code identifiers, and source quotations exactly.";
const ESCALATION_INSTRUCTIONS: &str = "The inexpensive baseline maintenance model explicitly deferred this decision. Independently re-evaluate only the supplied evidence with deeper reasoning. Do not assume the baseline had a preferred answer, do not invent missing evidence, and return defer again when the supplied evidence is genuinely insufficient.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SummaryLanguage {
    Chinese,
    English,
    Unspecified,
}

pub struct InferRuntimeSemanticWorker {
    client: Client,
    timeout: Duration,
    summary_deployment_id: String,
    reasoning_deployment_id: String,
    relation_deployment_id: Option<String>,
    escalation_deployment_id: Option<String>,
    escalation_operations: BTreeSet<String>,
}

impl InferRuntimeSemanticWorker {
    pub fn new(
        credential_file: PathBuf,
        timeout: Duration,
        summary_deployment_id: String,
        reasoning_deployment_id: String,
        relation_deployment_id: Option<String>,
        escalation_deployment_id: Option<String>,
        escalation_operations: Vec<String>,
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
            escalation_deployment_id,
            escalation_operations: escalation_operations.into_iter().collect(),
        })
    }

    async fn evaluate_inner(
        &self,
        request: &MaintenanceWorkerRequest,
        additional_instructions: Option<&str>,
        deployment_override: Option<&str>,
    ) -> Result<MaintenanceWorkerOutcome> {
        let mut infer_request = infer_request(
            request,
            self.timeout,
            &self.summary_deployment_id,
            &self.reasoning_deployment_id,
            self.relation_deployment_id.as_deref(),
            deployment_override,
        )?;
        if let Some(additional_instructions) = additional_instructions {
            let instructions = infer_request
                .instructions
                .as_ref()
                .and_then(Value::as_str)
                .context("Infer Runtime maintenance request has no text instructions")?;
            infer_request.instructions = Some(Value::String(format!(
                "{instructions}\n\n{additional_instructions}"
            )));
        }
        let started = Instant::now();
        let mut response = timeout(self.timeout, self.client.create_response(&infer_request))
            .await
            .context("Infer Runtime maintenance submission timed out")?
            .context("submit PCP maintenance inference")?;

        loop {
            match response.status.as_str() {
                "completed" => {
                    return Ok(MaintenanceWorkerOutcome {
                        response: decode_response(&response, request)?,
                        usage: Some(response_usage(&response)),
                        model_attempts: 1,
                        escalated: false,
                    });
                }
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
        Ok(self.evaluate_with_usage(request).await?.response)
    }

    async fn evaluate_with_usage(
        &self,
        request: MaintenanceWorkerRequest,
    ) -> Result<MaintenanceWorkerOutcome> {
        let initial = self.evaluate_inner(&request, None, None).await?;
        let Some(repair_instructions) =
            output_language_repair_instructions(&request, &initial.response)
        else {
            return self.maybe_escalate(&request, initial).await;
        };

        let repaired = self
            .evaluate_inner(&request, Some(repair_instructions), None)
            .await?;
        let mut usage = initial.usage.unwrap_or_default();
        if let Some(repaired_usage) = repaired.usage.as_ref() {
            usage.add_assign(repaired_usage);
        }
        if output_language_repair_instructions(&request, &repaired.response).is_none() {
            Ok(MaintenanceWorkerOutcome {
                response: repaired.response,
                usage: Some(usage),
                model_attempts: 2,
                escalated: false,
            })
        } else {
            // Wrong-language maintenance evidence is worse than an absent proposal: it makes
            // human review less reliable and can poison later routing surfaces.
            Ok(MaintenanceWorkerOutcome {
                response: MaintenanceWorkerResponse::Defer,
                usage: Some(usage),
                model_attempts: 2,
                escalated: false,
            })
        }
    }

    async fn repair_packing_analysis_overlap(
        &self,
        request: MaintenanceWorkerRequest,
    ) -> Result<MaintenanceWorkerResponse> {
        Ok(self
            .repair_packing_analysis_overlap_with_usage(request)
            .await?
            .response)
    }

    async fn repair_packing_analysis_overlap_with_usage(
        &self,
        request: MaintenanceWorkerRequest,
    ) -> Result<MaintenanceWorkerOutcome> {
        self.evaluate_inner(&request, Some(PACKING_OVERLAP_REPAIR_INSTRUCTIONS), None)
            .await
    }
}

impl InferRuntimeSemanticWorker {
    async fn maybe_escalate(
        &self,
        request: &MaintenanceWorkerRequest,
        mut baseline: MaintenanceWorkerOutcome,
    ) -> Result<MaintenanceWorkerOutcome> {
        let operation = operation_name(request);
        let Some(deployment_id) = self.escalation_deployment_id.as_deref() else {
            return Ok(baseline);
        };
        if !self.escalation_operations.contains(operation) || !response_defers(&baseline.response) {
            return Ok(baseline);
        }
        let mut escalated = self
            .evaluate_inner(request, Some(ESCALATION_INSTRUCTIONS), Some(deployment_id))
            .await?;
        if let (Some(total), Some(extra)) = (&mut baseline.usage, &escalated.usage) {
            total.add_assign(extra);
        } else if baseline.usage.is_none() {
            baseline.usage = escalated.usage.take();
        }
        baseline.response = escalated.response;
        baseline.model_attempts = baseline
            .model_attempts
            .saturating_add(escalated.model_attempts);
        baseline.escalated = true;
        Ok(baseline)
    }
}

fn response_defers(response: &MaintenanceWorkerResponse) -> bool {
    matches!(
        response,
        MaintenanceWorkerResponse::Defer
            | MaintenanceWorkerResponse::ArchiveReview {
                outcome: ArchiveWorkerDecision::Defer,
                ..
            }
    )
}

fn operation_name(request: &MaintenanceWorkerRequest) -> &'static str {
    match request {
        MaintenanceWorkerRequest::SummarizePage { .. } => "summarize_page",
        MaintenanceWorkerRequest::SummarizePages { .. } => "summarize_pages",
        MaintenanceWorkerRequest::SelectPacking { .. } => "select_packing",
        MaintenanceWorkerRequest::AnalyzePacking { .. } => "analyze_packing",
        MaintenanceWorkerRequest::SelectRelation { .. } => "select_relation",
        MaintenanceWorkerRequest::ExtractTopic { .. } => "extract_topic",
        MaintenanceWorkerRequest::AssessArchive { .. } => "assess_archive",
        MaintenanceWorkerRequest::ReconcileFeedback { .. } => "reconcile_feedback",
        MaintenanceWorkerRequest::ReviewUpdate { .. } => "review_update",
        MaintenanceWorkerRequest::SelectRetentionMilestones { .. } => "select_retention_milestones",
    }
}

fn response_usage(response: &ResponsesResult) -> ModelTokenUsage {
    let Some(reported) = response.extra.get("usage").and_then(Value::as_object) else {
        return ModelTokenUsage {
            unreported_responses: 1,
            ..ModelTokenUsage::default()
        };
    };
    let (Some(input_tokens), Some(output_tokens)) = (
        reported.get("input_tokens").and_then(Value::as_u64),
        reported.get("output_tokens").and_then(Value::as_u64),
    ) else {
        return ModelTokenUsage {
            unreported_responses: 1,
            ..ModelTokenUsage::default()
        };
    };
    ModelTokenUsage {
        reported_responses: 1,
        input_tokens,
        output_tokens,
        total_tokens: reported
            .get("total_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| input_tokens.saturating_add(output_tokens)),
        cached_input_tokens: reported
            .get("input_tokens_details")
            .and_then(Value::as_object)
            .and_then(|details| details.get("cached_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        reasoning_tokens: reported
            .get("output_tokens_details")
            .and_then(Value::as_object)
            .and_then(|details| details.get("reasoning_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        unreported_responses: 0,
    }
}

fn infer_request(
    request: &MaintenanceWorkerRequest,
    timeout: Duration,
    summary_deployment_id: &str,
    reasoning_deployment_id: &str,
    relation_deployment_id: Option<&str>,
    deployment_override: Option<&str>,
) -> Result<ResponsesRequest> {
    let payload = inference_payload(request)?;
    let deadline_ms = timeout.as_millis().clamp(1, u128::from(u64::MAX));
    let mut metadata = BTreeMap::from([
        ("infer.priority".to_owned(), "background".to_owned()),
        ("infer.max_cost_usd".to_owned(), "0".to_owned()),
        ("infer.fallback".to_owned(), "none".to_owned()),
        ("infer.deadline_ms".to_owned(), deadline_ms.to_string()),
    ]);
    let reasoning = match request {
        MaintenanceWorkerRequest::SummarizePage { .. }
        | MaintenanceWorkerRequest::SummarizePages { .. } => {
            metadata.insert(
                "infer.deployment_ids".to_owned(),
                summary_deployment_id.to_owned(),
            );
            metadata.insert("infer.placement".to_owned(), "cloud_only".to_owned());
            metadata.insert("infer.prefer".to_owned(), "cloud".to_owned());
            metadata.insert(
                "infer.provider_access_class".to_owned(),
                "subscription".to_owned(),
            );
            metadata.insert("infer.capability_floor".to_owned(), "advanced".to_owned());
            Some(serde_json::json!({"effort": "medium"}))
        }
        MaintenanceWorkerRequest::SelectPacking { .. }
        | MaintenanceWorkerRequest::AnalyzePacking { .. }
        | MaintenanceWorkerRequest::ExtractTopic { .. }
        | MaintenanceWorkerRequest::AssessArchive { .. }
        | MaintenanceWorkerRequest::ReconcileFeedback { .. }
        | MaintenanceWorkerRequest::ReviewUpdate { .. }
        | MaintenanceWorkerRequest::SelectRelation { .. }
        | MaintenanceWorkerRequest::SelectRetentionMilestones { .. } => {
            let deployment_id = deployment_override.unwrap_or_else(|| match request {
                MaintenanceWorkerRequest::SelectRelation { .. } => {
                    relation_deployment_id.unwrap_or(reasoning_deployment_id)
                }
                _ => reasoning_deployment_id,
            });
            metadata.insert("infer.deployment_ids".to_owned(), deployment_id.to_owned());
            metadata.insert("infer.placement".to_owned(), "cloud_only".to_owned());
            metadata.insert("infer.prefer".to_owned(), "cloud".to_owned());
            metadata.insert(
                "infer.provider_access_class".to_owned(),
                "subscription".to_owned(),
            );
            metadata.insert("infer.capability_floor".to_owned(), "advanced".to_owned());
            let effort = if matches!(
                request,
                MaintenanceWorkerRequest::SelectRelation { .. }
                    | MaintenanceWorkerRequest::ReconcileFeedback { .. }
                    | MaintenanceWorkerRequest::ReviewUpdate { .. }
            ) {
                "high"
            } else {
                "medium"
            };
            Some(serde_json::json!({"effort": effort}))
        }
    };
    Ok(ResponsesRequest {
        model: intent_for(request).to_owned(),
        input: Value::String(payload),
        instructions: Some(Value::String(instructions_for(request))),
        stream: false,
        background: false,
        metadata,
        tools: Vec::new(),
        reasoning,
        // The Codex App Server bridge deliberately has no output-budget control.
        // Summary length is constrained by the prompt and PCP's response validation.
        max_output_tokens: None,
    })
}

fn inference_payload(request: &MaintenanceWorkerRequest) -> Result<String> {
    let value = match request {
        MaintenanceWorkerRequest::SummarizePage { page } => json!({
            "pageId": page.page_id,
            "revisionId": page.revision_id,
            "content": summary_source_text(page),
        }),
        _ => serde_json::to_value(request).context("encode PCP maintenance inference input")?,
    };
    serde_json::to_string(&value).context("serialize PCP maintenance inference input")
}

fn intent_for(request: &MaintenanceWorkerRequest) -> &'static str {
    match request {
        MaintenanceWorkerRequest::SummarizePage { .. }
        | MaintenanceWorkerRequest::SummarizePages { .. } => "language.respond",
        MaintenanceWorkerRequest::SelectPacking { .. }
        | MaintenanceWorkerRequest::AnalyzePacking { .. }
        | MaintenanceWorkerRequest::ExtractTopic { .. }
        | MaintenanceWorkerRequest::AssessArchive { .. }
        | MaintenanceWorkerRequest::ReconcileFeedback { .. }
        | MaintenanceWorkerRequest::ReviewUpdate { .. }
        | MaintenanceWorkerRequest::SelectRelation { .. }
        | MaintenanceWorkerRequest::SelectRetentionMilestones { .. } => "reasoning.solve",
    }
}

fn instructions_for(request: &MaintenanceWorkerRequest) -> String {
    match request {
        MaintenanceWorkerRequest::ReviewUpdate { target, evidence } => {
            let source = format!("{}\n{}", target.content.as_deref().unwrap_or_default(), evidence.content.as_deref().unwrap_or_default());
            let language = if matches!(summary_language_for_text(&source), SummaryLanguage::Chinese) {
                "Write rationale and scope in Chinese."
            } else { "Use the dominant language of the supplied content." };
            format!("Compare the exact target and evidence Pages. Neither timestamps, similarity, provenance nor a different client establishes correctness. Distinguish historical events, different subjects, time-bounded preferences, complementary details and partial corrections. Never replace an entire Page for a partial correction that loses independently useful claims. Return only {{\"decision\":\"no_candidate\"}} for merely related/complementary content; {{\"decision\":\"defer\"}} if uncertain; otherwise {{\"decision\":\"reconcile_feedback\",\"target_revision_id\":\"the supplied target Revision\",\"disposition\":\"qualified|disputed|superseded\",\"rationale\":\"specific evidence grounded explanation\",\"scope\":null,\"replacement_revision_id\":null}}. superseded is only for direct, complete replacement of the target claim by the supplied evidence Revision, and must name that exact evidence Revision as replacement_revision_id. qualified and disputed must not name a replacement. Do not invent facts, authority, consent or source material. This is a proposal requiring human Console approval, never an applied update. {language}")
        }
        MaintenanceWorkerRequest::SummarizePage { page } => summary_instructions(page),
        MaintenanceWorkerRequest::SummarizePages { .. } => {
            "Return exactly one JSON object and no markdown. Use either {\"decision\":\"summaries\",\"summaries\":[{\"pageId\":\"pg_...\",\"content\":\"...\"}]}, {\"decision\":\"no_candidate\"}, or {\"decision\":\"defer\"}. Every supplied Page is an eligible missing-summary Page: return exactly one entry for every supplied pageId, do not omit or merge Pages, and use each exact pageId once. Each routing summary must be 60-180 Unicode characters, name the concrete subject and the key assertion, decision, observation, or unresolved question, preserve qualifications, distinguish evidence from inference, and never invent facts. Return no_candidate only when none of the supplied Pages has usable content; otherwise return defer rather than a partial list."
                .to_owned()
        }
        MaintenanceWorkerRequest::SelectPacking { .. } => {
            "Return exactly one JSON object and no markdown. Use either {\"decision\":\"candidate\",\"page_ids\":[\"pg_...\",\"pg_...\"]}, {\"decision\":\"no_candidate\"}, or {\"decision\":\"defer\"}. Select a maximal useful ordered contiguous segment for lossless physical packing. Pages need not state the same fact: keep a coherent local episode together, including its question, answer, correction, qualification, or short reasoning transition. Split only at a clear independent subject or event boundary. A candidate may merge up to two Pages marked packed=true only when they form the same continuous topic; do not merge Packs solely because they are adjacent."
                .to_owned()
        }
        MaintenanceWorkerRequest::AnalyzePacking { .. } => {
            "Return exactly one JSON object and no markdown. Use either {\"decision\":\"packing_candidates\",\"candidates\":[[\"pg_...\",\"pg_...\"]]}, {\"decision\":\"no_candidate\"}, or {\"decision\":\"defer\"}. Analyze every supplied group and return every maximal useful ordered, contiguous, non-overlapping segment for lossless physical packing, with at least two Pages and no more than max_pages_per_candidate. Candidates must form a disjoint partition within each group: every page_id may occur in zero or one candidate only. Before responding, verify no page_id repeats anywhere in candidates. Pages need not state the same fact: keep coherent local episodes together, including questions, answers, corrections, qualifications, and short reasoning transitions. Split only at a clear independent subject or event boundary; temporal adjacency alone is not enough, but do not require semantic equivalence. Never combine Pages from different groups. A candidate may merge up to two Pages marked packed=true only when they form one continuous topic; adjacency alone is insufficient."
                .to_owned()
        }
        MaintenanceWorkerRequest::ExtractTopic { .. } => {
            "Return exactly one JSON object and no markdown. Use either {\"decision\":\"extract_topic\",\"page_ids\":[\"pg_...\",\"pg_...\"],\"title\":\"...\",\"content\":\"...\",\"reason\":\"...\",\"refresh_topic_page_id\":\"pg_...\"}, the same extract_topic form without refresh_topic_page_id, {\"decision\":\"no_candidate\"}, or {\"decision\":\"defer\"}. A Topic Page is a durable front door, not a chronological digest or a replacement for sources. Select 2..=max_source_pages supplied Pages only when they establish one narrow, stable subject that a future query should reach before expanding evidence. Compare the proposed subject with existing_topics. When an existing Topic already represents the same stable subject and its source Page identities substantially overlap the selected sources, set refresh_topic_page_id to that exact offered Page instead of creating a parallel Topic. If the selected logical source Page set exactly matches an existing Topic, refreshing is mandatory. Shared sources alone do not prove semantic identity: omit refresh_topic_page_id for a genuinely distinct narrow subtopic. Temporal adjacency, a shared Scope, broad AI/tool/workspace themes, or superficial keyword overlap are insufficient. When selecting sources, write a specific 120-4000 Unicode-character Topic Page body grounded only in them and a concise title (1-160 chars). Also provide one concise, source-grounded reason (1-480 chars) explaining why these particular Pages jointly warrant a durable Topic Page or refresh. Preserve qualifications, uncertainty, and disagreement; do not invent missing connective claims. Return no_candidate when the window contains no clearly bounded subject."
                .to_owned()
        }
        MaintenanceWorkerRequest::AssessArchive { .. } => {
            "Return exactly one JSON object and no markdown: {\"decision\":\"archive_review\",\"outcome\":\"archive\",\"reason\":\"...\"}, {\"decision\":\"archive_review\",\"outcome\":\"retain\",\"reason\":\"...\"}, or {\"decision\":\"archive_review\",\"outcome\":\"defer\",\"reason\":\"...\"}. This is a human-reviewed content-governance decision: you never delete anything and you do not apply lifecycle changes. Treat candidate_signals only as reasons to inspect, never as a value score. Recommend archive for a low-durability transient record, routine status update, greeting, duplicate observation, or resolved conversational turn that adds no independent reusable evidence, decision, definition, unresolved question, source material, or useful retrieval entry point. Existing summaries and explicit relations are review context, not retention proof by themselves: a Page may still be archived when those links do not rely on its detail. Retain any Page with durable evidence, a specific decision, a stable concept, a useful counterexample, source material, or plausible future value. Defer when the evidence is ambiguous. Your reason must be one concise, grounded sentence naming the Page's actual role; never cite age, lack of visits, lexical similarity, an existing summary, or a relation as sufficient evidence by itself."
                .to_owned()
        }
        MaintenanceWorkerRequest::ReconcileFeedback {
            feedback, targets, ..
        } => {
            let source = std::iter::once(feedback.content.as_deref().unwrap_or_default())
                .chain(targets.iter().filter_map(|page| page.content.as_deref()))
                .collect::<Vec<_>>()
                .join("\n");
            let language = match summary_language_for_text(&source) {
                SummaryLanguage::Chinese => "Write rationale and scope in Chinese prose. Preserve technical names and identifiers exactly.",
                SummaryLanguage::English => "Write rationale and scope in English prose. Preserve technical names and identifiers exactly.",
                SummaryLanguage::Unspecified => "Write rationale and scope in the dominant natural language of the supplied feedback and target Pages.",
            };
            format!(
                "Return exactly one JSON object and no markdown. Use either {{\"decision\":\"reconcile_feedback\",\"target_revision_id\":\"rev_...\",\"disposition\":\"no_source_change|qualified|disputed|superseded|retracted\",\"rationale\":\"...\",\"scope\":null,\"replacement_revision_id\":null}}, or {{\"decision\":\"defer\"}}. Select exactly one target_revision_id from signal.challengedRevisionIds. signal.usedRevisionIds records context actually used by the old response. signal.evidenceRevisionIds contains additional corrective evidence, possibly written later. Neither used-only nor evidence-only Revisions may be selected as the target. This is a validity decision about the exact target Revision, not a judgment of the tenant's opaque source. no_source_change means the feedback does not change stored source validity; qualified means the claim remains useful only within a stated scope; disputed means credible conflict exists without a settled replacement; superseded requires replacement_revision_id naming another supplied Revision that completely replaces it; retracted means the target claim should leave default recall without a replacement. Do not replace an entire Page for a partial correction that would lose unrelated useful claims. A later timestamp, provenance or a client's claimed authority is not proof. Never invent a Revision or infer source contents that PCP did not receive. Preserve uncertainty and prefer defer when evidence is insufficient. rationale must be concise and grounded. {language}"
            )
        }
        MaintenanceWorkerRequest::SelectRelation { pages, .. } => {
            relation_instructions(pages)
        }
        MaintenanceWorkerRequest::SelectRetentionMilestones { .. } => {
            "Return exactly one JSON object and no markdown. Use either {\"decision\":\"retain\",\"milestones\":[{\"revisionId\":\"rev_...\",\"reason\":\"...\"}]}, {\"decision\":\"no_candidate\"}, or {\"decision\":\"defer\"}. Select only supplied Revisions with durable semantic importance and stay within max_revisions."
                .to_owned()
        }
    }
}

fn summary_instructions(page: &super::MaintenanceDetailPage) -> String {
    match summary_language_for_text(&summary_source_text(page)) {
        SummaryLanguage::Chinese => "只输出路由摘要正文，不要 JSON、键、标题、Markdown、引号或解释。该页面已通过结构筛选。写一段 60-180 个 Unicode 字符的摘要。输出语言合同：中文。摘要的自然语言叙述必须以中文书写；技术名、产品名、模型名、版本号、URL、代码标识符和原文引号可按原样保留。不得把中文正文整体翻译成英语。每个提及的命名实体、产品、模型、版本和标识符都必须逐字复制自页面，不得改写拼写、大小写、音译或版本。只写具体主题以及关键主张、决策、观察或未决问题；保留限定条件，不得虚构事实。仅当页面内容无法理解时，精确输出 DEFER。".to_owned(),
        SummaryLanguage::English => "Return only the routing summary itself as plain text: no JSON, keys, heading, Markdown, quotation marks, or explanation. The supplied Page already passed structural eligibility; write a 60-180 Unicode-character routing summary. Output language contract: English. The summary's natural-language prose must be English; preserve technical terms, product names, model names, versions, URLs, code identifiers, and source quotations exactly. Do not translate the Page into another language. Every named entity, product, model, version, and identifier you mention must be copied exactly from the supplied Page: do not change spelling, capitalization, transliteration, or version. State only the concrete subject and key assertion, decision, observation, or unresolved question. Preserve qualifications and never invent facts. Return exactly DEFER only when the Page content cannot be interpreted.".to_owned(),
        SummaryLanguage::Unspecified => "Return only the routing summary itself as plain text: no JSON, keys, heading, Markdown, quotation marks, or explanation. The supplied Page already passed structural eligibility; write a 60-180 Unicode-character routing summary in the Page body's dominant natural language. Preserve technical terms and quotations in their original language. Every named entity, product, model, version, and identifier you mention must be copied exactly from the supplied Page: do not change spelling, capitalization, transliteration, or version. State only the concrete subject and key assertion, decision, observation, or unresolved question. Preserve qualifications and never invent facts. Return exactly DEFER only when the Page content cannot be interpreted.".to_owned(),
    }
}

fn relation_instructions(pages: &[super::RelationCandidatePage]) -> String {
    let base = "Return exactly one JSON object and no markdown. Use either {\"decision\":\"relate\",\"page_ids\":[\"pg_...\",\"pg_...\"],\"reason\":\"...\"}, {\"decision\":\"no_candidate\"}, or {\"decision\":\"defer\"}. Select exactly two supplied Pages only when each directly helps understand, verify, or act on the same stable subject or evidence chain, and never select a pair listed in excluded_page_pairs. With relate, reason must be one concise, grounded sentence that names the shared subject or evidence chain and what the two Pages contribute; it is review evidence, not a generic statement of similarity. Temporal adjacency, shared Scope, co-retrieval, lexical similarity, broad analogy, or merely both discussing AI, tools, infrastructure, harnesses, runtimes, or workspaces are insufficient. Return no_candidate when no pair meets this bar.";
    let language_contract = match relation_language_for_pages(pages) {
        SummaryLanguage::Chinese => {
            "输出语言合同：中文。reason 的自然语言叙述必须使用中文；技术名、产品名、模型名、版本号、URL、代码标识符和原文引号可按原样保留。不得把中文页面整体翻译成英语。"
        }
        SummaryLanguage::English => {
            "Output language contract: English. Write reason in English prose; preserve technical terms, product names, model names, versions, URLs, code identifiers, and source quotations exactly."
        }
        SummaryLanguage::Unspecified => {
            "Write reason in the dominant natural language of the two selected Pages. Preserve technical terms, identifiers, and source quotations in their original language."
        }
    };
    format!("{base} {language_contract}")
}

fn summary_source_text(page: &super::MaintenanceDetailPage) -> String {
    let Some(content) = page.content.as_deref() else {
        return String::new();
    };

    let parsed = page
        .media_type
        .as_deref()
        .is_some_and(|media_type| {
            media_type == PACKED_PAGE_MEDIA_TYPE || media_type.ends_with("+json")
        })
        .then(|| serde_json::from_str::<Value>(content).ok())
        .flatten();
    let Some(parsed) = parsed else {
        return content.to_owned();
    };

    if page.media_type.as_deref() == Some(PACKED_PAGE_MEDIA_TYPE) {
        let entries = parsed
            .get("entries")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                entry
                    .get("payload")
                    .and_then(|payload| payload.get("content"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
            })
            .collect::<Vec<_>>();
        if !entries.is_empty() {
            return entries.join("\n\n");
        }
    }

    let fields = ["title", "summary", "content", "description", "caption"];
    let text = fields
        .iter()
        .filter_map(|field| parsed.get(*field).and_then(Value::as_str))
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>();
    if text.is_empty() {
        content.to_owned()
    } else {
        text.join("\n\n")
    }
}

fn summary_language_for_text(source: &str) -> SummaryLanguage {
    let chinese = source
        .chars()
        .filter(|character| matches!(character, '\u{4E00}'..='\u{9FFF}'))
        .count();
    let latin = source
        .chars()
        .filter(|character| character.is_ascii_alphabetic())
        .count();
    if chinese >= 24 && chinese.saturating_mul(4) >= latin {
        SummaryLanguage::Chinese
    } else if latin >= 24 && latin.saturating_mul(4) >= chinese {
        SummaryLanguage::English
    } else {
        SummaryLanguage::Unspecified
    }
}

fn summary_language_repair_instructions(
    request: &MaintenanceWorkerRequest,
    response: &MaintenanceWorkerResponse,
) -> Option<&'static str> {
    let MaintenanceWorkerRequest::SummarizePage { page } = request else {
        return None;
    };
    let MaintenanceWorkerResponse::WriteSummary { content } = response else {
        return None;
    };
    match summary_language_for_text(&summary_source_text(page)) {
        SummaryLanguage::Chinese if !summary_has_chinese_prose(content) => {
            Some(CHINESE_SUMMARY_REPAIR_INSTRUCTIONS)
        }
        SummaryLanguage::English if !summary_has_english_prose(content) => {
            Some(ENGLISH_SUMMARY_REPAIR_INSTRUCTIONS)
        }
        SummaryLanguage::Unspecified | SummaryLanguage::Chinese | SummaryLanguage::English => None,
    }
}

fn relation_language_for_pages(pages: &[super::RelationCandidatePage]) -> SummaryLanguage {
    let source = pages
        .iter()
        .map(|page| page.routing_text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    summary_language_for_text(&source)
}

fn relation_language_repair_instructions(
    request: &MaintenanceWorkerRequest,
    response: &MaintenanceWorkerResponse,
) -> Option<&'static str> {
    let MaintenanceWorkerRequest::SelectRelation { pages, .. } = request else {
        return None;
    };
    let MaintenanceWorkerResponse::Relate { page_ids, reason } = response else {
        return None;
    };
    match relation_language_for_selected_pages(pages, page_ids) {
        SummaryLanguage::Chinese if !summary_has_chinese_prose(reason) => {
            Some(CHINESE_RELATION_REPAIR_INSTRUCTIONS)
        }
        SummaryLanguage::English if !summary_has_english_prose(reason) => {
            Some(ENGLISH_RELATION_REPAIR_INSTRUCTIONS)
        }
        SummaryLanguage::Unspecified | SummaryLanguage::Chinese | SummaryLanguage::English => None,
    }
}

fn relation_language_for_selected_pages(
    pages: &[super::RelationCandidatePage],
    page_ids: &[String; 2],
) -> SummaryLanguage {
    let selected = pages
        .iter()
        .filter(|page| page_ids.contains(&page.page_id))
        .map(|page| page.routing_text.as_str())
        .collect::<Vec<_>>();
    if selected.len() != 2 {
        return relation_language_for_pages(pages);
    }
    summary_language_for_text(&selected.join("\n"))
}

fn output_language_repair_instructions(
    request: &MaintenanceWorkerRequest,
    response: &MaintenanceWorkerResponse,
) -> Option<&'static str> {
    summary_language_repair_instructions(request, response)
        .or_else(|| relation_language_repair_instructions(request, response))
}

fn summary_has_chinese_prose(summary: &str) -> bool {
    summary
        .chars()
        .filter(|character| matches!(character, '\u{4E00}'..='\u{9FFF}'))
        .count()
        >= 12
}

fn summary_has_english_prose(summary: &str) -> bool {
    summary
        .chars()
        .filter(|character| character.is_ascii_alphabetic())
        .count()
        >= 12
}

fn decode_response(
    response: &ResponsesResult,
    request: &MaintenanceWorkerRequest,
) -> Result<MaintenanceWorkerResponse> {
    let text = extract_output_text(response)?;
    if matches!(request, MaintenanceWorkerRequest::SummarizePage { .. }) {
        return decode_single_summary_response(&text);
    }
    serde_json::from_str(&text).context("decode strict PCP maintenance decision from Infer Runtime")
}

fn decode_single_summary_response(text: &str) -> Result<MaintenanceWorkerResponse> {
    let text = text.trim();
    anyhow::ensure!(
        !text.is_empty(),
        "Infer Runtime maintenance summary response contains no text"
    );
    if text == "DEFER" {
        return Ok(MaintenanceWorkerResponse::Defer);
    }

    // Accept the previous JSON form during the rollout, but only take the summary content.
    // A single-page summary has no model-owned identity or other structured decision surface.
    if text.starts_with('{') {
        let value = serde_json::from_str::<Value>(text)
            .context("decode legacy JSON summary response from Infer Runtime")?;
        if value.get("decision").and_then(Value::as_str) == Some("defer") {
            return Ok(MaintenanceWorkerResponse::Defer);
        }
        let content = value
            .get("content")
            .and_then(Value::as_str)
            .context("legacy JSON summary response has no string content")?;
        return Ok(MaintenanceWorkerResponse::WriteSummary {
            content: content.to_owned(),
        });
    }

    Ok(MaintenanceWorkerResponse::WriteSummary {
        content: text.to_owned(),
    })
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
    use crate::maintenance::worker::{ArchiveCandidatePage, ExistingTopicPage};
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

    fn summary_request() -> MaintenanceWorkerRequest {
        MaintenanceWorkerRequest::SummarizePage {
            page: Box::new(MaintenanceDetailPage {
                page_id: "pg_1".to_owned(),
                revision_id: "rev_1".to_owned(),
                namespace: "project:one".to_owned(),
                created_at: "2026-08-15T00:00:00Z".to_owned(),
                observed_at: None,
                media_type: Some("text/markdown".to_owned()),
                content: Some("这是关于 OpenAI 与 GPT-5.6 的中文页面内容。".repeat(3)),
                summary: None,
                facets: None,
                source_refs: Vec::new(),
                relations: Vec::new(),
            }),
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
            "codex_gpt_5_6_luna",
            "codex_gpt_5_6_luna",
            None,
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
        assert_eq!(infer.reasoning, Some(serde_json::json!({"effort": "high"})));
        assert_eq!(infer.max_output_tokens, None);
    }

    #[test]
    fn summary_request_uses_prompt_bound_without_provider_output_constraint() {
        let request = summary_request();
        let infer = infer_request(
            &request,
            Duration::from_secs(12),
            "codex_gpt_5_6_luna",
            "codex_gpt_5_6_luna",
            None,
            None,
        )
        .expect("build Infer summary request");

        assert_eq!(infer.model, "language.respond");
        assert_eq!(infer.metadata["infer.deployment_ids"], "codex_gpt_5_6_luna");
        assert_eq!(infer.metadata["infer.placement"], "cloud_only");
        assert_eq!(infer.metadata["infer.prefer"], "cloud");
        assert!(!infer.metadata.contains_key("infer.offline_required"));
        assert_eq!(
            infer.metadata["infer.provider_access_class"],
            "subscription"
        );
        assert_eq!(infer.metadata["infer.capability_floor"], "advanced");
        assert_eq!(infer.metadata["infer.fallback"], "none");
        assert_eq!(
            infer.reasoning,
            Some(serde_json::json!({"effort": "medium"}))
        );
        assert_eq!(infer.max_output_tokens, None);
        assert!(
            infer
                .instructions
                .as_ref()
                .and_then(Value::as_str)
                .is_some_and(|instructions| {
                    instructions.contains("60-180 个 Unicode 字符")
                        && instructions.contains("输出语言合同：中文")
                        && instructions.contains("不得把中文正文整体翻译成英语")
                        && !instructions.contains("write_summary")
                        && !instructions.contains("pageId")
                        && !instructions.contains("summaries")
                })
        );
        let payload = infer
            .input
            .as_str()
            .expect("summary inference input is text");
        assert!(payload.contains("这是关于 OpenAI 与 GPT-5.6 的中文页面内容"));
        assert!(!payload.contains("project:one"));
    }

    #[test]
    fn packed_summary_input_uses_entry_bodies_not_serialized_envelope() {
        let request = MaintenanceWorkerRequest::SummarizePage {
            page: Box::new(MaintenanceDetailPage {
                page_id: "pg_pack".to_owned(),
                revision_id: "rev_pack".to_owned(),
                namespace: "conversation:test".to_owned(),
                created_at: "2026-08-15T00:00:00Z".to_owned(),
                observed_at: None,
                media_type: Some(PACKED_PAGE_MEDIA_TYPE.to_owned()),
                content: Some(
                    r#"{"entries":[{"payload":{"content":"用户用中文询问 OpenAI 的长期上下文。"},"createdBy":{"actorId":"local-user"}},{"payload":{"content":"回答指出 PCP 应按相关性召回。"},"createdBy":{"actorId":"model:assistant"}}]}"#
                        .to_owned(),
                ),
                summary: None,
                facets: None,
                source_refs: Vec::new(),
                relations: Vec::new(),
            }),
        };

        let infer = infer_request(
            &request,
            Duration::from_secs(12),
            "codex_gpt_5_6_luna",
            "codex_gpt_5_6_luna",
            None,
            None,
        )
        .expect("build Infer summary request");

        let payload = infer
            .input
            .as_str()
            .expect("summary inference input is text");
        assert!(payload.contains("用户用中文询问 OpenAI 的长期上下文"));
        assert!(payload.contains("回答指出 PCP 应按相关性召回"));
        assert!(!payload.contains("createdBy"));
        assert!(!payload.contains("local-user"));
        assert!(
            infer
                .instructions
                .as_ref()
                .and_then(Value::as_str)
                .is_some_and(|instructions| instructions.contains("输出语言合同：中文"))
        );
    }

    #[test]
    fn summary_language_contract_allows_identifiers_but_rejects_english_prose_for_chinese_pages() {
        let request = summary_request();
        let MaintenanceWorkerRequest::SummarizePage { page } = &request else {
            panic!("expected a summary request");
        };

        assert_eq!(
            summary_language_for_text(&summary_source_text(page)),
            SummaryLanguage::Chinese
        );
        assert!(summary_has_chinese_prose(
            "页面讨论 OpenAI 与 GPT-5.6 的中文长期上下文策略，并保留了版本与产品名称。"
        ));
        assert!(!summary_has_chinese_prose(
            "The page discusses OpenAI and GPT-5.6 long-context strategy in Chinese."
        ));
        assert!(
            summary_language_repair_instructions(
                &request,
                &MaintenanceWorkerResponse::WriteSummary {
                    content:
                        "The page discusses OpenAI and GPT-5.6 long-context strategy in Chinese."
                            .to_owned(),
                }
            )
            .is_some()
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
            None,
        )
        .expect("build Infer relation escalation request");

        assert_eq!(
            infer.metadata["infer.deployment_ids"],
            "codex_gpt_5_6_terra"
        );
        assert_eq!(infer.metadata["infer.fallback"], "none");
        assert_eq!(infer.reasoning, Some(serde_json::json!({"effort": "high"})));
        assert_eq!(infer.max_output_tokens, None);
    }

    #[test]
    fn relation_request_follows_chinese_source_language_and_repairs_english_reasoning() {
        let pages = vec![
            RelationCandidatePage {
                page_id: "pg_1".to_owned(),
                namespace: "project:one".to_owned(),
                kind: "document".to_owned(),
                created_at: "2026-08-15T00:00:00Z".to_owned(),
                observed_at: None,
                routing_text:
                    "页面记录 PCP 如何维护稳定的中文长期记忆，并说明关联必须经过人工审阅。"
                        .repeat(2),
                facets: None,
                relation_types: Vec::new(),
            },
            RelationCandidatePage {
                page_id: "pg_2".to_owned(),
                namespace: "project:one".to_owned(),
                kind: "document".to_owned(),
                created_at: "2026-08-16T00:00:00Z".to_owned(),
                observed_at: None,
                routing_text: "另一页解释 PCP 的关联审阅如何保留证据链，并避免相似度直接变成事实。"
                    .repeat(2),
                facets: None,
                relation_types: Vec::new(),
            },
        ];
        let request = MaintenanceWorkerRequest::SelectRelation {
            pages,
            excluded_page_pairs: Vec::new(),
        };
        let infer = infer_request(
            &request,
            Duration::from_secs(12),
            "codex_gpt_5_6_luna",
            "codex_gpt_5_6_luna",
            None,
            None,
        )
        .expect("build Chinese relation request");

        assert!(
            infer
                .instructions
                .as_ref()
                .and_then(Value::as_str)
                .is_some_and(|instructions| {
                    instructions.contains("输出语言合同：中文")
                        && instructions.contains("reason 的自然语言叙述必须使用中文")
                })
        );
        assert!(
            relation_language_repair_instructions(
                &request,
                &MaintenanceWorkerResponse::Relate {
                    page_ids: ["pg_1".to_owned(), "pg_2".to_owned()],
                    reason: "The two Pages establish one stable PCP evidence chain.".to_owned(),
                }
            )
            .is_some()
        );
        assert!(
            relation_language_repair_instructions(
                &request,
                &MaintenanceWorkerResponse::Relate {
                    page_ids: ["pg_1".to_owned(), "pg_2".to_owned()],
                    reason: "两页共同说明 PCP 关联审阅如何保留同一条稳定证据链。".to_owned(),
                }
            )
            .is_none()
        );
    }

    #[test]
    fn bounded_escalation_override_selects_sol_without_fallback() {
        let request = MaintenanceWorkerRequest::AssessArchive {
            page: ArchiveCandidatePage {
                page: MaintenanceDetailPage {
                    page_id: "pg_1".to_owned(),
                    revision_id: "rev_1".to_owned(),
                    namespace: "project:one".to_owned(),
                    created_at: "2026-01-01T00:00:00Z".to_owned(),
                    observed_at: None,
                    media_type: Some("text/markdown".to_owned()),
                    content: Some("transient status".to_owned()),
                    summary: None,
                    facets: None,
                    source_refs: Vec::new(),
                    relations: Vec::new(),
                },
                candidate_signals: vec!["older_than_14_days".to_owned()],
            },
        };
        let infer = infer_request(
            &request,
            Duration::from_secs(12),
            "codex_gpt_5_6_luna",
            "codex_gpt_5_6_luna",
            None,
            Some("codex_gpt_5_6_sol"),
        )
        .expect("build Sol escalation request");

        assert_eq!(infer.metadata["infer.deployment_ids"], "codex_gpt_5_6_sol");
        assert_eq!(infer.metadata["infer.fallback"], "none");
        assert!(response_defers(&MaintenanceWorkerResponse::Defer));
        assert!(response_defers(&MaintenanceWorkerResponse::ArchiveReview {
            outcome: ArchiveWorkerDecision::Defer,
            reason: "ambiguous".to_owned(),
        }));
        assert!(!response_defers(&MaintenanceWorkerResponse::NoCandidate));
    }

    #[test]
    fn topic_extraction_is_a_reasoning_decision_with_explicit_sources() {
        let request = MaintenanceWorkerRequest::ExtractTopic {
            pages: vec![RelationCandidatePage {
                page_id: "pg_1".to_owned(),
                namespace: "project:one".to_owned(),
                kind: "document".to_owned(),
                created_at: "2026-08-15T00:00:00Z".to_owned(),
                observed_at: None,
                routing_text: "PCP topics preserve source Pages as evidence.".to_owned(),
                facets: None,
                relation_types: Vec::new(),
            }],
            existing_topics: vec![ExistingTopicPage {
                page_id: "pg_topic".to_owned(),
                revision_id: "rev_topic".to_owned(),
                title: "Existing PCP topic".to_owned(),
                routing_text: "An existing durable PCP retrieval boundary.".to_owned(),
                source_page_ids: vec!["pg_1".to_owned(), "pg_2".to_owned()],
            }],
            max_source_pages: 8,
        };
        let infer = infer_request(
            &request,
            Duration::from_secs(12),
            "codex_gpt_5_6_luna",
            "codex_gpt_5_6_luna",
            None,
            None,
        )
        .expect("build Topic extraction request");
        assert_eq!(infer.model, "reasoning.solve");
        assert_eq!(infer.metadata["infer.deployment_ids"], "codex_gpt_5_6_luna");
        assert!(
            infer
                .instructions
                .as_ref()
                .and_then(Value::as_str)
                .is_some_and(|instructions| instructions.contains("durable front door")
                    && instructions.contains("2..=max_source_pages")
                    && instructions.contains("refresh_topic_page_id")
                    && instructions.contains("Temporal adjacency"))
        );
        let decoded = decode_response(
            &result("{\"decision\":\"extract_topic\",\"page_ids\":[\"pg_1\",\"pg_2\"],\"title\":\"PCP topic\",\"content\":\"A durable, source-grounded Topic Page for PCP retrieval.\",\"reason\":\"The two Pages establish one stable PCP retrieval boundary.\",\"refresh_topic_page_id\":\"pg_topic\"}"),
            &request,
        )
        .expect("decode Topic extraction response");
        assert!(matches!(
            decoded,
            MaintenanceWorkerResponse::ExtractTopic {
                refresh_topic_page_id: Some(ref page_id),
                ..
            } if page_id == "pg_topic"
        ));
    }

    #[test]
    fn non_summary_output_requires_exact_json_without_markdown_fences() {
        let request = MaintenanceWorkerRequest::SelectRetentionMilestones {
            pages: Vec::new(),
            max_revisions: 1,
            lease_days: 30,
        };
        let decoded = decode_response(&result(
            "{\"decision\":\"retain\",\"milestones\":[{\"revisionId\":\"rev_1\",\"reason\":\"durable\"}]}",
        ), &request)
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
        assert!(
            decode_response(
                &result("```json\n{\"decision\":\"no_candidate\"}\n```"),
                &request
            )
            .is_err()
        );
    }

    #[test]
    fn single_summary_output_uses_raw_text_or_the_defer_sentinel() {
        let request = summary_request();
        let decoded = decode_response(
            &result("原始页面讨论本地 AI 工作负载下的存储压力与外置 NVMe 方案。"),
            &request,
        )
        .expect("decode raw summary output");
        assert!(matches!(
            decoded,
            MaintenanceWorkerResponse::WriteSummary { content }
                if content == "原始页面讨论本地 AI 工作负载下的存储压力与外置 NVMe 方案。"
        ));
        assert!(matches!(
            decode_response(&result("DEFER"), &request).expect("decode defer sentinel"),
            MaintenanceWorkerResponse::Defer
        ));
    }

    #[test]
    fn single_summary_legacy_json_uses_content_and_ignores_extra_fields() {
        let decoded = decode_response(
            &result(
                "{\"decision\":\"write_summary\",\"content\":\"页面讨论本地 AI 工作负载的存储取舍。\",\"rests_on\":[\"pg_previous\"]}",
            ),
            &summary_request(),
        )
        .expect("decode legacy JSON summary output");
        assert!(matches!(
            decoded,
            MaintenanceWorkerResponse::WriteSummary { content }
                if content == "页面讨论本地 AI 工作负载的存储取舍。"
        ));
    }

    #[test]
    fn infer_output_rejects_unknown_decision_fields() {
        let request = MaintenanceWorkerRequest::SelectRelation {
            pages: Vec::new(),
            excluded_page_pairs: Vec::new(),
        };
        assert!(
            decode_response(&result(
                "{\"decision\":\"relate\",\"page_ids\":[\"pg_1\",\"pg_2\"],\"reason\":\"The two pages establish one durable subject.\",\"relation_type\":\"summarizes\"}"
            ), &request)
            .is_err()
        );
    }

    #[test]
    fn relation_output_requires_grounded_review_evidence() {
        let request = MaintenanceWorkerRequest::SelectRelation {
            pages: Vec::new(),
            excluded_page_pairs: Vec::new(),
        };
        let decoded = decode_response(
            &result(
                "{\"decision\":\"relate\",\"page_ids\":[\"pg_1\",\"pg_2\"],\"reason\":\"The two Pages establish the same bounded evidence chain.\"}",
            ),
            &request,
        )
        .expect("decode relation evidence");
        assert!(matches!(
            decoded,
            MaintenanceWorkerResponse::Relate { reason, .. }
                if reason == "The two Pages establish the same bounded evidence chain."
        ));
        assert!(
            decode_response(
                &result("{\"decision\":\"relate\",\"page_ids\":[\"pg_1\",\"pg_2\"]}"),
                &request,
            )
            .is_err()
        );
    }
}
