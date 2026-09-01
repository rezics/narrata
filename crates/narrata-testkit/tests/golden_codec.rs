#![allow(clippy::panic, clippy::unwrap_used)]

use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use narrata_core::{
    ExecutionId, FieldId, InputId, Value,
    codec::{encode_canonical_value, sha256},
    program::{encode_program_artifact, load_program},
    runtime::{CheckedRuntimeInput, SliceBudget, encode_receipt, new_execution},
    snapshot::export_snapshot,
};
use narrata_testkit::{
    backend::{ConformanceBackend, NativeBackend},
    generator::hello_v0,
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
