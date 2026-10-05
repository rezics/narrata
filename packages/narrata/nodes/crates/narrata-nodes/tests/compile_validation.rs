#![allow(clippy::panic, clippy::unwrap_used)]

mod support;

use narrata_nodes::{
    NodeRegistry, ProjectSource, Scalar, compile, source::NodeSource, validate_source,
};
use serde_json::json;

fn source() -> ProjectSource {
    ProjectSource {
        manifest: serde_json::from_value(support::manifest()).unwrap(),
        packages: [
            (
                "main".into(),
                serde_json::from_value(support::main_package()).unwrap(),
            ),
            (
                "side".into(),
                serde_json::from_value(support::side_package()).unwrap(),
            ),
        ]
        .into(),
    }
}

#[test]
fn collecting_validation_keeps_the_compilers_first_error_and_checks_other_fields_and_graphs() {
    let mut source = source();
    source.manifest.product.bindings.clear();
    let data = &mut source
        .packages
        .get_mut("main")
        .unwrap()
        .graphs
        .get_mut("start")
        .unwrap()
        .nodes
        .get_mut("gate")
        .unwrap()
        .data;
    data["choice_points"][0]["max"] = json!(999);
    data["choice_points"][0]["options"][0]["visible_if"] = support::literal(json!(42));
    data["choice_points"][0]["options"][1]["enabled_if"] = support::read("shared", "absent");
    data["choice_points"][2]["options"][0]["outcome"]["target"] = json!("missing");
    source
        .packages
        .get_mut("side")
        .unwrap()
        .graphs
        .get_mut("visit")
        .unwrap()
        .locals
        .insert(
            "large".into(),
            Scalar::Text("x".repeat(narrata_nodes::MAX_TEXT_BYTES + 1)),
        );
    let first = compile(&source, &NodeRegistry::gamebook()).unwrap_err();
    let report = validate_source(&source, &NodeRegistry::gamebook(), 100_000, 1_000);
    assert!(report.complete);
    assert!(report.errors.iter().any(|error| error.code == first.code
        && error.path.starts_with(&first.path)
        && error.message == first.message));
    for code in ["cardinality", "type", "reference", "binding", "limit"] {
        assert!(
            report.errors.iter().any(|error| error.code == code),
            "{report:?}"
        );
    }
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.path.starts_with("packages.side"))
    );
    assert!(
        report
            .skipped
            .iter()
            .any(|error| error.code == "checks_skipped")
    );
    assert_eq!(
        report.errors,
        validate_source(&source, &NodeRegistry::gamebook(), 100_000, 1_000).errors
    );
}

#[test]
fn invalid_node_data_does_not_hide_independent_nodes_or_cause_false_dangling_targets() {
    let mut source = source();
    let nodes = &mut source
        .packages
        .get_mut("main")
        .unwrap()
        .graphs
        .get_mut("start")
        .unwrap()
        .nodes;
    nodes.get_mut("gate").unwrap().data["unknown"] = json!(true);
    nodes.get_mut("won").unwrap().data["outcome"] = json!("unknown");
    let report = validate_source(&source, &NodeRegistry::gamebook(), 100_000, 1_000);
    assert!(report.errors.iter().any(|error| error.code == "schema"));
    assert!(report.errors.iter().any(|error| error.code == "contract"));
    assert!(
        !report
            .errors
            .iter()
            .any(|error| error.message.contains("target node"))
    );
    assert!(
        report
            .skipped
            .iter()
            .any(|error| error.path.ends_with("gate"))
    );
}

#[test]
fn source_and_diagnostic_budgets_never_claim_a_complete_report() {
    let mut source = source();
    source.manifest.format_version = 99;
    source.manifest.product.id = "invalid name".into();
    for (work, diagnostics) in [(0, 100), (1, 100), (1000, 0), (1000, 1)] {
        let report = validate_source(&source, &NodeRegistry::gamebook(), work, diagnostics);
        assert!(!report.complete && report.errors.iter().any(|error| error.code == "limit"));
    }
    let first = compile(&source, &NodeRegistry::gamebook()).unwrap_err();
    assert_eq!(first.code, "version");
    assert_eq!(first.path, "format_version");
    assert_eq!(first.message, "unsupported project format");
}

#[test]
fn independent_chunk_size_failures_are_all_reported_before_compiling() {
    let mut source = source();
    let arguments: serde_json::Map<String, serde_json::Value> = (0..128)
        .map(|index| {
            (
                format!("a{index}"),
                support::literal(json!("x".repeat(4096))),
            )
        })
        .collect();
    for (scope, graph, next, offset) in
        [("main", "start", "won", 100), ("side", "visit", "out", 200)]
    {
        let nodes = &mut source
            .packages
            .get_mut(scope)
            .unwrap()
            .graphs
            .get_mut(graph)
            .unwrap()
            .nodes;
        for index in 0..10 {
            nodes.insert(
                format!("large{index}"),
                NodeSource {
                    id: Some(support::node(offset + index).parse().unwrap()),
                    type_id: "narrata.passage".into(),
                    data: json!({"args":arguments,"next":next}),
                },
            );
        }
    }
    let report = validate_source(&source, &NodeRegistry::gamebook(), 100_000, 1_000);
    assert!(report.complete);
    assert_eq!(
        report
            .errors
            .iter()
            .filter(|error| error.message == "chunk exceeds 4 MiB")
            .count(),
        2,
        "{report:?}"
    );
    assert!(report.errors.iter().all(|error| error.code == "limit"));
    assert_eq!(
        compile(&source, &NodeRegistry::gamebook())
            .unwrap_err()
            .code,
        "limit"
    );
}

#[test]
fn operands_assignment_targets_and_values_are_checked_independently() {
    let mut source = source();
    let data = &mut source
        .packages
        .get_mut("main")
        .unwrap()
        .graphs
        .get_mut("start")
        .unwrap()
        .nodes
        .get_mut("gate")
        .unwrap()
        .data;
    data["choice_points"][0]["options"][0]["visible_if"] = json!({"kind":"binary","op":"and","left":support::read("shared","left_missing"),"right":support::read("shared","right_missing")});
    data["choice_points"][0]["options"][0]["effects"] = json!([{"target":{"scope":"local","name":"target_missing"},"value":support::read("shared","value_missing")}]);
    let first = compile(&source, &NodeRegistry::gamebook()).unwrap_err();
    assert!(first.message.contains("left_missing"));
    let report = validate_source(&source, &NodeRegistry::gamebook(), 100_000, 1_000);
    assert!(report.complete);
    for name in [
        "left_missing",
        "right_missing",
        "target_missing",
        "value_missing",
    ] {
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.message.contains(name)),
            "{report:?}"
        );
    }
    for tail in [
        "visible_if.left",
        "visible_if.right",
        "effects[0].target",
        "effects[0].value",
    ] {
        assert!(
            report.errors.iter().any(|error| error.path.ends_with(tail)),
            "{report:?}"
        );
    }
    assert!(
        report
            .skipped
            .iter()
            .any(|error| error.path.ends_with("visible_if"))
    );
}

#[test]
fn missing_entry_signatures_do_not_hide_independent_scalar_constraints() {
    let mut source = source();
    source.manifest.product.entry.graph = "missing".into();
    source.manifest.product.arguments.insert(
        "wide".into(),
        Scalar::Text("x".repeat(narrata_nodes::MAX_TEXT_BYTES + 1)),
    );
    let report = validate_source(&source, &NodeRegistry::gamebook(), 100_000, 1_000);
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.code == "reference" && error.path == "product.entry")
    );
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.code == "limit" && error.path == "product.arguments.wide")
    );
    assert!(
        report
            .skipped
            .iter()
            .any(|error| error.path == "product.entry")
    );
}
