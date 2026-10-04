//! Native benchmark runner. Scratch `.columns` files concatenate little-endian
//! vectors solely to estimate compression; they are not a readable tile format.

use std::{error::Error, fs, path::PathBuf, time::Instant};

use narrata_graph::{
    ClusterId, Columns, DetailLevel, Edge, EdgeKind, Ending, EndingClass, Graph, Id, Node,
    NodeKind, prepare,
};
use narrata_kernel::codec::sha256;

const NODES_PER_CLUSTER: usize = 500;

fn synthetic(count: usize) -> Graph {
    let id = |index: usize| Id::from_u128(index as u128);
    let mut graph = Graph {
        nodes: (0..count)
            .map(|index| Node {
                id: id(index),
                kind: if index.is_multiple_of(25) {
                    NodeKind::Branch
                } else {
                    NodeKind::Passage
                },
                cluster: ClusterId::from_u128((index / NODES_PER_CLUSTER) as u128),
                author_order: Some(index as u64),
            })
            .collect(),
        entries: vec![id(0)],
        endings: vec![
            Ending {
                node: id(count - 2),
                class: EndingClass::Failure,
            },
            Ending {
                node: id(count - 1),
                class: EndingClass::Success,
            },
        ],
        ..Graph::default()
    };
    for index in 0..count - 2 {
        let local = index % NODES_PER_CLUSTER;
        let target = if local % 25 == 1 {
            index + 2
        } else {
            index + 1
        };
        graph.edges.push(Edge {
            source: id(index),
            target: id(target),
            kind: EdgeKind::Next,
        });
        if local.is_multiple_of(25) {
            graph.edges.push(Edge {
                source: id(index),
                target: id(index + 2),
                kind: EdgeKind::Choice,
            });
        }
        // A small local loop in each cluster; no recursive algorithms required.
        if local == 250 {
            graph.edges.push(Edge {
                source: id(index),
                target: id(index - 10),
                kind: EdgeKind::Conditional,
            });
        }
        // Sparse long branches skip one authored cluster, while most choices
        // rejoin after three passages.
        if local == 400 && (index / NODES_PER_CLUSTER).is_multiple_of(10) && index + 600 < count {
            graph.edges.push(Edge {
                source: id(index),
                target: id(index + 600),
                kind: EdgeKind::Choice,
            });
        }
    }
    graph.edges.push(Edge {
        source: id(count - 3),
        target: id(count - 1),
        kind: EdgeKind::Choice,
    });
    graph.nodes[count - 2].kind = NodeKind::Ending;
    graph.nodes[count - 1].kind = NodeKind::Ending;
    graph
}

fn pack(columns: &Columns) -> Vec<u8> {
    let mut bytes = Vec::new();
    for id in &columns.ids {
        bytes.extend_from_slice(id);
    }
    for value in &columns.x {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in &columns.y {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in &columns.clusters {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&columns.kinds);
    bytes.extend_from_slice(&columns.importance);
    for value in &columns.node_flags {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in &columns.author_order {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&columns.has_author_order);
    for value in &columns.sources {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in &columns.targets {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&columns.edge_kind_masks);
    for counts in &columns.edge_kind_counts {
        for count in counts {
            bytes.extend_from_slice(&count.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&columns.edge_importance);
    bytes.extend_from_slice(&columns.edge_flags);
    bytes
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let mut count = 10_000;
    let mut output = PathBuf::from(".temp/graph-layout/native");
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--nodes" => count = args.next().ok_or("missing node count")?.parse()?,
            "--output" => output = PathBuf::from(args.next().ok_or("missing output directory")?),
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    if !(500..=narrata_graph::MAX_NODES).contains(&count) || !count.is_multiple_of(500) {
        return Err("node count must be a multiple of 500 within the checked node budget".into());
    }
    fs::create_dir_all(&output)?;
    let generate_start = Instant::now();
    let graph = synthetic(count);
    let input_edges = graph.edges.len();
    let generate_ns = generate_start.elapsed().as_nanos();
    let prepare_start = Instant::now();
    let publication = prepare(graph)?;
    let prepare_ns = prepare_start.elapsed().as_nanos();
    let pack_start = Instant::now();
    let mut blobs = Vec::new();
    let mut total_bytes = 0;
    for tile in &publication.tiles {
        let bytes = pack(&tile.columns);
        total_bytes += bytes.len();
        blobs.push(bytes);
    }
    let pack_ns = pack_start.elapsed().as_nanos();
    // Filesystem writes and the sampling grace period are outside measured CPU.
    let mut digests = Vec::new();
    for (index, bytes) in blobs.iter().enumerate() {
        fs::write(output.join(format!("{index:06}.columns")), bytes)?;
        digests.extend_from_slice(&sha256(bytes));
    }
    let result = serde_json::json!({
        "nodes": count,
        "input_edges": input_edges,
        "normalized_edges": publication.edges.len(),
        "clusters": publication.layout.clusters.len(),
        "partitions": publication.layout.partitions.len(),
        "tiles": publication.tiles.len(),
        "node_tiles": publication.tiles.iter().filter(|tile| tile.level == DetailLevel::Node).count(),
        "generate_ns": generate_ns,
        "prepare_ns": prepare_ns,
        "pack_ns": pack_ns,
        "column_bytes": total_bytes,
        // Conservative budget for 15 byte strings plus small metadata and the
        // kernel's 56-byte envelope; no normative field IDs are assigned here.
        "framing_budget_bytes": publication.tiles.len() * 256,
        "summary_entries": publication.summary.entries.len(),
        "summary_endings": publication.summary.endings.len(),
        "summary_bottleneck_runs": publication.summary.bottleneck_runs.len(),
        "columns_checksum": sha256(&digests),
    });
    fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    println!("{result}");
    std::thread::sleep(std::time::Duration::from_millis(200));
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_graph_has_twenty_choice_points_per_cluster_and_two_endings()
    -> Result<(), Box<dyn Error>> {
        let graph = synthetic(10_000);
        assert_eq!(
            graph
                .nodes
                .iter()
                .filter(|node| node.kind == NodeKind::Branch)
                .count(),
            400
        );
        let publication = prepare(graph)?;
        assert!(publication.analysis.rows.iter().all(|row| row.reachable));
        assert_eq!(publication.summary.endings.len(), 2);
        assert!(publication.summary.bottleneck_runs.len() <= 20);
        Ok(())
    }

    #[test]
    fn measurement_buffers_are_little_endian_and_deterministic() -> Result<(), Box<dyn Error>> {
        let mut graph = synthetic(1000);
        let before = prepare(graph.clone())?;
        graph.nodes.reverse();
        graph.edges.reverse();
        let after = prepare(graph)?;
        for (left, right) in before.tiles.iter().zip(&after.tiles) {
            let bytes = pack(&left.columns);
            assert_eq!(bytes, pack(&right.columns));
            let offset = left.columns.ids.len() * 16;
            for (index, coordinate) in left.columns.x.iter().enumerate() {
                assert_eq!(
                    &bytes[offset + index * 4..offset + index * 4 + 4],
                    &coordinate.to_le_bytes()
                );
            }
        }
        Ok(())
    }
}
