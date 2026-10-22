use super::*;
use crate::context_hub::automatic_review::{
    AutomaticReview, CandidateReviewInput, OutputAssessment,
};
use crate::context_hub::synthesis::Synthesis;
use crate::maintenance::{MaintenanceDetailPage, MaintenanceReviewStep};

fn comparison(scope: &str, content: &str) -> MaintenanceDetailPage {
    MaintenanceDetailPage {
        page_id: "pg_compare".into(),
        revision_id: "rev_compare".into(),
        namespace: scope.into(),
        created_at: timestamp(Utc::now()),
        observed_at: None,
        media_type: Some("text/markdown".into()),
        content: Some(content.into()),
        summary: None,
        facets: None,
        source_refs: vec![],
        relations: vec![],
    }
}

pub(super) async fn inspection_fixture(r: &Rig) -> (String, String) {
    r.enable("publisher").await;
    let receipt = r
        .client("publisher", &["a"])
        .context_hub(ContextHubRequest::SubmitCandidate(candidate(
            "inspection-evidence",
        )))
        .await
        .unwrap();
    let candidate_id = receipt["candidateId"].as_str().unwrap().to_string();
    let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
        .await
        .unwrap();
    let sentinel = "FULL_REVIEW_EVIDENCE_DO_NOT_DOWNLOAD_WITH_LISTS".repeat(300);
    let source = &mut db.state.candidates[0];
    source.promotion_request = Some(CandidateReview {
        title: Some("exact reviewed title".into()),
        content: Some(sentinel.clone()),
        ..promote(&receipt)
    });
    source.result = Some(
        json!({"status":"partially_represented","pageId":"pg_written",
        "outputs":[{"pageId":"pg_written","reviewInput":sentinel}]}),
    );
    let source = source.clone();
    let outputs = vec![SynthesisOutput {
        candidate_ids: vec![candidate_id.clone()],
        title: "proposed memory".into(),
        content: sentinel.clone(),
        action: "create".into(),
        target_revision_id: None,
    }];
    let page = comparison("a", &sentinel);
    let group_id = "syn_inspect".to_string();
    let review_request = SynthesisReview {
        synthesis_id: group_id.clone(),
        version: 1,
        outputs: outputs.clone(),
    };
    let group = Synthesis {
        synthesis_id: group_id.clone(),
        version: 1,
        scope: "a".into(),
        candidates: vec![CandidateVersion {
            candidate_id: candidate_id.clone(),
            version: source.version,
        }],
        title: "candidate summary".into(),
        narrative: sentinel.clone(),
        reason: "compare complete evidence".into(),
        unresolved: vec![],
        maturity: "ready".into(),
        outputs: outputs.clone(),
        offered_revision_ids: vec!["rev_compare".into()],
        updateable_revision_ids: vec![],
        compared_pages: vec![page.clone()],
        context_id: Some("ctx_inspect".into()),
        status: "pending".into(),
        created_at: timestamp(Utc::now()),
        review_request: Some(review_request),
        results: vec![
            json!({"action":"create","pageId":"pg_written","revisionId":"rev_written","reviewInput":sentinel}),
        ],
        execution_receipts: vec![],
        automatic_review: Some(AutomaticReview {
            state: "needs_review".into(),
            reason: "requires operator assessment".into(),
            input: Some(CandidateReviewInput {
                intake_watermark: timestamp(Utc::now()),
                synthesis_id: group_id.clone(),
                version: 1,
                scope: "a".into(),
                title: "immutable assessment".into(),
                narrative: sentinel.clone(),
                unresolved: vec![],
                candidates: vec![source],
                pages: vec![page.clone()],
                outputs,
            }),
            ..Default::default()
        }),
    };
    db.state.syntheses.push(group);
    db.state
        .synthesis_contexts
        .insert("ctx_inspect".into(), vec![page]);
    db.save().unwrap();
    (candidate_id, group_id)
}

#[tokio::test]
async fn operator_lists_are_light_and_details_retain_exact_review_and_retry_evidence() {
    let r = Rig::new().await;
    let (candidate_id, synthesis_id) = inspection_fixture(&r).await;
    r.client("publisher", &["a"])
        .context_hub(ContextHubRequest::PublishActivity(activity(
            "work",
            "awaiting review",
        )))
        .await
        .unwrap();
    let activity = r
        .admin
        .context_hub(ContextHubRequest::InspectActivity)
        .await
        .unwrap();
    assert_eq!(activity["activity"].as_array().unwrap().len(), 1);
    assert_eq!(activity["limits"]["topicsPerClientScope"], 12);
    assert!(activity.get("candidates").is_none());
    assert!(activity.get("syntheses").is_none());
    assert!(serde_json::to_vec(&activity).unwrap().len() < 2048);
    let policies = r
        .admin
        .context_hub(ContextHubRequest::InspectPolicies)
        .await
        .unwrap();
    assert_eq!(policies.as_object().unwrap().len(), 1);
    assert!(policies.get("policies").is_some());
    let inbox = r
        .admin
        .context_hub(ContextHubRequest::InspectInbox)
        .await
        .unwrap();
    let encoded = serde_json::to_string(&inbox).unwrap();
    assert!(!encoded.contains("FULL_REVIEW_EVIDENCE"));
    let candidate = &inbox["candidates"][0];
    assert_eq!(candidate["summaryOnly"], true);
    assert_eq!(
        candidate["input"]["content"],
        "用户正在考虑候选暂存，尚未定案。"
    );
    assert_eq!(candidate["result"]["pageId"], "pg_written");
    assert!(candidate.get("promotionRequest").is_none());
    let summary = &inbox["syntheses"][0];
    assert_eq!(summary["summaryOnly"], true);
    assert_eq!(summary["automaticReview"]["state"], "needs_review");
    assert!(summary["automaticReview"].get("input").is_none());
    assert!(summary["outputs"][0].get("content").is_none());
    assert!(
        summary["reviewRequest"]["outputs"][0]
            .get("content")
            .is_none()
    );
    assert!(summary.get("comparedPages").is_none());
    assert!(inbox.get("synthesisContexts").is_none());
    let details = r
        .admin
        .context_hub(ContextHubRequest::InspectSynthesis {
            synthesis_id: synthesis_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(details["synthesis"]["synthesisId"], synthesis_id);
    assert_eq!(details["detailToken"], summary["detailToken"]);
    assert_eq!(
        details["synthesisContext"][0]["content"],
        details["synthesis"]["comparedPages"][0]["content"]
    );
    assert!(
        details["synthesis"]["automaticReview"]["input"]["pages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("FULL_REVIEW_EVIDENCE")
    );
    assert_eq!(details["candidates"][0]["candidateId"], candidate_id);
    let standalone = r
        .admin
        .context_hub(ContextHubRequest::InspectCandidate {
            candidate_id: candidate_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(standalone["candidate"], details["candidates"][0]);
    assert_eq!(standalone["detailToken"], candidate["detailToken"]);
    assert_eq!(
        standalone["detailToken"],
        details["candidateDetailTokens"][&candidate_id]
    );
    assert!(
        standalone["candidate"]["promotionRequest"]["content"]
            .as_str()
            .unwrap()
            .contains("FULL_REVIEW_EVIDENCE")
    );
    let legacy = r
        .admin
        .context_hub(ContextHubRequest::Inspect)
        .await
        .unwrap();
    assert_eq!(legacy["syntheses"][0], details["synthesis"]);
    assert!(legacy.get("synthesisContexts").is_some());
    assert!(serde_json::to_vec(&legacy).unwrap().len() > encoded.len() * 10);
    assert!(
        r.admin
            .context_hub(ContextHubRequest::InspectCandidate {
                candidate_id: "missing".into()
            })
            .await
            .is_err()
    );
    assert!(
        r.admin
            .context_hub(ContextHubRequest::InspectSynthesis {
                synthesis_id: "missing".into()
            })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn new_inspectors_require_operator_and_full_detail_checks_embedded_scope_permissions() {
    let r = Rig::new().await;
    let (candidate_id, synthesis_id) = inspection_fixture(&r).await;
    let client = r.client("publisher", &["a"]);
    for request in [
        ContextHubRequest::InspectActivity,
        ContextHubRequest::InspectPolicies,
        ContextHubRequest::InspectInbox,
        ContextHubRequest::InspectCandidate {
            candidate_id: candidate_id.clone(),
        },
        ContextHubRequest::InspectSynthesis {
            synthesis_id: synthesis_id.clone(),
        },
    ] {
        assert!(client.context_hub(request).await.is_err());
    }
    // Manage/Write alone grants operator actions but does not grant Scope reads.
    let limited = access("limited-operator", &["a"], false)
        .with_store_permissions([AccessPermission::ManageScope, AccessPermission::Write]);
    r.client("publisher", &["b"])
        .context_hub(ContextHubRequest::PublishActivity(ActivityInput {
            scope: "b".into(),
            ..activity("private", "hidden scope activity")
        }))
        .await
        .unwrap();
    let mut private_candidate = candidate("private-evidence");
    private_candidate.scope = "b".into();
    let private = r
        .client("publisher", &["b"])
        .context_hub(ContextHubRequest::SubmitCandidate(private_candidate))
        .await
        .unwrap();
    let activity = r
        .hub
        .execute(&limited, ContextHubRequest::InspectActivity)
        .await
        .unwrap();
    assert!(activity["activity"].as_array().unwrap().is_empty());
    let inbox = r
        .hub
        .execute(&limited, ContextHubRequest::InspectInbox)
        .await
        .unwrap();
    assert_eq!(inbox["candidates"].as_array().unwrap().len(), 1);
    assert_eq!(inbox["candidates"][0]["candidateId"], candidate_id);
    assert!(
        r.hub
            .execute(
                &limited,
                ContextHubRequest::InspectCandidate {
                    candidate_id: private["candidateId"].as_str().unwrap().into()
                }
            )
            .await
            .is_err()
    );
    assert!(
        r.hub
            .execute(
                &limited,
                ContextHubRequest::InspectSynthesis {
                    synthesis_id: synthesis_id.clone()
                }
            )
            .await
            .is_ok()
    );
    {
        let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
            .await
            .unwrap();
        db.state.synthesis_contexts.get_mut("ctx_inspect").unwrap()[0].namespace = "b".into();
        db.save().unwrap();
    }
    assert!(
        r.hub
            .execute(
                &limited,
                ContextHubRequest::InspectSynthesis {
                    synthesis_id: synthesis_id.clone()
                }
            )
            .await
            .is_err()
    );
    let limited_inbox = r
        .hub
        .execute(&limited, ContextHubRequest::InspectInbox)
        .await
        .unwrap();
    assert!(
        limited_inbox["syntheses"][0]["detailToken"].is_null(),
        "a readable summary must not hash unreadable evidence"
    );
    let readable_token = limited_inbox["candidates"][0]["detailToken"].clone();
    {
        let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
            .await
            .unwrap();
        db.state.synthesis_contexts.get_mut("ctx_inspect").unwrap()[0].content =
            Some("private changes must not influence a readable token".into());
        db.save().unwrap();
    }
    let changed = r
        .hub
        .execute(&limited, ContextHubRequest::InspectInbox)
        .await
        .unwrap();
    assert_eq!(changed["syntheses"][0]["detailToken"], Value::Null);
    assert_eq!(changed["candidates"][0]["detailToken"], readable_token);
}

fn completed_step(request_id: &str) -> MaintenanceReviewStep {
    MaintenanceReviewStep {
        tier: "sol".into(),
        stage: "candidate_review".into(),
        state: "completed".into(),
        reason: "Independent review".into(),
        request_id: Some(request_id.into()),
        actual_tokens: Some(100),
    }
}

async fn running_review(outputs: usize) -> (Rig, CandidateReviewInput) {
    let r = Rig::new().await;
    r.admin
        .context_hub(ContextHubRequest::SetAutomaticReview { enabled: true })
        .await
        .unwrap();
    let input = super::synthesis::inputs(&r, 1).await;
    let mut proposal = super::synthesis::proposed(&input);
    for index in 1..outputs {
        let mut output = proposal.outputs[0].clone();
        output.title = format!("Independent output {index}");
        proposal.outputs.push(output);
    }
    r.hub
        .finish_organization(&input, vec![proposal])
        .await
        .unwrap();
    {
        let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
            .await
            .unwrap();
        let before = timestamp(Utc::now() - Duration::minutes(6));
        for group in &mut db.state.syntheses {
            group.created_at = before.clone();
        }
        for candidate in &mut db.state.candidates {
            candidate.created_at = before.clone();
        }
        db.save().unwrap();
    }
    let review = r
        .hub
        .prepare_automatic_review(r.admin.as_ref(), &[])
        .await
        .unwrap()
        .unwrap();
    (r, review)
}

async fn inbox(r: &Rig) -> Value {
    r.admin
        .context_hub(ContextHubRequest::InspectInbox)
        .await
        .unwrap()
}

async fn approve_all(r: &Rig, input: &CandidateReviewInput) {
    let decisions = input
        .outputs
        .iter()
        .enumerate()
        .map(|(index, _)| OutputAssessment {
            output_index: index,
            verdict: "approve".into(),
            reason: "Source coverage verified".into(),
            revision: None,
        })
        .collect();
    r.hub
        .finish_automatic_review(
            input,
            decisions,
            vec![completed_step("approved-review")],
            "completed",
            "Source-based review",
        )
        .await
        .unwrap();
    r.hub
        .resume_automatic_write(r.admin.access(), &[])
        .await
        .unwrap();
}

#[tokio::test]
async fn detail_tokens_follow_same_version_review_and_cross_window_withdrawal() {
    let (r, input) = running_review(1).await;
    let running = inbox(&r).await;
    assert_eq!(
        running["syntheses"][0]["automaticReview"]["state"],
        "running"
    );
    let stable = inbox(&r).await;
    assert_eq!(
        running["syntheses"][0]["detailToken"],
        stable["syntheses"][0]["detailToken"]
    );
    r.hub
        .finish_automatic_review(
            &input,
            vec![],
            vec![completed_step("budget-wait")],
            "waiting_budget",
            "Shared budget is not available",
        )
        .await
        .unwrap();
    let waiting = inbox(&r).await;
    assert_eq!(
        running["syntheses"][0]["version"],
        waiting["syntheses"][0]["version"]
    );
    assert_ne!(
        running["syntheses"][0]["detailToken"],
        waiting["syntheses"][0]["detailToken"]
    );
    approve_all(&r, &input).await;
    let written = inbox(&r).await;
    let token = written["syntheses"][0]["detailToken"].clone();
    let other_window =
        EmbeddedPcpClient::new(r.store.clone(), access("other-operator-window", &[], true))
            .with_context_hub(r.hub.clone());
    other_window
        .context_hub(ContextHubRequest::UndoAutomaticOutput {
            synthesis_id: input.synthesis_id.clone(),
            version: written["syntheses"][0]["version"].as_u64().unwrap(),
            output_index: 0,
        })
        .await
        .unwrap();
    let withdrawn = inbox(&r).await;
    assert_eq!(
        written["syntheses"][0]["version"],
        withdrawn["syntheses"][0]["version"]
    );
    assert_ne!(token, withdrawn["syntheses"][0]["detailToken"]);
    let mut before = written["syntheses"][0].clone();
    before.as_object_mut().unwrap().remove("detailToken");
    let mut after = withdrawn["syntheses"][0].clone();
    after.as_object_mut().unwrap().remove("detailToken");
    assert_eq!(
        before, after,
        "withdrawal changes only detail evidence, not the existing lightweight summary fields"
    );
    let detail = r
        .admin
        .context_hub(ContextHubRequest::InspectSynthesis {
            synthesis_id: input.synthesis_id,
        })
        .await
        .unwrap();
    assert_eq!(
        detail["detailToken"],
        withdrawn["syntheses"][0]["detailToken"]
    );
    assert_eq!(
        detail["synthesis"]["automaticReview"]["undoResults"][0]["status"],
        "archived"
    );
}

#[tokio::test]
async fn detail_tokens_cover_inflight_receipts_without_replaying_or_rewriting_them() {
    let (r, input) = running_review(2).await;
    approve_all(&r, &input).await;
    let complete = r
        .admin
        .context_hub(ContextHubRequest::InspectSynthesis {
            synthesis_id: input.synthesis_id.clone(),
        })
        .await
        .unwrap();
    let results = complete["synthesis"]["results"].as_array().unwrap().clone();
    let request: SynthesisReview =
        serde_json::from_value(complete["synthesis"]["reviewRequest"].clone()).unwrap();
    {
        // Recoverable checkpoint after plan persistence, before the first receipt.
        let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
            .await
            .unwrap();
        let group = &mut db.state.syntheses[0];
        group.status = "promoting".into();
        group.version = input.version;
        group.results.clear();
        for source in &mut db.state.candidates {
            source.status = "promoting".into();
            source.version = input.version;
        }
        db.save().unwrap();
    }
    let before = inbox(&r).await;
    {
        let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
            .await
            .unwrap();
        db.state.syntheses[0].results.push(results[0].clone());
        db.save().unwrap();
    }
    let partial = inbox(&r).await;
    assert_eq!(
        before["syntheses"][0]["version"],
        partial["syntheses"][0]["version"]
    );
    assert_eq!(
        before["syntheses"][0]["status"],
        partial["syntheses"][0]["status"]
    );
    assert_ne!(
        before["syntheses"][0]["detailToken"],
        partial["syntheses"][0]["detailToken"]
    );
    assert_eq!(
        before["candidates"][0]["detailToken"],
        partial["candidates"][0]["detailToken"]
    );
    let detail = r
        .admin
        .context_hub(ContextHubRequest::InspectSynthesis {
            synthesis_id: input.synthesis_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(
        detail["detailToken"],
        partial["syntheses"][0]["detailToken"]
    );
    assert_eq!(detail["synthesis"]["results"].as_array().unwrap().len(), 1);
    let recovered = r
        .admin
        .context_hub(ContextHubRequest::ReviewSynthesis(request))
        .await
        .unwrap();
    assert_eq!(recovered["outputs"], complete["synthesis"]["results"]);
    assert_eq!(
        r.admin.page_count(vec![]).await.unwrap(),
        2,
        "inspection never replays a write; exact retry recovers both original outputs"
    );
}

#[tokio::test]
async fn detail_tokens_cover_current_candidate_metadata_and_shared_comparison_context() {
    let r = Rig::new().await;
    let (candidate_id, synthesis_id) = inspection_fixture(&r).await;
    r.hub.enable_organization();
    {
        let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
            .await
            .unwrap();
        db.state.candidates[0].organized_version = 1;
        db.state.candidates[0].snoozed_until = Some(timestamp(Utc::now() + Duration::days(1)));
        db.save().unwrap();
    }
    let organized = inbox(&r).await;
    r.admin
        .context_hub(ContextHubRequest::OrganizeCandidates)
        .await
        .unwrap();
    let queued = inbox(&r).await;
    assert_eq!(
        organized["candidates"][0]["version"],
        queued["candidates"][0]["version"]
    );
    assert_ne!(
        organized["candidates"][0]["detailToken"],
        queued["candidates"][0]["detailToken"]
    );
    assert_ne!(
        organized["syntheses"][0]["detailToken"],
        queued["syntheses"][0]["detailToken"]
    );
    {
        let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
            .await
            .unwrap();
        db.state.synthesis_contexts.get_mut("ctx_inspect").unwrap()[0].content =
            Some("New shared comparison content".into());
        db.save().unwrap();
    }
    let changed = inbox(&r).await;
    assert_eq!(
        queued["candidates"][0]["detailToken"],
        changed["candidates"][0]["detailToken"]
    );
    assert_ne!(
        queued["syntheses"][0]["detailToken"],
        changed["syntheses"][0]["detailToken"]
    );
    let candidate = r
        .admin
        .context_hub(ContextHubRequest::InspectCandidate { candidate_id })
        .await
        .unwrap();
    let group = r
        .admin
        .context_hub(ContextHubRequest::InspectSynthesis { synthesis_id })
        .await
        .unwrap();
    assert_eq!(
        candidate["detailToken"],
        changed["candidates"][0]["detailToken"]
    );
    assert_eq!(group["detailToken"], changed["syntheses"][0]["detailToken"]);
}
