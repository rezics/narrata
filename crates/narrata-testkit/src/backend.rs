use std::sync::Arc;

use narrata_core::{
    CheckedProgram,
    limits::MacrostepLimits,
    runtime::{
        CheckedRuntimeInput, RuntimeFault, RuntimeStateV0, SliceBudget, SliceOutcome,
        TransitionDraft, TransitionStartError, begin_transition,
    },
};
use thiserror::Error;

pub trait ConformanceBackend {
    fn transition(
        &self,
        program: Arc<CheckedProgram>,
        state: Arc<RuntimeStateV0>,
        input: CheckedRuntimeInput,
        limits: MacrostepLimits,
        slice: SliceBudget,
    ) -> Result<TransitionDraft, BackendError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NativeBackend;

impl ConformanceBackend for NativeBackend {
    fn transition(
        &self,
        program: Arc<CheckedProgram>,
        state: Arc<RuntimeStateV0>,
        input: CheckedRuntimeInput,
        limits: MacrostepLimits,
        slice: SliceBudget,
    ) -> Result<TransitionDraft, BackendError> {
        let mut outcome = begin_transition(program, state, input, limits)
            .map_err(BackendError::Start)?
            .run_slice(slice);
        loop {
            match outcome {
                SliceOutcome::Yielded { runner, .. } => outcome = runner.run_slice(slice),
                SliceOutcome::Completed(draft) => return Ok(draft),
                SliceOutcome::Faulted(fault) => return Err(BackendError::Fault(fault)),
            }
        }
    }
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum BackendError {
    #[error(transparent)]
    Start(TransitionStartError),
    #[error(transparent)]
    Fault(RuntimeFault),
}
