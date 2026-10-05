//! Compiled artifact objects (ADR 0013 §8): the manifest, program chunks, the tombstone set
//! and the name table. Chunks hold the checked execution vocabulary addressed by ID; aliases
//! live only in the name table.

use std::collections::{BTreeMap, BTreeSet};

use narrata_kernel::content::{AnchorId, ContentRef, Segment};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    ArtifactId, Assignment, ChoicePointId, Expr, NodeId, ObjectId, OptionId, Scalar, ScalarType,
    source::R1ArtifactId,
};

/// A graph of one package instance. Package aliases and graph names are composition
/// structure, so unlike node aliases they are part of the artifact identity.
#[derive(
    Clone, Debug, Deserialize, Eq, Hash, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(deny_unknown_fields)]
pub struct GraphRef {
    pub package: String,
    pub graph: String,
}

impl GraphRef {
    pub fn label(&self) -> String {
        format!("{}.{}", self.package, self.graph)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImportRef {
    pub package: String,
    pub graph: String,
    pub port: String,
}

impl ImportRef {
    pub fn owner(&self) -> GraphRef {
        GraphRef {
            package: self.package.clone(),
            graph: self.graph.clone(),
        }
    }
}

/// Parameters and outcomes; checked signatures keep outcomes sorted and unique.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Signature {
    #[serde(default)]
    pub parameters: BTreeMap<String, ScalarType>,
    pub outcomes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CallTarget {
    /// A graph of the same package instance.
    Local { graph: String },
    /// An import port of the calling graph, bound by the product.
    Import { port: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Manifest {
    pub product: ProductHeader,
    pub packages: BTreeMap<String, PackageInstance>,
    pub graphs: BTreeMap<GraphRef, GraphEntry>,
    pub chunks: Vec<ObjectId>,
    pub node_types: BTreeMap<String, String>,
    pub tombstones: ObjectId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductHeader {
    pub id: String,
    pub title: Option<ContentRef>,
    pub entry: GraphRef,
    pub arguments: BTreeMap<String, Scalar>,
    pub shared: BTreeMap<String, SharedVariable>,
    pub endings: BTreeMap<String, Ending>,
    pub bindings: BTreeMap<ImportRef, GraphRef>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SharedVariable {
    pub value: Scalar,
    pub label: Option<ContentRef>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Ending {
    pub title: Option<ContentRef>,
    pub body: Option<Segment>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageInstance {
    pub id: String,
    pub version: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphEntry {
    pub signature: Signature,
    pub exported: bool,
    pub chunk: u32,
}

/// One or more graphs of one package instance. Calls cross chunks only through manifest
/// signatures and branches stay inside a graph, so a chunk is checked against the manifest
/// alone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Chunk {
    pub graphs: BTreeMap<GraphRef, Graph>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Graph {
    pub header: GraphHeader,
    pub nodes: BTreeMap<NodeId, Plan>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphHeader {
    pub title: Option<ContentRef>,
    pub parameters: BTreeMap<String, ScalarType>,
    pub locals: BTreeMap<String, Scalar>,
    pub shared: BTreeMap<String, ScalarType>,
    pub imports: BTreeMap<String, Signature>,
    pub outcomes: Vec<String>,
    pub entry: NodeId,
}

impl GraphHeader {
    pub(crate) fn declarations(&self) -> crate::expr::Declarations<'_> {
        crate::expr::Declarations {
            parameters: &self.parameters,
            locals: &self.locals,
            shared: &self.shared,
        }
    }

    pub fn signature(&self) -> Signature {
        Signature {
            parameters: self.parameters.clone(),
            outcomes: self.outcomes.clone(),
        }
    }
}

/// The checked execution vocabulary (ADR 0013 §4).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Plan {
    Passage(Passage),
    Branch {
        condition: Expr,
        when_true: NodeId,
        when_false: NodeId,
    },
    Mutate {
        assignments: Vec<Assignment>,
        next: NodeId,
    },
    Call {
        target: CallTarget,
        arguments: BTreeMap<String, Expr>,
        on_return: BTreeMap<String, NodeId>,
    },
    Return {
        outcome: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Passage {
    pub title: Option<ContentRef>,
    pub body: Option<Segment>,
    pub args: BTreeMap<String, Expr>,
    pub choice_points: Vec<ChoicePoint>,
    pub next: Option<NodeId>,
}

impl Passage {
    /// Whether the passage can run to its end: it has no choice point, or its last choice
    /// point has a local outcome (or allows choosing nothing).
    pub fn reaches_end(&self) -> bool {
        self.choice_points.last().is_none_or(|point| {
            point.min == 0
                || point
                    .options
                    .iter()
                    .any(|option| matches!(option.outcome, Outcome::Local { .. }))
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ChoicePoint {
    pub id: ChoicePointId,
    pub placement: Option<AnchorId>,
    pub min: u16,
    pub max: u16,
    pub options: Vec<OptionPlan>,
    /// Whether the host may append options here and propose passages (ADR 0013 §5).
    pub proposals: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OptionPlan {
    pub id: OptionId,
    pub label: Option<ContentRef>,
    pub visible_if: Option<Expr>,
    pub enabled_if: Option<Expr>,
    pub reason: Option<ContentRef>,
    pub effects: Vec<Assignment>,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    Local {
        reply: Option<Segment>,
        rejoin: Option<AnchorId>,
    },
    Branch {
        target: NodeId,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TombstoneSet {
    pub nodes: BTreeSet<NodeId>,
    pub choice_points: BTreeSet<ChoicePointId>,
    pub options: BTreeSet<OptionId>,
}

impl TombstoneSet {
    pub fn contains(&self, id: &crate::AuthoredId) -> bool {
        match id {
            crate::AuthoredId::Node(id) => self.nodes.contains(id),
            crate::AuthoredId::ChoicePoint(id) => self.choice_points.contains(id),
            crate::AuthoredId::Option(id) => self.options.contains(id),
        }
    }

    pub fn insert(&mut self, id: crate::AuthoredId) -> bool {
        match id {
            crate::AuthoredId::Node(id) => self.nodes.insert(id),
            crate::AuthoredId::ChoicePoint(id) => self.choice_points.insert(id),
            crate::AuthoredId::Option(id) => self.options.insert(id),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = crate::AuthoredId> + '_ {
        self.nodes
            .iter()
            .copied()
            .map(crate::AuthoredId::Node)
            .chain(
                self.choice_points
                    .iter()
                    .copied()
                    .map(crate::AuthoredId::ChoicePoint),
            )
            .chain(self.options.iter().copied().map(crate::AuthoredId::Option))
    }

    pub fn is_superset(&self, other: &Self) -> bool {
        self.nodes.is_superset(&other.nodes)
            && self.choice_points.is_superset(&other.choice_points)
            && self.options.is_superset(&other.options)
    }
}

/// Author aliases. They are distributed with an artifact but are not part of its identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NameTable {
    pub artifact: ArtifactId,
    pub migrated_from_r1: Option<R1ArtifactId>,
    pub graphs: BTreeMap<GraphRef, GraphNames>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GraphNames {
    pub nodes: BTreeMap<NodeId, String>,
    pub choice_points: BTreeMap<ChoicePointId, String>,
    pub options: BTreeMap<OptionId, String>,
}

impl NameTable {
    pub fn node(&self, graph: &GraphRef, id: &NodeId) -> Option<&str> {
        self.graphs.get(graph)?.nodes.get(id).map(String::as_str)
    }

    pub fn choice_point(&self, graph: &GraphRef, id: &ChoicePointId) -> Option<&str> {
        self.graphs
            .get(graph)?
            .choice_points
            .get(id)
            .map(String::as_str)
    }

    pub fn option(&self, graph: &GraphRef, id: &OptionId) -> Option<&str> {
        self.graphs.get(graph)?.options.get(id).map(String::as_str)
    }

    pub fn node_id(&self, graph: &GraphRef, alias: &str) -> Option<NodeId> {
        self.graphs
            .get(graph)?
            .nodes
            .iter()
            .find_map(|(id, name)| (name == alias).then_some(*id))
    }
}

/// Renders node locations in errors: aliases when a name table is known, IDs otherwise.
pub(crate) fn node_path(names: Option<&NameTable>, graph: &GraphRef, node: &NodeId) -> String {
    match names.and_then(|table| table.node(graph, node)) {
        Some(alias) => format!(
            "packages.{}.graphs.{}.nodes.{alias}",
            graph.package, graph.graph
        ),
        None => format!("{}/{node}", graph.label()),
    }
}
