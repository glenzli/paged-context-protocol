//! Topic evidence identity and bounded semantic-neighbor retrieval.
//! Similarity only routes review inputs; it never authorizes a write.
use super::MaintenanceTopicCandidate;
use pcp_store::DurablePageInventoryItem;
use std::collections::BTreeSet;

pub(super) fn same_evidence(a: &MaintenanceTopicCandidate, b: &MaintenanceTopicCandidate) -> bool {
    a.namespace == b.namespace
        && a.pages
            .iter()
            .map(|p| (&p.page_id, &p.revision_id))
            .collect::<BTreeSet<_>>()
            == b.pages
                .iter()
                .map(|p| (&p.page_id, &p.revision_id))
                .collect::<BTreeSet<_>>()
        && a.refresh_target
            .as_ref()
            .map(|p| (&p.page_id, &p.revision_id))
            == b.refresh_target
                .as_ref()
                .map(|p| (&p.page_id, &p.revision_id))
}

pub(super) fn subject_affinity(
    a: &DurablePageInventoryItem,
    b: &DurablePageInventoryItem,
) -> usize {
    let left = super::discovery::subject_terms(a);
    let right = super::discovery::subject_terms(b);
    let shared = left.intersection(&right).count();
    // Avoid offering every "AI" or "PCP" Page as the same subject.
    if shared >= 4 && shared * 3 >= left.len().min(right.len()).max(1) {
        shared
    } else {
        0
    }
}
