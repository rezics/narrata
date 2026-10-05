#![allow(clippy::panic, clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc};

use narrata_core::{
    ExecutionId, InputId, InteractionId,
    program::{encode_program_artifact, load_program},
    runtime::{CheckedRuntimeInput, SliceBudget, new_execution},
};
use narrata_testkit::{
    backend::NativeBackend,
    fixture::{EphemeralSession, FixtureError, load_fixture, run_fixture_with_backend},
    generator::hello_v0,
};

#[test]
fn repository_conformance_fixtures_match_all_slice_budgets() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for relative in [
        "fixtures/conformance/hello-v0.json",
        "fixtures/conformance/branch-call-choice-v0.json",
        "fixtures/conformance/hello-v1.json",
        "fixtures/conformance/branch-call-choice-v1.json",
    ] {
        let fixture = load_fixture(&root.join(relative)).unwrap();
        let baseline =
            run_fixture_with_backend(&fixture, &NativeBackend, SliceBudget::unlimited()).unwrap();
        for slice in [1, 2, 7, 64] {
            let manifest = run_fixture_with_backend(
                &fixture,
                &NativeBackend,
                SliceBudget::new(slice).unwrap(),
            )
            .unwrap();
            assert_eq!(manifest, baseline);
        }
    }
}

#[test]
fn ephemeral_session_retries_same_payload_and_rejects_conflict() {
    let program = load_program(&encode_program_artifact(&hello_v0()), &Default::default()).unwrap();
    let state = Arc::new(new_execution(&program, ExecutionId::from_u128(1)).unwrap());
    let input = CheckedRuntimeInput::start(InputId::from_u128(1));
    let mut session = EphemeralSession::default();
    let first = session
        .apply(
            &NativeBackend,
            program.clone(),
            state.clone(),
            input.clone(),
            Default::default(),
            SliceBudget::unlimited(),
        )
        .unwrap();
    let retry = session
        .apply(
            &NativeBackend,
            program.clone(),
            state.clone(),
            input,
            Default::default(),
            SliceBudget::unlimited(),
        )
        .unwrap();
    assert_eq!(first.receipt_digest(), retry.receipt_digest());
    let conflict = session
        .apply(
            &NativeBackend,
            program,
            state,
            CheckedRuntimeInput::advance(InputId::from_u128(1), InteractionId::from_bytes([0; 32])),
            Default::default(),
            SliceBudget::unlimited(),
        )
        .unwrap_err();
    assert!(matches!(conflict, FixtureError::InputConflict));
}
