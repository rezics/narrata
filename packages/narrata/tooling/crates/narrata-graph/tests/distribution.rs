use std::{collections::BTreeSet, error::Error, fs, path::PathBuf};

use narrata_graph::{
    ClusterId, DetailLevel, Edge, EdgeKind, Graph, Id, Node, NodeKind, SemanticSummary, prepare,
    summary_schema,
    wire::{decode_index, decode_tile, encode_publication},
};
use narrata_kernel::codec::sha256;
use serde::Deserialize;

mod support;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format_version: u16,
    index_object_id: String,
    files: Vec<File>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    file: String,
    bytes: usize,
    sha256: String,
}

#[test]
fn frozen_graph_v1_is_checked_and_regenerates_byte_for_byte() -> Result<(), Box<dyn Error>> {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../../fixtures/compat/graph-v1");
    let manifest: Manifest = serde_json::from_slice(&fs::read(root.join("manifest.json"))?)?;
    assert_eq!(manifest.format_version, 1);
    let publication = prepare(support::compatibility_graph())?;
    let generated = encode_publication(&publication)?;
    assert_eq!(hex::encode(generated.index.id), manifest.index_object_id);
    let mut names = BTreeSet::new();
    for file in &manifest.files {
        assert!(names.insert(file.file.clone()));
        let bytes = fs::read(root.join(&file.file))?;
        assert_eq!(bytes.len(), file.bytes);
        assert_eq!(hex::encode(sha256(&bytes)), file.sha256);
    }
    let actual_files: BTreeSet<_> = fs::read_dir(&root)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()?;
    names.insert("manifest.json".to_owned());
    assert_eq!(names, actual_files);
    assert_eq!(manifest.files.len(), generated.tiles.len() + 2);
    let frozen_index = fs::read(root.join(generated.index.filename()))?;
    assert_eq!(generated.index.bytes, frozen_index);
    let index = decode_index(&frozen_index, &generated.index.id)?;
    let mut expected_files = BTreeSet::from([
        "manifest.json".to_owned(),
        "summary.json".to_owned(),
        generated.index.filename(),
    ]);
    for (row, (object, expected)) in generated.tiles.iter().zip(&publication.tiles).enumerate() {
        expected_files.insert(object.filename());
        let bytes = fs::read(root.join(object.filename()))?;
        assert_eq!(object.bytes, bytes);
        assert_eq!(decode_tile(&bytes, &index, row as u32)?, *expected);
    }
    assert_eq!(actual_files, expected_files);
    let json = fs::read(root.join("summary.json"))?;
    assert_eq!(publication.summary.encode_json()?, json);
    assert_eq!(SemanticSummary::decode_json(&json)?, publication.summary);
    assert!(publication.analysis.rows.iter().any(|row| row.cyclic));
    assert!(publication.analysis.rows.iter().any(|row| !row.reachable));
    assert!(
        publication
            .analysis
            .rows
            .iter()
            .any(|row| row.reachable && row.dead_end)
    );
    assert_eq!(publication.summary.endings.len(), 3);
    assert!(
        publication
            .tiles
            .iter()
            .any(|tile| tile.level == DetailLevel::Node
                && tile.columns.node_flags.iter().any(|flags| flags & 32 != 0))
    );
    Ok(())
}

#[test]
fn input_order_does_not_change_any_distributed_bytes() -> Result<(), Box<dyn Error>> {
    let graph = support::compatibility_graph();
    let before = prepare(graph.clone())?;
    let mut shuffled = graph;
    shuffled.nodes.reverse();
    shuffled.edges.reverse();
    shuffled.endings.reverse();
    let after = prepare(shuffled)?;
    assert_eq!(encode_publication(&before)?, encode_publication(&after)?);
    assert_eq!(before.summary.encode_json()?, after.summary.encode_json()?);
    Ok(())
}

#[test]
fn empty_graph_has_one_checked_empty_work_tile() -> Result<(), Box<dyn Error>> {
    let publication = prepare(Graph::default())?;
    let objects = encode_publication(&publication)?;
    let index = decode_index(&objects.index.bytes, &objects.index.id)?;
    assert!(index.clusters().is_empty());
    assert_eq!(index.tiles().len(), 1);
    assert_eq!(index.tiles()[0].bounds, None);
    assert_eq!(
        decode_tile(&objects.tiles[0].bytes, &index, 0)?,
        publication.tiles[0]
    );
    assert_eq!(
        SemanticSummary::decode_json(&publication.summary.encode_json()?)?,
        publication.summary
    );
    Ok(())
}

#[test]
fn partitioned_high_degree_graph_round_trips_every_split_tile() -> Result<(), Box<dyn Error>> {
    let count = 6500_u64;
    let id = |n: u64| Id::from_u128(u128::from(n));
    let mut graph = Graph {
        nodes: (0..count)
            .map(|n| Node {
                id: id(n),
                kind: if n == count - 1 {
                    NodeKind::Ending
                } else {
                    NodeKind::Passage
                },
                cluster: ClusterId::from_u128(u128::from(n >= 5000)),
                author_order: Some(n),
            })
            .collect(),
        edges: (1..count)
            .map(|n| Edge {
                source: id(n - 1),
                target: id(n),
                kind: EdgeKind::Next,
            })
            .collect(),
        entries: vec![id(0)],
        ..Graph::default()
    };
    graph.edges.extend((2..count).map(|n| Edge {
        source: id(0),
        target: id(n),
        kind: EdgeKind::Choice,
    }));
    let publication = prepare(graph)?;
    assert!(publication.layout.partitions.len() > publication.layout.clusters.len());
    assert!(publication.tiles.iter().any(|tile| tile.ordinal > 0));
    let objects = encode_publication(&publication)?;
    let index = decode_index(&objects.index.bytes, &objects.index.id)?;
    for (row, (object, expected)) in objects.tiles.iter().zip(&publication.tiles).enumerate() {
        assert_eq!(decode_tile(&object.bytes, &index, row as u32)?, *expected);
    }
    Ok(())
}

#[test]
fn geometry_changes_identity_and_artifact_attachment_keeps_geometry_ids()
-> Result<(), Box<dyn Error>> {
    let mut publication = prepare(support::compatibility_graph())?;
    let before = encode_publication(&publication)?;
    let summary = publication.summary.encode_json()?;
    publication.summary.artifact_id = Some("ab".repeat(32));
    assert_eq!(encode_publication(&publication)?, before);
    assert_ne!(publication.summary.encode_json()?, summary);
    let row = publication
        .tiles
        .iter()
        .position(|tile| tile.level == DetailLevel::Node)
        .ok_or("no node tile")?;
    publication.tiles[row].columns.x[0] += 1;
    let after = encode_publication(&publication)?;
    assert_ne!(before.tiles[row].id, after.tiles[row].id);
    assert_ne!(before.index.id, after.index.id);
    for (other, (before, after)) in before.tiles.iter().zip(after.tiles).enumerate() {
        if other != row {
            assert_eq!(*before, after);
        }
    }
    Ok(())
}

#[test]
fn summary_schema_matches_rust_types() -> Result<(), Box<dyn Error>> {
    let frozen =
        fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schema/summary-v1.schema.json"))?;
    assert_eq!(
        serde_json::to_value(summary_schema())?,
        serde_json::from_slice::<serde_json::Value>(&frozen)?
    );
    Ok(())
}

#[test]
fn summary_rejects_bad_versions_identities_sets_and_fields() -> Result<(), Box<dyn Error>> {
    let mut summary = prepare(support::compatibility_graph())?.summary;
    summary.artifact_id = Some("ab".repeat(32));
    assert_eq!(
        SemanticSummary::decode_json(&summary.encode_json()?)?,
        summary
    );
    let valid = serde_json::to_value(&summary)?;
    for invalid in [
        serde_json::json!(2),
        serde_json::json!(0),
        serde_json::json!("1"),
    ] {
        let mut json = valid.clone();
        json["format_version"] = invalid;
        assert!(SemanticSummary::decode_json(&serde_json::to_vec(&json)?).is_err());
    }
    for (field, value) in [
        ("extra", serde_json::json!(0)),
        ("artifact_id", serde_json::json!("AB".repeat(32))),
        (
            "entries",
            serde_json::json!(["node:0000000000000000000000000000000A"]),
        ),
        (
            "entries",
            serde_json::json!([
                "node:00000000000000000000000000000000",
                "node:00000000000000000000000000000000"
            ]),
        ),
    ] {
        let mut json = valid.clone();
        json[field] = value;
        assert!(SemanticSummary::decode_json(&serde_json::to_vec(&json)?).is_err());
    }
    let encoded = summary.encode_json()?;
    let duplicate = format!(
        "{{\"format_version\":1,{}",
        std::str::from_utf8(&encoded)?.trim_start_matches('{')
    );
    assert!(SemanticSummary::decode_json(duplicate.as_bytes()).is_err());
    let mut bad = summary.clone();
    let duplicate = bad.endings[0].clusters[0];
    bad.endings[0].clusters.push(duplicate);
    assert!(bad.encode_json().is_err());
    let mut bad = summary.clone();
    bad.bottleneck_runs[0].node_count = 0;
    assert!(bad.encode_json().is_err());
    let mut bad = summary;
    bad.bottleneck_runs
        .insert(1, bad.bottleneck_runs[0].clone());
    assert!(bad.encode_json().is_err());
    Ok(())
}
