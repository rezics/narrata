use std::collections::{BTreeMap, BTreeSet};

/// Bounds make integer coordinate ranges and edge multiplicities provable.
pub const MAX_NODES: usize = 1_000_000;
pub const MAX_EDGES: usize = 4_000_000;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Id(pub [u8; 16]);

impl Id {
    pub const fn from_u128(value: u128) -> Self {
        Self(value.to_be_bytes())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ClusterId(pub [u8; 16]);

impl ClusterId {
    pub const fn from_u128(value: u128) -> Self {
        Self(value.to_be_bytes())
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum NodeKind {
    Passage = 0,
    Branch = 1,
    State = 2,
    Call = 3,
    Return = 4,
    Ending = 5,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum EdgeKind {
    Choice = 0,
    Next = 1,
    Conditional = 2,
    Call = 3,
    Return = 4,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Node {
    pub id: Id,
    pub kind: NodeKind,
    pub cluster: ClusterId,
    pub author_order: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Edge {
    pub source: Id,
    pub target: Id,
    pub kind: EdgeKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndingClass {
    Unspecified,
    Success,
    Failure,
    Neutral,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ending {
    pub node: Id,
    pub class: EndingClass,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub entries: Vec<Id>,
    /// `NodeKind::Ending` also declares an unspecified ending. An explicit row
    /// supplies its classification; other kinds may be declared endings too.
    pub endings: Vec<Ending>,
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum GraphError {
    #[error("graph exceeds the {0} budget")]
    Limit(&'static str),
    #[error("duplicate node ID: {0:?}")]
    DuplicateNode(Id),
    #[error("unknown node ID: {0:?}")]
    UnknownNode(Id),
    #[error("duplicate entry ID: {0:?}")]
    DuplicateEntry(Id),
    #[error("duplicate ending ID: {0:?}")]
    DuplicateEnding(Id),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalizedEdge {
    /// Indices into the canonical node rows, independent of input ordering.
    pub source: u32,
    pub target: u32,
    /// Choice, next, conditional, call, return, respectively. Different kinds
    /// between the same endpoints remain distinguishable after parallel merge.
    pub kind_counts: [u32; 5],
}

impl NormalizedEdge {
    pub fn multiplicity(&self) -> u64 {
        self.kind_counts.iter().map(|count| u64::from(*count)).sum()
    }

    pub fn kind_mask(&self) -> u8 {
        self.kind_counts
            .iter()
            .enumerate()
            .fold(0, |mask, (kind, count)| {
                mask | (u8::from(*count > 0) << kind)
            })
    }
}

pub(crate) struct CheckedGraph {
    pub nodes: Vec<Node>,
    pub edges: Vec<NormalizedEdge>,
    pub successors: Vec<Vec<usize>>,
    pub predecessors: Vec<Vec<usize>>,
    pub outgoing_edges: Vec<Vec<usize>>,
    pub entries: Vec<usize>,
    pub endings: BTreeMap<usize, EndingClass>,
}

impl CheckedGraph {
    pub fn new(mut input: Graph) -> Result<Self, GraphError> {
        if input.nodes.len() > MAX_NODES {
            return Err(GraphError::Limit("node"));
        }
        if input.edges.len() > MAX_EDGES {
            return Err(GraphError::Limit("edge"));
        }
        input.nodes.sort_by_key(|node| node.id);
        let mut indices = BTreeMap::new();
        for (index, node) in input.nodes.iter().enumerate() {
            if indices.insert(node.id, index).is_some() {
                return Err(GraphError::DuplicateNode(node.id));
            }
        }
        let lookup = |id: Id| indices.get(&id).copied().ok_or(GraphError::UnknownNode(id));
        let mut entry_set = BTreeSet::new();
        for entry in input.entries {
            let index = lookup(entry)?;
            if !entry_set.insert(index) {
                return Err(GraphError::DuplicateEntry(entry));
            }
        }
        let mut endings = BTreeMap::new();
        for ending in input.endings {
            let index = lookup(ending.node)?;
            if endings.insert(index, ending.class).is_some() {
                return Err(GraphError::DuplicateEnding(ending.node));
            }
        }
        for (index, node) in input.nodes.iter().enumerate() {
            if node.kind == NodeKind::Ending {
                endings.entry(index).or_insert(EndingClass::Unspecified);
            }
        }
        let mut merged = BTreeMap::<(usize, usize), [u32; 5]>::new();
        for edge in input.edges {
            let source = lookup(edge.source)?;
            let target = lookup(edge.target)?;
            merged.entry((source, target)).or_default()[edge.kind as usize] += 1;
        }
        let count = input.nodes.len();
        let mut successors = vec![Vec::new(); count];
        let mut predecessors = vec![Vec::new(); count];
        let mut outgoing_edges = vec![Vec::new(); count];
        let mut edges = Vec::with_capacity(merged.len());
        for ((source, target), kind_counts) in merged {
            outgoing_edges[source].push(edges.len());
            edges.push(NormalizedEdge {
                // Checked node budget is below u32::MAX on native and Wasm.
                source: source as u32,
                target: target as u32,
                kind_counts,
            });
            successors[source].push(target);
            predecessors[target].push(source);
        }
        Ok(Self {
            nodes: input.nodes,
            edges,
            successors,
            predecessors,
            outgoing_edges,
            entries: entry_set.into_iter().collect(),
            endings,
        })
    }
}
