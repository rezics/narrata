//! Publication-time structural analysis; no dependency on a narrative node format.
//!
//! All ordering and geometry use integers. The result is an in-memory projection,
//! not a second narrative source. A persisted tile/summary format requires its own
//! ADR and compatibility corpus before an encoder is exposed.

mod analysis;
mod layout;
mod model;
mod summary;
mod tiles;

pub use analysis::{Analysis, NodeAnalysis};
pub use layout::{ClusterLayout, CutReason, Layout, OrderKey, Partition, Position};
pub use model::{
    ClusterId, Edge, EdgeKind, Ending, EndingClass, Graph, GraphError, Id, MAX_EDGES, MAX_NODES,
    Node, NodeKind, NormalizedEdge,
};
pub use summary::{BottleneckRun, EndingSummary, SemanticSummary};
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
    let graph = model::CheckedGraph::new(graph)?;
    let analysis = analysis::analyze(&graph);
    let layout = layout::place(&graph, &analysis);
    let tiles = tiles::build(&graph, &analysis, &layout);
    let summary = summary::summarize(&graph, &analysis);
    Ok(Publication {
        nodes: graph.nodes,
        edges: graph.edges,
        analysis,
        layout,
        tiles,
        summary,
    })
}
