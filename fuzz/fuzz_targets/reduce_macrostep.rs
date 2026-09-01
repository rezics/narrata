#![no_main]

use std::sync::Arc;

use libfuzzer_sys::fuzz_target;
use narrata_core::{
    ExecutionId, InputId,
    limits::MacrostepLimits,
    program::{encode_program_artifact, load_program},
    runtime::{CheckedRuntimeInput, SliceBudget, SliceOutcome, begin_transition, new_execution},
};

fuzz_target!(|data: &[u8]| {
    let first = data.first().copied().unwrap_or(0);
    let hard_limit = u64::from(first);
    if let Ok(program) = load_program(
        &encode_program_artifact(&narrata_testkit::generator::hello_v0()),
        &Default::default(),
    ) && let Ok(state) = new_execution(&program, ExecutionId::from_u128(u128::from(first)))
    {
        let limits = MacrostepLimits {
            max_instructions: hard_limit,
            ..MacrostepLimits::default()
        };
        if let Ok(runner) = begin_transition(
            program,
            Arc::new(state),
            CheckedRuntimeInput::start(InputId::from_u128(u128::from(first))),
            limits,
        ) {
            let mut outcome = runner.run_slice(SliceBudget::unlimited());
            while let SliceOutcome::Yielded { runner, .. } = outcome {
                outcome = runner.run_slice(SliceBudget::unlimited());
            }
        }
    }
});
