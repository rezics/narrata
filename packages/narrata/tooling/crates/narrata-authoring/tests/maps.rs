#![allow(clippy::unwrap_used, clippy::panic)]
#[path = "../../../../nodes/crates/narrata-nodes/tests/support/mod.rs"]
mod support;
use narrata_authoring::{
    maps::*,
    project_relations,
    relations::{ReaderRelationAllowlist, RelationRow},
};
use narrata_nodes::{
    Session,
    history::{evolution::*, visits::*},
};
use serde_json::{Value, json};
use support::*;

fn visits(session: &Session) -> CheckedVisits {
    let mut registry = EvolutionRegistry::new();
    registry
        .register_program(session.program().clone())
        .unwrap();
    registry
        .visited_from_save(
            session.history(),
            session.cursor().unwrap(),
            PathLimits::default(),
        )
        .unwrap()
}

fn assert_no_hidden(value: &Value, forbidden: &[String]) {
    match value {
        Value::String(text) => assert!(!forbidden.contains(text), "hidden string {text}"),
        Value::Array(values) => {
            for value in values {
                assert_no_hidden(value, forbidden);
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                assert!(
                    ![
                        "cluster_id",
                        "routes",
                        "first_anchor",
                        "last_anchor",
                        "title",
                        "target_unit",
                        "target_node",
                        "node_count",
                        "bounds"
                    ]
                    .contains(&key.as_str()),
                    "hidden field {key}"
                );
                assert_no_hidden(value, forbidden);
            }
        }
        _ => {}
    }
}

#[test]
fn map_recursively_omits_hidden_content_topology_and_future_anchors() {
    let compilation = compiled();
    let (mut session, names) = session(&compilation);
    let projection = project_relations(session.program()).unwrap();
    let initial = reader_map(&projection, &visits(&session)).unwrap();
    assert_eq!(initial.nodes.len(), 1);
    assert!(initial.edges.is_empty());
    assert!(initial.endings.is_empty());
    assert_eq!(
        initial.units[0].segments[0].last.as_ref().unwrap().as_str(),
        "b1"
    );
    assert_no_hidden(
        &serde_json::to_value(&initial).unwrap(),
        &[
            node(2),
            node(3),
            node(4),
            node(5),
            node(6),
            node(11),
            node(12),
            "side.talk".into(),
            "ending".into(),
            "b2".into(),
            "b3".into(),
            "b4".into(),
            "r-rope".into(),
        ],
    );
    choose(&mut session, &names, &["wave"]).unwrap();
    let current = reader_map(&projection, &visits(&session)).unwrap();
    assert!(
        current.sources[0]
            .frontier
            .iter()
            .any(|f| f.option.to_string() == option(5) && !f.enabled)
    );
    for (old, new) in initial.nodes.iter().zip(&current.nodes) {
        assert_eq!((old.node, old.x, old.y), (new.node, new.x, new.y));
    }
    assert_no_hidden(
        &serde_json::to_value(&current.sources[0].frontier).unwrap(),
        &[node(2), node(5), node(11), "side.talk".into()],
    );
}

#[test]
fn actual_control_and_same_unit_edges_are_retained_but_other_choices_are_not_inferred() {
    let compilation = try_variant(|_, main, _| {
        // Distinct options share a target: visiting both endpoints must not imply both edges.
        set(
            main,
            "/graphs/start/nodes/gate/data/choice_points/2/options/1/outcome",
            "target",
            json!("visit"),
        );
    })
    .unwrap();
    let (mut session, names) = session(&compilation);
    let projection = project_relations(session.program()).unwrap();
    for keys in [&["wave"][..], &["rope", "lamp"][..], &["left"][..]] {
        choose(&mut session, &names, keys).unwrap();
    }
    let evidence = visits(&session);
    let map = reader_map(&projection, &evidence).unwrap();
    assert_eq!(
        map.nodes
            .iter()
            .map(|n| n.node.to_string())
            .collect::<Vec<_>>(),
        [node(1), node(2), node(11)]
    );
    assert_eq!(map.edges.len(), 2);
    assert!(
        map.edges
            .iter()
            .any(|e| e.option.map(|o| o.to_string()) == Some(option(6)))
    );
    assert!(
        !map.edges
            .iter()
            .any(|e| e.option.map(|o| o.to_string()) == Some(option(7)))
    );
    let allowlist = ReaderRelationAllowlist::from_visits(&projection, &evidence).unwrap();
    let sparse = allowlist.check_rows(allowlist.rows().to_vec()).unwrap();
    assert_eq!(reader_map_from_subset(&sparse, &evidence).unwrap(), map);
    let edges: Vec<_> = sparse
        .rows()
        .iter()
        .filter_map(|r| match r {
            RelationRow::Edge(e) => Some(e),
            _ => None,
        })
        .collect();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].option.unwrap().to_string(), option(6));
}

#[test]
fn same_unit_automatic_nodes_are_drawn_from_trace_even_without_relation_edges() {
    let compilation = try_variant(|_, main, side| {
        set(main, "/graphs/start", "entry", json!("visit"));
        set(side, "/graphs/visit/nodes", "second", json!({"id":node(13),"type_id":"narrata.passage","data":{"body":{"unit":content("side.talk")},"choice_points":[{"id":point(13),"key":"continue","options":[{"id":option(13),"key":"continue","outcome":{"kind":"branch","target":"out"}}]}]}}));
        set(side, "/graphs/visit/nodes/talk/data", "choice_points", json!([]));
        set(side, "/graphs/visit/nodes/talk/data", "next", json!("second"));
    }).unwrap();
    let (session, _) = session(&compilation);
    let projection = project_relations(session.program()).unwrap();
    let map = reader_map(&projection, &visits(&session)).unwrap();
    assert!(
        map.edges
            .iter()
            .any(|e| e.source.to_string() == node(11) && e.target.to_string() == node(13))
    );
    assert!(!projection.rows().iter().any(|r| matches!(r, RelationRow::Edge(e) if e.source_node.to_string() == node(11) && e.target_node.to_string() == node(13))));
}

#[test]
fn sparse_results_reject_missing_duplicate_reordered_foreign_and_renumbered_rows() {
    let compilation = compiled();
    let (mut session, names) = session(&compilation);
    for keys in [&["wave"][..], &["rope", "lamp"][..], &["right"][..]] {
        choose(&mut session, &names, keys).unwrap();
    }
    let projection = project_relations(session.program()).unwrap();
    let allowlist = ReaderRelationAllowlist::from_visits(&projection, &visits(&session)).unwrap();
    let rows = allowlist.rows();
    let edge = rows
        .iter()
        .find_map(|r| match r {
            RelationRow::Edge(e) => Some(e),
            _ => None,
        })
        .unwrap();
    assert!(edge.edge_no > 0, "the permitted set must really be sparse");
    let encoded = serde_json::to_vec(rows).unwrap();
    assert_eq!(allowlist.read_rows(&encoded).unwrap().rows(), rows);
    let mut missing = rows.to_vec();
    missing.remove(0);
    assert!(allowlist.check_rows(missing).is_err());
    let mut duplicate = rows.to_vec();
    duplicate.insert(0, rows[0].clone());
    assert!(allowlist.check_rows(duplicate).is_err());
    let mut reordered = rows.to_vec();
    reordered.swap(0, 1);
    assert!(allowlist.check_rows(reordered).is_err());
    let mut foreign = rows.to_vec();
    let RelationRow::Unit(unit) = &mut foreign[0] else {
        panic!()
    };
    unit.artifact_id = narrata_nodes::ArtifactId::from_bytes([99; 32]);
    assert!(allowlist.check_rows(foreign).is_err());
    let mut renumbered = rows.to_vec();
    for row in &mut renumbered {
        if let RelationRow::Edge(e) = row {
            e.edge_no = 0;
        }
    }
    assert!(allowlist.check_rows(renumbered).is_err());
    let mut leaked = rows.to_vec();
    leaked.extend(
        projection
            .rows()
            .iter()
            .filter(|r| matches!(r, RelationRow::Route(_)))
            .cloned(),
    );
    assert!(allowlist.check_rows(leaked).is_err());
}
