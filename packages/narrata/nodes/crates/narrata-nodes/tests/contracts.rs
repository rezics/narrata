mod support;

use narrata_nodes::{Bundle, Error, NodePlan, NodeRegistry, Session, compile, parse_json};
use serde_json::json;
use support::{TestResult, click, compile_error, product, source};

#[test]
fn source_is_checked_and_analysis_keeps_call_and_return_edges_distinct() -> TestResult {
    let p = product(source())?;
    let graph = p
        .analysis()
        .iter()
        .find(|g| g.reference.package == "main")
        .ok_or("main graph missing")?;
    let call = graph
        .nodes
        .iter()
        .find(|n| n.id == "call")
        .ok_or("call missing")?;
    assert!(
        call.edges
            .iter()
            .any(|e| e.kind == "call" && e.target.package == "camp")
    );
    assert!(
        call.edges
            .iter()
            .any(|e| e.kind == "return" && e.target.package == "main")
    );
    let mut s = Session::new(p)?;
    assert_eq!(click(&mut s, "visit")?.title, "营地");
    Ok(())
}

#[test]
fn rejects_missing_duplicate_and_incompatible_providers() -> TestResult {
    let mut raw = source();
    raw["product"]["bindings"] = json!([]);
    assert_eq!(compile_error(raw)?, "binding");
    let mut raw = source();
    let binding = raw["product"]["bindings"][0].clone();
    raw["product"]["bindings"] = json!([binding.clone(), binding]);
    assert_eq!(compile_error(raw)?, "duplicate");
    let mut raw = source();
    raw["packages"]["camp"]["graphs"]["visit"]["parameters"]["name"] = json!("int");
    assert_eq!(compile_error(raw)?, "contract");
    let mut raw = source();
    raw["packages"]["camp"]["exports"] = json!([]);
    assert_eq!(compile_error(raw)?, "contract");
    Ok(())
}

#[test]
fn checks_targets_arguments_outcomes_templates_and_shared_types() -> TestResult {
    let mut raw = source();
    raw["packages"]["main"]["graphs"]["journey"]["nodes"]["call"]["data"]["on_return"] =
        json!({"unknown":"start"});
    assert_eq!(compile_error(raw)?, "contract");
    let mut raw = source();
    raw["packages"]["main"]["graphs"]["journey"]["nodes"]["call"]["data"]["arguments"]["name"] =
        json!({"kind":"literal","value":23});
    assert_eq!(compile_error(raw)?, "type");
    let mut raw = source();
    raw["packages"]["camp"]["graphs"]["visit"]["nodes"]["page"]["data"]["next"] = json!("missing");
    assert_eq!(compile_error(raw)?, "reference");
    let mut raw = source();
    raw["packages"]["camp"]["content"]["camp"]["title"] = json!("{{shared.notDeclared}}");
    assert_eq!(compile_error(raw)?, "reference");
    let mut raw = source();
    raw["product"]["shared"]["visits"] = json!(false);
    assert_eq!(compile_error(raw)?, "type");
    Ok(())
}

#[test]
fn parameters_are_immutable_and_assignment_types_are_checked() -> TestResult {
    let mut raw = source();
    raw["packages"]["camp"]["graphs"]["visit"]["nodes"]["increment"]["data"]["assignments"][0]["target"] =
        json!({"scope":"parameter","name":"name"});
    assert_eq!(compile_error(raw)?, "readonly");
    let mut raw = source();
    raw["packages"]["camp"]["graphs"]["visit"]["nodes"]["increment"]["data"]["assignments"][0]["value"] =
        json!({"kind":"literal","value":false});
    assert_eq!(compile_error(raw)?, "type");
    Ok(())
}

#[test]
fn boundary_rejects_duplicate_keys_unknown_fields_and_wrong_versions() -> TestResult {
    assert_eq!(
        parse_json::<Bundle>(r#"{"format_version":1,"format_version":2}"#)
            .err()
            .ok_or("expected error")?
            .code,
        "json"
    );
    assert!(parse_json::<Bundle>(&format!("{} trailing", source())).is_err());
    let mut raw = source();
    raw["packages"]["camp"]["graphs"]["visit"]["nodes"]["page"]["data"]["typo"] = json!(true);
    assert_eq!(compile_error(raw)?, "schema");
    let mut raw = source();
    raw["format_version"] = json!(500);
    assert_eq!(compile_error(raw)?, "version");
    Ok(())
}

fn alternate_content(data: &serde_json::Value) -> narrata_nodes::Result<NodePlan> {
    let content = data
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::new("schema", "data", "content required"))?;
    let next = data
        .get("next")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::new("schema", "data", "next required"))?;
    Ok(NodePlan::Content {
        content: content.into(),
        label: "替换类型".into(),
        next: next.into(),
    })
}

#[test]
fn registered_node_types_lower_without_bypassing_graph_validation() -> TestResult {
    let mut raw = source();
    raw["packages"]["camp"]["graphs"]["visit"]["nodes"]["page"]["type_id"] =
        json!("example.caption");
    let bundle: Bundle = parse_json(&raw.to_string())?;
    let mut registry = NodeRegistry::gamebook();
    assert!(compile(bundle.clone(), &registry).is_err());
    registry.register("example.caption", "1", alternate_content)?;
    assert!(
        registry
            .register("example.caption", "1", alternate_content)
            .is_err()
    );
    let p = compile(bundle, &registry)?.product;
    assert_eq!(
        p.lock()
            .node_types
            .get("example.caption")
            .map(String::as_str),
        Some("1")
    );
    raw["packages"]["camp"]["graphs"]["visit"]["nodes"]["page"]["data"]["next"] = json!("missing");
    let error = compile(parse_json(&raw.to_string())?, &registry)
        .err()
        .ok_or("expected error")?;
    assert_eq!(error.code, "reference");
    Ok(())
}

#[test]
fn compiled_identity_survives_json_roundtrip_and_tracks_semantic_changes() -> TestResult {
    let p = product(source())?;
    let roundtrip: Bundle = parse_json(&serde_json::to_string_pretty(p.source())?)?;
    let p2 = compile(roundtrip, &NodeRegistry::gamebook())?.product;
    assert_eq!(p.artifact_id(), p2.artifact_id());
    let mut raw = source();
    raw["product"]["shared"]["visits"] = json!(2);
    assert_ne!(p.artifact_id(), product(raw)?.artifact_id());
    Ok(())
}
