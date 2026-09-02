use thiserror::Error;

use crate::{
    codec::{ObjectKind, digest_bytes, encode_envelope},
    identity::{ExecutionId, StateDigest},
    runtime::{PendingInteractionV0, RuntimeStateV0, RuntimeStatusV0, derive_interaction_id},
    version::SNAPSHOT_SCHEMA_V0,
};

use super::encode_state_payload;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum SnapshotExportError {
    #[error("Runtime State is not at a safe point")]
    NotSafePoint,
    #[error("a pending external Effect cannot be copied to a new Execution")]
    PendingEffectCannotChangeExecution,
}

pub fn export_snapshot(state: &RuntimeStateV0) -> Result<Vec<u8>, SnapshotExportError> {
    ensure_safe(state)?;
    Ok(encode_envelope(
        ObjectKind::Snapshot,
        SNAPSHOT_SCHEMA_V0.get(),
        &encode_state_payload(state),
    ))
}

pub fn state_digest(state: &RuntimeStateV0) -> StateDigest {
    StateDigest::from_bytes(digest_bytes(
        "runtime-state",
        SNAPSHOT_SCHEMA_V0.get(),
        &encode_state_payload(state),
    ))
}

pub fn copy_as_new_execution(
    state: &RuntimeStateV0,
    execution_id: ExecutionId,
) -> Result<RuntimeStateV0, SnapshotExportError> {
    ensure_safe(state)?;
    let mut copied = state.clone();
    if matches!(
        &copied.status,
        RuntimeStatusV0::AwaitingEffect { .. } | RuntimeStatusV0::AwaitingStatechartEffect { .. }
    ) {
        return Err(SnapshotExportError::PendingEffectCannotChangeExecution);
    }
    copied.execution_id = execution_id;
    if let RuntimeStatusV0::Awaiting { pending, .. } = &mut copied.status {
        match pending {
            PendingInteractionV0::Say {
                interaction_id,
                origin_instruction,
                origin_parent_state,
                origin_input_digest,
                occurrence,
                ..
            } => {
                *interaction_id = derive_interaction_id(
                    execution_id,
                    *origin_parent_state,
                    *origin_input_digest,
                    *origin_instruction,
                    *occurrence,
                    0,
                );
            }
            PendingInteractionV0::Choice {
                interaction_id,
                origin_instruction,
                origin_parent_state,
                origin_input_digest,
                occurrence,
                ..
            } => {
                *interaction_id = derive_interaction_id(
                    execution_id,
                    *origin_parent_state,
                    *origin_input_digest,
                    *origin_instruction,
                    *occurrence,
                    1,
                );
            }
        }
    }
    Ok(copied)
}

fn ensure_safe(state: &RuntimeStateV0) -> Result<(), SnapshotExportError> {
    let chart_safe = match (&state.statechart, &state.status) {
        (None, _) => true,
        (Some(chart), RuntimeStatusV0::StatechartStable | RuntimeStatusV0::StatechartFinished) => {
            chart.internal_queue.is_empty()
                && chart.deferred_events.is_empty()
                && chart.deferred_work.is_empty()
                && chart.invocation.is_none()
        }
        (Some(chart), RuntimeStatusV0::AwaitingStatechartEffect { .. }) => {
            chart.internal_queue.is_empty()
                && chart.deferred_events.is_empty()
                && chart.invocation.is_none()
        }
        (
            Some(chart),
            RuntimeStatusV0::Awaiting { .. } | RuntimeStatusV0::AwaitingEffect { .. },
        ) => {
            chart.internal_queue.is_empty()
                && chart.deferred_events.is_empty()
                && chart.invocation.is_some()
        }
        (Some(_), RuntimeStatusV0::Ready { .. } | RuntimeStatusV0::Finished { .. }) => false,
    };
    if !chart_safe {
        return Err(SnapshotExportError::NotSafePoint);
    }
    let frames = match &state.status {
        RuntimeStatusV0::Ready { vm }
        | RuntimeStatusV0::Awaiting { vm, .. }
        | RuntimeStatusV0::AwaitingEffect { vm, .. } => &vm.frames,
        RuntimeStatusV0::Finished { final_frames, .. } => final_frames,
        RuntimeStatusV0::StatechartStable
        | RuntimeStatusV0::AwaitingStatechartEffect { .. }
        | RuntimeStatusV0::StatechartFinished => return Ok(()),
    };
    if frames.is_empty()
        || frames
            .iter()
            .any(|frame| !frame.evaluation_stack.is_empty())
    {
        Err(SnapshotExportError::NotSafePoint)
    } else {
        Ok(())
    }
}
