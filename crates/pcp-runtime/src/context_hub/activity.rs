//! Temporary handoff cards: capacity admission, publisher state and query snapshots.
use super::persistence::{ActivityCard, LockedState};
use super::{ContextHub, digest, permission, text_limit, timestamp};
use anyhow::{Result, ensure};
use chrono::{Duration, Utc};
use pcp_client::context_hub::{ActivityInput, ActivityQuery, ActivityStatus};
use pcp_core::{AccessPermission, AccessSession};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

pub(super) const MAX_TOPICS_PER_CLIENT_SCOPE: usize = 12;
pub(super) const MAX_CARDS: usize = 192;
const PAGE_PREFIX: &str = "activity-page-v1";

impl ContextHub {
    pub(super) fn publish(
        &self,
        access: &AccessSession,
        db: &mut LockedState,
        input: ActivityInput,
        now: &str,
    ) -> Result<Value> {
        permission(access, &input.scope, AccessPermission::Ingest)?;
        text_limit("topicKey", &input.topic_key, 64)?;
        text_limit("summary", &input.summary, 180)?;
        for (name, value, max) in [
            ("title", input.title.as_deref(), 120),
            ("nextStep", input.next_step.as_deref(), 180),
            ("parentTopicKey", input.parent_topic_key.as_deref(), 64),
        ] {
            if let Some(value) = value {
                text_limit(name, value, max)?;
            }
        }
        ensure!(
            input.parent_topic_key.as_deref() != Some(input.topic_key.as_str()),
            "activity parentTopicKey cannot refer to itself"
        );
        let ttl = input.ttl_hours.unwrap_or(48);
        ensure!((1..=168).contains(&ttl), "activity ttlHours must be 1..168");
        let client = &access.principal.principal_id;
        // Preserve the established identity across upgrades and Scope reconciliation.
        let id = format!("act_{}", digest(&(client, &input.topic_key))?);
        let existing = db.state.activity.iter().find(|c| c.card_id == id);
        if let Some(card) = existing {
            // A revoked Scope must not be moved into a readable one, carrying its
            // stored handoff metadata along with it.
            permission(access, &card.scope, AccessPermission::Ingest)?;
        }
        let title = input
            .title
            .or_else(|| existing.and_then(|c| c.title.clone()));
        let status = input.status.or_else(|| existing.and_then(|c| c.status));
        let next_step = input
            .next_step
            .or_else(|| existing.and_then(|c| c.next_step.clone()));
        let parent_topic_key = input
            .parent_topic_key
            .or_else(|| existing.and_then(|c| c.parent_topic_key.clone()));
        if let Some(card) = existing {
            if card.scope == input.scope
                && card.summary == input.summary
                && card.title == title
                && card.status == status
                && card.next_step == next_step
                && card.parent_topic_key == parent_topic_key
            {
                return Ok(
                    json!({"cardId":id,"version":card.version,"changed":false,"expiresAt":card.expires_at,"reclaimedCompletedCount":0}),
                );
            }
            ensure!(
                input.expected_version == Some(card.version),
                "activity card changed; read your current card before updating it"
            );
        } else {
            ensure!(
                input.expected_version.is_none(),
                "activity card expired or was removed; publish a fresh snapshot"
            );
        }

        // Plan admission without changing any existing card. Only this publisher's
        // explicitly completed cards can be reclaimed; missing state is unfinished.
        let mut completed = db
            .state
            .activity
            .iter()
            .filter(|card| {
                card.card_id != id
                    && card.client_id == *client
                    && card.status == Some(ActivityStatus::Completed)
            })
            .collect::<Vec<_>>();
        completed.sort_by(|a, b| {
            (a.scope != input.scope)
                .cmp(&(b.scope != input.scope))
                .then_with(|| a.updated_at.cmp(&b.updated_at))
                .then_with(|| a.card_id.cmp(&b.card_id))
        });
        let mut reclaimed = HashSet::new();
        if existing.is_none_or(|card| card.scope != input.scope) {
            let mut count = db
                .state
                .activity
                .iter()
                .filter(|card| card.client_id == *client && card.scope == input.scope)
                .count();
            for card in completed.iter().filter(|card| card.scope == input.scope) {
                if count < MAX_TOPICS_PER_CLIENT_SCOPE {
                    break;
                }
                reclaimed.insert(card.card_id.as_str());
                count -= 1;
            }
            ensure!(
                count < MAX_TOPICS_PER_CLIENT_SCOPE,
                "activity client/Scope capacity reached: 12 topics; update an existing topic, explicitly complete finished work, or wait for expiry. Existing cards were retained"
            );
        }
        if existing.is_none() {
            let mut count = db.state.activity.len() - reclaimed.len();
            for card in &completed {
                if count < MAX_CARDS {
                    break;
                }
                if reclaimed.insert(card.card_id.as_str()) {
                    count -= 1;
                }
            }
            ensure!(
                count < MAX_CARDS,
                "activity global capacity reached: 192 cards; no unfinished or other client's unexpired cards were removed. Update an existing topic or wait for expiry"
            );
        }
        let version = db
            .state
            .sequence
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("activity version capacity reached"))?;
        let expires = timestamp(Utc::now() + Duration::hours(ttl.into()));
        let reclaimed_count = reclaimed.len();
        // Owned IDs end the shared borrow before the atomic replacement.
        let reclaimed = reclaimed
            .into_iter()
            .map(str::to_owned)
            .collect::<HashSet<_>>();
        db.state
            .activity
            .retain(|card| card.card_id != id && !reclaimed.contains(&card.card_id));
        db.state.sequence = version;
        db.state.activity.push(ActivityCard {
            card_id: id.clone(),
            client_id: client.clone(),
            scope: input.scope,
            topic_key: input.topic_key,
            summary: input.summary,
            title,
            status,
            next_step,
            parent_topic_key,
            version,
            updated_at: now.into(),
            expires_at: expires.clone(),
        });
        db.save()?;
        Ok(
            json!({"cardId":id,"version":version,"changed":true,"expiresAt":expires,"reclaimedCompletedCount":reclaimed_count}),
        )
    }

    pub(super) fn activity(
        &self,
        access: &AccessSession,
        db: &LockedState,
        query: ActivityQuery,
    ) -> Result<Value> {
        ensure!(query.scopes.len() <= 32, "too many activity scopes");
        for scope in &query.scopes {
            permission(access, scope, AccessPermission::ReadDetail)?;
        }
        let limit = query.limit.unwrap_or(5);
        ensure!((1..=5).contains(&limit), "activity limit must be 1..5");
        ensure!(
            query.cursor.is_none() || query.page_cursor.is_none(),
            "cursor checks changes; pageCursor continues pages. Omit cursor while paging"
        );
        if let Some(q) = &query.query {
            text_limit("query", q, 120)?;
        }
        let terms = query
            .query
            .as_deref()
            .unwrap_or("")
            .split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>();
        let mut visible = db
            .state
            .activity
            .iter()
            .filter(|card| {
                access.allows(&card.scope, AccessPermission::ReadDetail)
                    && (query.scopes.is_empty() || query.scopes.contains(&card.scope))
                    && (query.include_own || card.client_id != access.principal.principal_id)
                    && db
                        .state
                        .policies
                        .iter()
                        .any(|p| p.client_id == card.client_id && p.publish_activity)
                    && (terms.is_empty()
                        || terms.iter().any(|term| {
                            format!(
                                "{} {} {} {}",
                                card.topic_key,
                                card.summary,
                                card.title.as_deref().unwrap_or(""),
                                card.next_step.as_deref().unwrap_or("")
                            )
                            .to_lowercase()
                            .contains(term)
                        }))
            })
            .collect::<Vec<_>>();
        visible.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.card_id.cmp(&b.card_id))
        });
        // A default overview gives each Scope a turn. Explicit Scope or text
        // queries retain the established newest-first order.
        if query.scopes.is_empty() && terms.is_empty() {
            let mut ranks = HashMap::<&str, usize>::new();
            let mut ordered = visible
                .into_iter()
                .enumerate()
                .map(|(index, card)| {
                    let rank = ranks.entry(card.scope.as_str()).or_default();
                    let row = (*rank, index, card);
                    *rank += 1;
                    row
                })
                .collect::<Vec<_>>();
            ordered.sort_by_key(|(rank, index, _)| (*rank, *index));
            visible = ordered.into_iter().map(|(_, _, card)| card).collect();
        }
        // Bind the complete visible snapshot, not only the first five cards.
        // Private cards are never included in the digest or continuation token.
        let cursor = digest(&(
            "activity-query-v2",
            &db.state.identity_id,
            &access.principal,
            &access.grants,
            &access.store_permissions,
            &query.scopes,
            &query.query,
            query.include_own,
            limit,
            &visible,
        ))?;
        let offset = match query.page_cursor.as_deref() {
            None => 0,
            Some(token) => {
                ensure!(
                    token.len() <= 128,
                    "invalid activity pageCursor; read the first page again"
                );
                let mut parts = token.split(':');
                let prefix = parts.next();
                let offset = parts.next().and_then(|n| n.parse::<usize>().ok());
                let snapshot = parts.next();
                ensure!(
                    prefix == Some(PAGE_PREFIX)
                        && snapshot == Some(cursor.as_str())
                        && parts.next().is_none()
                        && offset
                            .is_some_and(|n| n > 0 && n < visible.len() && n % limit as usize == 0),
                    "activity pageCursor is invalid or its query, permissions or snapshot changed; read the first page again"
                );
                offset.unwrap()
            }
        };
        let end = (offset + limit as usize).min(visible.len());
        let truncated = end < visible.len();
        let next_cursor = truncated.then(|| format!("{PAGE_PREFIX}:{end}:{cursor}"));
        let unchanged = query.cursor.as_deref() == Some(cursor.as_str());
        Ok(if unchanged {
            json!({"items":[],"cursor":cursor,"unchanged":true,"nextCursor":next_cursor})
        } else {
            json!({"items":&visible[offset..end],"cursor":cursor,"unchanged":false,
                "replace":offset == 0,"truncated":truncated,"nextCursor":next_cursor})
        })
    }
}
