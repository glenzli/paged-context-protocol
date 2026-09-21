//! Preserve experience evidence through staging and formal review. This module
//! performs no source dereferencing, task evaluation, or automatic promotion.
use super::persistence::Candidate;
use anyhow::Result;
use pcp_client::context_hub::CandidateInput;
use serde_json::{Value, json};

pub(super) fn normalize(
    input: &mut CandidateInput,
    experience: &pcp_client::experience::Experience,
) -> Result<()> {
    experience.validate()?;
    for receipt in &experience.receipts {
        if !input
            .source_refs
            .iter()
            .any(|s| serde_json::to_value(s).ok() == serde_json::to_value(&receipt.source).ok())
        {
            input.source_refs.push(receipt.source.clone());
        }
    }
    Ok(())
}

pub(super) fn same_topic(a: &Candidate, b: &Candidate) -> bool {
    a.client_id == b.client_id
        && a.input.scope == b.input.scope
        && a.experience
            .as_ref()
            .and_then(|e| e.topic_key.as_ref())
            .is_some_and(|key| {
                b.experience.as_ref().and_then(|e| e.topic_key.as_ref()) == Some(key)
            })
}

pub(super) fn evidence_chars(candidate: &Candidate) -> usize {
    candidate.input.content.chars().count()
        + candidate.experience.as_ref().map_or(0, |e| {
            serde_json::to_string(e).map_or(usize::MAX / 2, |s| s.chars().count())
        })
}

/// Store original experience snapshots for manual and automatic promotions alike.
/// Terminal inbox cleanup must not erase observations supporting a memory.
pub(super) fn preserve(facets: &mut Value, items: &[&Candidate]) {
    let evidence = items
        .iter()
        .filter_map(|c| {
            c.experience.as_ref().map(|experience| {
                json!({
                    "candidateId": c.candidate_id, "version": c.version,
                    "submittedBy": c.client_id, "submittedAt": c.created_at,
                    "experience": experience, "sourceRefs": c.input.source_refs,
                    "basedOnRevisionIds": c.input.based_on_revision_ids
                })
            })
        })
        .collect::<Vec<_>>();
    if !evidence.is_empty() {
        facets["experienceEvidence"] =
            json!({"schemaVersion":1,"attribution":"producer_report","items":evidence});
    }
}
