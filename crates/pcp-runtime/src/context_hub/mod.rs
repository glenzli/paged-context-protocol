//! Opt-in candidate inbox and bounded activity snapshots, separate from Page recall.
mod persistence;
mod review;
#[cfg(test)]
mod tests;

use anyhow::{Result, ensure};
use async_trait::async_trait;
use chrono::{Duration, SecondsFormat, Utc};
use pcp_client::{EmbeddedPcpClient, PcpTenantApi, context_hub::*};
use pcp_core::{AccessPermission, AccessSession, Projection, ReadPagesRequest};
use pcp_store::PcpStore;
use persistence::{ActivityCard, Candidate, LockedState};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub struct ContextHub {
    store: Arc<dyn PcpStore>,
    path: PathBuf,
}

impl ContextHub {
    pub fn new(store: Arc<dyn PcpStore>, path: PathBuf) -> Self {
        Self { store, path }
    }

    pub fn state_path(store_path: &Path) -> PathBuf {
        store_path.with_extension("context.json")
    }

    async fn validate_basis(&self, access: &AccessSession, input: &CandidateInput) -> Result<()> {
        ensure!(
            input.source_refs.len() <= 8 && input.based_on_revision_ids.len() <= 16,
            "too many candidate source references"
        );
        ensure!(
            serde_json::to_vec(&input.source_refs)?.len() <= 4096,
            "candidate source references exceed budget"
        );
        ensure!(
            input
                .source_refs
                .iter()
                .all(|source| !source.provider_id.trim().is_empty()
                    && !source.locator.trim().is_empty()),
            "candidate source providerId and locator cannot be empty"
        );
        if input.based_on_revision_ids.is_empty() {
            return Ok(());
        }
        let client = EmbeddedPcpClient::new(self.store.clone(), access.clone());
        let pages = client
            .read_pages(ReadPagesRequest {
                page_ids: vec![],
                revision_ids: input.based_on_revision_ids.clone(),
                projections: vec![Projection::Manifest],
                max_chars: 1,
            })
            .await?;
        for id in &input.based_on_revision_ids {
            let page = pages
                .iter()
                .find(|p| &p.revision.revision_id == id)
                .ok_or_else(|| anyhow::anyhow!("candidate basis Revision is unavailable"))?;
            if page.revision.namespace != input.scope {
                ensure!(
                    access.allows(&input.scope, AccessPermission::DeriveAcrossScopes)
                        && access.allows(
                            &page.revision.namespace,
                            AccessPermission::DeriveAcrossScopes
                        ),
                    "cross-Scope candidate derivation is not authorized"
                );
            }
        }
        Ok(())
    }

    async fn submit(
        &self,
        access: &AccessSession,
        db: &mut LockedState,
        input: CandidateInput,
        now: &str,
    ) -> Result<Value> {
        permission(access, &input.scope, AccessPermission::Ingest)?;
        text_limit("eventId", &input.event_id, 160)?;
        text_limit("title", &input.title, 120)?;
        text_limit("content", &input.content, 2000)?;
        self.validate_basis(access, &input).await?;
        let id = format!(
            "cand_{}",
            digest(&(access.principal.principal_id.as_str(), &input.event_id))?
        );
        if let Some(existing) = db.state.candidates.iter().find(|c| c.candidate_id == id) {
            ensure!(
                serde_json::to_value(&existing.input)? == serde_json::to_value(&input)?,
                "eventId was already used for different candidate content"
            );
            return Ok(
                json!({"candidateId":id, "status":existing.status, "created":false, "version":existing.version, "result":existing.result}),
            );
        }
        ensure!(
            db.state.candidates.len() < 500,
            "candidate inbox capacity reached; wait for expiry, do not retry repeatedly"
        );
        ensure!(
            db.state
                .candidates
                .iter()
                .filter(|c| c.client_id == access.principal.principal_id
                    && matches!(c.status.as_str(), "pending" | "deferred" | "promoting"))
                .count()
                < 50,
            "client candidate quota reached; do not retry this submission repeatedly"
        );
        db.state.candidates.push(Candidate {
            candidate_id: id.clone(),
            client_id: access.principal.principal_id.clone(),
            input,
            created_at: now.into(),
            expires_at: timestamp(Utc::now() + Duration::days(30)),
            version: 1,
            status: "pending".into(),
            snoozed_until: None,
            review_key: None,
            promotion_request: None,
            result: None,
        });
        db.save()?;
        Ok(json!({"candidateId":id, "status":"pending", "created":true, "version":1}))
    }

    fn publish(
        &self,
        access: &AccessSession,
        db: &mut LockedState,
        input: ActivityInput,
        now: &str,
    ) -> Result<Value> {
        permission(access, &input.scope, AccessPermission::Ingest)?;
        text_limit("topicKey", &input.topic_key, 64)?;
        text_limit("summary", &input.summary, 180)?;
        let ttl = input.ttl_hours.unwrap_or(48);
        ensure!((1..=168).contains(&ttl), "activity ttlHours must be 1..168");
        let client = &access.principal.principal_id;
        let id = format!("act_{}", digest(&(client, &input.topic_key))?);
        if let Some(card) = db.state.activity.iter().find(|c| c.card_id == id) {
            if card.scope == input.scope && card.summary == input.summary {
                return Ok(
                    json!({"cardId":id, "version":card.version, "changed":false, "expiresAt":card.expires_at}),
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
        db.state.activity.retain(|c| c.card_id != id);
        let own: Vec<_> = db
            .state
            .activity
            .iter()
            .filter(|c| &c.client_id == client)
            .collect();
        if own.len() >= 3 {
            let oldest = own
                .iter()
                .min_by_key(|c| &c.updated_at)
                .unwrap()
                .card_id
                .clone();
            db.state.activity.retain(|c| c.card_id != oldest);
        }
        ensure!(db.state.activity.len() < 192, "activity capacity reached");
        db.state.sequence += 1;
        let version = db.state.sequence;
        let expires = timestamp(Utc::now() + Duration::hours(ttl.into()));
        db.state.activity.push(ActivityCard {
            card_id: id.clone(),
            client_id: client.clone(),
            scope: input.scope,
            topic_key: input.topic_key,
            summary: input.summary,
            version,
            updated_at: now.into(),
            expires_at: expires.clone(),
        });
        db.save()?;
        Ok(json!({"cardId":id,"version":version,"changed":true,"expiresAt":expires}))
    }

    fn activity(
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
        if let Some(q) = &query.query {
            text_limit("query", q, 120)?;
        }
        let terms: Vec<_> = query
            .query
            .as_deref()
            .unwrap_or("")
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let mut visible: Vec<_> = db
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
                            format!("{} {}", card.topic_key, card.summary)
                                .to_lowercase()
                                .contains(term)
                        }))
            })
            .collect();
        visible.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.card_id.cmp(&b.card_id))
        });
        let truncated = visible.len() > limit as usize;
        visible.truncate(limit as usize);
        let cursor = digest(&(
            &access.principal.principal_id,
            &query.scopes,
            &query.query,
            query.include_own,
            limit,
            &visible,
            truncated,
        ))?;
        let unchanged = query.cursor.as_deref() == Some(cursor.as_str());
        Ok(if unchanged {
            json!({"items":[],"cursor":cursor,"unchanged":true})
        } else {
            json!({"items":visible,"cursor":cursor,"unchanged":false,"replace":true,"truncated":truncated})
        })
    }
}

#[async_trait]
impl ContextHubService for ContextHub {
    async fn execute(&self, access: &AccessSession, request: ContextHubRequest) -> Result<Value> {
        let mut db = LockedState::open(&self.path, self.store.identity_id()).await?;
        let now = timestamp(Utc::now());
        let pruned = db.prune(&now);
        let policy = db
            .state
            .policies
            .iter()
            .find(|p| p.client_id == access.principal.principal_id)
            .cloned()
            .unwrap_or_default();
        match request {
            ContextHubRequest::SubmitCandidate(input) => {
                ensure!(
                    policy.submit_candidates,
                    "Candidate submission is disabled for this client; enable it in Console"
                );
                self.submit(access, &mut db, input, &now).await
            }
            ContextHubRequest::PublishActivity(input) => {
                ensure!(
                    policy.publish_activity,
                    "Activity publishing is disabled for this client; enable it in Console"
                );
                self.publish(access, &mut db, input, &now)
            }
            ContextHubRequest::ReadActivity(query) => {
                ensure!(
                    policy.read_activity || operator(access),
                    "Activity reading is disabled for this client; enable it in Console"
                );
                let result = self.activity(access, &db, query)?;
                if pruned {
                    db.save()?;
                }
                Ok(result)
            }
            ContextHubRequest::Inspect => {
                require_operator(access)?;
                db.save()?; // also removes expired operational content without model work
                let suggestions = review::similar_candidates(&db.state.candidates);
                Ok(
                    json!({"policies":db.state.policies,"candidates":db.state.candidates,"activity":db.state.activity,"similarCandidates":suggestions}),
                )
            }
            ContextHubRequest::SetPolicy(policy) => {
                require_operator(access)?;
                text_limit("clientId", &policy.client_id, 200)?;
                ensure!(
                    db.state.policies.len() < 64
                        || db
                            .state
                            .policies
                            .iter()
                            .any(|p| p.client_id == policy.client_id),
                    "too many context clients"
                );
                db.state
                    .policies
                    .retain(|p| p.client_id != policy.client_id);
                db.state.policies.push(policy);
                db.save()?;
                Ok(json!({"saved":true}))
            }
            ContextHubRequest::Review(request) => {
                require_operator(access)?;
                self.review(access, &mut db, request).await
            }
            ContextHubRequest::RemoveActivity { card_id, version } => {
                require_operator(access)?;
                let card = db
                    .state
                    .activity
                    .iter()
                    .find(|c| c.card_id == card_id)
                    .ok_or_else(|| anyhow::anyhow!("activity card is absent or expired"))?;
                ensure!(
                    card.version == version,
                    "activity card changed; reload before removing"
                );
                db.state.activity.retain(|c| c.card_id != card_id);
                db.save()?;
                Ok(json!({"removed":true}))
            }
        }
    }
}

fn operator(access: &AccessSession) -> bool {
    access.has_store_permissions(&[AccessPermission::ManageScope, AccessPermission::Write])
}
fn require_operator(access: &AccessSession) -> Result<()> {
    ensure!(
        operator(access),
        "Context review and policy changes require the local Store operator"
    );
    Ok(())
}
fn permission(access: &AccessSession, scope: &str, permission: AccessPermission) -> Result<()> {
    ensure!(
        access.allows(scope, permission),
        "Scope access is not authorized"
    );
    Ok(())
}
fn text_limit(name: &str, value: &str, max: usize) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.chars().count() <= max,
        "{name} must contain 1..{max} characters"
    );
    Ok(())
}
fn digest(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn timestamp(time: chrono::DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Millis, true)
}
