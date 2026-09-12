//! Bounded review/repair/independent-verification paths over immutable evidence.
use super::{
    MaintenanceWorkerOutcome, MaintenanceWorkerRequest, MaintenanceWorkerResponse,
    infer_worker::{
        InferRuntimeSemanticWorker, decode_response, infer_request, operation_name,
        response_defers, response_usage,
    },
    review_budget::{Admission, BudgetStore, ReviewAttempt, ReviewTier, hash},
    worker::{MaintenanceReviewStep, MaintenanceVerification, VerificationVerdict},
};
use anyhow::{Context, Result};
use infer_runtime_client::ResponsesResult;
use serde_json::{Value, json};

#[derive(Debug)]
pub(super) struct ReviewCallFailure {
    pub(super) attempt: ReviewAttempt,
    message: String,
}
impl std::fmt::Display for ReviewCallFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for ReviewCallFailure {}

impl InferRuntimeSemanticWorker {
    pub(super) fn settle_review_response(
        &self,
        attempt: &ReviewAttempt,
        response: &ResponsesResult,
    ) -> Result<()> {
        let usage = response_usage(response);
        BudgetStore::new(self.review_budget.clone()).settle(
            &attempt.key,
            (usage.reported_responses > 0)
                .then(|| usage.input_tokens.saturating_add(usage.output_tokens)),
            Some(serde_json::to_value(response)?),
            &response.status,
        )
    }

    pub(super) async fn review_with_budget(
        &self,
        request: &MaintenanceWorkerRequest,
        mut outcome: MaintenanceWorkerOutcome,
    ) -> Result<MaintenanceWorkerOutcome> {
        if !matches!(request, MaintenanceWorkerRequest::VerifyMaintenance { .. })
            && !self.escalation_operations.contains(operation_name(request))
        {
            return Ok(outcome);
        }
        let seen =
            BudgetStore::new(self.review_budget.clone()).evidence_seen(&evidence_key(request)?);
        match seen {
            Ok(true) => {
                if let MaintenanceWorkerResponse::VerifyMaintenance { assessment } =
                    &mut outcome.response
                {
                    if matches!(assessment.verdict, VerificationVerdict::Approve) {
                        assessment.verdict = VerificationVerdict::NeedsReview;
                    }
                }
            }
            Err(error) => {
                stop(
                    &mut outcome.response,
                    &format!("Review budget unavailable: {error:#}"),
                    "waiting_budget",
                    Vec::new(),
                );
                return Ok(outcome);
            }
            _ => {}
        }
        execute_review(
            request,
            self.review_budget.astra_enabled,
            outcome,
            |request, evidence, tier, stage, instructions| async move {
                self.budgeted_call(&request, &evidence, tier, &stage, &instructions)
                    .await
            },
        )
        .await
    }
}

async fn execute_review<F, Fut>(
    request: &MaintenanceWorkerRequest,
    allow_astra: bool,
    mut outcome: MaintenanceWorkerOutcome,
    mut call: F,
) -> Result<MaintenanceWorkerOutcome>
where
    F: FnMut(MaintenanceWorkerRequest, String, ReviewTier, String, String) -> Fut,
    Fut: std::future::Future<Output = Result<(MaintenanceWorkerOutcome, ReviewAttempt)>>,
{
    sanitize_baseline(&mut outcome.response);
    if !eligible(request, &outcome.response) {
        return Ok(outcome);
    }
    let evidence = evidence_key(request)?;
    let mut current = request.clone();
    let mut approved_revision = None;
    let mut steps = Vec::new();
    for tier in [ReviewTier::Sol, ReviewTier::Astra] {
        if tier == ReviewTier::Astra && !allow_astra {
            break;
        }
        for stage in ["review", "verify_revision"] {
            let instructions = review_instructions(&current, &outcome.response, stage);
            let result = call(
                current.clone(),
                evidence.clone(),
                tier,
                stage.into(),
                instructions,
            )
            .await;
            match result {
                Ok((extra, attempt)) => {
                    steps.push(MaintenanceReviewStep {
                        tier: format!("{tier:?}").to_lowercase(),
                        stage: stage.into(),
                        state: "completed".into(),
                        reason: attempt.reason,
                        request_id: attempt.response_id,
                        actual_tokens: attempt.actual_tokens,
                    });
                    if let Some(usage) = extra.usage {
                        outcome
                            .usage
                            .get_or_insert_with(Default::default)
                            .add_assign(&usage);
                    }
                    outcome.model_attempts =
                        outcome.model_attempts.saturating_add(extra.model_attempts);
                    outcome.escalated = true;
                    outcome.response = extra.response;
                }
                Err(error) => {
                    let reason = format!("{error:#}");
                    let paid = error.downcast_ref::<ReviewCallFailure>();
                    if let Some(failure) = paid {
                        outcome.model_attempts = outcome
                            .model_attempts
                            .saturating_add(u32::from(failure.attempt.state != "not_submitted"));
                        outcome.escalated = true;
                        if let Some(raw) = &failure.attempt.result {
                            if let Ok(response) =
                                serde_json::from_value::<ResponsesResult>(raw.clone())
                            {
                                outcome
                                    .usage
                                    .get_or_insert_with(Default::default)
                                    .add_assign(&response_usage(&response));
                            }
                        }
                    }
                    steps.push(MaintenanceReviewStep {
                        tier: format!("{tier:?}").to_lowercase(),
                        stage: stage.into(),
                        state: "waiting_budget".into(),
                        reason: reason.clone(),
                        request_id: paid.and_then(|f| f.attempt.response_id.clone()),
                        actual_tokens: paid.and_then(|f| f.attempt.actual_tokens),
                    });
                    let state = if paid.is_some_and(|f| {
                        f.attempt.state == "not_submitted" || f.attempt.actual_tokens.is_some()
                    }) {
                        "needs_review"
                    } else if reason.contains("input limit") {
                        "missing_evidence"
                    } else if reason.contains("human review required") {
                        "needs_review"
                    } else {
                        "waiting_budget"
                    };
                    if let Some(step) = steps.last_mut() {
                        step.state = state.into();
                    }
                    stop(&mut outcome.response, &reason, state, steps);
                    return Ok(outcome); // Infrastructure failure never spends Astra.
                }
            }
            let MaintenanceWorkerResponse::VerifyMaintenance { assessment } = &mut outcome.response
            else {
                return Ok(outcome);
            };
            assessment.review_steps.clear();
            assessment.review_state = None;
            normalize(assessment);
            if assessment.requires_user_input {
                assessment.revision = None;
                assessment.verdict = VerificationVerdict::NeedsReview;
                assessment.review_state = Some("missing_evidence".into());
                assessment.review_steps = steps;
                return Ok(outcome);
            }
            if let Some(revision) = assessment.revision.take() {
                if stage == "verify_revision" {
                    assessment.verdict = VerificationVerdict::NeedsReview;
                    assessment.concerns.push(
                        "Independent verification requested another repair; this tier cannot loop."
                            .into(),
                    );
                    break;
                }
                let valid = match &current {
                    MaintenanceWorkerRequest::VerifyMaintenance { kind, .. } if kind == "topic" => {
                        !revision.title.trim().is_empty()
                            && revision.title.chars().count() <= 160
                            && (120..=4000).contains(&revision.content.chars().count())
                    }
                    _ => {
                        !revision.content.trim().is_empty()
                            && revision.content.chars().count() <= 1200
                    }
                };
                if !valid {
                    assessment.verdict = VerificationVerdict::NeedsReview;
                    assessment
                        .concerns
                        .push("Proposed revision violates content bounds.".into());
                    break;
                }
                if let MaintenanceWorkerRequest::VerifyMaintenance { title, content, .. } =
                    &mut current
                {
                    *title = revision.title.clone();
                    *content = revision.content.clone();
                }
                approved_revision = Some(revision);
                continue;
            }
            if !matches!(assessment.verdict, VerificationVerdict::NeedsReview) {
                if matches!(assessment.verdict, VerificationVerdict::Approve) {
                    assessment.revision = approved_revision;
                }
                assessment.review_state = Some(
                    if matches!(assessment.verdict, VerificationVerdict::Approve) {
                        "approved"
                    } else {
                        "no_change"
                    }
                    .into(),
                );
                assessment.review_steps = steps;
                return Ok(outcome);
            }
            break;
        }
    }
    if let MaintenanceWorkerResponse::VerifyMaintenance { assessment } = &mut outcome.response {
        assessment.revision = None;
        assessment.review_state = Some("needs_review".into());
        assessment.review_steps = steps;
    }
    Ok(outcome)
}

impl InferRuntimeSemanticWorker {
    pub(super) async fn budgeted_call(
        &self,
        request: &MaintenanceWorkerRequest,
        evidence: &str,
        tier: ReviewTier,
        stage: &str,
        instructions: &str,
    ) -> Result<(MaintenanceWorkerOutcome, ReviewAttempt)> {
        let store = BudgetStore::new(self.review_budget.clone());
        // Reconcile known remote identities before checking the shared slot.
        for attempt in store.unsettled()? {
            if let Some(id) = &attempt.response_id {
                if let Ok(Ok(response)) =
                    tokio::time::timeout(self.timeout, self.client.get_response(id)).await
                {
                    if matches!(
                        response.status.as_str(),
                        "completed" | "failed" | "cancelled" | "incomplete"
                    ) {
                        self.settle_review_response(&attempt, &response)?;
                    }
                }
            }
        }
        let deployment = match tier {
            ReviewTier::Sol => &self.review_budget.sol_deployment_id,
            ReviewTier::Astra => &self.review_budget.astra_deployment_id,
        };
        let wire = infer_request(
            request,
            self.timeout,
            &self.summary_deployment_id,
            &self.reasoning_deployment_id,
            self.relation_deployment_id.as_deref(),
            Some(deployment),
        )?;
        let bytes = serde_json::to_vec(&wire)?
            .len()
            .saturating_add(instructions.len())
            .saturating_add(1024);
        anyhow::ensure!(
            bytes <= self.review_budget.max_input_bytes,
            "Full evidence exceeds upgraded review input limit; human review required"
        );
        let request_hash = hash(&serde_json::to_string(request)?);
        let admission = store.reserve(
            evidence,
            &request_hash,
            tier,
            stage,
            (bytes as u64).saturating_add(self.review_budget.max_output_tokens as u64),
            if matches!(
                request,
                MaintenanceWorkerRequest::ReviewCandidateSynthesis { .. }
            ) {
                "Candidate memory review before formal write"
            } else {
                "Uncertain maintenance decision or existing Topic refresh"
            },
        )?;
        match admission {
            Admission::Waiting(reason) => anyhow::bail!("{reason}"),
            Admission::Existing(attempt) => {
                anyhow::ensure!(
                    attempt.request_hash == request_hash,
                    "This evidence was already reviewed with different proposal content; explicit human review required"
                );
                let response: ResponsesResult = serde_json::from_value(
                    attempt
                        .result
                        .clone()
                        .context("Previous submission remains unsettled; do not submit again")?,
                )?;
                anyhow::ensure!(
                    response.status == "completed",
                    "Previous review ended with {}; human review required",
                    response.status
                );
                Ok((
                    MaintenanceWorkerOutcome {
                        response: decode_response(&response, request)?,
                        usage: None,
                        model_attempts: 0,
                        escalated: true,
                    },
                    attempt,
                ))
            }
            Admission::Reserved(attempt) => {
                let result = self
                    .evaluate_inner(
                        request,
                        Some(instructions),
                        Some(deployment),
                        Some(&attempt),
                    )
                    .await;
                let actual = store
                    .attempt(&attempt.key)
                    .ok()
                    .flatten()
                    .unwrap_or(attempt);
                match result {
                    Ok(result) => Ok((result, actual)),
                    Err(error) => Err(ReviewCallFailure {
                        attempt: actual,
                        message: format!("{error:#}"),
                    }
                    .into()),
                }
            }
        }
    }
}
pub(super) fn sanitize_baseline(response: &mut MaintenanceWorkerResponse) {
    if let MaintenanceWorkerResponse::VerifyMaintenance { assessment } = response {
        assessment.revision = None;
        assessment.review_steps.clear();
        assessment.review_state = None;
        normalize(assessment);
    }
}

fn normalize(a: &mut MaintenanceVerification) {
    if matches!(a.verdict, VerificationVerdict::Approve)
        && (!a.concerns.is_empty()
            || a.added_information.trim().is_empty()
            || a.preserved_boundaries.trim().is_empty()
            || a.requires_user_input)
    {
        a.verdict = VerificationVerdict::NeedsReview;
    }
}
fn eligible(request: &MaintenanceWorkerRequest, response: &MaintenanceWorkerResponse) -> bool {
    match (request, response) {
        (
            MaintenanceWorkerRequest::VerifyMaintenance {
                refresh_topic_page_id,
                ..
            },
            MaintenanceWorkerResponse::VerifyMaintenance { assessment },
        ) => {
            assessment.review_state.as_deref() != Some("missing_evidence")
                && !matches!(assessment.verdict, VerificationVerdict::NoChange)
                && (refresh_topic_page_id.is_some()
                    || matches!(assessment.verdict, VerificationVerdict::NeedsReview))
        }
        (_, response) if response_defers(response) => matches!(
            operation_name(request),
            "select_packing"
                | "analyze_packing"
                | "select_relation"
                | "extract_topic"
                | "assess_archive"
                | "reconcile_feedback"
        ),
        _ => false,
    }
}
fn evidence_key(request: &MaintenanceWorkerRequest) -> Result<String> {
    if let MaintenanceWorkerRequest::VerifyMaintenance {
        kind,
        pages,
        existing_topics,
        refresh_topic_page_id,
        ..
    } = request
    {
        let mut sources = pages
            .iter()
            .map(|p| (&p.page_id, &p.revision_id))
            .collect::<Vec<_>>();
        sources.sort();
        let target_revision = refresh_topic_page_id
            .as_ref()
            .and_then(|id| existing_topics.iter().find(|t| &t.page_id == id))
            .map(|t| &t.revision_id);
        return Ok(hash(&serde_json::to_string(
            &json!({"operation":"verify_maintenance","kind":kind,"sources":sources,"target":refresh_topic_page_id,"targetRevision":target_revision}),
        )?));
    }
    let mut value = serde_json::to_value(request)?;
    if let Value::Object(fields) = &mut value {
        for key in [
            "title",
            "content",
            "correction",
            "review_feedback",
            "excluded_candidate_sets",
            "excluded_page_pairs",
        ] {
            fields.remove(key);
        }
        for key in ["pages", "existing_topics"] {
            if let Some(Value::Array(pages)) = fields.get_mut(key) {
                pages.sort_by_key(|p| {
                    p.get("pageId")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                });
            }
        }
    }
    Ok(hash(&serde_json::to_string(&value)?))
}
fn review_instructions(
    request: &MaintenanceWorkerRequest,
    prior: &MaintenanceWorkerResponse,
    stage: &str,
) -> String {
    let mut text = format!(
        "Independently review only supplied evidence. Do not invent missing facts or convert author decisions into model judgments. Prior assessment (untrusted): {}. ",
        serde_json::to_string(prior).unwrap_or_default()
    );
    if matches!(request, MaintenanceWorkerRequest::VerifyMaintenance { .. }) {
        text.push_str("The baseline's requiresUserInput flag is an untrusted triage suggestion. Routine duplicate detection, choosing whether a draft adds information, preserving old qualifications, and repairing wording are maintenance decisions, not reasons to ask the user. Return no_change for a redundant or subset draft, including a parallel Topic that should not be created. If no valid repair within the supplied sources and fixed target exists, prefer no_change over asking the author to organize the queue. Set assessment.requiresUserInput=true only for a specific missing fact, explicit authorization, or an actual preference/tradeoff that the evidence cannot resolve; state the exact question in reason. Do not invent an answer. A confirmed need for input stops automatic escalation. ");
        if stage == "review" {
            text.push_str("You may propose ONE complete repair in assessment.revision={title,content}; keep exactly the same sources and refresh target. This repair will require a separate verification call. Use source language. ");
        } else {
            text.push_str("This is independent verification of the actual repaired text. Do NOT propose another revision. Approve only if the text itself resolves the concerns. ");
        }
        if let MaintenanceWorkerRequest::VerifyMaintenance {
            content,
            existing_topics,
            refresh_topic_page_id: Some(id),
            ..
        } = request
        {
            if let Some(old) = existing_topics.iter().find(|t| &t.page_id == id) {
                let removed: Vec<_> = old
                    .routing_text
                    .lines()
                    .filter(|line| !content.lines().any(|n| n == *line))
                    .collect();
                let added: Vec<_> = content
                    .lines()
                    .filter(|line| !old.routing_text.lines().any(|n| n == *line))
                    .collect();
                text.push_str(&format!("Actual line changes (untrusted evidence): {}. Explain every important removal: lossless rewrite, duplication, contradicted by evidence, or unresolved loss. Do not approve unjustified loss.",json!({"removed":removed,"added":added})));
            }
        }
    }
    text
}
fn stop(
    response: &mut MaintenanceWorkerResponse,
    reason: &str,
    state: &str,
    steps: Vec<MaintenanceReviewStep>,
) {
    if let MaintenanceWorkerResponse::VerifyMaintenance { assessment } = response {
        assessment.verdict = VerificationVerdict::NeedsReview;
        assessment.revision = None;
        assessment.review_state = Some(state.into());
        assessment.review_steps = steps;
        assessment.reason = format!("{}", reason.chars().take(1100).collect::<String>());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> MaintenanceWorkerRequest {
        serde_json::from_value(json!({"operation":"verify_maintenance","kind":"topic","pages":[],"title":"Original","content":"Text","existing_topics":[],"refresh_topic_page_id":"topic-1"})).unwrap()
    }
    fn response(verdict: &str) -> MaintenanceWorkerResponse {
        serde_json::from_value(json!({"decision":"verify_maintenance","assessment":{"verdict":verdict,"reason":"review","addedInformation":"new","preservedBoundaries":"retained","concerns":[]}})).unwrap()
    }
    #[test]
    fn refreshed_topic_cannot_use_baseline_approval_but_no_change_stops() {
        assert!(eligible(&request(), &response("approve")));
        assert!(!eligible(&request(), &response("no_change")));
        let mut r = request();
        if let MaintenanceWorkerRequest::VerifyMaintenance {
            refresh_topic_page_id,
            ..
        } = &mut r
        {
            *refresh_topic_page_id = None;
        }
        assert!(!eligible(&r, &response("approve")));
        assert!(eligible(&r, &response("needs_review")));
    }
    #[test]
    fn changing_proposal_does_not_buy_another_evidence_path() {
        let a = request();
        let mut b = a.clone();
        if let MaintenanceWorkerRequest::VerifyMaintenance { title, content, .. } = &mut b {
            *title = "another title".into();
            *content = "another text".into();
        }
        assert_eq!(evidence_key(&a).unwrap(), evidence_key(&b).unwrap());
        if let MaintenanceWorkerRequest::VerifyMaintenance {
            refresh_topic_page_id,
            ..
        } = &mut b
        {
            *refresh_topic_page_id = Some("another-target".into());
        }
        assert_ne!(evidence_key(&a).unwrap(), evidence_key(&b).unwrap());
    }
    fn outcome(response: MaintenanceWorkerResponse) -> MaintenanceWorkerOutcome {
        MaintenanceWorkerOutcome {
            response,
            usage: Some(pcp_core::ModelTokenUsage {
                reported_responses: 1,
                input_tokens: 10,
                output_tokens: 5,
                total_tokens: 15,
                ..Default::default()
            }),
            model_attempts: 1,
            escalated: false,
        }
    }
    fn attempt(tier: ReviewTier, stage: String) -> ReviewAttempt {
        ReviewAttempt {
            key: stage.clone(),
            evidence_key: "e".into(),
            request_hash: "h".into(),
            tier,
            stage,
            deployment: "test".into(),
            effort: tier.effort().into(),
            submitted_at_ms: 0,
            reserved_tokens: 20,
            actual_tokens: Some(15),
            response_id: Some("response".into()),
            state: "completed".into(),
            reason: "test".into(),
            result: None,
        }
    }
    #[tokio::test]
    async fn repair_is_independently_verified_and_returns_actual_revised_text() {
        let repaired = "A complete repaired topic preserves the original source attribution and the distinction between a user decision and a model proposal. It must retain all original boundaries.";
        let mut first = response("needs_review");
        if let MaintenanceWorkerResponse::VerifyMaintenance { assessment } = &mut first {
            assessment.revision = Some(super::super::worker::VerifiedMaintenanceRevision {
                title: "Repaired".into(),
                content: repaired.into(),
            });
        }
        let mut answers = std::collections::VecDeque::from([first, response("approve")]);
        let mut calls = Vec::new();
        let result = execute_review(
            &request(),
            true,
            outcome(response("approve")),
            |r, _, tier, stage, instructions| {
                calls.push((r, tier, stage.clone(), instructions));
                std::future::ready(Ok((
                    outcome(answers.pop_front().unwrap()),
                    attempt(tier, stage),
                )))
            },
        )
        .await
        .unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].1, ReviewTier::Sol);
        assert_eq!(calls[1].2, "verify_revision");
        let MaintenanceWorkerRequest::VerifyMaintenance { content, .. } = &calls[1].0 else {
            panic!()
        };
        assert_eq!(content, repaired);
        assert!(calls[1].3.contains("Do NOT propose another revision"));
        let MaintenanceWorkerResponse::VerifyMaintenance { assessment } = result.response else {
            panic!()
        };
        assert_eq!(assessment.revision.unwrap().content, repaired);
        assert_eq!(assessment.review_steps.len(), 2);
        assert_eq!(result.model_attempts, 3);
        assert_eq!(result.usage.unwrap().total_tokens, 45);
    }
    #[tokio::test]
    async fn sol_infrastructure_failure_never_spends_astra_or_approves_refresh() {
        let mut calls = 0;
        let result = execute_review(
            &request(),
            true,
            outcome(response("approve")),
            |_, _, tier, _, _| {
                calls += 1;
                assert_eq!(tier, ReviewTier::Sol);
                std::future::ready(Err(anyhow::anyhow!("Rolling review budget exhausted")))
            },
        )
        .await
        .unwrap();
        assert_eq!(calls, 1);
        let MaintenanceWorkerResponse::VerifyMaintenance { assessment } = result.response else {
            panic!()
        };
        assert!(matches!(
            assessment.verdict,
            VerificationVerdict::NeedsReview
        ));
        assert_eq!(assessment.review_state.as_deref(), Some("waiting_budget"));
    }
    #[tokio::test]
    async fn every_tier_can_repair_only_once_and_missing_evidence_stops() {
        let mut repair = response("needs_review");
        if let MaintenanceWorkerResponse::VerifyMaintenance { assessment } = &mut repair {
            assessment.revision=Some(super::super::worker::VerifiedMaintenanceRevision{title:"Repair".into(),content:"Source attribution and uncertainty must remain explicit in this repaired topic, without silently converting a model suggestion into the user's decision.".into()});
        }
        let mut calls = Vec::new();
        let result = execute_review(
            &request(),
            true,
            outcome(response("needs_review")),
            |_, _, tier, stage, _| {
                calls.push((tier, stage.clone()));
                std::future::ready(Ok((outcome(repair.clone()), attempt(tier, stage))))
            },
        )
        .await
        .unwrap();
        assert_eq!(
            calls,
            vec![
                (ReviewTier::Sol, "review".into()),
                (ReviewTier::Sol, "verify_revision".into()),
                (ReviewTier::Astra, "review".into()),
                (ReviewTier::Astra, "verify_revision".into())
            ]
        );
        let MaintenanceWorkerResponse::VerifyMaintenance { assessment } = result.response else {
            panic!()
        };
        assert!(assessment.revision.is_none());
        assert!(matches!(
            assessment.verdict,
            VerificationVerdict::NeedsReview
        ));
        let mut missing = response("needs_review");
        if let MaintenanceWorkerResponse::VerifyMaintenance { assessment } = &mut missing {
            assessment.requires_user_input = true;
        }
        let mut calls = 0;
        execute_review(
            &request(),
            true,
            outcome(missing.clone()),
            |_, _, tier, stage, _| {
                calls += 1;
                std::future::ready(Ok((outcome(missing.clone()), attempt(tier, stage))))
            },
        )
        .await
        .unwrap();
        // One stronger reviewer confirms a real missing-input boundary. It
        // stops here, without trying repairs or escalating to Astra.
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn baseline_author_flag_cannot_skip_duplicate_triage() {
        let mut baseline = response("needs_review");
        if let MaintenanceWorkerResponse::VerifyMaintenance { assessment } = &mut baseline {
            assessment.requires_user_input = true;
            assessment.reason = "Ask the author which duplicate topic to keep".into();
        }
        let mut calls = 0;
        let reviewed = execute_review(
            &request(),
            true,
            outcome(baseline),
            |_, _, tier, stage, instructions| {
                calls += 1;
                assert!(instructions.contains("Routine duplicate detection"));
                std::future::ready(Ok((outcome(response("no_change")), attempt(tier, stage))))
            },
        )
        .await
        .unwrap();
        assert_eq!(calls, 1);
        let MaintenanceWorkerResponse::VerifyMaintenance { assessment } = reviewed.response else {
            panic!()
        };
        assert!(matches!(assessment.verdict, VerificationVerdict::NoChange));
        assert_eq!(assessment.review_steps.len(), 1);
    }
}
