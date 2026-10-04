use narrata_graph::{ClusterId, Edge, EdgeKind, Ending, EndingClass, Graph, Id, Node, NodeKind};

/// Small compatibility graph: loops, unreachable chains, a reachable dead end,
/// classified endings, parallel edge kinds, and branches crossing cluster frames.
pub fn compatibility_graph() -> Graph {
    let id = |n| Id::from_u128(n);
    let mut graph = Graph {
        nodes: (0..36)
            .map(|n| Node {
                id: id(n),
                cluster: ClusterId::from_u128(n / 6),
                kind: match n {
                    3 | 8 | 24 => NodeKind::Branch,
                    14 => NodeKind::State,
                    16 => NodeKind::Call,
                    17 => NodeKind::Return,
                    25 | 27 | 35 => NodeKind::Ending,
                    _ => NodeKind::Passage,
                },
                author_order: if n % 2 == 0 {
                    Some(n as u64 * 1000)
                } else {
                    None
                },
            })
            .collect(),
        entries: vec![id(0)],
        endings: vec![
            Ending {
                node: id(25),
                class: EndingClass::Failure,
            },
            Ending {
                node: id(27),
                class: EndingClass::Success,
            },
            Ending {
                node: id(35),
                class: EndingClass::Neutral,
            },
        ],
        edges: (0..25)
            .map(|n| Edge {
                source: id(n),
                target: id(n + 1),
                kind: EdgeKind::Next,
            })
            .collect(),
    };
    for (source, target, kind) in [
        (3, 12, EdgeKind::Choice),
        (8, 10, EdgeKind::Choice),
        (10, 8, EdgeKind::Conditional),
        (16, 17, EdgeKind::Call),
        (17, 18, EdgeKind::Return),
        (17, 30, EdgeKind::Choice),
        (24, 26, EdgeKind::Choice),
        (26, 27, EdgeKind::Next),
        (28, 29, EdgeKind::Next),
        (30, 31, EdgeKind::Next),
        (32, 33, EdgeKind::Next),
        (33, 34, EdgeKind::Next),
        (34, 35, EdgeKind::Next),
    ] {
        graph.edges.push(Edge {
            source: id(source),
            target: id(target),
            kind,
        });
    }
    graph
}
