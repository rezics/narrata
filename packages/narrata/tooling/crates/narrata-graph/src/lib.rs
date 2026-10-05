//! Publication-time structural analysis; no dependency on a narrative node format.
//!
//! All ordering and geometry use integers. ADR 0016 fixes the checked CBOR tile
//! and JSON summary projections; neither is a second narrative source.

#[cfg(test)]
extern crate self as narrata_graph;

mod analysis;
mod labels;
mod layout;
mod model;
mod summary;
mod tiles;
pub mod wire;

pub use analysis::{Analysis, NodeAnalysis};
pub use labels::{LabelTable, decode_labels, encode_labels};
pub use layout::{ClusterLayout, CutReason, Layout, OrderKey, Partition, Position};
pub use model::{
    ClusterId, Edge, EdgeKind, Ending, EndingClass, Graph, GraphError, Id, MAX_EDGES, MAX_NODES,
    Node, NodeKind, NormalizedEdge,
};
pub use summary::{
    BottleneckRun, EndingSummary, MAX_SUMMARY_BYTES, SUMMARY_VERSION, SemanticSummary,
    SummaryError, summary_schema,
};
pub use tiles::{Columns, DetailLevel, TILE_ELEMENT_LIMIT, Tile};

/// Above this size, authored clusters acquire bounded subclusters.
pub const CLUSTER_NODE_LIMIT: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Publication {
    /// Canonical ID order; rows in `analysis` and `layout.positions` use this order.
    pub nodes: Vec<Node>,
    /// Parallel edges have one row with counts for each edge kind.
    pub edges: Vec<NormalizedEdge>,
    pub analysis: Analysis,
    pub layout: Layout,
    pub tiles: Vec<Tile>,
    pub summary: SemanticSummary,
}

pub fn prepare(graph: Graph) -> Result<Publication, GraphError> {
    prepare_inner(graph, None)
}

/// A collection publisher bounds route expansion before allocating layout or tile output.
/// The existing graph-only publication retains its original budgets and bytes.
pub fn prepare_with_route_limit(
    graph: Graph,
    route_limit: usize,
) -> Result<Publication, GraphError> {
    prepare_inner(graph, Some(route_limit))
}

/// Structural analysis and summary only; relation reconstruction does not need geometry.
pub fn structural_summary(graph: Graph, route_limit: usize) -> Result<SemanticSummary, GraphError> {
    let graph = model::CheckedGraph::new(graph)?;
    let analysis = analysis::analyze(&graph);
    summary::summarize(&graph, &analysis, Some(route_limit))
}

fn prepare_inner(graph: Graph, route_limit: Option<usize>) -> Result<Publication, GraphError> {
    let graph = model::CheckedGraph::new(graph)?;
    let analysis = analysis::analyze(&graph);
    let summary = summary::summarize(&graph, &analysis, route_limit)?;
    let layout = layout::place(&graph, &analysis);
    let tiles = tiles::build(&graph, &analysis, &layout);
    Ok(Publication {
        nodes: graph.nodes,
        edges: graph.edges,
        analysis,
        layout,
        tiles,
        summary,
    })
}
