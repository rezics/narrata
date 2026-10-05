#![allow(clippy::unwrap_used, clippy::panic)]

use narrata_authoring::{
    objects::Digest,
    project_relations,
    relations::{CheckedRelations, RelationKind, RelationRow, read_relations, unit_id},
};
use narrata_kernel::content::ContentRef;
use narrata_nodes::{Error, NodeId, NodeRegistry, ProjectSource, canonical_json, compile};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn node(id: u128) -> NodeId {
    NodeId::from_bytes(id.to_be_bytes())
}
fn source() -> ProjectSource {
    let mut value = json!({
        "manifest": {"format_version":2,"product":{"id":"relations","entry":{"package":"main","graph":"main"},"endings":{"done":{"body":{"unit":{"provider":"local","key":"ending"},"first":"e1","last":"e2"}}}},"packages":{"main":"main.json"}},
        "packages":{"main":{"id":"relations.main","version":"1","exports":["main"],"graphs":{"main":{"entry":"entry","outcomes":["done"],"nodes":{
            "entry":{"id":node(1),"type_id":"narrata.branch","data":{"condition":{"kind":"literal","value":true},"when_true":"a","when_false":"b"}},
            "a":{"id":node(2),"type_id":"narrata.passage","data":{"body":{"unit":{"provider":"local","key":"shared"},"first":"b1","last":"b3"},"choice_points":[
                {"id":"choice-point:00000000000000000000000000000002","key":"reply","placement":"b1","options":[{"id":"option:00000000000000000000000000000003","key":"reply","outcome":{"kind":"local","reply":{"unit":{"provider":"local","key":"shared"},"first":"r1","last":"r1"},"rejoin":"b2"}}]},
                {"id":"choice-point:00000000000000000000000000000001","key":"route","options":[
                    {"id":"option:00000000000000000000000000000001","key":"one","label":{"provider":"local","key":"one"},"outcome":{"kind":"branch","target":"control"}},
                    {"id":"option:00000000000000000000000000000002","key":"two","label":{"provider":"local","key":"two"},"outcome":{"kind":"branch","target":"control"}}
                ]}
            ]}},
            "b":{"id":node(3),"type_id":"narrata.passage","data":{"body":{"unit":{"provider":"local","key":"shared"}},"next":"loop"}},
            "control":{"id":node(4),"type_id":"narrata.mutate","data":{"next":"end"}},
            "end":{"id":node(5),"type_id":"narrata.return","data":{"outcome":"done"}},
            "loop":{"id":node(6),"type_id":"narrata.branch","data":{"condition":{"kind":"literal","value":false},"when_true":"loop","when_false":"control"}},
            "unreachable":{"id":node(7),"type_id":"narrata.passage","data":{"body":{"unit":{"provider":"local","key":"unreachable"}},"next":"end"}}
        }}}}}
    });
    ProjectSource {
        manifest: serde_json::from_value(value["manifest"].take()).unwrap(),
        packages: serde_json::from_value(value["packages"].take()).unwrap(),
    }
}
fn relation_digest(domain: &str, bytes: &[u8]) -> Digest {
    Digest::from_bytes(narrata_kernel::codec::digest_bytes(domain, 1, bytes))
}
fn read(index: &[u8], map: &BTreeMap<Digest, Vec<u8>>) -> narrata_nodes::Result<CheckedRelations> {
    read_relations(
        index,
        relation_digest("narrata.nodes.relation-index", index),
        |id| {
            map.get(&id)
                .cloned()
                .ok_or_else(|| Error::new("missing_part", id.to_string(), "missing"))
        },
    )
}

#[test]
fn projection_preserves_options_collapses_control_cycles_and_keeps_first_ties() {
    let compilation = compile(&source(), &NodeRegistry::gamebook()).unwrap();
    let projection = project_relations(&compilation.program).unwrap();
    let kinds: BTreeSet<_> = projection.rows().iter().map(RelationRow::kind).collect();
    assert_eq!(kinds.len(), 8);
    let shared = unit_id(&ContentRef::new("local", "shared").unwrap());
    let ending = unit_id(&ContentRef::new("local", "ending").unwrap());
    let edges: Vec<_> = projection
        .rows()
        .iter()
        .filter_map(|r| {
            if let RelationRow::Edge(r) = r {
                Some(r)
            } else {
                None
            }
        })
        .collect();
    let choices: Vec<_> = edges.iter().filter(|e| e.source_node == node(2)).collect();
    assert_eq!(choices.len(), 2);
    assert_ne!(choices[0].option, choices[1].option);
    assert!(choices.iter().all(|e| e.edge_kind == 0
        && e.source_unit == shared
        && e.target_unit == ending
        && e.target_node == node(5)));
    let from_b: Vec<_> = edges.iter().filter(|e| e.source_node == node(3)).collect();
    assert_eq!(from_b.len(), 1);
    assert_eq!(from_b[0].edge_kind, 1);
    let first: Vec<_> = projection
        .rows()
        .iter()
        .filter_map(|r| {
            if let RelationRow::First(r) = r {
                Some(r)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        first
            .iter()
            .filter(|r| r.unit_id == shared)
            .map(|r| (r.node_id, r.distance))
            .collect::<Vec<_>>(),
        vec![(node(2), 1), (node(3), 1)]
    );
    assert_eq!(
        first.iter().find(|r| r.unit_id == ending).unwrap().distance,
        3
    );
    assert!(!first.iter().any(|r| r.node_id == node(7)));
    let ending_node = projection
        .rows()
        .iter()
        .find_map(|r| {
            if let RelationRow::Node(r) = r {
                (r.node_id == node(5)).then_some(r)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(ending_node.unit_id, Some(ending));
    let cluster = projection
        .rows()
        .iter()
        .find_map(|r| {
            if let RelationRow::Cluster(r) = r {
                (r.cluster_id == ending_node.cluster_id).then_some(r)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(cluster.unit_id, None);
    assert_eq!(cluster.graph.as_deref(), Some("main"));
    assert!(
        projection
            .rows()
            .iter()
            .any(|r| matches!(r,RelationRow::Entry(r) if r.node_id == node(1)))
    );
    assert!(projection.rows().iter().any(|r| matches!(r,RelationRow::Ending(r) if r.class == narrata_graph::EndingClass::Unspecified && r.reachable)));
    let map = projection
        .parts()
        .iter()
        .map(|p| (p.id, p.bytes.clone()))
        .collect();
    let reread = read(&projection.index_object().bytes, &map).unwrap();
    reread.verify_program(&compilation.program).unwrap();
    assert_eq!(reread.rows(), projection.rows());
}

fn changed_part(
    projection: &CheckedRelations,
    kind: RelationKind,
    mutate: impl FnOnce(&mut Value),
) -> (Vec<u8>, BTreeMap<Digest, Vec<u8>>) {
    let mut index = projection.index().clone();
    let mut map: BTreeMap<_, _> = projection
        .parts()
        .iter()
        .map(|p| (p.id, p.bytes.clone()))
        .collect();
    let reference = index.parts.iter_mut().find(|p| p.kind == kind).unwrap();
    let mut part: Value = serde_json::from_slice(&map[&reference.object_id]).unwrap();
    mutate(&mut part);
    let bytes = canonical_json(&part).unwrap();
    map.remove(&reference.object_id);
    reference.object_id = relation_digest("narrata.nodes.relation-part", &bytes);
    reference.bytes = bytes.len() as u64;
    map.insert(reference.object_id, bytes);
    (canonical_json(&index).unwrap(), map)
}

#[test]
fn checked_read_rejects_unknown_columns_bad_ids_duplicate_or_unsorted_keys_and_bad_references() {
    let compilation = compile(&source(), &NodeRegistry::gamebook()).unwrap();
    let projection = project_relations(&compilation.program).unwrap();
    for mutate in [
        |v: &mut Value| {
            v["rows"][0]["extra"] = json!(true);
        },
        |v: &mut Value| {
            v["rows"][0]["node_id"] = json!("node:ff");
        },
        |v: &mut Value| {
            let first = v["rows"][0].clone();
            v["rows"][1] = first;
        },
        |v: &mut Value| {
            v["rows"].as_array_mut().unwrap().reverse();
        },
        |v: &mut Value| {
            v["rows"][0]["unit_id"] = json!("0".repeat(64));
        },
        |v: &mut Value| {
            v["rows"][0]["artifact_id"] = json!(format!("artifact:{}", "0".repeat(64)));
        },
        |v: &mut Value| {
            v["rows"][0]["projection_version"] = json!(2);
        },
        |v: &mut Value| {
            v["projection_version"] = json!(2);
        },
    ] {
        let (index, map) = changed_part(&projection, RelationKind::Node, mutate);
        assert!(read(&index, &map).is_err());
    }
    let (index, map) = changed_part(&projection, RelationKind::Edge, |v| {
        v["rows"][0]["edge_no"] = json!(42);
    });
    assert!(read(&index, &map).is_err());
    let mut map: BTreeMap<_, _> = projection
        .parts()
        .iter()
        .map(|p| (p.id, p.bytes.clone()))
        .collect();
    map.remove(&projection.parts()[0].id);
    assert!(read(&projection.index_object().bytes, &map).is_err());
    let mut invalid = projection.index().clone();
    invalid.parts[0].row_count = u64::MAX;
    assert!(read(&canonical_json(&invalid).unwrap(), &map).is_err());
    invalid = projection.index().clone();
    invalid.parts.remove(0);
    assert!(read(&canonical_json(&invalid).unwrap(), &map).is_err());
    let (index, map) = changed_part(&projection, RelationKind::Ending, |v| {
        v["rows"][0]["reachable"] = json!(false);
    });
    // Shape and foreign keys cannot prove reachability; reconstruction supplies that proof.
    let shaped = read(&index, &map).unwrap();
    assert!(shaped.verify_program(&compilation.program).is_err());
}

#[test]
fn calls_and_returns_are_transparent_transfers_and_program_reconstruction_needs_no_names() {
    let generated =
        narrata_node_bench::generate(narrata_node_bench::WorkSize::new(100).unwrap()).unwrap();
    let original = project_relations(&generated.program).unwrap();
    let mut pack = narrata_nodes::Pack::decode(&generated.pack).unwrap();
    pack.names = None;
    let program = narrata_nodes::Program::from_pack(&pack.encode()).unwrap().0;
    let rebuilt = project_relations(&program).unwrap();
    assert_eq!(original.index_object(), rebuilt.index_object());
    assert_eq!(original.parts(), rebuilt.parts());
    assert!(original.rows().iter().any(
        |r| matches!(r,RelationRow::Edge(e) if e.edge_kind == 0 && e.source_unit != e.target_unit)
    ));
}
