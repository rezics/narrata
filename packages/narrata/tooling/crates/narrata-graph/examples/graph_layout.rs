//! Native benchmark runner measuring the complete ADR 0016 distribution bytes.

use std::{error::Error, fs, path::PathBuf, time::Instant};

use narrata_graph::{
    ClusterId, DetailLevel, Edge, EdgeKind, Ending, EndingClass, Graph, Id, Node, NodeKind,
    prepare, wire::encode_publication,
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
    let encode_start = Instant::now();
    let encoded = encode_publication(&publication)?;
    let summary = publication.summary.encode_json()?;
    let tile_bytes: usize = encoded.tiles.iter().map(|object| object.bytes.len()).sum();
    let encode_ns = encode_start.elapsed().as_nanos();
    // Filesystem writes and the sampling grace period are outside measured CPU.
    let mut digests = Vec::new();
    for object in std::iter::once(&encoded.index).chain(&encoded.tiles) {
        fs::write(output.join(object.filename()), &object.bytes)?;
        digests.extend_from_slice(&sha256(&object.bytes));
    }
    fs::write(output.join("summary.json"), &summary)?;
    digests.extend_from_slice(&sha256(&summary));
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
        "encode_ns": encode_ns,
        "tile_bytes": tile_bytes,
        "index_bytes": encoded.index.bytes.len(),
        "index_filename": encoded.index.filename(),
        "summary_bytes": summary.len(),
        "total_bytes": tile_bytes + encoded.index.bytes.len() + summary.len(),
        "summary_entries": publication.summary.entries.len(),
        "summary_endings": publication.summary.endings.len(),
        "summary_bottleneck_runs": publication.summary.bottleneck_runs.len(),
        "distribution_checksum": hex::encode(sha256(&digests)),
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
    fn distribution_bytes_are_deterministic_at_benchmark_scale() -> Result<(), Box<dyn Error>> {
        let mut graph = synthetic(1000);
        let before = prepare(graph.clone())?;
        graph.nodes.reverse();
        graph.edges.reverse();
        let after = prepare(graph)?;
        assert_eq!(encode_publication(&before)?, encode_publication(&after)?);
        assert_eq!(before.summary.encode_json()?, after.summary.encode_json()?);
        Ok(())
    }
}
