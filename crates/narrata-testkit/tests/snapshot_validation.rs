#![allow(clippy::panic, clippy::unwrap_used)]

use std::sync::Arc;

use narrata_core::{
    ExecutionId, InputId,
    program::{encode_program_artifact, load_program},
    runtime::{
        CheckedRuntimeInput, PendingContent, PendingInteractionV0, SliceBudget, new_execution,
    },
    snapshot::{copy_as_new_execution, export_snapshot, restore_snapshot},
};
use narrata_testkit::{
    backend::{ConformanceBackend, NativeBackend},
    generator::{branch_call_choice_v0, hello_v0},
};

#[test]
fn restore_rejects_wrong_program_corruption_and_tampered_offered_set() {
    let program = load_program(
        &encode_program_artifact(&branch_call_choice_v0()),
        &Default::default(),
    )
    .unwrap();
    let hello = load_program(&encode_program_artifact(&hello_v0()), &Default::default()).unwrap();
    let initial = Arc::new(new_execution(&program, ExecutionId::from_u128(1)).unwrap());
    let say = NativeBackend
        .transition(
            program.clone(),
            initial,
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            SliceBudget::unlimited(),
        )
        .unwrap();
    let snapshot = export_snapshot(say.next_state()).unwrap();
    assert!(restore_snapshot(&snapshot, &hello, &Default::default()).is_err());
    let mut corrupt = snapshot.clone();
    let last = corrupt.len().saturating_sub(1);
    corrupt[last] ^= 1;
    assert!(restore_snapshot(&corrupt, &program, &Default::default()).is_err());

    let interaction = say.next_state().pending().unwrap().interaction_id();
    let choice = NativeBackend
        .transition(
            program.clone(),
            Arc::new(say.next_state().clone()),
            CheckedRuntimeInput::advance(InputId::from_u128(2), interaction),
            Default::default(),
            SliceBudget::unlimited(),
        )
        .unwrap();
    let mut tampered = choice.next_state().clone();
    if let Some(PendingInteractionV0::Choice { offered, .. }) = match &mut tampered.status {
        narrata_core::runtime::RuntimeStatusV0::Awaiting { pending, .. } => Some(pending),
        _ => None,
    } {
        offered[0].label = PendingContent::LegacyText(Arc::from("tampered"));
    }
    let tampered_snapshot = export_snapshot(&tampered).unwrap();
    assert!(restore_snapshot(&tampered_snapshot, &program, &Default::default()).is_err());
}

#[test]
fn copy_as_new_execution_changes_interaction_scope() {
    let program = load_program(&encode_program_artifact(&hello_v0()), &Default::default()).unwrap();
    let initial = Arc::new(new_execution(&program, ExecutionId::from_u128(1)).unwrap());
    let say = NativeBackend
        .transition(
            program,
            initial,
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            SliceBudget::unlimited(),
        )
        .unwrap();
    let copied = copy_as_new_execution(say.next_state(), ExecutionId::from_u128(2)).unwrap();
    assert_ne!(
        copied.pending().unwrap().interaction_id(),
        say.next_state().pending().unwrap().interaction_id()
    );
    assert_ne!(
        narrata_core::snapshot::state_digest(&copied),
        say.next_state_digest()
    );
}
