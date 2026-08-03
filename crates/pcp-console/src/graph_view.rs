use std::collections::HashSet;

use anyhow::{Context, Result};
use pcp_client::PcpApi;
use pcp_core::{
    Projection, ReadPage, ReadPagesRequest, Relation, SearchFilters, SearchHit, SearchMode,
    SearchPagesRequest, SearchTermMatch,
};
use pcp_rpc::RemotePcpClient;
use serde::Serialize;

pub const DEFAULT_GRAPH_DEPTH: usize = 2;
pub const DEFAULT_GRAPH_NODE_LIMIT: usize = 120;
pub const MAX_GRAPH_DEPTH: usize = 3;
pub const MAX_GRAPH_NODE_LIMIT: usize = 240;

const MAX_DIRECT_NEIGHBORS: usize = 40;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub page_id: String,
    pub depth: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub from_page_id: String,
    pub relation_type: String,
    pub to_page_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphTopology {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub direct_neighbor_count: usize,
    pub depth: usize,
    pub node_limit: usize,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageGraph {
    pub root: ReadPage,
    pub neighbors: Vec<ReadPage>,
    pub hits: Vec<SearchHit>,
    pub topology: GraphTopology,
}

pub async fn load_page_graph(
    client: &RemotePcpClient,
    page_id: String,
    scopes: Vec<String>,
    requested_depth: Option<usize>,
    requested_node_limit: Option<usize>,
) -> Result<PageGraph> {
    let depth = requested_depth
        .unwrap_or(DEFAULT_GRAPH_DEPTH)
        .clamp(1, MAX_GRAPH_DEPTH);
    let node_limit = requested_node_limit
        .unwrap_or(DEFAULT_GRAPH_NODE_LIMIT)
        .clamp(20, MAX_GRAPH_NODE_LIMIT);
    let root = read_one_page(
        client,
        page_id.clone(),
        vec![
            Projection::Manifest,
            Projection::Summary,
            Projection::Validity,
            Projection::Facets,
            Projection::Relations,
        ],
        24_000,
    )
    .await?;

    let direct_limit = usize::try_from(client.capabilities().max_read_pages)
        .unwrap_or(MAX_DIRECT_NEIGHBORS)
        .min(MAX_DIRECT_NEIGHBORS);
    let neighbor_ids = graph_neighbor_ids(&root.relations, &page_id, direct_limit);
    let neighbors = if neighbor_ids.is_empty() {
        Vec::new()
    } else {
        client
            .read_pages(ReadPagesRequest {
                revision_ids: neighbor_ids,
                projections: vec![
                    Projection::Manifest,
                    Projection::Summary,
                    Projection::Validity,
                    Projection::Facets,
                    Projection::Relations,
                ],
                max_chars: 64_000,
            })
            .await?
            .into_iter()
            .filter(|page| !is_superseded(page))
            .collect()
    };
    let graph_hits = client
        .search_pages(SearchPagesRequest {
            query: page_id.clone(),
            scopes,
            mode: SearchMode::Graph,
            term_match: SearchTermMatch::All,
            projections: vec![
                Projection::Manifest,
                Projection::Summary,
                Projection::Payload,
                Projection::Facets,
            ],
            filters: SearchFilters::default(),
            limit: u32::try_from(direct_limit).unwrap_or(MAX_DIRECT_NEIGHBORS as u32),
            cursor: None,
        })
        .await?;
    let topology = load_topology(client, &root, depth, node_limit).await?;

    Ok(PageGraph {
        root,
        neighbors,
        hits: graph_hits.hits,
        topology,
    })
}

async fn load_topology(
    client: &RemotePcpClient,
    root: &ReadPage,
    depth: usize,
    node_limit: usize,
) -> Result<GraphTopology> {
    let root_id = root.revision.revision_id.clone();
    let mut nodes = vec![GraphNode {
        page_id: root_id.clone(),
        depth: 0,
    }];
    let mut seen_nodes = HashSet::from([root_id.clone()]);
    let mut seen_edges = HashSet::new();
    let mut edges = Vec::new();
    let mut current_pages = vec![root.clone()];
    let mut truncated = false;

    for current_depth in 0..depth {
        let mut candidate_ids = Vec::new();
        let mut candidate_relations = Vec::new();
        for page in &current_pages {
            let current_id = &page.revision.revision_id;
            if page.relations.len() >= 200 {
                truncated = true;
            }
            for relation in &page.relations {
                if !is_default_graph_relation(relation) {
                    continue;
                }
                let neighbor_id = match relation_neighbor(relation, current_id) {
                    Some(page_id) => page_id,
                    None => continue,
                };
                candidate_relations.push(relation.clone());
                if !seen_nodes.contains(neighbor_id)
                    && !candidate_ids
                        .iter()
                        .any(|candidate| candidate == neighbor_id)
                {
                    candidate_ids.push(neighbor_id.to_owned());
                }
            }
        }

        let remaining = node_limit.saturating_sub(seen_nodes.len());
        if candidate_ids.len() > remaining {
            candidate_ids.truncate(remaining);
            truncated = true;
        }
        let loaded_candidates = read_relation_pages(client, candidate_ids).await?;
        let mut next_pages = Vec::new();
        for page in loaded_candidates {
            if is_superseded(&page) {
                continue;
            }
            let page_id = page.revision.revision_id.clone();
            if seen_nodes.insert(page_id.clone()) {
                nodes.push(GraphNode {
                    page_id,
                    depth: current_depth + 1,
                });
                next_pages.push(page);
            }
        }

        for relation in candidate_relations {
            if seen_nodes.contains(&relation.from_revision_id)
                && seen_nodes.contains(&relation.to_revision_id)
            {
                let edge_key = (
                    relation.from_revision_id.clone(),
                    relation.relation_type.clone(),
                    relation.to_revision_id.clone(),
                );
                if seen_edges.insert(edge_key.clone()) {
                    edges.push(GraphEdge {
                        from_page_id: edge_key.0,
                        relation_type: edge_key.1,
                        to_page_id: edge_key.2,
                    });
                }
            }
        }

        if current_depth + 1 >= depth || next_pages.is_empty() {
            break;
        }
        current_pages = next_pages;
    }

    Ok(GraphTopology {
        direct_neighbor_count: nodes.iter().filter(|node| node.depth == 1).count(),
        nodes,
        edges,
        depth,
        node_limit,
        truncated,
    })
}

async fn read_relation_pages(
    client: &RemotePcpClient,
    page_ids: Vec<String>,
) -> Result<Vec<ReadPage>> {
    let batch_size = usize::try_from(client.capabilities().max_read_pages)
        .unwrap_or(20)
        .max(1);
    let mut pages = Vec::new();
    for batch in page_ids.chunks(batch_size) {
        let loaded = client
            .read_pages(ReadPagesRequest {
                revision_ids: batch.to_vec(),
                projections: vec![Projection::Manifest, Projection::Relations],
                max_chars: 8_000,
            })
            .await?;
        pages.extend(loaded);
    }
    Ok(pages)
}

async fn read_one_page(
    client: &RemotePcpClient,
    page_id: String,
    projections: Vec<Projection>,
    max_chars: u32,
) -> Result<ReadPage> {
    client
        .read_pages(ReadPagesRequest {
            revision_ids: vec![page_id],
            projections,
            max_chars,
        })
        .await?
        .pop()
        .context("PCP Page was not found")
}

fn relation_neighbor<'a>(relation: &'a Relation, page_id: &str) -> Option<&'a str> {
    if relation.from_revision_id == page_id {
        Some(&relation.to_revision_id)
    } else if relation.to_revision_id == page_id {
        Some(&relation.from_revision_id)
    } else {
        None
    }
}

fn is_default_graph_relation(relation: &Relation) -> bool {
    !matches!(relation.relation_type.as_str(), "supersedes" | "follows")
}

fn is_superseded(page: &ReadPage) -> bool {
    page.relations.iter().any(|relation| {
        relation.relation_type == "supersedes"
            && relation.to_revision_id == page.revision.revision_id
    })
}

fn graph_neighbor_ids(relations: &[Relation], root_page_id: &str, limit: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    relations
        .iter()
        .filter(|relation| is_default_graph_relation(relation))
        .filter_map(|relation| relation_neighbor(relation, root_page_id).map(str::to_owned))
        .filter(|page_id| seen.insert(page_id.clone()))
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use pcp_core::{Actor, ActorType};

    use super::*;

    fn relation(from: &str, relation_type: &str, to: &str) -> Relation {
        Relation {
            relation_id: format!("{from}:{relation_type}:{to}"),
            from_revision_id: from.to_owned(),
            relation_type: relation_type.to_owned(),
            to_revision_id: to.to_owned(),
            created_by: Actor {
                actor_type: ActorType::System,
                actor_id: "system:test".to_owned(),
            },
            created_at: "2026-08-03T00:00:00Z".to_owned(),
        }
    }

    #[test]
    fn graph_neighbors_are_bidirectional_deduplicated_and_bounded() {
        let relations = vec![
            relation("rev_root", "aggregates", "rev_a"),
            relation("rev_root", "derived_from", "rev_a"),
            relation("rev_b", "summarizes", "rev_root"),
            relation("rev_root", "follows", "rev_temporal"),
            relation("rev_root", "supersedes", "rev_old"),
            relation("rev_other", "related", "rev_unrelated"),
        ];

        assert_eq!(
            graph_neighbor_ids(&relations, "rev_root", 10),
            vec!["rev_a", "rev_b"]
        );
        assert_eq!(graph_neighbor_ids(&relations, "rev_root", 1), vec!["rev_a"]);
    }
}
