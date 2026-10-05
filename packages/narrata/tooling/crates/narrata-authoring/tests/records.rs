#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};

use narrata_authoring::{
    CheckedRecord, CheckedRecords, DraftRecord, MAX_RECORD_BYTES, assemble_records,
    derive_draft_indexes,
    records::{DraftVersion, TargetStatus, normalize_source},
    split_source,
    validate::{ValidationBudget, validate_project_with_budget},
    validate_project, validate_record,
};
use narrata_nodes::{NodeRegistry, ProjectSource, canonical_json, compile};
use serde_json::json;

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../..")
}
fn source() -> ProjectSource {
    narrata_node_tools::load_project(&repository().join("products/gamebook-demo/project.json"))
        .unwrap()
        .source
}
fn bytes(records: &[CheckedRecord]) -> Vec<&[u8]> {
    records.iter().map(CheckedRecord::bytes).collect()
}

#[test]
fn split_assemble_preserves_source_pack_lock_and_author_order() {
    let source = source();
    let records = split_source(&source).into_result().unwrap();
    let assembled = assemble_records(&records).into_result().unwrap();
    assert_eq!(assembled, normalize_source(&source).unwrap());
    let original = compile(&source, &NodeRegistry::gamebook()).unwrap();
    let rebuilt = compile(&assembled, &NodeRegistry::gamebook()).unwrap();
    assert_eq!(rebuilt.pack, original.pack);
    assert_eq!(rebuilt.lock, original.lock);
    let again = split_source(&assembled).into_result().unwrap();
    assert_eq!(bytes(&records), bytes(&again));
    let mut shuffled = records.clone();
    shuffled.reverse();
    assert_eq!(
        assemble_records(&shuffled).into_result().unwrap(),
        assembled
    );
    assert!(validate_project(&bytes(&records)).value.is_some());
}

#[test]
fn strict_json_rejects_duplicate_keys_unknown_fields_versions_and_unminted_ids() {
    let records = split_source(&source()).into_result().unwrap();
    let valid = &records[0];
    let mut value: serde_json::Value = serde_json::from_slice(valid.bytes()).unwrap();
    value["extra"] = json!(true);
    assert!(
        validate_record(&canonical_json(&value).unwrap())
            .value
            .is_none()
    );
    value.as_object_mut().unwrap().remove("extra");
    value["payload"]["product"]["extra"] = json!(true);
    assert!(
        validate_record(&canonical_json(&value).unwrap())
            .value
            .is_none()
    );
    let duplicate = String::from_utf8(valid.bytes().to_vec()).unwrap().replacen(
        "{",
        "{\"format_version\":1,",
        1,
    );
    assert_eq!(
        validate_record(duplicate.as_bytes()).diagnostics[0].code,
        "json"
    );
    let nested_duplicate = String::from_utf8(valid.bytes().to_vec())
        .unwrap()
        .replace("\"product\":{", "\"product\":{\"id\":\"discarded\",");
    assert_eq!(
        validate_record(nested_duplicate.as_bytes()).diagnostics[0].code,
        "json"
    );
    for bad in [b"\xff".as_slice(), b"{}{}", b"{\"format_version\":2,\"kind\":\"tombstone\",\"package\":\"main\",\"payload\":\"node:00000000000000000000000000000001\"}"] {
        assert!(validate_record(bad).value.is_none());
    }
    for checked in records.iter().filter(|record| {
        matches!(
            record.record(),
            DraftRecord::Node { .. } | DraftRecord::ChoicePoint { .. }
        )
    }) {
        let mut value: serde_json::Value = serde_json::from_slice(checked.bytes()).unwrap();
        value["payload"].as_object_mut().unwrap().remove("id");
        assert!(
            validate_record(&canonical_json(&value).unwrap())
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "missing_id")
        );
    }
}

#[test]
fn the_one_mib_boundary_is_exact_in_utf8_bytes() {
    let node = narrata_nodes::source::NodeSource {
        id: Some(narrata_nodes::NodeId::from_bytes([1; 16])),
        type_id: "extension.custom".into(),
        data: json!({"opaque":""}),
    };
    let mut record = DraftRecord::Node {
        format_version: DraftVersion,
        package: "main".into(),
        graph: "journey".into(),
        key: "entry".into(),
        choice_order: None,
        payload: node,
    };
    let overhead = canonical_json(&record).unwrap().len();
    if let DraftRecord::Node { payload, .. } = &mut record {
        payload.data["opaque"] = json!("x".repeat(MAX_RECORD_BYTES - overhead));
    }
    let at_limit = canonical_json(&record).unwrap();
    assert_eq!(at_limit.len(), MAX_RECORD_BYTES);
    assert!(validate_record(&at_limit).value.is_some());
    let mut over = at_limit;
    over.push(b' ');
    let report = validate_record(&over);
    assert!(report.value.is_none());
    assert_eq!(report.diagnostics[0].code, "record_too_large");
    if let DraftRecord::Node { payload, .. } = &mut record {
        payload.data["opaque"] = json!("山".repeat(MAX_RECORD_BYTES / 3));
    }
    assert_eq!(
        CheckedRecord::new(record).diagnostics[0].code,
        "record_too_large"
    );
}

#[test]
fn assembly_reports_duplicates_missing_orphan_and_wrong_owners() {
    let records = split_source(&source()).into_result().unwrap();
    let mut duplicate = records.clone();
    duplicate.push(records[0].clone());
    let report = assemble_records(&duplicate);
    assert!(report.value.is_none());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "duplicate" && !diagnostic.related.is_empty())
    );
    let index = records
        .iter()
        .position(|record| matches!(record.record(), DraftRecord::ChoicePoint { .. }))
        .unwrap();
    let mut missing = records.clone();
    missing.remove(index);
    assert!(
        assemble_records(&missing)
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "reference")
    );
    let mut wrong = records[index].record().clone();
    if let DraftRecord::ChoicePoint { node, .. } = &mut wrong {
        *node = narrata_nodes::NodeId::from_bytes([99; 16]);
    }
    let mut wrong_set = records.clone();
    wrong_set[index] = CheckedRecord::new(wrong).into_result().unwrap();
    assert!(
        assemble_records(&wrong_set)
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "owner")
    );
    let mut orphan = records[index].record().clone();
    if let DraftRecord::ChoicePoint { payload, .. } = &mut orphan {
        payload.id = Some(narrata_nodes::ChoicePointId::from_bytes([98; 16]));
        for (index, option) in payload.options.iter_mut().enumerate() {
            option.id = Some(narrata_nodes::OptionId::from_bytes([index as u8 + 100; 16]));
        }
    }
    let mut orphan_set = records.clone();
    orphan_set.push(CheckedRecord::new(orphan).into_result().unwrap());
    assert!(
        assemble_records(&orphan_set)
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "orphan")
    );
}

#[test]
fn indexes_include_every_option_resolve_graph_aliases_and_mark_control_and_unresolved() {
    let mut source = source();
    let node = source
        .packages
        .get_mut("road")
        .unwrap()
        .graphs
        .get_mut("crossing")
        .unwrap()
        .nodes
        .get_mut("rocks")
        .unwrap();
    node.data["choice_points"][0]["options"][0]["outcome"]["target"] = json!("absent");
    let checked = CheckedRecords::new(split_source(&source).into_result().unwrap())
        .into_result()
        .unwrap();
    let indexes = derive_draft_indexes(&checked);
    let total = checked
        .records()
        .iter()
        .filter_map(|record| match record.record() {
            DraftRecord::ChoicePoint { payload, .. } => Some(payload.options.len()),
            _ => None,
        })
        .sum::<usize>();
    assert_eq!(indexes.options.len(), total);
    assert!(
        indexes
            .options
            .iter()
            .any(|row| row.target_status == TargetStatus::Control
                && row.target_node.is_some()
                && row.target_unit.is_none())
    );
    assert!(
        indexes
            .options
            .iter()
            .any(|row| row.target_status == TargetStatus::Passage && row.target_unit.is_some())
    );
    assert!(
        indexes
            .options
            .iter()
            .any(|row| row.target_status == TargetStatus::Local
                && row.target_node.is_none()
                && row.target_unit.is_none())
    );
    assert!(
        indexes
            .options
            .iter()
            .any(|row| row.target_status == TargetStatus::Unresolved && row.target_node.is_none())
    );
    assert!(indexes.options.iter().all(|row| row.source_unit.is_some()));
}

#[test]
fn project_validation_collects_independent_fields_records_and_syntax_failures() {
    let mut source = source();
    let node = source
        .packages
        .get_mut("road")
        .unwrap()
        .graphs
        .get_mut("crossing")
        .unwrap()
        .nodes
        .get_mut("rocks")
        .unwrap();
    node.data["choice_points"][0]["options"][0]["visible_if"] = json!({"kind":"literal","value":4});
    node.data["choice_points"][0]["options"][1]["enabled_if"] =
        json!({"kind":"read","scope":"shared","name":"absent"});
    node.data["choice_points"][0]["options"][2]["outcome"]["target"] = json!("absent");
    let records = split_source(&source).into_result().unwrap();
    let report = validate_project(&bytes(&records));
    assert!(report.complete && report.value.is_none());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "type"
                && diagnostic.location.option.is_some()
                && diagnostic.location.pointer.ends_with("visible_if"))
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "reference"
                && diagnostic.location.pointer.ends_with("enabled_if"))
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "reference"
                && diagnostic.message.contains("target node absent"))
    );
    let mut inputs = bytes(&records);
    inputs.push(b"{bad");
    inputs.push(b"{also bad");
    let report = validate_project(&inputs);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "type")
    );
    assert_eq!(
        report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "json")
            .count(),
        2
    );
    assert!(
        serde_json::to_value(report.diagnostic_report())
            .unwrap()
            .get("complete")
            .is_some()
    );
}

#[test]
fn diagnostic_exchange_has_flat_identity_locations_and_round_trips() {
    let records = split_source(&source()).into_result().unwrap();
    let record = records
        .iter()
        .find(|record| matches!(record.record(), DraftRecord::ChoicePoint { .. }))
        .unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(record.bytes()).unwrap();
    value["payload"]["options"][0]
        .as_object_mut()
        .unwrap()
        .remove("id");
    let report = validate_record(&canonical_json(&value).unwrap());
    let diagnostic = &report.diagnostics[0];
    let encoded = serde_json::to_value(diagnostic).unwrap();
    assert!(encoded.get("location").is_none());
    for field in [
        "severity",
        "code",
        "record",
        "pointer",
        "node",
        "choice_point",
        "related",
        "message",
    ] {
        assert!(encoded.get(field).is_some(), "{field}");
    }
    let decoded: narrata_authoring::Diagnostic = serde_json::from_value(encoded).unwrap();
    assert_eq!(&decoded, diagnostic);
}

#[test]
fn local_record_work_and_project_work_budgets_are_accounted_across_records() {
    let records = split_source(&source()).into_result().unwrap();
    let budget = ValidationBudget {
        max_checks: 1,
        ..ValidationBudget::default()
    };
    let record =
        narrata_authoring::validate::validate_record_with_budget(records[0].bytes(), budget);
    assert!(!record.complete && record.value.is_none());
    assert!(
        record
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "limit")
    );
    let budget = ValidationBudget {
        max_checks: 250,
        ..ValidationBudget::default()
    };
    let project = validate_project_with_budget(&bytes(&records), &NodeRegistry::gamebook(), budget);
    assert!(!project.complete && project.value.is_none());
}

#[test]
fn local_headers_collect_independent_metadata_errors_before_assembly() {
    let records = split_source(&source()).into_result().unwrap();
    let project = records
        .iter()
        .find(|record| matches!(record.record(), DraftRecord::Project { .. }))
        .unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(project.bytes()).unwrap();
    value["payload"]["format_version"] = json!(99);
    value["payload"]["product"]["id"] = json!("bad id");
    let report = validate_record(&canonical_json(&value).unwrap());
    for code in ["version", "identifier"] {
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == code)
        );
    }
    let graph = records
        .iter()
        .find(|record| matches!(record.record(), DraftRecord::Graph { .. }))
        .unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(graph.bytes()).unwrap();
    value["payload"]["outcomes"] = json!([]);
    value["payload"]["locals"] = json!({"bad key":"x".repeat(narrata_nodes::MAX_TEXT_BYTES + 1)});
    let report = validate_record(&canonical_json(&value).unwrap());
    for code in ["contract", "identifier", "limit"] {
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == code)
        );
    }
}

#[test]
fn map_key_locations_are_json_pointers_and_equal_errors_on_distinct_fields_survive() {
    let records = split_source(&source()).into_result().unwrap();
    let record = records
        .iter()
        .find(|record| matches!(record.record(), DraftRecord::Graph { .. }))
        .unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(record.bytes()).unwrap();
    value["payload"]["locals"] = json!({"one":"x".repeat(narrata_nodes::MAX_TEXT_BYTES + 1), "bad.name/~[\"":"x".repeat(narrata_nodes::MAX_TEXT_BYTES + 1)});
    let report = validate_record(&canonical_json(&value).unwrap());
    assert_eq!(
        report
            .diagnostics
            .iter()
            .filter(|error| error.code == "limit")
            .count(),
        2
    );
    for diagnostic in &report.diagnostics {
        assert!(
            value.pointer(&diagnostic.location.pointer).is_some(),
            "{diagnostic:?}"
        );
    }
    let record = records
        .iter()
        .find(|record| matches!(record.record(), DraftRecord::Package { .. }))
        .unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(record.bytes()).unwrap();
    value["payload"]["id"] = json!("");
    value["payload"]["exports"] = json!(["same", "same"]);
    let report = validate_record(&canonical_json(&value).unwrap());
    for code in ["identifier", "duplicate"] {
        assert!(report.diagnostics.iter().any(|error| error.code == code));
    }
}

#[test]
fn tombstones_split_with_their_package_and_cannot_reuse_live_ids() {
    let mut source = source();
    source
        .packages
        .get_mut("main")
        .unwrap()
        .tombstones
        .push(narrata_nodes::AuthoredId::Node(
            narrata_nodes::NodeId::from_bytes([99; 16]),
        ));
    let records = split_source(&source).into_result().unwrap();
    assert_eq!(
        assemble_records(&records).into_result().unwrap(),
        normalize_source(&source).unwrap()
    );
    assert!(records.iter().any(|record| matches!(record.record(), DraftRecord::Tombstone { package, .. } if package == "main")));
    let live = source.packages["road"].graphs["crossing"].nodes["rocks"]
        .id
        .unwrap();
    source
        .packages
        .get_mut("main")
        .unwrap()
        .tombstones
        .push(narrata_nodes::AuthoredId::Node(live));
    let report = assemble_records(&split_source(&source).into_result().unwrap());
    assert!(report.value.is_none());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "tombstone" && !diagnostic.related.is_empty())
    );
}

#[test]
fn local_choice_validation_reports_all_independent_constraints() {
    let record = split_source(&source()).into_result().unwrap().into_iter().find(|record| matches!(record.record(), DraftRecord::ChoicePoint { payload, .. } if payload.options.len() > 1)).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(record.bytes()).unwrap();
    value["payload"]["max"] = json!(500);
    value["payload"]["options"][0]
        .as_object_mut()
        .unwrap()
        .remove("id");
    value["payload"]["options"][1]
        .as_object_mut()
        .unwrap()
        .remove("label");
    let report = validate_record(&canonical_json(&value).unwrap());
    for code in ["cardinality", "missing_id", "label"] {
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == code),
            "{report:?}"
        );
    }
}

#[test]
fn every_budget_failure_is_explicit_and_never_returns_an_artifact() {
    let records = split_source(&source()).into_result().unwrap();
    for budget in [
        ValidationBudget {
            max_records: 1,
            ..ValidationBudget::default()
        },
        ValidationBudget {
            max_input_bytes: 1,
            ..ValidationBudget::default()
        },
        ValidationBudget {
            max_checks: 1,
            ..ValidationBudget::default()
        },
        ValidationBudget {
            max_diagnostics: 0,
            max_checks: 0,
            ..ValidationBudget::default()
        },
    ] {
        let report =
            validate_project_with_budget(&bytes(&records), &NodeRegistry::gamebook(), budget);
        assert!(!report.complete && report.value.is_none());
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "limit")
        );
    }
}

#[test]
fn record_schema_and_canonical_bytes_are_generated_deterministically() {
    let schema = narrata_authoring::records::schema();
    let expected =
        std::fs::read(repository().join("packages/narrata/nodes/schemas/draft-record.schema.json"))
            .unwrap();
    assert_eq!(
        narrata_node_tools::pretty(&schema).unwrap().as_bytes(),
        expected
    );
    let record = split_source(&source()).into_result().unwrap().remove(0);
    let pretty = narrata_node_tools::pretty(record.record()).unwrap();
    assert_eq!(
        validate_record(pretty.as_bytes())
            .into_result()
            .unwrap()
            .bytes(),
        record.bytes()
    );
}
