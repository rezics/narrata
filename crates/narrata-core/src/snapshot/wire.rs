use std::{collections::BTreeMap, sync::Arc};

use crate::{
    CapabilityId, CapabilityVersion, DeliveryPolicy, EffectRequestV0, RewindPolicy,
    codec::{CborReader, CborWriter, DecodeError},
    identity::{
        ActionId, ChoiceId, CommitId, EffectId, EffectPayloadDigest, EffectRequestDigest,
        EventTypeId, ExecutionId, FlowId, GlobalId, InputPayloadDigest, InstructionId,
        InteractionId, LocalId, ProgramArtifactId, StateDigest,
    },
    limits::SnapshotLoadLimits,
    runtime::{
        EffectPathV0, FrameStateV0, PendingChoiceItemV0, PendingEffectV0, PendingInteractionV0,
        RuntimeStateV0, RuntimeStatusV0, Turn, VmStateV0,
    },
    scene::{SceneState, decode_scene, encode_scene},
    statechart::{decode_statechart_state, encode_statechart_state},
    version::SemanticsVersion,
};

pub(crate) fn encode_state_payload(state: &RuntimeStateV0) -> Vec<u8> {
    let mut writer = CborWriter::new();
    let has_statechart = state.statechart.is_some();
    let has_scene = state.scene != SceneState::default() || has_statechart;
    let field_count = 7 + u64::from(has_scene) + u64::from(has_statechart);
    writer.map(field_count);
    writer.unsigned(0);
    writer.unsigned(u64::from(state.semantics_version.get()));
    writer.unsigned(1);
    writer.bytes(state.execution_id.as_bytes());
    writer.unsigned(2);
    writer.bytes(state.program_artifact_id.as_bytes());
    writer.unsigned(3);
    writer.unsigned(state.turn.0);
    writer.unsigned(4);
    writer.unsigned(state.interaction_counter);
    writer.unsigned(5);
    writer.map(state.globals.len() as u64);
    for (id, value) in &state.globals {
        writer.bytes(id.as_bytes());
        crate::value::encode_value(&mut writer, value);
    }
    writer.unsigned(6);
    encode_status(&mut writer, &state.status);
    if has_scene {
        writer.unsigned(7);
        encode_scene(&mut writer, &state.scene);
    }
    if let Some(statechart) = &state.statechart {
        writer.unsigned(8);
        encode_statechart_state(&mut writer, statechart);
    }
    writer.into_bytes()
}

fn encode_status(writer: &mut CborWriter, status: &RuntimeStatusV0) {
    match status {
        RuntimeStatusV0::Ready { vm } => {
            writer.array(2);
            writer.unsigned(0);
            encode_vm(writer, vm);
        }
        RuntimeStatusV0::Awaiting { vm, pending } => {
            writer.array(3);
            writer.unsigned(1);
            encode_vm(writer, vm);
            encode_pending(writer, pending);
        }
        RuntimeStatusV0::AwaitingEffect { vm, pending } => {
            writer.array(3);
            writer.unsigned(3);
            encode_vm(writer, vm);
            encode_pending_effect(writer, pending);
        }
        RuntimeStatusV0::Finished {
            result,
            final_frames,
        } => {
            writer.array(3);
            writer.unsigned(2);
            crate::value::encode_value(writer, result);
            writer.array(final_frames.len() as u64);
            for frame in final_frames {
                encode_frame(writer, frame);
            }
        }
        RuntimeStatusV0::StatechartStable => {
            writer.array(1);
            writer.unsigned(4);
        }
        RuntimeStatusV0::AwaitingStatechartEffect { pending } => {
            writer.array(2);
            writer.unsigned(5);
            encode_pending_effect(writer, pending);
        }
        RuntimeStatusV0::StatechartFinished => {
            writer.array(1);
            writer.unsigned(6);
        }
    }
}

fn encode_pending_effect(writer: &mut CborWriter, pending: &PendingEffectV0) {
    match pending.path {
        EffectPathV0::Flow { origin, resume_to } => {
            // Preserve the Stage 3 canonical representation byte-for-byte.
            writer.array(7);
            encode_effect_request(writer, &pending.request);
            writer.bytes(origin.as_bytes());
            writer.bytes(pending.origin_parent_commit.as_bytes());
            writer.bytes(pending.origin_parent_state.as_bytes());
            writer.bytes(pending.origin_input_digest.as_bytes());
            writer.unsigned(pending.occurrence);
            writer.bytes(resume_to.as_bytes());
        }
        EffectPathV0::Statechart {
            site,
            response_to,
            response_event,
        } => {
            writer.array(9);
            encode_effect_request(writer, &pending.request);
            writer.bytes(site.as_bytes());
            writer.bytes(pending.origin_parent_commit.as_bytes());
            writer.bytes(pending.origin_parent_state.as_bytes());
            writer.bytes(pending.origin_input_digest.as_bytes());
            writer.unsigned(pending.occurrence);
            writer.unsigned(1);
            match response_to {
                Some(global) => writer.bytes(global.as_bytes()),
                None => writer.null(),
            }
            match response_event {
                Some(event) => writer.bytes(event.as_bytes()),
                None => writer.null(),
            }
        }
    }
}

fn encode_effect_request(writer: &mut CborWriter, request: &EffectRequestV0) {
    writer.array(9);
    writer.bytes(request.id.as_bytes());
    writer.bytes(request.execution.as_bytes());
    writer.text(request.capability.as_str());
    writer.unsigned(u64::from(request.capability_version.get()));
    crate::value::encode_value(writer, &request.payload);
    writer.bytes(request.payload_digest.as_bytes());
    writer.bytes(request.request_digest.as_bytes());
    writer.unsigned(request.delivery as u64);
    crate::effect::encode_rewind_policy(writer, &request.rewind);
}

fn encode_vm(writer: &mut CborWriter, vm: &VmStateV0) {
    writer.array(vm.frames.len() as u64);
    for frame in &vm.frames {
        encode_frame(writer, frame);
    }
}

fn encode_frame(writer: &mut CborWriter, frame: &FrameStateV0) {
    writer.array(5);
    writer.bytes(frame.flow.as_bytes());
    writer.bytes(frame.instruction.as_bytes());
    match frame.return_to {
        Some(instruction) => writer.bytes(instruction.as_bytes()),
        None => writer.null(),
    }
    writer.map(frame.locals.len() as u64);
    for (id, value) in &frame.locals {
        writer.bytes(id.as_bytes());
        crate::value::encode_value(writer, value);
    }
    writer.array(frame.evaluation_stack.len() as u64);
    for value in &frame.evaluation_stack {
        crate::value::encode_value(writer, value);
    }
}

fn encode_pending(writer: &mut CborWriter, pending: &PendingInteractionV0) {
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
            writer.array(9);
            writer.unsigned(0);
            writer.bytes(interaction_id.as_bytes());
            writer.bytes(origin_instruction.as_bytes());
            writer.bytes(origin_parent_state.as_bytes());
            writer.bytes(origin_input_digest.as_bytes());
            writer.unsigned(*occurrence);
            match speaker {
                Some(speaker) => writer.text(speaker),
                None => writer.null(),
            }
            writer.text(text);
            writer.bytes(resume_to.as_bytes());
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
            writer.array(8);
            writer.unsigned(1);
            writer.bytes(interaction_id.as_bytes());
            writer.bytes(origin_instruction.as_bytes());
            writer.bytes(origin_parent_state.as_bytes());
            writer.bytes(origin_input_digest.as_bytes());
            writer.unsigned(*occurrence);
            match prompt {
                Some(prompt) => writer.text(prompt),
                None => writer.null(),
            }
            writer.array(offered.len() as u64);
            for choice in offered {
                writer.array(3);
                writer.bytes(choice.id.as_bytes());
                writer.text(&choice.label);
                writer.bytes(choice.target_for_runtime().as_bytes());
            }
        }
    }
}

pub(crate) fn decode_state_payload(
    bytes: &[u8],
    limits: &SnapshotLoadLimits,
) -> Result<RuntimeStateV0, DecodeError> {
    let mut reader = CborReader::new(bytes);
    let field_count = reader.map_len()?;
    if !(7..=9).contains(&field_count) {
        return Err(DecodeError::Schema("Runtime State field count"));
    }
    expect_key(&mut reader, 0)?;
    let semantics_version = SemanticsVersion::new(read_u16(&mut reader)?);
    expect_key(&mut reader, 1)?;
    let execution_id = ExecutionId::from_bytes(reader.bytes_exact::<16>()?);
    expect_key(&mut reader, 2)?;
    let program_artifact_id = ProgramArtifactId::from_bytes(reader.bytes_exact::<32>()?);
    expect_key(&mut reader, 3)?;
    let turn = Turn(reader.unsigned()?);
    expect_key(&mut reader, 4)?;
    let interaction_counter = reader.unsigned()?;
    expect_key(&mut reader, 5)?;
    let globals_len = reader.map_len()?;
    if globals_len > limits.decode.max_collection_items {
        return Err(DecodeError::Limit("Runtime globals"));
    }
    let mut globals = BTreeMap::new();
    let mut previous_global = None;
    let mut value_nodes = 0_u64;
    for _ in 0..globals_len {
        let raw = reader.bytes_exact::<16>()?;
        if previous_global.is_some_and(|previous| previous >= raw) {
            return Err(DecodeError::NonCanonical("Runtime global key order"));
        }
        previous_global = Some(raw);
        globals.insert(
            GlobalId::from_bytes(raw),
            crate::value::decode_value(&mut reader, &limits.decode, 1, &mut value_nodes)?,
        );
    }
    expect_key(&mut reader, 6)?;
    let status = decode_status(&mut reader, limits, &mut value_nodes)?;
    let scene = if field_count >= 8 {
        expect_key(&mut reader, 7)?;
        decode_scene(&mut reader, &limits.decode)?
    } else {
        SceneState::default()
    };
    let statechart = if field_count == 9 {
        expect_key(&mut reader, 8)?;
        Some(decode_statechart_state(
            &mut reader,
            limits,
            &mut value_nodes,
        )?)
    } else {
        None
    };
    reader.finish()?;
    Ok(RuntimeStateV0 {
        semantics_version,
        execution_id,
        program_artifact_id,
        turn,
        interaction_counter,
        globals,
        scene,
        statechart,
        status,
    })
}

fn decode_status(
    reader: &mut CborReader<'_>,
    limits: &SnapshotLoadLimits,
    nodes: &mut u64,
) -> Result<RuntimeStatusV0, DecodeError> {
    let length = reader.array_len()?;
    let tag = reader.unsigned()?;
    match (tag, length) {
        (0, 2) => Ok(RuntimeStatusV0::Ready {
            vm: decode_vm(reader, limits, nodes)?,
        }),
        (1, 3) => Ok(RuntimeStatusV0::Awaiting {
            vm: decode_vm(reader, limits, nodes)?,
            pending: decode_pending(reader, limits)?,
        }),
        (2, 3) => {
            let result = crate::value::decode_value(reader, &limits.decode, 1, nodes)?;
            let frames_len = bounded_frames(reader, limits)?;
            let capacity = usize::try_from(frames_len).map_err(|_| DecodeError::LengthOverflow)?;
            let mut final_frames = Vec::with_capacity(capacity);
            for _ in 0..frames_len {
                final_frames.push(decode_frame(reader, limits, nodes)?);
            }
            Ok(RuntimeStatusV0::Finished {
                result,
                final_frames,
            })
        }
        (3, 3) => Ok(RuntimeStatusV0::AwaitingEffect {
            vm: decode_vm(reader, limits, nodes)?,
            pending: decode_pending_effect(reader, limits, nodes)?,
        }),
        (4, 1) => Ok(RuntimeStatusV0::StatechartStable),
        (5, 2) => Ok(RuntimeStatusV0::AwaitingStatechartEffect {
            pending: decode_pending_effect(reader, limits, nodes)?,
        }),
        (6, 1) => Ok(RuntimeStatusV0::StatechartFinished),
        _ => Err(DecodeError::Schema("Runtime status")),
    }
}

fn decode_pending_effect(
    reader: &mut CborReader<'_>,
    limits: &SnapshotLoadLimits,
    nodes: &mut u64,
) -> Result<PendingEffectV0, DecodeError> {
    let length = reader.array_len()?;
    if !matches!(length, 7 | 9) {
        return Err(DecodeError::Schema("pending Effect"));
    }
    let request = decode_effect_request(reader, limits, nodes)?;
    let origin = reader.bytes_exact::<16>()?;
    let origin_parent_commit = CommitId::from_bytes(reader.bytes_exact::<32>()?);
    let origin_parent_state = StateDigest::from_bytes(reader.bytes_exact::<32>()?);
    let origin_input_digest = InputPayloadDigest::from_bytes(reader.bytes_exact::<32>()?);
    let occurrence = reader.unsigned()?;
    let path = if length == 7 {
        EffectPathV0::Flow {
            origin: InstructionId::from_bytes(origin),
            resume_to: InstructionId::from_bytes(reader.bytes_exact::<16>()?),
        }
    } else {
        if reader.unsigned()? != 1 {
            return Err(DecodeError::Schema("pending Statechart Effect tag"));
        }
        EffectPathV0::Statechart {
            site: ActionId::from_bytes(origin),
            response_to: reader
                .optional(|reader| reader.bytes_exact::<16>())?
                .map(GlobalId::from_bytes),
            response_event: reader
                .optional(|reader| reader.bytes_exact::<16>())?
                .map(EventTypeId::from_bytes),
        }
    };
    Ok(PendingEffectV0 {
        request,
        path,
        origin_parent_commit,
        origin_parent_state,
        origin_input_digest,
        occurrence,
    })
}

fn decode_effect_request(
    reader: &mut CborReader<'_>,
    limits: &SnapshotLoadLimits,
    nodes: &mut u64,
) -> Result<EffectRequestV0, DecodeError> {
    if reader.array_len()? != 9 {
        return Err(DecodeError::Schema("Effect request"));
    }
    let id = EffectId::from_bytes(reader.bytes_exact::<32>()?);
    let execution = ExecutionId::from_bytes(reader.bytes_exact::<16>()?);
    let capability = CapabilityId::new(reader.text(limits.decode.max_string_bytes)?)
        .map_err(|_| DecodeError::Schema("Effect capability"))?;
    let capability_version = CapabilityVersion::new(read_u16(reader)?)
        .ok_or(DecodeError::Schema("Effect capability version"))?;
    let payload = crate::value::decode_value(reader, &limits.decode, 1, nodes)?;
    let payload_digest = EffectPayloadDigest::from_bytes(reader.bytes_exact::<32>()?);
    let request_digest = EffectRequestDigest::from_bytes(reader.bytes_exact::<32>()?);
    let delivery = DeliveryPolicy::from_u64(reader.unsigned()?)
        .ok_or(DecodeError::Schema("Effect delivery policy"))?;
    let rewind = decode_rewind(reader, limits)?;
    Ok(EffectRequestV0 {
        id,
        execution,
        capability,
        capability_version,
        payload,
        payload_digest,
        request_digest,
        delivery,
        rewind,
    })
}

fn decode_rewind(
    reader: &mut CborReader<'_>,
    limits: &SnapshotLoadLimits,
) -> Result<RewindPolicy, DecodeError> {
    let length = reader.array_len()?;
    match (reader.unsigned()?, length) {
        (0, 1) => Ok(RewindPolicy::Reapply),
        (1, 1) => Ok(RewindPolicy::ReuseRecordedResponse),
        (2, 1) => Ok(RewindPolicy::Barrier),
        (3, 2) => Ok(RewindPolicy::Compensatable {
            capability: CapabilityId::new(reader.text(limits.decode.max_string_bytes)?)
                .map_err(|_| DecodeError::Schema("compensation capability"))?,
        }),
        _ => Err(DecodeError::Schema("Effect rewind policy")),
    }
}

fn decode_vm(
    reader: &mut CborReader<'_>,
    limits: &SnapshotLoadLimits,
    nodes: &mut u64,
) -> Result<VmStateV0, DecodeError> {
    let frames_len = bounded_frames(reader, limits)?;
    let capacity = usize::try_from(frames_len).map_err(|_| DecodeError::LengthOverflow)?;
    let mut frames = Vec::with_capacity(capacity);
    for _ in 0..frames_len {
        frames.push(decode_frame(reader, limits, nodes)?);
    }
    Ok(VmStateV0 { frames })
}

fn decode_frame(
    reader: &mut CborReader<'_>,
    limits: &SnapshotLoadLimits,
    nodes: &mut u64,
) -> Result<FrameStateV0, DecodeError> {
    if reader.array_len()? != 5 {
        return Err(DecodeError::Schema("VM frame"));
    }
    let flow = FlowId::from_bytes(reader.bytes_exact::<16>()?);
    let instruction = InstructionId::from_bytes(reader.bytes_exact::<16>()?);
    let return_to =
        reader.optional(|reader| reader.bytes_exact::<16>().map(InstructionId::from_bytes))?;
    let locals_len = reader.map_len()?;
    if locals_len > limits.decode.max_collection_items {
        return Err(DecodeError::Limit("frame locals"));
    }
    let mut locals = BTreeMap::new();
    let mut previous_local = None;
    for _ in 0..locals_len {
        let raw = reader.bytes_exact::<16>()?;
        if previous_local.is_some_and(|previous| previous >= raw) {
            return Err(DecodeError::NonCanonical("frame local key order"));
        }
        previous_local = Some(raw);
        locals.insert(
            LocalId::from_bytes(raw),
            crate::value::decode_value(reader, &limits.decode, 1, nodes)?,
        );
    }
    let stack_len = reader.array_len()?;
    if stack_len > limits.runtime.max_stack_depth {
        return Err(DecodeError::Limit("evaluation stack"));
    }
    let stack_capacity = usize::try_from(stack_len).map_err(|_| DecodeError::LengthOverflow)?;
    let mut evaluation_stack = Vec::with_capacity(stack_capacity);
    for _ in 0..stack_len {
        evaluation_stack.push(crate::value::decode_value(
            reader,
            &limits.decode,
            1,
            nodes,
        )?);
    }
    Ok(FrameStateV0 {
        flow,
        instruction,
        return_to,
        locals,
        evaluation_stack,
    })
}

fn decode_pending(
    reader: &mut CborReader<'_>,
    limits: &SnapshotLoadLimits,
) -> Result<PendingInteractionV0, DecodeError> {
    let length = reader.array_len()?;
    let tag = reader.unsigned()?;
    let interaction_id = InteractionId::from_bytes(reader.bytes_exact::<32>()?);
    let origin_instruction = InstructionId::from_bytes(reader.bytes_exact::<16>()?);
    let origin_parent_state = StateDigest::from_bytes(reader.bytes_exact::<32>()?);
    let origin_input_digest = InputPayloadDigest::from_bytes(reader.bytes_exact::<32>()?);
    let occurrence = reader.unsigned()?;
    match (tag, length) {
        (0, 9) => Ok(PendingInteractionV0::Say {
            interaction_id,
            origin_instruction,
            origin_parent_state,
            origin_input_digest,
            occurrence,
            speaker: reader
                .optional(|reader| reader.text(limits.decode.max_string_bytes))?
                .map(Arc::from),
            text: Arc::from(reader.text(limits.decode.max_string_bytes)?),
            resume_to: InstructionId::from_bytes(reader.bytes_exact::<16>()?),
        }),
        (1, 8) => {
            let prompt = reader
                .optional(|reader| reader.text(limits.decode.max_string_bytes))?
                .map(Arc::from);
            let offered_len = reader.array_len()?;
            if offered_len > limits.decode.max_collection_items {
                return Err(DecodeError::Limit("offered choices"));
            }
            let capacity = usize::try_from(offered_len).map_err(|_| DecodeError::LengthOverflow)?;
            let mut offered = Vec::with_capacity(capacity);
            for _ in 0..offered_len {
                if reader.array_len()? != 3 {
                    return Err(DecodeError::Schema("offered choice"));
                }
                offered.push(PendingChoiceItemV0 {
                    id: ChoiceId::from_bytes(reader.bytes_exact::<16>()?),
                    label: Arc::from(reader.text(limits.decode.max_string_bytes)?),
                    target: InstructionId::from_bytes(reader.bytes_exact::<16>()?),
                });
            }
            Ok(PendingInteractionV0::Choice {
                interaction_id,
                origin_instruction,
                origin_parent_state,
                origin_input_digest,
                occurrence,
                prompt,
                offered,
            })
        }
        _ => Err(DecodeError::Schema("pending interaction")),
    }
}

fn bounded_frames(
    reader: &mut CborReader<'_>,
    limits: &SnapshotLoadLimits,
) -> Result<u64, DecodeError> {
    let length = reader.array_len()?;
    if length == 0 {
        return Err(DecodeError::Schema("VM must contain a root frame"));
    }
    if length > limits.runtime.max_call_depth {
        Err(DecodeError::Limit("VM call depth"))
    } else {
        Ok(length)
    }
}

fn expect_key(reader: &mut CborReader<'_>, expected: u64) -> Result<(), DecodeError> {
    if reader.unsigned()? == expected {
        Ok(())
    } else {
        Err(DecodeError::NonCanonical(
            "state map key order or unknown field",
        ))
    }
}

fn read_u16(reader: &mut CborReader<'_>) -> Result<u16, DecodeError> {
    u16::try_from(reader.unsigned()?).map_err(|_| DecodeError::IntegerOverflow)
}
