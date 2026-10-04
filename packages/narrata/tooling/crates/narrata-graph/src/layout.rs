use std::collections::{BTreeMap, BTreeSet};

use crate::{Analysis, CLUSTER_NODE_LIMIT, ClusterId, Id, analysis, model::CheckedGraph};

const X_STEP: i32 = 32;
const Y_STEP: i32 = 64;
// Every checked graph has <= 1M nodes: a cluster's local axes fit this frame.
// Keeping frame size independent of content prevents unrelated-cluster drift.
const FRAME_STEP: i64 = 1 << 26;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct OrderKey {
    pub author_order: Option<u64>,
    pub id: Id,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Position {
    pub cluster: u32,
    pub layer: u32,
    pub order: u32,
    pub order_key: OrderKey,
    /// Small exact local coordinates used by node-level typed columns.
    pub local_x: i32,
    pub local_y: i32,
    /// Cluster frame translation plus local coordinates, without f32 rounding.
    pub x: i64,
    pub y: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClusterLayout {
    pub id: ClusterId,
    pub layer: u32,
    pub order: u32,
    pub origin_x: i64,
    pub origin_y: i64,
    /// Compact work-level coordinates; node-level views use the local frame.
    pub overview_x: i32,
    pub overview_y: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CutReason {
    Authored,
    Bottleneck,
    Cycle,
    /// A large acyclic wide region may have no bottleneck near the budget.
    /// The hard cap still applies; arbitrary nodes are never dropped.
    Budget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Partition {
    pub cluster: u32,
    pub ordinal: u32,
    pub cut: CutReason,
    /// Canonical row indices, in layered display order.
    pub nodes: Vec<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Layout {
    pub positions: Vec<Position>,
    pub clusters: Vec<ClusterLayout>,
    pub partitions: Vec<Partition>,
    /// Edges omitted from rank constraints, including meta-graph feedback.
    /// Every other edge strictly increases global y.
    pub feedback_edges: Vec<bool>,
}

pub(crate) fn place(graph: &CheckedGraph, analysis: &Analysis) -> Layout {
    let mut grouped = BTreeMap::<ClusterId, Vec<usize>>::new();
    for (index, node) in graph.nodes.iter().enumerate() {
        grouped.entry(node.cluster).or_default().push(index);
    }
    let cluster_indices: BTreeMap<_, _> = grouped
        .keys()
        .enumerate()
        .map(|(index, id)| (*id, index))
        .collect();
    let node_clusters: Vec<_> = graph
        .nodes
        .iter()
        .map(|node| cluster_indices[&node.cluster])
        .collect();
    let mut meta_edges = BTreeSet::new();
    for edge in &graph.edges {
        let source = node_clusters[edge.source as usize];
        let target = node_clusters[edge.target as usize];
        if source != target {
            meta_edges.insert((source, target));
        }
    }
    let mut meta_successors = vec![Vec::new(); grouped.len()];
    let mut meta_predecessors = vec![Vec::new(); grouped.len()];
    for (source, target) in meta_edges {
        meta_successors[source].push(target);
        meta_predecessors[target].push(source);
    }
    let meta_keys: Vec<_> = grouped
        .keys()
        .map(|id| OrderKey {
            author_order: None,
            id: Id(id.0),
        })
        .collect();
    let meta = layered(&meta_successors, &meta_predecessors, &meta_keys);
    let clusters: Vec<_> = grouped
        .keys()
        .enumerate()
        .map(|(index, id)| ClusterLayout {
            id: *id,
            layer: meta.layers[index],
            order: meta.orders[index],
            origin_x: i64::from(meta.orders[index]) * FRAME_STEP,
            origin_y: i64::from(meta.layers[index]) * FRAME_STEP,
            overview_x: meta.orders[index] as i32 * X_STEP,
            overview_y: meta.layers[index] as i32 * Y_STEP,
        })
        .collect();
    let mut positions = vec![None; graph.nodes.len()];
    let mut local_indices = vec![0; graph.nodes.len()];
    let mut feedback_edges = vec![false; graph.edges.len()];
    let mut partitions = Vec::new();
    for (cluster, nodes) in grouped.values().enumerate() {
        for (index, &node) in nodes.iter().enumerate() {
            local_indices[node] = index;
        }
        let mut successors = vec![Vec::new(); nodes.len()];
        let mut predecessors = vec![Vec::new(); nodes.len()];
        let mut local_edges = Vec::new();
        for (source, &node) in nodes.iter().enumerate() {
            for &edge_index in &graph.outgoing_edges[node] {
                let edge = &graph.edges[edge_index];
                let target_node = edge.target as usize;
                if node_clusters[target_node] == cluster {
                    let target = local_indices[target_node];
                    successors[source].push(target);
                    predecessors[target].push(source);
                    local_edges.push((edge_index, source, target));
                } else {
                    feedback_edges[edge_index] = meta.feedback[cluster]
                        .binary_search(&node_clusters[target_node])
                        .is_ok();
                }
            }
        }
        let keys: Vec<_> = nodes
            .iter()
            .map(|&node| OrderKey {
                author_order: graph.nodes[node].author_order,
                id: graph.nodes[node].id,
            })
            .collect();
        let local = layered(&successors, &predecessors, &keys);
        for (edge, source, target) in local_edges {
            feedback_edges[edge] = local.feedback[source].binary_search(&target).is_ok();
        }
        for (index, &node) in nodes.iter().enumerate() {
            let local_x = local.orders[index] as i32 * X_STEP;
            let local_y = local.layers[index] as i32 * Y_STEP;
            positions[node] = Some(Position {
                cluster: cluster as u32,
                layer: local.layers[index],
                order: local.orders[index],
                order_key: keys[index],
                local_x,
                local_y,
                x: clusters[cluster].origin_x + i64::from(local_x),
                y: clusters[cluster].origin_y + i64::from(local_y),
            });
        }
        let mut display_order = nodes.clone();
        display_order.sort_by_key(|node| {
            let local_node = local_indices[*node];
            (local.layers[local_node], local.orders[local_node])
        });
        partition(cluster, &display_order, analysis, &mut partitions);
    }
    Layout {
        // Every canonical node belongs to exactly one authored group; filling
        // by index above proves all entries exist, including unreachable nodes.
        positions: positions.into_iter().flatten().collect(),
        clusters,
        partitions,
        feedback_edges,
    }
}

fn partition(cluster: usize, nodes: &[usize], analysis: &Analysis, output: &mut Vec<Partition>) {
    let mut start = 0;
    let mut ordinal = 0;
    let mut cut = CutReason::Authored;
    while start < nodes.len() {
        let mut end = (start + CLUSTER_NODE_LIMIT).min(nodes.len());
        let mut next_cut = CutReason::Budget;
        if end < nodes.len() {
            // Prefer a structural boundary in the last quarter of the budget.
            // Never exceed the budget to keep an exceptionally large SCC whole.
            for candidate in (start + CLUSTER_NODE_LIMIT * 3 / 4..=end).rev() {
                let node = nodes[candidate];
                let row = &analysis.rows[node];
                if row.bottleneck {
                    end = candidate;
                    next_cut = CutReason::Bottleneck;
                    break;
                }
                if row.cyclic {
                    end = candidate;
                    next_cut = CutReason::Cycle;
                    break;
                }
            }
        }
        output.push(Partition {
            cluster: cluster as u32,
            ordinal,
            cut,
            nodes: nodes[start..end].iter().map(|node| *node as u32).collect(),
        });
        cut = next_cut;
        ordinal += 1;
        start = end;
    }
}

struct Layered {
    layers: Vec<u32>,
    orders: Vec<u32>,
    feedback: Vec<Vec<usize>>,
}

/// Longest-path ranks after deterministic DFS cycle breaking. Two integer
/// barycenter sweeps are sufficient for locally rejoining narrative choices;
/// no network-simplex objective or floating-point convergence is needed.
fn layered(successors: &[Vec<usize>], predecessors: &[Vec<usize>], keys: &[OrderKey]) -> Layered {
    let count = successors.len();
    let feedback = analysis::feedback(successors);
    let is_forward =
        |source: usize, target: usize| feedback[source].binary_search(&target).is_err();
    let mut indegree: Vec<_> = predecessors
        .iter()
        .enumerate()
        .map(|(target, parents)| {
            parents
                .iter()
                .filter(|source| is_forward(**source, target))
                .count()
        })
        .collect();
    let mut ready: BTreeSet<_> = (0..count).filter(|node| indegree[*node] == 0).collect();
    let mut layers = vec![0_u32; count];
    while let Some(node) = ready.pop_first() {
        for &child in &successors[node] {
            if is_forward(node, child) {
                layers[child] = layers[child].max(layers[node] + 1);
                indegree[child] -= 1;
                if indegree[child] == 0 {
                    ready.insert(child);
                }
            }
        }
    }
    let mut groups = vec![
        Vec::new();
        layers
            .iter()
            .copied()
            .max()
            .map_or(0, |rank| rank as usize + 1)
    ];
    for (node, &layer) in layers.iter().enumerate() {
        groups[layer as usize].push(node);
    }
    let mut orders = vec![0; count];
    for group in &mut groups {
        group.sort_by_key(|node| keys[*node]);
        assign_order(group, &mut orders);
    }
    for _ in 0..2 {
        for group in &mut groups {
            sweep(group, predecessors, &mut orders, keys, |source, target| {
                is_forward(source, target)
            });
        }
        for group in groups.iter_mut().rev() {
            sweep(group, successors, &mut orders, keys, |target, source| {
                is_forward(source, target)
            });
        }
    }
    Layered {
        layers,
        orders,
        feedback,
    }
}

fn assign_order(group: &[usize], orders: &mut [u32]) {
    for (order, &node) in group.iter().enumerate() {
        orders[node] = order as u32;
    }
}

fn sweep(
    group: &mut [usize],
    neighbors: &[Vec<usize>],
    orders: &mut [u32],
    keys: &[OrderKey],
    allowed: impl Fn(usize, usize) -> bool,
) {
    if group.len() < 2 {
        return;
    }
    let centers: BTreeMap<_, _> = group
        .iter()
        .map(|&node| {
            let mut sum = 0_u64;
            let mut weight = 0_u64;
            for &neighbor in &neighbors[node] {
                if allowed(neighbor, node) {
                    sum += u64::from(orders[neighbor]);
                    weight += 1;
                }
            }
            if weight == 0 {
                sum = u64::from(orders[node]);
                weight = 1;
            }
            (node, (sum, weight))
        })
        .collect();
    group.sort_by(|left, right| {
        let (left_sum, left_weight) = centers[left];
        let (right_sum, right_weight) = centers[right];
        (u128::from(left_sum) * u128::from(right_weight))
            .cmp(&(u128::from(right_sum) * u128::from(left_weight)))
            .then_with(|| keys[*left].cmp(&keys[*right]))
    });
    assign_order(group, orders);
}
