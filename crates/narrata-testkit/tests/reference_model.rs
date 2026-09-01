#![allow(clippy::panic, clippy::unwrap_used)]

use std::sync::Arc;

use narrata_core::{
    ExecutionId, InputId,
    program::{encode_program_artifact, load_program},
    runtime::{CheckedRuntimeInput, SliceBudget, new_execution},
};
use narrata_testkit::{
    backend::{ConformanceBackend, NativeBackend},
    generator::hello_v0,
    reference_model::{ReferenceOp, ReferenceState, reduce},
};

#[test]
fn reference_model_matches_hello_safe_point_sequence() {
    let reference = [ReferenceOp::Await { next: 1 }, ReferenceOp::Finish];
    let first_reference = reduce(&reference, &ReferenceState::default()).unwrap();
    assert!(first_reference.awaiting);
    let second_reference = reduce(&reference, &first_reference).unwrap();
    assert!(second_reference.finished);

    let program = load_program(&encode_program_artifact(&hello_v0()), &Default::default()).unwrap();
    let initial = Arc::new(new_execution(&program, ExecutionId::from_u128(1)).unwrap());
    let first = NativeBackend
        .transition(
            program.clone(),
            initial,
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            SliceBudget::unlimited(),
        )
        .unwrap();
    assert!(first.next_state().pending().is_some());
    let second = NativeBackend
        .transition(
            program,
            Arc::new(first.next_state().clone()),
            CheckedRuntimeInput::advance(
                InputId::from_u128(2),
                first.next_state().pending().unwrap().interaction_id(),
            ),
            Default::default(),
            SliceBudget::unlimited(),
        )
        .unwrap();
    assert!(matches!(
        second.next_state().status,
        narrata_core::runtime::RuntimeStatusV0::Finished { .. }
    ));
}
