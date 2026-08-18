//! Deterministic ranking policy for lexical PCP search.
//!
//! SQLite FTS ranks each projection independently.  This module fuses those
//! ranked lists and then lets an explicitly asserted Page relation provide a
//! bounded structural signal.  Provenance is intentionally not an input: a
//! derivation dependency does not, by itself, assert semantic relatedness.

use std::collections::{BTreeMap, BTreeSet};

use pcp_core::SearchHit;

/// Keep the lexical score numerically stable while making its scale explicit.
const RECIPROCAL_RANK_SCALE: u64 = 10_000;
const RECIPROCAL_RANK_OFFSET: u64 = 60;
/// One direct asserted relation is meaningful supporting evidence, but it
/// cannot make a non-lexical Page eligible for a text result.
const RELATION_SUPPORT_UNITS: u64 = RECIPROCAL_RANK_SCALE / (RECIPROCAL_RANK_OFFSET + 1) / 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RelationPair {
    pub from_page_id: String,
    pub to_page_id: String,
}

#[derive(Debug)]
struct Candidate {
    hit: SearchHit,
    lexical_units: u64,
    surface_count: usize,
}

/// Fuse independently-ranked text surfaces, then boost candidates directly
/// connected to another lexical candidate by an active asserted Relation.
///
/// Every returned Page matched text on at least one requested projection. The
/// graph never introduces a new result here; it only orders lexical candidates
/// more coherently.  That keeps ordinary search recall predictable while still
/// making PCP's explicitly curated structure matter.
pub(crate) fn rank_text_hits(
    surfaces: impl IntoIterator<Item = Vec<SearchHit>>,
    relation_pairs: &[RelationPair],
) -> Vec<SearchHit> {
    let mut candidates = BTreeMap::<String, Candidate>::new();
    for surface in surfaces {
        let mut seen_in_surface = BTreeSet::new();
        for (index, hit) in surface.into_iter().enumerate() {
            if !seen_in_surface.insert(hit.revision_id.clone()) {
                continue;
            }
            let lexical_units = reciprocal_rank_units(index + 1);
            candidates
                .entry(hit.revision_id.clone())
                .and_modify(|candidate| {
                    candidate.lexical_units += lexical_units;
                    candidate.surface_count += 1;
                })
                .or_insert(Candidate {
                    hit,
                    lexical_units,
                    surface_count: 1,
                });
        }
    }

    let revision_by_page = candidates
        .iter()
        .map(|(revision_id, candidate)| (candidate.hit.page_id.clone(), revision_id.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut relation_neighbors = BTreeMap::<String, BTreeSet<String>>::new();
    for pair in relation_pairs {
        let Some(from_revision_id) = revision_by_page.get(&pair.from_page_id) else {
            continue;
        };
        let Some(to_revision_id) = revision_by_page.get(&pair.to_page_id) else {
            continue;
        };
        if from_revision_id == to_revision_id {
            continue;
        }
        relation_neighbors
            .entry(from_revision_id.clone())
            .or_default()
            .insert(to_revision_id.clone());
        relation_neighbors
            .entry(to_revision_id.clone())
            .or_default()
            .insert(from_revision_id.clone());
    }

    let mut ranked = candidates
        .into_iter()
        .map(|(revision_id, candidate)| {
            let support_count = relation_neighbors
                .get(&revision_id)
                .map_or(0, BTreeSet::len) as u64;
            let total_units =
                candidate.lexical_units + support_count.saturating_mul(RELATION_SUPPORT_UNITS);
            (
                total_units,
                support_count,
                candidate.surface_count,
                candidate.hit,
            )
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| right.1.cmp(&left.1))
            .then_with(|| right.2.cmp(&left.2))
            .then_with(|| hit_recency(&right.3).cmp(hit_recency(&left.3)))
            .then_with(|| right.3.revision_id.cmp(&left.3.revision_id))
    });
    ranked.into_iter().map(|(_, _, _, hit)| hit).collect()
}

fn reciprocal_rank_units(rank: usize) -> u64 {
    RECIPROCAL_RANK_SCALE / (RECIPROCAL_RANK_OFFSET + rank as u64)
}

fn hit_recency(hit: &SearchHit) -> &str {
    hit.observed_at.as_deref().unwrap_or(&hit.created_at)
}

#[cfg(test)]
mod tests {
    use pcp_core::{LifecycleStatus, PageMutability, SearchHit};

    use super::{RelationPair, rank_text_hits};

    fn hit(page_id: &str, revision_id: &str) -> SearchHit {
        SearchHit {
            page_id: page_id.to_owned(),
            revision_id: revision_id.to_owned(),
            kind: "document".to_owned(),
            mutability: PageMutability::Sealed,
            namespace: "project:test".to_owned(),
            lifecycle_status: LifecycleStatus::Active,
            created_at: "2026-08-18T00:00:00Z".to_owned(),
            observed_at: None,
            snippet: String::new(),
            matched_by: "text".to_owned(),
            matched_projection: "payload".to_owned(),
            summary_revision_id: None,
            facets: None,
            validity: None,
            graph_edges: Vec::new(),
        }
    }

    #[test]
    fn direct_asserted_relation_boosts_only_lexical_candidates() {
        let ranked = rank_text_hits(
            vec![vec![
                hit("pg_anchor", "rev_anchor"),
                hit("pg_isolated", "rev_isolated"),
                hit("pg_supported", "rev_supported"),
            ]],
            &[RelationPair {
                from_page_id: "pg_anchor".to_owned(),
                to_page_id: "pg_supported".to_owned(),
            }],
        );

        assert_eq!(ranked[0].page_id, "pg_anchor");
        assert_eq!(ranked[1].page_id, "pg_supported");
        assert_eq!(ranked[2].page_id, "pg_isolated");
    }

    #[test]
    fn relations_do_not_add_non_lexical_pages() {
        let ranked = rank_text_hits(
            vec![vec![hit("pg_anchor", "rev_anchor")]],
            &[RelationPair {
                from_page_id: "pg_anchor".to_owned(),
                to_page_id: "pg_nonlexical".to_owned(),
            }],
        );

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].page_id, "pg_anchor");
    }
}
