use crate::model::CheckedGraph;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeAnalysis {
    pub reachable: bool,
    pub dead_end: bool,
    pub ending: bool,
    pub cyclic: bool,
    /// Components are ordered by their smallest canonical node index.
    pub component: u32,
    /// Immediate dominator from a virtual root connected to every entry.
    /// `None` means unreachable or dominated only by that virtual root.
    pub immediate_dominator: Option<u32>,
    /// On every finite entry-to-reachable-ending path. With no reachable ending,
    /// no node is a bottleneck (rather than using vacuous truth).
    pub bottleneck: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Analysis {
    pub rows: Vec<NodeAnalysis>,
    /// DFS ancestor edges, including self-loops. Layout may need additional
    /// feedback edges when authored clusters themselves form a cycle.
    pub back_edges: Vec<bool>,
    pub components: Vec<Vec<u32>>,
    /// Root-to-ending order of the common dominator chain.
    pub bottlenecks: Vec<u32>,
}

pub(crate) fn analyze(graph: &CheckedGraph) -> Analysis {
    let count = graph.nodes.len();
    let mut reachable = vec![false; count];
    let mut pending = graph.entries.clone();
    while let Some(node) = pending.pop() {
        if !reachable[node] {
            reachable[node] = true;
            pending.extend(&graph.successors[node]);
        }
    }
    let components = components(&graph.successors, &graph.predecessors);
    let mut component_ids = vec![0; count];
    let mut cyclic = vec![false; count];
    for (index, component) in components.iter().enumerate() {
        let is_cycle = component.len() > 1
            || component
                .first()
                .is_some_and(|node| graph.successors[*node].binary_search(node).is_ok());
        for &node in component {
            component_ids[node] = index as u32;
            cyclic[node] = is_cycle;
        }
    }
    let feedback = feedback(&graph.successors);
    let back_edges = graph
        .edges
        .iter()
        .map(|edge| {
            feedback[edge.source as usize]
                .binary_search(&(edge.target as usize))
                .is_ok()
        })
        .collect();

    let root = count;
    let sink = count + 1;
    let mut successors = graph.successors.clone();
    let mut predecessors = graph.predecessors.clone();
    successors.push(graph.entries.clone());
    successors.push(Vec::new());
    predecessors.push(Vec::new());
    predecessors.push(Vec::new());
    for &entry in &graph.entries {
        predecessors[entry].push(root);
    }
    for &ending in graph.endings.keys() {
        if reachable[ending] {
            successors[ending].push(sink);
            predecessors[sink].push(ending);
        }
    }
    let dominators = dominators(&successors, &predecessors, root);
    let mut bottlenecks = Vec::new();
    let mut cursor = dominators[sink];
    while let Some(node) = cursor {
        if node == root {
            break;
        }
        bottlenecks.push(node as u32);
        cursor = dominators[node];
    }
    bottlenecks.reverse();
    let mut is_bottleneck = vec![false; count];
    for &node in &bottlenecks {
        is_bottleneck[node as usize] = true;
    }
    let rows = (0..count)
        .map(|node| NodeAnalysis {
            reachable: reachable[node],
            dead_end: !graph.endings.contains_key(&node) && graph.successors[node].is_empty(),
            ending: graph.endings.contains_key(&node),
            cyclic: cyclic[node],
            component: component_ids[node],
            immediate_dominator: dominators[node]
                .filter(|parent| *parent != root)
                .map(|parent| parent as u32),
            bottleneck: is_bottleneck[node],
        })
        .collect();
    Analysis {
        rows,
        back_edges,
        components: components
            .into_iter()
            .map(|component| component.into_iter().map(|node| node as u32).collect())
            .collect(),
        bottlenecks,
    }
}

/// Iterative DFS avoids call-stack growth on a 100k-node passage chain.
fn finish_order(successors: &[Vec<usize>]) -> Vec<usize> {
    let mut visited = vec![false; successors.len()];
    let mut finished = Vec::with_capacity(successors.len());
    for start in 0..successors.len() {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut stack = vec![(start, 0)];
        while let Some((node, next)) = stack.last_mut() {
            if *next == successors[*node].len() {
                finished.push(*node);
                stack.pop();
            } else {
                let child = successors[*node][*next];
                *next += 1;
                if !visited[child] {
                    visited[child] = true;
                    stack.push((child, 0));
                }
            }
        }
    }
    finished
}

pub(crate) fn components(
    successors: &[Vec<usize>],
    predecessors: &[Vec<usize>],
) -> Vec<Vec<usize>> {
    let mut assigned = vec![false; successors.len()];
    let mut components = Vec::new();
    for start in finish_order(successors).into_iter().rev() {
        if assigned[start] {
            continue;
        }
        let mut component = Vec::new();
        let mut stack = vec![start];
        assigned[start] = true;
        while let Some(node) = stack.pop() {
            component.push(node);
            for &parent in &predecessors[node] {
                if !assigned[parent] {
                    assigned[parent] = true;
                    stack.push(parent);
                }
            }
        }
        component.sort_unstable();
        components.push(component);
    }
    components.sort_by_key(|component| component[0]);
    components
}

pub(crate) fn feedback(successors: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let mut color = vec![0_u8; successors.len()];
    let mut feedback = vec![Vec::new(); successors.len()];
    for start in 0..successors.len() {
        if color[start] != 0 {
            continue;
        }
        color[start] = 1;
        let mut stack = vec![(start, 0)];
        while let Some((node, next)) = stack.last_mut() {
            if *next == successors[*node].len() {
                color[*node] = 2;
                stack.pop();
            } else {
                let child = successors[*node][*next];
                *next += 1;
                match color[child] {
                    0 => {
                        color[child] = 1;
                        stack.push((child, 0));
                    }
                    1 => feedback[*node].push(child),
                    _ => {}
                }
            }
        }
    }
    for edges in &mut feedback {
        edges.sort_unstable();
    }
    feedback
}

/// Lengauer–Tarjan with iterative path compression. A set-of-dominators per
/// node would require quadratic memory on long narrative chains.
fn dominators(
    successors: &[Vec<usize>],
    predecessors: &[Vec<usize>],
    root: usize,
) -> Vec<Option<usize>> {
    let count = successors.len();
    let absent = usize::MAX;
    let mut number = vec![absent; count];
    let mut vertex = vec![root];
    let mut parent = vec![absent; count];
    number[root] = 0;
    let mut stack = vec![(root, 0)];
    while let Some((node, next)) = stack.last_mut() {
        if *next == successors[*node].len() {
            stack.pop();
        } else {
            let child = successors[*node][*next];
            *next += 1;
            if number[child] == absent {
                parent[child] = *node;
                number[child] = vertex.len();
                vertex.push(child);
                stack.push((child, 0));
            }
        }
    }
    let mut semi = number;
    let mut ancestor = vec![absent; count];
    let mut label: Vec<_> = (0..count).collect();
    let mut buckets = vec![Vec::new(); count];
    let mut idom = vec![None; count];
    let mut path = Vec::new();
    for &node in vertex.iter().skip(1).rev() {
        for &predecessor in &predecessors[node] {
            if semi[predecessor] != absent {
                let candidate = evaluate(predecessor, &mut ancestor, &mut label, &semi, &mut path);
                semi[node] = semi[node].min(semi[candidate]);
            }
        }
        buckets[vertex[semi[node]]].push(node);
        let parent_node = parent[node];
        ancestor[node] = parent_node;
        for pending in std::mem::take(&mut buckets[parent_node]) {
            let candidate = evaluate(pending, &mut ancestor, &mut label, &semi, &mut path);
            idom[pending] = Some(if semi[candidate] < semi[pending] {
                candidate
            } else {
                parent_node
            });
        }
    }
    for &node in vertex.iter().skip(1) {
        if let Some(dominator) = idom[node]
            && dominator != vertex[semi[node]]
        {
            idom[node] = idom[dominator];
        }
    }
    idom
}

fn evaluate(
    node: usize,
    ancestor: &mut [usize],
    label: &mut [usize],
    semi: &[usize],
    path: &mut Vec<usize>,
) -> usize {
    let absent = usize::MAX;
    path.clear();
    let mut cursor = node;
    while ancestor[cursor] != absent && ancestor[ancestor[cursor]] != absent {
        path.push(cursor);
        cursor = ancestor[cursor];
    }
    for &cursor in path.iter().rev() {
        let parent = ancestor[cursor];
        if semi[label[parent]] < semi[label[cursor]] {
            label[cursor] = label[parent];
        }
        ancestor[cursor] = ancestor[parent];
    }
    label[node]
}
