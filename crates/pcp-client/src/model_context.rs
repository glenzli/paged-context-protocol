//! Model-facing projections of already-authorized API results.
//!
//! These functions perform no I/O, inference, authorization or persistence. Keep
//! the original API result for application diagnostics; send this projection to
//! a model instead. Character budgets apply to evidence text, not identifiers.
#![doc = include_str!("../../../integrations/TOOL_INTEGRATION.md")]

use pcp_core::{
    ContextDetail, GraphSliceEdge, GraphSliceResponse, PageValidityHint, Projection,
    QueryContextResponse, ReadPage, SearchResult, SourceRef, SourceSpan,
};
use serde::Serialize;
use serde_json::{Value, json};

const STORE_TRUNCATION: &str = "[projection truncated by host budget]";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ContextView {
    #[default]
    Content,
    Context,
    Sources,
    History,
    Full,
}

impl ContextView {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "content" => Ok(Self::Content),
            "context" => Ok(Self::Context),
            "sources" => Ok(Self::Sources),
            "history" => Ok(Self::History),
            "full" => Ok(Self::Full),
            _ => Err("view must be content, context, sources, history, or full"),
        }
    }

    /// Validity is never optional when reading evidence for a model.
    pub fn projections(self) -> Vec<Projection> {
        let mut result = vec![Projection::Manifest, Projection::Validity];
        if matches!(self, Self::Content | Self::Context | Self::Full) {
            result.push(Projection::Payload);
        }
        if matches!(self, Self::Context | Self::Full) {
            result.push(Projection::Relations);
        }
        if matches!(self, Self::Sources | Self::Full) {
            result.extend([Projection::Sources, Projection::Provenance]);
        }
        if matches!(self, Self::History | Self::Full) {
            result.push(Projection::History);
        }
        result
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelContext {
    pub items: Vec<ContextItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Some evidence text, graph coverage or projection is incomplete.
    pub truncated: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<GraphSliceEdge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped_reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextItem {
    pub page_id: String,
    pub revision_id: String,
    pub scope: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// payload, summary, excerpt or reference; a summary is not the original text.
    pub detail: String,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_status: Option<String>,
    /// Present when the requested snapshot is not the current Page head.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_revision_id: Option<String>,
    /// Absent means no assessment was supplied, never "verified true".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validity: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_revision_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<SourceRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpan>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub basis_revision_ids: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub relations: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_page_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub assessment_revision_ids: Vec<String>,
}

impl ContextItem {
    fn new(page_id: &str, revision_id: &str, scope: &str, kind: &str) -> Self {
        Self {
            page_id: page_id.into(),
            revision_id: revision_id.into(),
            scope: scope.into(),
            kind: kind.into(),
            media_type: None,
            content: None,
            detail: "reference".into(),
            truncated: false,
            observed_at: None,
            valid_from: None,
            valid_to: None,
            page_status: None,
            revision_status: None,
            current_revision_id: None,
            validity: None,
            summary_revision_id: None,
            source_refs: vec![],
            source_span: None,
            basis_revision_ids: vec![],
            relations: vec![],
            anchor_page_id: None,
            history: vec![],
            assessment_revision_ids: vec![],
        }
    }
}

/// A per-response evidence budget and per-hit search preview limit, in Unicode
/// scalar values. Never shorten identifiers, effective dates or validity caveats.
#[derive(Clone, Copy, Debug)]
pub struct ContextBudget {
    pub content_chars: usize,
    pub preview_chars: usize,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            content_chars: 8_000,
            preview_chars: 400,
        }
    }
}

fn text(value: &str, remaining: &mut usize, cap: usize) -> (String, bool) {
    let limit = (*remaining).min(cap);
    let count = value.chars().count();
    let retained: String = value.chars().take(limit).collect();
    *remaining = remaining.saturating_sub(count.min(limit));
    (retained, count > limit || value.contains(STORE_TRUNCATION))
}

fn validity(hint: &PageValidityHint) -> Value {
    json!({"standing": hint.standing, "scope": hint.scope,
        "rationale": hint.rationale, "assessmentRevisionId": hint.assessment_revision_id})
}

fn finish(mut items: Vec<ContextItem>, next_cursor: Option<String>) -> ModelContext {
    for item in &mut items {
        item.truncated |= item.validity.as_ref().is_some_and(|v| {
            v["rationale"]
                .as_str()
                .is_some_and(|s| s.contains(STORE_TRUNCATION))
        });
    }
    ModelContext {
        truncated: items.iter().any(|item| item.truncated),
        items,
        next_cursor,
        edges: vec![],
        stopped_reason: None,
    }
}

pub fn search_context(result: &SearchResult, budget: ContextBudget) -> ModelContext {
    let mut remaining = budget.content_chars;
    let items = result
        .hits
        .iter()
        .map(|hit| {
            let mut item =
                ContextItem::new(&hit.page_id, &hit.revision_id, &hit.namespace, &hit.kind);
            let (snippet, truncated) = text(&hit.snippet, &mut remaining, budget.preview_chars);
            item.content = Some(snippet);
            item.detail = "excerpt".into();
            item.truncated = truncated;
            item.observed_at = hit.observed_at.clone();
            item.page_status = Some(hit.lifecycle_status.as_str().into());
            item.validity = hit.validity.as_ref().map(validity);
            item.summary_revision_id = hit.summary_revision_id.clone();
            // A graph match must retain direction and relation/provenance distinction.
            item.relations = hit.graph_edges.iter().map(|edge| json!(edge)).collect();
            item
        })
        .collect();
    finish(items, result.next_cursor.clone())
}

pub fn read_context(pages: &[ReadPage], view: ContextView, budget: ContextBudget) -> ModelContext {
    let mut remaining = budget.content_chars;
    let items = pages.iter().map(|read| {
        let revision = &read.revision;
        let mut item = ContextItem::new(&read.page.page_id, &revision.revision_id, &revision.namespace, &read.page.kind);
        item.page_status = Some(read.page.lifecycle_status.as_str().into());
        item.revision_status = Some(revision.lifecycle_status.as_str().into());
        item.current_revision_id = (read.page.head_revision_id != revision.revision_id)
            .then(|| read.page.head_revision_id.clone());
        item.observed_at = revision.observed_at.clone();
        item.valid_from = revision.valid_from.clone();
        item.valid_to = revision.valid_to.clone();
        item.validity = read.validity.as_ref().map(|assessment| json!({
            "standing": assessment.standing, "scope": assessment.scope,
            "rationale": assessment.rationale, "assessmentRevisionId": assessment.assessment_revision_id
        }));
        if matches!(view, ContextView::Content | ContextView::Context | ContextView::Full) {
            if let Some(payload) = &revision.payload {
                item.media_type = Some(payload.media_type.clone());
                let (content, truncated) = text(&payload.content, &mut remaining, usize::MAX);
                item.content = Some(content); item.truncated = truncated; item.detail = "payload".into();
            } else if let Some(summary) = &read.summary {
                let (content, truncated) = text(&summary.content, &mut remaining, usize::MAX);
                item.content = Some(content); item.truncated = truncated; item.detail = "summary".into();
                item.summary_revision_id = Some(summary.summary_revision_id.clone());
            }
        }
        if matches!(view, ContextView::Context | ContextView::Full) {
            item.relations = read.relations.iter().map(|relation| json!({
                "fromPageId": relation.from_page_id, "toPageId": relation.to_page_id,
                "type": relation.relation_type, "basisRevisionIds": relation.basis_revision_ids
            })).collect();
        }
        if matches!(view, ContextView::Sources | ContextView::Full) {
            item.source_refs = revision.source_refs.clone();
            item.source_span = revision.source_span.clone();
            for event in &revision.provenance {
                for id in &event.input_revision_ids {
                    if !item.basis_revision_ids.contains(id) { item.basis_revision_ids.push(id.clone()); }
                }
            }
        }
        if matches!(view, ContextView::History | ContextView::Full) {
            item.history = read.history.clone();
            item.assessment_revision_ids = read.validity_history.iter()
                .map(|assessment| assessment.assessment_revision_id.clone()).collect();
        }
        item
    }).collect();
    finish(items, None)
}

pub fn query_context(result: &QueryContextResponse, budget: ContextBudget) -> ModelContext {
    let mut remaining = budget.content_chars;
    let items = result
        .entries
        .iter()
        .map(|entry| {
            let mut item = ContextItem::new(
                &entry.page_id,
                &entry.revision_id,
                &entry.namespace,
                &entry.kind,
            );
            item.detail = match entry.detail {
                ContextDetail::Payload => "payload",
                ContextDetail::Excerpt => "excerpt",
                ContextDetail::Summary => "summary",
                ContextDetail::Reference => "reference",
            }
            .into();
            item.truncated = entry.source_projection_truncated;
            if let Some(content) = &entry.content {
                let (content, truncated) = text(content, &mut remaining, budget.preview_chars);
                item.content = Some(content);
                item.truncated |= truncated;
                if truncated && item.detail == "payload" {
                    item.detail = "excerpt".into();
                }
            }
            item.validity = entry.validity.as_ref().map(validity);
            if let Some(relation) = &entry.relation {
                item.relations.push(json!(relation));
                item.anchor_page_id = result
                    .entries
                    .iter()
                    .find(|anchor| {
                        anchor.anchor_rank == entry.anchor_rank && anchor.relation.is_none()
                    })
                    .map(|anchor| anchor.page_id.clone());
            }
            item
        })
        .collect();
    let mut response = finish(items, None);
    response.stopped_reason = result
        .intent_match
        .as_ref()
        .map(|audit| audit.stopped_reason.clone());
    response
}

pub fn graph_context(
    result: &GraphSliceResponse,
    view: ContextView,
    budget: ContextBudget,
) -> ModelContext {
    let mut response = read_context(&result.nodes, view, budget);
    // The bounded edge list already carries direction and provenance kind.
    // Do not duplicate each node's potentially larger relation neighborhood.
    for item in &mut response.items {
        item.relations.clear();
    }
    response.edges = result.edges.clone();
    response.truncated |= result.truncated;
    response
}

impl ModelContext {
    /// Plain text retains every field of the compact result, including caveats
    /// and follow-up identifiers. Body text is not summarized or interpreted.
    pub fn to_text(&self) -> String {
        let mut blocks = vec![];
        for item in &self.items {
            let mut metadata = serde_json::to_value(item).expect("serializable context item");
            metadata.as_object_mut().expect("object").remove("content");
            blocks.push(format!(
                "Evidence {}\n{}",
                metadata,
                item.content.as_deref().unwrap_or("")
            ));
        }
        if blocks.is_empty() {
            blocks.push("No results returned; this does not establish absence.".into());
        }
        if !self.edges.is_empty() {
            blocks.push(format!("Edges: {}", json!(self.edges)));
        }
        blocks.push(format!(
            "Retrieval: {}",
            json!({"truncated": self.truncated,
            "nextCursor": self.next_cursor, "stoppedReason": self.stopped_reason})
        ));
        blocks.join("\n\n")
    }
}

#[cfg(test)]
mod tests;
