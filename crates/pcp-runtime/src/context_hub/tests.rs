use super::*;
use pcp_client::{AccessMode, PcpApi};
use pcp_core::{AccessPrincipal, AccessPrincipalType, CreateScopeRequest};
use pcp_sqlite::SqlitePcpStore;

struct Rig {
    root: PathBuf,
    hub: Arc<ContextHub>,
    admin: Arc<dyn PcpApi>,
    store: Arc<dyn PcpStore>,
}
impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn access(id: &str, scopes: &[&str], admin: bool) -> AccessSession {
    let principal = AccessPrincipal {
        principal_id: id.into(),
        principal_type: AccessPrincipalType::Service,
        display_name: None,
    };
    if admin {
        AccessMode::Admin.store_wide_session(principal, "test", vec![], true)
    } else {
        AccessMode::Contribute.session(
            principal,
            "test",
            scopes.iter().map(|s| s.to_string()).collect(),
            false,
        )
    }
}
impl Rig {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("pcp-context-hub-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let store: Arc<dyn PcpStore> = Arc::new(
            SqlitePcpStore::open(root.join("store.sqlite3"))
                .await
                .unwrap(),
        );
        let hub = Arc::new(ContextHub::new(store.clone(), root.join("context.json")));
        let admin: Arc<dyn PcpApi> = Arc::new(
            EmbeddedPcpClient::new(store.clone(), access("operator:test", &[], true))
                .with_context_hub(hub.clone()),
        );
        for scope in ["a", "b"] {
            admin
                .create_scope(CreateScopeRequest {
                    namespace: scope.into(),
                    display_name: scope.into(),
                    description: None,
                    parent_namespace: None,
                })
                .await
                .unwrap();
        }
        Self {
            root,
            hub,
            admin,
            store,
        }
    }
    fn client(&self, id: &str, scopes: &[&str]) -> Arc<dyn PcpApi> {
        Arc::new(
            EmbeddedPcpClient::new(self.store.clone(), access(id, scopes, false))
                .with_context_hub(self.hub.clone()),
        )
    }
    async fn enable(&self, id: &str) {
        self.admin
            .context_hub(ContextHubRequest::SetPolicy(ClientContextPolicy {
                client_id: id.into(),
                submit_candidates: true,
                publish_activity: true,
                read_activity: true,
            }))
            .await
            .unwrap();
    }
}
fn candidate(event: &str) -> CandidateInput {
    CandidateInput {
        scope: "a".into(),
        event_id: event.into(),
        title: "PCP 候选记忆".into(),
        content: "用户正在考虑候选暂存，尚未定案。".into(),
        source_refs: vec![],
        based_on_revision_ids: vec![],
    }
}
fn activity(topic: &str, summary: &str) -> ActivityInput {
    ActivityInput {
        scope: "a".into(),
        topic_key: topic.into(),
        summary: summary.into(),
        expected_version: None,
        ttl_hours: None,
    }
}
fn promote(receipt: &Value) -> CandidateReview {
    CandidateReview {
        candidates: vec![CandidateVersion {
            candidate_id: receipt["candidateId"].as_str().unwrap().into(),
            version: 1,
        }],
        action: CandidateAction::Promote,
        title: Some("PCP 暂存设计".into()),
        content: Some("候选暂存与正式记忆分开。".into()),
        target_revision_id: None,
    }
}

#[tokio::test]
async fn candidate_is_opt_in_isolated_idempotent_and_operator_reviewed() {
    let r = Rig::new().await;
    let a = r.client("client:a", &["a"]);
    assert!(
        a.context_hub(ContextHubRequest::SubmitCandidate(candidate("e1")))
            .await
            .unwrap_err()
            .to_string()
            .contains("disabled")
    );
    r.enable("client:a").await;
    let first = a
        .context_hub(ContextHubRequest::SubmitCandidate(candidate("e1")))
        .await
        .unwrap();
    assert_eq!(a.page_count(vec!["a".into()]).await.unwrap(), 0);
    let second = a
        .context_hub(ContextHubRequest::SubmitCandidate(candidate("e1")))
        .await
        .unwrap();
    assert_eq!(first["candidateId"], second["candidateId"]);
    assert_eq!(second["created"], false);
    let mut changed = candidate("e1");
    changed.content = "different".into();
    assert!(
        a.context_hub(ContextHubRequest::SubmitCandidate(changed))
            .await
            .is_err()
    );
    assert!(a.context_hub(ContextHubRequest::Inspect).await.is_err());
    assert!(
        a.context_hub(ContextHubRequest::Review(promote(&first)))
            .await
            .is_err()
    );
    let request = promote(&first);
    let written = r
        .admin
        .context_hub(ContextHubRequest::Review(request.clone()))
        .await
        .unwrap();
    let replay = r
        .admin
        .context_hub(ContextHubRequest::Review(request))
        .await
        .unwrap();
    assert_eq!(written, replay);
    assert_eq!(a.page_count(vec!["a".into()]).await.unwrap(), 1);
    let page = a
        .read_pages(ReadPagesRequest {
            page_ids: vec![],
            revision_ids: vec![written["revisionId"].as_str().unwrap().into()],
            projections: vec![Projection::Payload],
            max_chars: 1000,
        })
        .await
        .unwrap();
    assert_eq!(
        page[0].revision.payload.as_ref().unwrap().content,
        "# PCP 暂存设计\n\n候选暂存与正式记忆分开。"
    );
}

#[tokio::test]
async fn activity_has_bounded_slots_versions_expiry_and_query_bound_snapshot_tokens() {
    let r = Rig::new().await;
    r.enable("publisher").await;
    r.enable("reader").await;
    let a = r.client("publisher", &["a"]);
    let b = r.client("reader", &["a"]);
    let first = a
        .context_hub(ContextHubRequest::PublishActivity(activity(
            "one",
            "测试第一条近况",
        )))
        .await
        .unwrap();
    let unchanged = a
        .context_hub(ContextHubRequest::PublishActivity(activity(
            "one",
            "测试第一条近况",
        )))
        .await
        .unwrap();
    assert_eq!(unchanged["changed"], false);
    assert_eq!(first["expiresAt"], unchanged["expiresAt"]);
    assert!(
        a.context_hub(ContextHubRequest::PublishActivity(activity(
            "one", "changed"
        )))
        .await
        .is_err()
    );
    let mut update = activity("one", "changed");
    update.expected_version = first["version"].as_u64();
    a.context_hub(ContextHubRequest::PublishActivity(update))
        .await
        .unwrap();
    for key in ["two", "three", "four"] {
        a.context_hub(ContextHubRequest::PublishActivity(activity(
            key,
            &"中".repeat(180),
        )))
        .await
        .unwrap();
    }
    let q = ActivityQuery::default();
    let snapshot = b
        .context_hub(ContextHubRequest::ReadActivity(q.clone()))
        .await
        .unwrap();
    assert_eq!(snapshot["items"].as_array().unwrap().len(), 3);
    assert_eq!(snapshot["replace"], true);
    let mut incremental = q;
    incremental.cursor = snapshot["cursor"].as_str().map(String::from);
    let empty = b
        .context_hub(ContextHubRequest::ReadActivity(incremental.clone()))
        .await
        .unwrap();
    assert_eq!(empty["unchanged"], true);
    assert_eq!(empty["items"], json!([]));
    incremental.query = Some("missing-term".into());
    let different = b
        .context_hub(ContextHubRequest::ReadActivity(incremental))
        .await
        .unwrap();
    assert_eq!(different["unchanged"], false);
    assert_eq!(different["items"], json!([]));
    assert_eq!(a.page_count(vec![]).await.unwrap(), 0);
    let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
        .await
        .unwrap();
    db.state
        .activity
        .iter_mut()
        .for_each(|c| c.expires_at = "2000-01-01T00:00:00.000Z".into());
    db.save().unwrap();
    drop(db);
    let expired = b
        .context_hub(ContextHubRequest::ReadActivity(ActivityQuery {
            cursor: snapshot["cursor"].as_str().map(String::from),
            ..Default::default()
        }))
        .await
        .unwrap();
    assert_eq!(expired["replace"], true);
    assert_eq!(expired["items"], json!([]));
}

#[tokio::test]
async fn authorization_covers_publish_read_policies_and_candidate_bases() {
    let r = Rig::new().await;
    r.enable("a").await;
    r.enable("b").await;
    let a = r.client("a", &["a"]);
    let b = r.client("b", &["b"]);
    a.context_hub(ContextHubRequest::PublishActivity(activity(
        "private",
        "Only scope a",
    )))
    .await
    .unwrap();
    let invisible = b
        .context_hub(ContextHubRequest::ReadActivity(ActivityQuery::default()))
        .await
        .unwrap();
    assert_eq!(invisible["items"], json!([]));
    assert!(
        b.context_hub(ContextHubRequest::PublishActivity(activity("x", "no")))
            .await
            .is_err()
    );
    assert!(
        b.context_hub(ContextHubRequest::ReadActivity(ActivityQuery {
            scopes: vec!["a".into()],
            ..Default::default()
        }))
        .await
        .is_err()
    );
    assert!(
        a.context_hub(ContextHubRequest::SetPolicy(ClientContextPolicy {
            client_id: "a".into(),
            ..Default::default()
        }))
        .await
        .is_err()
    );
    let mut input = candidate("fakebasis");
    input.based_on_revision_ids = vec!["rev_missing".into()];
    assert!(
        a.context_hub(ContextHubRequest::SubmitCandidate(input))
            .await
            .is_err()
    );
    r.admin
        .context_hub(ContextHubRequest::SetPolicy(ClientContextPolicy {
            client_id: "a".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    assert!(
        a.context_hub(ContextHubRequest::PublishActivity(activity(
            "new", "disabled"
        )))
        .await
        .is_err()
    );
}

#[tokio::test]
async fn promotion_recovers_exact_plan_after_store_write_before_receipt_save() {
    let r = Rig::new().await;
    r.enable("a").await;
    let a = r.client("a", &["a"]);
    let receipt = a
        .context_hub(ContextHubRequest::SubmitCandidate(candidate("crash")))
        .await
        .unwrap();
    let request = promote(&receipt);
    let written = r
        .admin
        .context_hub(ContextHubRequest::Review(request.clone()))
        .await
        .unwrap();
    let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
        .await
        .unwrap();
    let item = &mut db.state.candidates[0];
    item.status = "promoting".into();
    item.version = 1;
    item.result = None;
    item.promotion_request = Some(request.clone());
    db.save().unwrap();
    drop(db);
    let reopened = ContextHub::new(r.store.clone(), r.hub.path.clone());
    let retry = reopened
        .execute(r.admin.access(), ContextHubRequest::Review(request))
        .await
        .unwrap();
    assert_eq!(retry, written);
    assert_eq!(a.page_count(vec![]).await.unwrap(), 1);
}

#[tokio::test]
async fn group_review_requires_same_scope_and_similarity_never_auto_promotes() {
    let r = Rig::new().await;
    r.enable("a").await;
    r.enable("b").await;
    let a = r.client("a", &["a"]);
    let b = r.client("b", &["a", "b"]);
    let one = a
        .context_hub(ContextHubRequest::SubmitCandidate(candidate("same")))
        .await
        .unwrap();
    let two = b
        .context_hub(ContextHubRequest::SubmitCandidate(candidate("same")))
        .await
        .unwrap();
    assert_ne!(one["candidateId"], two["candidateId"]);
    let list = r
        .admin
        .context_hub(ContextHubRequest::Inspect)
        .await
        .unwrap();
    assert_eq!(
        list["similarCandidates"][one["candidateId"].as_str().unwrap()][0],
        two["candidateId"]
    );
    assert_eq!(a.page_count(vec![]).await.unwrap(), 0);
    let mut request = promote(&one);
    request.candidates.push(promote(&two).candidates.remove(0));
    let mut other_input = candidate("other-scope");
    other_input.scope = "b".into();
    let other = b
        .context_hub(ContextHubRequest::SubmitCandidate(other_input))
        .await
        .unwrap();
    let mut cross_scope = promote(&one);
    cross_scope
        .candidates
        .push(promote(&other).candidates.remove(0));
    assert!(
        r.admin
            .context_hub(ContextHubRequest::Review(cross_scope))
            .await
            .is_err()
    );
    r.admin
        .context_hub(ContextHubRequest::Review(request))
        .await
        .unwrap();
    assert_eq!(a.page_count(vec![]).await.unwrap(), 1);
}

#[tokio::test]
async fn readable_bases_do_not_grant_cross_scope_derivation_or_read_only_writes() {
    let r = Rig::new().await;
    r.enable("a").await;
    let a = r.client("a", &["a", "b"]);
    let mut input = candidate("basis");
    input.scope = "b".into();
    let receipt = a
        .context_hub(ContextHubRequest::SubmitCandidate(input))
        .await
        .unwrap();
    let written = r
        .admin
        .context_hub(ContextHubRequest::Review(promote(&receipt)))
        .await
        .unwrap();
    let mut derived = candidate("derived");
    derived.based_on_revision_ids = vec![written["revisionId"].as_str().unwrap().into()];
    let error = a
        .context_hub(ContextHubRequest::SubmitCandidate(derived))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("cross-Scope"), "{error}");
    let mut read_access = access("reader-only", &["a"], false);
    read_access = AccessMode::Read.session(read_access.principal, "read", vec!["a".into()], false);
    let reader =
        EmbeddedPcpClient::new(r.store.clone(), read_access).with_context_hub(r.hub.clone());
    r.enable("reader-only").await;
    assert!(
        reader
            .context_hub(ContextHubRequest::SubmitCandidate(candidate("readonly")))
            .await
            .is_err()
    );
    assert!(
        reader
            .context_hub(ContextHubRequest::PublishActivity(activity(
                "readonly", "no"
            )))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn review_versions_defer_reject_and_represented_do_not_create_extra_pages() {
    let r = Rig::new().await;
    r.enable("a").await;
    let a = r.client("a", &["a"]);
    let one = a
        .context_hub(ContextHubRequest::SubmitCandidate(candidate("one")))
        .await
        .unwrap();
    let mut deferred = promote(&one);
    deferred.action = CandidateAction::Defer;
    let deferred_result = r
        .admin
        .context_hub(ContextHubRequest::Review(deferred.clone()))
        .await
        .unwrap();
    assert_eq!(
        deferred_result,
        r.admin
            .context_hub(ContextHubRequest::Review(deferred))
            .await
            .unwrap()
    );
    assert!(
        r.admin
            .context_hub(ContextHubRequest::Review(promote(&one)))
            .await
            .is_err()
    );
    let mut next = promote(&one);
    next.candidates[0].version = 2;
    let written = r
        .admin
        .context_hub(ContextHubRequest::Review(next))
        .await
        .unwrap();
    let two = a
        .context_hub(ContextHubRequest::SubmitCandidate(candidate("two")))
        .await
        .unwrap();
    let mut represented = promote(&two);
    represented.action = CandidateAction::Represented;
    represented.target_revision_id = written["revisionId"].as_str().map(String::from);
    assert_eq!(
        r.admin
            .context_hub(ContextHubRequest::Review(represented))
            .await
            .unwrap()["status"],
        "represented"
    );
    let three = a
        .context_hub(ContextHubRequest::SubmitCandidate(candidate("three")))
        .await
        .unwrap();
    let mut rejected = promote(&three);
    rejected.action = CandidateAction::Reject;
    r.admin
        .context_hub(ContextHubRequest::Review(rejected))
        .await
        .unwrap();
    assert_eq!(a.page_count(vec!["a".into()]).await.unwrap(), 1);
    assert_eq!(
        a.context_hub(ContextHubRequest::SubmitCandidate(candidate("three")))
            .await
            .unwrap()["status"],
        "rejected"
    );
}

#[tokio::test]
async fn concurrent_hub_instances_do_not_lose_distinct_submissions() {
    let r = Rig::new().await;
    r.enable("a").await;
    let other = Arc::new(ContextHub::new(r.store.clone(), r.hub.path.clone()));
    let first = r.client("a", &["a"]);
    let second =
        EmbeddedPcpClient::new(r.store.clone(), access("a", &["a"], false)).with_context_hub(other);
    let (one, two) = tokio::join!(
        first.context_hub(ContextHubRequest::SubmitCandidate(candidate(
            "concurrent-one"
        ))),
        second.context_hub(ContextHubRequest::SubmitCandidate(candidate(
            "concurrent-two"
        )))
    );
    assert!(one.is_ok() && two.is_ok());
    let list = r
        .admin
        .context_hub(ContextHubRequest::Inspect)
        .await
        .unwrap();
    assert_eq!(list["candidates"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn rpc_preserves_server_attested_identity_and_runtime_extension() {
    let r = Rig::new().await;
    r.enable("rpc-client").await;
    let client = r.client("rpc-client", &["a"]);
    let socket = std::env::temp_dir().join(format!(
        "h{}.sock",
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    ));
    let endpoint = pcp_rpc::RunningRuntimeEndpoint::start(&socket, client)
        .await
        .unwrap();
    let remote = pcp_rpc::RemotePcpClient::connect_expected(&socket, "rpc-client")
        .await
        .unwrap();
    remote
        .context_hub(ContextHubRequest::SubmitCandidate(candidate("rpc")))
        .await
        .unwrap();
    assert!(
        remote
            .context_hub(ContextHubRequest::Inspect)
            .await
            .is_err()
    );
    let snapshot = r
        .admin
        .context_hub(ContextHubRequest::Inspect)
        .await
        .unwrap();
    assert_eq!(snapshot["candidates"][0]["clientId"], "rpc-client");
    endpoint.shutdown().await;
}

#[tokio::test]
async fn rejects_oversized_input_and_symlinked_operational_state() {
    let r = Rig::new().await;
    r.enable("a").await;
    let a = r.client("a", &["a"]);
    assert!(
        a.context_hub(ContextHubRequest::PublishActivity(activity(
            "long",
            &"中".repeat(181)
        )))
        .await
        .is_err()
    );
    let mut invalid_source = candidate("invalid-source");
    invalid_source.source_refs.push(pcp_core::SourceRef {
        provider_id: "".into(),
        locator: "missing-provider".into(),
        media_type: None,
        content_digest: None,
    });
    assert!(
        a.context_hub(ContextHubRequest::SubmitCandidate(invalid_source))
            .await
            .is_err()
    );
    let unsafe_path = r.root.join("symlink.json");
    std::os::unix::fs::symlink(&r.hub.path, &unsafe_path).unwrap();
    let unsafe_hub = ContextHub::new(r.store.clone(), unsafe_path);
    assert!(
        unsafe_hub
            .execute(r.admin.access(), ContextHubRequest::Inspect)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn only_formal_promotion_notifies_the_page_maintenance_observer() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let r = Rig::new().await;
    r.enable("a").await;
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let client = EmbeddedPcpClient::new(r.store.clone(), access("a", &["a"], false))
        .with_context_hub(r.hub.clone())
        .with_successful_write_observer(Arc::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
        }));
    let receipt = client
        .context_hub(ContextHubRequest::SubmitCandidate(candidate("observer")))
        .await
        .unwrap();
    client
        .context_hub(ContextHubRequest::PublishActivity(activity(
            "observer",
            "a bounded snapshot",
        )))
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let count = calls.clone();
    let operator = EmbeddedPcpClient::new(r.store.clone(), access("operator:test", &[], true))
        .with_context_hub(r.hub.clone())
        .with_successful_write_observer(Arc::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
        }));
    operator
        .context_hub(ContextHubRequest::Review(promote(&receipt)))
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
