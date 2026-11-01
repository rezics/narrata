#![allow(clippy::unwrap_used, clippy::panic)]

use narrata_authoring::{
    ComposeMode, DraftRecord, Severity, split_source, validate_record,
    vn::{preview_records, validate_records},
};
use narrata_nodes::{
    AuthoredId, FORMAT_VERSION, NodeRegistry, ProjectSource, VISUAL_NOVEL_FORMAT_VERSION,
    canonical_json, compile, compile_objects,
};

mod support;

fn repository() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../..")
}

fn vn_source() -> ProjectSource {
    let mut value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(repository().join("fixtures/compat/nodes-vn-v1/source.json")).unwrap(),
    )
    .unwrap();
    ProjectSource {
        manifest: serde_json::from_value(value["manifest"].take()).unwrap(),
        packages: std::collections::BTreeMap::from([(
            "main".into(),
            serde_json::from_value(value["package"].take()).unwrap(),
        )]),
    }
}

fn encoded(source: &ProjectSource) -> Vec<Vec<u8>> {
    split_source(source)
        .into_result()
        .unwrap()
        .into_iter()
        .map(|record| record.bytes().to_vec())
        .collect()
}

fn inputs(records: &[Vec<u8>]) -> Vec<&[u8]> {
    records.iter().map(Vec::as_slice).collect()
}

#[test]
fn vn_records_preview_the_same_candidate_as_direct_compilation_deterministically() {
    let source = vn_source();
    let records = encoded(&source);
    let first = preview_records(&inputs(&records), None, ComposeMode::Update);
    let second = preview_records(&inputs(&records), None, ComposeMode::Update);
    assert!(first.complete);
    assert!(
        first
            .diagnostics
            .iter()
            .all(|item| item.severity != Severity::Error)
    );
    let first = first.value.unwrap();
    let second = second.value.unwrap();
    let direct = compile_objects(&source, &NodeRegistry::visual_novel()).unwrap();

    assert_eq!(
        first.composed.compilation.objects.encode(),
        direct.objects.encode()
    );
    assert_eq!(
        first.composed.compilation.program.artifact_id(),
        direct.program.artifact_id()
    );
    assert_eq!(first.composed.compilation.lock, direct.lock);
    assert_eq!(first.composed.compilation.names, direct.names);
    assert_eq!(
        first.composed.compilation.objects.encode(),
        second.composed.compilation.objects.encode()
    );
    assert_eq!(
        first.composed.compilation.lock,
        second.composed.compilation.lock
    );
    assert_eq!(first.records.records().len(), records.len());
    assert_eq!(
        first.composed.compilation.program.source_format(),
        VISUAL_NOVEL_FORMAT_VERSION
    );
}

#[test]
fn dialect_is_explicit_on_vn_and_legacy_gamebook_record_paths() {
    let vn = encoded(&vn_source());
    let legacy = support::demo().unwrap();
    let legacy_records = encoded(&legacy);

    assert!(validate_records(&inputs(&vn)).value.is_some());
    assert!(
        narrata_authoring::validate_records(&inputs(&vn))
            .value
            .is_none()
    );
    assert!(validate_records(&inputs(&legacy_records)).value.is_none());
    assert!(
        narrata_authoring::validate_records(&inputs(&legacy_records))
            .value
            .is_some()
    );

    let gamebook = compile(&legacy, &NodeRegistry::gamebook()).unwrap();
    let baseline_rejected =
        preview_records(&inputs(&vn), Some(&gamebook.program), ComposeMode::Update);
    assert!(baseline_rejected.value.is_none());
    assert_eq!(baseline_rejected.diagnostics[0].code, "dialect_mismatch");
    assert_eq!(FORMAT_VERSION, gamebook.program.source_format());
}

#[test]
fn full_vn_semantics_reject_invalid_dialogue_data_with_a_record_location() {
    let mut records = encoded(&vn_source());
    let dialogue = records
        .iter_mut()
        .find(|bytes| {
            let record: DraftRecord = serde_json::from_slice(bytes).unwrap();
            matches!(record, DraftRecord::Node { key, payload, .. } if key == "line_one" && payload.type_id == "narrata.dialogue")
        })
        .unwrap();
    let mut record: DraftRecord = serde_json::from_slice(dialogue).unwrap();
    let DraftRecord::Node { payload, .. } = &mut record else {
        unreachable!();
    };
    payload.data.as_object_mut().unwrap().remove("line");
    *dialogue = canonical_json(&record).unwrap();

    assert!(validate_record(dialogue).value.is_some());
    let checked = validate_records(&inputs(&records));
    assert!(checked.value.is_none());
    let error = checked
        .diagnostics
        .iter()
        .find(|item| item.severity == Severity::Error)
        .unwrap();
    assert_eq!(
        error.location.source_path.as_deref(),
        Some("packages.main.graphs.start.nodes.line_one")
    );
}

#[test]
fn full_vn_semantics_reject_invalid_scene_data_with_a_record_location() {
    let mut records = encoded(&vn_source());
    let scene = records
        .iter_mut()
        .find(|bytes| {
            let record: DraftRecord = serde_json::from_slice(bytes).unwrap();
            matches!(record, DraftRecord::Node { key, payload, .. } if key == "scene_one" && payload.type_id == "narrata.scene")
        })
        .unwrap();
    let mut record: DraftRecord = serde_json::from_slice(scene).unwrap();
    let DraftRecord::Node { payload, .. } = &mut record else {
        unreachable!();
    };
    let layer = payload.data["layers"][0].clone();
    payload.data["layers"].as_array_mut().unwrap().push(layer);
    *scene = canonical_json(&record).unwrap();

    assert!(validate_record(scene).value.is_some());
    let checked = validate_records(&inputs(&records));
    assert!(checked.value.is_none());
    let error = checked
        .diagnostics
        .iter()
        .find(|item| item.severity == Severity::Error)
        .unwrap();
    assert_eq!(
        error.location.source_path.as_deref(),
        Some("packages.main.graphs.start.nodes.scene_one.layers[1]")
    );
}

#[test]
fn editing_records_changes_the_next_candidate_without_mutating_the_old_one() {
    let mut records = encoded(&vn_source());
    let first = preview_records(&inputs(&records), None, ComposeMode::Update)
        .value
        .unwrap();
    let old_artifact = first.composed.compilation.program.artifact_id();
    let old_objects = first.composed.compilation.objects.encode();

    let dialogue = records
        .iter_mut()
        .find(|bytes| {
            let record: DraftRecord = serde_json::from_slice(bytes).unwrap();
            matches!(record, DraftRecord::Node { payload, .. } if payload.type_id == "narrata.dialogue")
        })
        .unwrap();
    let mut record: DraftRecord = serde_json::from_slice(dialogue).unwrap();
    let DraftRecord::Node { payload, .. } = &mut record else {
        unreachable!();
    };
    payload.data["speaker"]["key"] = serde_json::Value::String("speaker.edited".into());
    *dialogue = canonical_json(&record).unwrap();

    let second = preview_records(&inputs(&records), None, ComposeMode::Update)
        .value
        .unwrap();
    assert_ne!(
        old_artifact,
        second.composed.compilation.program.artifact_id()
    );
    assert_eq!(
        old_artifact,
        first.composed.compilation.program.artifact_id()
    );
    assert_eq!(old_objects, first.composed.compilation.objects.encode());
}

#[test]
fn locked_preview_rejects_a_changed_candidate() {
    let mut records = encoded(&vn_source());
    let initial = preview_records(&inputs(&records), None, ComposeMode::Update)
        .value
        .unwrap();
    let old_lock = initial.composed.compilation.lock.clone();

    let dialogue = records
        .iter_mut()
        .find(|bytes| {
            let record: DraftRecord = serde_json::from_slice(bytes).unwrap();
            matches!(record, DraftRecord::Node { payload, .. } if payload.type_id == "narrata.dialogue")
        })
        .unwrap();
    let mut record: DraftRecord = serde_json::from_slice(dialogue).unwrap();
    let DraftRecord::Node { payload, .. } = &mut record else {
        unreachable!();
    };
    payload.data["speaker"]["key"] = serde_json::Value::String("speaker.edited".into());
    *dialogue = canonical_json(&record).unwrap();

    let locked = preview_records(&inputs(&records), None, ComposeMode::Locked(&old_lock));
    assert!(locked.value.is_none());
    assert!(
        locked
            .diagnostics
            .iter()
            .any(|item| item.code == "lock_mismatch")
    );
}

#[test]
fn preview_uses_only_candidate_composition_without_publication_or_reader_state() {
    let records = encoded(&vn_source());
    let preview = preview_records(&inputs(&records), None, ComposeMode::Update)
        .value
        .unwrap();
    assert!(!preview.composed.compared);
    assert!(preview.composed.tombstones.is_empty());
    assert!(preview.composed.appended.is_empty());
}

#[test]
fn vn_baseline_continuity_keeps_identity_rules_and_reports_comparison() {
    let records = encoded(&vn_source());
    let baseline = preview_records(&inputs(&records), None, ComposeMode::Update)
        .value
        .unwrap();
    let previous = &baseline.composed.compilation.program;
    let continued = preview_records(&inputs(&records), Some(previous), ComposeMode::Update)
        .value
        .unwrap();

    assert!(continued.composed.compared);
    assert_eq!(
        baseline.composed.compilation.program.artifact_id(),
        continued.composed.compilation.program.artifact_id()
    );
    assert_eq!(continued.composed.tombstones.len(), 0);
}

#[test]
fn deleting_a_vn_node_appends_a_tombstone_and_locked_mode_refuses_it() {
    let mut source = vn_source();
    let graph = source
        .packages
        .get_mut("main")
        .unwrap()
        .graphs
        .get_mut("start")
        .unwrap();
    let mut unused = graph.nodes["scene_one"].clone();
    unused.id = Some(
        serde_json::from_value(serde_json::json!("node:00000000000000000000000000000010")).unwrap(),
    );
    graph.nodes.insert("unused_scene".into(), unused);

    let baseline_records = encoded(&source);
    let baseline = preview_records(&inputs(&baseline_records), None, ComposeMode::Update)
        .value
        .unwrap();
    let old_artifact = baseline.composed.compilation.program.artifact_id();
    let old_objects = baseline.composed.compilation.objects.encode();
    let old_program = &baseline.composed.compilation.program;
    let old_lock = &baseline.composed.compilation.lock;
    assert!(baseline.records.records().iter().any(
        |record| matches!(record.record(), DraftRecord::Node { key, .. } if key == "unused_scene")
    ));

    let next_records: Vec<_> = baseline_records
        .iter()
        .filter(|bytes| {
            !matches!(
                serde_json::from_slice::<DraftRecord>(bytes).unwrap(),
                DraftRecord::Node { key, .. } if key == "unused_scene"
            )
        })
        .cloned()
        .collect();
    let updated = preview_records(
        &inputs(&next_records),
        Some(old_program),
        ComposeMode::Update,
    )
    .value
    .unwrap();
    let deleted_node_id = baseline
        .records
        .records()
        .iter()
        .find_map(|record| match record.record() {
            DraftRecord::Node { key, payload, .. } if key == "unused_scene" => payload.id,
            _ => None,
        })
        .unwrap();
    let deleted_id = AuthoredId::Node(deleted_node_id);
    assert_eq!(updated.composed.appended, vec![("main".into(), deleted_id)]);
    assert!(matches!(
        updated.composed.tombstones.as_slice(),
        [DraftRecord::Tombstone { package, payload, .. }]
            if package == "main" && *payload == deleted_id
    ));
    assert_eq!(
        old_artifact,
        baseline.composed.compilation.program.artifact_id()
    );
    assert_eq!(old_objects, baseline.composed.compilation.objects.encode());

    let locked = preview_records(
        &inputs(&next_records),
        Some(old_program),
        ComposeMode::Locked(old_lock),
    );
    assert!(locked.value.is_none());
    assert!(
        locked
            .diagnostics
            .iter()
            .any(|item| item.code == "tombstones_needed")
    );
}
