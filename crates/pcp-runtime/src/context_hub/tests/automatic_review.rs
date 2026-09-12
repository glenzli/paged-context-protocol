use super::synthesis::{inputs, proposed};
use super::*;
use crate::context_hub::automatic_review::{CandidateReviewInput, OutputAssessment};
use crate::maintenance::MaintenanceReviewStep;

async fn setup(count: usize, overlap: bool) -> (Rig, CandidateReviewInput) {
    let r = Rig::new().await;
    r.admin
        .context_hub(ContextHubRequest::SetAutomaticReview { enabled: true })
        .await
        .unwrap();
    let input = inputs(&r, count).await;
    let mut group = proposed(&input);
    if overlap {
        group.outputs.push(group.outputs[0].clone());
        group.outputs[1].title = "仍待观察的假说".into();
    }
    r.hub
        .finish_organization(&input, vec![group])
        .await
        .unwrap();
    assert!(
        r.hub
            .prepare_automatic_review(r.admin.as_ref(), &[])
            .await
            .unwrap()
            .is_none()
    );
    age(&r).await;
    let input = r
        .hub
        .prepare_automatic_review(r.admin.as_ref(), &[])
        .await
        .unwrap()
        .unwrap();
    (r, input)
}
async fn age(r: &Rig) {
    let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
        .await
        .unwrap();
    let before = timestamp(Utc::now() - Duration::minutes(6));
    for group in &mut db.state.syntheses {
        group.created_at = before.clone();
    }
    for c in &mut db.state.candidates {
        c.created_at = before.clone();
    }
    db.save().unwrap();
}
fn assessment(index: usize, verdict: &str) -> OutputAssessment {
    OutputAssessment {
        output_index: index,
        verdict: verdict.into(),
        reason: "已逐项核对原始依据，保留观察与假说的区别。".into(),
        revision: None,
    }
}
fn steps() -> Vec<MaintenanceReviewStep> {
    vec![MaintenanceReviewStep {
        tier: "sol".into(),
        stage: "candidate_review".into(),
        state: "completed".into(),
        reason: "codex_gpt_5_6_sol".into(),
        request_id: Some("review-1".into()),
        actual_tokens: Some(1000),
    }]
}
async fn finish(r: &Rig, input: &CandidateReviewInput, decisions: Vec<OutputAssessment>) {
    r.hub
        .finish_automatic_review(
            input,
            decisions,
            steps(),
            "completed",
            "Source-based review",
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn partial_automatic_write_preserves_shared_unresolved_evidence_and_does_not_repeat() {
    let (r, input) = setup(2, true).await;
    finish(
        &r,
        &input,
        vec![assessment(0, "approve"), assessment(1, "accumulating")],
    )
    .await;
    assert_eq!(
        r.admin.page_count(vec![]).await.unwrap(),
        0,
        "approval precedes durable writes"
    );
    assert!(
        r.hub
            .resume_automatic_write(r.admin.access(), &[])
            .await
            .unwrap()
    );
    assert_eq!(r.admin.page_count(vec![]).await.unwrap(), 1);
    let snapshot = r
        .admin
        .context_hub(ContextHubRequest::Inspect)
        .await
        .unwrap();
    assert!(
        snapshot["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["status"] == "pending")
    );
    let group = &snapshot["syntheses"][0];
    assert_eq!(group["automaticReview"]["state"], "written");
    let id = group["results"][0]["pageId"].as_str().unwrap();
    let pages = r
        .admin
        .read_pages(ReadPagesRequest {
            page_ids: vec![id.into()],
            revision_ids: vec![],
            projections: vec![Projection::Facets, Projection::Payload],
            max_chars: 32000,
        })
        .await
        .unwrap();
    assert_eq!(
        pages[0].revision.facets.as_ref().unwrap()["automaticCandidateReview"]["evidence"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let inventory = r.admin.durable_page_inventory(vec![]).await.unwrap();
    assert!(
        r.hub
            .prepare_automatic_review(r.admin.as_ref(), &inventory)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        r.hub
            .prepare_organization(r.admin.as_ref(), &inventory)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !r.hub
            .resume_automatic_write(r.admin.access(), &inventory)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn review_wait_and_pause_keep_exact_evidence_without_writing() {
    let (r, input) = setup(1, false).await;
    r.hub
        .finish_automatic_review(&input, vec![], vec![], "waiting_budget", "Budget exhausted")
        .await
        .unwrap();
    assert!(
        r.hub
            .prepare_automatic_review(r.admin.as_ref(), &[])
            .await
            .unwrap()
            .is_none()
    );
    {
        let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
            .await
            .unwrap();
        db.state.syntheses[0]
            .automatic_review
            .as_mut()
            .unwrap()
            .next_attempt_at = Some(timestamp(Utc::now() - Duration::seconds(1)));
        db.save().unwrap();
    }
    let resumed = r
        .hub
        .prepare_automatic_review(r.admin.as_ref(), &[])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(digest(&input).unwrap(), digest(&resumed).unwrap());
    r.admin
        .context_hub(ContextHubRequest::SetAutomaticReview { enabled: false })
        .await
        .unwrap();
    finish(&r, &input, vec![assessment(0, "approve")]).await;
    assert!(
        !r.hub
            .resume_automatic_write(r.admin.access(), &[])
            .await
            .unwrap()
    );
    assert_eq!(r.admin.page_count(vec![]).await.unwrap(), 0);
    r.admin
        .context_hub(ContextHubRequest::SetAutomaticReview { enabled: true })
        .await
        .unwrap();
    let reopened = ContextHub::new(r.store.clone(), r.hub.path.clone());
    assert!(
        reopened
            .resume_automatic_write(r.admin.access(), &[])
            .await
            .unwrap()
    );
    assert_eq!(r.admin.page_count(vec![]).await.unwrap(), 1);
}

#[tokio::test]
async fn unapproved_candidates_wait_for_new_evidence_and_tenants_cannot_enable_writes() {
    let (r, input) = setup(1, false).await;
    finish(&r, &input, vec![assessment(0, "needs_input")]).await;
    assert!(
        !r.hub
            .resume_automatic_write(r.admin.access(), &[])
            .await
            .unwrap()
    );
    assert!(
        r.hub
            .prepare_automatic_review(r.admin.as_ref(), &[])
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(r.admin.page_count(vec![]).await.unwrap(), 0);
    assert!(
        r.client("writer", &["a"])
            .context_hub(ContextHubRequest::SetAutomaticReview { enabled: true })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn automatic_approval_is_discarded_when_candidate_versions_change() {
    let (r, input) = setup(1, false).await;
    finish(&r, &input, vec![assessment(0, "approve")]).await;
    {
        let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
            .await
            .unwrap();
        db.state.candidates[0].version += 1;
        db.save().unwrap();
    }
    r.hub
        .resume_automatic_write(r.admin.access(), &[])
        .await
        .unwrap();
    assert_eq!(r.admin.page_count(vec![]).await.unwrap(), 0);
}

#[tokio::test]
async fn withdrawn_auto_create_is_archived_and_remains_readable_with_idempotent_receipt() {
    let (r, input) = setup(1, false).await;
    finish(&r, &input, vec![assessment(0, "approve")]).await;
    r.hub
        .resume_automatic_write(r.admin.access(), &[])
        .await
        .unwrap();
    let request = ContextHubRequest::UndoAutomaticOutput {
        synthesis_id: input.synthesis_id.clone(),
        version: input.version + 1,
        output_index: 0,
    };
    let result = r.admin.context_hub(request.clone()).await.unwrap();
    assert_eq!(result["status"], "archived");
    assert_eq!(r.admin.context_hub(request).await.unwrap(), result);
    let pages = r
        .admin
        .read_pages(ReadPagesRequest {
            page_ids: vec![result["pageId"].as_str().unwrap().into()],
            revision_ids: vec![],
            projections: vec![Projection::Payload],
            max_chars: 32000,
        })
        .await
        .unwrap();
    assert_eq!(
        pages[0].page.lifecycle_status,
        pcp_core::LifecycleStatus::Archived
    );
    assert!(pages[0].revision.payload.is_some());
}

async fn setup_update() -> (Rig, CandidateReviewInput, pcp_core::WriteResult) {
    let r = Rig::new().await;
    r.admin
        .context_hub(ContextHubRequest::SetAutomaticReview { enabled: true })
        .await
        .unwrap();
    let old = super::synthesis::existing_page(&r).await;
    let input = inputs(&r, 1).await;
    let mut proposal = proposed(&input);
    proposal.outputs[0].action = "update".into();
    proposal.outputs[0].target_revision_id = Some(old.revision_id.clone());
    proposal.outputs[0].content =
        "PCP 候选记忆：原始决定与独立限定。新增：由 Sol 进行有界审核。".into();
    r.hub
        .finish_organization(&input, vec![proposal])
        .await
        .unwrap();
    age(&r).await;
    let inventory = r.admin.durable_page_inventory(vec![]).await.unwrap();
    let prepared = r
        .hub
        .prepare_automatic_review(r.admin.as_ref(), &inventory)
        .await
        .unwrap()
        .unwrap();
    (r, prepared, old)
}

#[tokio::test]
async fn undo_automatic_update_restores_exact_content_and_metadata_without_deleting_history() {
    let (r, input, old) = setup_update().await;
    finish(&r, &input, vec![assessment(0, "approve")]).await;
    let inventory = r.admin.durable_page_inventory(vec![]).await.unwrap();
    assert!(
        r.hub
            .resume_automatic_write(r.admin.access(), &inventory)
            .await
            .unwrap()
    );
    let after = r
        .admin
        .read_pages(ReadPagesRequest {
            page_ids: vec![old.page_id.clone()],
            revision_ids: vec![],
            projections: vec![Projection::Payload, Projection::Provenance],
            max_chars: 32000,
        })
        .await
        .unwrap();
    assert!(
        after[0]
            .revision
            .payload
            .as_ref()
            .unwrap()
            .content
            .contains("Sol")
    );
    assert!(
        after[0]
            .revision
            .provenance
            .iter()
            .any(|p| p.actor.actor_type == pcp_core::ActorType::Model)
    );
    let request = ContextHubRequest::UndoAutomaticOutput {
        synthesis_id: input.synthesis_id.clone(),
        version: input.version + 1,
        output_index: 0,
    };
    let undone = r.admin.context_hub(request.clone()).await.unwrap();
    assert_eq!(undone["status"], "restored_previous");
    assert_eq!(r.admin.context_hub(request).await.unwrap(), undone);
    let restored = r
        .admin
        .read_pages(ReadPagesRequest {
            page_ids: vec![old.page_id.clone()],
            revision_ids: vec![],
            projections: vec![
                Projection::Payload,
                Projection::Sources,
                Projection::Facets,
                Projection::Provenance,
                Projection::History,
            ],
            max_chars: 32000,
        })
        .await
        .unwrap();
    assert_eq!(
        restored[0].revision.payload.as_ref().unwrap().content,
        "PCP 候选记忆：原始决定与独立限定。"
    );
    assert_eq!(
        restored[0].revision.facets.as_ref().unwrap()["keep"],
        "original facet"
    );
    assert_eq!(restored[0].revision.source_refs.len(), 1);
    assert_ne!(restored[0].revision.revision_id, old.revision_id);
    assert!(restored[0].history.len() >= 3);
}

#[tokio::test]
async fn changed_comparison_head_invalidates_automatic_approval_before_any_write() {
    let (r, input, old) = setup_update().await;
    finish(&r, &input, vec![assessment(0, "approve")]).await;
    r.admin.revise_page(serde_json::from_value(json!({"pageId":old.page_id,"expectedRevisionId":old.revision_id,"createdBy":{"actorType":"user","actorId":"operator:test"},"lifecycleStatus":"active","payload":{"mediaType":"text/plain","content":"A newer user correction"},"sourceRefs":[],"provenance":[],"initialRelations":[]})).unwrap()).await.unwrap();
    let inventory = r.admin.durable_page_inventory(vec![]).await.unwrap();
    r.hub
        .resume_automatic_write(r.admin.access(), &inventory)
        .await
        .unwrap();
    let snapshot = r
        .admin
        .context_hub(ContextHubRequest::Inspect)
        .await
        .unwrap();
    assert_eq!(
        snapshot["syntheses"][0]["automaticReview"]["state"],
        "stale"
    );
    assert_eq!(snapshot["syntheses"][0]["results"], json!([]));
}

#[tokio::test]
async fn candidate_arriving_during_review_prevents_applying_the_older_interpretation() {
    let (r, input) = setup(1, false).await;
    // Ensure the adversarial arrival is strictly after the captured millisecond watermark.
    {
        let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
            .await
            .unwrap();
        let mut newer = db.state.candidates[0].clone();
        newer.candidate_id = "newer".into();
        newer.created_at = timestamp(Utc::now() + Duration::seconds(1));
        db.state.candidates.push(newer);
        db.save().unwrap();
    }
    finish(&r, &input, vec![assessment(0, "approve")]).await;
    r.hub
        .resume_automatic_write(r.admin.access(), &[])
        .await
        .unwrap();
    assert_eq!(r.admin.page_count(vec![]).await.unwrap(), 0);
}

#[tokio::test]
async fn automatic_write_recovers_after_receipt_loss_without_duplicate_pages() {
    let (r, input) = setup(1, false).await;
    finish(&r, &input, vec![assessment(0, "approve")]).await;
    r.hub
        .resume_automatic_write(r.admin.access(), &[])
        .await
        .unwrap();
    {
        let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
            .await
            .unwrap();
        let group = &mut db.state.syntheses[0];
        group.status = "promoting".into();
        group.version = input.version;
        group.results.clear();
        group.automatic_review.as_mut().unwrap().state = "applying".into();
        db.state.candidates[0] = input.candidates[0].clone();
        db.state.candidates[0].status = "promoting".into();
        db.save().unwrap();
    }
    let reopened = ContextHub::new(r.store.clone(), r.hub.path.clone());
    let inventory = r.admin.durable_page_inventory(vec![]).await.unwrap();
    assert!(
        reopened
            .resume_automatic_write(r.admin.access(), &inventory)
            .await
            .unwrap()
    );
    assert_eq!(r.admin.page_count(vec![]).await.unwrap(), 1);
    let snapshot = r
        .admin
        .context_hub(ContextHubRequest::Inspect)
        .await
        .unwrap();
    assert_eq!(
        snapshot["syntheses"][0]["automaticReview"]["state"],
        "written"
    );
}

#[tokio::test]
async fn repaired_content_cannot_be_applied_without_independent_verification_receipt() {
    let (r, input) = setup(1, false).await;
    let mut decision = assessment(0, "approve");
    decision.revision = Some(crate::maintenance::VerifiedMaintenanceRevision {
        title: "Repaired".into(),
        content: "Repaired content".into(),
    });
    assert!(
        r.hub
            .finish_automatic_review(&input, vec![decision], steps(), "completed", "review")
            .await
            .is_err()
    );
    assert_eq!(r.admin.page_count(vec![]).await.unwrap(), 0);
}

struct AutomaticWorker;
#[async_trait]
impl crate::maintenance::SemanticMaintenanceWorker for AutomaticWorker {
    fn automatic_candidate_review_enabled(&self) -> bool {
        true
    }
    async fn evaluate(
        &self,
        request: crate::maintenance::MaintenanceWorkerRequest,
    ) -> Result<crate::maintenance::MaintenanceWorkerResponse> {
        let crate::maintenance::MaintenanceWorkerRequest::ReviewCandidateSynthesis { input } =
            request
        else {
            anyhow::bail!("unexpected operation")
        };
        Ok(
            crate::maintenance::MaintenanceWorkerResponse::CandidateSynthesisReview {
                decisions: input
                    .outputs
                    .iter()
                    .enumerate()
                    .map(|(i, _)| assessment(i, "approve"))
                    .collect(),
                steps: steps(),
                state: "completed".into(),
                reason: "Grounded memory".into(),
            },
        )
    }
}

#[tokio::test]
async fn scheduled_automatic_write_uses_existing_apply_gate_and_one_job_budget() {
    let (r, _) = setup(1, false).await;
    // Restore a fresh, settled queue before exercising the actual scheduler.
    {
        let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
            .await
            .unwrap();
        db.state.syntheses[0].automatic_review = None;
        db.save().unwrap();
    }
    let mut config = super::synthesis::config(&r);
    let mut preview = crate::maintenance::RuntimeMaintainer::for_test(
        r.admin.clone(),
        Arc::new(AutomaticWorker),
        config.clone(),
    )
    .with_context_hub(r.hub.clone());
    preview.run_scheduled_cycle().await.unwrap();
    assert_eq!(r.admin.page_count(vec![]).await.unwrap(), 0);
    config.mode = crate::maintenance::MaintenanceMode::Apply;
    let mut apply = crate::maintenance::RuntimeMaintainer::for_test(
        r.admin.clone(),
        Arc::new(AutomaticWorker),
        config,
    )
    .with_context_hub(r.hub.clone());
    let report = apply.run_scheduled_cycle().await.unwrap();
    assert_eq!(report.jobs_advanced, 1);
    assert_eq!(report.worker_calls, 1);
    assert_eq!(r.admin.page_count(vec![]).await.unwrap(), 1);
}

#[tokio::test]
async fn withdrawal_recovers_after_receipt_loss_and_refuses_later_edits() {
    for update in [false, true] {
        let (r, input) = if update {
            let (r, i, _) = setup_update().await;
            (r, i)
        } else {
            setup(1, false).await
        };
        finish(&r, &input, vec![assessment(0, "approve")]).await;
        let inventory = r.admin.durable_page_inventory(vec![]).await.unwrap();
        r.hub
            .resume_automatic_write(r.admin.access(), &inventory)
            .await
            .unwrap();
        let req = ContextHubRequest::UndoAutomaticOutput {
            synthesis_id: input.synthesis_id.clone(),
            version: input.version + 1,
            output_index: 0,
        };
        let expected = r.admin.context_hub(req.clone()).await.unwrap();
        {
            let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
                .await
                .unwrap();
            db.state.syntheses[0]
                .automatic_review
                .as_mut()
                .unwrap()
                .undo_results[0] = json!({"outputIndex":0,"status":"withdrawing","requestedAt":expected["requestedAt"]});
            db.save().unwrap();
        }
        assert_eq!(r.admin.context_hub(req).await.unwrap(), expected);
    }
    let (r, input, old) = setup_update().await;
    finish(&r, &input, vec![assessment(0, "approve")]).await;
    r.hub
        .resume_automatic_write(
            r.admin.access(),
            &r.admin.durable_page_inventory(vec![]).await.unwrap(),
        )
        .await
        .unwrap();
    let snapshot = r
        .admin
        .context_hub(ContextHubRequest::Inspect)
        .await
        .unwrap();
    let written = &snapshot["syntheses"][0]["results"][0]["revisionId"];
    r.admin.revise_page(serde_json::from_value(json!({"pageId":old.page_id,"expectedRevisionId":written,"createdBy":{"actorType":"user","actorId":"operator:test"},"lifecycleStatus":"active","payload":{"mediaType":"text/plain","content":"Later user edit must survive"},"sourceRefs":[],"provenance":[],"initialRelations":[]})).unwrap()).await.unwrap();
    assert!(
        r.admin
            .context_hub(ContextHubRequest::UndoAutomaticOutput {
                synthesis_id: input.synthesis_id,
                version: input.version + 1,
                output_index: 0
            })
            .await
            .is_err()
    );
    let pages = r
        .admin
        .read_pages(ReadPagesRequest {
            page_ids: vec![old.page_id],
            revision_ids: vec![],
            projections: vec![Projection::Payload],
            max_chars: 1000,
        })
        .await
        .unwrap();
    assert_eq!(
        pages[0].revision.payload.as_ref().unwrap().content,
        "Later user edit must survive"
    );
}

#[tokio::test]
async fn older_inboxes_keep_manual_review_until_operator_enables_automatic_writes() {
    let (r, input) = setup(1, false).await;
    {
        let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
            .await
            .unwrap();
        db.state.organization.automatic_review_enabled = None;
        db.state.syntheses[0].automatic_review = None;
        db.save().unwrap();
    }
    assert!(
        r.hub
            .prepare_automatic_review(r.admin.as_ref(), &[])
            .await
            .unwrap()
            .is_none()
    );
    r.admin
        .context_hub(ContextHubRequest::SetAutomaticReview { enabled: true })
        .await
        .unwrap();
    let fresh = r
        .hub
        .prepare_automatic_review(r.admin.as_ref(), &[])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fresh.synthesis_id, input.synthesis_id);
}

#[tokio::test]
async fn terminal_receipt_expiry_does_not_expire_unresolved_sources_or_formal_memories() {
    let (r, input) = setup(1, false).await;
    finish(&r, &input, vec![assessment(0, "approve")]).await;
    r.hub
        .resume_automatic_write(r.admin.access(), &[])
        .await
        .unwrap();
    let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
        .await
        .unwrap();
    assert!(db.prune(&timestamp(Utc::now() + Duration::days(31))));
    assert!(db.state.syntheses.is_empty());
    assert_eq!(db.state.candidates.len(), 1);
    assert_eq!(db.state.candidates[0].status, "pending");
    assert_eq!(r.admin.page_count(vec![]).await.unwrap(), 1);
}
