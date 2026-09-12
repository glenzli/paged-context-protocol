//! Operator correction of an exact automatic output without deleting history.
use super::{ContextHub, persistence::LockedState, timestamp};
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use pcp_client::{EmbeddedPcpClient, PcpApi, PcpTenantApi};
use pcp_core::{
    AccessSession, Actor, ActorType, ArchivePageRequest, LifecycleStatus, Projection,
    ProvenanceEvent, ReadPagesRequest, RevisePageRequest,
};
use serde_json::{Value, json};

impl ContextHub {
    pub(super) async fn undo_automatic_output(
        &self,
        access: &AccessSession,
        db: &mut LockedState,
        synthesis_id: &str,
        version: u64,
        output_index: usize,
    ) -> Result<Value> {
        let index = db
            .state
            .syntheses
            .iter()
            .position(|s| s.synthesis_id == synthesis_id)
            .context("Automatic synthesis unavailable")?;
        let group = db.state.syntheses[index].clone();
        let review = group
            .automatic_review
            .as_ref()
            .context("This output was not automatically reviewed")?;
        ensure!(
            review.authorized && group.status == "promoted" && group.version == version,
            "Automatic result changed; reload before withdrawing"
        );
        if let Some(result) = review.undo_results.iter().find(|r| {
            r["outputIndex"].as_u64() == Some(output_index as u64) && r["status"] != "withdrawing"
        }) {
            return Ok(result.clone());
        }
        let requested_at = review
            .undo_results
            .iter()
            .find(|r| r["outputIndex"].as_u64() == Some(output_index as u64))
            .and_then(|r| r["requestedAt"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| timestamp(Utc::now()));
        let result = group
            .results
            .get(output_index)
            .context("Unknown automatic output")?;
        let output = group
            .review_request
            .as_ref()
            .and_then(|r| r.outputs.get(output_index))
            .context("Original output plan unavailable")?;
        let page_id = result["pageId"]
            .as_str()
            .context("Written Page identity missing")?;
        let revision_id = result["revisionId"]
            .as_str()
            .context("Written Revision identity missing")?;
        ensure!(
            matches!(output.action.as_str(), "create" | "update"),
            "Represented outputs made no memory change to withdraw"
        );
        let undo_results = &mut db.state.syntheses[index]
            .automatic_review
            .as_mut()
            .unwrap()
            .undo_results;
        if !undo_results
            .iter()
            .any(|r| r["outputIndex"].as_u64() == Some(output_index as u64))
        {
            undo_results.push(json!({"outputIndex":output_index,"status":"withdrawing","requestedAt":requested_at}));
            db.save()?;
        }
        let client = EmbeddedPcpClient::new(self.store.clone(), access.clone());
        let mut undo = match output.action.as_str() {
            "create" => {
                let pages = client
                    .read_pages(ReadPagesRequest {
                        page_ids: vec![page_id.into()],
                        revision_ids: vec![],
                        projections: vec![Projection::Manifest],
                        max_chars: 1,
                    })
                    .await?;
                let page = pages.first().context("Written Page unavailable")?;
                ensure!(
                    page.page.head_revision_id == revision_id
                        && page.revision.namespace == group.scope,
                    "Written Page changed; refusing to withdraw a later edit"
                );
                if page.page.lifecycle_status != LifecycleStatus::Archived {
                    client.archive_page(ArchivePageRequest { page_id: page_id.into(), expected_revision_id: revision_id.into(), reason: Some("Operator withdrew automatic candidate memory; content retained for audit and restoration".into()) }).await?;
                }
                json!({"outputIndex":output_index,"status":"archived","pageId":page_id,"revisionId":revision_id})
            }
            "update" => {
                let old_id = output
                    .target_revision_id
                    .as_ref()
                    .context("Original Revision missing")?;
                let pages = client
                    .read_pages(ReadPagesRequest {
                        page_ids: vec![],
                        revision_ids: vec![old_id.clone(), revision_id.into()],
                        projections: vec![
                            Projection::Manifest,
                            Projection::Payload,
                            Projection::Sources,
                            Projection::Facets,
                            Projection::Provenance,
                        ],
                        max_chars: 128000,
                    })
                    .await?;
                let old = &pages
                    .iter()
                    .find(|p| p.revision.revision_id == *old_id)
                    .context("Original Revision unavailable")?
                    .revision;
                let written = pages
                    .iter()
                    .find(|p| p.revision.revision_id == revision_id)
                    .context("Written Revision unavailable")?;
                ensure!(
                    old.namespace == group.scope && written.revision.namespace == group.scope,
                    "Undo target is outside candidate Scope"
                );
                let expected_content = review
                    .input
                    .as_ref()
                    .and_then(|i| i.pages.iter().find(|p| p.revision_id == *old_id))
                    .and_then(|p| p.content.as_ref())
                    .context("Complete original comparison missing")?;
                ensure!(
                    old.payload
                        .as_ref()
                        .is_some_and(|p| &p.content == expected_content),
                    "Original content is incomplete; refusing partial restoration"
                );
                let actor = Actor {
                    actor_type: ActorType::User,
                    actor_id: access.principal.principal_id.clone(),
                };
                let mut provenance = written.revision.provenance.clone();
                provenance.push(ProvenanceEvent {
                    operation: "undo_automatic_candidate_update".into(),
                    actor: actor.clone(),
                    timestamp: requested_at.clone(),
                    input_revision_ids: vec![old_id.clone(), revision_id.into()],
                    tool_or_model: None,
                    reason: Some(
                        "Operator restored the exact content preceding automatic review".into(),
                    ),
                });
                let restored = client
                    .revise_page(RevisePageRequest {
                        page_id: page_id.into(),
                        expected_revision_id: revision_id.into(),
                        created_by: actor,
                        lifecycle_status: LifecycleStatus::Active,
                        observed_at: old.observed_at.clone(),
                        valid_from: old.valid_from.clone(),
                        valid_to: old.valid_to.clone(),
                        payload: old.payload.clone(),
                        source_refs: old.source_refs.clone(),
                        facets: old.facets.clone(),
                        provenance,
                        initial_relations: vec![],
                        idempotency_key: Some(format!(
                            "pcp-auto-undo:{synthesis_id}:{output_index}:{revision_id}"
                        )),
                    })
                    .await?;
                json!({"outputIndex":output_index,"status":"restored_previous","pageId":page_id,"revisionId":restored.revision_id})
            }
            _ => anyhow::bail!("Represented outputs made no memory change to withdraw"),
        };
        undo["requestedAt"] = json!(requested_at);
        let receipts = &mut db.state.syntheses[index]
            .automatic_review
            .as_mut()
            .unwrap()
            .undo_results;
        receipts.retain(|r| r["outputIndex"].as_u64() != Some(output_index as u64));
        receipts.push(undo.clone());
        db.save()?;
        Ok(undo)
    }
}
