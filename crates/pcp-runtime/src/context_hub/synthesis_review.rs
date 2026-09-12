//! Explicit multi-output review with exact receipts across partial completion.
use super::{
    ContextHub, digest,
    persistence::LockedState,
    synthesis::{active, validate_output},
    timestamp,
};
use anyhow::{Result, ensure};
use chrono::Utc;
use pcp_client::{
    EmbeddedPcpClient, PcpApi, PcpTenantApi,
    context_hub::{SynthesisReview, SynthesisStop},
};
use pcp_core::{
    AccessSession, Actor, ActorType, IngestPageRequest, LifecycleStatus, PageMutability,
    PagePayload, Projection, ProvenanceEvent, ReadPagesRequest, RevisePageRequest,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

impl ContextHub {
    /// Explicitly release an interrupted plan without rolling back or deleting
    /// any Page. Unknown output outcomes remain visible for operator comparison.
    pub(super) fn stop_synthesis(
        &self,
        db: &mut LockedState,
        request: SynthesisStop,
    ) -> Result<Value> {
        let index = db
            .state
            .syntheses
            .iter()
            .position(|s| s.synthesis_id == request.synthesis_id)
            .ok_or_else(|| anyhow::anyhow!("Synthesis is unavailable"))?;
        let group = db.state.syntheses[index].clone();
        if group.status == "interrupted" && request.version.checked_add(1) == Some(group.version) {
            return Ok(json!({"status":"interrupted", "confirmedOutputs":group.results}));
        }
        ensure!(
            group.version == request.version && group.status == "promoting",
            "Only the exact interrupted submission can be stopped"
        );
        let covered = group
            .review_request
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Submission plan is unavailable"))?
            .outputs
            .iter()
            .flat_map(|output| output.candidate_ids.iter())
            .collect::<BTreeSet<_>>();
        for candidate in &mut db.state.candidates {
            if covered.contains(&candidate.candidate_id) && candidate.status == "promoting" {
                candidate.status = "pending".into();
                candidate.version += 1;
                candidate.organized_version = 0;
                candidate.snoozed_until = None;
                candidate.review_key = None;
                candidate.result = Some(
                    json!({"status":"interrupted", "synthesisId":group.synthesis_id, "confirmedOutputs":group.results}),
                );
            }
        }
        db.state.organization.queued = true;
        db.state.organization.next_attempt_at = None;
        db.state.syntheses[index].status = "interrupted".into();
        db.state.syntheses[index].version += 1;
        super::synthesis::invalidate_stale(&mut db.state);
        db.save()?;
        Ok(json!({"status":"interrupted", "confirmedOutputs":group.results}))
    }

    pub(super) async fn review_synthesis(
        &self,
        access: &AccessSession,
        db: &mut LockedState,
        request: SynthesisReview,
        automatic: bool,
    ) -> Result<Value> {
        let index = db
            .state
            .syntheses
            .iter()
            .position(|s| s.synthesis_id == request.synthesis_id)
            .ok_or_else(|| anyhow::anyhow!("Synthesis is unavailable; reload inbox"))?;
        let mut snapshot = db.state.syntheses[index].clone();
        let review_key = digest(&request)?;
        let retry = snapshot
            .review_request
            .as_ref()
            .is_some_and(|old| digest(old).ok().as_deref() == Some(review_key.as_str()));
        let automatic = automatic
            || (snapshot.status == "promoting"
                && retry
                && snapshot
                    .automatic_review
                    .as_ref()
                    .is_some_and(|a| a.authorized));
        if !automatic && snapshot.status == "pending" {
            // A new operator-edited plan is not an approval by the earlier model.
            snapshot.automatic_review = None;
            db.state.syntheses[index].automatic_review = None;
        }
        if snapshot.status == "promoted" && retry {
            return Ok(json!({"status":"promoted", "outputs":snapshot.results}));
        }
        ensure!(
            snapshot.version == request.version
                && (snapshot.status == "pending" || (snapshot.status == "promoting" && retry)),
            "Synthesis changed or another decision is in progress; reload"
        );
        ensure!(
            !request.outputs.is_empty() && request.outputs.len() <= 4,
            "Review requires 1..4 memory outputs"
        );
        let candidate_ids = snapshot
            .candidates
            .iter()
            .map(|c| c.candidate_id.clone())
            .collect::<Vec<_>>();
        let mut covered = BTreeSet::new();
        let mut targets = BTreeSet::new();
        for output in &request.outputs {
            validate_output(output, &candidate_ids, &snapshot.offered_revision_ids)?;
            ensure!(
                output.action != "update"
                    || output
                        .target_revision_id
                        .as_ref()
                        .is_some_and(|id| snapshot.updateable_revision_ids.contains(id)),
                "Target cannot be updated; choose a new memory"
            );
            covered.extend(output.candidate_ids.iter().cloned());
            if let Some(id) = &output.target_revision_id {
                ensure!(
                    targets.insert(id.clone()),
                    "Review cannot target one Page twice"
                );
            }
        }
        if !retry {
            ensure!(
                snapshot
                    .candidates
                    .iter()
                    .all(|r| db
                        .state
                        .candidates
                        .iter()
                        .any(|c| c.candidate_id == r.candidate_id
                            && c.version == r.version
                            && active(c))),
                "Synthesis evidence changed; reload before reviewing"
            );
        }
        let items = snapshot
            .candidates
            .iter()
            .filter(|r| covered.contains(&r.candidate_id))
            .map(|r| {
                db.state
                    .candidates
                    .iter()
                    .find(|c| {
                        c.candidate_id == r.candidate_id
                            && c.version == r.version
                            && c.input.scope == snapshot.scope
                            && (active(c) || (retry && c.status == "promoting"))
                    })
                    .cloned()
                    .ok_or_else(|| {
                        anyhow::anyhow!("Candidate evidence changed; reload before reviewing")
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        let client = EmbeddedPcpClient::new(self.store.clone(), access.clone());
        // Preflight every target before any write. Returning to an in-flight exact plan
        // uses Store idempotency rather than treating our own committed head as stale.
        if !retry {
            for output in &request.outputs {
                if let Some(id) = &output.target_revision_id {
                    let pages = client
                        .read_pages(ReadPagesRequest {
                            page_ids: vec![],
                            revision_ids: vec![id.clone()],
                            projections: vec![Projection::Manifest, Projection::Validity],
                            max_chars: 1000,
                        })
                        .await?;
                    let target = pages
                        .first()
                        .ok_or_else(|| anyhow::anyhow!("Target Revision is unavailable"))?;
                    ensure!(
                        target.revision.namespace == snapshot.scope
                            && target.page.head_revision_id == *id
                            && target.page.lifecycle_status == LifecycleStatus::Active,
                        "Target changed or is outside candidate Scope"
                    );
                    ensure!(
                        !target.validity.as_ref().is_some_and(|v| matches!(
                            v.standing,
                            pcp_core::ValidityStanding::Superseded
                                | pcp_core::ValidityStanding::Retracted
                        )),
                        "Target has been superseded or retracted"
                    );
                    if output.action == "update" {
                        ensure!(
                            target.page.mutability == PageMutability::Revisioned,
                            "Target is sealed; choose a new memory instead"
                        );
                    }
                }
            }
            db.state.syntheses[index].status = "promoting".into();
            db.state.syntheses[index].review_request = Some(request.clone());
            for c in &mut db.state.candidates {
                if covered.contains(&c.candidate_id) {
                    c.status = "promoting".into();
                }
            }
            db.save()?;
        }
        for (i, output) in request
            .outputs
            .iter()
            .enumerate()
            .skip(db.state.syntheses[index].results.len())
        {
            let evidence = items
                .iter()
                .filter(|c| output.candidate_ids.contains(&c.candidate_id))
                .collect::<Vec<_>>();
            let mut sources = vec![];
            let mut basis = vec![];
            for item in &evidence {
                for source in &item.input.source_refs {
                    if !sources
                        .iter()
                        .any(|s| serde_json::to_value(s).ok() == serde_json::to_value(source).ok())
                    {
                        sources.push(source.clone());
                    }
                }
                for id in &item.input.based_on_revision_ids {
                    if !basis.contains(id) {
                        basis.push(id.clone());
                    }
                }
            }
            let mut facets = json!({"title":output.title,"reviewedSynthesis":snapshot.synthesis_id,"reviewedCandidates":evidence.iter().map(|c| json!({"candidateId":c.candidate_id,"clientId":c.client_id,"submittedAt":c.created_at})).collect::<Vec<_>>()});
            if automatic {
                let review = snapshot
                    .automatic_review
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Automatic approval missing"))?;
                ensure!(review.authorized, "Automatic review has no write authority");
                facets["automaticCandidateReview"] = json!({"reviewedAt":review.reviewed_at,"steps":review.steps,"decisions":review.decisions,"evidence":evidence.iter().map(|c| json!({"candidateId":c.candidate_id,"clientId":c.client_id,"submittedAt":c.created_at,"input":c.input})).collect::<Vec<_>>()});
            }
            let key = format!("pcp-synthesis:{review_key}:{i}");
            let result = match output.action.as_str() {
                "create" => {
                    let written = client
                        .ingest_page(IngestPageRequest {
                            namespace: snapshot.scope.clone(),
                            kind: "reviewed_capture".into(),
                            observed_at: None,
                            source_span: None,
                            payload: Some(PagePayload {
                                media_type: "text/markdown".into(),
                                content: format!("# {}\n\n{}", output.title, output.content),
                            }),
                            source_refs: sources,
                            based_on_revision_ids: basis,
                            facets: Some(facets),
                            external_event_id: Some(key),
                        })
                        .await?;
                    json!({"action":"create","pageId":written.page_id,"revisionId":written.revision_id})
                }
                action => {
                    let id = output.target_revision_id.as_ref().unwrap();
                    let pages = client
                        .read_pages(ReadPagesRequest {
                            page_ids: vec![],
                            revision_ids: vec![id.clone()],
                            projections: vec![
                                Projection::Manifest,
                                Projection::Sources,
                                Projection::Provenance,
                                Projection::Facets,
                            ],
                            max_chars: 32000,
                        })
                        .await?;
                    let target = pages
                        .first()
                        .ok_or_else(|| anyhow::anyhow!("Target Revision is unavailable"))?;
                    ensure!(
                        target.revision.namespace == snapshot.scope,
                        "Target is outside candidate Scope"
                    );
                    if action == "represented" {
                        ensure!(
                            target.page.head_revision_id == *id
                                && target.page.lifecycle_status == LifecycleStatus::Active,
                            "Represented target changed; reload"
                        );
                        json!({"action":"represented","pageId":target.page.page_id,"revisionId":id})
                    } else {
                        let mut old = target.revision.clone();
                        for source in sources {
                            if !old.source_refs.iter().any(|s| {
                                serde_json::to_value(s).ok() == serde_json::to_value(&source).ok()
                            }) {
                                old.source_refs.push(source);
                            }
                        }
                        let mut merged_facets = old.facets.take().unwrap_or_else(|| json!({}));
                        ensure!(
                            merged_facets.is_object(),
                            "Target facets cannot be safely extended"
                        );
                        let mut new_facets = facets.as_object().unwrap().clone();
                        if let Some(previous) = merged_facets
                            .get("reviewedCandidates")
                            .and_then(Value::as_array)
                        {
                            let combined = new_facets
                                .get_mut("reviewedCandidates")
                                .unwrap()
                                .as_array_mut()
                                .unwrap();
                            for old in previous {
                                if !combined.contains(old) {
                                    combined.push(old.clone());
                                }
                            }
                        }
                        merged_facets.as_object_mut().unwrap().extend(new_facets);
                        let actor = Actor {
                            actor_type: if automatic {
                                ActorType::Model
                            } else {
                                ActorType::User
                            },
                            actor_id: access.principal.principal_id.clone(),
                        };
                        if !basis.contains(id) {
                            basis.push(id.clone());
                        }
                        old.provenance.push(ProvenanceEvent {
                            operation: "candidate_synthesis".into(),
                            actor: actor.clone(),
                            timestamp: snapshot.created_at.clone(),
                            input_revision_ids: basis,
                            tool_or_model: automatic.then(|| "shared-budget-sol-review".into()),
                            reason: Some(
                                if automatic {
                                    "Automatically applied after source-based Sol review"
                                } else {
                                    "Operator-reviewed candidate synthesis"
                                }
                                .into(),
                            ),
                        });
                        let written = client
                            .revise_page(RevisePageRequest {
                                page_id: old.page_id,
                                expected_revision_id: id.clone(),
                                created_by: actor,
                                lifecycle_status: LifecycleStatus::Active,
                                observed_at: old.observed_at,
                                valid_from: old.valid_from,
                                valid_to: old.valid_to,
                                payload: Some(PagePayload {
                                    media_type: "text/markdown".into(),
                                    content: format!("# {}\n\n{}", output.title, output.content),
                                }),
                                source_refs: old.source_refs,
                                facets: Some(merged_facets),
                                provenance: old.provenance,
                                initial_relations: vec![],
                                idempotency_key: Some(key),
                            })
                            .await?;
                        json!({"action":"update","pageId":written.page_id,"revisionId":written.revision_id})
                    }
                }
            };
            db.state.syntheses[index].results.push(result);
            db.save()?;
        }
        let results = db.state.syntheses[index].results.clone();
        for candidate in &mut db.state.candidates {
            if covered.contains(&candidate.candidate_id) {
                let own = request
                    .outputs
                    .iter()
                    .zip(&results)
                    .filter(|(out, _)| out.candidate_ids.contains(&candidate.candidate_id))
                    .map(|(_, result)| result.clone())
                    .collect::<Vec<_>>();
                let retained = automatic
                    && snapshot
                        .automatic_review
                        .as_ref()
                        .is_some_and(|a| a.retain_candidate_ids.contains(&candidate.candidate_id));
                candidate.status = if retained { "pending" } else { "promoted" }.into();
                candidate.version += 1;
                if retained {
                    candidate.organized_version = candidate.version;
                }
                candidate.review_key = Some(review_key.clone());
                candidate.result = Some(
                    json!({"status":if retained {"partially_represented"} else {"promoted"},"pageId":own.first().and_then(|v| v.get("pageId")),"outputs":own}),
                );
                candidate.expires_at = timestamp(Utc::now() + chrono::Duration::days(30));
            }
        }
        db.state.syntheses[index].status = "promoted".into();
        db.state.syntheses[index].version += 1;
        super::synthesis::invalidate_stale(&mut db.state);
        db.save()?;
        Ok(json!({"status":"promoted","outputs":results}))
    }
}
