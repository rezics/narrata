#![allow(clippy::panic, clippy::unwrap_used)]

use std::sync::Arc;

use narrata_core::{
    ChoiceId, ExecutionId, InputId,
    limits::{MacrostepLimits, ProgramLoadLimits},
    program::{encode_program_artifact, load_program},
    runtime::{
        CheckedRuntimeInput, DraftResult, PendingInteractionV0, RuntimeFault, SliceBudget,
        TransitionStartError, new_execution,
    },
    snapshot::{export_snapshot, restore_snapshot},
};
use narrata_testkit::{
    backend::{BackendError, ConformanceBackend, NativeBackend},
    generator::{branch_call_choice_v0, hello_v0},
};

fn checked(
    artifact: &narrata_core::program::ProgramArtifactV0,
) -> Arc<narrata_core::CheckedProgram> {
    load_program(
        &encode_program_artifact(artifact),
        &ProgramLoadLimits::default(),
    )
    .unwrap()
}

#[test]
fn hello_runs_start_say_advance_finish() {
    let program = checked(&hello_v0());
    let state = Arc::new(new_execution(&program, ExecutionId::from_u128(1)).unwrap());
    let first = NativeBackend
        .transition(
            program.clone(),
            state,
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            SliceBudget::unlimited(),
        )
        .unwrap();
    assert!(matches!(first.result(), DraftResult::AwaitSay(view) if view.text.as_ref() == "Hello"));
    let interaction = first.next_state().pending().unwrap().interaction_id();
    let second = NativeBackend
        .transition(
            program,
            Arc::new(first.next_state().clone()),
            CheckedRuntimeInput::advance(InputId::from_u128(2), interaction),
            Default::default(),
            SliceBudget::unlimited(),
        )
        .unwrap();
    assert!(matches!(
        second.result(),
        DraftResult::Finished(narrata_core::Value::Null)
    ));
}

#[test]
fn branch_call_choice_covers_interactions_hidden_choice_and_call_return() {
    let program = checked(&branch_call_choice_v0());
    let state = Arc::new(new_execution(&program, ExecutionId::from_u128(2)).unwrap());
    let say = transition(
        program.clone(),
        state,
        CheckedRuntimeInput::start(InputId::from_u128(10)),
        7,
    );
    let say_id = say.next_state().pending().unwrap().interaction_id();
    let choice = transition(
        program.clone(),
        Arc::new(say.next_state().clone()),
        CheckedRuntimeInput::advance(InputId::from_u128(11), say_id),
        2,
    );
    let (choice_interaction, offered) = match choice.next_state().pending().unwrap() {
        PendingInteractionV0::Choice {
            interaction_id,
            offered,
            ..
        } => (*interaction_id, offered),
        _ => panic!("expected Choice"),
    };
    assert_eq!(offered.len(), 1);
    assert_eq!(offered[0].id, ChoiceId::from_u128(1));
    let callee_say = transition(
        program.clone(),
        Arc::new(choice.next_state().clone()),
        CheckedRuntimeInput::select(
            InputId::from_u128(12),
            choice_interaction,
            ChoiceId::from_u128(1),
        ),
        1,
    );
    assert!(
        matches!(callee_say.result(), DraftResult::AwaitSay(view) if view.text.as_ref() == "Inside callee")
    );
    let callee_interaction = callee_say.next_state().pending().unwrap().interaction_id();
    let finished = transition(
        program,
        Arc::new(callee_say.next_state().clone()),
        CheckedRuntimeInput::advance(InputId::from_u128(13), callee_interaction),
        64,
    );
    assert!(matches!(finished.result(), DraftResult::Finished(_)));
}

#[test]
fn every_interaction_snapshot_restores_and_continues_identically() {
    let program = checked(&branch_call_choice_v0());
    let initial = Arc::new(new_execution(&program, ExecutionId::from_u128(3)).unwrap());
    let say = transition(
        program.clone(),
        initial,
        CheckedRuntimeInput::start(InputId::from_u128(20)),
        64,
    );
    let restored_say = restore_snapshot(
        &export_snapshot(say.next_state()).unwrap(),
        &program,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(&restored_say, say.next_state());
    let input = CheckedRuntimeInput::advance(
        InputId::from_u128(21),
        say.next_state().pending().unwrap().interaction_id(),
    );
    let uninterrupted = transition(
        program.clone(),
        Arc::new(say.next_state().clone()),
        input.clone(),
        1,
    );
    let restored = transition(program, Arc::new(restored_say), input, 64);
    assert_eq!(
        uninterrupted.next_state_digest(),
        restored.next_state_digest()
    );
    assert_eq!(uninterrupted.receipt_digest(), restored.receipt_digest());
    assert_eq!(uninterrupted.result(), restored.result());
}

#[test]
fn slice_size_does_not_change_state_receipt_or_trace_counts() {
    let program = checked(&branch_call_choice_v0());
    let state = Arc::new(new_execution(&program, ExecutionId::from_u128(4)).unwrap());
    let input = CheckedRuntimeInput::start(InputId::from_u128(30));
    let drafts = [1, 2, 7, 64]
        .into_iter()
        .map(|slice| transition(program.clone(), state.clone(), input.clone(), slice))
        .collect::<Vec<_>>();
    for draft in drafts.iter().skip(1) {
        assert_eq!(draft.next_state_digest(), drafts[0].next_state_digest());
        assert_eq!(draft.receipt_digest(), drafts[0].receipt_digest());
        assert_eq!(
            draft.receipt().instruction_count,
            drafts[0].receipt().instruction_count
        );
    }
}

#[test]
fn hard_limit_faults_leave_parent_bytes_and_digest_unchanged() {
    let program = checked(&branch_call_choice_v0());
    let parent = Arc::new(new_execution(&program, ExecutionId::from_u128(5)).unwrap());
    let bytes = export_snapshot(&parent).unwrap();
    let digest = narrata_core::snapshot::state_digest(&parent);
    for hard_limit in 0..11 {
        let limits = MacrostepLimits {
            max_instructions: hard_limit,
            ..MacrostepLimits::default()
        };
        let error = NativeBackend
            .transition(
                program.clone(),
                parent.clone(),
                CheckedRuntimeInput::start(InputId::from_u128(40 + u128::from(hard_limit))),
                limits,
                SliceBudget::unlimited(),
            )
            .unwrap_err();
        assert_eq!(error, BackendError::Fault(RuntimeFault::InstructionLimit));
        assert_eq!(export_snapshot(&parent).unwrap(), bytes);
        assert_eq!(narrata_core::snapshot::state_digest(&parent), digest);
    }
}

#[test]
fn stale_interaction_and_unoffered_choice_are_rejected_before_working_state() {
    let program = checked(&hello_v0());
    let parent = Arc::new(new_execution(&program, ExecutionId::from_u128(6)).unwrap());
    let bytes = export_snapshot(&parent).unwrap();
    let error = narrata_core::begin_transition(
        program.clone(),
        parent.clone(),
        CheckedRuntimeInput::advance(
            InputId::from_u128(50),
            narrata_core::InteractionId::from_bytes([0; 32]),
        ),
        Default::default(),
    )
    .unwrap_err();
    assert!(matches!(error, TransitionStartError::InvalidInput(_)));
    assert_eq!(export_snapshot(&parent).unwrap(), bytes);
}

fn transition(
    program: Arc<narrata_core::CheckedProgram>,
    state: Arc<narrata_core::RuntimeStateV0>,
    input: CheckedRuntimeInput,
    slice: u64,
) -> narrata_core::TransitionDraft {
    NativeBackend
        .transition(
            program,
            state,
            input,
            Default::default(),
            SliceBudget::new(slice).unwrap(),
        )
        .unwrap()
}
