#![allow(clippy::panic, clippy::unwrap_used)]

use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use narrata_core::{
    ExecutionId, InputId,
    program::{encode_program_artifact, load_program},
    runtime::{CheckedRuntimeInput, SliceBudget, new_execution},
};
use narrata_testkit::{
    backend::{ConformanceBackend, NativeBackend},
    generator::branch_call_choice_v0,
};

fn start_to_first_safe_point(criterion: &mut Criterion) {
    let program = load_program(
        &encode_program_artifact(&branch_call_choice_v0()),
        &Default::default(),
    )
    .unwrap();
    let state = Arc::new(new_execution(&program, ExecutionId::from_u128(1)).unwrap());
    criterion.bench_function("branch_call_choice/start_to_say", |bencher| {
        bencher.iter(|| {
            NativeBackend
                .transition(
                    program.clone(),
                    state.clone(),
                    CheckedRuntimeInput::start(InputId::from_u128(1)),
                    Default::default(),
                    SliceBudget::unlimited(),
                )
                .unwrap()
        });
    });
}

criterion_group!(benches, start_to_first_safe_point);
criterion_main!(benches);
