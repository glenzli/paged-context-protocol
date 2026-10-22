//! Operator read views. Lists omit review evidence; detail keeps the exact audit inputs.
use super::persistence::{Candidate, HubState, LockedState};
use super::synthesis::Synthesis;
use super::{ContextHub, digest, permission, require_operator, review, synthesis};
use crate::maintenance::MaintenanceDetailPage;
use anyhow::Result;
use pcp_core::{AccessPermission, AccessSession};
use serde_json::{Value, json};

impl ContextHub {
    fn refresh_inspection(&self, db: &mut LockedState) -> Result<()> {
        db.state.organization.available = self
            .organization_available
            .load(std::sync::atomic::Ordering::Relaxed);
        db.state.organization.automatic_review_available = self
            .automatic_review_available
            .load(std::sync::atomic::Ordering::Relaxed);
        synthesis::invalidate_stale(&mut db.state);
        db.save()
    }

    pub(super) fn inspect(&self, access: &AccessSession, db: &mut LockedState) -> Result<Value> {
        require_operator(access)?;
        self.refresh_inspection(db)?;
        // Retain the complete legacy operator response for existing consumers.
        Ok(
            json!({"policies":db.state.policies,"candidates":db.state.candidates,
            "activity":db.state.activity,"similarCandidates":review::similar_candidates(&db.state.candidates),
            "syntheses":db.state.syntheses,"synthesisContexts":db.state.synthesis_contexts,
            "organization":db.state.organization}),
        )
    }

    pub(super) fn inspect_activity(
        &self,
        access: &AccessSession,
        db: &LockedState,
    ) -> Result<Value> {
        require_operator(access)?;
        let mut cards = db
            .state
            .activity
            .iter()
            .filter(|card| access.allows(&card.scope, AccessPermission::ReadDetail))
            .collect::<Vec<_>>();
        cards.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.card_id.cmp(&b.card_id))
        });
        Ok(json!({"activity":cards,"limits":{
            "topicsPerClientScope":super::activity::MAX_TOPICS_PER_CLIENT_SCOPE,
            "totalCards":super::activity::MAX_CARDS}}))
    }

    pub(super) fn inspect_inbox(
        &self,
        access: &AccessSession,
        db: &mut LockedState,
    ) -> Result<Value> {
        require_operator(access)?;
        self.refresh_inspection(db)?;
        let visible = db
            .state
            .candidates
            .iter()
            .filter(|c| access.allows(&c.input.scope, AccessPermission::ReadDetail))
            .cloned()
            .collect::<Vec<_>>();
        let candidates = visible
            .iter()
            .map(candidate_summary)
            .collect::<Result<Vec<_>>>()?;
        let syntheses = db
            .state
            .syntheses
            .iter()
            .filter(|s| access.allows(&s.scope, AccessPermission::ReadDetail))
            .map(|group| {
                // A digest also describes evidence. Never derive it from unreadable Scopes.
                let token = synthesis_evidence(access, &db.state, group)
                    .ok()
                    .map(|evidence| synthesis_detail_token(group, &evidence))
                    .transpose()?;
                Ok(synthesis_summary(group, token))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(json!({"candidates":candidates,"syntheses":syntheses,
            "similarCandidates":review::similar_candidates(&visible),"organization":db.state.organization}))
    }

    pub(super) fn inspect_candidate(
        &self,
        access: &AccessSession,
        db: &LockedState,
        id: &str,
    ) -> Result<Value> {
        require_operator(access)?;
        let candidate = db
            .state
            .candidates
            .iter()
            .find(|c| c.candidate_id == id)
            .ok_or_else(|| anyhow::anyhow!("candidate is absent or expired"))?;
        permission(access, &candidate.input.scope, AccessPermission::ReadDetail)?;
        Ok(json!({"candidate":candidate,"detailToken":digest(candidate)?}))
    }

    pub(super) fn inspect_synthesis(
        &self,
        access: &AccessSession,
        db: &mut LockedState,
        id: &str,
    ) -> Result<Value> {
        require_operator(access)?;
        self.refresh_inspection(db)?;
        let group = db
            .state
            .syntheses
            .iter()
            .find(|s| s.synthesis_id == id)
            .ok_or_else(|| anyhow::anyhow!("synthesis is absent or expired"))?;
        let evidence = synthesis_evidence(access, &db.state, group)?;
        let candidate_tokens = evidence
            .candidates
            .iter()
            .map(|candidate| Ok((candidate.candidate_id.clone(), digest(*candidate)?)))
            .collect::<Result<std::collections::BTreeMap<_, _>>>()?;
        Ok(json!({"synthesis":group,"candidates":evidence.candidates,
            "synthesisContext":evidence.context,"detailToken":synthesis_detail_token(group, &evidence)?,
            "candidateDetailTokens":candidate_tokens}))
    }
}

struct SynthesisEvidence<'a> {
    candidates: Vec<&'a Candidate>,
    context: &'a [MaintenanceDetailPage],
}

fn synthesis_evidence<'a>(
    access: &AccessSession,
    state: &'a HubState,
    group: &'a Synthesis,
) -> Result<SynthesisEvidence<'a>> {
    permission(access, &group.scope, AccessPermission::ReadDetail)?;
    let candidates = state
        .candidates
        .iter()
        .filter(|candidate| {
            group
                .candidates
                .iter()
                .any(|reference| reference.candidate_id == candidate.candidate_id)
        })
        .collect::<Vec<_>>();
    for candidate in &candidates {
        permission(access, &candidate.input.scope, AccessPermission::ReadDetail)?;
    }
    let context = group
        .context_id
        .as_ref()
        .and_then(|id| state.synthesis_contexts.get(id))
        .unwrap_or(&group.compared_pages);
    // Both the returned detail and its identity require the complete evidence ACL.
    for page in context.iter().chain(&group.compared_pages) {
        permission(access, &page.namespace, AccessPermission::ReadDetail)?;
    }
    if let Some(input) = group
        .automatic_review
        .as_ref()
        .and_then(|review| review.input.as_ref())
    {
        permission(access, &input.scope, AccessPermission::ReadDetail)?;
        for candidate in &input.candidates {
            permission(access, &candidate.input.scope, AccessPermission::ReadDetail)?;
        }
        for page in &input.pages {
            permission(access, &page.namespace, AccessPermission::ReadDetail)?;
        }
    }
    Ok(SynthesisEvidence {
        candidates,
        context,
    })
}

fn synthesis_detail_token(group: &Synthesis, evidence: &SynthesisEvidence<'_>) -> Result<String> {
    // Version guards write authority; this identity also covers in-version review and receipt updates.
    digest(&(group, &evidence.candidates, evidence.context))
}

fn candidate_summary(candidate: &Candidate) -> Result<Value> {
    Ok(
        json!({"candidateId":candidate.candidate_id,"clientId":candidate.client_id,
        "input":candidate.input,"createdAt":candidate.created_at,"expiresAt":candidate.expires_at,
        "version":candidate.version,"status":candidate.status,"snoozedUntil":candidate.snoozed_until,
        "organizedVersion":candidate.organized_version,"result":candidate.result.as_ref().map(receipt_summary),
        "summaryOnly":true,"detailToken":digest(candidate)?}),
    )
}

fn output_summary(output: &pcp_client::context_hub::SynthesisOutput) -> Value {
    json!({"candidateIds":output.candidate_ids,"title":output.title,
        "action":output.action,"targetRevisionId":output.target_revision_id})
}

fn synthesis_summary(group: &Synthesis, detail_token: Option<String>) -> Value {
    let automatic = group.automatic_review.as_ref().map(|review| {
        json!({
            "state":review.state,"reason":review.reason,"nextAttemptAt":review.next_attempt_at,
            "reviewedAt":review.reviewed_at,"authorized":review.authorized,
            "decisions":review.decisions.iter().map(|d| json!({"outputIndex":d.output_index,
                "verdict":d.verdict,"reason":d.reason})).collect::<Vec<_>>()
        })
    });
    let request = group.review_request.as_ref().map(|request| {
        json!({
            "synthesisId":request.synthesis_id,"version":request.version,
            "outputs":request.outputs.iter().map(output_summary).collect::<Vec<_>>()
        })
    });
    json!({"synthesisId":group.synthesis_id,"version":group.version,"scope":group.scope,
        "candidates":group.candidates,"title":group.title,"maturity":group.maturity,
        "status":group.status,"createdAt":group.created_at,"contextId":group.context_id,
        "outputs":group.outputs.iter().map(output_summary).collect::<Vec<_>>(),
        "reviewRequest":request,"results":group.results.iter().map(receipt_summary).collect::<Vec<_>>(),
        "automaticReview":automatic,"summaryOnly":true,"detailToken":detail_token})
}

/// Receipts remain useful for queue state and Page navigation without admitting
/// arbitrary nested review inputs into an otherwise bounded list response.
fn receipt_summary(receipt: &Value) -> Value {
    let mut summary = serde_json::Map::new();
    for key in ["status", "action", "pageId", "revisionId", "synthesisId"] {
        if let Some(value) = receipt.get(key) {
            summary.insert(key.into(), value.clone());
        }
    }
    for key in ["outputs", "confirmedOutputs"] {
        if let Some(values) = receipt.get(key).and_then(Value::as_array) {
            summary.insert(
                key.into(),
                Value::Array(values.iter().map(receipt_summary).collect()),
            );
        }
    }
    Value::Object(summary)
}
