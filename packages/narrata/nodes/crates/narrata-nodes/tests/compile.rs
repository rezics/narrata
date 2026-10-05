#![allow(clippy::panic, clippy::unwrap_used)]

mod support;

use narrata_nodes::{AuthoredId, Lookup, NodeRegistry, Owner, Program};
use serde_json::{Value, json};
use support::*;

const GREET: &str = "/graphs/start/nodes/gate/data/choice_points/0";
const PACK: &str = "/graphs/start/nodes/gate/data/choice_points/1";
const ROUTE: &str = "/graphs/start/nodes/gate/data/choice_points/2";
const TALK: &str = "/graphs/visit/nodes/talk/data";

fn remove(value: &mut Value, pointer: &str, field: &str) {
    at(value, pointer)
        .as_object_mut()
        .unwrap_or_else(|| panic!("{pointer} is not an object"))
        .remove(field);
}

#[test]
fn the_fixture_compiles_to_a_text_free_artifact() {
    let compilation = compiled();
    assert!(
        compilation.diagnostics.is_empty(),
        "{:?}",
        compilation.diagnostics
    );
    assert_eq!(compilation.lock.format_version, 2);
    assert_eq!(
        compilation
            .lock
            .node_types
            .get("narrata.passage")
            .map(String::as_str),
        Some("2")
    );
    let pack = narrata_nodes::Pack::decode(&compilation.pack).unwrap();
    // One chunk per graph; aliases live only in the name table.
    assert_eq!(pack.chunks.len(), 2);
    for object in pack.chunks.iter().chain([&pack.manifest]) {
        let text = String::from_utf8_lossy(object);
        assert!(
            ["tally", "greet", "route"]
                .iter()
                .all(|alias| !text.contains(alias))
        );
    }
    assert!(
        pack.names
            .is_some_and(|names| String::from_utf8_lossy(&names).contains("tally"))
    );
}

#[test]
fn compile_never_mints_ids() {
    assert_eq!(
        rejected(|_, main, _| remove(main, "/graphs/start/nodes/won", "id")),
        "missing_id"
    );
    assert_eq!(
        rejected(|_, main, _| remove(main, GREET, "id")),
        "missing_id"
    );
    assert_eq!(
        rejected(|_, main, _| remove(main, &format!("{GREET}/options/0"), "id")),
        "missing_id"
    );
}

#[test]
fn an_id_is_unique_across_the_whole_work() {
    assert_eq!(
        rejected(|_, _, side| *at(side, "/graphs/visit/nodes/out/id") = json!(node(1))),
        "duplicate"
    );
    assert_eq!(
        rejected(|_, _, side| *at(side, &format!("{TALK}/choice_points/0/id")) = json!(point(2))),
        "duplicate"
    );
    assert_eq!(
        rejected(|_, main, _| *at(main, &format!("{ROUTE}/options/1/id")) = json!(option(6))),
        "duplicate"
    );
}

#[test]
fn malformed_ids_are_rejected_by_the_source_schema() {
    assert_eq!(
        rejected(|_, main, _| *at(main, "/graphs/start/nodes/won/id") = json!("node:1")),
        "schema"
    );
    assert_eq!(
        rejected(
            |_, main, _| *at(main, "/graphs/start/nodes/won/id") = json!(node(5).to_uppercase())
        ),
        "schema"
    );
}

#[test]
fn a_live_id_cannot_also_be_a_tombstone() {
    assert_eq!(
        rejected(|_, main, _| set(main, "", "tombstones", json!([node(1)]))),
        "tombstone"
    );
    assert_eq!(
        rejected(|_, _, side| set(side, "", "tombstones", json!([option(1)]))),
        "tombstone"
    );
    assert_eq!(
        rejected(|_, main, _| set(main, "", "tombstones", json!([node(90), node(90)]))),
        "duplicate"
    );
    try_variant(|_, main, _| set(main, "", "tombstones", json!([node(90), option(90)]))).unwrap();
}

#[test]
fn renaming_aliases_keeps_the_artifact_id_and_changes_only_names_and_lock() {
    let base = compiled();
    let renamed = try_variant(|_, main, _| {
        *at(main, &format!("{GREET}/key")) = json!("salute");
        *at(main, &format!("{GREET}/options/0/key")) = json!("bow");
        let nodes = at(main, "/graphs/start/nodes").as_object_mut().unwrap();
        let tally = nodes.remove("tally").unwrap();
        nodes.insert("record".into(), tally);
        *at(main, "/graphs/start/nodes/visit/data/on_return/back") = json!("record");
    })
    .unwrap();
    assert_eq!(base.program.artifact_id(), renamed.program.artifact_id());
    assert_ne!(base.lock, renamed.lock);
    assert_ne!(base.names, renamed.names);
}

#[test]
fn changing_a_content_reference_changes_the_artifact_id() {
    let base = compiled();
    let moved =
        try_variant(|_, main, _| *at(main, &format!("{GATE}/title")) = content("elsewhere.title"))
            .unwrap();
    assert_ne!(base.program.artifact_id(), moved.program.artifact_id());
}

#[test]
fn only_the_last_choice_point_may_omit_its_placement() {
    assert_eq!(
        rejected(|_, main, _| remove(main, GREET, "placement")),
        "placement"
    );
    try_variant(|_, main, _| set(main, ROUTE, "placement", json!("b4"))).unwrap();
}

#[test]
fn a_passage_that_can_run_to_its_end_needs_a_next_node() {
    let local_route = |main: &mut Value| {
        for option in 0..2 {
            *at(main, &format!("{ROUTE}/options/{option}/outcome")) =
                json!({"kind": "local", "rejoin": "b4"});
        }
    };
    assert_eq!(rejected(|_, main, _| local_route(main)), "contract");
    try_variant(|_, main, _| {
        local_route(main);
        set(main, GATE, "next", json!("count"));
    })
    .unwrap();
    assert_eq!(
        rejected(|_, main, _| set(main, GATE, "next", json!("count"))),
        "contract"
    );
    assert_eq!(
        rejected(|_, main, _| {
            local_route(main);
            set(main, GATE, "next", json!("nowhere"));
        }),
        "reference"
    );
}

#[test]
fn cardinality_requires_one_to_option_count_and_min_at_most_max() {
    assert_eq!(
        rejected(|_, main, _| set(main, GREET, "max", json!(0))),
        "cardinality"
    );
    assert_eq!(
        rejected(|_, main, _| set(main, PACK, "max", json!(4))),
        "cardinality"
    );
    assert_eq!(
        rejected(|_, main, _| set(main, GREET, "min", json!(2))),
        "cardinality"
    );
    try_variant(|_, main, _| set(main, PACK, "max", json!(3))).unwrap();
}

#[test]
fn multiple_selection_needs_local_options_sharing_one_rejoin() {
    assert_eq!(
        rejected(
            |_, main, _| *at(main, &format!("{PACK}/options/2/outcome")) =
                json!({"kind": "branch", "target": "count"})
        ),
        "cardinality"
    );
    assert_eq!(
        rejected(|_, main, _| *at(main, &format!("{PACK}/options/2/outcome/rejoin")) = json!("b4")),
        "cardinality"
    );
    // min = 0 with a single selection follows the same rule.
    assert_eq!(
        rejected(|_, main, _| {
            set(main, GREET, "min", json!(0));
            *at(main, &format!("{GREET}/options/1/outcome/rejoin")) = json!("b3");
        }),
        "cardinality"
    );
    // Omitting every rejoin is a shared rejoin too.
    try_variant(|_, main, _| {
        for option in 0..3 {
            remove(main, &format!("{PACK}/options/{option}/outcome"), "rejoin");
        }
    })
    .unwrap();
}

#[test]
fn every_option_of_a_choice_point_with_two_or_more_options_has_a_label() {
    assert_eq!(
        rejected(|_, main, _| remove(main, &format!("{GREET}/options/0"), "label")),
        "label"
    );
}

#[test]
fn local_outcomes_need_a_body_and_replies_in_its_unit() {
    assert_eq!(
        rejected(|_, _, side| {
            remove(side, TALK, "body");
            *at(side, &format!("{TALK}/choice_points/0/options/0/outcome")) =
                json!({"kind": "local"});
            set(side, TALK, "next", json!("out"));
        }),
        "local"
    );
    assert_eq!(
        rejected(
            |_, main, _| *at(main, &format!("{GREET}/options/0/outcome/reply")) =
                segment("other", "x", "x")
        ),
        "local"
    );
}

#[test]
fn conditions_and_effects_are_type_checked() {
    assert_eq!(
        rejected(|_, main, _| set(
            main,
            &format!("{GREET}/options/0"),
            "visible_if",
            literal(json!(1))
        )),
        "type"
    );
    assert_eq!(
        rejected(
            |_, main, _| *at(main, &format!("{GREET}/options/0/effects/0/target/name")) =
                json!("lamp")
        ),
        "type"
    );
    assert_eq!(
        rejected(|_, main, _| set(
            main,
            &format!("{GREET}/options/0"),
            "visible_if",
            read("shared", "missing")
        )),
        "reference"
    );
    // A `ref` only supports equality.
    assert_eq!(
        rejected(|_, _, side| set(
            side,
            TALK,
            "args",
            json!({"x": {"kind": "binary", "op": "lt", "left": read("parameter", "visitor"), "right": read("parameter", "visitor")}})
        )),
        "type"
    );
    assert_eq!(
        rejected(|_, _, side| set(
            side,
            &format!("{TALK}/choice_points/0/options/0"),
            "effects",
            json!([{"target": {"scope": "parameter", "name": "visitor"}, "value": literal(content("x"))}])
        )),
        "readonly"
    );
}

#[test]
fn branch_targets_must_exist_in_the_same_graph() {
    assert_eq!(
        rejected(
            |_, main, _| *at(main, &format!("{ROUTE}/options/0/outcome/target")) = json!("talk")
        ),
        "reference"
    );
}

#[test]
fn accepting_proposals_is_part_of_the_artifact() {
    let open = try_variant(|_, main, _| set(main, ROUTE, "proposals", json!(true))).unwrap();
    assert_ne!(open.program.artifact_id(), compiled().program.artifact_id());
}

#[test]
fn calls_go_through_bound_imports_with_matching_signatures() {
    assert_eq!(
        rejected(|manifest, _, _| *at(manifest, "/product/bindings") = json!([])),
        "binding"
    );
    assert_eq!(
        rejected(
            |_, main, _| *at(main, "/graphs/start/imports/helper/parameters/visitor") =
                json!("text")
        ),
        "contract"
    );
    assert_eq!(
        rejected(
            |_, main, _| *at(main, "/graphs/start/nodes/visit/data/arguments/visitor") =
                literal(json!("you"))
        ),
        "type"
    );
}

#[test]
fn endings_name_outcomes_of_the_entry_graph() {
    assert_eq!(
        rejected(|manifest, _, _| set(manifest, "/product/endings", "escaped", json!({}))),
        "reference"
    );
}

#[test]
fn node_types_must_be_installed_and_own_their_kind() {
    assert_eq!(
        rejected(
            |_, main, _| *at(main, "/graphs/start/nodes/won/type_id") = json!("narrata.unknown")
        ),
        "node_type"
    );
    assert_eq!(
        rejected(|_, main, _| set(
            main,
            "/graphs/start/nodes/won/data",
            "kind",
            json!("return")
        )),
        "schema"
    );
}

#[test]
fn choice_points_and_options_are_bounded() {
    assert_eq!(
        rejected(|_, main, _| {
            let template = at(main, &format!("{PACK}/options/0")).clone();
            let options: Vec<Value> = (0..129)
                .map(|n| {
                    let mut option = template.clone();
                    option["id"] = json!(option_id(1000 + n));
                    option["key"] = json!(format!("o{n}"));
                    option
                })
                .collect();
            *at(main, &format!("{PACK}/options")) = json!(options);
        }),
        "limit"
    );
}

fn option_id(n: u32) -> String {
    option(n)
}

#[test]
fn aliases_are_unique_within_their_scope() {
    assert_eq!(
        rejected(|_, main, _| *at(main, &format!("{PACK}/key")) = json!("greet")),
        "duplicate"
    );
    assert_eq!(
        rejected(|_, main, _| *at(main, &format!("{GREET}/options/1/key")) = json!("wave")),
        "duplicate"
    );
}

#[test]
fn unknown_source_fields_are_rejected() {
    assert_eq!(
        rejected(|_, main, _| set(main, GREET, "label", content("x"))),
        "schema"
    );
}

#[test]
fn lookup_finds_live_deleted_and_unknown_ids() {
    let compilation =
        try_variant(|_, main, _| set(main, "", "tombstones", json!([option(90)]))).unwrap();
    let program = &compilation.program;
    let id = |text: &str| text.parse::<AuthoredId>().unwrap();
    assert!(matches!(
        program.lookup(id(&option(1))).unwrap(),
        Lookup::Live {
            choice_point: Some(_),
            ..
        }
    ));
    assert_eq!(program.lookup(id(&option(90))).ok(), Some(Lookup::Deleted));
    assert_eq!(program.lookup(id(&node(77))).ok(), Some(Lookup::Unknown));
    let owners = program.owners().unwrap();
    assert!(matches!(owners.get(&id(&point(11))), Some(Owner::Node(_))));
    assert!(matches!(owners.get(&id(&node(11))), Some(Owner::Graph(_))));
}

#[test]
fn verify_artifact_accepts_compiled_packs() {
    let compilation = compiled();
    let (program, _) = Program::from_pack(&compilation.pack).unwrap();
    program.verify_artifact().unwrap();
}

#[test]
fn the_analysis_keeps_the_graph_inspector_usable_without_text() {
    let compilation = compiled();
    let graph = compilation
        .analysis
        .graphs
        .iter()
        .find(|graph| graph.reference.graph == "start")
        .unwrap();
    let gate = graph
        .nodes
        .iter()
        .find(|node| node.key.as_deref() == Some("gate"))
        .unwrap();
    assert_eq!(gate.edges.len(), 2);
    assert!(gate.edges.iter().all(|edge| edge.label.is_some()));
    let text = serde_json::to_string(&compilation.analysis).unwrap();
    assert!(text.contains("\"provider\":\"local\""));
}

#[test]
fn unreachable_nodes_are_diagnosed_structurally() {
    let compilation = try_variant(|_, main, _| {
        set(
            main,
            "/graphs/start/nodes",
            "orphan",
            json!({"id": node(40), "type_id": "narrata.return", "data": {"outcome": "lost"}}),
        );
    })
    .unwrap();
    assert_eq!(compilation.diagnostics.len(), 1);
    assert_eq!(compilation.diagnostics[0].code, "structurally_unreachable");
    assert!(compilation.diagnostics[0].path.ends_with("nodes.orphan"));
}

#[test]
fn an_empty_registry_installs_no_node_types() {
    let project = narrata_nodes::ProjectSource {
        manifest: serde_json::from_value(manifest()).unwrap(),
        packages: [
            (
                "main".to_owned(),
                serde_json::from_value(main_package()).unwrap(),
            ),
            (
                "side".to_owned(),
                serde_json::from_value(side_package()).unwrap(),
            ),
        ]
        .into(),
    };
    let error = narrata_nodes::compile(&project, &NodeRegistry::empty())
        .err()
        .unwrap();
    assert_eq!(error.code, "node_type");
}
