//! Opt-in candidate inbox and bounded activity snapshots, separate from Page recall.
mod activity;
pub(crate) mod automatic_review;
mod automatic_review_undo;
mod experience;
mod inspection;
mod persistence;
mod reconcile;
mod review;
pub(crate) mod synthesis;
mod synthesis_review;
#[cfg(test)]
mod tests;

use anyhow::{Result, ensure};
use async_trait::async_trait;
use chrono::{Duration, SecondsFormat, Utc};
use pcp_client::{EmbeddedPcpClient, PcpTenantApi, context_hub::*};
use pcp_core::{AccessPermission, AccessSession, Projection, ReadPagesRequest};
use pcp_store::PcpStore;
use persistence::{Candidate, LockedState};
pub use reconcile::PreparedContextProjectReconciliation;
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
    organization_available: std::sync::atomic::AtomicBool,
    automatic_review_available: std::sync::atomic::AtomicBool,
}

impl ContextHub {
    pub fn new(store: Arc<dyn PcpStore>, path: PathBuf) -> Self {
        Self {
            store,
            path,
            organization_available: std::sync::atomic::AtomicBool::new(false),
            automatic_review_available: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn state_path(store_path: &Path) -> PathBuf {
        store_path.with_extension("context.json")
    }

    /// Inbox polling must not turn activity-only traffic or a settled inbox
    /// into another full Store maintenance cycle.
    pub(crate) async fn automatic_work_due(&self, access: &AccessSession) -> Result<bool> {
        let db = LockedState::open(&self.path, self.store.identity_id()).await?;
        let now = timestamp(Utc::now());
        let settled = timestamp(Utc::now() - Duration::minutes(5));
        let quiet = timestamp(Utc::now() - Duration::seconds(120));
        let can_read =
            |scope: &str| permission(access, scope, AccessPermission::ReadDetail).is_ok();
        let organization_due = db
            .state
            .organization
            .next_attempt_at
            .as_deref()
            .is_none_or(|at| at <= now.as_str());
        if organization_due
            && db.state.candidates.iter().any(|candidate| {
                synthesis::active(candidate)
                    && candidate.organized_version != candidate.version
                    && can_read(&candidate.input.scope)
                    && candidate
                        .snoozed_until
                        .as_deref()
                        .is_none_or(|until| until <= now.as_str())
            })
        {
            return Ok(true);
        }
        Ok(self
            .automatic_review_available
            .load(std::sync::atomic::Ordering::Relaxed)
            && db.state.organization.automatic_review_enabled == Some(true)
            && db.state.syntheses.iter().any(|group| {
                matches!(group.status.as_str(), "pending" | "promoting")
                    && can_read(&group.scope)
                    && permission(access, &group.scope, AccessPermission::Ingest).is_ok()
                    && !group.outputs.is_empty()
                    && (group.automatic_review.as_ref().is_some_and(|review| {
                        review.authorized
                            && matches!(review.state.as_str(), "approved" | "applying")
                    }) || (group.created_at <= settled
                        && group.candidates.iter().all(|source| {
                            db.state.candidates.iter().any(|candidate| {
                                candidate.candidate_id == source.candidate_id
                                    && candidate.version == source.version
                                    && synthesis::active(candidate)
                            })
                        })
                        && !db.state.candidates.iter().any(|candidate| {
                            synthesis::active(candidate)
                                && candidate.input.scope == group.scope
                                && (candidate.created_at > quiet
                                    || (group.candidates.iter().any(|source| {
                                        source.candidate_id == candidate.candidate_id
                                    }) && candidate
                                        .snoozed_until
                                        .as_deref()
                                        .is_some_and(|until| until > now.as_str())))
                        })))
                    && group.automatic_review.as_ref().is_none_or(|review| {
                        (review.authorized
                            && matches!(review.state.as_str(), "approved" | "applying"))
                            || (matches!(review.state.as_str(), "running" | "waiting_budget")
                                && review
                                    .next_attempt_at
                                    .as_deref()
                                    .is_some_and(|at| at <= now.as_str()))
                    })
            }))
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
        mut input: CandidateInput,
        experience: Option<pcp_client::experience::Experience>,
        now: &str,
    ) -> Result<Value> {
        permission(access, &input.scope, AccessPermission::Ingest)?;
        text_limit("eventId", &input.event_id, 160)?;
        text_limit("title", &input.title, 120)?;
        text_limit("content", &input.content, 2000)?;
        if let Some(profile) = &experience {
            experience::normalize(&mut input, profile)?;
        }
        self.validate_basis(access, &input).await?;
        let id = format!(
            "cand_{}",
            digest(&(access.principal.principal_id.as_str(), &input.event_id))?
        );
        if let Some(existing) = db.state.candidates.iter().find(|c| c.candidate_id == id) {
            ensure!(
                serde_json::to_value(&existing.input)? == serde_json::to_value(&input)?
                    && serde_json::to_value(&existing.experience)?
                        == serde_json::to_value(&experience)?,
                "eventId was already used for different candidate content"
            );
            return Ok(
                json!({"candidateId":id, "status":existing.status, "created":false, "version":existing.version, "result":existing.result}),
            );
        }
        ensure!(
            db.state.candidates.len() < 500,
            "candidate inbox capacity reached; review existing candidates, do not retry repeatedly"
        );
        ensure!(
            db.state
                .candidates
                .iter()
                .filter(|c| c.client_id == access.principal.principal_id
                    && (c.status == "promoting"
                        || (matches!(c.status.as_str(), "pending" | "deferred")
                            && c.organized_version != c.version)))
                .count()
                < 50,
            "client candidate quota reached: 50 candidates still await organization or write; organized evidence does not count. Do not retry this submission repeatedly"
        );
        db.state.candidates.push(Candidate {
            candidate_id: id.clone(),
            client_id: access.principal.principal_id.clone(),
            input,
            experience,
            created_at: now.into(),
            expires_at: timestamp(Utc::now() + Duration::days(30)),
            version: 1,
            status: "pending".into(),
            snoozed_until: None,
            review_key: None,
            promotion_request: None,
            result: None,
            organized_version: 0,
        });
        db.save()?;
        Ok(json!({"candidateId":id, "status":"pending", "created":true, "version":1}))
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
                self.submit(access, &mut db, input, None, &now).await
            }
            ContextHubRequest::SubmitExperience(input) => {
                ensure!(
                    policy.submit_candidates,
                    "Candidate submission is disabled for this client; enable it in Console"
                );
                self.submit(
                    access,
                    &mut db,
                    input.candidate,
                    Some(input.experience),
                    &now,
                )
                .await
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
            ContextHubRequest::Inspect => self.inspect(access, &mut db),
            ContextHubRequest::InspectActivity => {
                let result = self.inspect_activity(access, &db)?;
                if pruned {
                    db.save()?;
                }
                Ok(result)
            }
            ContextHubRequest::InspectPolicies => {
                require_operator(access)?;
                if pruned {
                    db.save()?;
                }
                Ok(json!({"policies":db.state.policies}))
            }
            ContextHubRequest::InspectInbox => self.inspect_inbox(access, &mut db),
            ContextHubRequest::InspectCandidate { candidate_id } => {
                let result = self.inspect_candidate(access, &db, &candidate_id)?;
                if pruned {
                    db.save()?;
                }
                Ok(result)
            }
            ContextHubRequest::InspectSynthesis { synthesis_id } => {
                self.inspect_synthesis(access, &mut db, &synthesis_id)
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
            ContextHubRequest::OrganizeCandidates => {
                require_operator(access)?;
                ensure!(
                    self.organization_available
                        .load(std::sync::atomic::Ordering::Relaxed),
                    "Candidate organization requires enabled Runtime maintenance"
                );
                for candidate in &mut db.state.candidates {
                    if matches!(candidate.status.as_str(), "pending" | "deferred") {
                        candidate.organized_version = 0;
                        candidate.snoozed_until = None;
                    }
                }
                db.state.organization.next_attempt_at = None;
                db.state.organization.queued = true;
                db.save()?;
                Ok(json!({"status":"queued"}))
            }
            ContextHubRequest::StopSynthesis(request) => {
                require_operator(access)?;
                self.stop_synthesis(&mut db, request)
            }
            ContextHubRequest::SetAutomaticReview { enabled } => {
                require_operator(access)?;
                db.state.organization.automatic_review_enabled = Some(enabled);
                db.save()?;
                Ok(json!({"enabled":enabled}))
            }
            ContextHubRequest::UndoAutomaticOutput {
                synthesis_id,
                version,
                output_index,
            } => {
                require_operator(access)?;
                self.undo_automatic_output(access, &mut db, &synthesis_id, version, output_index)
                    .await
            }
            ContextHubRequest::ReviewSynthesis(request) => {
                require_operator(access)?;
                self.review_synthesis(access, &mut db, request, false).await
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
