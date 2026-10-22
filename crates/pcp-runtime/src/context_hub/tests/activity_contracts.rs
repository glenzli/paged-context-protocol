use super::*;
use crate::context_hub::activity::{MAX_CARDS, MAX_TOPICS_PER_CLIENT_SCOPE};
use crate::context_hub::persistence::ActivityCard;
use std::collections::BTreeSet;

#[tokio::test]
async fn scope_quota_preserves_active_and_legacy_cards_and_isolates_clients() {
    let r = Rig::new().await;
    r.enable("publisher").await;
    r.enable("peer").await;
    let client = r.client("publisher", &["a", "b"]);
    let mut receipts = Vec::new();
    for index in 0..MAX_TOPICS_PER_CLIENT_SCOPE {
        receipts.push(
            client
                .context_hub(ContextHubRequest::PublishActivity(activity(
                    &format!("a-{index}"),
                    "Legacy state is unknown, not completed",
                )))
                .await
                .unwrap(),
        );
    }
    let before = std::fs::read(&r.hub.path).unwrap();
    let error = client
        .context_hub(ContextHubRequest::PublishActivity(activity(
            "a-overflow",
            "new topic",
        )))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("client/Scope capacity reached"), "{error}");
    assert_eq!(before, std::fs::read(&r.hub.path).unwrap());
    // Other Scopes and other clients have separate admission quotas.
    for index in 0..MAX_TOPICS_PER_CLIENT_SCOPE {
        let mut input = activity(&format!("b-{index}"), "another project");
        input.scope = "b".into();
        client
            .context_hub(ContextHubRequest::PublishActivity(input))
            .await
            .unwrap();
        r.client("peer", &["a"])
            .context_hub(ContextHubRequest::PublishActivity(activity(
                &format!("peer-{index}"),
                "another publisher",
            )))
            .await
            .unwrap();
    }
    let db = LockedState::open(&r.hub.path, r.store.identity_id())
        .await
        .unwrap();
    assert_eq!(db.state.activity.len(), 36);
    for receipt in receipts {
        assert!(
            db.state
                .activity
                .iter()
                .any(|card| card.card_id == receipt["cardId"].as_str().unwrap())
        );
    }
    assert!(db.state.activity.iter().all(|card| card.status.is_none()));
}

#[tokio::test]
async fn same_scope_completion_is_reclaimed_before_other_scope_and_move_rejection_is_atomic() {
    let r = Rig::new().await;
    r.enable("publisher").await;
    let client = r.client("publisher", &["a", "b"]);
    let mut b = activity("b-completed", "finished project");
    b.scope = "b".into();
    b.status = Some(ActivityStatus::Completed);
    let b_receipt = client
        .context_hub(ContextHubRequest::PublishActivity(b))
        .await
        .unwrap();
    for index in 0..MAX_TOPICS_PER_CLIENT_SCOPE {
        let mut input = activity(&format!("a-{index}"), "project state");
        input.status = Some(if index == 0 {
            ActivityStatus::Completed
        } else {
            ActivityStatus::InProgress
        });
        client
            .context_hub(ContextHubRequest::PublishActivity(input))
            .await
            .unwrap();
    }
    let written = client
        .context_hub(ContextHubRequest::PublishActivity(activity(
            "a-next",
            "new topic",
        )))
        .await
        .unwrap();
    assert_eq!(written["reclaimedCompletedCount"], 1);
    let snapshot = r
        .admin
        .context_hub(ContextHubRequest::InspectActivity)
        .await
        .unwrap();
    let cards = snapshot["activity"].as_array().unwrap();
    assert_eq!(cards.len(), 13);
    assert!(!cards.iter().any(|c| c["topicKey"] == "a-0"));
    assert!(cards.iter().any(|c| c["topicKey"] == "b-completed"));
    let before = std::fs::read(&r.hub.path).unwrap();
    let mut move_to_full_scope = activity("b-completed", "move this card");
    move_to_full_scope.expected_version = b_receipt["version"].as_u64();
    assert!(
        client
            .context_hub(ContextHubRequest::PublishActivity(move_to_full_scope))
            .await
            .is_err()
    );
    assert_eq!(before, std::fs::read(&r.hub.path).unwrap());
    let mut update = activity("a-next", "update at full capacity");
    update.expected_version = written["version"].as_u64();
    assert!(
        client
            .context_hub(ContextHubRequest::PublishActivity(update))
            .await
            .unwrap()["changed"]
            == true
    );
}

#[tokio::test]
async fn reclaimed_topic_rejects_stale_receipts_and_reuses_its_stable_id_with_a_new_version() {
    let r = Rig::new().await;
    r.enable("publisher").await;
    let client = r.client("publisher", &["a"]);
    let mut done = activity("old-topic", "completed evidence");
    done.status = Some(ActivityStatus::Completed);
    let first = client
        .context_hub(ContextHubRequest::PublishActivity(done))
        .await
        .unwrap();
    let mut last = Value::Null;
    for index in 0..MAX_TOPICS_PER_CLIENT_SCOPE {
        last = client
            .context_hub(ContextHubRequest::PublishActivity(activity(
                &format!("active-{index}"),
                "still active",
            )))
            .await
            .unwrap();
    }
    assert_eq!(last["reclaimedCompletedCount"], 1);
    let before = std::fs::read(&r.hub.path).unwrap();
    let mut stale = activity("old-topic", "resumed evidence");
    stale.expected_version = first["version"].as_u64();
    let error = client
        .context_hub(ContextHubRequest::PublishActivity(stale.clone()))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("expired or was removed"), "{error}");
    assert_eq!(before, std::fs::read(&r.hub.path).unwrap());
    let mut release = activity("active-11", "explicitly finished");
    release.status = Some(ActivityStatus::Completed);
    release.expected_version = last["version"].as_u64();
    client
        .context_hub(ContextHubRequest::PublishActivity(release))
        .await
        .unwrap();
    let rebuilt = client
        .context_hub(ContextHubRequest::PublishActivity(activity(
            "old-topic",
            "resumed evidence",
        )))
        .await
        .unwrap();
    assert_eq!(rebuilt["cardId"], first["cardId"]);
    assert!(rebuilt["version"].as_u64().unwrap() > first["version"].as_u64().unwrap());
    stale.summary = "attempt using the older receipt".into();
    assert!(
        client
            .context_hub(ContextHubRequest::PublishActivity(stale))
            .await
            .is_err()
    );
    let replay = client
        .context_hub(ContextHubRequest::PublishActivity(activity(
            "old-topic",
            "resumed evidence",
        )))
        .await
        .unwrap();
    assert_eq!(replay["changed"], false);
    assert_eq!(replay["version"], rebuilt["version"]);
}

#[tokio::test]
async fn global_capacity_never_reclaims_another_clients_unexpired_completed_card() {
    let r = Rig::new().await;
    for id in ["newcomer", "seed-0"] {
        r.enable(id).await;
    }
    let now = timestamp(Utc::now());
    let expires = timestamp(Utc::now() + Duration::hours(48));
    {
        let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
            .await
            .unwrap();
        for index in 0..MAX_CARDS {
            let client = format!("seed-{}", index / MAX_TOPICS_PER_CLIENT_SCOPE);
            let topic = format!("seed-topic-{index}");
            db.state.activity.push(ActivityCard {
                card_id: format!("act_{}", digest(&(&client, &topic)).unwrap()),
                client_id: client,
                scope: "a".into(),
                topic_key: topic,
                summary: "unexpired snapshot".into(),
                title: None,
                next_step: None,
                parent_topic_key: None,
                status: (index == 0 || index == 12).then_some(ActivityStatus::Completed),
                version: index as u64 + 1,
                updated_at: now.clone(),
                expires_at: expires.clone(),
            });
        }
        db.state.sequence = MAX_CARDS as u64;
        db.save().unwrap();
    }
    let newcomer = r.client("newcomer", &["b"]);
    let mut input = activity(
        "new-project",
        "no authority over another publisher's completed cards",
    );
    input.scope = "b".into();
    let before = std::fs::read(&r.hub.path).unwrap();
    let error = newcomer
        .context_hub(ContextHubRequest::PublishActivity(input.clone()))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("global capacity reached"), "{error}");
    assert_eq!(before, std::fs::read(&r.hub.path).unwrap());
    // The card owner can reclaim its own completed card across Scopes.
    let owner = r.client("seed-0", &["a", "b"]);
    let receipt = owner
        .context_hub(ContextHubRequest::PublishActivity(input.clone()))
        .await
        .unwrap();
    assert_eq!(receipt["reclaimedCompletedCount"], 1);
    {
        let db = LockedState::open(&r.hub.path, r.store.identity_id())
            .await
            .unwrap();
        assert_eq!(db.state.activity.len(), MAX_CARDS);
        assert!(
            !db.state
                .activity
                .iter()
                .any(|c| c.topic_key == "seed-topic-0")
        );
        assert!(
            db.state
                .activity
                .iter()
                .any(|c| c.topic_key == "seed-topic-12")
        );
        assert!(
            db.state
                .activity
                .iter()
                .any(|c| c.topic_key == "seed-topic-1")
        );
    }
    let mut update = activity("seed-topic-1", "update without increasing cardinality");
    update.expected_version = Some(2);
    owner
        .context_hub(ContextHubRequest::PublishActivity(update))
        .await
        .unwrap();
    // Natural expiry remains independent of client identity and frees capacity.
    {
        let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
            .await
            .unwrap();
        db.state
            .activity
            .iter_mut()
            .find(|c| c.topic_key == "seed-topic-100")
            .unwrap()
            .expires_at = "2000-01-01T00:00:00.000Z".into();
        db.save().unwrap();
    }
    let fresh = newcomer
        .context_hub(ContextHubRequest::PublishActivity(input))
        .await
        .unwrap();
    assert_eq!(fresh["reclaimedCompletedCount"], 0);
    let db = LockedState::open(&r.hub.path, r.store.identity_id())
        .await
        .unwrap();
    assert_eq!(db.state.activity.len(), MAX_CARDS);
    assert!(
        !db.state
            .activity
            .iter()
            .any(|c| c.topic_key == "seed-topic-100")
    );
}

#[tokio::test]
async fn legacy_persistence_and_omitted_metadata_preserve_unknown_and_explicit_publisher_state() {
    let r = Rig::new().await;
    r.enable("publisher").await;
    let client = r.client("publisher", &["a", "b"]);
    let legacy: ActivityInput = serde_json::from_value(json!({
        "scope":"a","topicKey":"legacy","summary":"old client",
    }))
    .unwrap();
    let first = client
        .context_hub(ContextHubRequest::PublishActivity(legacy))
        .await
        .unwrap();
    // Saving and reopening old schema-1 cards does not invent a completed status.
    let persisted: Value = serde_json::from_slice(&std::fs::read(&r.hub.path).unwrap()).unwrap();
    assert!(persisted["activity"][0].get("status").is_none());
    let mut input = activity("legacy", "old client");
    input.title = Some("可读主题".into());
    input.status = Some(ActivityStatus::WaitingUser);
    input.next_step = Some("用户重启后验证侧栏".into());
    input.parent_topic_key = Some("main-topic".into());
    input.expected_version = first["version"].as_u64();
    let changed = client
        .context_hub(ContextHubRequest::PublishActivity(input.clone()))
        .await
        .unwrap();
    let replay = client
        .context_hub(ContextHubRequest::PublishActivity(input))
        .await
        .unwrap();
    assert_eq!(replay["changed"], false);
    assert_eq!(replay["version"], changed["version"]);
    assert_eq!(first["cardId"], changed["cardId"]);
    let mut old_client_update = activity("legacy", "summary updated by an older client");
    old_client_update.expected_version = changed["version"].as_u64();
    let updated = client
        .context_hub(ContextHubRequest::PublishActivity(old_client_update))
        .await
        .unwrap();
    let details = client
        .context_hub(ContextHubRequest::ReadActivity(ActivityQuery::default()))
        .await
        .unwrap();
    assert_eq!(details["items"][0]["title"], "可读主题");
    assert_eq!(details["items"][0]["status"], "waiting_user");
    assert_eq!(details["items"][0]["nextStep"], "用户重启后验证侧栏");
    assert_eq!(details["items"][0]["parentTopicKey"], "main-topic");
    let mut finish = activity("legacy", "finished");
    finish.expected_version = updated["version"].as_u64();
    finish.status = Some(ActivityStatus::Completed);
    let done = client
        .context_hub(ContextHubRequest::PublishActivity(finish))
        .await
        .unwrap();
    let mut resume = activity("legacy", "resumed explicitly");
    resume.expected_version = done["version"].as_u64();
    resume.status = Some(ActivityStatus::InProgress);
    client
        .context_hub(ContextHubRequest::PublishActivity(resume))
        .await
        .unwrap();
    let db = LockedState::open(&r.hub.path, r.store.identity_id())
        .await
        .unwrap();
    assert_eq!(
        db.state.activity[0].status,
        Some(ActivityStatus::InProgress)
    );
    drop(db);
    // Metadata cannot be moved out of a Scope after that grant was revoked.
    let mut revoked_move = activity("legacy", "move without prior Scope access");
    revoked_move.scope = "b".into();
    assert!(
        r.client("publisher", &["b"])
            .context_hub(ContextHubRequest::PublishActivity(revoked_move))
            .await
            .is_err()
    );
    let before = std::fs::read(&r.hub.path).unwrap();
    for (title, next, parent) in [
        (Some(" ".into()), None, None),
        (Some("中".repeat(121)), None, None),
        (None, Some("中".repeat(181)), None),
        (None, None, Some("bad".into())),
    ] {
        let mut bad = activity("bad", "invalid metadata");
        bad.title = title;
        bad.next_step = next;
        bad.parent_topic_key = parent;
        assert!(
            client
                .context_hub(ContextHubRequest::PublishActivity(bad))
                .await
                .is_err()
        );
        assert_eq!(before, std::fs::read(&r.hub.path).unwrap());
    }
}

#[tokio::test]
async fn pagination_reads_every_visible_card_without_private_cards_or_duplicate_items() {
    let r = Rig::new().await;
    r.enable("publisher").await;
    r.enable("reader").await;
    let publisher = r.client("publisher", &["a", "b"]);
    for index in 0..11 {
        publisher
            .context_hub(ContextHubRequest::PublishActivity(activity(
                &format!("a-{index}"),
                "shared state",
            )))
            .await
            .unwrap();
    }
    let mut private = activity("b-private", "private state");
    private.scope = "b".into();
    publisher
        .context_hub(ContextHubRequest::PublishActivity(private))
        .await
        .unwrap();
    let reader = r.client("reader", &["a"]);
    let mut query = ActivityQuery {
        scopes: vec!["a".into()],
        ..Default::default()
    };
    let mut ids = BTreeSet::new();
    let mut pages = Vec::new();
    loop {
        let page = reader
            .context_hub(ContextHubRequest::ReadActivity(query.clone()))
            .await
            .unwrap();
        assert_eq!(page["replace"], query.page_cursor.is_none());
        assert_eq!(page["unchanged"], false);
        let cards = page["items"].as_array().unwrap();
        pages.push(cards.len());
        for card in cards {
            assert_eq!(card["scope"], "a");
            assert!(ids.insert(card["cardId"].as_str().unwrap().to_string()));
        }
        query.page_cursor = page["nextCursor"].as_str().map(str::to_owned);
        if query.page_cursor.is_none() {
            assert_eq!(page["truncated"], false);
            break;
        }
    }
    assert_eq!(pages, [5, 5, 1]);
    assert_eq!(ids.len(), 11);
}

#[tokio::test]
async fn pagination_tokens_bind_query_principal_permissions_and_the_whole_visible_snapshot() {
    let r = Rig::new().await;
    for id in ["publisher", "reader", "other-reader"] {
        r.enable(id).await;
    }
    let publisher = r.client("publisher", &["a", "b"]);
    for index in 0..8 {
        publisher
            .context_hub(ContextHubRequest::PublishActivity(activity(
                &format!("topic-{index}"),
                "shared state",
            )))
            .await
            .unwrap();
    }
    // Make deterministic chronology so the final card stays outside page one.
    {
        let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
            .await
            .unwrap();
        for (index, card) in db.state.activity.iter_mut().enumerate() {
            card.updated_at = format!("2026-10-20T00:00:{index:02}.000Z");
        }
        db.save().unwrap();
    }
    let reader = r.client("reader", &["a"]);
    let base = ActivityQuery {
        scopes: vec!["a".into()],
        ..Default::default()
    };
    let first = reader
        .context_hub(ContextHubRequest::ReadActivity(base.clone()))
        .await
        .unwrap();
    let page = ActivityQuery {
        page_cursor: first["nextCursor"].as_str().map(str::to_owned),
        ..base.clone()
    };
    assert!(page.page_cursor.is_some());
    for changed in [
        ActivityQuery {
            limit: Some(4),
            ..page.clone()
        },
        ActivityQuery {
            query: Some("shared".into()),
            ..page.clone()
        },
        ActivityQuery {
            scopes: vec![],
            ..page.clone()
        },
        ActivityQuery {
            include_own: false,
            ..page.clone()
        },
        ActivityQuery {
            cursor: Some("change-check-token".into()),
            ..page.clone()
        },
        ActivityQuery {
            page_cursor: Some("bad-token".into()),
            ..page.clone()
        },
    ] {
        assert!(
            reader
                .context_hub(ContextHubRequest::ReadActivity(changed))
                .await
                .is_err()
        );
    }
    assert!(
        r.client("other-reader", &["a"])
            .context_hub(ContextHubRequest::ReadActivity(page.clone()))
            .await
            .is_err()
    );
    // Even a grant change with the same explicitly selected visible Scope is bound.
    assert!(
        r.client("reader", &["a", "b"])
            .context_hub(ContextHubRequest::ReadActivity(page.clone()))
            .await
            .is_err()
    );
    // Changes in an unreadable Scope neither disclose cards nor invalidate a visible snapshot.
    let mut private = activity("private", "private update");
    private.scope = "b".into();
    publisher
        .context_hub(ContextHubRequest::PublishActivity(private))
        .await
        .unwrap();
    let unchanged = reader
        .context_hub(ContextHubRequest::ReadActivity(ActivityQuery {
            cursor: first["cursor"].as_str().map(str::to_owned),
            ..base.clone()
        }))
        .await
        .unwrap();
    assert_eq!(unchanged["unchanged"], true);
    assert_eq!(
        reader
            .context_hub(ContextHubRequest::ReadActivity(page.clone()))
            .await
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    {
        let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
            .await
            .unwrap();
        let last = db
            .state
            .activity
            .iter_mut()
            .find(|c| c.topic_key == "topic-0")
            .unwrap();
        last.summary = "changed beyond the first page".into();
        last.version += 1;
        db.save().unwrap();
    }
    let changed = reader
        .context_hub(ContextHubRequest::ReadActivity(ActivityQuery {
            cursor: first["cursor"].as_str().map(str::to_owned),
            ..base
        }))
        .await
        .unwrap();
    assert_eq!(changed["unchanged"], false);
    assert_eq!(changed["items"], first["items"]);
    let error = reader
        .context_hub(ContextHubRequest::ReadActivity(page))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("snapshot changed"), "{error}");
    let fresh = reader
        .context_hub(ContextHubRequest::ReadActivity(ActivityQuery::default()))
        .await
        .unwrap();
    r.admin
        .context_hub(ContextHubRequest::SetPolicy(ClientContextPolicy {
            client_id: "publisher".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    assert!(
        reader
            .context_hub(ContextHubRequest::ReadActivity(ActivityQuery {
                page_cursor: fresh["nextCursor"].as_str().map(str::to_owned),
                ..Default::default()
            }))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn default_overview_balances_scopes_while_explicit_queries_preserve_recency() {
    let r = Rig::new().await;
    r.enable("publisher").await;
    let client = r.client("publisher", &["a", "b"]);
    for index in 0..7 {
        client
            .context_hub(ContextHubRequest::PublishActivity(activity(
                &format!("a-{index}"),
                "common topic",
            )))
            .await
            .unwrap();
    }
    let mut b = activity("b-older", "common topic");
    b.scope = "b".into();
    client
        .context_hub(ContextHubRequest::PublishActivity(b))
        .await
        .unwrap();
    {
        let mut db = LockedState::open(&r.hub.path, r.store.identity_id())
            .await
            .unwrap();
        for (index, card) in db.state.activity.iter_mut().enumerate() {
            card.updated_at = if card.scope == "a" {
                format!("2026-10-20T00:00:{index:02}.000Z")
            } else {
                "2026-10-19T00:00:00.000Z".into()
            };
        }
        db.save().unwrap();
    }
    let overview = client
        .context_hub(ContextHubRequest::ReadActivity(ActivityQuery::default()))
        .await
        .unwrap();
    assert_eq!(overview["items"][0]["scope"], "a");
    assert_eq!(overview["items"][1]["scope"], "b");
    for query in [
        ActivityQuery {
            scopes: vec!["a".into(), "b".into()],
            ..Default::default()
        },
        ActivityQuery {
            query: Some("common".into()),
            ..Default::default()
        },
    ] {
        let page = client
            .context_hub(ContextHubRequest::ReadActivity(query))
            .await
            .unwrap();
        assert!(
            page["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|c| c["scope"] == "a")
        );
        assert_eq!(page["items"][0]["topicKey"], "a-6");
    }
}
