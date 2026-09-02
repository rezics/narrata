#![allow(clippy::panic, clippy::unwrap_used)]

use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use narrata_core::{
    CommitId, ExecutionId, FieldId, InputId, Value,
    codec::{encode_canonical_value, sha256},
    program::{encode_program_artifact, load_program},
    runtime::{
        CheckedRuntimeInput, SliceBudget, SliceOutcome, begin_transition_with_parent_commit,
        encode_receipt, new_execution,
    },
    snapshot::export_snapshot,
};
use narrata_testkit::{
    backend::{ConformanceBackend, NativeBackend},
    generator::{hello_v0, statechart_parallel_history_v0},
};

#[test]
fn exact_codec_program_snapshot_and_receipt_vectors_are_stable() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixture = |name: &str| {
        std::fs::read_to_string(root.join("fixtures/codec").join(name))
            .unwrap()
            .trim()
            .to_owned()
    };
    let mut record = BTreeMap::new();
    record.insert(FieldId::from_u128(1), Value::from("é"));
    record.insert(FieldId::from_u128(2), Value::I64(-1));
    let value_bytes = encode_canonical_value(&Value::Record(record));
    assert_eq!(hex::encode(&value_bytes), fixture("value-v0.cbor.hex"));
    assert_eq!(
        hex::encode(sha256(&value_bytes)),
        fixture("value-v0.sha256")
    );

    let program_bytes = encode_program_artifact(&hello_v0());
    assert_eq!(hex::encode(&program_bytes), fixture("program-v0.cbor.hex"));
    let program = load_program(&program_bytes, &Default::default()).unwrap();
    let state = Arc::new(new_execution(&program, ExecutionId::from_u128(1)).unwrap());
    let draft = NativeBackend
        .transition(
            program,
            state,
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            SliceBudget::unlimited(),
        )
        .unwrap();
    assert_eq!(
        hex::encode(export_snapshot(draft.next_state()).unwrap()),
        fixture("snapshot-v0.cbor.hex")
    );
    assert_eq!(
        hex::encode(encode_receipt(draft.receipt())),
        fixture("receipt-v0.cbor.hex")
    );
}

#[test]
fn statechart_program_snapshot_and_receipt_hashes_are_stable() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let expected =
        std::fs::read_to_string(root.join("fixtures/codec/statechart-v0.sha256")).unwrap();
    let expected = expected
        .lines()
        .map(|line| line.split_once(' ').unwrap())
        .collect::<BTreeMap<_, _>>();
    let artifact = statechart_parallel_history_v0().unwrap();
    let program_bytes = encode_program_artifact(&artifact);
    let program = load_program(&program_bytes, &Default::default()).unwrap();
    let state = Arc::new(new_execution(&program, ExecutionId::from_u128(1)).unwrap());
    let outcome = begin_transition_with_parent_commit(
        program,
        state,
        CommitId::from_bytes([1; 32]),
        CheckedRuntimeInput::start(InputId::from_u128(1)),
        Default::default(),
    )
    .unwrap()
    .run_slice(SliceBudget::unlimited());
    let SliceOutcome::Completed(draft) = outcome else {
        panic!("Stage 4 golden transition did not complete");
    };
    assert_eq!(hex::encode(sha256(&program_bytes)), expected["program"]);
    assert_eq!(
        hex::encode(sha256(&export_snapshot(draft.next_state()).unwrap())),
        expected["snapshot"]
    );
    assert_eq!(
        hex::encode(sha256(&encode_receipt(draft.receipt()))),
        expected["receipt"]
    );
}
