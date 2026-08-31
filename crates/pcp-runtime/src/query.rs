use std::{collections::HashMap, sync::Arc, time::Instant};

use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use pcp_client::PcpTenantApi;
use pcp_core::{
    AccessDecision, ContextDetail, ContextPackEntry, GraphEdgeDirection, GraphEdgeKind,
    IntentEffort, Projection, QueryAuditEvent, QueryAuditMethod, QueryContextRequest,
    QueryContextResponse, QueryRelation, QueryVisibility, ReadPage, ReadPagesRequest,
    RuntimeUsageEvent, SearchFilters, SearchHit, SearchMode, SearchPagesRequest, SearchTermMatch,
};
use pcp_store::PcpStore;
use serde_json::Value;
use uuid::Uuid;

use pcp_rpc::RuntimeQueryService;

use crate::{IntentMatchConfig, SemanticSearchConfig};
use crate::{intent_match::IntentMatchProvider, semantic_search::SemanticSearchProvider};

const DEFAULT_TOP_K: u32 = 6;
const MAX_TOP_K: u32 = 20;
const DEFAULT_PACK_BUDGET_CHARS: u32 = 24_000;
const MIN_PACK_BUDGET_CHARS: u32 = 4_000;
const MAX_PACK_BUDGET_CHARS: u32 = 48_000;
const FOCUS_ANCHOR_COUNT: usize = 3;
const SEMANTIC_CANDIDATE_MULTIPLIER: usize = 3;
const MIN_SEMANTIC_CANDIDATES: usize = 12;
const MAX_SEMANTIC_CANDIDATES: usize = 36;
const STRUCTURAL_RERANK_SEED_COUNT: usize = 8;
const STRUCTURAL_RERANK_NEIGHBOR_LIMIT: u32 = 32;
const STRUCTURAL_RERANK_ELIGIBLE_CANDIDATES: usize = 12;
const MAX_STRUCTURAL_SEMANTIC_GAP: f32 = 0.08;
const MAX_STRUCTURAL_BOOST: f32 = 0.025;
const MAX_FOCUS_ENTRY_CHARS: usize = 4_000;
const MAX_SUMMARY_ENTRY_CHARS: usize = 1_600;
const MAX_RELATED_ENTRY_CHARS: usize = 1_600;
const MAX_QUERY_READ_CHARS: u32 = 64_000;
const PROJECTION_TRUNCATION_MARKER: &str = "[projection truncated by host budget]";

#[derive(Clone)]
struct PlannedHit {
    hit: SearchHit,
    anchor_rank: usize,
    relation: Option<QueryRelation>,
    semantic_score: Option<f32>,
    structural_boost: f32,
    structural_relations: Vec<QueryRelation>,
    intent_reason: Option<String>,
}

#[derive(Clone)]
struct RankedHit {
    hit: SearchHit,
    semantic_score: Option<f32>,
    structural_boost: f32,
    structural_relations: Vec<QueryRelation>,
    intent_reason: Option<String>,
}

type QueryPackEntry = ContextPackEntry;
type QueryDetail = ContextDetail;

#[derive(Clone, Copy)]
enum QueryMethod {
    SemanticSearch,
    MatchIntent,
}

/// Runtime-owned retrieval composition. It is deliberately separate from the
/// Store client: provider credentials, vector caches, and Router budgets are
/// runtime policy, while every Page read still runs through the caller's ACL.
pub struct QueryRuntime {
    semantic_search: Option<SemanticSearchProvider>,
    intent_match: Option<IntentMatchProvider>,
    audit_store: Arc<dyn PcpStore>,
}

impl QueryRuntime {
    pub fn from_config(
        audit_store: Arc<dyn PcpStore>,
        semantic_search: Option<SemanticSearchConfig>,
        intent_match: Option<IntentMatchConfig>,
    ) -> Result<Self> {
        Ok(Self {
            semantic_search: semantic_search
                .map(SemanticSearchProvider::new)
                .transpose()?,
            intent_match: intent_match.map(IntentMatchProvider::new).transpose()?,
            audit_store,
        })
    }
}

#[async_trait]
impl RuntimeQueryService for QueryRuntime {
    async fn semantic_search(
        &self,
        client: &dyn PcpTenantApi,
        request: QueryContextRequest,
    ) -> Result<QueryContextResponse> {
        execute(
            client,
            self.audit_store.as_ref(),
            self.semantic_search.as_ref(),
            self.intent_match.as_ref(),
            request,
            QueryMethod::SemanticSearch,
            IntentEffort::Medium,
        )
        .await
    }

    async fn match_intent(
        &self,
        client: &dyn PcpTenantApi,
        request: QueryContextRequest,
        effort: IntentEffort,
    ) -> Result<QueryContextResponse> {
        execute(
            client,
            self.audit_store.as_ref(),
            self.semantic_search.as_ref(),
            self.intent_match.as_ref(),
            request,
            QueryMethod::MatchIntent,
            effort,
        )
        .await
    }
}

async fn execute(
    client: &dyn PcpTenantApi,
    audit_store: &dyn PcpStore,
    semantic_search: Option<&SemanticSearchProvider>,
    intent_match: Option<&IntentMatchProvider>,
    request: QueryContextRequest,
    method: QueryMethod,
    intent_effort: IntentEffort,
) -> Result<QueryContextResponse> {
    let started = Instant::now();
    let audit_scopes = audit_scopes(client, &request.scopes);
    let result = execute_inner(
        client,
        semantic_search,
        intent_match,
        request,
        method,
        intent_effort,
    )
    .await;
    let event = query_audit_event(
        client,
        audit_scopes,
        method,
        intent_effort,
        started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        &result,
    );
    if let Err(error) = audit_store.record_runtime_query_audit(event.clone()).await {
        // Observability must never turn an otherwise usable context pack into
        // a failure. The Runtime still reports this local persistence problem.
        eprintln!("PCP query audit write failed: {error:#}");
    }
    let semantic_model_calls = result
        .as_ref()
        .ok()
        .and_then(|response| response.semantic_model_calls)
        .unwrap_or_default();
    let mut usage = event.router_usage.clone().unwrap_or_default();
    usage.unreported_responses = usage
        .unreported_responses
        .saturating_add(semantic_model_calls);
    if usage.response_count() > 0 {
        let usage_event = RuntimeUsageEvent {
            event_id: format!("ru_{}", Uuid::new_v4().simple()),
            occurred_at: event.occurred_at.clone(),
            principal: event.principal.clone(),
            session_id: event.session_id.clone(),
            source: "query".to_owned(),
            operation: match method {
                QueryMethod::SemanticSearch => "semantic_search",
                QueryMethod::MatchIntent => "match_intent",
            }
            .to_owned(),
            scopes: event.scopes.clone(),
            duration_ms: event.duration_ms,
            usage: Some(usage),
            failure_kind: event.failure_kind.clone(),
        };
        if let Err(error) = audit_store.record_runtime_usage(usage_event).await {
            eprintln!("PCP Runtime model usage write failed: {error:#}");
        }
    }
    result
}

async fn execute_inner(
    client: &dyn PcpTenantApi,
    semantic_search: Option<&SemanticSearchProvider>,
    intent_match: Option<&IntentMatchProvider>,
    request: QueryContextRequest,
    method: QueryMethod,
    intent_effort: IntentEffort,
) -> Result<QueryContextResponse> {
    let query = request.query.trim();
    ensure!(!query.is_empty(), "A query is required");
    let scopes = normalized_scopes(request.scopes);
    let top_k = request
        .result_limit
        .unwrap_or(DEFAULT_TOP_K)
        .clamp(1, MAX_TOP_K);
    let pack_budget_chars = request
        .context_budget_chars
        .unwrap_or(DEFAULT_PACK_BUDGET_CHARS)
        .clamp(MIN_PACK_BUDGET_CHARS, MAX_PACK_BUDGET_CHARS);
    let (
        anchors,
        semantic_indexed_count,
        semantic_embedded_count,
        semantic_model_calls,
        intent_audit,
    ) = match method {
        QueryMethod::SemanticSearch => {
            let provider = semantic_search.context(unavailable_reason(&method))?;
            let candidate_limit = semantic_candidate_limit(top_k);
            let result = provider
                .search(client, query, &scopes, candidate_limit)
                .await
                .context("semantic search is unavailable")?;
            let mut anchors = result
                .hits
                .into_iter()
                .map(|hit| RankedHit {
                    hit: hit.hit,
                    semantic_score: Some(hit.score),
                    structural_boost: 0.0,
                    structural_relations: Vec::new(),
                    intent_reason: None,
                })
                .collect::<Vec<_>>();
            rerank_semantic_with_relations(client, &mut anchors, &scopes).await?;
            anchors.truncate(top_k as usize);
            (
                anchors,
                Some(result.indexed_count),
                Some(result.embedded_count),
                Some(result.model_calls),
                None,
            )
        }
        QueryMethod::MatchIntent => {
            let semantic_search = semantic_search.context(unavailable_reason(&method))?;
            let intent_match = intent_match.context(unavailable_reason(&method))?;
            let result = intent_match
                .search(
                    client,
                    semantic_search,
                    query,
                    &scopes,
                    intent_effort,
                    top_k as usize,
                )
                .await
                .context("intent matching is unavailable")?;
            let anchors = result
                .hits
                .into_iter()
                .map(|hit| RankedHit {
                    hit: SearchHit {
                        matched_by: "intent_router".to_owned(),
                        matched_projection: "reviewed_page".to_owned(),
                        ..hit.hit
                    },
                    semantic_score: hit.semantic_score,
                    structural_boost: 0.0,
                    structural_relations: Vec::new(),
                    intent_reason: Some(hit.intent_reason),
                })
                .collect::<Vec<_>>();
            (
                anchors,
                Some(result.indexed_count),
                Some(result.embedded_count),
                Some(result.semantic_model_calls),
                Some(result.audit),
            )
        }
    };
    // Semantic search must remain a conservative page retriever. Graph
    // structure can only make a small, transparent adjustment between pages
    // that were independently retrieved above; it cannot introduce a graph
    // neighbor into the Context Pack. Intent matching owns that later decision.
    let anchor_count = anchors.len();
    let planned = plan_pack(&anchors);
    let pages = read_planned_pages(client, &planned).await?;
    let mut remaining_chars = pack_budget_chars as usize;
    let entries = planned
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let detail = pages.get(&candidate.hit.revision_id);
            pack_entry(index + 1, candidate, detail, &mut remaining_chars)
        })
        .collect::<Vec<_>>();
    Ok(QueryContextResponse {
        visibility: if scopes.is_empty() {
            QueryVisibility::AllAuthorized
        } else {
            QueryVisibility::Scoped
        },
        scopes,
        result_limit: top_k,
        context_budget_chars: pack_budget_chars,
        anchor_count,
        related_count: entries
            .iter()
            .filter(|entry| entry.relation.is_some())
            .count(),
        semantic_indexed_count,
        semantic_embedded_count,
        semantic_model_calls,
        intent_match: intent_audit,
        entries,
    })
}

fn audit_scopes(client: &dyn PcpTenantApi, requested_scopes: &[String]) -> Vec<String> {
    let mut scopes = if requested_scopes.is_empty() {
        client
            .access()
            .grants
            .iter()
            .map(|grant| grant.namespace.clone())
            .collect::<Vec<_>>()
    } else {
        requested_scopes.to_vec()
    };
    scopes.sort();
    scopes.dedup();
    scopes
}

fn query_audit_event(
    client: &dyn PcpTenantApi,
    scopes: Vec<String>,
    method: QueryMethod,
    intent_effort: IntentEffort,
    duration_ms: u64,
    result: &Result<QueryContextResponse>,
) -> QueryAuditEvent {
    let access = client.access();
    let (
        decision,
        anchor_count,
        related_count,
        context_chars,
        semantic_indexed_count,
        semantic_embedded_count,
        router_rounds,
        router_usage,
        failure_kind,
    ) = match result {
        Ok(response) => (
            AccessDecision::Allowed,
            response.anchor_count.try_into().unwrap_or(u64::MAX),
            response.related_count.try_into().unwrap_or(u64::MAX),
            response
                .entries
                .iter()
                .filter_map(|entry| entry.content.as_deref())
                .map(|content| content.chars().count() as u64)
                .sum(),
            response.semantic_indexed_count.map(|value| value as u64),
            response.semantic_embedded_count.map(|value| value as u64),
            response
                .intent_match
                .as_ref()
                .map(|audit| audit.router_rounds as u64),
            response
                .intent_match
                .as_ref()
                .map(|audit| audit.router_usage.clone()),
            None,
        ),
        Err(error) => (
            AccessDecision::Failed,
            0,
            0,
            0,
            None,
            None,
            None,
            None,
            Some(query_failure_kind(error)),
        ),
    };
    QueryAuditEvent {
        event_id: format!("qa_{}", Uuid::new_v4().simple()),
        occurred_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        principal: access.principal.clone(),
        session_id: access.session_id.clone(),
        method: match method {
            QueryMethod::SemanticSearch => QueryAuditMethod::SemanticSearch,
            QueryMethod::MatchIntent => QueryAuditMethod::MatchIntent,
        },
        effort: matches!(method, QueryMethod::MatchIntent).then_some(intent_effort),
        scopes,
        decision,
        duration_ms,
        anchor_count,
        related_count,
        context_chars,
        semantic_indexed_count,
        semantic_embedded_count,
        router_rounds,
        router_usage,
        failure_kind,
    }
}

fn query_failure_kind(error: &anyhow::Error) -> String {
    let message = error.to_string().to_ascii_lowercase();
    if message.contains("a query is required") {
        "invalid_request".to_owned()
    } else if message.contains("unavailable") {
        "provider_unavailable".to_owned()
    } else if message.contains("not authorized") || message.contains("permission") {
        "access_denied".to_owned()
    } else if message.contains("provider")
        || message.contains("embedding")
        || message.contains("router")
    {
        "provider_failed".to_owned()
    } else {
        "failed".to_owned()
    }
}

fn semantic_candidate_limit(top_k: u32) -> usize {
    ((top_k as usize) * SEMANTIC_CANDIDATE_MULTIPLIER)
        .clamp(MIN_SEMANTIC_CANDIDATES, MAX_SEMANTIC_CANDIDATES)
}

async fn read_planned_pages(
    client: &dyn PcpTenantApi,
    planned: &[PlannedHit],
) -> Result<HashMap<String, ReadPage>> {
    let mut pages = HashMap::with_capacity(planned.len());
    for candidate in planned {
        let read = client
            .read_pages(ReadPagesRequest {
                page_ids: Vec::new(),
                revision_ids: vec![candidate.hit.revision_id.clone()],
                projections: vec![
                    Projection::Manifest,
                    Projection::Payload,
                    Projection::Summary,
                    Projection::Sources,
                    Projection::Provenance,
                ],
                max_chars: MAX_QUERY_READ_CHARS,
            })
            .await?;
        if let Some(page) = read.into_iter().next() {
            pages.insert(page.revision.revision_id.clone(), page);
        }
    }
    Ok(pages)
}

/// Applies a deliberately bounded structural signal to semantic candidates.
///
/// A relation can only corroborate a page that semantic retrieval already
/// placed in the candidate pool; it never introduces a new anchor, follows
/// more than one hop, or uses provenance as if it were a semantic assertion.
async fn rerank_semantic_with_relations(
    client: &dyn PcpTenantApi,
    anchors: &mut Vec<RankedHit>,
    scopes: &[String],
) -> Result<()> {
    if anchors.len() < 2 {
        return Ok(());
    }
    let candidate_positions = anchors
        .iter()
        .enumerate()
        .map(|(index, candidate)| (candidate.hit.revision_id.clone(), index))
        .collect::<HashMap<_, _>>();
    let seeds = anchors
        .iter()
        .take(STRUCTURAL_RERANK_SEED_COUNT)
        .enumerate()
        .map(|(index, candidate)| {
            (
                index,
                candidate.hit.revision_id.clone(),
                candidate.semantic_score,
            )
        })
        .collect::<Vec<_>>();
    for (seed_index, revision_id, seed_score) in seeds {
        let graph = client
            .search_pages(SearchPagesRequest {
                query: revision_id,
                scopes: scopes.to_vec(),
                mode: SearchMode::Graph,
                term_match: SearchTermMatch::All,
                projections: vec![Projection::Summary, Projection::Payload],
                filters: SearchFilters::default(),
                limit: STRUCTURAL_RERANK_NEIGHBOR_LIMIT,
                cursor: None,
            })
            .await?;
        for neighbor in graph.hits {
            let Some(&candidate_index) = candidate_positions.get(&neighbor.revision_id) else {
                continue;
            };
            if candidate_index == seed_index {
                continue;
            }
            if !can_reinforce_semantic_candidate(
                seed_score,
                anchors[candidate_index].semantic_score,
                candidate_index,
            ) {
                continue;
            }
            let relations = neighbor
                .graph_edges
                .iter()
                .filter(|edge| edge.edge_kind == GraphEdgeKind::Relation)
                .filter_map(structural_relation)
                .collect::<Vec<_>>();
            let edge_weight = relations
                .iter()
                .map(|relation| structural_relation_weight(&relation.relation_type))
                .fold(0.0_f32, f32::max);
            if edge_weight == 0.0 {
                continue;
            }
            // Strong semantic seeds may corroborate a near-tie a little more,
            // but no seed can turn a relation into the primary retrieval signal.
            let semantic_confidence = seed_score.unwrap_or_default().clamp(0.0, 1.0);
            let boost = edge_weight * (0.75 + 0.25 * semantic_confidence);
            let candidate = &mut anchors[candidate_index];
            let remaining = (MAX_STRUCTURAL_BOOST - candidate.structural_boost).max(0.0);
            let applied = boost.min(remaining);
            if applied == 0.0 {
                continue;
            }
            candidate.structural_boost += applied;
            for relation in relations {
                if !candidate.structural_relations.iter().any(|existing| {
                    existing.relation_type == relation.relation_type
                        && existing.direction == relation.direction
                }) {
                    candidate.structural_relations.push(relation);
                }
            }
        }
    }
    anchors.sort_by(|left, right| {
        let left_score = left.semantic_score.unwrap_or_default() + left.structural_boost;
        let right_score = right.semantic_score.unwrap_or_default() + right.structural_boost;
        right_score.total_cmp(&left_score).then_with(|| {
            right
                .semantic_score
                .unwrap_or_default()
                .total_cmp(&left.semantic_score.unwrap_or_default())
        })
    });
    Ok(())
}

/// A graph relation is corroborating evidence, not a second retrieval system.
/// Both endpoints must be independently strong semantic candidates for this
/// query and must be close enough that a small structural tie-break is useful.
fn can_reinforce_semantic_candidate(
    seed_score: Option<f32>,
    candidate_score: Option<f32>,
    candidate_rank: usize,
) -> bool {
    candidate_rank < STRUCTURAL_RERANK_ELIGIBLE_CANDIDATES
        && seed_score.is_some_and(|seed| {
            candidate_score.is_some_and(|candidate| candidate >= seed - MAX_STRUCTURAL_SEMANTIC_GAP)
        })
}

fn structural_relation(edge: &pcp_core::GraphSearchEdge) -> Option<QueryRelation> {
    (edge.edge_kind == GraphEdgeKind::Relation).then(|| QueryRelation {
        relation_type: edge.relation_type.clone(),
        direction: graph_direction(&edge.direction).to_owned(),
    })
}

fn structural_relation_weight(relation_type: &str) -> f32 {
    match relation_type {
        "summarizes" => 0.025,
        "aggregates" | "depends_on" => 0.020,
        "derived_from" => 0.015,
        "references" | "cites" => 0.012,
        "related_to" => 0.008,
        _ => 0.010,
    }
}

fn plan_pack(anchors: &[RankedHit]) -> Vec<PlannedHit> {
    let mut planned = Vec::with_capacity(anchors.len());
    for (index, hit) in anchors.iter().enumerate() {
        let anchor_rank = index + 1;
        planned.push(PlannedHit {
            hit: hit.hit.clone(),
            anchor_rank,
            relation: None,
            semantic_score: hit.semantic_score,
            structural_boost: hit.structural_boost,
            structural_relations: hit.structural_relations.clone(),
            intent_reason: hit.intent_reason.clone(),
        });
    }
    planned
}

fn graph_direction(direction: &GraphEdgeDirection) -> &'static str {
    match direction {
        GraphEdgeDirection::Incoming => "incoming",
        GraphEdgeDirection::Outgoing => "outgoing",
    }
}

fn normalized_scopes(scopes: Vec<String>) -> Vec<String> {
    let mut normalized = scopes
        .into_iter()
        .map(|scope| scope.trim().to_owned())
        .filter(|scope| !scope.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn unavailable_reason(method: &QueryMethod) -> &'static str {
    match method {
        QueryMethod::SemanticSearch => {
            "semantic_search is unavailable: configure a vector retrieval provider for this Runtime."
        }
        QueryMethod::MatchIntent => {
            "match_intent is unavailable: configure a query intent Router for this Runtime."
        }
    }
}

fn pack_entry(
    rank: usize,
    candidate: &PlannedHit,
    page: Option<&ReadPage>,
    remaining_chars: &mut usize,
) -> QueryPackEntry {
    let hit = &candidate.hit;
    let (detail, content) = if candidate.relation.is_some() {
        related_context(page, hit, remaining_chars)
    } else if candidate.anchor_rank <= FOCUS_ANCHOR_COUNT {
        anchor_context(page, hit, remaining_chars)
    } else {
        current_summary(page, hit, remaining_chars)
            .map(|content| (QueryDetail::Summary, Some(content)))
            .unwrap_or((QueryDetail::Reference, None))
    };
    let source_span = page.and_then(|page| page.revision.source_span.clone());
    let provenance_revision_ids = page
        .map(|page| {
            page.revision
                .provenance
                .iter()
                .flat_map(|event| event.input_revision_ids.iter().cloned())
                .collect()
        })
        .unwrap_or_default();
    QueryPackEntry {
        rank,
        anchor_rank: candidate.anchor_rank,
        page_id: hit.page_id.clone(),
        revision_id: hit.revision_id.clone(),
        namespace: hit.namespace.clone(),
        kind: hit.kind.clone(),
        matched_by: hit.matched_by.clone(),
        matched_projection: hit.matched_projection.clone(),
        semantic_score: candidate.semantic_score,
        structural_boost: (candidate.structural_boost > 0.0).then_some(candidate.structural_boost),
        structural_relations: candidate.structural_relations.clone(),
        intent_reason: candidate.intent_reason.clone(),
        detail,
        relation: candidate.relation.clone(),
        source_projection_truncated: page_has_truncated_projection(page),
        content,
        source_span,
        provenance_revision_ids,
        validity: hit.validity.clone(),
    }
}

fn anchor_context(
    page: Option<&ReadPage>,
    hit: &SearchHit,
    remaining_chars: &mut usize,
) -> (QueryDetail, Option<String>) {
    if payload_projection_truncated(page) || model_payload(page).is_none() {
        if let Some(content) = current_summary(page, hit, remaining_chars) {
            return (QueryDetail::Summary, Some(content));
        }
    }
    payload_or_snippet(page, hit, remaining_chars, MAX_FOCUS_ENTRY_CHARS)
}

fn payload_or_snippet(
    page: Option<&ReadPage>,
    hit: &SearchHit,
    remaining_chars: &mut usize,
    entry_limit: usize,
) -> (QueryDetail, Option<String>) {
    let source = model_payload(page).or_else(|| model_search_snippet(hit));
    let limit = (*remaining_chars).min(entry_limit);
    let Some(source) = source else {
        return (QueryDetail::Reference, None);
    };
    if limit == 0 || source.trim().is_empty() {
        return (QueryDetail::Reference, None);
    }
    let (content, truncated) = truncate_chars(&source, limit);
    *remaining_chars = remaining_chars.saturating_sub(content.chars().count());
    (
        if truncated {
            QueryDetail::Excerpt
        } else {
            QueryDetail::Payload
        },
        Some(content),
    )
}

fn related_context(
    page: Option<&ReadPage>,
    hit: &SearchHit,
    remaining_chars: &mut usize,
) -> (QueryDetail, Option<String>) {
    current_summary(page, hit, remaining_chars)
        .map(|content| (QueryDetail::Summary, Some(content)))
        .unwrap_or_else(|| payload_or_snippet(page, hit, remaining_chars, MAX_RELATED_ENTRY_CHARS))
}

fn current_summary(
    page: Option<&ReadPage>,
    hit: &SearchHit,
    remaining_chars: &mut usize,
) -> Option<String> {
    let summary = page?.summary.as_ref()?;
    if summary.target_revision_id != hit.revision_id
        || summary.content.trim().is_empty()
        || projection_was_truncated(&summary.content)
    {
        return None;
    }
    let limit = (*remaining_chars).min(MAX_SUMMARY_ENTRY_CHARS);
    if limit == 0 {
        return None;
    }
    let (content, _) = truncate_chars(&summary.content, limit);
    *remaining_chars = remaining_chars.saturating_sub(content.chars().count());
    Some(content)
}

pub(crate) fn truncate_chars(value: &str, limit: usize) -> (String, bool) {
    let value = value.trim();
    if value.chars().count() <= limit {
        return (value.to_owned(), false);
    }
    if limit <= 1 {
        return ("…".to_owned(), true);
    }
    let mut output = value.chars().take(limit - 1).collect::<String>();
    output.push('…');
    (output, true)
}

fn page_has_truncated_projection(page: Option<&ReadPage>) -> bool {
    payload_projection_truncated(page)
        || page
            .and_then(|page| page.summary.as_ref())
            .is_some_and(|summary| projection_was_truncated(&summary.content))
}

fn payload_projection_truncated(page: Option<&ReadPage>) -> bool {
    page.and_then(|page| page.revision.payload.as_ref())
        .is_some_and(|payload| projection_was_truncated(&payload.content))
}

pub(crate) fn projection_was_truncated(content: &str) -> bool {
    content.trim_end().ends_with(PROJECTION_TRUNCATION_MARKER)
}

fn model_payload(page: Option<&ReadPage>) -> Option<String> {
    let page = page?;
    let payload = page.revision.payload.as_ref()?;
    if projection_was_truncated(&payload.content) {
        return None;
    }
    model_projection(&payload.media_type, &payload.content)
}

fn model_search_snippet(hit: &SearchHit) -> Option<String> {
    model_projection("text/plain", &hit.snippet)
}

pub(crate) fn model_projection(media_type: &str, content: &str) -> Option<String> {
    let content = content.trim();
    if content.is_empty() || projection_was_truncated(content) {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(content) {
        return conversation_bundle_projection(&value);
    }
    if media_type.eq_ignore_ascii_case("application/json")
        || content.starts_with('{')
        || content.starts_with('[')
    {
        return None;
    }
    Some(content.to_owned())
}

fn conversation_bundle_projection(value: &Value) -> Option<String> {
    let entries = value.get("entries")?.as_array()?;
    let messages = entries
        .iter()
        .filter_map(conversation_message_projection)
        .collect::<Vec<_>>();
    (!messages.is_empty()).then(|| messages.join("\n\n"))
}

fn conversation_message_projection(entry: &Value) -> Option<String> {
    let content = entry.pointer("/payload/content")?.as_str()?.trim();
    if content.is_empty() || projection_was_truncated(content) {
        return None;
    }
    let role = entry
        .pointer("/facets/role")
        .and_then(Value::as_str)
        .or_else(|| entry.get("role").and_then(Value::as_str))
        .or_else(|| entry.pointer("/actor/actorType").and_then(Value::as_str))
        .unwrap_or("message");
    Some(format!("{}:\n{}", display_conversation_role(role), content))
}

fn display_conversation_role(role: &str) -> &'static str {
    match role.trim().to_ascii_lowercase().as_str() {
        "user" => "User",
        "assistant" | "model" => "Assistant",
        "system" => "System",
        "tool" => "Tool",
        _ => "Message",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_preserves_character_budget() {
        assert_eq!(truncate_chars("abcdef", 4), ("abc…".to_owned(), true));
        assert_eq!(truncate_chars("abc", 4), ("abc".to_owned(), false));
    }

    #[test]
    fn unavailable_query_methods_fail_closed() {
        assert!(unavailable_reason(&QueryMethod::SemanticSearch).contains("unavailable"));
        assert!(unavailable_reason(&QueryMethod::MatchIntent).contains("unavailable"));
    }

    #[test]
    fn query_request_has_no_method_discriminator() {
        serde_json::from_str::<QueryContextRequest>(r#"{"query":"OET 是什么"}"#)
            .expect("decode query request");
        assert!(
            serde_json::from_str::<QueryContextRequest>(
                r#"{"query":"OET 是什么","method":"semantic_search"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn semantic_structural_weights_are_small_and_type_aware() {
        assert_eq!(semantic_candidate_limit(1), MIN_SEMANTIC_CANDIDATES);
        assert_eq!(semantic_candidate_limit(12), MAX_SEMANTIC_CANDIDATES);
        assert_eq!(semantic_candidate_limit(MAX_TOP_K), MAX_SEMANTIC_CANDIDATES);
        assert!(
            structural_relation_weight("summarizes") > structural_relation_weight("related_to")
        );
        assert!(structural_relation_weight("related_to") < MAX_STRUCTURAL_BOOST);
    }

    #[test]
    fn structure_only_reinforces_nearby_strong_semantic_candidates() {
        assert!(can_reinforce_semantic_candidate(Some(0.72), Some(0.66), 11));
        assert!(!can_reinforce_semantic_candidate(
            Some(0.72),
            Some(0.63),
            11
        ));
        assert!(!can_reinforce_semantic_candidate(
            Some(0.72),
            Some(0.70),
            12
        ));
        assert!(!can_reinforce_semantic_candidate(Some(0.72), None, 1));
    }

    #[test]
    fn conversation_bundle_projects_messages_without_storage_envelope() {
        let source = r#"{
          "entries": [
            {
              "createdAt": "2026-08-01T23:46:09Z",
              "facets": {"role": "user"},
              "pageId": "pg_hidden",
              "payload": {"content": "What is OET?"}
            },
            {
              "actor": {"actorType": "model"},
              "provenance": [{"operation": "ingest"}],
              "payload": {"content": "OET is a formal theory runtime."}
            }
          ]
        }"#;

        let projection =
            model_projection("text/markdown", source).expect("conversation projection");
        assert_eq!(
            projection,
            "User:\nWhat is OET?\n\nAssistant:\nOET is a formal theory runtime."
        );
        assert!(!projection.contains("createdAt"));
        assert!(!projection.contains("pg_hidden"));
        assert!(!projection.contains("provenance"));
    }

    #[test]
    fn unknown_json_is_not_a_model_projection() {
        assert_eq!(
            model_projection("application/json", r#"{"internal": "record"}"#),
            None
        );
        assert_eq!(
            model_projection("text/markdown", r#"{"internal": "record"}"#),
            None
        );
    }
}
