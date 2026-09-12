//! Candidate review admission, immutable evidence and automatic write receipts.
//! Sol owns semantic assessment; the hub owns exact-version authority and recovery.
use super::{
    ContextHub, digest, permission,
    persistence::{Candidate, LockedState},
    synthesis::{Synthesis, active, validate_output},
    timestamp,
};
use crate::maintenance::{MaintenanceDetailPage, MaintenanceReviewStep};
use anyhow::{Result, ensure};
use chrono::{Duration, Utc};
use pcp_client::{
    PcpApi,
    context_hub::{SynthesisOutput, SynthesisReview},
};
use pcp_core::{AccessPermission, AccessSession, Projection, ReadPagesRequest};
use pcp_store::DurablePageInventoryItem;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateReviewInput {
    pub intake_watermark: String,
    pub synthesis_id: String,
    pub version: u64,
    pub scope: String,
    pub title: String,
    pub narrative: String,
    pub unresolved: Vec<String>,
    pub candidates: Vec<Candidate>,
    pub pages: Vec<MaintenanceDetailPage>,
    pub outputs: Vec<SynthesisOutput>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputAssessment {
    /// Zero-based index into the immutable proposed outputs.
    pub output_index: usize,
    /// approve, accumulating, needs_input, or no_change.
    pub verdict: String,
    pub reason: String,
    #[serde(default)]
    pub revision: Option<crate::maintenance::VerifiedMaintenanceRevision>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomaticReview {
    pub state: String,
    pub reason: String,
    pub next_attempt_at: Option<String>,
    pub reviewed_at: Option<String>,
    pub input: Option<CandidateReviewInput>,
    pub decisions: Vec<OutputAssessment>,
    pub steps: Vec<MaintenanceReviewStep>,
    pub retain_candidate_ids: Vec<String>,
    /// Set by validated review completion, never by a model or tenant request.
    pub authorized: bool,
    #[serde(default)]
    pub undo_results: Vec<serde_json::Value>,
}

impl CandidateReviewInput {
    pub fn validate(&self, decisions: &[OutputAssessment]) -> Result<()> {
        ensure!(
            decisions.len() == self.outputs.len(),
            "Review must assess every output exactly once"
        );
        let mut seen = BTreeSet::new();
        for decision in decisions {
            ensure!(
                decision.output_index < self.outputs.len() && seen.insert(decision.output_index),
                "Unknown or repeated reviewed output"
            );
            ensure!(
                matches!(
                    decision.verdict.as_str(),
                    "approve" | "accumulating" | "needs_input" | "no_change"
                ),
                "Invalid candidate review verdict"
            );
            super::text_limit("review reason", &decision.reason, 2400)?;
            if let Some(revision) = &decision.revision {
                ensure!(
                    decision.verdict == "approve",
                    "Only an approved output may propose a repair"
                );
                super::text_limit("reviewed title", &revision.title, 160)?;
                super::text_limit("reviewed memory", &revision.content, 16000)?;
            }
        }
        Ok(())
    }
}

fn current(group: &Synthesis, candidates: &[Candidate]) -> bool {
    group.candidates.iter().all(|r| {
        candidates
            .iter()
            .any(|c| c.candidate_id == r.candidate_id && c.version == r.version && active(c))
    })
}

fn review_input_changed(
    input: &CandidateReviewInput,
    candidates: &[Candidate],
    inventory: &[DurablePageInventoryItem],
) -> bool {
    candidates
        .iter()
        .any(|c| active(c) && c.input.scope == input.scope && c.created_at > input.intake_watermark)
        || inventory
            .iter()
            .any(|p| p.namespace == input.scope && p.created_at > input.intake_watermark)
        || input.pages.iter().any(|p| {
            !inventory
                .iter()
                .any(|i| i.page_id == p.page_id && i.revision_id == p.revision_id)
        })
}

impl ContextHub {
    pub(crate) fn enable_automatic_review(&self, available: bool) {
        self.automatic_review_available
            .store(available, std::sync::atomic::Ordering::Relaxed);
    }
    pub(crate) async fn prepare_automatic_review(
        &self,
        client: &dyn PcpApi,
        inventory: &[DurablePageInventoryItem],
    ) -> Result<Option<CandidateReviewInput>> {
        let now = timestamp(Utc::now());
        let settled = timestamp(Utc::now() - Duration::minutes(5));
        let quiet = timestamp(Utc::now() - Duration::seconds(120));
        let mut db = LockedState::open(&self.path, self.store.identity_id()).await?;
        if db.state.organization.automatic_review_enabled != Some(true) {
            return Ok(None);
        }
        let next = db
            .state
            .syntheses
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.status == "pending"
                    && !s.outputs.is_empty()
                    && s.created_at <= settled
                    && permission(client.access(), &s.scope, AccessPermission::ReadDetail).is_ok()
                    && permission(client.access(), &s.scope, AccessPermission::Ingest).is_ok()
                    && current(s, &db.state.candidates)
                    && !db.state.candidates.iter().any(|c| {
                        active(c)
                            && c.input.scope == s.scope
                            && (c.created_at > quiet
                                || (s
                                    .candidates
                                    .iter()
                                    .any(|r| r.candidate_id == c.candidate_id)
                                    && c.snoozed_until
                                        .as_deref()
                                        .is_some_and(|until| until > now.as_str())))
                    })
                    && s.automatic_review.as_ref().is_none_or(|a| {
                        matches!(a.state.as_str(), "running" | "waiting_budget")
                            && a.next_attempt_at
                                .as_deref()
                                .is_some_and(|at| at <= now.as_str())
                    })
            })
            .min_by_key(|(_, s)| &s.created_at)
            .map(|(i, s)| (i, s.clone()));
        let Some((index, group)) = next else {
            return Ok(None);
        };
        let input = if let Some(input) = group
            .automatic_review
            .as_ref()
            .and_then(|a| a.input.clone())
        {
            if review_input_changed(&input, &db.state.candidates, inventory) {
                let auto = db.state.syntheses[index].automatic_review.as_mut().unwrap();
                auto.state = "stale".into();
                auto.reason =
                    "Review evidence changed while waiting; reorganization required".into();
                for c in &mut db.state.candidates {
                    if group
                        .candidates
                        .iter()
                        .any(|r| r.candidate_id == c.candidate_id)
                        && active(c)
                    {
                        c.organized_version = 0;
                    }
                }
                db.save()?;
                return Ok(None);
            }
            input // Reuse exact evidence after a budget wait or a crashed call.
        } else {
            let candidates = group
                .candidates
                .iter()
                .filter_map(|r| {
                    db.state
                        .candidates
                        .iter()
                        .find(|c| c.candidate_id == r.candidate_id)
                        .cloned()
                })
                .collect::<Vec<_>>();
            let mut revisions = group.offered_revision_ids.clone();
            // Refresh nearby memories before review, including writes after organization.
            let terms = super::synthesis::grams(&format!("{} {}", group.title, group.narrative));
            let mut related = inventory
                .iter()
                .filter(|p| p.namespace == group.scope)
                .collect::<Vec<_>>();
            related.sort_by_key(|p| {
                (
                    std::cmp::Reverse(super::synthesis::similarity(
                        &terms,
                        &format!("{} {}", p.summary.as_deref().unwrap_or_default(), p.snippet),
                    )),
                    p.page_id.clone(),
                )
            });
            for page in related.into_iter().take(6) {
                if !revisions.contains(&page.revision_id) {
                    revisions.push(page.revision_id.clone());
                }
            }
            let pages: Vec<MaintenanceDetailPage> = if revisions.is_empty() {
                vec![]
            } else {
                client
                    .read_pages(ReadPagesRequest {
                        page_ids: vec![],
                        revision_ids: revisions,
                        projections: vec![
                            Projection::Manifest,
                            Projection::Payload,
                            Projection::Summary,
                            Projection::Validity,
                            Projection::Sources,
                            Projection::Facets,
                        ],
                        max_chars: 64000,
                    })
                    .await?
                    .into_iter()
                    .map(Into::into)
                    .collect()
            };
            // Truncated comparison is not sufficient for automatic replacement or duplicate resolution.
            if !pages.iter().all(|p| {
                p.content.as_ref().is_some_and(|content| {
                    inventory.iter().any(|i| {
                        i.revision_id == p.revision_id
                            && i.content_chars == content.chars().count() as u64
                    })
                })
            }) {
                db.state.syntheses[index].automatic_review = Some(AutomaticReview {
                    state: "needs_review".into(),
                    reason:
                        "Complete current comparison Pages are required before automatic review"
                            .into(),
                    ..Default::default()
                });
                db.save()?;
                return Ok(None);
            }
            CandidateReviewInput {
                intake_watermark: now.clone(),
                synthesis_id: group.synthesis_id,
                version: group.version,
                scope: group.scope,
                title: group.title,
                narrative: group.narrative,
                unresolved: group.unresolved,
                candidates,
                pages,
                outputs: group.outputs,
            }
        };
        db.state.syntheses[index].automatic_review = Some(AutomaticReview {
            state: "running".into(),
            reason: "Shared-budget Sol review queued".into(),
            next_attempt_at: Some(timestamp(Utc::now() + Duration::minutes(30))),
            input: Some(input.clone()),
            ..group.automatic_review.unwrap_or_default()
        });
        db.save()?;
        Ok(Some(input))
    }

    pub(crate) async fn finish_automatic_review(
        &self,
        input: &CandidateReviewInput,
        decisions: Vec<OutputAssessment>,
        steps: Vec<MaintenanceReviewStep>,
        state: &str,
        reason: &str,
    ) -> Result<()> {
        let mut db = LockedState::open(&self.path, self.store.identity_id()).await?;
        let Some(index) = db.state.syntheses.iter().position(|s| {
            s.synthesis_id == input.synthesis_id
                && s.version == input.version
                && s.status == "pending"
        }) else {
            return Ok(());
        };
        if !current(&db.state.syntheses[index], &db.state.candidates) {
            return Ok(());
        }
        let saved = db.state.syntheses[index]
            .automatic_review
            .as_ref()
            .and_then(|a| a.input.as_ref());
        ensure!(
            saved.is_some_and(|s| digest(s).ok() == digest(input).ok()),
            "Automatic review input changed"
        );
        let mut review = db.state.syntheses[index].automatic_review.clone().unwrap();
        review.steps = steps;
        review.state = state.into();
        review.reason = reason.chars().take(2400).collect();
        review.next_attempt_at =
            (state == "waiting_budget").then(|| timestamp(Utc::now() + Duration::minutes(10)));
        if state == "completed" {
            input.validate(&decisions)?;
            ensure!(
                !review.steps.is_empty()
                    && review.steps.len() <= 2
                    && review.steps[0].stage == "candidate_review"
                    && review.steps.iter().all(|s| s.tier == "sol"
                        && s.state == "completed"
                        && s.request_id.is_some()),
                "Automatic write requires completed Sol review receipts"
            );
            ensure!(
                !decisions.iter().any(|d| d.revision.is_some())
                    || review
                        .steps
                        .iter()
                        .any(|s| s.stage == "candidate_verify_revision"),
                "Repaired memories require independent verification"
            );
            let mut outputs = vec![];
            let mut retain = BTreeSet::new();
            // An approved observation need not settle every open question carried
            // by the same source. Keep that source available for later evidence.
            if !input.unresolved.is_empty() {
                retain.extend(input.candidates.iter().map(|c| c.candidate_id.clone()));
            }
            for decision in &decisions {
                let mut output = input.outputs[decision.output_index].clone();
                if decision.verdict == "approve" {
                    if let Some(revision) = &decision.revision {
                        output.title = revision.title.clone();
                        output.content = revision.content.clone();
                    }
                    validate_output(
                        &output,
                        &input
                            .candidates
                            .iter()
                            .map(|c| c.candidate_id.clone())
                            .collect::<Vec<_>>(),
                        &db.state.syntheses[index].offered_revision_ids,
                    )?;
                    outputs.push(output);
                } else {
                    retain.extend(output.candidate_ids.clone());
                }
            }
            review.retain_candidate_ids = retain.into_iter().collect();
            review.decisions = decisions;
            review.reviewed_at = Some(timestamp(Utc::now()));
            review.authorized = !outputs.is_empty();
            review.state = if outputs.is_empty() {
                if review.decisions.iter().any(|d| d.verdict == "needs_input") {
                    "needs_input"
                } else {
                    "accumulating"
                }
            } else {
                "approved"
            }
            .into();
            if !outputs.is_empty() {
                // Persist approval before the first Store write; resume this exact plan after restart.
                db.state.syntheses[index].review_request = Some(SynthesisReview {
                    synthesis_id: input.synthesis_id.clone(),
                    version: input.version,
                    outputs,
                });
            }
        }
        db.state.syntheses[index].automatic_review = Some(review);
        db.save()
    }

    pub(crate) async fn resume_automatic_write(
        &self,
        access: &AccessSession,
        inventory: &[DurablePageInventoryItem],
    ) -> Result<bool> {
        let mut db = LockedState::open(&self.path, self.store.identity_id()).await?;
        if db.state.organization.automatic_review_enabled != Some(true) {
            return Ok(false);
        }
        let Some(index) = db.state.syntheses.iter().position(|s| {
            matches!(s.status.as_str(), "pending" | "promoting")
                && permission(access, &s.scope, AccessPermission::ReadDetail).is_ok()
                && permission(access, &s.scope, AccessPermission::Ingest).is_ok()
                && s.automatic_review.as_ref().is_some_and(|a| {
                    a.authorized && matches!(a.state.as_str(), "approved" | "applying")
                })
        }) else {
            return Ok(false);
        };
        let snapshot = db.state.syntheses[index].clone();
        let review = snapshot.automatic_review.as_ref().unwrap();
        let request = snapshot
            .review_request
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Automatic plan missing"))?;
        if snapshot.status == "pending" {
            let stale = !current(&snapshot, &db.state.candidates)
                || review.input.as_ref().is_none_or(|input| {
                    review_input_changed(input, &db.state.candidates, inventory)
                });
            if stale {
                let auto = db.state.syntheses[index].automatic_review.as_mut().unwrap();
                auto.authorized = false;
                auto.state = "stale".into();
                auto.reason =
                    "Evidence or comparison memory changed; awaiting reorganization".into();
                db.state.syntheses[index].review_request = None;
                for c in &mut db.state.candidates {
                    if snapshot
                        .candidates
                        .iter()
                        .any(|r| r.candidate_id == c.candidate_id)
                        && active(c)
                    {
                        c.organized_version = 0;
                    }
                }
                db.save()?;
                return Ok(true);
            }
            // review_synthesis distinguishes a saved approval from a started Store plan.
            db.state.syntheses[index].review_request = None;
        }
        db.state.syntheses[index]
            .automatic_review
            .as_mut()
            .unwrap()
            .state = "applying".into();
        let result = self.review_synthesis(access, &mut db, request, true).await;
        let auto = db.state.syntheses[index].automatic_review.as_mut().unwrap();
        match result {
            Ok(_) => {
                auto.state = "written".into();
                auto.reason = "Sol-reviewed memories applied; unresolved evidence retained".into();
            }
            Err(error) => {
                auto.state = "write_interrupted".into();
                auto.reason = format!("{error:#}");
            }
        }
        db.save()?;
        Ok(true)
    }
}
