//! Same-project fusion owns its review, publication recovery and rollback receipt.
use anyhow::{Context, Result, ensure};
use pcp_client::PcpApi;
use pcp_core::{
    ArchivePageRequest, ConsolidatePagesRequest, LifecycleStatus, PageRevisionRef, Projection,
    ReadPagesRequest, ReconciliationDisposition,
};
use pcp_store::DurablePageInventoryItem;

use super::{
    MaintenanceCycleReport, MaintenanceReconciliationCandidate, MaintenanceReviewPayload,
    MaintenanceReviewStatus, RuntimeMaintainer, VerificationVerdict, ledger::MaintenanceLedger,
};

pub(super) fn automatic_project_fusion(candidate: &MaintenanceReconciliationCandidate) -> bool {
    candidate.signal.is_none()
        && candidate.evidence.len() == 1
        && candidate.disposition == ReconciliationDisposition::NoSourceChange
        && candidate.replacement.is_none()
        && candidate.target.namespace.starts_with("project:")
        && candidate
            .evidence
            .iter()
            .all(|p| p.namespace == candidate.target.namespace)
        && candidate
            .suggested_consolidation
            .as_ref()
            .is_some_and(|plan| {
                super::update_discovery::validate_consolidation_suggestion(
                    &plan.outputs,
                    &plan.coverage,
                )
                .is_ok()
                    && plan.coverage.iter().all(|c| c.complete)
            })
}

impl RuntimeMaintainer {
    pub(super) async fn resume_consolidation_review(
        &mut self,
        inventory: &[DurablePageInventoryItem],
        report: &mut MaintenanceCycleReport,
    ) -> Result<bool> {
        let Some(item) = self.ledger.review_items().into_iter().filter(|item| {
            matches!(&item.payload, MaintenanceReviewPayload::Reconciliation(c) if automatic_project_fusion(c))
                && self.ledger.eligible(&format!("review_resume:{}", item.candidate_id))
                && matches!(super::review::review_queue(item, &self.config, inventory).state.as_str(), "automatic" | "waiting_budget")
        }).min_by(|a,b| a.proposed_at.cmp(&b.proposed_at)) else { return Ok(false); };
        let MaintenanceReviewPayload::Reconciliation(mut candidate) = item.payload.clone() else {
            unreachable!()
        };
        let plan = candidate
            .suggested_consolidation
            .clone()
            .context("missing fusion plan")?;
        let sources = std::iter::once(&candidate.target)
            .chain(candidate.evidence.iter())
            .map(|p| PageRevisionRef {
                page_id: p.page_id.clone(),
                revision_id: p.revision_id.clone(),
            })
            .collect::<Vec<_>>();
        let before = report.worker_calls;
        let assessment = if candidate.verification.as_ref().is_some_and(|v| {
            matches!(v.verdict, VerificationVerdict::Approve)
                && v.revision.is_none()
                && !v.requires_user_input
        }) {
            candidate.verification.clone().unwrap()
        } else {
            self.verify_maintenance_candidate("consolidation", sources.iter().map(|p| p.revision_id.clone()).collect(), "Same-project complete fusion".into(),
                serde_json::to_string(&serde_json::json!({"sources":sources,"outputs":plan.outputs,"coverage":plan.coverage}))?,
                Vec::new(), None, candidate.verification.clone(), report).await?
        };
        let no_change = matches!(assessment.verdict, VerificationVerdict::NoChange);
        let approved = matches!(assessment.verdict, VerificationVerdict::Approve)
            && assessment.revision.is_none()
            && !assessment.requires_user_input;
        let apply = approved
            && self.config.applies_changes()
            && self.config.reconciliation.auto_apply_project_consolidation;
        candidate.verification = Some(assessment);
        if let Some(v) = &mut candidate.verification {
            // A modified JSON plan is never covered by the original approval.
            if v.revision.is_some() {
                v.verdict = VerificationVerdict::NeedsReview;
                v.review_state = Some("needs_review".into());
            }
            if apply {
                v.review_state = Some("publishing".into());
            }
        }
        if !apply
            && !no_change
            && report.worker_calls == before
            && serde_json::to_value(MaintenanceReviewPayload::Reconciliation(candidate.clone()))?
                == serde_json::to_value(&item.payload)?
        {
            self.ledger
                .defer_cached_review(format!("review_resume:{}", item.candidate_id));
            self.ledger.save(&self.config.state_path).await?;
            return Ok(false);
        }
        // No model call while holding the ledger decision lock. Recheck the
        // exact proposal after inference before granting publication authority.
        let _decision = MaintenanceLedger::decision_lock(&self.config.state_path).await?;
        let mut fresh = MaintenanceLedger::load(&self.config.state_path).await?;
        if fresh.review_item(&item.candidate_id).is_none_or(|now| {
            now.status != MaintenanceReviewStatus::Pending || now.updated_at != item.updated_at
        }) {
            self.ledger = fresh;
            return Ok(true);
        }
        let reason = candidate.verification.as_ref().unwrap().reason.clone();
        fresh.replace_pending_review(
            &item,
            MaintenanceReviewPayload::Reconciliation(candidate.clone()),
            reason,
            report.worker_calls - before,
        )?;
        fresh.record(
            format!("review_resume:{}", item.candidate_id),
            "review_attempted",
            300,
        );
        if no_change {
            fresh.resolve_review(&item.candidate_id, MaintenanceReviewStatus::Rejected)?;
        }
        // Persist an exact, replayable publication intent before the Store call.
        fresh.save(&self.config.state_path).await?;
        self.ledger = fresh;
        if !apply {
            return Ok(true);
        }
        // Native Store publication checks every exact source head atomically;
        // identical replay returns the original receipt without a second write.
        let result = self
            .client
            .consolidate_pages(ConsolidatePagesRequest {
                require_uncontested_sources: true,
                namespace: candidate.target.namespace.clone(),
                source_pages: sources,
                outputs: plan.outputs,
                coverage: plan.coverage,
                created_by: self.config.worker_actor(),
                idempotency_key: format!("maintenance:fusion:{}", item.candidate_id),
            })
            .await;
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                // These are definite pre-commit rejections. Unknown transport
                // outcomes retain the exact publishing intent for replay.
                let message = error.to_string();
                if message.contains("source is contested")
                    || message.contains("source changed")
                    || message.contains("already covers these source Revisions")
                {
                    if let Some(v) = &mut candidate.verification {
                        v.verdict = VerificationVerdict::NeedsReview;
                        v.review_state = Some("needs_review".into());
                        v.reason = message.clone();
                    }
                    let pending = self
                        .ledger
                        .review_item(&item.candidate_id)
                        .context("missing publication intent")?;
                    self.ledger.replace_pending_review(
                        &pending,
                        MaintenanceReviewPayload::Reconciliation(candidate),
                        message,
                        0,
                    )?;
                    self.ledger.save(&self.config.state_path).await?;
                    report.deferred += 1;
                    return Ok(true);
                }
                return Err(error);
            }
        };
        candidate.consolidation_result = Some(result.clone());
        if let Some(v) = &mut candidate.verification {
            v.review_state = Some("published".into());
        }
        let pending = self
            .ledger
            .review_item(&item.candidate_id)
            .context("missing publication intent")?;
        self.ledger.replace_pending_review(&pending, MaintenanceReviewPayload::Reconciliation(candidate), "Independent complete-coverage review passed; exact publication receipt retained for rollback.".into(), 0)?;
        self.ledger
            .resolve_review(&item.candidate_id, MaintenanceReviewStatus::Accepted)?;
        self.ledger.save(&self.config.state_path).await?;
        report.consolidations_written += u32::from(result.created);
        Ok(true)
    }

    /// Explicit operator action. Archive only unchanged generated outputs; the
    /// Store's coverage contract restores original sources to default recall.
    pub async fn rollback_consolidation_review(
        &mut self,
        candidate_id: &str,
        repair: &dyn PcpApi,
    ) -> Result<Vec<PageRevisionRef>> {
        let _decision = MaintenanceLedger::decision_lock(&self.config.state_path).await?;
        self.ledger = MaintenanceLedger::load(&self.config.state_path).await?;
        let item = self
            .ledger
            .review_item(candidate_id)
            .context("unknown fusion review")?;
        let MaintenanceReviewPayload::Reconciliation(candidate) = item.payload else {
            anyhow::bail!("not a fusion review");
        };
        let result = candidate
            .consolidation_result
            .context("fusion has no publication receipt")?;
        let pages = repair
            .read_pages(ReadPagesRequest {
                page_ids: result.outputs.iter().map(|p| p.page_id.clone()).collect(),
                revision_ids: vec![],
                projections: vec![Projection::Manifest],
                max_chars: 1024,
            })
            .await?;
        ensure!(
            pages.len() == result.outputs.len()
                && pages
                    .iter()
                    .all(|p| p.page.lifecycle_status == LifecycleStatus::Archived
                        || result.outputs.iter().any(|o| o.page_id == p.page.page_id
                            && o.revision_id == p.page.head_revision_id)),
            "fusion output changed; review rollback explicitly"
        );
        for output in &result.outputs {
            if pages.iter().any(|p| {
                p.page.page_id == output.page_id
                    && p.page.lifecycle_status == LifecycleStatus::Archived
            }) {
                continue;
            }
            repair.archive_page(ArchivePageRequest { page_id: output.page_id.clone(), expected_revision_id: output.revision_id.clone(), reason: Some("Operator rollback of reviewed automatic fusion; preserve original sources.".into()) }).await?;
        }
        self.ledger.set_review_reason(candidate_id, "Automatic fusion rolled back by the operator; original sources retained and restored to default recall.".into())?;
        self.ledger.save(&self.config.state_path).await?;
        Ok(result.outputs)
    }
}
