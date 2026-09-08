//! Cheap subject-affinity routing across authorized Scopes. These groups only
//! choose bounded model inputs; they never assert a Relation or a Topic.
use std::collections::{BTreeMap, BTreeSet};

use pcp_store::DurablePageInventoryItem;

use super::TopicMaintenanceConfig;

pub(super) fn accumulated(
    pages: &[&DurablePageInventoryItem],
    config: &TopicMaintenanceConfig,
) -> bool {
    pages.len() >= 2
        && (pages.len() >= config.minimum_pages
            || pages.iter().map(|page| page.content_chars).sum::<u64>()
                >= config.minimum_total_chars)
}

/// Prefer subject markers shared by multiple Pages, independent of chronology
/// and namespace. Keep each prompt bounded and deduplicate equal source sets.
pub(super) fn affinity_windows(
    pages: &[DurablePageInventoryItem],
    max_pages: usize,
) -> Vec<Vec<DurablePageInventoryItem>> {
    let mut index: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    let terms = pages.iter().map(subject_terms).collect::<Vec<_>>();
    for (position, terms) in terms.iter().enumerate() {
        for term in terms {
            index.entry(term.clone()).or_default().insert(position);
        }
    }
    let mut groups = Vec::new();
    let mut seen = BTreeSet::new();
    for (anchor, markers) in terms.iter().enumerate() {
        let mut scores: BTreeMap<usize, (u64, usize, bool)> = BTreeMap::new();
        for marker in markers {
            let positions = &index[marker];
            let explicit = marker.starts_with("facet:");
            if positions.len() < 2 {
                continue;
            }
            for position in positions
                .iter()
                .cycle()
                .skip(anchor % positions.len())
                .take(positions.len().min(max_pages * 4))
            {
                if *position == anchor {
                    continue;
                }
                let score = scores.entry(*position).or_default();
                score.0 += (if explicit { 4_000 } else { 1_000 }) / positions.len() as u64;
                score.1 += 1;
                score.2 |= explicit || marker.starts_with("word:");
            }
        }
        let mut related = scores
            .into_iter()
            .filter(|(_, (_, matches, named))| *named || *matches >= 2)
            .collect::<Vec<_>>();
        related.sort_by(|(left, a), (right, b)| {
            b.0.cmp(&a.0)
                .then_with(|| pages[*left].page_id.cmp(&pages[*right].page_id))
        });
        let score = related.iter().map(|(_, score)| score.0).sum::<u64>();
        let mut selected = vec![anchor];
        selected.extend(
            related
                .into_iter()
                .take(max_pages.saturating_sub(1))
                .map(|(position, _)| position),
        );
        selected.sort_by(|a, b| pages[*a].page_id.cmp(&pages[*b].page_id));
        let key = selected
            .iter()
            .map(|position| pages[*position].revision_id.clone())
            .collect::<Vec<_>>();
        if selected.len() >= 2 && seen.insert(key) {
            groups.push((
                score,
                selected
                    .into_iter()
                    .map(|position| pages[position].clone())
                    .collect::<Vec<_>>(),
            ));
        }
    }
    groups.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1[0].page_id.cmp(&b.1[0].page_id))
    });
    groups.into_iter().map(|(_, pages)| pages).collect()
}

pub(super) fn subject_terms(page: &DurablePageInventoryItem) -> BTreeSet<String> {
    let mut terms = BTreeSet::new();
    if let Some(facets) = page.facets.as_ref().and_then(|value| value.as_object()) {
        for key in [
            "topic",
            "topics",
            "subject",
            "subjects",
            "project",
            "projects",
            "entity",
            "entities",
            "concept",
            "concepts",
            "keywords",
            "tags",
            "topicTitle",
        ] {
            if let Some(value) = facets.get(key) {
                let values = match value {
                    serde_json::Value::String(value) => vec![value.as_str()],
                    serde_json::Value::Array(values) => {
                        values.iter().filter_map(|value| value.as_str()).collect()
                    }
                    _ => Vec::new(),
                };
                for value in values.into_iter().take(32) {
                    let value = value.trim().to_lowercase();
                    if !value.is_empty() && value.chars().count() <= 120 {
                        terms.insert(format!("facet:{value}"));
                    }
                }
            }
        }
    }
    let text = page
        .summary
        .as_deref()
        .unwrap_or(&page.snippet)
        .chars()
        .take(800)
        .collect::<String>();
    for word in text.split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_') {
        let word = word.to_ascii_lowercase();
        if (3..=64).contains(&word.len())
            && word.chars().any(|c| c.is_ascii_alphabetic())
            && ![
                "the",
                "and",
                "for",
                "with",
                "this",
                "that",
                "from",
                "are",
                "not",
                "has",
                "have",
                "into",
                "only",
                "user",
                "page",
                "pages",
                "scope",
                "scopes",
                "should",
                "must",
                "will",
                "can",
                "model",
                "system",
                "context",
                "source",
                "sources",
                "maintenance",
            ]
            .contains(&word.as_str())
        {
            terms.insert(format!("word:{word}"));
        }
    }
    // CJK prose has no word separators. Two shared trigrams are a routing hint;
    // the semantic worker still has to reject generic or accidental overlap.
    for run in text.split(|c: char| !(('\u{3400}'..='\u{9fff}').contains(&c))) {
        let chars = run.chars().collect::<Vec<_>>();
        for trigram in chars.windows(3) {
            terms.insert(format!("cjk:{}", trigram.iter().collect::<String>()));
        }
    }
    terms
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(id: &str, scope: &str, text: &str) -> DurablePageInventoryItem {
        serde_json::from_value(serde_json::json!({
            "pageId": id, "revisionId": format!("rev_{id}"), "namespace": scope,
            "kind": "note", "mutability": "sealed", "createdAt": "2026-09-01T00:00:00Z",
            "contentChars": text.chars().count(), "snippet": text, "relationTypes": []
        }))
        .unwrap()
    }

    #[test]
    fn short_same_subject_pages_across_scopes_are_grouped_without_long_page_threshold() {
        let pages = (0..6)
            .map(|i| {
                page(
                    &format!("p{i}"),
                    if i % 2 == 0 { "a" } else { "b" },
                    "OET certificate locality preserves explicit proof assumptions",
                )
            })
            .collect::<Vec<_>>();
        let windows = affinity_windows(&pages, 4);
        assert!(!windows.is_empty());
        assert!(windows.iter().all(|window| window.len() <= 4));
        assert!(windows.iter().any(|window| {
            window
                .iter()
                .map(|p| &p.namespace)
                .collect::<BTreeSet<_>>()
                .len()
                == 2
        }));
        assert!(accumulated(
            &windows[0].iter().collect::<Vec<_>>(),
            &TopicMaintenanceConfig::default()
        ));
    }

    #[test]
    fn shared_namespace_and_generic_words_are_not_subject_evidence() {
        let pages = vec![
            page("a", "same", "The user and the system"),
            page("b", "same", "The system and the user"),
        ];
        assert!(affinity_windows(&pages, 8).is_empty());
    }

    #[test]
    fn cjk_subject_overlap_routes_across_sources() {
        let pages = vec![
            page("a", "one", "谱截断条件需要保留证明前提"),
            page("b", "two", "分析谱截断条件与误差估计"),
        ];
        assert_eq!(affinity_windows(&pages, 8).len(), 1);
    }
}
