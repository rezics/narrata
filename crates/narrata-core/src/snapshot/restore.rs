use std::{collections::BTreeSet, error::Error, fmt, sync::Arc};

use crate::{
    codec::{DecodeError, ObjectKind, decode_envelope},
    limits::SnapshotLoadLimits,
    program::{CheckedProgram, OpV0, SlotRefV0},
    runtime::{
        FrameStateV0, PendingChoiceItemV0, PendingEffectV0, PendingInteractionV0, RuntimeStateV0,
        RuntimeStatusV0, derive_interaction_id,
    },
    statechart::{DeferredStatechartWorkV0, StateKindV0, StatechartActionV0},
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
    if state.statechart.is_some() != program.statechart().is_some() {
        return Err(SnapshotRestoreError::InvalidState("Statechart presence"));
    }
    let frames: &[FrameStateV0] = match &state.status {
        RuntimeStatusV0::Ready { vm }
        | RuntimeStatusV0::Awaiting { vm, .. }
        | RuntimeStatusV0::AwaitingEffect { vm, .. } => &vm.frames,
        RuntimeStatusV0::Finished { final_frames, .. } => final_frames,
        RuntimeStatusV0::StatechartStable
        | RuntimeStatusV0::AwaitingStatechartEffect { .. }
        | RuntimeStatusV0::StatechartFinished => &[],
    };
    if !frames.is_empty() {
        validate_frames(frames, program)?;
    }
    let mut live_units = state
        .globals
        .values()
        .chain(frames.iter().flat_map(|frame| frame.locals.values()))
        .map(Value::logical_units)
        .sum::<u64>();
    if let Some(chart) = &state.statechart {
        live_units = live_units.saturating_add(
            chart
                .deferred_work
                .iter()
                .filter_map(|work| match work {
                    DeferredStatechartWorkV0::Effect { payload, .. } => {
                        Some(payload.logical_units())
                    }
                    DeferredStatechartWorkV0::Flow { .. } => None,
                })
                .sum::<u64>(),
        );
    }
    if let Some(pending) = state.pending_effect() {
        live_units = live_units.saturating_add(pending.request.payload.logical_units());
    }
    if live_units > limits.runtime.max_total_live_values {
        return Err(SnapshotRestoreError::LimitExceeded("live values"));
    }
    if state.statechart.is_some() {
        validate_statechart_state(state, program, limits)?;
    }
    match &state.status {
        RuntimeStatusV0::Awaiting { vm, pending } => {
            validate_pending(state, vm.frames.last(), pending, program)
        }
        RuntimeStatusV0::AwaitingEffect { vm, pending } => {
            validate_pending_effect(state, vm.frames.last(), pending, program)
        }
        RuntimeStatusV0::AwaitingStatechartEffect { pending } => {
            validate_statechart_effect(state, pending, program)
        }
        RuntimeStatusV0::Ready { .. }
        | RuntimeStatusV0::Finished { .. }
        | RuntimeStatusV0::StatechartStable
        | RuntimeStatusV0::StatechartFinished => Ok(()),
    }
}

fn validate_statechart_state(
    state: &RuntimeStateV0,
    program: &CheckedProgram,
    limits: &SnapshotLoadLimits,
) -> Result<(), SnapshotRestoreError> {
    let chart = state
        .statechart
        .as_ref()
        .ok_or(SnapshotRestoreError::InvalidState(
            "missing Statechart state",
        ))?;
    let history_entries = chart.history.values().map(Vec::len).sum::<usize>();
    let internal_events = chart
        .internal_queue
        .len()
        .saturating_add(chart.deferred_events.len())
        .saturating_add(
            chart
                .invocation
                .as_ref()
                .map_or(0, |invocation| invocation.deferred_events.len()),
        );
    if chart.active.len() as u64 > limits.runtime.max_active_states
        || history_entries as u64 > limits.runtime.max_history_entries
        || internal_events as u64 > limits.runtime.max_internal_events
        || chart.deferred_work.len() as u64 > limits.runtime.max_deferred_work
    {
        return Err(SnapshotRestoreError::LimitExceeded("Statechart state"));
    }
    if state.turn.0 == 0 && matches!(state.status, RuntimeStatusV0::StatechartStable) {
        if !chart.active.is_empty()
            || !chart.history.is_empty()
            || !chart.completed.is_empty()
            || !chart.internal_queue.is_empty()
            || !chart.deferred_events.is_empty()
            || !chart.deferred_work.is_empty()
            || chart.invocation.is_some()
        {
            return Err(SnapshotRestoreError::InvalidState(
                "unstarted Statechart state",
            ));
        }
        return Ok(());
    }
    if chart.active.is_empty() {
        return Err(SnapshotRestoreError::InvalidState(
            "empty active Statechart configuration",
        ));
    }
    let mut regions = BTreeSet::new();
    for leaf in &chart.active {
        let declaration = program
            .state(*leaf)
            .ok_or(SnapshotRestoreError::InvalidState("unknown active StateId"))?;
        if !matches!(declaration.kind, StateKindV0::Atomic | StateKindV0::Final) {
            return Err(SnapshotRestoreError::InvalidState(
                "active configuration contains a non-leaf state",
            ));
        }
        if !regions.insert(declaration.region) {
            return Err(SnapshotRestoreError::InvalidState(
                "active configuration contains two leaves in one region",
            ));
        }
        if chart.active.iter().any(|other| {
            other != leaf
                && crate::statechart::is_descendant(
                    program,
                    program.statechart_indices(),
                    *other,
                    *leaf,
                )
        }) {
            return Err(SnapshotRestoreError::InvalidState(
                "active configuration contains ancestor and descendant",
            ));
        }
    }
    let definition = program
        .statechart()
        .ok_or(SnapshotRestoreError::InvalidState(
            "missing Statechart definition",
        ))?;
    for structural in definition.states.iter().filter(|candidate| {
        matches!(
            candidate.kind,
            StateKindV0::Compound | StateKindV0::Parallel
        ) && chart.active.iter().any(|leaf| {
            crate::statechart::is_descendant(
                program,
                program.statechart_indices(),
                *leaf,
                candidate.id,
            )
        })
    }) {
        for region in definition
            .regions
            .iter()
            .filter(|region| region.parent == structural.id)
        {
            let active_children = chart
                .active
                .iter()
                .filter(|leaf| state_in_region(program, **leaf, region.id))
                .filter_map(|leaf| direct_child(program, structural.id, *leaf))
                .collect::<BTreeSet<_>>();
            if active_children.len() != 1 {
                return Err(SnapshotRestoreError::InvalidState(
                    "active configuration does not cover each entered region exactly once",
                ));
            }
        }
    }
    for (history_id, saved) in &chart.history {
        let declaration = program
            .history(*history_id)
            .ok_or(SnapshotRestoreError::InvalidState("unknown HistoryId"))?;
        if saved.is_empty() {
            return Err(SnapshotRestoreError::InvalidState("empty saved history"));
        }
        for saved_state in saved {
            let node = program
                .state(*saved_state)
                .ok_or(SnapshotRestoreError::InvalidState(
                    "unknown history StateId",
                ))?;
            if !state_in_region(program, *saved_state, declaration.region)
                || (declaration.kind == crate::statechart::HistoryKindV0::Shallow
                    && node.parent != Some(declaration.parent))
            {
                return Err(SnapshotRestoreError::InvalidState(
                    "saved history is outside its declared region",
                ));
            }
        }
    }
    for completed in &chart.completed {
        let node = program
            .state(*completed)
            .ok_or(SnapshotRestoreError::InvalidState(
                "unknown completed StateId",
            ))?;
        if !matches!(
            node.kind,
            StateKindV0::Compound | StateKindV0::Parallel | StateKindV0::Final
        ) {
            return Err(SnapshotRestoreError::InvalidState(
                "invalid completed state kind",
            ));
        }
    }
    let recomputed_completed = definition
        .states
        .iter()
        .filter_map(|node| {
            state_complete_snapshot(program, &chart.active, node.id).then_some(node.id)
        })
        .collect::<BTreeSet<_>>();
    if chart.completed != recomputed_completed {
        return Err(SnapshotRestoreError::InvalidState(
            "completed-state set does not match active configuration",
        ));
    }
    for event in chart
        .internal_queue
        .iter()
        .chain(&chart.deferred_events)
        .chain(
            chart
                .invocation
                .iter()
                .flat_map(|invocation| invocation.deferred_events.iter()),
        )
    {
        if !program
            .statechart()
            .is_some_and(|definition| definition.events.binary_search(event).is_ok())
        {
            return Err(SnapshotRestoreError::InvalidState("unknown EventTypeId"));
        }
    }
    for work in &chart.deferred_work {
        validate_deferred_work(work, program)?;
    }
    if let Some(invocation) = &chart.invocation {
        let flow = program
            .flow(invocation.flow)
            .ok_or(SnapshotRestoreError::InvalidState("invoked Flow missing"))?;
        let action_matches = program
            .statechart_action(invocation.site)
            .is_some_and(|record| {
                matches!(
                    record.action,
                    StatechartActionV0::StartFlow {
                        flow,
                        result_to,
                        done_event,
                    } if flow == invocation.flow
                        && result_to == invocation.result_to
                        && done_event == invocation.done_event
                )
            });
        if !flow.parameters.is_empty()
            || flow.return_kind.is_some() != invocation.result_to.is_some()
            || !action_matches
        {
            return Err(SnapshotRestoreError::InvalidState("invoked Flow contract"));
        }
    }
    let stable = matches!(
        state.status,
        RuntimeStatusV0::StatechartStable | RuntimeStatusV0::StatechartFinished
    );
    if stable
        && (!chart.internal_queue.is_empty()
            || !chart.deferred_events.is_empty()
            || !chart.deferred_work.is_empty()
            || chart.invocation.is_some())
    {
        return Err(SnapshotRestoreError::InvalidState(
            "stable Statechart contains pending work",
        ));
    }
    if matches!(
        state.status,
        RuntimeStatusV0::Awaiting { .. } | RuntimeStatusV0::AwaitingEffect { .. }
    ) != chart.invocation.is_some()
    {
        return Err(SnapshotRestoreError::InvalidState(
            "invoked Flow status mismatch",
        ));
    }
    let root_complete = chart.completed.contains(&definition.root);
    if matches!(state.status, RuntimeStatusV0::StatechartFinished) != root_complete
        || matches!(state.status, RuntimeStatusV0::StatechartStable) && root_complete
    {
        return Err(SnapshotRestoreError::InvalidState(
            "Statechart terminal status mismatch",
        ));
    }
    Ok(())
}

fn state_in_region(
    program: &CheckedProgram,
    mut state: crate::StateId,
    region: crate::RegionId,
) -> bool {
    loop {
        let Some(node) = program.state(state) else {
            return false;
        };
        if node.region == Some(region) {
            return true;
        }
        let Some(parent) = node.parent else {
            return false;
        };
        state = parent;
    }
}

fn direct_child(
    program: &CheckedProgram,
    parent: crate::StateId,
    mut descendant: crate::StateId,
) -> Option<crate::StateId> {
    loop {
        let node = program.state(descendant)?;
        if node.parent == Some(parent) {
            return Some(descendant);
        }
        descendant = node.parent?;
    }
}

fn state_complete_snapshot(
    program: &CheckedProgram,
    active: &BTreeSet<crate::StateId>,
    state: crate::StateId,
) -> bool {
    let Some(node) = program.state(state) else {
        return false;
    };
    match node.kind {
        StateKindV0::Final => active.contains(&state),
        StateKindV0::Atomic => false,
        StateKindV0::Compound | StateKindV0::Parallel => program
            .statechart_indices()
            .regions_by_parent
            .get(&state)
            .is_some_and(|regions| {
                regions.iter().all(|region| {
                    program.statechart().is_some_and(|chart| {
                        chart.states.iter().any(|child| {
                            child.parent == Some(state)
                                && child.region == Some(*region)
                                && child.kind == StateKindV0::Final
                                && active.contains(&child.id)
                        })
                    })
                })
            }),
    }
}

fn validate_deferred_work(
    work: &DeferredStatechartWorkV0,
    program: &CheckedProgram,
) -> Result<(), SnapshotRestoreError> {
    match work {
        DeferredStatechartWorkV0::Flow {
            site,
            owner,
            flow,
            result_to,
            done_event,
        } => {
            let action = program
                .statechart_action(*site)
                .ok_or(SnapshotRestoreError::InvalidState("deferred Flow action"))?;
            if owner.is_some_and(|owner| program.state(owner).is_none())
                || !matches!(action.action, StatechartActionV0::StartFlow { flow: expected, result_to: expected_result, done_event: expected_event } if expected == *flow && expected_result == *result_to && expected_event == *done_event)
            {
                return Err(SnapshotRestoreError::InvalidState("deferred Flow contract"));
            }
        }
        DeferredStatechartWorkV0::Effect {
            site,
            owner,
            capability,
            payload,
            response_to,
            response_event,
        } => {
            let action = program
                .statechart_action(*site)
                .ok_or(SnapshotRestoreError::InvalidState("deferred Effect action"))?;
            if owner.is_some_and(|owner| program.state(owner).is_none())
                || !matches!(&action.action, StatechartActionV0::EmitEffect { capability: expected, payload: expected_payload, response_to: expected_response, response_event: expected_event } if expected == capability && expected_payload == payload && expected_response == response_to && expected_event == response_event)
            {
                return Err(SnapshotRestoreError::InvalidState(
                    "deferred Effect contract",
                ));
            }
        }
    }
    Ok(())
}

fn validate_statechart_effect(
    state: &RuntimeStateV0,
    pending: &PendingEffectV0,
    program: &CheckedProgram,
) -> Result<(), SnapshotRestoreError> {
    let site = pending
        .statechart_site()
        .ok_or(SnapshotRestoreError::InvalidState("Statechart Effect path"))?;
    let action = program
        .statechart_action(site)
        .ok_or(SnapshotRestoreError::InvalidState(
            "Statechart Effect action",
        ))?;
    let StatechartActionV0::EmitEffect {
        capability,
        payload,
        response_to,
        response_event,
    } = &action.action
    else {
        return Err(SnapshotRestoreError::InvalidState(
            "Statechart Effect action kind",
        ));
    };
    let request = &pending.request;
    let declaration = program
        .capability(capability)
        .ok_or(SnapshotRestoreError::InvalidState(
            "Statechart Effect capability",
        ))?;
    if &request.capability != capability
        || &request.payload != payload
        || request.capability_version != declaration.version
        || request.delivery != declaration.delivery
        || request.rewind != declaration.rewind
        || pending.statechart_continuation() != Some((*response_to, *response_event))
        || request.execution != state.execution_id
    {
        return Err(SnapshotRestoreError::InvalidState(
            "Statechart Effect contract",
        ));
    }
    let payload_digest = crate::effect_payload_digest(&request.payload);
    let request_digest = crate::effect_request_digest(
        &request.capability,
        request.capability_version,
        payload_digest,
        request.delivery,
        &request.rewind,
    );
    let effect = crate::derive_statechart_effect_id(
        state.execution_id,
        pending.origin_parent_commit,
        pending.origin_input_digest,
        site,
        pending.occurrence,
        request_digest,
    );
    if request.payload_digest != payload_digest
        || request.request_digest != request_digest
        || request.id != effect
        || state.interaction_counter != pending.occurrence.saturating_add(1)
    {
        return Err(SnapshotRestoreError::InvalidState(
            "Statechart Effect identity",
        ));
    }
    Ok(())
}

fn validate_pending_effect(
    state: &RuntimeStateV0,
    frame: Option<&FrameStateV0>,
    pending: &PendingEffectV0,
    program: &CheckedProgram,
) -> Result<(), SnapshotRestoreError> {
    let origin_instruction = pending
        .flow_origin()
        .ok_or(SnapshotRestoreError::InvalidState("Effect origin kind"))?;
    let resume_to = pending
        .flow_continuation()
        .ok_or(SnapshotRestoreError::InvalidState(
            "Effect continuation kind",
        ))?;
    let frame = frame.ok_or(SnapshotRestoreError::InvalidState(
        "pending Effect without frame",
    ))?;
    if frame.instruction != origin_instruction {
        return Err(SnapshotRestoreError::InvalidState(
            "Effect origin instruction",
        ));
    }
    let instruction = program.instruction(frame.flow, origin_instruction).ok_or(
        SnapshotRestoreError::InvalidState("Effect instruction missing"),
    )?;
    let OpV0::Effect { capability, next } = &instruction.op else {
        return Err(SnapshotRestoreError::InvalidState(
            "pending Effect points to another opcode",
        ));
    };
    let declaration = program
        .capability(capability)
        .ok_or(SnapshotRestoreError::InvalidState(
            "pending Effect capability missing",
        ))?;
    let request = &pending.request;
    if &request.capability != capability
        || request.capability_version != declaration.version
        || request.delivery != declaration.delivery
        || request.rewind != declaration.rewind
        || resume_to != *next
        || request.execution != state.execution_id
        || !declaration.request_schema.accepts(&request.payload)
    {
        return Err(SnapshotRestoreError::InvalidState(
            "pending Effect contract",
        ));
    }
    let payload_digest = crate::effect_payload_digest(&request.payload);
    let request_digest = crate::effect_request_digest(
        &request.capability,
        request.capability_version,
        payload_digest,
        request.delivery,
        &request.rewind,
    );
    let effect = crate::derive_effect_id(
        state.execution_id,
        pending.origin_parent_commit,
        pending.origin_input_digest,
        origin_instruction,
        pending.occurrence,
        request_digest,
    );
    if request.payload_digest != payload_digest
        || request.request_digest != request_digest
        || request.id != effect
        || state.interaction_counter != pending.occurrence.saturating_add(1)
    {
        return Err(SnapshotRestoreError::InvalidState(
            "pending Effect identity",
        ));
    }
    Ok(())
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
