//! Explicit promotion with a durable idempotency boundary before the Page write.
use super::{
    ContextHub, digest,
    persistence::{Candidate, LockedState},
    text_limit, timestamp,
};
use anyhow::{Result, ensure};
use chrono::{Duration, Utc};
use pcp_client::{
    EmbeddedPcpClient, PcpTenantApi,
    context_hub::{CandidateAction, CandidateReview},
};
use pcp_core::{AccessSession, IngestPageRequest, PagePayload, Projection, ReadPagesRequest};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

impl ContextHub {
    pub(super) async fn review(
        &self,
        access: &AccessSession,
        db: &mut LockedState,
        mut request: CandidateReview,
    ) -> Result<Value> {
        ensure!(
            !request.candidates.is_empty() && request.candidates.len() <= 20,
            "review requires 1..20 candidates"
        );
        request
            .candidates
            .sort_by(|a, b| a.candidate_id.cmp(&b.candidate_id));
        ensure!(
            request
                .candidates
                .windows(2)
                .all(|pair| pair[0].candidate_id != pair[1].candidate_id),
            "duplicate candidate in review"
        );
        let key = digest(&request)?;
        let items: Vec<_> = request
            .candidates
            .iter()
            .map(|selected| {
                db.state
                    .candidates
                    .iter()
                    .find(|c| c.candidate_id == selected.candidate_id)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("candidate is absent or expired; reload inbox"))
            })
            .collect::<Result<_>>()?;
        if items
            .iter()
            .all(|c| c.review_key.as_deref() == Some(&key) && c.result.is_some())
        {
            return Ok(items[0].result.clone().unwrap());
        }
        for (item, selected) in items.iter().zip(&request.candidates) {
            ensure!(
                item.version == selected.version,
                "candidate changed; reload before reviewing"
            );
            ensure!(
                matches!(item.status.as_str(), "pending" | "deferred")
                    || (item.status == "promoting" && item.review_key.as_deref() == Some(&key)),
                "candidate already decided or another promotion is in progress"
            );
        }
        let scope = &items[0].input.scope;
        ensure!(
            items.iter().all(|c| &c.input.scope == scope),
            "review groups must stay within one Scope"
        );
        let client = EmbeddedPcpClient::new(self.store.clone(), access.clone());
        let result = match request.action {
            CandidateAction::Promote => {
                let title = request
                    .title
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("reviewed title is required"))?;
                let content = request
                    .content
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("reviewed content is required"))?;
                text_limit("title", title, 160)?;
                text_limit("content", content, 16000)?;
                let mut sources = vec![];
                let mut basis = vec![];
                for item in &items {
                    // Never count mentions as truth or make candidate IDs pretend to be Revisions.
                    for source in &item.input.source_refs {
                        if !sources.iter().any(|existing| {
                            serde_json::to_value(existing).ok() == serde_json::to_value(source).ok()
                        }) {
                            sources.push(source.clone());
                        }
                    }
                    for revision in &item.input.based_on_revision_ids {
                        if !basis.contains(revision) {
                            basis.push(revision.clone());
                        }
                    }
                }
                // Persist the exact operator plan before the independently transactional Store write.
                // Retrying this same request uses ingest's event identity, including after a crash.
                for selected in &request.candidates {
                    let item = db
                        .state
                        .candidates
                        .iter_mut()
                        .find(|c| c.candidate_id == selected.candidate_id)
                        .unwrap();
                    item.status = "promoting".into();
                    item.review_key = Some(key.clone());
                    item.promotion_request = Some(request.clone());
                }
                db.save()?;
                let written = client.ingest_page(IngestPageRequest {
                    namespace: scope.clone(), kind: "reviewed_capture".into(), observed_at: None, source_span: None,
                    payload: Some(PagePayload { media_type: "text/markdown".into(), content: format!("# {title}\n\n{content}") }),
                    source_refs: sources, based_on_revision_ids: basis,
                    facets: Some(json!({"title":title, "reviewedCandidates":items.iter().map(|c| json!({"candidateId":c.candidate_id,"clientId":c.client_id,"submittedAt":c.created_at})).collect::<Vec<_>>()})),
                    external_event_id: Some(format!("pcp-context-review:{key}")),
                }).await?;
                json!({"status":"promoted","pageId":written.page_id,"revisionId":written.revision_id})
            }
            CandidateAction::Represented => {
                let id = request
                    .target_revision_id
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("targetRevisionId is required"))?;
                let pages = client
                    .read_pages(ReadPagesRequest {
                        page_ids: vec![],
                        revision_ids: vec![id.clone()],
                        projections: vec![Projection::Manifest, Projection::Validity],
                        max_chars: 1000,
                    })
                    .await?;
                let page = pages
                    .first()
                    .ok_or_else(|| anyhow::anyhow!("target Revision is unavailable"))?;
                ensure!(
                    &page.revision.namespace == scope,
                    "represented target must be in the candidate Scope"
                );
                ensure!(
                    page.page.head_revision_id == page.revision.revision_id
                        && page.page.lifecycle_status == pcp_core::LifecycleStatus::Active,
                    "represented target must be an active current Revision"
                );
                ensure!(
                    !page.validity.as_ref().is_some_and(|v| matches!(
                        v.standing,
                        pcp_core::ValidityStanding::Superseded
                            | pcp_core::ValidityStanding::Retracted
                    )),
                    "represented target has been superseded or retracted"
                );
                json!({"status":"represented","pageId":page.page.page_id,"revisionId":page.revision.revision_id})
            }
            CandidateAction::Defer => json!({"status":"deferred"}),
            CandidateAction::Reject => json!({"status":"rejected"}),
        };
        for selected in &request.candidates {
            let item = db
                .state
                .candidates
                .iter_mut()
                .find(|c| c.candidate_id == selected.candidate_id)
                .unwrap();
            item.version += 1;
            item.status = result["status"].as_str().unwrap().into();
            item.review_key = Some(key.clone());
            item.result = Some(result.clone());
            item.promotion_request = None;
            item.snoozed_until = (request.action == CandidateAction::Defer)
                .then(|| timestamp(Utc::now() + Duration::days(7)));
        }
        db.save()?;
        Ok(result)
    }
}

/// Similarity only suggests side-by-side inspection; it never merges facts or votes.
pub(super) fn similar_candidates(items: &[Candidate]) -> BTreeMap<String, Vec<String>> {
    let active: Vec<_> = items
        .iter()
        .filter(|c| matches!(c.status.as_str(), "pending" | "deferred"))
        .collect();
    let grams: Vec<HashSet<String>> = active
        .iter()
        .map(|c| {
            let chars: Vec<_> = c
                .input
                .title
                .to_lowercase()
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect();
            chars.windows(2).map(|pair| pair.iter().collect()).collect()
        })
        .collect();
    let mut result = BTreeMap::new();
    for (i, a) in active.iter().enumerate() {
        let mut matches = vec![];
        for (j, b) in active.iter().enumerate() {
            if i == j || a.input.scope != b.input.scope {
                continue;
            }
            let same = a.input.content.trim() == b.input.content.trim();
            let overlap = grams[i].intersection(&grams[j]).count();
            let union = grams[i].union(&grams[j]).count();
            if same || (union >= 4 && overlap as f64 / union as f64 >= 0.65) {
                matches.push(b.candidate_id.clone());
                if matches.len() == 5 {
                    break;
                }
            }
        }
        if !matches.is_empty() {
            result.insert(a.candidate_id.clone(), matches);
        }
    }
    result
}
