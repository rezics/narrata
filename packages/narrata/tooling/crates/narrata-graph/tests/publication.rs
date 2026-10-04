use std::collections::{BTreeMap, BTreeSet};

use narrata_graph::{
    CLUSTER_NODE_LIMIT, ClusterId, CutReason, DetailLevel, Edge, EdgeKind, Ending, EndingClass,
    Graph, GraphError, Id, Node, NodeKind, Publication, TILE_ELEMENT_LIMIT, prepare,
};

fn id(value: usize) -> Id {
    Id::from_u128(value as u128)
}

fn node(value: usize, cluster: usize, kind: NodeKind) -> Node {
    Node {
        id: id(value),
        kind,
        cluster: ClusterId::from_u128(cluster as u128),
        author_order: None,
    }
}

fn edge(source: usize, target: usize) -> Edge {
    Edge {
        source: id(source),
        target: id(target),
        kind: EdgeKind::Next,
    }
}

fn chain(count: usize, cluster_size: usize) -> Graph {
    Graph {
        nodes: (0..count)
            .map(|index| node(index, index / cluster_size, NodeKind::Passage))
            .collect(),
        edges: (1..count).map(|index| edge(index - 1, index)).collect(),
        entries: if count == 0 { Vec::new() } else { vec![id(0)] },
        endings: if count == 0 {
            Vec::new()
        } else {
            vec![Ending {
                node: id(count - 1),
                class: EndingClass::Success,
            }]
        },
    }
}

fn check_geometry(publication: &Publication) {
    let mut occupied = BTreeSet::new();
    for position in &publication.layout.positions {
        assert!(occupied.insert((position.x, position.y)), "nodes overlap");
        assert_eq!(position.local_x, position.order as i32 * 32);
        assert_eq!(position.local_y, position.layer as i32 * 64);
    }
    for (index, edge) in publication.edges.iter().enumerate() {
        if !publication.layout.feedback_edges[index] {
            assert!(
                publication.layout.positions[edge.source as usize].y
                    < publication.layout.positions[edge.target as usize].y
            );
        }
    }
}

fn check_tiles(publication: &Publication) {
    let node_indices: BTreeMap<_, _> = publication
        .nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.id.0, i))
        .collect();
    let mut seen_nodes = BTreeSet::new();
    let mut seen_edges = BTreeMap::new();
    for tile in &publication.tiles {
        let columns = &tile.columns;
        let nodes = columns.ids.len();
        let edges = columns.sources.len();
        assert_eq!(columns.x.len(), nodes);
        assert_eq!(columns.y.len(), nodes);
        assert_eq!(columns.clusters.len(), nodes);
        assert_eq!(columns.kinds.len(), nodes);
        assert_eq!(columns.importance.len(), nodes);
        assert_eq!(columns.node_flags.len(), nodes);
        assert_eq!(columns.author_order.len(), nodes);
        assert_eq!(columns.has_author_order.len(), nodes);
        assert_eq!(columns.targets.len(), edges);
        assert_eq!(columns.edge_kind_counts.len(), edges);
        assert_eq!(columns.edge_kind_masks.len(), edges);
        assert_eq!(columns.edge_flags.len(), edges);
        assert_eq!(columns.edge_importance.len(), edges);
        for (&source, &target) in columns.sources.iter().zip(&columns.targets) {
            assert!((source as usize) < nodes);
            assert!((target as usize) < nodes);
        }
        if tile.level != DetailLevel::Node {
            continue;
        }
        assert!(columns.element_count() <= TILE_ELEMENT_LIMIT);
        assert!(columns.element_count() > 0);
        for (row, &node_id) in columns.ids.iter().enumerate() {
            seen_nodes.insert(node_id);
            let index = node_indices[&node_id];
            let position = &publication.layout.positions[index];
            assert_eq!(columns.x[row], position.local_x);
            assert_eq!(columns.y[row], position.local_y);
            assert_eq!(columns.clusters[row], position.cluster);
        }
        for index in 0..edges {
            let source = columns.ids[columns.sources[index] as usize];
            let target = columns.ids[columns.targets[index] as usize];
            assert!(
                seen_edges
                    .insert((source, target), columns.edge_kind_counts[index])
                    .is_none()
            );
        }
    }
    assert_eq!(seen_nodes.len(), publication.nodes.len());
    assert_eq!(seen_edges.len(), publication.edges.len());
    for edge in &publication.edges {
        assert_eq!(
            seen_edges[&(
                publication.nodes[edge.source as usize].id.0,
                publication.nodes[edge.target as usize].id.0
            )],
            edge.kind_counts
        );
    }
}

#[test]
fn diagnostics_cover_parallel_kinds_cycles_dead_ends_and_dominators() -> Result<(), GraphError> {
    let mut graph = Graph {
        nodes: (1..=10)
            .map(|index| node(index, 1, NodeKind::Passage))
            .collect(),
        edges: vec![
            edge(1, 2),
            edge(2, 3),
            edge(2, 4),
            edge(3, 5),
            edge(4, 5),
            edge(5, 2),
            edge(5, 6),
            edge(2, 7),
            edge(9, 9),
        ],
        entries: vec![id(1)],
        endings: vec![Ending {
            node: id(6),
            class: EndingClass::Success,
        }],
    };
    graph.nodes[9].kind = NodeKind::Ending;
    graph.edges.push(Edge {
        source: id(2),
        target: id(3),
        kind: EdgeKind::Choice,
    });
    graph.edges.push(Edge {
        source: id(2),
        target: id(3),
        kind: EdgeKind::Choice,
    });
    graph.edges.push(Edge {
        source: id(2),
        target: id(3),
        kind: EdgeKind::Conditional,
    });
    graph.edges.push(Edge {
        source: id(2),
        target: id(3),
        kind: EdgeKind::Call,
    });
    graph.edges.push(Edge {
        source: id(2),
        target: id(3),
        kind: EdgeKind::Return,
    });
    let publication = prepare(graph)?;
    let rows = &publication.analysis.rows;
    assert!(rows[..7].iter().all(|row| row.reachable));
    assert!(rows[7..].iter().all(|row| !row.reachable));
    assert!(rows[6].dead_end && rows[7].dead_end);
    assert!(!rows[5].dead_end && !rows[9].dead_end);
    assert!(rows[1..5].iter().all(|row| row.cyclic));
    assert!(rows[8].cyclic);
    assert!(!rows[0].cyclic);
    assert_eq!(rows[4].immediate_dominator, Some(1));
    assert_eq!(rows[5].immediate_dominator, Some(4));
    assert_eq!(publication.analysis.bottlenecks, vec![0, 1, 4, 5]);
    let parallel = publication
        .edges
        .iter()
        .find(|edge| edge.source == 1 && edge.target == 2);
    assert_eq!(parallel.map(|edge| edge.kind_counts), Some([2, 1, 1, 1, 1]));
    assert_eq!(parallel.map(|edge| edge.multiplicity()), Some(6));
    assert_eq!(parallel.map(|edge| edge.kind_mask()), Some(31));
    assert_eq!(
        publication
            .analysis
            .back_edges
            .iter()
            .filter(|back| **back)
            .count(),
        2
    );
    assert_eq!(publication.summary.endings.len(), 2);
    assert!(publication.summary.endings[0].reachable);
    assert!(!publication.summary.endings[1].reachable);
    assert!(publication.summary.endings[1].clusters.is_empty());
    check_geometry(&publication);
    check_tiles(&publication);
    Ok(())
}

#[test]
fn multiple_entries_endings_and_no_reachable_ending_have_explicit_semantics()
-> Result<(), GraphError> {
    let mut graph = Graph {
        nodes: (0..6)
            .map(|index| node(index, index, NodeKind::Passage))
            .collect(),
        edges: vec![edge(0, 2), edge(1, 2), edge(2, 3), edge(2, 4)],
        entries: vec![id(0), id(1)],
        endings: vec![
            Ending {
                node: id(3),
                class: EndingClass::Success,
            },
            Ending {
                node: id(4),
                class: EndingClass::Failure,
            },
            Ending {
                node: id(5),
                class: EndingClass::Neutral,
            },
        ],
    };
    let publication = prepare(graph.clone())?;
    assert_eq!(publication.analysis.rows[2].immediate_dominator, None);
    assert_eq!(publication.analysis.bottlenecks, vec![2]);
    assert_eq!(
        publication.summary.endings[0].clusters,
        (0..4).map(ClusterId::from_u128).collect::<Vec<_>>()
    );
    assert_eq!(
        publication.summary.endings[1].clusters,
        [0, 1, 2, 4].map(ClusterId::from_u128)
    );
    graph.endings = vec![Ending {
        node: id(5),
        class: EndingClass::Neutral,
    }];
    assert!(prepare(graph.clone())?.analysis.bottlenecks.is_empty());
    graph.entries.clear();
    assert!(
        prepare(graph)?
            .analysis
            .rows
            .iter()
            .all(|row| !row.reachable && !row.bottleneck)
    );
    assert!(prepare(Graph::default())?.analysis.rows.is_empty());
    Ok(())
}

#[test]
fn checked_boundary_rejects_duplicates_and_dangling_references() {
    let mut graph = chain(2, 2);
    graph.nodes.push(graph.nodes[0].clone());
    assert_eq!(prepare(graph).err(), Some(GraphError::DuplicateNode(id(0))));
    let mut graph = chain(2, 2);
    graph.edges.push(edge(0, 999));
    assert_eq!(prepare(graph).err(), Some(GraphError::UnknownNode(id(999))));
    let mut graph = chain(2, 2);
    graph.entries.push(id(999));
    assert_eq!(prepare(graph).err(), Some(GraphError::UnknownNode(id(999))));
    let mut graph = chain(2, 2);
    graph.entries.push(id(0));
    assert_eq!(
        prepare(graph).err(),
        Some(GraphError::DuplicateEntry(id(0)))
    );
    let mut graph = chain(2, 2);
    graph.endings.push(graph.endings[0]);
    assert_eq!(
        prepare(graph).err(),
        Some(GraphError::DuplicateEnding(id(1)))
    );
    let mut graph = chain(2, 2);
    graph.endings[0].node = id(999);
    assert_eq!(prepare(graph).err(), Some(GraphError::UnknownNode(id(999))));
}

#[test]
fn internal_edit_preserves_every_other_cluster_position_even_with_meta_cycles()
-> Result<(), GraphError> {
    let mut graph = chain(60, 20);
    graph.edges.extend([edge(49, 8), edge(7, 40)]);
    // Leave ID gaps so inserted nodes also shift canonical row indices.
    for node in &mut graph.nodes {
        node.id = Id::from_u128(u128::from_be_bytes(node.id.0) * 10);
    }
    for edge in &mut graph.edges {
        edge.source = Id::from_u128(u128::from_be_bytes(edge.source.0) * 10);
        edge.target = Id::from_u128(u128::from_be_bytes(edge.target.0) * 10);
    }
    for entry in &mut graph.entries {
        *entry = Id::from_u128(u128::from_be_bytes(entry.0) * 10);
    }
    for ending in &mut graph.endings {
        ending.node = Id::from_u128(u128::from_be_bytes(ending.node.0) * 10);
    }
    let before = prepare(graph.clone())?;
    graph.nodes.push(node(205, 1, NodeKind::Branch));
    graph.nodes.push(node(215, 1, NodeKind::Call));
    graph.edges.extend([
        edge(200, 205),
        edge(205, 215),
        edge(215, 370),
        edge(215, 230),
    ]);
    graph.nodes[25].author_order = Some(7);
    let after = prepare(graph)?;
    let after_positions: BTreeMap<_, _> = after
        .nodes
        .iter()
        .zip(&after.layout.positions)
        .map(|(node, position)| (node.id, position))
        .collect();
    for (index, node) in before.nodes.iter().enumerate() {
        if node.cluster != ClusterId::from_u128(1) {
            assert_eq!(&before.layout.positions[index], after_positions[&node.id]);
        }
    }
    assert_eq!(before.layout.clusters, after.layout.clusters);
    check_geometry(&after);
    check_tiles(&after);
    Ok(())
}

#[test]
fn parallel_external_edges_and_author_order_do_not_move_other_clusters() -> Result<(), GraphError> {
    let mut graph = chain(20, 5);
    let before = prepare(graph.clone())?;
    graph.edges.extend([edge(4, 5), edge(4, 5)]);
    graph.nodes[2].author_order = Some(u64::MAX);
    let after = prepare(graph)?;
    assert_eq!(before.layout.clusters, after.layout.clusters);
    assert_eq!(before.layout.positions[5..], after.layout.positions[5..]);
    let work = &after.tiles[0].columns;
    assert_eq!(work.edge_kind_counts[0], [0, 3, 0, 0, 0]);
    Ok(())
}

#[test]
fn input_permutations_produce_identical_analysis_layout_tiles_and_summary() -> Result<(), GraphError>
{
    let mut graph = chain(100, 20);
    graph.entries.push(id(70));
    graph.endings.push(Ending {
        node: id(45),
        class: EndingClass::Neutral,
    });
    graph
        .edges
        .extend([edge(50, 30), edge(12, 37), edge(22, 28)]);
    graph.nodes[30].author_order = Some(10);
    let before = prepare(graph.clone())?;
    graph.nodes.reverse();
    graph.edges.reverse();
    graph.entries.reverse();
    graph.endings.reverse();
    assert_eq!(before, prepare(graph.clone())?);
    graph.nodes.rotate_left(31);
    graph.edges.rotate_left(73);
    assert_eq!(before, prepare(graph)?);
    Ok(())
}

#[test]
fn high_degree_cross_cluster_tiles_preserve_every_edge_and_ghost_endpoint() -> Result<(), GraphError>
{
    let mut graph = chain(6000, 3000);
    graph.edges.extend((2..6000).map(|target| edge(0, target)));
    let publication = prepare(graph)?;
    assert!(
        publication
            .tiles
            .iter()
            .filter(|tile| tile.level == DetailLevel::Node)
            .count()
            > 6
    );
    assert!(
        publication.tiles.iter().any(|tile| tile
            .columns
            .node_flags
            .iter()
            .any(|flags| flags & 32 != 0))
    );
    check_geometry(&publication);
    check_tiles(&publication);
    Ok(())
}

#[test]
fn oversized_cluster_splits_at_bottlenecks_cycles_or_the_hard_budget() -> Result<(), GraphError> {
    let publication = prepare(chain(CLUSTER_NODE_LIMIT * 2 + 100, usize::MAX))?;
    assert_eq!(publication.layout.partitions.len(), 3);
    assert_eq!(publication.layout.partitions[1].cut, CutReason::Bottleneck);
    assert!(
        publication
            .layout
            .partitions
            .iter()
            .all(|partition| partition.nodes.len() <= CLUSTER_NODE_LIMIT)
    );
    assert_eq!(publication.summary.bottleneck_runs.len(), 1);
    assert_eq!(
        publication.summary.bottleneck_runs[0].node_count as usize,
        publication.nodes.len()
    );
    let mut cycle = chain(CLUSTER_NODE_LIMIT + 100, usize::MAX);
    cycle.entries.clear();
    cycle.edges.push(edge(CLUSTER_NODE_LIMIT + 99, 0));
    let publication = prepare(cycle)?;
    assert_eq!(publication.layout.partitions[1].cut, CutReason::Cycle);
    let wide = Graph {
        nodes: (0..CLUSTER_NODE_LIMIT + 100)
            .map(|index| node(index, 0, NodeKind::Passage))
            .collect(),
        ..Graph::default()
    };
    let publication = prepare(wide)?;
    assert_eq!(publication.layout.partitions[1].cut, CutReason::Budget);
    check_geometry(&publication);
    check_tiles(&publication);
    Ok(())
}

#[test]
fn long_chain_has_linear_size_analysis_and_compact_bottleneck_runs() -> Result<(), GraphError> {
    let publication = prepare(chain(20_000, 500))?;
    assert_eq!(publication.analysis.bottlenecks.len(), 20_000);
    assert_eq!(publication.summary.bottleneck_runs.len(), 40);
    assert_eq!(
        publication.analysis.rows[19_999].immediate_dominator,
        Some(19_998)
    );
    check_geometry(&publication);
    check_tiles(&publication);
    Ok(())
}

fn reaches(graph: &Graph, target: Id, removed: Option<Id>) -> bool {
    let mut seen = BTreeSet::new();
    let mut stack = graph.entries.clone();
    while let Some(node) = stack.pop() {
        if Some(node) != removed && seen.insert(node) {
            if node == target {
                return true;
            }
            stack.extend(
                graph
                    .edges
                    .iter()
                    .filter(|edge| edge.source == node)
                    .map(|edge| edge.target),
            );
        }
    }
    false
}

#[test]
fn dominators_match_vertex_removal_oracle_for_all_three_node_directed_graphs()
-> Result<(), GraphError> {
    // Exhaustive topology includes self-loops, irreducible SCCs, several entries,
    // endings with outgoing edges and unreachable endings.
    for mask in 0..512 {
        for entry_mask in [1, 3, 7] {
            let graph = Graph {
                nodes: (0..3)
                    .map(|index| node(index, index / 2, NodeKind::Passage))
                    .collect(),
                edges: (0..9)
                    .filter(|bit| mask & (1 << bit) != 0)
                    .map(|bit| edge(bit / 3, bit % 3))
                    .collect(),
                entries: (0..3)
                    .filter(|bit| entry_mask & (1 << bit) != 0)
                    .map(id)
                    .collect(),
                endings: vec![
                    Ending {
                        node: id(1),
                        class: EndingClass::Success,
                    },
                    Ending {
                        node: id(2),
                        class: EndingClass::Failure,
                    },
                ],
            };
            let publication = prepare(graph.clone())?;
            for node_index in 0..3 {
                let reachable = reaches(&graph, id(node_index), None);
                assert_eq!(publication.analysis.rows[node_index].reachable, reachable);
                let expected: BTreeSet<_> = (0..3)
                    .filter(|candidate| {
                        *candidate != node_index
                            && reachable
                            && !reaches(&graph, id(node_index), Some(id(*candidate)))
                    })
                    .collect();
                let mut actual = BTreeSet::new();
                let mut cursor = publication.analysis.rows[node_index].immediate_dominator;
                while let Some(parent) = cursor {
                    assert!(actual.insert(parent as usize));
                    cursor = publication.analysis.rows[parent as usize].immediate_dominator;
                }
                assert_eq!(
                    actual, expected,
                    "topology={mask}, entries={entry_mask}, node={node_index}"
                );
                let reachable_endings: Vec<_> = graph
                    .endings
                    .iter()
                    .filter(|ending| reaches(&graph, ending.node, None))
                    .collect();
                let expected_bottleneck = !reachable_endings.is_empty()
                    && reachable_endings
                        .iter()
                        .all(|ending| !reaches(&graph, ending.node, Some(id(node_index))));
                assert_eq!(
                    publication.analysis.rows[node_index].bottleneck,
                    expected_bottleneck
                );
                let cyclic = graph
                    .edges
                    .iter()
                    .filter(|edge| edge.source == id(node_index))
                    .any(|edge| {
                        let mut from_child = graph.clone();
                        from_child.entries = vec![edge.target];
                        reaches(&from_child, id(node_index), None)
                    });
                assert_eq!(publication.analysis.rows[node_index].cyclic, cyclic);
            }
            check_geometry(&publication);
        }
    }
    Ok(())
}
