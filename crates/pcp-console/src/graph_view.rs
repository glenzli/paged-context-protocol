use std::collections::HashSet;

use anyhow::{Context, Result};
use pcp_client::PcpTenantApi;
use pcp_core::{
    GraphEdgeDirection, GraphEdgeKind, Projection, ReadPage, ReadPagesRequest, Relation,
    SearchFilters, SearchHit, SearchMode, SearchPagesRequest, SearchTermMatch,
};
use pcp_rpc::RemotePcpClient;
use serde::Serialize;

pub const DEFAULT_GRAPH_DEPTH: usize = 2;
pub const DEFAULT_GRAPH_NODE_LIMIT: usize = 120;
pub const MAX_GRAPH_DEPTH: usize = 3;
pub const MAX_GRAPH_NODE_LIMIT: usize = 240;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub page_id: String,
    pub depth: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub from_page_id: String,
    pub relation_type: String,
    pub to_page_id: String,
    pub edge_kind: GraphEdgeKind,
    pub basis_revision_ids: Vec<String>,
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

struct GraphStep {
    pages: Vec<ReadPage>,
    hits: Vec<SearchHit>,
    edges: Vec<GraphEdge>,
    truncated: bool,
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

    let root_step = load_graph_step(client, &root, &scopes).await?;
    let neighbors = root_step.pages.clone();
    let hits = root_step.hits.clone();
    let topology =
        load_topology(client, root.clone(), root_step, scopes, depth, node_limit).await?;

    Ok(PageGraph {
        root,
        neighbors,
        hits,
        topology,
    })
}

async fn load_topology(
    client: &RemotePcpClient,
    root: ReadPage,
    root_step: GraphStep,
    scopes: Vec<String>,
    depth: usize,
    node_limit: usize,
) -> Result<GraphTopology> {
    let root_id = root.page.page_id.clone();
    let mut nodes = vec![GraphNode {
        page_id: root_id.clone(),
        depth: 0,
    }];
    let mut seen_nodes = HashSet::from([root_id.clone()]);
    let mut edges = Vec::new();
    let mut current = vec![(root, root_step)];
    let mut truncated = false;

    for current_depth in 0..depth {
        let mut candidate_pages = Vec::new();
        let mut candidate_edges = Vec::new();
        for (_, step) in &current {
            truncated |= step.truncated;
            candidate_edges.extend(step.edges.iter().cloned());
            for page in &step.pages {
                if !candidate_pages
                    .iter()
                    .any(|candidate: &ReadPage| candidate.page.page_id == page.page.page_id)
                {
                    candidate_pages.push(page.clone());
                }
            }
        }

        let mut next_pages = Vec::new();
        let remaining = node_limit.saturating_sub(seen_nodes.len());
        for page in candidate_pages {
            if seen_nodes.contains(&page.page.page_id) {
                continue;
            }
            if next_pages.len() >= remaining {
                truncated = true;
                continue;
            }
            if is_superseded(&page) {
                continue;
            }
            let page_id = page.page.page_id.clone();
            seen_nodes.insert(page_id.clone());
            nodes.push(GraphNode {
                page_id,
                depth: current_depth + 1,
            });
            next_pages.push(page);
        }

        for edge in candidate_edges {
            if seen_nodes.contains(&edge.from_page_id) && seen_nodes.contains(&edge.to_page_id) {
                insert_graph_edge(&mut edges, edge);
            }
        }

        if current_depth + 1 >= depth || next_pages.is_empty() {
            break;
        }
        current = Vec::with_capacity(next_pages.len());
        for page in next_pages {
            let step = load_graph_step(client, &page, &scopes).await?;
            current.push((page, step));
        }
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

async fn load_graph_step(
    client: &RemotePcpClient,
    page: &ReadPage,
    scopes: &[String],
) -> Result<GraphStep> {
    let graph = client
        .search_pages(SearchPagesRequest {
            query: page.page.page_id.clone(),
            scopes: scopes.to_vec(),
            mode: SearchMode::Graph,
            term_match: SearchTermMatch::All,
            projections: vec![
                Projection::Manifest,
                Projection::Summary,
                Projection::Payload,
                Projection::Facets,
            ],
            filters: SearchFilters::default(),
            limit: client.capabilities().max_search_results,
            cursor: None,
        })
        .await?;
    let mut edges = relation_edges(page);
    let mut candidate_ids = edges
        .iter()
        .flat_map(|edge| [&edge.from_page_id, &edge.to_page_id])
        .filter(|page_id| *page_id != &page.page.page_id)
        .cloned()
        .collect::<Vec<_>>();
    for hit in &graph.hits {
        for metadata in &hit.graph_edges {
            if !is_default_graph_edge(metadata.edge_kind.clone(), &metadata.relation_type) {
                continue;
            }
            let edge = match metadata.direction {
                GraphEdgeDirection::Outgoing => GraphEdge {
                    from_page_id: page.page.page_id.clone(),
                    relation_type: metadata.relation_type.clone(),
                    to_page_id: hit.page_id.clone(),
                    edge_kind: metadata.edge_kind.clone(),
                    basis_revision_ids: metadata.basis_revision_ids.clone(),
                },
                GraphEdgeDirection::Incoming => GraphEdge {
                    from_page_id: hit.page_id.clone(),
                    relation_type: metadata.relation_type.clone(),
                    to_page_id: page.page.page_id.clone(),
                    edge_kind: metadata.edge_kind.clone(),
                    basis_revision_ids: metadata.basis_revision_ids.clone(),
                },
            };
            if edge.from_page_id != edge.to_page_id {
                candidate_ids.push(hit.page_id.clone());
                insert_graph_edge(&mut edges, edge);
            }
        }
    }
    candidate_ids.sort();
    candidate_ids.dedup();
    candidate_ids.retain(|page_id| page_id != &page.page.page_id);
    let pages = read_graph_pages(client, candidate_ids).await?;
    let readable = pages
        .iter()
        .map(|page| page.page.page_id.as_str())
        .collect::<HashSet<_>>();
    edges.retain(|edge| {
        (edge.from_page_id == page.page.page_id || readable.contains(edge.from_page_id.as_str()))
            && (edge.to_page_id == page.page.page_id || readable.contains(edge.to_page_id.as_str()))
    });

    Ok(GraphStep {
        pages,
        hits: graph.hits,
        edges,
        truncated: graph.next_cursor.is_some() || page.relations.len() >= 200,
    })
}

fn relation_edges(page: &ReadPage) -> Vec<GraphEdge> {
    let mut edges = Vec::new();
    for relation in &page.relations {
        if !is_default_graph_relation(relation)
            || relation_neighbor(relation, &page.page.page_id).is_none()
        {
            continue;
        }
        insert_graph_edge(
            &mut edges,
            GraphEdge {
                from_page_id: relation.from_page_id.clone(),
                relation_type: relation.relation_type.clone(),
                to_page_id: relation.to_page_id.clone(),
                edge_kind: GraphEdgeKind::Relation,
                basis_revision_ids: relation.basis_revision_ids.clone(),
            },
        );
    }
    edges
}

fn insert_graph_edge(edges: &mut Vec<GraphEdge>, edge: GraphEdge) {
    if let Some(existing) = edges.iter_mut().find(|existing| {
        existing.from_page_id == edge.from_page_id
            && existing.relation_type == edge.relation_type
            && existing.to_page_id == edge.to_page_id
            && existing.edge_kind == edge.edge_kind
    }) {
        for revision_id in edge.basis_revision_ids {
            if !existing.basis_revision_ids.contains(&revision_id) {
                existing.basis_revision_ids.push(revision_id);
            }
        }
        return;
    }
    edges.push(edge);
}

fn is_default_graph_edge(edge_kind: GraphEdgeKind, relation_type: &str) -> bool {
    edge_kind == GraphEdgeKind::Provenance || !matches!(relation_type, "supersedes" | "follows")
}

async fn read_graph_pages(
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
                page_ids: batch.to_vec(),
                revision_ids: Vec::new(),
                projections: vec![
                    Projection::Manifest,
                    Projection::Summary,
                    Projection::Validity,
                    Projection::Facets,
                    Projection::Relations,
                ],
                max_chars: 64_000,
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
            page_ids: vec![page_id],
            revision_ids: Vec::new(),
            projections,
            max_chars,
        })
        .await?
        .pop()
        .context("PCP Page was not found")
}

fn relation_neighbor<'a>(relation: &'a Relation, page_id: &str) -> Option<&'a str> {
    if relation.from_page_id == page_id {
        Some(&relation.to_page_id)
    } else if relation.to_page_id == page_id {
        Some(&relation.from_page_id)
    } else {
        None
    }
}

fn is_default_graph_relation(relation: &Relation) -> bool {
    !matches!(relation.relation_type.as_str(), "supersedes" | "follows")
}

fn is_superseded(page: &ReadPage) -> bool {
    page.page.lifecycle_status == pcp_core::LifecycleStatus::Superseded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_keeps_asserted_relations_and_provenance_distinct() {
        let mut edges = Vec::new();
        for edge_kind in [GraphEdgeKind::Relation, GraphEdgeKind::Provenance] {
            insert_graph_edge(
                &mut edges,
                GraphEdge {
                    from_page_id: "pg_derived".to_owned(),
                    relation_type: "derived_from".to_owned(),
                    to_page_id: "pg_source".to_owned(),
                    edge_kind,
                    basis_revision_ids: vec!["rev_source_a".to_owned()],
                },
            );
        }
        insert_graph_edge(
            &mut edges,
            GraphEdge {
                from_page_id: "pg_derived".to_owned(),
                relation_type: "derived_from".to_owned(),
                to_page_id: "pg_source".to_owned(),
                edge_kind: GraphEdgeKind::Provenance,
                basis_revision_ids: vec!["rev_source_b".to_owned()],
            },
        );

        assert_eq!(edges.len(), 2);
        assert_eq!(edges[1].basis_revision_ids.len(), 2);
        assert!(is_default_graph_edge(
            GraphEdgeKind::Provenance,
            "derived_from"
        ));
        assert!(!is_default_graph_edge(GraphEdgeKind::Relation, "follows"));
    }
}
