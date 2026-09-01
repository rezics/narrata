use std::{error::Error, fmt, sync::Arc};

use crate::{
    codec::{DecodeError, ObjectKind, decode_envelope},
    limits::SnapshotLoadLimits,
    program::{CheckedProgram, OpV0, SlotRefV0},
    runtime::{
        FrameStateV0, PendingChoiceItemV0, PendingInteractionV0, RuntimeStateV0, RuntimeStatusV0,
        derive_interaction_id,
    },
    value::Value,
    version::{SEMANTICS_V0, SNAPSHOT_SCHEMA_V0},
};

use super::{decode_state_payload, encode_state_payload};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotRestoreError {
    Decode(DecodeError),
    Incompatible(&'static str),
    InvalidState(&'static str),
    LimitExceeded(&'static str),
}

impl fmt::Display for SnapshotRestoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(error) => write!(formatter, "Snapshot decode failed: {error}"),
            Self::Incompatible(message) => write!(formatter, "Snapshot is incompatible: {message}"),
            Self::InvalidState(message) => {
                write!(formatter, "Snapshot state is invalid: {message}")
            }
            Self::LimitExceeded(message) => write!(formatter, "Snapshot exceeds limit: {message}"),
        }
    }
}

impl Error for SnapshotRestoreError {}

pub fn restore_snapshot(
    bytes: &[u8],
    program: &CheckedProgram,
    limits: &SnapshotLoadLimits,
) -> Result<RuntimeStateV0, SnapshotRestoreError> {
    let envelope = decode_envelope(
        bytes,
        ObjectKind::Snapshot,
        SNAPSHOT_SCHEMA_V0.get(),
        &limits.decode,
    )
    .map_err(SnapshotRestoreError::Decode)?;
    let state =
        decode_state_payload(envelope.payload, limits).map_err(SnapshotRestoreError::Decode)?;
    if encode_state_payload(&state) != envelope.payload {
        return Err(SnapshotRestoreError::Decode(DecodeError::NonCanonical(
            "Snapshot payload round-trip mismatch",
        )));
    }
    validate_state(&state, program, limits)?;
    Ok(state)
}

fn validate_state(
    state: &RuntimeStateV0,
    program: &CheckedProgram,
    limits: &SnapshotLoadLimits,
) -> Result<(), SnapshotRestoreError> {
    if state.semantics_version != SEMANTICS_V0
        || state.semantics_version != program.artifact().semantics_version
    {
        return Err(SnapshotRestoreError::Incompatible("semantics version"));
    }
    if state.program_artifact_id != program.artifact_id() {
        return Err(SnapshotRestoreError::Incompatible(
            "Program Artifact identity",
        ));
    }
    if state.globals.len() != program.artifact().globals.len() {
        return Err(SnapshotRestoreError::InvalidState("global slot set"));
    }
    for declaration in &program.artifact().globals {
        let value = state
            .globals
            .get(&declaration.id)
            .ok_or(SnapshotRestoreError::InvalidState("missing global slot"))?;
        if value.kind() != declaration.kind {
            return Err(SnapshotRestoreError::InvalidState("global Value kind"));
        }
    }
    let frames = match &state.status {
        RuntimeStatusV0::Ready { vm } | RuntimeStatusV0::Awaiting { vm, .. } => &vm.frames,
        RuntimeStatusV0::Finished { final_frames, .. } => final_frames,
    };
    validate_frames(frames, program)?;
    let live_units = state
        .globals
        .values()
        .chain(frames.iter().flat_map(|frame| frame.locals.values()))
        .map(Value::logical_units)
        .sum::<u64>();
    if live_units > limits.runtime.max_total_live_values {
        return Err(SnapshotRestoreError::LimitExceeded("live values"));
    }
    match &state.status {
        RuntimeStatusV0::Awaiting { vm, pending } => {
            validate_pending(state, vm.frames.last(), pending, program)
        }
        RuntimeStatusV0::Ready { .. } | RuntimeStatusV0::Finished { .. } => Ok(()),
    }
}

fn validate_frames(
    frames: &[FrameStateV0],
    program: &CheckedProgram,
) -> Result<(), SnapshotRestoreError> {
    if frames.is_empty() {
        return Err(SnapshotRestoreError::InvalidState("empty frame stack"));
    }
    for (index, frame) in frames.iter().enumerate() {
        let flow = program
            .flow(frame.flow)
            .ok_or(SnapshotRestoreError::InvalidState("unknown frame Flow"))?;
        if program.instruction(frame.flow, frame.instruction).is_none() {
            return Err(SnapshotRestoreError::InvalidState(
                "unknown frame instruction",
            ));
        }
        if !frame.evaluation_stack.is_empty() {
            return Err(SnapshotRestoreError::InvalidState(
                "safe-point evaluation stack is not empty",
            ));
        }
        if frame.locals.len() != flow.parameters.len().saturating_add(flow.locals.len()) {
            return Err(SnapshotRestoreError::InvalidState("frame local slot set"));
        }
        for declaration in flow.parameters.iter().chain(&flow.locals) {
            let value = frame
                .locals
                .get(&declaration.id)
                .ok_or(SnapshotRestoreError::InvalidState("missing frame local"))?;
            if value.kind() != declaration.kind {
                return Err(SnapshotRestoreError::InvalidState("frame local Value kind"));
            }
        }
        match (index, frame.return_to) {
            (0, None) => {}
            (0, Some(_)) | (_, None) => {
                return Err(SnapshotRestoreError::InvalidState(
                    "frame return target shape",
                ));
            }
            (_, Some(return_to)) => {
                let caller = frames
                    .get(index.saturating_sub(1))
                    .ok_or(SnapshotRestoreError::InvalidState("missing caller frame"))?;
                if program.instruction(caller.flow, return_to).is_none() {
                    return Err(SnapshotRestoreError::InvalidState(
                        "return target is not in caller Flow",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_pending(
    state: &RuntimeStateV0,
    frame: Option<&FrameStateV0>,
    pending: &PendingInteractionV0,
    program: &CheckedProgram,
) -> Result<(), SnapshotRestoreError> {
    let frame = frame.ok_or(SnapshotRestoreError::InvalidState(
        "pending interaction without frame",
    ))?;
    match pending {
        PendingInteractionV0::Say {
            interaction_id,
            origin_instruction,
            origin_parent_state,
            origin_input_digest,
            occurrence,
            speaker,
            text,
            resume_to,
        } => {
            if frame.instruction != *origin_instruction {
                return Err(SnapshotRestoreError::InvalidState("Say origin instruction"));
            }
            let instruction = program.instruction(frame.flow, *origin_instruction).ok_or(
                SnapshotRestoreError::InvalidState("Say instruction missing"),
            )?;
            let OpV0::Say {
                speaker: expected_speaker,
                text: expected_text,
                next,
            } = instruction.op
            else {
                return Err(SnapshotRestoreError::InvalidState(
                    "pending Say points to another opcode",
                ));
            };
            if *resume_to != next
                || speaker.as_deref() != optional_text(program, expected_speaker)?.as_deref()
                || text.as_ref() != constant_text(program, expected_text)?.as_ref()
            {
                return Err(SnapshotRestoreError::InvalidState("pending Say payload"));
            }
            let expected_id = derive_interaction_id(
                state.execution_id,
                *origin_parent_state,
                *origin_input_digest,
                *origin_instruction,
                *occurrence,
                0,
            );
            if *interaction_id != expected_id {
                return Err(SnapshotRestoreError::InvalidState("Say InteractionId"));
            }
        }
        PendingInteractionV0::Choice {
            interaction_id,
            origin_instruction,
            origin_parent_state,
            origin_input_digest,
            occurrence,
            prompt,
            offered,
        } => {
            if frame.instruction != *origin_instruction {
                return Err(SnapshotRestoreError::InvalidState(
                    "Choice origin instruction",
                ));
            }
            let instruction = program.instruction(frame.flow, *origin_instruction).ok_or(
                SnapshotRestoreError::InvalidState("Choice instruction missing"),
            )?;
            let OpV0::Choice {
                prompt: expected_prompt,
                choices,
            } = &instruction.op
            else {
                return Err(SnapshotRestoreError::InvalidState(
                    "pending Choice points to another opcode",
                ));
            };
            if prompt.as_deref() != optional_text(program, *expected_prompt)?.as_deref() {
                return Err(SnapshotRestoreError::InvalidState("Choice prompt"));
            }
            let recomputed = recompute_offered(state, frame, choices, program)?;
            if offered != &recomputed {
                return Err(SnapshotRestoreError::InvalidState(
                    "Choice offered set does not match state",
                ));
            }
            let expected_id = derive_interaction_id(
                state.execution_id,
                *origin_parent_state,
                *origin_input_digest,
                *origin_instruction,
                *occurrence,
                1,
            );
            if *interaction_id != expected_id {
                return Err(SnapshotRestoreError::InvalidState("Choice InteractionId"));
            }
        }
    }
    Ok(())
}

fn recompute_offered(
    state: &RuntimeStateV0,
    frame: &FrameStateV0,
    choices: &[crate::program::ChoiceArmV0],
    program: &CheckedProgram,
) -> Result<Vec<PendingChoiceItemV0>, SnapshotRestoreError> {
    let mut offered = Vec::new();
    for choice in choices {
        let visible = match choice.visible_if {
            Some(SlotRefV0::Global(id)) => match state.globals.get(&id) {
                Some(Value::Bool(value)) => *value,
                _ => {
                    return Err(SnapshotRestoreError::InvalidState(
                        "Choice global visibility",
                    ));
                }
            },
            Some(SlotRefV0::Local(id)) => match frame.locals.get(&id) {
                Some(Value::Bool(value)) => *value,
                _ => {
                    return Err(SnapshotRestoreError::InvalidState(
                        "Choice local visibility",
                    ));
                }
            },
            None => true,
        };
        if visible {
            offered.push(PendingChoiceItemV0 {
                id: choice.id,
                label: constant_text(program, choice.label)?,
                target: choice.target,
            });
        }
    }
    if offered.is_empty() {
        return Err(SnapshotRestoreError::InvalidState(
            "Choice has no visible items",
        ));
    }
    Ok(offered)
}

fn optional_text(
    program: &CheckedProgram,
    index: Option<crate::program::ConstIndex>,
) -> Result<Option<Arc<str>>, SnapshotRestoreError> {
    index.map(|index| constant_text(program, index)).transpose()
}

fn constant_text(
    program: &CheckedProgram,
    index: crate::program::ConstIndex,
) -> Result<Arc<str>, SnapshotRestoreError> {
    match program.constant(index) {
        Some(Value::String(text)) => Ok(text.clone()),
        _ => Err(SnapshotRestoreError::InvalidState(
            "interaction text constant",
        )),
    }
}
