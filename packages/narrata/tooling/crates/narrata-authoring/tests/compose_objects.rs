#![allow(clippy::unwrap_used, clippy::panic)]

mod support;

use narrata_authoring::{
    CheckedRecord, ComposeMode, DraftRecord, Severity, compose_records, split_source,
    validate_records,
};
use narrata_nodes::{AuthoredId, NodeRegistry, Program, canonical_json, compile};

fn records() -> Vec<CheckedRecord> {
    split_source(&support::demo().unwrap())
        .into_result()
        .unwrap()
}
fn encoded(records: &[CheckedRecord]) -> Vec<Vec<u8>> {
    records
        .iter()
        .map(|record| record.bytes().to_vec())
        .collect()
}
fn inputs(records: &[Vec<u8>]) -> Vec<&[u8]> {
    records.iter().map(Vec::as_slice).collect()
}
fn demo_pack() -> Vec<u8> {
    std::fs::read(support::repository().join("products/gamebook-demo/story.narpack")).unwrap()
}
/// The record index of a passage's single choice point, found by graph and node alias.
fn choice_point(records: &[CheckedRecord], graph: &str, key: &str) -> usize {
    let node = records
        .iter()
        .find_map(|checked| match checked.record() {
            DraftRecord::Node {
                graph: g,
                key: k,
                payload,
                ..
            } if g == graph && k == key => payload.id,
            _ => None,
        })
        .unwrap();
    records
        .iter()
        .position(|checked| {
            matches!(checked.record(), DraftRecord::ChoicePoint { node: n, .. } if *n == node)
        })
        .unwrap()
}
fn edit(records: &mut [Vec<u8>], index: usize, change: impl FnOnce(&mut DraftRecord)) {
    let mut record: DraftRecord = serde_json::from_slice(&records[index]).unwrap();
    change(&mut record);
    records[index] = canonical_json(&record).unwrap();
}
fn options(record: &mut DraftRecord) -> &mut Vec<narrata_nodes::source::OptionSource> {
    match record {
        DraftRecord::ChoicePoint { payload, .. } => &mut payload.options,
        _ => panic!("expected a choice point record"),
    }
}

#[test]
fn records_compose_once_into_the_cli_program_objects() {
    let source = support::demo().unwrap();
    let checked = records();
    let report = compose_records(&inputs(&encoded(&checked)), None, ComposeMode::Update);
    assert!(report.complete);
    let composed = report.value.unwrap();
    let packed = compile(&source, &NodeRegistry::gamebook()).unwrap();
    // The object compilation is the CLI's program: packing it reproduces the committed pack.
    assert_eq!(composed.composed.compilation.objects.encode(), demo_pack());
    assert_eq!(
        composed.composed.compilation.program.artifact_id(),
        packed.program.artifact_id()
    );
    assert_eq!(composed.composed.compilation.lock, packed.lock);
    assert!(!composed.composed.compared);
    assert!(composed.composed.tombstones.is_empty());
    assert_eq!(composed.records.records().len(), checked.len());
    let check = validate_records(&inputs(&encoded(&checked)));
    assert!(check.value.is_some());
}

#[test]
fn independent_defects_are_collected_and_located_without_compiling() {
    let checked = records();
    let mut bytes = encoded(&checked);
    let fire = choice_point(&checked, "visit", "fire");
    let DraftRecord::ChoicePoint { payload, .. } = checked[fire].record() else {
        panic!("expected a choice point");
    };
    let (point, option) = (payload.id.unwrap(), payload.options[2].id.unwrap());
    edit(&mut bytes, fire, |record| {
        options(record)[2].outcome = narrata_nodes::source::OutcomeSource::Branch {
            target: "nowhere".into(),
        };
    });
    let broken = bytes.len() - 1;
    bytes[broken] = b"{\"format_version\":1,".to_vec();
    let report = compose_records(&inputs(&bytes), None, ComposeMode::Update);
    assert!(report.complete);
    assert!(report.value.is_none());
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    assert!(
        errors.iter().any(|d| d.location.source_path.as_deref()
            == Some(&format!("records[{broken}]"))
            && d.location.record.is_none()),
        "{errors:#?}"
    );
    let skipped = report
        .diagnostics
        .iter()
        .find(|d| d.code == "checks_skipped")
        .unwrap();
    assert_eq!(
        skipped.location.source_path.as_deref(),
        Some(&*format!("records[{broken}]"))
    );
    // Assembly of the remaining valid records still reports the dangling branch target.
    let target = errors
        .iter()
        .find(|d| d.location.option == Some(option))
        .unwrap_or_else(|| panic!("{errors:#?}"));
    assert_eq!(target.location.record, Some(checked[fire].key()));
    assert_eq!(target.location.choice_point, Some(point));
    assert!(target.location.pointer.starts_with("/payload/options/2"));
}

#[test]
fn continuity_errors_locate_identities_and_locked_composition_never_appends() {
    let checked = records();
    let previous = Program::from_pack(&demo_pack()).unwrap().0;
    let lock = compile(&support::demo().unwrap(), &NodeRegistry::gamebook())
        .unwrap()
        .lock;

    let mut deleted = encoded(&checked);
    let rocks = choice_point(&checked, "crossing", "rocks");
    let mut retreat = None;
    edit(&mut deleted, rocks, |record| {
        retreat = options(record).pop().unwrap().id;
    });
    let retreat = retreat.unwrap();
    let appended = compose_records(&inputs(&deleted), Some(&previous), ComposeMode::Update)
        .into_result()
        .unwrap();
    assert!(appended.composed.compared);
    assert_eq!(
        appended.composed.appended,
        vec![("road".to_owned(), AuthoredId::Option(retreat))]
    );
    assert!(matches!(
        appended.composed.tombstones.as_slice(),
        [DraftRecord::Tombstone { package, payload, .. }]
            if package == "road" && *payload == AuthoredId::Option(retreat)
    ));
    let locked = compose_records(
        &inputs(&deleted),
        Some(&previous),
        ComposeMode::Locked(&lock),
    );
    assert!(locked.value.is_none());
    let needed = locked
        .diagnostics
        .iter()
        .find(|d| d.code == "tombstones_needed")
        .unwrap();
    assert_eq!(needed.location.option, Some(retreat));
    assert_eq!(needed.location.record, None, "a deleted ID has no record");

    let mut moved = encoded(&checked);
    let fire = choice_point(&checked, "visit", "fire");
    let letter = choice_point(&checked, "visit", "letter");
    let mut leave = None;
    edit(&mut moved, fire, |record| leave = options(record).pop());
    let leave = leave.unwrap();
    let id = leave.id.unwrap();
    edit(&mut moved, letter, |record| options(record).push(leave));
    let report = compose_records(&inputs(&moved), Some(&previous), ComposeMode::Update);
    assert!(report.value.is_none());
    let owner = report
        .diagnostics
        .iter()
        .find(|d| d.code == "owner_changed")
        .unwrap_or_else(|| panic!("{:#?}", report.diagnostics));
    assert_eq!(owner.location.record, Some(checked[letter].key()));
    assert_eq!(owner.location.option, Some(id));
    assert_eq!(owner.location.pointer, "/payload/options/1");
    // The first composition of a moved option has no baseline and therefore succeeds.
    assert!(
        compose_records(&inputs(&moved), None, ComposeMode::Update)
            .value
            .is_some()
    );
}
