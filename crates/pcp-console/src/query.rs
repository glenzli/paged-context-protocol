use std::collections::{HashMap, HashSet};

use anyhow::{Result, ensure};
use pcp_client::PcpTenantApi;
use pcp_core::{
    GraphEdgeDirection, GraphEdgeKind, Projection, ReadPage, ReadPagesRequest, SearchFilters,
    SearchHit, SearchMode, SearchPagesRequest, SearchTermMatch, SourceSpan,
};
use pcp_rpc::RemotePcpClient;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const DEFAULT_TOP_K: u32 = 12;
const MAX_TOP_K: u32 = 20;
const DEFAULT_PACK_BUDGET_CHARS: u32 = 24_000;
const MIN_PACK_BUDGET_CHARS: u32 = 4_000;
const MAX_PACK_BUDGET_CHARS: u32 = 48_000;
const FOCUS_ANCHOR_COUNT: usize = 3;
const GRAPH_SEED_COUNT: usize = 4;
const MAX_RELATED_CONTEXTS: usize = 4;
const MAX_FOCUS_ENTRY_CHARS: usize = 4_000;
const MAX_SUMMARY_ENTRY_CHARS: usize = 1_600;
const MAX_RELATED_ENTRY_CHARS: usize = 1_600;
const MAX_QUERY_READ_CHARS: u32 = 64_000;
const PROJECTION_TRUNCATION_MARKER: &str = "[projection truncated by host budget]";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryMethod {
    #[default]
    Search,
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
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryDetail {
    Payload,
    Excerpt,
    Summary,
    Reference,
}

pub fn capabilities() -> QueryCapabilities {
    QueryCapabilities {
        available_methods: vec![QueryMethod::Search],
        unavailable_methods: vec![
            UnavailableQueryMethod {
                method: QueryMethod::SemanticSearch,
                reason: "No vector retrieval provider is configured for this Runtime.",
            },
            UnavailableQueryMethod {
                method: QueryMethod::MatchIntent,
                reason: "No query intent router is configured for this Runtime.",
            },
        ],
    }
}

pub async fn execute(client: &RemotePcpClient, request: QueryRequest) -> Result<QueryResponse> {
    ensure!(
        matches!(request.method, QueryMethod::Search),
        unavailable_reason(&request.method)
    );
    let query = request.query.trim();
    ensure!(!query.is_empty(), "A query is required");
    let scope = request.scope.and_then(normalized_scope);
    let top_k = request.top_k.unwrap_or(DEFAULT_TOP_K).clamp(1, MAX_TOP_K);
    let pack_budget_chars = request
        .pack_budget_chars
        .unwrap_or(DEFAULT_PACK_BUDGET_CHARS)
        .clamp(MIN_PACK_BUDGET_CHARS, MAX_PACK_BUDGET_CHARS);
    let scopes = scope.iter().cloned().collect::<Vec<_>>();
    let anchors = client
        .search_pages(SearchPagesRequest {
            query: query.to_owned(),
            scopes: scopes.clone(),
            mode: SearchMode::Text,
            term_match: SearchTermMatch::All,
            projections: vec![Projection::Summary, Projection::Payload],
            filters: SearchFilters::default(),
            limit: top_k,
            cursor: None,
        })
        .await?;
    let relation_budget = ((top_k as usize) / 3).min(MAX_RELATED_CONTEXTS);
    let related = relation_candidates(client, &anchors.hits, &scopes, relation_budget).await?;
    let anchor_count = anchors
        .hits
        .len()
        .min((top_k as usize).saturating_sub(related.len()));
    let related = related
        .into_iter()
        .filter(|candidate| candidate.anchor_rank <= anchor_count)
        .collect::<Vec<_>>();
    let planned = plan_pack(&anchors.hits, related, anchor_count);
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
        model_context: model_context(&entries),
        entries,
    })
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

async fn relation_candidates(
    client: &RemotePcpClient,
    anchors: &[SearchHit],
    scopes: &[String],
    limit: usize,
) -> Result<Vec<PlannedHit>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let anchor_revisions = anchors
        .iter()
        .map(|hit| hit.revision_id.clone())
        .collect::<HashSet<_>>();
    let mut related_revisions = HashSet::new();
    let mut related = Vec::new();
    for (index, anchor) in anchors.iter().take(GRAPH_SEED_COUNT).enumerate() {
        if related.len() >= limit {
            break;
        }
        let graph = client
            .search_pages(SearchPagesRequest {
                query: anchor.revision_id.clone(),
                scopes: scopes.to_vec(),
                mode: SearchMode::Graph,
                term_match: SearchTermMatch::All,
                projections: vec![Projection::Summary, Projection::Payload],
                filters: SearchFilters::default(),
                limit: 2,
                cursor: None,
            })
            .await?;
        for hit in graph.hits {
            if anchor_revisions.contains(&hit.revision_id)
                || !related_revisions.insert(hit.revision_id.clone())
            {
                continue;
            }
            let relation = hit
                .graph_edges
                .iter()
                .find(|edge| edge.edge_kind == GraphEdgeKind::Relation)
                .map(|edge| QueryRelation {
                    relation_type: edge.relation_type.clone(),
                    direction: graph_direction(&edge.direction),
                });
            let Some(relation) = relation else {
                continue;
            };
            related.push(PlannedHit {
                hit,
                anchor_rank: index + 1,
                relation: Some(relation),
            });
            break;
        }
    }
    Ok(related)
}

fn plan_pack(
    anchors: &[SearchHit],
    related: Vec<PlannedHit>,
    anchor_count: usize,
) -> Vec<PlannedHit> {
    let mut planned = Vec::with_capacity(anchor_count + related.len());
    for (index, hit) in anchors.iter().take(anchor_count).enumerate() {
        let anchor_rank = index + 1;
        planned.push(PlannedHit {
            hit: hit.clone(),
            anchor_rank,
            relation: None,
        });
        planned.extend(
            related
                .iter()
                .filter(|candidate| candidate.anchor_rank == anchor_rank)
                .cloned(),
        );
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
        QueryMethod::Search => "",
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

fn truncate_chars(value: &str, limit: usize) -> (String, bool) {
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

fn projection_was_truncated(content: &str) -> bool {
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

fn model_projection(media_type: &str, content: &str) -> Option<String> {
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
