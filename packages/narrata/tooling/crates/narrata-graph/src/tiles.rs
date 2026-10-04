use std::collections::BTreeMap;

use crate::{Analysis, Layout, NodeKind, model::CheckedGraph};

pub const TILE_ELEMENT_LIMIT: usize = 2000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DetailLevel {
    Work,
    Cluster,
    Node,
}

/// Structure-of-arrays working representation. A future wire codec can pack
/// each vector as a little-endian byte string without per-element JS objects.
/// Coordinates are i32; cluster frames carry the exact i64 world translation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Columns {
    pub ids: Vec<[u8; 16]>,
    pub x: Vec<i32>,
    pub y: Vec<i32>,
    /// Original cluster indices into `Layout::clusters`.
    pub clusters: Vec<u32>,
    /// NodeKind codes, or 6 for an authored cluster, 7 for a partition.
    pub kinds: Vec<u8>,
    pub importance: Vec<u8>,
    /// reachable=1, dead end=2, ending=4, cyclic=8, bottleneck=16, ghost=32.
    pub node_flags: Vec<u32>,
    pub author_order: Vec<u64>,
    pub has_author_order: Vec<u8>,
    /// Tile-local endpoint indices. Ghost rows make every edge self-contained.
    pub sources: Vec<u32>,
    pub targets: Vec<u32>,
    pub edge_kind_masks: Vec<u8>,
    pub edge_kind_counts: Vec<[u32; 5]>,
    pub edge_importance: Vec<u8>,
    /// DFS back edge=1, layout feedback edge=2.
    pub edge_flags: Vec<u8>,
}

impl Columns {
    pub fn element_count(&self) -> usize {
        self.ids.len() + self.sources.len()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tile {
    pub level: DetailLevel,
    pub cluster: Option<u32>,
    pub partition: Option<u32>,
    pub ordinal: u32,
    pub columns: Columns,
}

pub(crate) fn build(graph: &CheckedGraph, analysis: &Analysis, layout: &Layout) -> Vec<Tile> {
    let importance: Vec<_> = (0..graph.nodes.len())
        .map(|node| importance(graph, analysis, node))
        .collect();
    let mut owners = vec![0; graph.nodes.len()];
    for (partition, group) in layout.partitions.iter().enumerate() {
        for &node in &group.nodes {
            owners[node as usize] = partition;
        }
    }
    let mut output = Vec::new();
    let mut work = Columns::default();
    let mut cluster_importance = vec![0_u8; layout.clusters.len()];
    for (node, position) in layout.positions.iter().enumerate() {
        let value = &mut cluster_importance[position.cluster as usize];
        *value = (*value).max(importance[node]);
    }
    for (index, cluster) in layout.clusters.iter().enumerate() {
        aggregate_node(
            &mut work,
            cluster.id.0,
            cluster.overview_x,
            cluster.overview_y,
            index as u32,
            6,
            cluster_importance[index],
        );
    }
    let mut work_edges = BTreeMap::<(u32, u32), ([u32; 5], u8)>::new();
    let mut partition_edges = BTreeMap::<(usize, usize), ([u32; 5], u8)>::new();
    for (index, edge) in graph.edges.iter().enumerate() {
        let source = edge.source as usize;
        let target = edge.target as usize;
        let source_cluster = layout.positions[source].cluster;
        let target_cluster = layout.positions[target].cluster;
        let flags =
            u8::from(analysis.back_edges[index]) | (u8::from(layout.feedback_edges[index]) << 1);
        if source_cluster != target_cluster {
            merge_edge(
                &mut work_edges,
                (source_cluster, target_cluster),
                edge.kind_counts,
                flags,
            );
        } else if owners[source] != owners[target] {
            merge_edge(
                &mut partition_edges,
                (owners[source], owners[target]),
                edge.kind_counts,
                flags,
            );
        }
    }
    for ((source, target), (counts, flags)) in work_edges {
        aggregate_edge(&mut work, source, target, counts, flags);
    }
    output.push(Tile {
        level: DetailLevel::Work,
        cluster: None,
        partition: None,
        ordinal: 0,
        columns: work,
    });

    // Build cluster-level maps in one pass, without rescanning every partition
    // edge for every authored cluster.
    let mut cluster_columns = vec![Columns::default(); layout.clusters.len()];
    let mut partition_local = vec![0_u32; layout.partitions.len()];
    for (index, partition) in layout.partitions.iter().enumerate() {
        let columns = &mut cluster_columns[partition.cluster as usize];
        partition_local[index] = columns.ids.len() as u32;
        let mut sum_x = 0_i64;
        let mut sum_y = 0_i64;
        let mut value = 0;
        for &node in &partition.nodes {
            let position = &layout.positions[node as usize];
            sum_x += i64::from(position.local_x);
            sum_y += i64::from(position.local_y);
            value = value.max(importance[node as usize]);
        }
        let count = partition.nodes.len() as i64;
        aggregate_node(
            columns,
            graph.nodes[partition.nodes[0] as usize].id.0,
            (sum_x / count) as i32,
            (sum_y / count) as i32,
            partition.cluster,
            7,
            value,
        );
    }
    for ((source, target), (counts, flags)) in partition_edges {
        let cluster = layout.partitions[source].cluster as usize;
        aggregate_edge(
            &mut cluster_columns[cluster],
            partition_local[source],
            partition_local[target],
            counts,
            flags,
        );
    }
    for (cluster, columns) in cluster_columns.into_iter().enumerate() {
        output.push(Tile {
            level: DetailLevel::Cluster,
            cluster: Some(cluster as u32),
            partition: None,
            ordinal: 0,
            columns,
        });
    }

    for (partition_index, partition) in layout.partitions.iter().enumerate() {
        let mut builder = NodeTile {
            graph,
            analysis,
            layout,
            importance: &importance,
            owners: &owners,
            partition: partition_index,
            indices: BTreeMap::new(),
            columns: Columns::default(),
            ordinal: 0,
        };
        for &node in &partition.nodes {
            let node = node as usize;
            builder.reserve(&[node], 0, &mut output);
            builder.node(node);
            for &edge_index in &graph.outgoing_edges[node] {
                let edge = &graph.edges[edge_index];
                let target = edge.target as usize;
                builder.reserve(&[node, target], 1, &mut output);
                let source = builder.node(node);
                let target = builder.node(target);
                let flags = u8::from(analysis.back_edges[edge_index])
                    | (u8::from(layout.feedback_edges[edge_index]) << 1);
                aggregate_edge(
                    &mut builder.columns,
                    source,
                    target,
                    edge.kind_counts,
                    flags,
                );
            }
        }
        builder.flush(&mut output);
    }
    output
}

fn importance(graph: &CheckedGraph, analysis: &Analysis, node: usize) -> u8 {
    let row = &analysis.rows[node];
    if row.ending {
        255
    } else if row.bottleneck {
        224
    } else if graph.entries.binary_search(&node).is_ok() {
        208
    } else if graph.nodes[node].kind == NodeKind::Branch {
        160
    } else if matches!(graph.nodes[node].kind, NodeKind::Call | NodeKind::Return) {
        128
    } else {
        // Bounded degree score keeps structural landmarks above ordinary nodes.
        (graph.successors[node].len() + graph.predecessors[node].len()).min(95) as u8 + 32
    }
}

fn merge_edge<K: Ord>(
    merged: &mut BTreeMap<K, ([u32; 5], u8)>,
    key: K,
    counts: [u32; 5],
    flags: u8,
) {
    let (current, current_flags) = merged.entry(key).or_default();
    for (current, added) in current.iter_mut().zip(counts) {
        *current += added;
    }
    *current_flags |= flags;
}

fn aggregate_node(
    columns: &mut Columns,
    id: [u8; 16],
    x: i32,
    y: i32,
    cluster: u32,
    kind: u8,
    importance: u8,
) {
    columns.ids.push(id);
    columns.x.push(x);
    columns.y.push(y);
    columns.clusters.push(cluster);
    columns.kinds.push(kind);
    columns.importance.push(importance);
    columns.node_flags.push(0);
    columns.author_order.push(0);
    columns.has_author_order.push(0);
}

fn aggregate_edge(columns: &mut Columns, source: u32, target: u32, counts: [u32; 5], flags: u8) {
    columns.sources.push(source);
    columns.targets.push(target);
    columns
        .edge_kind_masks
        .push(counts.iter().enumerate().fold(0, |mask, (kind, count)| {
            mask | (u8::from(*count > 0) << kind)
        }));
    columns.edge_kind_counts.push(counts);
    columns
        .edge_importance
        .push(columns.importance[source as usize].max(columns.importance[target as usize]));
    columns.edge_flags.push(flags);
}

struct NodeTile<'a> {
    graph: &'a CheckedGraph,
    analysis: &'a Analysis,
    layout: &'a Layout,
    importance: &'a [u8],
    owners: &'a [usize],
    partition: usize,
    indices: BTreeMap<usize, u32>,
    columns: Columns,
    ordinal: u32,
}

impl NodeTile<'_> {
    fn reserve(&mut self, nodes: &[usize], edges: usize, output: &mut Vec<Tile>) {
        let additional = nodes
            .iter()
            .enumerate()
            .filter(|(offset, node)| {
                !nodes[..*offset].contains(node) && !self.indices.contains_key(node)
            })
            .count();
        if self.columns.element_count() + additional + edges > TILE_ELEMENT_LIMIT {
            self.flush(output);
        }
    }

    fn node(&mut self, node: usize) -> u32 {
        if let Some(&index) = self.indices.get(&node) {
            return index;
        }
        let index = self.columns.ids.len() as u32;
        self.indices.insert(node, index);
        let position = &self.layout.positions[node];
        let row = &self.analysis.rows[node];
        aggregate_node(
            &mut self.columns,
            self.graph.nodes[node].id.0,
            position.local_x,
            position.local_y,
            position.cluster,
            self.graph.nodes[node].kind as u8,
            self.importance[node],
        );
        self.columns.node_flags[index as usize] = u32::from(row.reachable)
            | (u32::from(row.dead_end) << 1)
            | (u32::from(row.ending) << 2)
            | (u32::from(row.cyclic) << 3)
            | (u32::from(row.bottleneck) << 4)
            | (u32::from(self.owners[node] != self.partition) << 5);
        if let Some(order) = position.order_key.author_order {
            self.columns.author_order[index as usize] = order;
            self.columns.has_author_order[index as usize] = 1;
        }
        index
    }

    fn flush(&mut self, output: &mut Vec<Tile>) {
        if self.columns.ids.is_empty() {
            return;
        }
        output.push(Tile {
            level: DetailLevel::Node,
            cluster: Some(self.layout.partitions[self.partition].cluster),
            partition: Some(self.partition as u32),
            ordinal: self.ordinal,
            columns: std::mem::take(&mut self.columns),
        });
        self.ordinal += 1;
        self.indices.clear();
    }
}
