use std::collections::HashMap;

use anyhow::{Context, Result, ensure};
use pcp_client::PcpTenantApi;
use pcp_core::{
    GraphEdgeDirection, GraphEdgeKind, Projection, ReadPage, ReadPagesRequest, SearchFilters,
    SearchHit, SearchMode, SearchPagesRequest, SearchTermMatch, SourceSpan,
};
use pcp_rpc::RemotePcpClient;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::intent_match::{IntentEffort, IntentMatchAudit, IntentMatchProvider};

pub use crate::semantic_search::SemanticSearchProvider;

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

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryMethod {
    #[default]
    SemanticSearch,
    MatchIntent,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryRequest {
    pub query: String,
    #[serde(default)]
    pub method: QueryMethod,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub top_k: Option<u32>,
    #[serde(default)]
    pub pack_budget_chars: Option<u32>,
    #[serde(default)]
    pub intent_effort: IntentEffort,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryCapabilities {
    pub available_methods: Vec<QueryMethod>,
    pub unavailable_methods: Vec<UnavailableQueryMethod>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnavailableQueryMethod {
    pub method: QueryMethod,
    pub reason: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResponse {
    pub method: QueryMethod,
    pub scope: Option<String>,
    pub visibility: &'static str,
    pub top_k: u32,
    pub pack_budget_chars: u32,
    pub anchor_count: usize,
    pub related_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_indexed_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_embedded_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent_match: Option<IntentMatchAudit>,
    pub entries: Vec<QueryPackEntry>,
    pub model_context: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryPackEntry {
    pub rank: usize,
    pub anchor_rank: usize,
    pub page_id: String,
    pub revision_id: String,
    pub namespace: String,
    pub kind: String,
    pub matched_by: String,
    pub matched_projection: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_score: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structural_boost: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub structural_relations: Vec<QueryRelation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent_reason: Option<String>,
    pub detail: QueryDetail,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<QueryRelation>,
    pub source_projection_truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provenance_revision_ids: Vec<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryRelation {
    pub relation_type: String,
    pub direction: &'static str,
}

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

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryDetail {
    Payload,
    Excerpt,
    Summary,
    Reference,
}

pub fn capabilities(
    semantic_search: Option<&SemanticSearchProvider>,
    intent_match: Option<&IntentMatchProvider>,
) -> QueryCapabilities {
    let mut available_methods = Vec::new();
    let mut unavailable_methods = Vec::new();
    if semantic_search.is_some() {
        available_methods.push(QueryMethod::SemanticSearch);
    } else {
        unavailable_methods.push(UnavailableQueryMethod {
            method: QueryMethod::SemanticSearch,
            reason: "No vector retrieval provider is configured for this Runtime.",
        });
    }
    if semantic_search.is_some() && intent_match.is_some() {
        available_methods.push(QueryMethod::MatchIntent);
    } else {
        unavailable_methods.push(UnavailableQueryMethod {
            method: QueryMethod::MatchIntent,
            reason: "Intent matching is unavailable: it requires both semantic retrieval and a configured query intent Router.",
        });
    }
    QueryCapabilities {
        available_methods,
        unavailable_methods,
    }
}

pub async fn execute(
    client: &RemotePcpClient,
    semantic_search: Option<&SemanticSearchProvider>,
    intent_match: Option<&IntentMatchProvider>,
    request: QueryRequest,
) -> Result<QueryResponse> {
    let query = request.query.trim();
    ensure!(!query.is_empty(), "A query is required");
    let scope = request.scope.and_then(normalized_scope);
    let top_k = request.top_k.unwrap_or(DEFAULT_TOP_K).clamp(1, MAX_TOP_K);
    let pack_budget_chars = request
        .pack_budget_chars
        .unwrap_or(DEFAULT_PACK_BUDGET_CHARS)
        .clamp(MIN_PACK_BUDGET_CHARS, MAX_PACK_BUDGET_CHARS);
    let scopes = scope.iter().cloned().collect::<Vec<_>>();
    let (anchors, semantic_indexed_count, semantic_embedded_count, intent_audit) = match request
        .method
    {
        QueryMethod::SemanticSearch => {
            let provider = semantic_search.context(unavailable_reason(&request.method))?;
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
                None,
            )
        }
        QueryMethod::MatchIntent => {
            let semantic_search = semantic_search.context(unavailable_reason(&request.method))?;
            let intent_match = intent_match.context(unavailable_reason(&request.method))?;
            let result = intent_match
                .search(
                    client,
                    semantic_search,
                    query,
                    &scopes,
                    request.intent_effort,
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
    Ok(QueryResponse {
        method: request.method,
        visibility: if scope.is_some() {
            "scoped"
        } else {
            "all_authorized"
        },
        scope,
        top_k,
        pack_budget_chars,
        anchor_count,
        related_count: entries
            .iter()
            .filter(|entry| entry.relation.is_some())
            .count(),
        semantic_indexed_count,
        semantic_embedded_count,
        intent_match: intent_audit,
        model_context: model_context(&entries),
        entries,
    })
}

fn semantic_candidate_limit(top_k: u32) -> usize {
    ((top_k as usize) * SEMANTIC_CANDIDATE_MULTIPLIER)
        .clamp(MIN_SEMANTIC_CANDIDATES, MAX_SEMANTIC_CANDIDATES)
}

async fn read_planned_pages(
    client: &RemotePcpClient,
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
    client: &RemotePcpClient,
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
        direction: graph_direction(&edge.direction),
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

fn normalized_scope(scope: String) -> Option<String> {
    let scope = scope.trim();
    (!scope.is_empty()).then(|| scope.to_owned())
}

fn unavailable_reason(method: &QueryMethod) -> &'static str {
    match method {
        QueryMethod::SemanticSearch => {
            "Semantic search is unavailable: no vector retrieval provider is configured."
        }
        QueryMethod::MatchIntent => {
            "Intent matching is unavailable: no query intent router is configured."
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

fn model_context(entries: &[QueryPackEntry]) -> String {
    let mut blocks = Vec::new();
    for entry in entries {
        let Some(content) = entry.content.as_deref() else {
            continue;
        };
        if projection_was_truncated(content) {
            continue;
        }
        let role = match &entry.relation {
            Some(relation) => format!(
                "related to anchor #{} via {} ({})",
                entry.anchor_rank, relation.relation_type, relation.direction
            ),
            None => format!("anchor #{}", entry.anchor_rank),
        };
        let context_rank = blocks.len() + 1;
        blocks.push(format!(
            "[Context {} | {} | {} | {} | {}]\n{}\n[/Context]",
            context_rank,
            role,
            detail_name(entry.detail),
            entry.kind,
            entry.namespace,
            content
        ));
    }
    blocks.join("\n\n")
}

fn detail_name(detail: QueryDetail) -> &'static str {
    match detail {
        QueryDetail::Payload => "payload",
        QueryDetail::Excerpt => "excerpt",
        QueryDetail::Summary => "summary",
        QueryDetail::Reference => "reference",
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
    fn query_defaults_to_semantic_retrieval_and_rejects_literal_search() {
        let default_request = serde_json::from_str::<QueryRequest>(r#"{"query":"OET 是什么"}"#)
            .expect("decode default query request");
        assert!(matches!(
            default_request.method,
            QueryMethod::SemanticSearch
        ));
        assert!(
            serde_json::from_str::<QueryRequest>(r#"{"query":"OET 是什么","method":"search"}"#)
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
    fn model_context_includes_related_content_without_internal_page_ids() {
        let related = QueryPackEntry {
            rank: 2,
            anchor_rank: 1,
            page_id: "pg_internal".to_owned(),
            revision_id: "rev_internal".to_owned(),
            namespace: "conversation:example".to_owned(),
            kind: "note".to_owned(),
            matched_by: "graph".to_owned(),
            matched_projection: "relations".to_owned(),
            semantic_score: None,
            structural_boost: None,
            structural_relations: Vec::new(),
            intent_reason: None,
            detail: QueryDetail::Summary,
            relation: Some(QueryRelation {
                relation_type: "depends_on".to_owned(),
                direction: "outgoing",
            }),
            source_projection_truncated: false,
            content: Some("required background".to_owned()),
            source_span: None,
            provenance_revision_ids: Vec::new(),
        };
        let reference = QueryPackEntry {
            content: None,
            detail: QueryDetail::Reference,
            relation: None,
            ..related.clone()
        };
        let later = QueryPackEntry {
            rank: 11,
            content: Some("later evidence".to_owned()),
            relation: None,
            ..related.clone()
        };

        let context = model_context(&[related, reference, later]);
        assert!(context.starts_with("[Context 1 |"));
        assert!(context.contains("[Context 2 |"));
        assert!(!context.contains("[Context 11 |"));
        assert!(context.contains("related to anchor #1 via depends_on (outgoing)"));
        assert!(context.contains("required background"));
        assert!(!context.contains("pg_internal"));
        assert!(!context.contains("rev_internal"));
    }

    #[test]
    fn model_context_rejects_host_truncation_marker() {
        let truncated = QueryPackEntry {
            rank: 1,
            anchor_rank: 1,
            page_id: "pg_internal".to_owned(),
            revision_id: "rev_internal".to_owned(),
            namespace: "conversation:example".to_owned(),
            kind: "note".to_owned(),
            matched_by: "text".to_owned(),
            matched_projection: "payload".to_owned(),
            semantic_score: None,
            structural_boost: None,
            structural_relations: Vec::new(),
            intent_reason: None,
            detail: QueryDetail::Excerpt,
            relation: None,
            source_projection_truncated: true,
            content: Some(format!("partial\n{PROJECTION_TRUNCATION_MARKER}")),
            source_span: None,
            provenance_revision_ids: Vec::new(),
        };

        let context = model_context(&[truncated]);
        assert!(context.is_empty());
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
