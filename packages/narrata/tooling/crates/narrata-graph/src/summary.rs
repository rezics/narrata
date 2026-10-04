use std::collections::BTreeSet;

use crate::{Analysis, ClusterId, EndingClass, Id, model::CheckedGraph};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BottleneckRun {
    pub cluster: ClusterId,
    pub first: Id,
    pub last: Id,
    pub node_count: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EndingSummary {
    pub node: Id,
    pub class: EndingClass,
    pub reachable: bool,
    /// Clusters containing a reachable ancestor of this ending. These are a
    /// structural route approximation, not a proof that conditions are feasible.
    pub clusters: Vec<ClusterId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticSummary {
    pub entries: Vec<Id>,
    pub endings: Vec<EndingSummary>,
    /// Common bottleneck sequence compacted into consecutive authored-cluster
    /// runs. Exact interior IDs remain in `Analysis::bottlenecks`.
    pub bottleneck_runs: Vec<BottleneckRun>,
    // Entity introductions deliberately have no inferred representation here:
    // they require a host-independent entity model before becoming facts.
}

pub(crate) fn summarize(graph: &CheckedGraph, analysis: &Analysis) -> SemanticSummary {
    let mut endings = Vec::new();
    // Epoch marks reuse the traversal allocation across ending routes.
    let mut visited = vec![0_usize; graph.nodes.len()];
    for (ordinal, (&ending, &class)) in graph.endings.iter().enumerate() {
        let mut clusters = BTreeSet::new();
        let mut pending = vec![ending];
        while let Some(node) = pending.pop() {
            if visited[node] != ordinal + 1 && analysis.rows[node].reachable {
                visited[node] = ordinal + 1;
                clusters.insert(graph.nodes[node].cluster);
                pending.extend(&graph.predecessors[node]);
            }
        }
        endings.push(EndingSummary {
            node: graph.nodes[ending].id,
            class,
            reachable: analysis.rows[ending].reachable,
            clusters: clusters.into_iter().collect(),
        });
    }
    let mut bottleneck_runs: Vec<BottleneckRun> = Vec::new();
    for &index in &analysis.bottlenecks {
        let node = &graph.nodes[index as usize];
        if let Some(last) = bottleneck_runs.last_mut()
            && last.cluster == node.cluster
        {
            last.last = node.id;
            last.node_count += 1;
        } else {
            bottleneck_runs.push(BottleneckRun {
                cluster: node.cluster,
                first: node.id,
                last: node.id,
                node_count: 1,
            });
        }
    }
    SemanticSummary {
        entries: graph
            .entries
            .iter()
            .map(|entry| graph.nodes[*entry].id)
            .collect(),
        endings,
        bottleneck_runs,
    }
}
