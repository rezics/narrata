#![allow(clippy::panic, clippy::unwrap_used)]

use std::sync::Arc;

use narrata_core::{
    CommitId, ExecutionId, InputId,
    codec::sha256,
    limits::ProgramLoadLimits,
    program::{encode_program_artifact, validate_program},
    runtime::{
        CheckedRuntimeInput, SliceBudget, SliceOutcome, begin_transition_with_parent_commit,
        encode_receipt, new_execution,
    },
    snapshot::export_snapshot,
};
use narrata_testkit::generator::statechart_parallel_history_v0;

fn main() {
    let artifact = statechart_parallel_history_v0().unwrap();
    let program_bytes = encode_program_artifact(&artifact);
    let program = Arc::new(validate_program(artifact, &ProgramLoadLimits::default()).unwrap());
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
    println!("program {}", hex::encode(sha256(&program_bytes)));
    println!(
        "snapshot {}",
        hex::encode(sha256(&export_snapshot(draft.next_state()).unwrap()))
    );
    println!(
        "receipt {}",
        hex::encode(sha256(&encode_receipt(draft.receipt())))
    );
}
