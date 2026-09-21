use super::synthesis::proposed;
use super::*;
use pcp_client::experience::{Experience, outbox::ExperienceOutbox, receipts};

fn evidence(observation: &str) -> pcp_client::experience::ExperienceCandidate {
    Experience {
        topic_key: Some("maintenance-timeouts".into()),
        conditions: Some("Local Runtime with a 120-second upstream deadline".into()),
        attempt: "Allow response grace after the provider deadline".into(),
        observation: observation.into(),
        interpretation: Some("Equal deadlines may race; longer-term effect is uncertain".into()),
        unresolved: vec!["Does the timeout rate fall in later tasks?".into()],
        receipts: vec![receipts::infer_response(br#"{"id":"resp-1","status":"completed","model":"fixture","output":"must not copy"}"#).unwrap()],
    }.into_candidate("a".into(), "Maintenance experience".into()).unwrap()
}

#[tokio::test]
async fn experience_outbox_survives_denial_restart_and_lost_ack_without_duplicate_evidence() {
    let r = Rig::new().await;
    let principal = "producer";
    let root = r.root.join("outbox");
    let outbox =
        ExperienceOutbox::open(root.clone(), r.store.identity_id().into(), principal.into())
            .unwrap();
    let input = evidence("The execution finished, task behavior is not yet evaluated");
    let key = outbox.stage(input.clone()).unwrap();
    assert_eq!(outbox.stage(input.clone()).unwrap(), key);
    let client = r.client(principal, &["a"]);
    assert!(
        outbox.flush_one(client.as_ref()).await.is_err(),
        "disabled policy must stop delivery"
    );
    assert_eq!(
        outbox.stage(input.clone()).unwrap(),
        key,
        "denial keeps exact evidence"
    );
    r.enable(principal).await;
    // The server committed, but the producer did not receive its acknowledgement.
    client
        .context_hub(ContextHubRequest::SubmitExperience(input))
        .await
        .unwrap();
    drop(outbox);
    let reopened =
        ExperienceOutbox::open(root, r.store.identity_id().into(), principal.into()).unwrap();
    let receipt = reopened.flush_one(client.as_ref()).await.unwrap().unwrap();
    assert_eq!(receipt["created"], false);
    assert!(reopened.flush_one(client.as_ref()).await.unwrap().is_none());
    let db = LockedState::open(&r.hub.path, r.store.identity_id())
        .await
        .unwrap();
    assert_eq!(db.state.candidates.len(), 1);
}

#[tokio::test]
async fn experience_preserves_source_and_interpretation_after_manual_review_and_cleanup() {
    let r = Rig::new().await;
    r.enable("writer").await;
    let input = evidence("The test passed; future production behavior remains unmeasured");
    let receipt = r
        .client("writer", &["a"])
        .context_hub(ContextHubRequest::SubmitExperience(input.clone()))
        .await
        .unwrap();
    let result = r
        .admin
        .context_hub(ContextHubRequest::Review(promote(&receipt)))
        .await
        .unwrap();
    let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
        .await
        .unwrap();
    db.state.candidates[0].expires_at = "2000-01-01T00:00:00Z".into();
    db.prune("2100-01-01T00:00:00Z");
    assert!(db.state.candidates.is_empty());
    db.save().unwrap();
    drop(db);
    let pages = r
        .admin
        .read_pages(ReadPagesRequest {
            page_ids: vec![result["pageId"].as_str().unwrap().into()],
            revision_ids: vec![],
            projections: vec![Projection::Facets, Projection::Sources],
            max_chars: 16000,
        })
        .await
        .unwrap();
    let facets = pages[0].revision.facets.as_ref().unwrap();
    assert_eq!(
        facets["experienceEvidence"]["items"][0]["experience"]["observation"],
        input.experience.observation
    );
    assert_eq!(
        facets["experienceEvidence"]["attribution"],
        "producer_report"
    );
    assert_eq!(pages[0].revision.source_refs[0].locator, "response:resp-1");
    assert!(
        !serde_json::to_string(facets)
            .unwrap()
            .contains("must not copy")
    );
}

#[tokio::test]
async fn new_experience_reopens_interpretation_but_unchanged_input_does_not_spin() {
    let r = Rig::new().await;
    r.enable("writer").await;
    r.hub.enable_organization();
    let client = r.client("writer", &["a"]);
    client
        .context_hub(ContextHubRequest::SubmitExperience(evidence(
            "First result",
        )))
        .await
        .unwrap();
    r.admin
        .context_hub(ContextHubRequest::OrganizeCandidates)
        .await
        .unwrap();
    let first = r
        .hub
        .prepare_organization(r.admin.as_ref(), &[])
        .await
        .unwrap()
        .unwrap();
    let execution = receipts::infer_execution("organizer-1", "m", "completed").unwrap();
    r.hub
        .finish_organization_with_receipts(&first, vec![proposed(&first)], vec![execution])
        .await
        .unwrap();
    assert!(
        r.hub
            .prepare_organization(r.admin.as_ref(), &[])
            .await
            .unwrap()
            .is_none()
    );
    client
        .context_hub(ContextHubRequest::SubmitExperience(evidence(
            "A later failure contradicts the first explanation",
        )))
        .await
        .unwrap();
    r.admin
        .context_hub(ContextHubRequest::OrganizeCandidates)
        .await
        .unwrap();
    let next = r
        .hub
        .prepare_organization(r.admin.as_ref(), &[])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next.candidates.len(), 2);
    assert!(!next.previous.is_empty());
    assert!(
        next.candidates
            .iter()
            .any(|c| c.input.content.contains("later failure"))
    );
    let db = LockedState::open(&r.hub.path, r.store.identity_id())
        .await
        .unwrap();
    assert_eq!(
        db.state.syntheses[0].execution_receipts[0].source.locator,
        "response:organizer-1"
    );
}

#[tokio::test]
async fn topic_keys_and_outbox_cannot_cross_principal_or_scope_boundaries() {
    let r = Rig::new().await;
    r.enable("writer").await;
    let input = evidence("Observed");
    r.client("writer", &["a"])
        .context_hub(ContextHubRequest::SubmitExperience(input.clone()))
        .await
        .unwrap();
    let db = LockedState::open(&r.hub.path, r.store.identity_id())
        .await
        .unwrap();
    let a = db.state.candidates[0].clone();
    drop(db);
    let mut b = a.clone();
    b.client_id = "other".into();
    assert!(!super::super::experience::same_topic(&a, &b));
    b.client_id = a.client_id.clone();
    b.input.scope = "b".into();
    assert!(!super::super::experience::same_topic(&a, &b));
    let outbox = ExperienceOutbox::open(
        r.root.join("outbox"),
        r.store.identity_id().into(),
        "writer".into(),
    )
    .unwrap();
    outbox.stage(input).unwrap();
    assert!(
        outbox
            .flush_one(r.client("other", &["a"]).as_ref())
            .await
            .is_err()
    );
    assert!(
        outbox
            .flush_one(r.client("writer", &["b"]).as_ref())
            .await
            .is_err()
    );
    assert!(
        outbox
            .flush_one(r.client("writer", &["a"]).as_ref())
            .await
            .unwrap()
            .is_some()
    );
}

#[test]
fn old_candidate_wire_stays_compatible_and_experience_is_bounded() {
    let old: CandidateInput =
        serde_json::from_value(json!({"scope":"a","eventId":"old","title":"old","content":"old"}))
            .unwrap();
    assert!(
        serde_json::to_value(&old)
            .unwrap()
            .get("experience")
            .is_none()
    );
    let mut input = evidence("observed");
    input.experience.observation = "x".repeat(1001);
    assert!(input.experience.validate().is_err());
}

#[tokio::test]
async fn native_experience_round_trips_over_rpc_with_attested_identity() {
    let r = Rig::new().await;
    r.enable("producer").await;
    let socket = std::env::temp_dir().join(format!(
        "ex{}.sock",
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    ));
    let endpoint = pcp_rpc::RunningRuntimeEndpoint::start(&socket, r.client("producer", &["a"]))
        .await
        .unwrap();
    let remote = pcp_rpc::RemotePcpClient::connect_expected(&socket, "producer")
        .await
        .unwrap();
    assert!(
        remote
            .capabilities()
            .features
            .iter()
            .any(|f| f == pcp_client::RUNTIME_EXPERIENCE_FEATURE)
    );
    let outbox = ExperienceOutbox::open(
        r.root.join("outbox"),
        r.store.identity_id().into(),
        "producer".into(),
    )
    .unwrap();
    outbox
        .stage(evidence("RPC transported the original evidence"))
        .unwrap();
    outbox.flush_one(&remote).await.unwrap();
    let db = LockedState::open(&r.hub.path, r.store.identity_id())
        .await
        .unwrap();
    assert_eq!(db.state.candidates[0].client_id, "producer");
    assert_eq!(
        db.state.candidates[0]
            .experience
            .as_ref()
            .unwrap()
            .receipts
            .len(),
        1
    );
    drop(db);
    endpoint.shutdown().await;
}

#[tokio::test]
async fn synthesis_keeps_execution_receipts_separate_from_original_experience() {
    let r = Rig::new().await;
    r.enable("writer").await;
    r.hub.enable_organization();
    r.client("writer", &["a"])
        .context_hub(ContextHubRequest::SubmitExperience(evidence(
            "Original observation",
        )))
        .await
        .unwrap();
    r.admin
        .context_hub(ContextHubRequest::OrganizeCandidates)
        .await
        .unwrap();
    let input = r
        .hub
        .prepare_organization(r.admin.as_ref(), &[])
        .await
        .unwrap()
        .unwrap();
    let proposal = proposed(&input);
    r.hub
        .finish_organization_with_receipts(
            &input,
            vec![proposal.clone()],
            vec![receipts::infer_execution("organization-response", "model", "completed").unwrap()],
        )
        .await
        .unwrap();
    let db = LockedState::open(&r.hub.path, r.store.identity_id())
        .await
        .unwrap();
    let group = db.state.syntheses[0].clone();
    drop(db);
    let result = r
        .admin
        .context_hub(ContextHubRequest::ReviewSynthesis(
            pcp_client::context_hub::SynthesisReview {
                synthesis_id: group.synthesis_id,
                version: group.version,
                outputs: proposal.outputs,
            },
        ))
        .await
        .unwrap();
    let pages = r
        .admin
        .read_pages(ReadPagesRequest {
            page_ids: vec![result["outputs"][0]["pageId"].as_str().unwrap().into()],
            revision_ids: vec![],
            projections: vec![Projection::Facets],
            max_chars: 16000,
        })
        .await
        .unwrap();
    let facets = pages[0].revision.facets.as_ref().unwrap();
    assert_eq!(
        facets["experienceEvidence"]["items"][0]["experience"]["receipts"][0]["source"]["locator"],
        "response:resp-1"
    );
    assert_eq!(
        facets["organizationExecutions"][0]["source"]["locator"],
        "response:organization-response"
    );
}

#[tokio::test]
async fn expired_outbox_records_require_reconciliation_instead_of_silent_replay() {
    let r = Rig::new().await;
    r.enable("writer").await;
    let root = r.root.join("outbox");
    let outbox =
        ExperienceOutbox::open(root.clone(), r.store.identity_id().into(), "writer".into())
            .unwrap();
    outbox.stage(evidence("Old unknown result")).unwrap();
    let path = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|x| x == "json"))
        .unwrap();
    let mut record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record["stagedAt"] = json!(1);
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let error = outbox
        .flush_one(r.client("writer", &["a"]).as_ref())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("safe retry window"));
    assert!(path.exists());
    let db = LockedState::open(&r.hub.path, r.store.identity_id())
        .await
        .unwrap();
    assert!(db.state.candidates.is_empty());
}
