use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use infer_runtime_client::{Client, ResponsesRequest, ResponsesResult};
use pcp_client::PcpTenantApi;
use pcp_core::{
    BrowseIndexOrder, GraphEdgeDirection, GraphEdgeKind, IntentEffort, IntentMatchAudit,
    Projection, ReadPagesRequest, RouterTokenUsage, SearchFilters, SearchHit, SearchMode,
    SearchPagesRequest, SearchTermMatch,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tokio::time::{Instant, sleep, timeout};

use crate::semantic_search::SemanticSearchProvider;

const POLL_INTERVAL: Duration = Duration::from_millis(250);
const MAX_ROUTER_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_PROFILE_CHARS: usize = 420;
const MAX_CONSULT_CHARS: usize = 1_600;
const MAX_EXACT_HITS_PER_TERM: u32 = 12;
const MAX_GRAPH_NEIGHBORS_PER_SEED: u32 = 16;

fn record_usage(tally: &mut RouterTokenUsage, response: &ResponsesResult) {
    let Some(reported) = response.extra.get("usage").and_then(Value::as_object) else {
        tally.unreported_responses += 1;
        return;
    };
    let Some(input_tokens) = reported.get("input_tokens").and_then(Value::as_u64) else {
        tally.unreported_responses += 1;
        return;
    };
    let Some(output_tokens) = reported.get("output_tokens").and_then(Value::as_u64) else {
        tally.unreported_responses += 1;
        return;
    };
    tally.reported_responses += 1;
    tally.input_tokens = tally.input_tokens.saturating_add(input_tokens);
    tally.output_tokens = tally.output_tokens.saturating_add(output_tokens);
    tally.total_tokens = tally.total_tokens.saturating_add(
        reported
            .get("total_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| input_tokens.saturating_add(output_tokens)),
    );
    tally.cached_input_tokens = tally.cached_input_tokens.saturating_add(
        reported
            .get("input_tokens_details")
            .and_then(Value::as_object)
            .and_then(|details| details.get("cached_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or_default(),
    );
    tally.reasoning_tokens = tally.reasoning_tokens.saturating_add(
        reported
            .get("output_tokens_details")
            .and_then(Value::as_object)
            .and_then(|details| details.get("reasoning_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or_default(),
    );
}

pub struct IntentMatchResult {
    pub hits: Vec<IntentMatchedHit>,
    pub indexed_count: usize,
    pub embedded_count: usize,
    pub audit: IntentMatchAudit,
}

pub struct IntentMatchedHit {
    pub hit: SearchHit,
    pub semantic_score: Option<f32>,
    pub intent_reason: String,
}

pub struct IntentMatchProvider {
    client: Client,
    timeout: Duration,
    max_catalog_pages: usize,
}

impl IntentMatchProvider {
    pub fn new(config: crate::IntentMatchConfig) -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .credential_file(config.credential_file)
                .build()
                .context("build Infer Runtime client for PCP intent matching")?,
            timeout: Duration::from_secs(config.timeout_seconds),
            max_catalog_pages: config.max_catalog_pages,
        })
    }

    pub async fn search(
        &self,
        client: &dyn PcpTenantApi,
        semantic_search: &SemanticSearchProvider,
        query: &str,
        scopes: &[String],
        effort: IntentEffort,
        top_k: usize,
    ) -> Result<IntentMatchResult> {
        let budget = IntentBudget::for_effort(effort);
        let mut router_usage = RouterTokenUsage::default();
        let plan = self.plan(query, budget, &mut router_usage).await?;
        let mut router_rounds = 1;
        let mut candidate_pool = CandidatePool::default();
        let mut semantic_probes = vec![query.to_owned()];
        semantic_probes.extend(plan.probes.into_iter().take(budget.initial_probe_limit));
        semantic_probes = deduplicate_strings(semantic_probes, budget.total_probe_limit);

        let mut indexed_count: usize = 0;
        let mut embedded_count: usize = 0;
        for probe in &semantic_probes {
            let result = semantic_search
                .search(client, probe, scopes, budget.semantic_candidates_per_probe)
                .await
                .with_context(|| {
                    format!("semantic candidate discovery for intent probe {probe:?}")
                })?;
            indexed_count = indexed_count.max(result.indexed_count);
            embedded_count = embedded_count.saturating_add(result.embedded_count);
            candidate_pool.add_semantic_hits(result.hits, probe);
        }
        let exact_terms = deduplicate_strings(plan.exact_terms, budget.exact_term_limit);
        for term in &exact_terms {
            let result = client
                .browse_retrieval_pages(
                    scopes.to_vec(),
                    Some(term.clone()),
                    BrowseIndexOrder::Recent,
                    MAX_EXACT_HITS_PER_TERM,
                    None,
                    16_000,
                )
                .await
                .with_context(|| format!("exact entity candidate discovery for {term:?}"))?;
            candidate_pool.add_exact_hits(result.hits, term);
        }
        let mut relation_candidates_considered = self
            .add_relation_candidates(
                client,
                &mut candidate_pool,
                scopes,
                budget.relation_seed_limit,
            )
            .await?;
        let catalog_pages_considered = if budget.include_catalog {
            self.add_catalog_candidates(client, &mut candidate_pool, scopes)
                .await?
        } else {
            0
        };

        let mut remaining_expansions = budget.expansion_round_limit;
        let consult = loop {
            let review = self
                .review(
                    query,
                    &candidate_pool.cards(budget.candidate_card_limit),
                    remaining_expansions
                        .min(1)
                        .saturating_mul(budget.expansion_probe_limit),
                    budget.consult_limit,
                    budget.effort,
                    &mut router_usage,
                )
                .await?;
            router_rounds += 1;
            let expansion_probes =
                deduplicate_strings(review.expansion_probes, budget.expansion_probe_limit);
            if expansion_probes.is_empty() || remaining_expansions == 0 {
                break review.consult_page_ids;
            }
            remaining_expansions = remaining_expansions.saturating_sub(1);
            for probe in expansion_probes {
                if semantic_probes.len() >= budget.total_probe_limit {
                    break;
                }
                if semantic_probes
                    .iter()
                    .any(|existing| equivalent(existing, &probe))
                {
                    continue;
                }
                let result = semantic_search
                    .search(client, &probe, scopes, budget.semantic_candidates_per_probe)
                    .await
                    .with_context(|| format!("semantic expansion for intent probe {probe:?}"))?;
                indexed_count = indexed_count.max(result.indexed_count);
                embedded_count = embedded_count.saturating_add(result.embedded_count);
                candidate_pool.add_semantic_hits(result.hits, &probe);
                semantic_probes.push(probe);
            }
            relation_candidates_considered = relation_candidates_considered.saturating_add(
                self.add_relation_candidates(
                    client,
                    &mut candidate_pool,
                    scopes,
                    budget.relation_seed_limit,
                )
                .await?,
            );
        };

        let consult = candidate_pool.valid_page_ids(consult, budget.consult_limit);
        let consulted = self.consult_cards(client, &consult).await?;
        let selection = self
            .finalize(query, &consulted, top_k, budget.effort, &mut router_usage)
            .await?;
        router_rounds += 1;
        let selected = candidate_pool.valid_page_ids(selection.page_ids, top_k);
        let hits = selected
            .into_iter()
            .filter_map(|page_id| candidate_pool.candidate(&page_id))
            .map(|candidate| IntentMatchedHit {
                hit: candidate.hit.clone(),
                semantic_score: candidate.semantic_score,
                intent_reason: candidate.intent_reason(),
            })
            .collect::<Vec<_>>();
        let stopped_reason = if hits.len() >= top_k {
            "router_selected_requested_limit".to_owned()
        } else if consulted.is_empty() {
            "router_found_no_candidate_worth_consulting".to_owned()
        } else {
            "router_completed_bounded_review".to_owned()
        };
        Ok(IntentMatchResult {
            hits,
            indexed_count,
            embedded_count,
            audit: IntentMatchAudit {
                effort,
                router_rounds,
                router_usage,
                semantic_probes,
                exact_terms,
                candidate_count: candidate_pool.len(),
                relation_candidates_considered,
                consulted_count: consulted.len(),
                catalog_pages_considered,
                stopped_reason,
            },
        })
    }

    async fn plan(
        &self,
        query: &str,
        budget: IntentBudget,
        usage: &mut RouterTokenUsage,
    ) -> Result<RouterPlan> {
        self.call_json(
            format!(
                "You are PCP's intent-retrieval planner. The user intent is:\n{query}\n\nReturn exactly JSON: {{\"probes\":[\"...\"],\"exactTerms\":[\"...\"]}}. Probes are alternative conceptual routes that could surface relevant Pages even when their wording is unlike the request. Do not answer the user. Do not include the original query. Propose at most {} probes and {} exact entity, acronym, or product-name anchors. Prefer recall-oriented but concrete hypotheses; avoid generic AI, tooling, or workspace queries.",
                budget.initial_probe_limit, budget.exact_term_limit
            ),
            "Plan only; output strict JSON without markdown.",
            budget.effort,
            usage,
        )
        .await
    }

    async fn review(
        &self,
        query: &str,
        candidates: &[CandidateCard],
        expansion_probe_limit: usize,
        consult_limit: usize,
        effort: IntentEffort,
        usage: &mut RouterTokenUsage,
    ) -> Result<RouterReview> {
        let input = json!({"intent": query, "candidates": candidates});
        self.call_json(
            serde_json::to_string(&input).context("encode intent candidate cards")?,
            &format!(
                "You are PCP's logical relevance reviewer. Given the intent and candidate Page cards, identify Pages worth consulting before final Context Pack selection. A relation is only a lead: it is not evidence of relevance on its own. Select at most {consult_limit} candidate pageIds. You may ask for up to {expansion_probe_limit} targeted semantic expansion probes only when the current cards reveal a concrete missing interpretation, bridge, prerequisite, counterexample, or unresolved conflict. Do not answer the intent and do not fabricate pageIds. Return exactly JSON: {{\"consultPageIds\":[\"pg_...\"],\"expansionProbes\":[\"...\"]}}."
            ),
            effort,
            usage,
        )
        .await
    }

    async fn finalize(
        &self,
        query: &str,
        consulted: &[ConsultCard],
        top_k: usize,
        effort: IntentEffort,
        usage: &mut RouterTokenUsage,
    ) -> Result<RouterSelection> {
        let input = json!({"intent": query, "consultedPages": consulted});
        self.call_json(
            serde_json::to_string(&input).context("encode consulted intent pages")?,
            &format!(
                "You are PCP's final intent-match judge. Select only consulted Pages that directly help answer, verify, qualify, or act on the requested intent. Prefer decisive evidence and necessary context over broad analogy. Do not select a Page merely because it is related to another selected Page. Return exactly JSON: {{\"pageIds\":[\"pg_...\"]}} with at most {top_k} IDs, in usefulness order."
            ),
            effort,
            usage,
        )
        .await
    }

    async fn add_relation_candidates(
        &self,
        client: &dyn PcpTenantApi,
        pool: &mut CandidatePool,
        scopes: &[String],
        seed_limit: usize,
    ) -> Result<usize> {
        let seeds = pool
            .semantic_candidates(seed_limit)
            .into_iter()
            .map(|candidate| candidate.hit.clone())
            .collect::<Vec<_>>();
        let mut considered = 0;
        for seed in seeds {
            let graph = client
                .search_pages(SearchPagesRequest {
                    query: seed.revision_id.clone(),
                    scopes: scopes.to_vec(),
                    mode: SearchMode::Graph,
                    term_match: SearchTermMatch::All,
                    projections: vec![Projection::Summary, Projection::Payload],
                    filters: SearchFilters::default(),
                    limit: MAX_GRAPH_NEIGHBORS_PER_SEED,
                    cursor: None,
                })
                .await?;
            for neighbor in graph.hits {
                let relations = neighbor
                    .graph_edges
                    .iter()
                    .filter(|edge| edge.edge_kind == GraphEdgeKind::Relation)
                    .map(|edge| {
                        format!(
                            "{} ({})",
                            edge.relation_type,
                            match edge.direction {
                                GraphEdgeDirection::Incoming => "incoming",
                                GraphEdgeDirection::Outgoing => "outgoing",
                            }
                        )
                    })
                    .collect::<Vec<_>>();
                if relations.is_empty() {
                    continue;
                }
                considered += 1;
                pool.add_relation_hit(neighbor, &seed.page_id, relations);
            }
        }
        Ok(considered)
    }

    async fn add_catalog_candidates(
        &self,
        client: &dyn PcpTenantApi,
        pool: &mut CandidatePool,
        scopes: &[String],
    ) -> Result<usize> {
        let mut cursor = None;
        let mut total = 0;
        while total < self.max_catalog_pages {
            let result = client
                .browse_retrieval_pages(
                    scopes.to_vec(),
                    None,
                    BrowseIndexOrder::Recent,
                    50,
                    cursor,
                    32_000,
                )
                .await?;
            if result.hits.is_empty() {
                break;
            }
            for hit in result.hits {
                if total >= self.max_catalog_pages {
                    break;
                }
                total += 1;
                pool.add_catalog_hit(hit);
            }
            cursor = result.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        Ok(total)
    }

    async fn consult_cards(
        &self,
        client: &dyn PcpTenantApi,
        page_ids: &[String],
    ) -> Result<Vec<ConsultCard>> {
        if page_ids.is_empty() {
            return Ok(Vec::new());
        }
        let pages = client
            .read_pages(ReadPagesRequest {
                page_ids: page_ids.to_vec(),
                revision_ids: Vec::new(),
                projections: vec![
                    Projection::Manifest,
                    Projection::Payload,
                    Projection::Summary,
                ],
                max_chars: 32_000,
            })
            .await?;
        Ok(pages
            .into_iter()
            .map(|page| {
                let source = page
                    .summary
                    .filter(|summary| summary.target_revision_id == page.revision.revision_id)
                    .map(|summary| summary.content)
                    .or_else(|| page.revision.payload.map(|payload| payload.content))
                    .unwrap_or_default();
                ConsultCard {
                    page_id: page.page.page_id,
                    namespace: page.revision.namespace,
                    content: truncate(&source, MAX_CONSULT_CHARS),
                }
            })
            .collect())
    }

    async fn call_json<T: DeserializeOwned>(
        &self,
        input: String,
        instructions: &str,
        effort: IntentEffort,
        usage: &mut RouterTokenUsage,
    ) -> Result<T> {
        let response = self
            .response(input, instructions, effort)
            .await
            .context("run PCP intent Router")?;
        record_usage(usage, &response);
        let output = extract_output_text(&response)?;
        serde_json::from_str(&output).context("decode PCP intent Router JSON")
    }

    async fn response(
        &self,
        input: String,
        instructions: &str,
        effort: IntentEffort,
    ) -> Result<ResponsesResult> {
        let deadline_ms = self.timeout.as_millis().clamp(1, u128::from(u64::MAX));
        let request = ResponsesRequest {
            model: "reasoning.solve".to_owned(),
            input: Value::String(input),
            instructions: Some(Value::String(instructions.to_owned())),
            stream: false,
            background: false,
            metadata: BTreeMap::from([
                ("infer.priority".to_owned(), "background".to_owned()),
                ("infer.max_cost_usd".to_owned(), "0".to_owned()),
                ("infer.fallback".to_owned(), "none".to_owned()),
                ("infer.deadline_ms".to_owned(), deadline_ms.to_string()),
                ("infer.placement".to_owned(), "cloud_only".to_owned()),
                ("infer.prefer".to_owned(), "cloud".to_owned()),
                (
                    "infer.provider_access_class".to_owned(),
                    "subscription".to_owned(),
                ),
                ("infer.capability_floor".to_owned(), "advanced".to_owned()),
            ]),
            tools: Vec::new(),
            reasoning: Some(json!({"effort": effort_name(effort)})),
            max_output_tokens: None,
        };
        let started = Instant::now();
        let mut response = timeout(self.timeout, self.client.create_response(&request))
            .await
            .context("PCP intent Router submission timed out")??;
        loop {
            match response.status.as_str() {
                "completed" => return Ok(response),
                "queued" | "in_progress" => {}
                "failed" | "cancelled" | "incomplete" => anyhow::bail!(
                    "PCP intent Router response {} ended with status {}",
                    response.id,
                    response.status
                ),
                status => anyhow::bail!(
                    "PCP intent Router response {} returned unknown status {status}",
                    response.id
                ),
            }
            let Some(remaining) = self.timeout.checked_sub(started.elapsed()) else {
                let _ = self.client.cancel_response(&response.id).await;
                anyhow::bail!("PCP intent Router timed out");
            };
            sleep(POLL_INTERVAL.min(remaining)).await;
            response = timeout(remaining, self.client.get_response(&response.id))
                .await
                .context("PCP intent Router polling timed out")??;
        }
    }
}

#[derive(Clone, Copy)]
struct IntentBudget {
    effort: IntentEffort,
    initial_probe_limit: usize,
    expansion_round_limit: usize,
    expansion_probe_limit: usize,
    total_probe_limit: usize,
    exact_term_limit: usize,
    semantic_candidates_per_probe: usize,
    candidate_card_limit: usize,
    relation_seed_limit: usize,
    consult_limit: usize,
    include_catalog: bool,
}

impl IntentBudget {
    fn for_effort(effort: IntentEffort) -> Self {
        match effort {
            IntentEffort::Low => Self {
                effort,
                initial_probe_limit: 0,
                expansion_round_limit: 0,
                expansion_probe_limit: 0,
                total_probe_limit: 1,
                exact_term_limit: 2,
                semantic_candidates_per_probe: 24,
                candidate_card_limit: 28,
                relation_seed_limit: 0,
                consult_limit: 4,
                include_catalog: false,
            },
            IntentEffort::Medium => Self {
                effort,
                initial_probe_limit: 2,
                expansion_round_limit: 1,
                expansion_probe_limit: 2,
                total_probe_limit: 5,
                exact_term_limit: 3,
                semantic_candidates_per_probe: 16,
                candidate_card_limit: 54,
                relation_seed_limit: 4,
                consult_limit: 8,
                include_catalog: false,
            },
            IntentEffort::High => Self {
                effort,
                initial_probe_limit: 3,
                expansion_round_limit: 2,
                expansion_probe_limit: 2,
                total_probe_limit: 8,
                exact_term_limit: 4,
                semantic_candidates_per_probe: 16,
                candidate_card_limit: 250,
                relation_seed_limit: 8,
                consult_limit: 12,
                include_catalog: true,
            },
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RouterPlan {
    #[serde(default)]
    probes: Vec<String>,
    #[serde(default)]
    exact_terms: Vec<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RouterReview {
    #[serde(default)]
    consult_page_ids: Vec<String>,
    #[serde(default)]
    expansion_probes: Vec<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RouterSelection {
    #[serde(default)]
    page_ids: Vec<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CandidateCard {
    page_id: String,
    namespace: String,
    kind: String,
    profile: String,
    origins: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    relation_leads: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConsultCard {
    page_id: String,
    namespace: String,
    content: String,
}

#[derive(Default)]
struct CandidatePool {
    candidates: BTreeMap<String, Candidate>,
}

#[derive(Clone)]
struct Candidate {
    hit: SearchHit,
    semantic_score: Option<f32>,
    origins: BTreeSet<String>,
    relation_leads: BTreeSet<String>,
}

impl CandidatePool {
    fn add_semantic_hits(
        &mut self,
        hits: Vec<crate::semantic_search::SemanticSearchHit>,
        probe: &str,
    ) {
        for item in hits {
            let candidate = self.entry(item.hit);
            candidate.semantic_score = Some(
                candidate
                    .semantic_score
                    .unwrap_or(f32::NEG_INFINITY)
                    .max(item.score),
            );
            candidate.origins.insert(format!("semantic:{probe}"));
        }
    }

    fn add_exact_hits(&mut self, hits: Vec<SearchHit>, term: &str) {
        for hit in hits {
            self.entry(hit).origins.insert(format!("exact:{term}"));
        }
    }

    fn add_relation_hit(&mut self, hit: SearchHit, seed_page_id: &str, relations: Vec<String>) {
        let candidate = self.entry(hit);
        candidate.origins.insert("relation_lead".to_owned());
        candidate.relation_leads.extend(
            relations
                .into_iter()
                .map(|relation| format!("{relation} from {seed_page_id}")),
        );
    }

    fn add_catalog_hit(&mut self, hit: SearchHit) {
        self.entry(hit).origins.insert("catalog".to_owned());
    }

    fn entry(&mut self, hit: SearchHit) -> &mut Candidate {
        self.candidates
            .entry(hit.page_id.clone())
            .and_modify(|candidate| {
                if candidate.hit.revision_id != hit.revision_id {
                    *candidate = Candidate {
                        hit: hit.clone(),
                        semantic_score: None,
                        origins: BTreeSet::new(),
                        relation_leads: BTreeSet::new(),
                    };
                }
            })
            .or_insert_with(|| Candidate {
                hit,
                semantic_score: None,
                origins: BTreeSet::new(),
                relation_leads: BTreeSet::new(),
            })
    }

    fn semantic_candidates(&self, limit: usize) -> Vec<&Candidate> {
        let mut candidates = self
            .candidates
            .values()
            .filter(|candidate| candidate.semantic_score.is_some())
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .semantic_score
                .unwrap_or_default()
                .total_cmp(&left.semantic_score.unwrap_or_default())
        });
        candidates.truncate(limit);
        candidates
    }

    fn cards(&self, limit: usize) -> Vec<CandidateCard> {
        let mut candidates = self.candidates.values().collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .semantic_score
                .unwrap_or_default()
                .total_cmp(&left.semantic_score.unwrap_or_default())
                .then_with(|| left.hit.page_id.cmp(&right.hit.page_id))
        });
        candidates.truncate(limit);
        candidates
            .into_iter()
            .map(|candidate| CandidateCard {
                page_id: candidate.hit.page_id.clone(),
                namespace: candidate.hit.namespace.clone(),
                kind: candidate.hit.kind.clone(),
                profile: truncate(&candidate.hit.snippet, MAX_PROFILE_CHARS),
                origins: candidate.origins.iter().cloned().collect(),
                relation_leads: candidate.relation_leads.iter().cloned().collect(),
            })
            .collect()
    }

    fn valid_page_ids(&self, page_ids: Vec<String>, limit: usize) -> Vec<String> {
        let mut seen = BTreeSet::new();
        page_ids
            .into_iter()
            .filter(|page_id| self.candidates.contains_key(page_id) && seen.insert(page_id.clone()))
            .take(limit)
            .collect()
    }

    fn candidate(&self, page_id: &str) -> Option<&Candidate> {
        self.candidates.get(page_id)
    }

    fn len(&self) -> usize {
        self.candidates.len()
    }
}

impl Candidate {
    fn intent_reason(&self) -> String {
        let sources = self.origins.iter().cloned().collect::<Vec<_>>().join(", ");
        if self.relation_leads.is_empty() {
            format!("Router selected after review of candidate evidence from {sources}.")
        } else {
            format!(
                "Router selected after review of {sources}; relation leads were inspected but not accepted automatically."
            )
        }
    }
}

fn effort_name(effort: IntentEffort) -> &'static str {
    match effort {
        IntentEffort::Low => "low",
        IntentEffort::Medium => "medium",
        IntentEffort::High => "high",
    }
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
                ensure!(
                    text.len().saturating_add(value.len()) <= MAX_ROUTER_OUTPUT_BYTES,
                    "PCP intent Router output exceeds {MAX_ROUTER_OUTPUT_BYTES} bytes"
                );
                text.push_str(value);
            }
        }
    }
    ensure!(
        !text.trim().is_empty(),
        "PCP intent Router response contains no output_text"
    );
    Ok(text)
}

fn deduplicate_strings(values: Vec<String>, limit: usize) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty() && seen.insert(value.to_lowercase()))
        .take(limit)
        .collect()
}

fn equivalent(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

fn truncate(value: &str, limit: usize) -> String {
    let value = value.trim();
    if value.chars().count() <= limit {
        return value.to_owned();
    }
    let mut output = value
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    output.push('…');
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effort_budgets_are_monotonic_and_bounded() {
        let low = IntentBudget::for_effort(IntentEffort::Low);
        let medium = IntentBudget::for_effort(IntentEffort::Medium);
        let high = IntentBudget::for_effort(IntentEffort::High);
        assert_eq!(low.expansion_round_limit, 0);
        assert!(medium.expansion_round_limit < high.expansion_round_limit);
        assert!(
            low.consult_limit < medium.consult_limit && medium.consult_limit < high.consult_limit
        );
        assert!(high.include_catalog);
    }

    #[test]
    fn router_outputs_are_strict_and_candidate_ids_are_fenced() {
        assert!(
            serde_json::from_str::<RouterPlan>(r#"{"probes":["边界"],"exactTerms":["OET"]}"#)
                .is_ok()
        );
        assert!(serde_json::from_str::<RouterPlan>(r#"{"unexpected":true}"#).is_err());
        let mut pool = CandidatePool::default();
        pool.add_catalog_hit(SearchHit {
            page_id: "pg_1".to_owned(),
            revision_id: "rev_1".to_owned(),
            kind: "note".to_owned(),
            mutability: pcp_core::PageMutability::Revisioned,
            namespace: "project:test".to_owned(),
            lifecycle_status: pcp_core::LifecycleStatus::Active,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            observed_at: None,
            snippet: "test".to_owned(),
            matched_by: "catalog".to_owned(),
            matched_projection: "summary".to_owned(),
            summary_revision_id: None,
            facets: None,
            validity: None,
            graph_edges: Vec::new(),
        });
        assert_eq!(
            pool.valid_page_ids(vec!["pg_missing".to_owned(), "pg_1".to_owned()], 2),
            vec!["pg_1"]
        );
    }

    #[test]
    fn router_usage_aggregates_only_provider_reported_tokens() {
        let response = ResponsesResult {
            id: "resp_usage".to_owned(),
            object: "response".to_owned(),
            created_at: 1,
            model: "reasoning.solve".to_owned(),
            status: "completed".to_owned(),
            output: Vec::new(),
            extra: BTreeMap::from([(
                "usage".to_owned(),
                json!({
                    "input_tokens": 100,
                    "output_tokens": 40,
                    "total_tokens": 140,
                    "input_tokens_details": {"cached_tokens": 25},
                    "output_tokens_details": {"reasoning_tokens": 18},
                }),
            )]),
        };
        let missing = ResponsesResult {
            id: "resp_missing".to_owned(),
            object: "response".to_owned(),
            created_at: 2,
            model: "reasoning.solve".to_owned(),
            status: "completed".to_owned(),
            output: Vec::new(),
            extra: BTreeMap::new(),
        };
        let mut usage = RouterTokenUsage::default();
        record_usage(&mut usage, &response);
        record_usage(&mut usage, &missing);
        assert_eq!(usage.reported_responses, 1);
        assert_eq!(usage.unreported_responses, 1);
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 40);
        assert_eq!(usage.total_tokens, 140);
        assert_eq!(usage.cached_input_tokens, 25);
        assert_eq!(usage.reasoning_tokens, 18);
    }
}
