use std::{error::Error, sync::Arc};

use narrata_core::{
    ChoiceId, ExecutionId, InputId,
    program::{encode_program_artifact, load_program},
    runtime::{
        CheckedRuntimeInput, PendingInteractionV0, RuntimeStateV0, SliceBudget, new_execution,
    },
};
use narrata_testkit::{
    backend::{ConformanceBackend, NativeBackend},
    generator::branch_call_choice_v0,
};

fn main() -> Result<(), Box<dyn Error>> {
    let program = load_program(
        &encode_program_artifact(&branch_call_choice_v0()),
        &Default::default(),
    )?;
    let mut state = Arc::new(new_execution(&program, ExecutionId::from_u128(2))?);
    for index in 0_u128..4 {
        let request = InputId::from_u128(10 + index);
        let input = match index {
            0 => CheckedRuntimeInput::start(request),
            1 | 3 => CheckedRuntimeInput::advance(request, pending_id(&state)?),
            2 => CheckedRuntimeInput::select(request, pending_id(&state)?, ChoiceId::from_u128(1)),
            _ => return Err("conformance input index is invalid".into()),
        };
        let draft = NativeBackend.transition(
            program.clone(),
            state,
            input,
            Default::default(),
            SliceBudget::new(7)?,
        )?;
        println!("{} {}", draft.next_state_digest(), draft.receipt_digest());
        state = Arc::new(draft.next_state().clone());
    }
    Ok(())
}

fn pending_id(state: &RuntimeStateV0) -> Result<narrata_core::InteractionId, Box<dyn Error>> {
    state
        .pending()
        .map(PendingInteractionV0::interaction_id)
        .ok_or_else(|| "conformance state has no pending interaction".into())
}
