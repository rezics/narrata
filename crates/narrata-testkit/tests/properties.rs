#![allow(clippy::panic, clippy::unwrap_used)]

use std::sync::Arc;

use narrata_core::{
    ExecutionId, InputId,
    program::{encode_program_artifact, load_program},
    runtime::{CheckedRuntimeInput, SliceBudget, new_execution},
    snapshot::{export_snapshot, restore_snapshot},
};
use narrata_testkit::{
    backend::{ConformanceBackend, NativeBackend},
    generator::hello_v0,
};
use proptest::prelude::*;

proptest! {
    #[test]
    fn same_program_state_and_input_produce_same_bytes(execution in any::<u128>(), request in any::<u128>()) {
        let program = load_program(&encode_program_artifact(&hello_v0()), &Default::default()).unwrap();
        let state = Arc::new(new_execution(&program, ExecutionId::from_u128(execution)).unwrap());
        let input = CheckedRuntimeInput::start(InputId::from_u128(request));
        let left = NativeBackend.transition(program.clone(), state.clone(), input.clone(), Default::default(), SliceBudget::new(1).unwrap()).unwrap();
        let right = NativeBackend.transition(program.clone(), state, input, Default::default(), SliceBudget::new(64).unwrap()).unwrap();
        prop_assert_eq!(left.next_state_digest(), right.next_state_digest());
        prop_assert_eq!(left.receipt_digest(), right.receipt_digest());
        let snapshot = export_snapshot(left.next_state()).unwrap();
        let restored = restore_snapshot(&snapshot, &program, &Default::default()).unwrap();
        prop_assert_eq!(&restored, left.next_state());
    }
}
