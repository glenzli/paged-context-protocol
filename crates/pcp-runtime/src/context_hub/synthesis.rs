//! Bounded candidate evolution. Drafts and scheduling state share the inbox's lock;
//! inference runs outside it, and exact input versions guard the returning result.
use super::persistence::{Candidate, HubState, LockedState};
use super::{ContextHub, digest, permission, text_limit, timestamp};
use anyhow::{Result, ensure};
use chrono::{Duration, Utc};
use pcp_client::{
    PcpApi,
    context_hub::{CandidateVersion, SynthesisOutput, SynthesisReview},
};
use pcp_core::{AccessPermission, Projection, ReadPagesRequest};
use pcp_store::DurablePageInventoryItem;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationState {
    #[serde(default)]
    /// None preserves the pre-automation operator-only behavior on upgrade.
    pub automatic_review_enabled: Option<bool>,
    #[serde(default)]
    pub automatic_review_available: bool,
    pub available: bool,
    pub queued: bool,
    pub last_attempt_at: Option<String>,
    pub next_attempt_at: Option<String>,
    pub last_completed_at: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub retry_when_changed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Synthesis {
    pub synthesis_id: String,
    pub version: u64,
    pub scope: String,
    pub candidates: Vec<CandidateVersion>,
    pub title: String,
    pub narrative: String,
    pub reason: String,
    pub unresolved: Vec<String>,
    pub maturity: String,
    pub outputs: Vec<SynthesisOutput>,
    pub offered_revision_ids: Vec<String>,
    #[serde(default)]
    pub updateable_revision_ids: Vec<String>,
    #[serde(default)]
    pub compared_pages: Vec<crate::maintenance::MaintenanceDetailPage>,
    #[serde(default)]
    pub context_id: Option<String>,
    pub status: String,
    pub created_at: String,
    pub review_request: Option<SynthesisReview>,
    pub results: Vec<Value>,
    #[serde(default)]
    pub automatic_review: Option<super::automatic_review::AutomaticReview>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedSynthesis {
    pub candidate_ids: Vec<String>,
    pub title: String,
    /// Source-grounded chronology, including corrections and remaining disagreement.
    pub narrative: String,
    pub reason: String,
    pub unresolved: Vec<String>,
    /// accumulating or ready; readiness is a proposal, never write authority.
    pub maturity: String,
    pub outputs: Vec<SynthesisOutput>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationInput {
    pub scope: String,
    pub candidates: Vec<Candidate>,
    pub previous: Vec<ProposedSynthesis>,
    pub pages: Vec<crate::maintenance::MaintenanceDetailPage>,
    pub updateable_revision_ids: Vec<String>,
}

pub fn active(candidate: &Candidate) -> bool {
    matches!(candidate.status.as_str(), "pending" | "deferred")
}

pub fn invalidate_stale(state: &mut HubState) {
    for synthesis in &mut state.syntheses {
        if synthesis.status == "pending"
            && synthesis.candidates.iter().any(|r| {
                !state.candidates.iter().any(|c| {
                    c.candidate_id == r.candidate_id && c.version == r.version && active(c)
                })
            })
        {
            synthesis.status = "superseded".into();
        }
    }
}

pub(super) fn grams(text: &str) -> HashSet<String> {
    let chars = text
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<Vec<_>>();
    chars.windows(2).map(|pair| pair.iter().collect()).collect()
}
pub(super) fn similarity(a: &HashSet<String>, b: &str) -> usize {
    a.intersection(&grams(b)).count()
}

impl ContextHub {
    pub(crate) fn enable_organization(&self) {
        self.organization_available
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) async fn prepare_organization(
        &self,
        client: &dyn PcpApi,
        inventory: &[DurablePageInventoryItem],
    ) -> Result<Option<OrganizationInput>> {
        let now = timestamp(Utc::now());
        let mut db = LockedState::open(&self.path, self.store.identity_id()).await?;
        if db
            .state
            .organization
            .next_attempt_at
            .as_deref()
            .is_some_and(|next| next > now.as_str())
        {
            return Ok(None);
        }
        let eligible = |c: &&Candidate| {
            active(c)
                && c.snoozed_until
                    .as_deref()
                    .is_none_or(|until| until <= now.as_str())
                && permission(
                    client.access(),
                    &c.input.scope,
                    AccessPermission::ReadDetail,
                )
                .is_ok()
        };
        let changed_context = db
            .state
            .syntheses
            .iter()
            .filter(|s| s.status == "pending")
            .filter(|s| {
                s.context_id
                    .as_ref()
                    .and_then(|id| db.state.synthesis_contexts.get(id))
                    .is_some_and(|pages| {
                        pages.iter().any(|page| {
                            !inventory.iter().any(|current| {
                                current.page_id == page.page_id
                                    && current.revision_id == page.revision_id
                            })
                        })
                    })
            })
            .flat_map(|s| s.candidates.iter().map(|c| c.candidate_id.as_str()))
            .collect::<BTreeSet<_>>();
        let Some(seed) = db
            .state
            .candidates
            .iter()
            .filter(eligible)
            .filter(|c| {
                c.organized_version != c.version
                    || changed_context.contains(c.candidate_id.as_str())
            })
            .min_by_key(|c| &c.created_at)
            .cloned()
        else {
            return Ok(None);
        };
        let scope = seed.input.scope.clone();
        if !db.state.organization.queued
            && seed.organized_version != seed.version
            && seed.created_at > timestamp(Utc::now() - Duration::minutes(10))
        {
            let cutoff = timestamp(Utc::now() - Duration::seconds(120));
            if db
                .state
                .candidates
                .iter()
                .filter(|c| active(c) && c.input.scope == scope)
                .any(|c| c.created_at > cutoff)
            {
                return Ok(None);
            }
        }
        let seed_text = format!("{} {}", seed.input.title, seed.input.content);
        let terms = grams(&seed_text);
        let mut peers = db
            .state
            .candidates
            .iter()
            .filter(|c| {
                active(c)
                    && permission(
                        client.access(),
                        &c.input.scope,
                        AccessPermission::ReadDetail,
                    )
                    .is_ok()
            })
            .filter(|c| c.input.scope == scope && c.candidate_id != seed.candidate_id)
            .cloned()
            .collect::<Vec<_>>();
        peers.sort_by_key(|c| {
            (
                std::cmp::Reverse(similarity(
                    &terms,
                    &format!("{} {}", c.input.title, c.input.content),
                )),
                c.organized_version == c.version,
                c.created_at.clone(),
            )
        });
        let mut candidates = vec![seed];
        let mut evidence_chars = candidates[0].input.content.chars().count();
        // Prefer fresh evidence when relevance ties, but retain related older evidence
        // so a new event can extend a previous interpretation.
        for peer in peers.into_iter().take(19) {
            let chars = peer.input.content.chars().count();
            if evidence_chars + chars > 16000 {
                break;
            }
            evidence_chars += chars;
            candidates.push(peer);
        }
        candidates.sort_by_key(|c| c.created_at.clone());
        let ids = candidates
            .iter()
            .map(|c| c.candidate_id.clone())
            .collect::<BTreeSet<_>>();
        let previous = db
            .state
            .syntheses
            .iter()
            .filter(|s| {
                s.status == "pending" && s.candidates.iter().any(|c| ids.contains(&c.candidate_id))
            })
            .take(4)
            .map(|s| ProposedSynthesis {
                candidate_ids: s
                    .candidates
                    .iter()
                    .map(|c| c.candidate_id.clone())
                    .collect(),
                title: s.title.clone(),
                narrative: s.narrative.chars().take(2000).collect(),
                reason: s.reason.chars().take(800).collect(),
                unresolved: s
                    .unresolved
                    .iter()
                    .take(4)
                    .map(|q| q.chars().take(400).collect())
                    .collect(),
                maturity: s.maturity.clone(),
                outputs: s
                    .outputs
                    .iter()
                    .map(|o| {
                        let mut prior = o.clone();
                        prior.content = prior.content.chars().take(1000).collect();
                        prior
                    })
                    .collect(),
            })
            .collect();
        // A durable lease prevents concurrent runs and restart storms. Success releases it;
        // failure leaves an explicit cooldown and retains all originals.
        db.state.organization.queued = false;
        db.state.organization.last_attempt_at = Some(now);
        db.state.organization.next_attempt_at = Some(timestamp(Utc::now() + Duration::minutes(30)));
        db.state.organization.error = None;
        db.state.organization.retry_when_changed = false;
        db.save()?;
        drop(db);
        let all_terms = grams(
            &candidates
                .iter()
                .map(|c| format!("{} {}", c.input.title, c.input.content))
                .collect::<Vec<_>>()
                .join(" "),
        );
        let basis = candidates
            .iter()
            .flat_map(|c| c.input.based_on_revision_ids.iter())
            .collect::<BTreeSet<_>>();
        let mut related = inventory
            .iter()
            .filter(|p| p.namespace == scope)
            .collect::<Vec<_>>();
        related.sort_by_key(|p| {
            (
                std::cmp::Reverse(
                    (basis.contains(&p.revision_id) as usize * 100_000)
                        + similarity(
                            &all_terms,
                            &format!("{} {}", p.summary.as_deref().unwrap_or_default(), p.snippet),
                        ),
                ),
                p.page_id.clone(),
            )
        });
        related.retain(|p| {
            basis.contains(&p.revision_id)
                || similarity(
                    &all_terms,
                    &format!("{} {}", p.summary.as_deref().unwrap_or_default(), p.snippet),
                ) >= 4
        });
        let revisions = related
            .into_iter()
            .take(6)
            .map(|p| p.revision_id.clone())
            .collect::<Vec<_>>();
        let revisions_for_updates = revisions.clone();
        let pages = if revisions.is_empty() {
            vec![]
        } else {
            client
                .read_pages(ReadPagesRequest {
                    page_ids: vec![],
                    revision_ids: revisions,
                    projections: vec![
                        Projection::Manifest,
                        Projection::Payload,
                        Projection::Summary,
                        Projection::Validity,
                        Projection::Sources,
                        Projection::Facets,
                    ],
                    max_chars: 12000,
                })
                .await?
                .into_iter()
                .filter(|p| {
                    p.page.head_revision_id == p.revision.revision_id
                        && p.page.lifecycle_status == pcp_core::LifecycleStatus::Active
                })
                .map(Into::into)
                .collect()
        };
        let updateable_revision_ids = inventory
            .iter()
            .filter(|p| {
                p.namespace == scope
                    && p.mutability == pcp_core::PageMutability::Revisioned
                    && p.media_type.as_deref() != Some(pcp_core::PACKED_PAGE_MEDIA_TYPE)
            })
            .map(|p| p.revision_id.clone())
            .filter(|id| {
                revisions_for_updates.contains(id)
                    && pages
                        .iter()
                        .any(|page: &crate::maintenance::MaintenanceDetailPage| {
                            page.revision_id == *id
                                && page.content.as_ref().is_some_and(|content| {
                                    inventory.iter().any(|item| {
                                        item.revision_id == *id
                                            && item.content_chars == content.chars().count() as u64
                                    })
                                })
                        })
            })
            .collect();
        Ok(Some(OrganizationInput {
            scope,
            candidates,
            previous,
            pages,
            updateable_revision_ids,
        }))
    }

    pub(crate) async fn finish_organization(
        &self,
        input: &OrganizationInput,
        proposals: Vec<ProposedSynthesis>,
    ) -> Result<()> {
        validate_proposals(input, &proposals)?;
        let mut db = LockedState::open(&self.path, self.store.identity_id()).await?;
        ensure!(
            input.candidates.iter().all(|given| db
                .state
                .candidates
                .iter()
                .any(|c| c.candidate_id == given.candidate_id
                    && c.version == given.version
                    && active(c))),
            "Candidate evidence changed during organization; result discarded"
        );
        let ids = input
            .candidates
            .iter()
            .map(|c| c.candidate_id.as_str())
            .collect::<BTreeSet<_>>();
        for old in &mut db.state.syntheses {
            if old.status == "pending"
                && old
                    .candidates
                    .iter()
                    .any(|c| ids.contains(c.candidate_id.as_str()))
            {
                old.status = "superseded".into();
            }
        }
        let now = timestamp(Utc::now());
        let context_id = digest(&input.pages)?;
        db.state
            .synthesis_contexts
            .insert(context_id.clone(), input.pages.clone());
        for proposal in proposals {
            let candidates = input
                .candidates
                .iter()
                .filter(|c| proposal.candidate_ids.contains(&c.candidate_id))
                .map(|c| CandidateVersion {
                    candidate_id: c.candidate_id.clone(),
                    version: c.version,
                })
                .collect::<Vec<_>>();
            let synthesis_id = format!("syn_{}", digest(&(now.as_str(), &candidates, &proposal))?);
            db.state.syntheses.push(Synthesis {
                synthesis_id,
                version: 1,
                scope: input.scope.clone(),
                candidates,
                title: proposal.title,
                narrative: proposal.narrative,
                reason: proposal.reason,
                unresolved: proposal.unresolved,
                maturity: proposal.maturity,
                outputs: proposal.outputs,
                offered_revision_ids: input.pages.iter().map(|p| p.revision_id.clone()).collect(),
                updateable_revision_ids: input.updateable_revision_ids.clone(),
                compared_pages: vec![],
                context_id: Some(context_id.clone()),
                status: "pending".into(),
                created_at: now.clone(),
                review_request: None,
                results: vec![],
                automatic_review: None,
            });
        }
        for c in &mut db.state.candidates {
            if ids.contains(c.candidate_id.as_str()) {
                c.organized_version = c.version;
            }
        }
        // Keep one bounded previous generation for inspection, never drop pending or in-flight work.
        let mut old_count = 0;
        db.state.syntheses.reverse();
        db.state.syntheses.retain(|s| {
            if s.status == "superseded" {
                old_count += 1;
                old_count <= 100
            } else {
                true
            }
        });
        db.state.syntheses.reverse();
        let referenced = db
            .state
            .syntheses
            .iter()
            .filter_map(|s| s.context_id.clone())
            .collect::<BTreeSet<_>>();
        db.state
            .synthesis_contexts
            .retain(|id, _| referenced.contains(id));
        db.state.organization.last_completed_at = Some(now);
        db.state.organization.next_attempt_at = None;
        db.state.organization.error = None;
        db.state.organization.retry_when_changed = false;
        db.save()
    }

    pub(crate) async fn organization_rejected(
        &self,
        input: &OrganizationInput,
        error: &str,
    ) -> Result<()> {
        let mut db = LockedState::open(&self.path, self.store.identity_id()).await?;
        for candidate in &mut db.state.candidates {
            if input.candidates.iter().any(|given| {
                given.candidate_id == candidate.candidate_id && given.version == candidate.version
            }) {
                candidate.organized_version = candidate.version;
            }
        }
        db.state.organization.error = Some(error.chars().take(600).collect());
        db.state.organization.retry_when_changed = true;
        db.state.organization.next_attempt_at = None;
        db.save()
    }

    pub(crate) async fn organization_failed(&self, error: &str) -> Result<()> {
        let mut db = LockedState::open(&self.path, self.store.identity_id()).await?;
        db.state.organization.error = Some(error.chars().take(600).collect());
        db.state.organization.retry_when_changed = false;
        db.state.organization.next_attempt_at = Some(timestamp(Utc::now() + Duration::minutes(30)));
        db.save()
    }
}

fn validate_proposals(input: &OrganizationInput, proposals: &[ProposedSynthesis]) -> Result<()> {
    ensure!(
        !proposals.is_empty() && proposals.len() <= input.candidates.len(),
        "Organization must account for every candidate in bounded groups"
    );
    ensure!(
        serde_json::to_vec(proposals)?.len() <= 128 * 1024,
        "Organization draft exceeds storage budget"
    );
    let offered = input
        .candidates
        .iter()
        .map(|c| c.candidate_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut covered = BTreeSet::new();
    for p in proposals {
        text_limit("synthesis title", &p.title, 160)?;
        text_limit("evolution", &p.narrative, 8000)?;
        text_limit("maturity reason", &p.reason, 1200)?;
        ensure!(
            matches!(p.maturity.as_str(), "ready" | "accumulating"),
            "Invalid maturity"
        );
        ensure!(
            !p.candidate_ids.is_empty() && p.candidate_ids.len() <= 20,
            "Invalid evidence group"
        );
        for id in &p.candidate_ids {
            ensure!(
                offered.contains(id.as_str()),
                "Unknown candidate in group evidence: {}",
                id
            );
            ensure!(
                covered.insert(id.as_str()),
                "Candidate repeated across groups: {}",
                id
            );
        }
        ensure!(
            p.unresolved.len() <= 8 && p.outputs.len() <= 4,
            "Organization output exceeds budget"
        );
        for question in &p.unresolved {
            text_limit("unresolved question", question, 1000)?;
        }
        if p.maturity == "ready" {
            ensure!(!p.outputs.is_empty(), "Ready group needs memory outputs");
        }
        for output in &p.outputs {
            ensure!(
                output.action != "update"
                    || output
                        .target_revision_id
                        .as_ref()
                        .is_some_and(|id| input.updateable_revision_ids.contains(id)),
                "Update target is immutable; propose a new memory instead"
            );
            validate_output(
                output,
                &p.candidate_ids,
                &input
                    .pages
                    .iter()
                    .map(|p| p.revision_id.clone())
                    .collect::<Vec<_>>(),
            )?;
        }
    }
    ensure!(
        covered == offered,
        "Organization omitted candidate evidence"
    );
    Ok(())
}

pub(super) fn validate_output(
    output: &SynthesisOutput,
    candidates: &[String],
    offered_revisions: &[String],
) -> Result<()> {
    ensure!(
        !output.candidate_ids.is_empty() && output.candidate_ids.len() <= 20,
        "Output needs bounded evidence"
    );
    let mut unique = BTreeSet::new();
    ensure!(
        output
            .candidate_ids
            .iter()
            .all(|id| candidates.contains(id) && unique.insert(id)),
        "Output evidence is unknown or repeated"
    );
    text_limit("memory title", &output.title, 160)?;
    text_limit("memory content", &output.content, 16000)?;
    ensure!(
        matches!(output.action.as_str(), "create" | "update" | "represented"),
        "Invalid memory action"
    );
    if output.action == "create" {
        ensure!(
            output.target_revision_id.is_none(),
            "New memory cannot target a Revision"
        );
    } else {
        ensure!(
            output
                .target_revision_id
                .as_ref()
                .is_some_and(|id| offered_revisions.contains(id)),
            "Target Revision was not offered for this synthesis"
        );
    }
    Ok(())
}

impl OrganizationInput {
    /// Model-local handles avoid asking a model to copy long content hashes.
    /// Canonical identity and authorization remain in the untouched input snapshot.
    pub(crate) fn worker_view(&self) -> Self {
        let mut view = self.clone();
        let candidate_aliases = self
            .candidates
            .iter()
            .enumerate()
            .map(|(i, c)| (c.candidate_id.clone(), format!("c{}", i + 1)))
            .collect::<std::collections::BTreeMap<_, _>>();
        let revision_aliases = self
            .pages
            .iter()
            .enumerate()
            .map(|(i, p)| (p.revision_id.clone(), format!("r{}", i + 1)))
            .collect::<std::collections::BTreeMap<_, _>>();
        for (i, c) in view.candidates.iter_mut().enumerate() {
            c.candidate_id = format!("c{}", i + 1);
            c.input.event_id = format!("submission{}", i + 1);
            c.input.based_on_revision_ids = c
                .input
                .based_on_revision_ids
                .iter()
                .filter_map(|id| revision_aliases.get(id).cloned())
                .collect();
            c.result = None;
            c.review_key = None;
            c.promotion_request = None;
        }
        for (i, page) in view.pages.iter_mut().enumerate() {
            page.page_id = format!("p{}", i + 1);
            page.revision_id = format!("r{}", i + 1);
        }
        view.updateable_revision_ids = self
            .updateable_revision_ids
            .iter()
            .filter_map(|id| revision_aliases.get(id).cloned())
            .collect();
        for prior in &mut view.previous {
            prior.candidate_ids = prior
                .candidate_ids
                .iter()
                .filter_map(|id| candidate_aliases.get(id).cloned())
                .collect();
            for output in &mut prior.outputs {
                output.candidate_ids = output
                    .candidate_ids
                    .iter()
                    .filter_map(|id| candidate_aliases.get(id).cloned())
                    .collect();
                output.target_revision_id = output
                    .target_revision_id
                    .as_ref()
                    .and_then(|id| revision_aliases.get(id).cloned());
            }
        }
        view
    }

    pub(crate) fn canonical_groups(
        &self,
        mut groups: Vec<ProposedSynthesis>,
    ) -> Result<Vec<ProposedSynthesis>> {
        let candidates = self
            .candidates
            .iter()
            .enumerate()
            .map(|(i, c)| (format!("c{}", i + 1), c.candidate_id.clone()))
            .collect::<std::collections::BTreeMap<_, _>>();
        let revisions = self
            .pages
            .iter()
            .enumerate()
            .map(|(i, p)| (format!("r{}", i + 1), p.revision_id.clone()))
            .collect::<std::collections::BTreeMap<_, _>>();
        for group in &mut groups {
            for id in &mut group.candidate_ids {
                *id = candidates.get(id).cloned().ok_or_else(|| {
                    anyhow::anyhow!(
                        "Unknown candidate handle: {}",
                        id.chars().take(90).collect::<String>()
                    )
                })?;
            }
            for output in &mut group.outputs {
                for id in &mut output.candidate_ids {
                    *id = candidates.get(id).cloned().ok_or_else(|| {
                        anyhow::anyhow!(
                            "Unknown output evidence handle: {}",
                            id.chars().take(90).collect::<String>()
                        )
                    })?;
                }
                if let Some(id) = output.target_revision_id.as_mut() {
                    *id = revisions.get(id).cloned().ok_or_else(|| {
                        anyhow::anyhow!(
                            "Unknown Page Revision handle: {}",
                            id.chars().take(90).collect::<String>()
                        )
                    })?;
                }
            }
        }
        Ok(groups)
    }
}
