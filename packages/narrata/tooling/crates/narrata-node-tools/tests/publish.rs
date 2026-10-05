#![allow(clippy::unwrap_used)]

use std::{collections::BTreeSet, fs, path::Path};

use narrata_graph::{
    EdgeKind, Id, NodeKind, SemanticSummary, decode_labels,
    wire::{decode_index, decode_tile},
};
use narrata_node_tools::{
    compose_published, pretty,
    publish::{GraphFiles, publish, structural_graph},
    write_text,
};
use narrata_nodes::{Analysis, Compilation};
use serde_json::{Value, json};

#[path = "../../../../nodes/crates/narrata-nodes/tests/support/mod.rs"]
mod support;

fn outline() -> Value {
    json!({"format_version": 1, "provider": "local", "units": {
        "main.gate": {
            "blocks": ["b1", "r-wave", "r-nod", "b2", "r-rope", "r-lamp", "b3", "b4"],
            "markers": {"b1": support::point(1), "b2": support::point(2)}
        },
        "side.talk": {"blocks": ["talk"]}
    }})
}

fn project(root: &Path) {
    for (file, value) in [
        ("project.json", support::manifest()),
        ("main.json", support::main_package()),
        ("side.json", support::side_package()),
        ("outline.json", outline()),
    ] {
        write_text(&root.join(file), &pretty(&value).unwrap()).unwrap();
    }
}

#[test]
fn the_adapter_preserves_static_node_and_edge_kinds_and_resolves_imports() {
    let compilation = support::compiled();
    let input = structural_graph(&compilation.program).unwrap();
    assert_eq!(input.graph.entries, [Id::from_u128(1)]);
    assert_eq!(input.graph.nodes.len(), 8);
    let kinds: BTreeSet<_> = input.graph.nodes.iter().map(|node| node.kind).collect();
    assert_eq!(
        kinds,
        [
            NodeKind::Passage,
            NodeKind::Branch,
            NodeKind::State,
            NodeKind::Call,
            NodeKind::Return
        ]
        .into()
    );
    let kinds: BTreeSet<_> = input.graph.edges.iter().map(|edge| edge.kind).collect();
    assert_eq!(
        kinds,
        [
            EdgeKind::Choice,
            EdgeKind::Next,
            EdgeKind::Conditional,
            EdgeKind::Call,
            EdgeKind::Return
        ]
        .into()
    );
    assert!(
        input
            .graph
            .edges
            .iter()
            .any(|edge| edge.source == Id::from_u128(2)
                && edge.target == Id::from_u128(11)
                && edge.kind == EdgeKind::Call)
    );
    assert!(
        input
            .graph
            .edges
            .iter()
            .any(|edge| edge.source == Id::from_u128(12)
                && edge.target == Id::from_u128(3)
                && edge.kind == EdgeKind::Return)
    );
    assert!(
        !input
            .graph
            .edges
            .iter()
            .any(|edge| edge.source == Id::from_u128(2) && edge.target == Id::from_u128(3))
    );
    assert_eq!(
        input
            .graph
            .endings
            .iter()
            .map(|ending| ending.node)
            .collect::<BTreeSet<_>>(),
        [Id::from_u128(5), Id::from_u128(6)].into()
    );
    let graph_cluster = input
        .graph
        .nodes
        .iter()
        .find(|node| node.id == Id::from_u128(2))
        .unwrap()
        .cluster;
    assert!(
        input
            .graph
            .nodes
            .iter()
            .filter(|node| (2..=6).contains(&u128::from_be_bytes(node.id.0)))
            .all(|node| node.cluster == graph_cluster)
    );
    let body_cluster = input
        .graph
        .nodes
        .iter()
        .find(|node| node.id == Id::from_u128(1))
        .unwrap()
        .cluster;
    assert_ne!(body_cluster, graph_cluster);
    assert!(
        input
            .graph
            .nodes
            .iter()
            .all(|node| node.author_order.is_none())
    );
    let publication = publish(&compilation.program, Some(&compilation.names)).unwrap();
    assert!(
        !publication
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "structurally_unreachable"
                || diagnostic.code == "structural_dead_end")
    );
    assert_eq!(publication.summary.endings.len(), 2);
    assert!(
        publication
            .summary
            .endings
            .iter()
            .all(|ending| ending.reachable)
    );
    assert_eq!(
        publication.summary.artifact_id,
        Some(hex::encode(compilation.program.artifact_id().as_bytes()))
    );
}

#[test]
fn local_calls_and_shared_content_units_are_projected() {
    let compilation = support::try_variant(|manifest, main, side| {
        manifest["product"]["bindings"] = json!([]);
        main["graphs"]["start"]["imports"] = json!({});
        main["graphs"]["start"]["nodes"]["visit"]["data"]["target"] =
            json!({"kind": "local", "graph": "visit"});
        let helper = side["graphs"]["visit"].clone();
        main["graphs"]["visit"] = helper;
        side["graphs"]["visit"]["nodes"]["talk"]["id"] = json!(support::node(21));
        side["graphs"]["visit"]["nodes"]["out"]["id"] = json!(support::node(22));
        side["graphs"]["visit"]["nodes"]["talk"]["data"]["choice_points"][0]["id"] =
            json!(support::point(21));
        side["graphs"]["visit"]["nodes"]["talk"]["data"]["choice_points"][0]["options"][0]["id"] =
            json!(support::option(21));
    })
    .unwrap();
    let input = structural_graph(&compilation.program).unwrap();
    assert!(
        input
            .graph
            .edges
            .iter()
            .any(|edge| edge.source == Id::from_u128(2)
                && edge.target == Id::from_u128(11)
                && edge.kind == EdgeKind::Call)
    );
    let cluster = |id| {
        input
            .graph
            .nodes
            .iter()
            .find(|node| node.id == Id::from_u128(id))
            .unwrap()
            .cluster
    };
    assert_eq!(cluster(11), cluster(21));
    assert_ne!(cluster(12), cluster(22));
}

#[test]
fn titles_and_author_node_aliases_do_not_change_geometry_objects() {
    let before = support::compiled();
    let after = support::try_variant(|_, main, _| {
        main["graphs"]["start"]["nodes"]["gate"]["data"]["title"] =
            support::content("changed:title");
        let gate = main["graphs"]["start"]["nodes"]
            .as_object_mut()
            .unwrap()
            .remove("gate")
            .unwrap();
        main["graphs"]["start"]["nodes"]["renamed"] = gate;
        main["graphs"]["start"]["entry"] = json!("renamed");
    })
    .unwrap();
    let a = publish(&before.program, None).unwrap();
    let b = publish(&after.program, None).unwrap();
    assert_eq!(a.geometry, b.geometry);
    assert_ne!(a.labels, b.labels);
    assert_ne!(a.summary.artifact_id, b.summary.artifact_id);
    let repeated = publish(&before.program, None).unwrap();
    assert_eq!(a.geometry, repeated.geometry);
    assert_eq!(a.labels, repeated.labels);
    assert_eq!(a.summary, repeated.summary);
}

fn with_topology_diagnostics() -> Compilation {
    support::try_variant(|_, main, side| {
        main["graphs"]["start"]["nodes"]["loop"] = json!({"id": support::node(7), "type_id": "narrata.mutate", "data": {"assignments": [], "next": "loop"}});
        main["graphs"]["start"]["nodes"]["gate"]["data"]["choice_points"][2]["options"].as_array_mut().unwrap().push(json!({
            "id": support::option(8), "key": "loop", "label": support::content("loop"), "outcome": {"kind": "branch", "target": "loop"}
        }));
        side["graphs"]["unused"] = json!({"outcomes": ["back"], "entry": "out", "nodes": {
            "out": {"id": support::node(25), "type_id": "narrata.return", "data": {"outcome": "back"}}
        }});
    }).unwrap()
}

#[test]
fn topology_diagnostics_include_unreachable_dead_ends_cycles_and_bottlenecks() {
    let compilation = with_topology_diagnostics();
    let publication = publish(&compilation.program, Some(&compilation.names)).unwrap();
    for code in [
        "structurally_unreachable",
        "structural_dead_end",
        "structural_cycle",
        "structural_bottleneck",
    ] {
        assert!(
            publication
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == code),
            "missing {code}"
        );
    }
    assert!(
        publication
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "structural_cycle"
                && diagnostic.path.ends_with("nodes.loop"))
    );
}

#[test]
fn publication_writes_checked_companions_and_merges_diagnostics_without_failing_compose() {
    let temp = tempfile::tempdir().unwrap();
    project(temp.path());
    let output = temp.path().join("story.narpack");
    let outline_path = temp.path().join("outline.json");
    let composed = compose_published(
        &temp.path().join("project.json"),
        &output,
        false,
        std::slice::from_ref(&outline_path),
    )
    .unwrap();
    assert!(
        !composed
            .compilation
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.starts_with("outline_"))
    );
    let files =
        GraphFiles::decode_json(&fs::read(output.with_extension("graph.json")).unwrap()).unwrap();
    let index_id: [u8; 32] = hex::decode(&files.index_object_id)
        .unwrap()
        .try_into()
        .unwrap();
    let objects = output.with_extension("graph");
    let index = decode_index(
        &fs::read(objects.join(format!("{}.cbor", files.index_object_id))).unwrap(),
        &index_id,
    )
    .unwrap();
    for (row, tile) in index.tiles().iter().enumerate() {
        let bytes =
            fs::read(objects.join(format!("{}.cbor", hex::encode(tile.object_id)))).unwrap();
        decode_tile(&bytes, &index, row as u32).unwrap();
    }
    for label in &files.labels {
        let id: [u8; 32] = hex::decode(&label.object_id).unwrap().try_into().unwrap();
        let bytes = fs::read(objects.join(format!("{}.cbor", label.object_id))).unwrap();
        decode_labels(&bytes, &id, label.cluster).unwrap();
        assert!(
            index
                .clusters()
                .iter()
                .any(|cluster| cluster.id == label.cluster)
        );
    }
    let summary =
        SemanticSummary::decode_json(&fs::read(output.with_extension("summary.json")).unwrap())
            .unwrap();
    assert_eq!(
        summary.artifact_id,
        Some(hex::encode(files.artifact_id.as_bytes()))
    );
    let pack = fs::read(&output).unwrap();
    let before = fs::read(output.with_extension("graph.json")).unwrap();
    compose_published(
        &temp.path().join("project.json"),
        &output,
        true,
        std::slice::from_ref(&outline_path),
    )
    .unwrap();
    assert_eq!(pack, fs::read(&output).unwrap());
    assert_eq!(
        before,
        fs::read(output.with_extension("graph.json")).unwrap()
    );
    let mut damaged = outline();
    damaged["units"]["main.gate"]["blocks"] = json!(["b1", "b2", "b3"]);
    write_text(&outline_path, &pretty(&damaged).unwrap()).unwrap();
    compose_published(
        &temp.path().join("project.json"),
        &output,
        true,
        &[outline_path],
    )
    .unwrap();
    let analysis: Analysis = narrata_nodes::parse_json(
        &fs::read_to_string(output.with_extension("analysis.json")).unwrap(),
    )
    .unwrap();
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "outline_anchor_missing")
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "structural_bottleneck")
    );
    assert_eq!(pack, fs::read(output).unwrap());
}

#[test]
fn absent_outlines_are_reported_and_malformed_or_duplicate_outlines_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    project(temp.path());
    let output = temp.path().join("story.narpack");
    let manifest = temp.path().join("project.json");
    let composed = compose_published(&manifest, &output, false, &[]).unwrap();
    assert!(
        composed
            .compilation
            .analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "outline_checks_skipped"
                && diagnostic.path == "content.local")
    );
    let outline = temp.path().join("outline.json");
    assert_eq!(
        compose_published(
            &manifest,
            &output,
            true,
            &[outline.clone(), outline.clone()]
        )
        .unwrap_err()
        .code,
        "outline_provider"
    );
    let previous = fs::read(&output).unwrap();
    write_text(
        &outline,
        r#"{"format_version":2,"provider":"local","units":{}}"#,
    )
    .unwrap();
    assert_eq!(
        compose_published(&manifest, &output, true, &[outline])
            .unwrap_err()
            .code,
        "outline_version"
    );
    assert_eq!(previous, fs::read(output).unwrap());
}

#[test]
fn publication_references_reject_bad_versions_ids_order_and_duplicate_fields() {
    let publication = publish(&support::compiled().program, None).unwrap();
    let valid = serde_json::to_value(&publication.files).unwrap();
    for (field, value) in [
        ("format_version", json!(2)),
        ("index_object_id", json!("AB".repeat(32))),
        ("unexpected", json!(true)),
    ] {
        let mut bad = valid.clone();
        bad[field] = value;
        assert!(GraphFiles::decode_json(bad.to_string().as_bytes()).is_err());
    }
    let mut reversed = valid.clone();
    reversed["labels"].as_array_mut().unwrap().reverse();
    assert!(GraphFiles::decode_json(reversed.to_string().as_bytes()).is_err());
    let mut duplicate = valid.clone();
    let first = duplicate["labels"][0].clone();
    duplicate["labels"].as_array_mut().unwrap().push(first);
    assert!(GraphFiles::decode_json(duplicate.to_string().as_bytes()).is_err());
    let mut repeated_object = valid.clone();
    repeated_object["labels"][1]["object_id"] = repeated_object["labels"][0]["object_id"].clone();
    assert!(GraphFiles::decode_json(repeated_object.to_string().as_bytes()).is_err());
    let duplicate_field = valid.to_string().replacen("{", "{\"format_version\":1,", 1);
    assert!(GraphFiles::decode_json(duplicate_field.as_bytes()).is_err());
}

#[test]
fn publication_reference_schema_matches_the_exchanged_rust_type() {
    let schema =
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/graph-files-v1.schema.json"))
            .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&schema).unwrap(),
        serde_json::to_value(narrata_node_tools::publish::graph_files_schema()).unwrap()
    );
}
