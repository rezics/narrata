//! Text-free structural analysis for graph inspectors: nodes, edges and structural
//! reachability. Labels are content references and aliases, resolved by the host.

use std::collections::{BTreeMap, BTreeSet};

use narrata_kernel::content::ContentRef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    ArtifactId, Diagnostic, NodeId, Program, Result,
    plan::{GraphRef, NameTable, Outcome, Plan},
};

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Analysis {
    pub artifact_id: ArtifactId,
    pub graphs: Vec<GraphAnalysis>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphAnalysis {
    pub reference: GraphRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<ContentRef>,
    pub entry: NodeId,
    pub exported: bool,
    pub nodes: Vec<NodeAnalysis>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Passage,
    Branch,
    Mutate,
    Call,
    Return,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeAnalysis {
    pub id: NodeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub kind: NodeKind,
    /// A passage's title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<ContentRef>,
    /// The graph a call runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callee: Option<GraphRef>,
    /// The outcome a return reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    pub edges: Vec<EdgeAnalysis>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Next,
    Choice,
    Condition,
    Return,
    Call,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeAnalysis {
    pub kind: EdgeKind,
    /// An option alias, an outcome name, or `true`/`false` for conditions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// An option label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<ContentRef>,
    pub target: NodeAddress,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeAddress {
    pub package: String,
    pub graph: String,
    pub node: NodeId,
}

/// Loads every chunk. Unreachable nodes are reported structurally; conditions are not solved.
pub fn analyze(program: &Program, names: Option<&NameTable>) -> Result<Analysis> {
    let mut graphs = Vec::new();
    let mut diagnostics = Vec::new();
    for (key, entry) in &program.manifest().graphs {
        let graph = program.graph(key)?;
        let address = |node: &NodeId| NodeAddress {
            package: key.package.clone(),
            graph: key.graph.clone(),
            node: *node,
        };
        let mut nodes = Vec::new();
        for (id, plan) in &graph.nodes {
            let mut node = NodeAnalysis {
                id: *id,
                key: names.and_then(|names| names.node(key, id).map(str::to_owned)),
                kind: NodeKind::Passage,
                title: None,
                callee: None,
                outcome: None,
                edges: Vec::new(),
            };
            let edge = |kind, key: Option<String>, label: Option<ContentRef>, target: &NodeId| {
                EdgeAnalysis {
                    kind,
                    key,
                    label,
                    target: address(target),
                }
            };
            match plan {
                Plan::Passage(passage) => {
                    node.title = passage.title.clone();
                    for point in &passage.choice_points {
                        for option in &point.options {
                            if let Outcome::Branch { target } = &option.outcome {
                                node.edges.push(edge(
                                    EdgeKind::Choice,
                                    names.and_then(|names| {
                                        names.option(key, &option.id).map(str::to_owned)
                                    }),
                                    option.label.clone(),
                                    target,
                                ));
                            }
                        }
                    }
                    if let Some(next) = &passage.next {
                        node.edges.push(edge(EdgeKind::Next, None, None, next));
                    }
                }
                Plan::Branch {
                    when_true,
                    when_false,
                    ..
                } => {
                    node.kind = NodeKind::Branch;
                    node.edges.push(edge(
                        EdgeKind::Condition,
                        Some("true".into()),
                        None,
                        when_true,
                    ));
                    node.edges.push(edge(
                        EdgeKind::Condition,
                        Some("false".into()),
                        None,
                        when_false,
                    ));
                }
                Plan::Mutate { next, .. } => {
                    node.kind = NodeKind::Mutate;
                    node.edges.push(edge(EdgeKind::Next, None, None, next));
                }
                Plan::Call {
                    target, on_return, ..
                } => {
                    node.kind = NodeKind::Call;
                    let callee = program.resolve_call(key, target)?;
                    let callee_entry = program.graph(&callee)?.header.entry;
                    for (outcome, target) in on_return {
                        node.edges.push(edge(
                            EdgeKind::Return,
                            Some(outcome.clone()),
                            None,
                            target,
                        ));
                    }
                    node.edges.push(EdgeAnalysis {
                        kind: EdgeKind::Call,
                        key: None,
                        label: None,
                        target: NodeAddress {
                            package: callee.package.clone(),
                            graph: callee.graph.clone(),
                            node: callee_entry,
                        },
                    });
                    node.callee = Some(callee);
                }
                Plan::Return { outcome } => {
                    node.kind = NodeKind::Return;
                    node.outcome = Some(outcome.clone());
                }
            }
            nodes.push(node);
        }
        let mut reachable = BTreeSet::new();
        let by_id: BTreeMap<_, _> = nodes.iter().map(|node| (node.id, node)).collect();
        let mut queue = vec![graph.header.entry];
        while let Some(id) = queue.pop() {
            if !reachable.insert(id) {
                continue;
            }
            if let Some(node) = by_id.get(&id) {
                queue.extend(
                    node.edges
                        .iter()
                        .filter(|edge| edge.kind != EdgeKind::Call)
                        .map(|edge| edge.target.node),
                );
            }
        }
        for node in nodes.iter().filter(|node| !reachable.contains(&node.id)) {
            diagnostics.push(Diagnostic {
                code: "structurally_unreachable".into(),
                path: crate::plan::node_path(names, key, &node.id),
                message: "No structural path from this graph's entry (conditions are not solved)."
                    .into(),
            });
        }
        graphs.push(GraphAnalysis {
            reference: key.clone(),
            title: graph.header.title.clone(),
            entry: graph.header.entry,
            exported: entry.exported,
            nodes,
        });
    }
    Ok(Analysis {
        artifact_id: program.artifact_id(),
        graphs,
        diagnostics,
    })
}
