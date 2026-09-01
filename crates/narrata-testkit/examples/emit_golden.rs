use std::{collections::BTreeMap, error::Error, sync::Arc};

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

fn main() -> Result<(), Box<dyn Error>> {
    let mut record = BTreeMap::new();
    record.insert(FieldId::from_u128(1), Value::from("é"));
    record.insert(FieldId::from_u128(2), Value::I64(-1));
    let value = Value::Record(record);
    let value_bytes = encode_canonical_value(&value);
    println!("VALUE_HEX={}", hex::encode(&value_bytes));
    println!("VALUE_SHA256={}", hex::encode(sha256(&value_bytes)));

    let artifact_bytes = encode_program_artifact(&hello_v0());
    println!("PROGRAM_HEX={}", hex::encode(&artifact_bytes));
    let program = load_program(&artifact_bytes, &Default::default())?;
    let state = Arc::new(new_execution(&program, ExecutionId::from_u128(1))?);
    let draft = NativeBackend.transition(
        program,
        state,
        CheckedRuntimeInput::start(InputId::from_u128(1)),
        Default::default(),
        SliceBudget::unlimited(),
    )?;
    println!(
        "SNAPSHOT_HEX={}",
        hex::encode(export_snapshot(draft.next_state())?)
    );
    println!(
        "RECEIPT_HEX={}",
        hex::encode(encode_receipt(draft.receipt()))
    );
    Ok(())
}
