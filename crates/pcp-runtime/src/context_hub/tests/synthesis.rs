use super::*;
use crate::context_hub::synthesis::{OrganizationInput, ProposedSynthesis};
use crate::maintenance::{
    MaintenanceConfig, MaintenanceWorkerRequest, MaintenanceWorkerResponse, RuntimeMaintainer,
    SemanticMaintenanceWorker,
};
use pcp_client::context_hub::{SynthesisOutput, SynthesisReview};
use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) async fn inputs(r: &Rig, count: usize) -> OrganizationInput {
    r.hub.enable_organization();
    r.enable("writer").await;
    for n in 0..count {
        r.client("writer", &["a"])
            .context_hub(ContextHubRequest::SubmitCandidate(candidate(&format!(
                "e{n}"
            ))))
            .await
            .unwrap();
    }
    r.admin
        .context_hub(ContextHubRequest::OrganizeCandidates)
        .await
        .unwrap();
    let inventory = r.admin.durable_page_inventory(vec![]).await.unwrap();
    r.hub
        .prepare_organization(r.admin.as_ref(), &inventory)
        .await
        .unwrap()
        .unwrap()
}
pub(super) fn proposed(input: &OrganizationInput) -> ProposedSynthesis {
    let ids = input
        .candidates
        .iter()
        .map(|c| c.candidate_id.clone())
        .collect::<Vec<_>>();
    ProposedSynthesis {
        candidate_ids: ids.clone(),
        title: "PCP 候选逐步形成记忆".into(),
        narrative: "初步目标、后续约束与最终选择按来源时间保留；未定事项仍明确标注。".into(),
        reason: "已形成明确选择，可供后续设计使用。".into(),
        unresolved: vec!["下一轮效果尚待观察。".into()],
        maturity: "ready".into(),
        outputs: vec![SynthesisOutput {
            candidate_ids: ids,
            title: "候选处理决定".into(),
            content: "相关候选经整理后再由用户审阅。".into(),
            action: "create".into(),
            target_revision_id: None,
        }],
    }
}
async fn review_request(r: &Rig) -> SynthesisReview {
    let snapshot = r
        .admin
        .context_hub(ContextHubRequest::Inspect)
        .await
        .unwrap();
    let group = snapshot["syntheses"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|s| s["status"] == "pending")
        .unwrap();
    SynthesisReview {
        synthesis_id: group["synthesisId"].as_str().unwrap().into(),
        version: group["version"].as_u64().unwrap(),
        outputs: serde_json::from_value(group["outputs"].clone()).unwrap(),
    }
}

#[tokio::test]
async fn evolution_preserves_pending_evidence_and_requires_new_information() {
    let r = Rig::new().await;
    let input = inputs(&r, 2).await;
    assert!(
        r.hub
            .prepare_organization(r.admin.as_ref(), &[])
            .await
            .unwrap()
            .is_none(),
        "in-flight lease blocks duplicate work"
    );
    let proposal = proposed(&input);
    r.hub
        .finish_organization(&input, vec![proposal])
        .await
        .unwrap();
    assert_eq!(
        r.admin.page_count(vec![]).await.unwrap(),
        0,
        "organization only creates drafts"
    );
    assert!(
        r.hub
            .prepare_organization(r.admin.as_ref(), &[])
            .await
            .unwrap()
            .is_none(),
        "unchanged candidates do not trigger another model call"
    );
    let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
        .await
        .unwrap();
    for c in &mut db.state.candidates {
        c.expires_at = "2000-01-01T00:00:00.000Z".into();
    }
    db.save().unwrap();
    drop(db);
    let snap = r
        .admin
        .context_hub(ContextHubRequest::Inspect)
        .await
        .unwrap();
    assert_eq!(
        snap["candidates"].as_array().unwrap().len(),
        2,
        "pending candidates survive former TTL"
    );
    r.client("writer", &["a"])
        .context_hub(ContextHubRequest::SubmitCandidate(candidate(
            "new-evidence",
        )))
        .await
        .unwrap();
    let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
        .await
        .unwrap();
    for c in &mut db.state.candidates {
        c.created_at = "2020-01-01T00:00:00.000Z".into();
    }
    db.save().unwrap();
    drop(db);
    let next = r
        .hub
        .prepare_organization(r.admin.as_ref(), &[])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next.candidates.len(), 3);
    assert_eq!(
        next.previous.len(),
        1,
        "previous interpretation accompanies original evidence"
    );
}

#[tokio::test]
async fn malformed_or_stale_synthesis_cannot_consume_evidence() {
    let r = Rig::new().await;
    let input = inputs(&r, 2).await;
    let mut bad = proposed(&input);
    bad.candidate_ids.pop();
    assert!(r.hub.finish_organization(&input, vec![bad]).await.is_err());
    let mut bad = proposed(&input);
    bad.outputs[0].candidate_ids = vec!["invented".into()];
    assert!(r.hub.finish_organization(&input, vec![bad]).await.is_err());
    let mut bad = proposed(&input);
    bad.outputs[0].action = "represented".into();
    bad.outputs[0].target_revision_id = Some("unoffered".into());
    assert!(r.hub.finish_organization(&input, vec![bad]).await.is_err());
    let mut rejected = promote(&json!({"candidateId":input.candidates[0].candidate_id}));
    rejected.action = CandidateAction::Reject;
    r.admin
        .context_hub(ContextHubRequest::Review(rejected))
        .await
        .unwrap();
    assert!(
        r.hub
            .finish_organization(&input, vec![proposed(&input)])
            .await
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
    assert_eq!(r.admin.page_count(vec![]).await.unwrap(), 0);
}

#[tokio::test]
async fn multiple_memories_use_exact_evidence_and_recover_without_duplicate_pages() {
    let r = Rig::new().await;
    let input = inputs(&r, 3).await;
    let mut proposal = proposed(&input);
    proposal.outputs[0].candidate_ids = vec![input.candidates[0].candidate_id.clone()];
    let mut second = proposal.outputs[0].clone();
    second.title = "独立的失败经验".into();
    second.content = "同一过程可以形成独立的经验。".into();
    proposal.outputs.push(second);
    r.hub
        .finish_organization(&input, vec![proposal])
        .await
        .unwrap();
    let request = review_request(&r).await;
    let tenant = r.client("writer", &["a"]);
    assert!(
        tenant
            .context_hub(ContextHubRequest::OrganizeCandidates)
            .await
            .is_err()
    );
    assert!(
        tenant
            .context_hub(ContextHubRequest::ReviewSynthesis(request.clone()))
            .await
            .is_err()
    );
    let result = r
        .admin
        .context_hub(ContextHubRequest::ReviewSynthesis(request.clone()))
        .await
        .unwrap();
    assert_eq!(result["outputs"].as_array().unwrap().len(), 2);
    assert_eq!(r.admin.page_count(vec![]).await.unwrap(), 2);
    let snapshot = r
        .admin
        .context_hub(ContextHubRequest::Inspect)
        .await
        .unwrap();
    assert_eq!(
        snapshot["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["status"] == "pending")
            .count(),
        2,
        "unused candidates remain pending"
    );
    assert_eq!(
        result,
        r.admin
            .context_hub(ContextHubRequest::ReviewSynthesis(request.clone()))
            .await
            .unwrap()
    );
    // Reconstruct a durable in-flight state with a missing post-write receipt. Store
    // event keys must recover the same outputs even after reopening ContextHub.
    let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
        .await
        .unwrap();
    let group = &mut db.state.syntheses[0];
    group.status = "promoting".into();
    group.version = 1;
    group.results.clear();
    let c = db
        .state
        .candidates
        .iter_mut()
        .find(|c| c.candidate_id == input.candidates[0].candidate_id)
        .unwrap();
    c.status = "promoting".into();
    c.version = 1;
    db.save().unwrap();
    drop(db);
    let reopened = ContextHub::new(r.store.clone(), r.hub.path.clone());
    let recovered = reopened
        .execute(
            r.admin.access(),
            ContextHubRequest::ReviewSynthesis(request.clone()),
        )
        .await
        .unwrap();
    assert_eq!(recovered, result);
    assert_eq!(r.admin.page_count(vec![]).await.unwrap(), 2);
    let mut changed = request;
    changed.outputs[0].content = "different plan".into();
    assert!(
        reopened
            .execute(
                r.admin.access(),
                ContextHubRequest::ReviewSynthesis(changed)
            )
            .await
            .is_err()
    );
}

struct OrganizerWorker {
    calls: AtomicUsize,
    fail: bool,
}
#[async_trait]
impl SemanticMaintenanceWorker for OrganizerWorker {
    async fn evaluate(
        &self,
        request: MaintenanceWorkerRequest,
    ) -> anyhow::Result<MaintenanceWorkerResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            anyhow::bail!("provider unavailable");
        }
        match request {
            MaintenanceWorkerRequest::OrganizeCandidates { input } => {
                Ok(MaintenanceWorkerResponse::CandidateSyntheses {
                    groups: vec![proposed(&input)],
                })
            }
            _ => anyhow::bail!("unexpected worker operation"),
        }
    }
}
pub(super) fn config(r: &Rig) -> MaintenanceConfig {
    toml::from_str(&format!(
        r#"
enabled = true
state_path = {:?}
store_wide = true
max_jobs_per_cycle = 1
[periodic_review]
enabled = false
[worker]
provider = "command"
program = "/bin/false"
actor_id = "model:organizer-test"
"#,
        r.root.join("maintenance.json").to_string_lossy()
    ))
    .unwrap()
}
#[tokio::test]
async fn scheduled_candidate_work_runs_without_any_page_write_and_respects_budget() {
    let r = Rig::new().await;
    r.enable("writer").await;
    r.client("writer", &["a"])
        .context_hub(ContextHubRequest::SubmitCandidate(candidate("scheduler")))
        .await
        .unwrap();
    let worker = Arc::new(OrganizerWorker {
        calls: AtomicUsize::new(0),
        fail: false,
    });
    let mut maintainer = RuntimeMaintainer::for_test(r.admin.clone(), worker.clone(), config(&r))
        .with_context_hub(r.hub.clone());
    assert_eq!(
        maintainer.run_scheduled_cycle().await.unwrap().worker_calls,
        0,
        "coalesce before model work"
    );
    r.admin
        .context_hub(ContextHubRequest::OrganizeCandidates)
        .await
        .unwrap();
    let report = maintainer.run_scheduled_cycle().await.unwrap();
    assert_eq!(report.jobs_advanced, 1);
    assert_eq!(report.worker_calls, 1);
    assert_eq!(r.admin.page_count(vec![]).await.unwrap(), 0);
    assert_eq!(
        maintainer.run_scheduled_cycle().await.unwrap().worker_calls,
        0
    );
    assert_eq!(worker.calls.load(Ordering::SeqCst), 1);
    let snapshot = r
        .admin
        .context_hub(ContextHubRequest::Inspect)
        .await
        .unwrap();
    assert_eq!(snapshot["syntheses"].as_array().unwrap().len(), 1);
}
#[tokio::test]
async fn provider_failure_is_visible_and_does_not_spin_or_expire_candidates() {
    let r = Rig::new().await;
    r.enable("writer").await;
    r.hub.enable_organization();
    r.client("writer", &["a"])
        .context_hub(ContextHubRequest::SubmitCandidate(candidate("failure")))
        .await
        .unwrap();
    r.admin
        .context_hub(ContextHubRequest::OrganizeCandidates)
        .await
        .unwrap();
    let worker = Arc::new(OrganizerWorker {
        calls: AtomicUsize::new(0),
        fail: true,
    });
    let mut maintainer = RuntimeMaintainer::for_test(r.admin.clone(), worker.clone(), config(&r))
        .with_context_hub(r.hub.clone());
    maintainer.run_scheduled_cycle().await.unwrap();
    maintainer.run_scheduled_cycle().await.unwrap();
    assert_eq!(worker.calls.load(Ordering::SeqCst), 1);
    let snapshot = r
        .admin
        .context_hub(ContextHubRequest::Inspect)
        .await
        .unwrap();
    assert!(
        snapshot["organization"]["error"]
            .as_str()
            .unwrap()
            .contains("provider unavailable")
    );
    assert!(snapshot["organization"]["nextAttemptAt"].is_string());
    assert_eq!(snapshot["candidates"][0]["status"], "pending");
}

#[tokio::test]
async fn organization_is_scope_bound_and_operator_controls_require_a_worker() {
    let r = Rig::new().await;
    assert!(
        r.admin
            .context_hub(ContextHubRequest::OrganizeCandidates)
            .await
            .is_err()
    );
    r.enable("writer").await;
    let a = r.client("writer", &["a", "b"]);
    for scope in ["a", "b"] {
        let mut c = candidate(scope);
        c.scope = scope.into();
        a.context_hub(ContextHubRequest::SubmitCandidate(c))
            .await
            .unwrap();
    }
    r.hub.enable_organization();
    r.admin
        .context_hub(ContextHubRequest::OrganizeCandidates)
        .await
        .unwrap();
    let input = r
        .hub
        .prepare_organization(r.client("reader", &["b"]).as_ref(), &[])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(input.scope, "b");
    assert_eq!(input.candidates.len(), 1);
}

pub(super) async fn existing_page(r: &Rig) -> pcp_core::WriteResult {
    r.admin.write_page(serde_json::from_value(json!({
        "namespace":"a", "lifecycleStatus":"active", "kind":"note", "mutability":"revisioned",
        "createdBy":{"actorType":"user","actorId":"operator:test"},
        "payload":{"mediaType":"text/markdown","content":"PCP 候选记忆：原始决定与独立限定。"},
        "facets":{"keep":"original facet"},
        "sourceRefs":[{"providerId":"fixture","locator":"local:original"}]
    })).unwrap()).await.unwrap()
}

#[tokio::test]
async fn update_preserves_original_sources_metadata_and_rejects_stale_targets() {
    let r = Rig::new().await;
    let existing = existing_page(&r).await;
    let input = inputs(&r, 1).await;
    assert!(
        input
            .updateable_revision_ids
            .contains(&existing.revision_id)
    );
    let mut proposal = proposed(&input);
    proposal.outputs[0].action = "update".into();
    proposal.outputs[0].target_revision_id = Some(existing.revision_id.clone());
    proposal.outputs[0].content = "原始决定与独立限定保持；现在相关候选先整理再审阅。".into();
    r.hub
        .finish_organization(&input, vec![proposal])
        .await
        .unwrap();
    let request = review_request(&r).await;
    let receipt = r
        .admin
        .context_hub(ContextHubRequest::ReviewSynthesis(request.clone()))
        .await
        .unwrap();
    assert_eq!(receipt["outputs"][0]["pageId"], existing.page_id);
    assert_eq!(r.admin.page_count(vec![]).await.unwrap(), 1);
    let page = r
        .admin
        .read_pages(ReadPagesRequest {
            page_ids: vec![existing.page_id.clone()],
            revision_ids: vec![],
            projections: vec![
                Projection::Payload,
                Projection::Sources,
                Projection::Facets,
                Projection::Provenance,
            ],
            max_chars: 5000,
        })
        .await
        .unwrap();
    assert_eq!(page[0].revision.source_refs.len(), 1);
    assert_eq!(
        page[0].revision.facets.as_ref().unwrap()["keep"],
        "original facet"
    );
    assert!(
        page[0]
            .revision
            .provenance
            .iter()
            .any(|p| p.input_revision_ids.contains(&existing.revision_id))
    );
    assert_eq!(
        receipt,
        r.admin
            .context_hub(ContextHubRequest::ReviewSynthesis(request.clone()))
            .await
            .unwrap()
    );
    // A separately staged proposal against the previous head must fail before writing.
    r.client("writer", &["a"])
        .context_hub(ContextHubRequest::SubmitCandidate(candidate("new")))
        .await
        .unwrap();
    r.admin
        .context_hub(ContextHubRequest::OrganizeCandidates)
        .await
        .unwrap();
    let mut next = r
        .hub
        .prepare_organization(
            r.admin.as_ref(),
            &r.admin.durable_page_inventory(vec![]).await.unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
    next.pages = input.pages.clone();
    next.updateable_revision_ids = vec![existing.revision_id.clone()];
    let mut stale = proposed(&next);
    stale.outputs[0].action = "update".into();
    stale.outputs[0].target_revision_id = Some(existing.revision_id);
    r.hub.finish_organization(&next, vec![stale]).await.unwrap();
    let review = review_request(&r).await;
    assert!(
        r.admin
            .context_hub(ContextHubRequest::ReviewSynthesis(review))
            .await
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
    assert_eq!(r.admin.page_count(vec![]).await.unwrap(), 1);
}

#[tokio::test]
async fn deferral_keeps_the_group_and_new_evidence_can_reopen_it() {
    let r = Rig::new().await;
    let input = inputs(&r, 2).await;
    r.hub
        .finish_organization(&input, vec![proposed(&input)])
        .await
        .unwrap();
    let review = CandidateReview {
        candidates: input
            .candidates
            .iter()
            .map(|c| CandidateVersion {
                candidate_id: c.candidate_id.clone(),
                version: c.version,
            })
            .collect(),
        action: CandidateAction::Defer,
        title: None,
        content: None,
        target_revision_id: None,
    };
    r.admin
        .context_hub(ContextHubRequest::Review(review))
        .await
        .unwrap();
    let snapshot = r
        .admin
        .context_hub(ContextHubRequest::Inspect)
        .await
        .unwrap();
    assert_eq!(snapshot["syntheses"][0]["status"], "pending");
    assert_eq!(snapshot["syntheses"][0]["maturity"], "accumulating");
    assert_eq!(snapshot["syntheses"][0]["version"], 2);
    assert!(
        r.hub
            .prepare_organization(r.admin.as_ref(), &[])
            .await
            .unwrap()
            .is_none()
    );
    r.client("writer", &["a"])
        .context_hub(ContextHubRequest::SubmitCandidate(candidate(
            "after-deferral",
        )))
        .await
        .unwrap();
    let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
        .await
        .unwrap();
    db.state.candidates.last_mut().unwrap().created_at = "2020-01-01T00:00:00.000Z".into();
    db.save().unwrap();
    drop(db);
    let next = r
        .hub
        .prepare_organization(r.admin.as_ref(), &[])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        next.candidates.len(),
        3,
        "new evidence may revisit deferred peers without waiting seven days"
    );
}

#[tokio::test]
async fn changes_to_compared_pages_reopen_a_pending_synthesis_once() {
    let r = Rig::new().await;
    existing_page(&r).await;
    let input = inputs(&r, 1).await;
    r.hub
        .finish_organization(&input, vec![proposed(&input)])
        .await
        .unwrap();
    let inventory = r.admin.durable_page_inventory(vec![]).await.unwrap();
    assert!(
        r.hub
            .prepare_organization(r.admin.as_ref(), &inventory)
            .await
            .unwrap()
            .is_none()
    );
    // Inventory is the scheduler's authoritative current-head view. A removed
    // compared Page constitutes new evidence even without a new candidate.
    let reopened = r
        .hub
        .prepare_organization(r.admin.as_ref(), &[])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reopened.candidates.len(), 1);
    assert!(reopened.pages.is_empty());
    r.hub
        .finish_organization(&reopened, vec![proposed(&reopened)])
        .await
        .unwrap();
    assert!(
        r.hub
            .prepare_organization(r.admin.as_ref(), &[])
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn operator_can_stop_a_partial_plan_without_discarding_pages_or_evidence() {
    let r = Rig::new().await;
    let input = inputs(&r, 1).await;
    let mut proposal = proposed(&input);
    proposal.outputs.push(SynthesisOutput {
        title: "Second independent memory".into(),
        ..proposal.outputs[0].clone()
    });
    r.hub
        .finish_organization(&input, vec![proposal])
        .await
        .unwrap();
    let request = review_request(&r).await;
    r.admin
        .context_hub(ContextHubRequest::ReviewSynthesis(request.clone()))
        .await
        .unwrap();
    let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
        .await
        .unwrap();
    db.state.syntheses[0].status = "promoting".into();
    db.state.syntheses[0].version = 1;
    db.state.syntheses[0].results.truncate(1);
    db.state.candidates[0].status = "promoting".into();
    db.state.candidates[0].version = 1;
    db.save().unwrap();
    drop(db);
    let stop = pcp_client::context_hub::SynthesisStop {
        synthesis_id: request.synthesis_id.clone(),
        version: 1,
    };
    assert!(
        r.client("writer", &["a"])
            .context_hub(ContextHubRequest::StopSynthesis(stop.clone()))
            .await
            .is_err()
    );
    let result = r
        .admin
        .context_hub(ContextHubRequest::StopSynthesis(stop.clone()))
        .await
        .unwrap();
    assert_eq!(result["confirmedOutputs"].as_array().unwrap().len(), 1);
    assert_eq!(
        r.admin.page_count(vec![]).await.unwrap(),
        2,
        "even an unconfirmed Page remains intact"
    );
    assert_eq!(
        result,
        r.admin
            .context_hub(ContextHubRequest::StopSynthesis(stop))
            .await
            .unwrap()
    );
    let snapshot = r
        .admin
        .context_hub(ContextHubRequest::Inspect)
        .await
        .unwrap();
    assert_eq!(snapshot["candidates"][0]["status"], "pending");
    assert_eq!(snapshot["candidates"][0]["version"], 2);
    assert_eq!(snapshot["syntheses"][0]["status"], "interrupted");
    assert!(
        r.admin
            .context_hub(ContextHubRequest::ReviewSynthesis(request))
            .await
            .is_err(),
        "old approval cannot resume after explicit stop"
    );
}

#[tokio::test]
async fn worker_handles_map_only_to_the_exact_offered_evidence() {
    let r = Rig::new().await;
    existing_page(&r).await;
    let input = inputs(&r, 2).await;
    let view = input.worker_view();
    assert_eq!(view.candidates[0].candidate_id, "c1");
    assert_eq!(view.pages[0].revision_id, "r1");
    let groups = input.canonical_groups(vec![proposed(&view)]).unwrap();
    assert_eq!(groups[0].candidate_ids[0], input.candidates[0].candidate_id);
    r.hub.finish_organization(&input, groups).await.unwrap();
    let mut bad = proposed(&view);
    bad.outputs[0].candidate_ids = vec!["c99".into()];
    assert!(input.canonical_groups(vec![bad]).is_err());
}

struct InvalidOrganizer(AtomicUsize);
#[async_trait]
impl SemanticMaintenanceWorker for InvalidOrganizer {
    async fn evaluate(
        &self,
        _: MaintenanceWorkerRequest,
    ) -> anyhow::Result<MaintenanceWorkerResponse> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(MaintenanceWorkerResponse::Defer)
    }
}
#[tokio::test]
async fn invalid_model_results_do_not_repeat_without_new_evidence_or_operator_retry() {
    let r = Rig::new().await;
    r.hub.enable_organization();
    r.enable("writer").await;
    r.client("writer", &["a"])
        .context_hub(ContextHubRequest::SubmitCandidate(candidate(
            "invalid-model",
        )))
        .await
        .unwrap();
    r.admin
        .context_hub(ContextHubRequest::OrganizeCandidates)
        .await
        .unwrap();
    let worker = Arc::new(InvalidOrganizer(AtomicUsize::new(0)));
    let mut maintainer = RuntimeMaintainer::for_test(r.admin.clone(), worker.clone(), config(&r))
        .with_context_hub(r.hub.clone());
    maintainer.run_scheduled_cycle().await.unwrap();
    maintainer.run_scheduled_cycle().await.unwrap();
    assert_eq!(worker.0.load(Ordering::SeqCst), 1);
    let snapshot = r
        .admin
        .context_hub(ContextHubRequest::Inspect)
        .await
        .unwrap();
    assert_eq!(snapshot["organization"]["retryWhenChanged"], true);
    assert_eq!(snapshot["candidates"][0]["status"], "pending");
    r.admin
        .context_hub(ContextHubRequest::OrganizeCandidates)
        .await
        .unwrap();
    maintainer.run_scheduled_cycle().await.unwrap();
    assert_eq!(worker.0.load(Ordering::SeqCst), 2);
}
