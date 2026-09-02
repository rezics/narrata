use crate::{
    ActionId, CapabilityId, EventTypeId, FlowId, GlobalId, HistoryId, RegionId, StateId,
    TransitionId,
    codec::{CborReader, CborWriter, DecodeError},
    diagnostic::SourceSpan,
    limits::ProgramLoadLimits,
};

use super::{
    ActionRecordV0, GuardExprV0, HistoryKindV0, HistoryV0, RegionV0, StateKindV0, StateV0,
    StatechartActionV0, StatechartV0, TransitionKindV0, TransitionTargetV0, TransitionV0,
};

pub(crate) fn encode_statechart(writer: &mut CborWriter, chart: &StatechartV0) {
    writer.array(7);
    writer.bytes(chart.root.as_bytes());
    writer.array(chart.events.len() as u64);
    for event in &chart.events {
        writer.bytes(event.as_bytes());
    }
    writer.array(chart.states.len() as u64);
    for state in &chart.states {
        encode_state(writer, state);
    }
    writer.array(chart.regions.len() as u64);
    for region in &chart.regions {
        encode_region(writer, region);
    }
    writer.array(chart.histories.len() as u64);
    for history in &chart.histories {
        encode_history(writer, history);
    }
    writer.array(chart.transitions.len() as u64);
    for transition in &chart.transitions {
        encode_transition(writer, transition);
    }
    encode_span(writer, chart.source_span);
}

pub(crate) fn decode_statechart(
    reader: &mut CborReader<'_>,
    limits: &ProgramLoadLimits,
    value_nodes: &mut u64,
) -> Result<StatechartV0, DecodeError> {
    expect_array(reader, 7, "Statechart")?;
    let root = StateId::from_bytes(reader.bytes_exact::<16>()?);
    let event_count = bounded_array(reader, limits.program.max_transitions, "Statechart events")?;
    let mut events = Vec::with_capacity(capacity(event_count)?);
    for _ in 0..event_count {
        events.push(EventTypeId::from_bytes(reader.bytes_exact::<16>()?));
    }
    let state_count = bounded_array(reader, limits.program.max_states, "Statechart states")?;
    let mut states = Vec::with_capacity(capacity(state_count)?);
    let mut action_count = 0_u64;
    for _ in 0..state_count {
        states.push(decode_state(
            reader,
            limits,
            value_nodes,
            &mut action_count,
        )?);
    }
    let region_count = bounded_array(reader, limits.program.max_regions, "Statechart regions")?;
    let mut regions = Vec::with_capacity(capacity(region_count)?);
    for _ in 0..region_count {
        regions.push(decode_region(reader)?);
    }
    let history_count = bounded_array(reader, limits.program.max_regions, "Statechart histories")?;
    let mut histories = Vec::with_capacity(capacity(history_count)?);
    for _ in 0..history_count {
        histories.push(decode_history(reader, limits)?);
    }
    let transition_count = bounded_array(
        reader,
        limits.program.max_transitions,
        "Statechart transitions",
    )?;
    let mut transitions = Vec::with_capacity(capacity(transition_count)?);
    for _ in 0..transition_count {
        transitions.push(decode_transition(
            reader,
            limits,
            value_nodes,
            &mut action_count,
        )?);
    }
    Ok(StatechartV0 {
        root,
        events,
        states,
        regions,
        histories,
        transitions,
        source_span: decode_span(reader)?,
    })
}

fn encode_state(writer: &mut CborWriter, state: &StateV0) {
    writer.array(8);
    writer.bytes(state.id.as_bytes());
    encode_optional_id(writer, state.parent.map(|id| *id.as_bytes()));
    encode_optional_id(writer, state.region.map(|id| *id.as_bytes()));
    writer.unsigned(state.kind as u64);
    encode_actions(writer, &state.on_entry);
    encode_actions(writer, &state.on_exit);
    encode_optional_id(writer, state.completion_event.map(|id| *id.as_bytes()));
    encode_span(writer, state.source_span);
}

fn decode_state(
    reader: &mut CborReader<'_>,
    limits: &ProgramLoadLimits,
    value_nodes: &mut u64,
    action_count: &mut u64,
) -> Result<StateV0, DecodeError> {
    expect_array(reader, 8, "Statechart state")?;
    let id = StateId::from_bytes(reader.bytes_exact::<16>()?);
    let parent = reader
        .optional(|reader| reader.bytes_exact::<16>())?
        .map(StateId::from_bytes);
    let region = reader
        .optional(|reader| reader.bytes_exact::<16>())?
        .map(RegionId::from_bytes);
    let kind =
        StateKindV0::from_u64(reader.unsigned()?).ok_or(DecodeError::Schema("State kind"))?;
    let on_entry = decode_actions(reader, limits, value_nodes, action_count)?;
    let on_exit = decode_actions(reader, limits, value_nodes, action_count)?;
    let completion_event = reader
        .optional(|reader| reader.bytes_exact::<16>())?
        .map(EventTypeId::from_bytes);
    Ok(StateV0 {
        id,
        parent,
        region,
        kind,
        on_entry,
        on_exit,
        completion_event,
        source_span: decode_span(reader)?,
    })
}

fn encode_region(writer: &mut CborWriter, region: &RegionV0) {
    writer.array(4);
    writer.bytes(region.id.as_bytes());
    writer.bytes(region.parent.as_bytes());
    writer.bytes(region.initial.as_bytes());
    encode_span(writer, region.source_span);
}

fn decode_region(reader: &mut CborReader<'_>) -> Result<RegionV0, DecodeError> {
    expect_array(reader, 4, "Statechart region")?;
    Ok(RegionV0 {
        id: RegionId::from_bytes(reader.bytes_exact::<16>()?),
        parent: StateId::from_bytes(reader.bytes_exact::<16>()?),
        initial: StateId::from_bytes(reader.bytes_exact::<16>()?),
        source_span: decode_span(reader)?,
    })
}

fn encode_history(writer: &mut CborWriter, history: &HistoryV0) {
    writer.array(6);
    writer.bytes(history.id.as_bytes());
    writer.bytes(history.parent.as_bytes());
    writer.bytes(history.region.as_bytes());
    writer.unsigned(history.kind as u64);
    writer.array(history.default_targets.len() as u64);
    for target in &history.default_targets {
        writer.bytes(target.as_bytes());
    }
    encode_span(writer, history.source_span);
}

fn decode_history(
    reader: &mut CborReader<'_>,
    limits: &ProgramLoadLimits,
) -> Result<HistoryV0, DecodeError> {
    expect_array(reader, 6, "Statechart history")?;
    let id = HistoryId::from_bytes(reader.bytes_exact::<16>()?);
    let parent = StateId::from_bytes(reader.bytes_exact::<16>()?);
    let region = RegionId::from_bytes(reader.bytes_exact::<16>()?);
    let kind =
        HistoryKindV0::from_u64(reader.unsigned()?).ok_or(DecodeError::Schema("history kind"))?;
    let count = bounded_array(reader, limits.program.max_states, "history default targets")?;
    let mut default_targets = Vec::with_capacity(capacity(count)?);
    for _ in 0..count {
        default_targets.push(StateId::from_bytes(reader.bytes_exact::<16>()?));
    }
    Ok(HistoryV0 {
        id,
        parent,
        region,
        kind,
        default_targets,
        source_span: decode_span(reader)?,
    })
}

fn encode_transition(writer: &mut CborWriter, transition: &TransitionV0) {
    writer.array(8);
    writer.bytes(transition.id.as_bytes());
    writer.bytes(transition.source.as_bytes());
    encode_optional_id(writer, transition.event.map(|id| *id.as_bytes()));
    writer.unsigned(transition.kind as u64);
    encode_guard(writer, &transition.guard);
    writer.array(transition.targets.len() as u64);
    for target in &transition.targets {
        writer.array(2);
        match target {
            TransitionTargetV0::State(id) => {
                writer.unsigned(0);
                writer.bytes(id.as_bytes());
            }
            TransitionTargetV0::History(id) => {
                writer.unsigned(1);
                writer.bytes(id.as_bytes());
            }
        }
    }
    encode_actions(writer, &transition.actions);
    encode_span(writer, transition.source_span);
}

fn decode_transition(
    reader: &mut CborReader<'_>,
    limits: &ProgramLoadLimits,
    value_nodes: &mut u64,
    action_count: &mut u64,
) -> Result<TransitionV0, DecodeError> {
    expect_array(reader, 8, "Statechart transition")?;
    let id = TransitionId::from_bytes(reader.bytes_exact::<16>()?);
    let source = StateId::from_bytes(reader.bytes_exact::<16>()?);
    let event = reader
        .optional(|reader| reader.bytes_exact::<16>())?
        .map(EventTypeId::from_bytes);
    let kind = TransitionKindV0::from_u64(reader.unsigned()?)
        .ok_or(DecodeError::Schema("transition kind"))?;
    let guard = decode_guard(reader, limits, value_nodes, 1)?;
    let target_count = bounded_array(reader, limits.program.max_regions, "transition targets")?;
    let mut targets = Vec::with_capacity(capacity(target_count)?);
    for _ in 0..target_count {
        expect_array(reader, 2, "transition target")?;
        let tag = reader.unsigned()?;
        let raw = reader.bytes_exact::<16>()?;
        targets.push(match tag {
            0 => TransitionTargetV0::State(StateId::from_bytes(raw)),
            1 => TransitionTargetV0::History(HistoryId::from_bytes(raw)),
            _ => return Err(DecodeError::Schema("transition target tag")),
        });
    }
    Ok(TransitionV0 {
        id,
        source,
        event,
        kind,
        guard,
        targets,
        actions: decode_actions(reader, limits, value_nodes, action_count)?,
        source_span: decode_span(reader)?,
    })
}

fn encode_actions(writer: &mut CborWriter, actions: &[ActionRecordV0]) {
    writer.array(actions.len() as u64);
    for action in actions {
        writer.array(3);
        writer.bytes(action.id.as_bytes());
        encode_action(writer, &action.action);
        encode_span(writer, action.source_span);
    }
}

fn decode_actions(
    reader: &mut CborReader<'_>,
    limits: &ProgramLoadLimits,
    value_nodes: &mut u64,
    action_count: &mut u64,
) -> Result<Vec<ActionRecordV0>, DecodeError> {
    let count = reader.array_len()?;
    *action_count = action_count.saturating_add(count);
    if *action_count > limits.program.max_statechart_actions {
        return Err(DecodeError::Limit("Statechart actions"));
    }
    let mut actions = Vec::with_capacity(capacity(count)?);
    for _ in 0..count {
        expect_array(reader, 3, "Statechart action record")?;
        actions.push(ActionRecordV0 {
            id: ActionId::from_bytes(reader.bytes_exact::<16>()?),
            action: decode_action(reader, limits, value_nodes)?,
            source_span: decode_span(reader)?,
        });
    }
    Ok(actions)
}

fn encode_action(writer: &mut CborWriter, action: &StatechartActionV0) {
    match action {
        StatechartActionV0::Assign { global, value } => {
            writer.array(3);
            writer.unsigned(0);
            writer.bytes(global.as_bytes());
            crate::value::encode_value(writer, value);
        }
        StatechartActionV0::Raise { event } => {
            writer.array(2);
            writer.unsigned(1);
            writer.bytes(event.as_bytes());
        }
        StatechartActionV0::StartFlow {
            flow,
            result_to,
            done_event,
        } => {
            writer.array(4);
            writer.unsigned(2);
            writer.bytes(flow.as_bytes());
            encode_optional_id(writer, result_to.map(|id| *id.as_bytes()));
            encode_optional_id(writer, done_event.map(|id| *id.as_bytes()));
        }
        StatechartActionV0::EmitEffect {
            capability,
            payload,
            response_to,
            response_event,
        } => {
            writer.array(5);
            writer.unsigned(3);
            writer.text(capability.as_str());
            crate::value::encode_value(writer, payload);
            encode_optional_id(writer, response_to.map(|id| *id.as_bytes()));
            encode_optional_id(writer, response_event.map(|id| *id.as_bytes()));
        }
    }
}

fn decode_action(
    reader: &mut CborReader<'_>,
    limits: &ProgramLoadLimits,
    value_nodes: &mut u64,
) -> Result<StatechartActionV0, DecodeError> {
    let length = reader.array_len()?;
    match (reader.unsigned()?, length) {
        (0, 3) => Ok(StatechartActionV0::Assign {
            global: GlobalId::from_bytes(reader.bytes_exact::<16>()?),
            value: crate::value::decode_value(reader, &limits.decode, 1, value_nodes)?,
        }),
        (1, 2) => Ok(StatechartActionV0::Raise {
            event: EventTypeId::from_bytes(reader.bytes_exact::<16>()?),
        }),
        (2, 4) => Ok(StatechartActionV0::StartFlow {
            flow: FlowId::from_bytes(reader.bytes_exact::<16>()?),
            result_to: reader
                .optional(|reader| reader.bytes_exact::<16>())?
                .map(GlobalId::from_bytes),
            done_event: reader
                .optional(|reader| reader.bytes_exact::<16>())?
                .map(EventTypeId::from_bytes),
        }),
        (3, 5) => Ok(StatechartActionV0::EmitEffect {
            capability: CapabilityId::new(reader.text(limits.decode.max_string_bytes)?)
                .map_err(|_| DecodeError::Schema("Statechart Effect capability"))?,
            payload: crate::value::decode_value(reader, &limits.decode, 1, value_nodes)?,
            response_to: reader
                .optional(|reader| reader.bytes_exact::<16>())?
                .map(GlobalId::from_bytes),
            response_event: reader
                .optional(|reader| reader.bytes_exact::<16>())?
                .map(EventTypeId::from_bytes),
        }),
        _ => Err(DecodeError::Schema("Statechart action")),
    }
}

fn encode_guard(writer: &mut CborWriter, guard: &GuardExprV0) {
    match guard {
        GuardExprV0::Always => {
            writer.array(1);
            writer.unsigned(0);
        }
        GuardExprV0::Bool { global, expected } => {
            writer.array(3);
            writer.unsigned(1);
            writer.bytes(global.as_bytes());
            writer.boolean(*expected);
        }
        GuardExprV0::Equals { global, value } => {
            writer.array(3);
            writer.unsigned(2);
            writer.bytes(global.as_bytes());
            crate::value::encode_value(writer, value);
        }
        GuardExprV0::Not(inner) => {
            writer.array(2);
            writer.unsigned(3);
            encode_guard(writer, inner);
        }
        GuardExprV0::All(items) | GuardExprV0::Any(items) => {
            writer.array(2);
            writer.unsigned(if matches!(guard, GuardExprV0::All(_)) {
                4
            } else {
                5
            });
            writer.array(items.len() as u64);
            for item in items {
                encode_guard(writer, item);
            }
        }
    }
}

fn decode_guard(
    reader: &mut CborReader<'_>,
    limits: &ProgramLoadLimits,
    value_nodes: &mut u64,
    depth: u32,
) -> Result<GuardExprV0, DecodeError> {
    if depth > limits.program.max_guard_depth {
        return Err(DecodeError::Limit("Statechart guard depth"));
    }
    let length = reader.array_len()?;
    match (reader.unsigned()?, length) {
        (0, 1) => Ok(GuardExprV0::Always),
        (1, 3) => Ok(GuardExprV0::Bool {
            global: GlobalId::from_bytes(reader.bytes_exact::<16>()?),
            expected: reader.boolean()?,
        }),
        (2, 3) => Ok(GuardExprV0::Equals {
            global: GlobalId::from_bytes(reader.bytes_exact::<16>()?),
            value: crate::value::decode_value(
                reader,
                &limits.decode,
                depth.saturating_add(1),
                value_nodes,
            )?,
        }),
        (3, 2) => Ok(GuardExprV0::Not(Box::new(decode_guard(
            reader,
            limits,
            value_nodes,
            depth.saturating_add(1),
        )?))),
        (tag @ (4 | 5), 2) => {
            let count =
                bounded_array(reader, limits.decode.max_collection_items, "guard operands")?;
            let mut items = Vec::with_capacity(capacity(count)?);
            for _ in 0..count {
                items.push(decode_guard(
                    reader,
                    limits,
                    value_nodes,
                    depth.saturating_add(1),
                )?);
            }
            Ok(if tag == 4 {
                GuardExprV0::All(items)
            } else {
                GuardExprV0::Any(items)
            })
        }
        _ => Err(DecodeError::Schema("Statechart guard")),
    }
}

fn encode_span(writer: &mut CborWriter, span: Option<SourceSpan>) {
    match span {
        Some(span) => {
            writer.array(2);
            writer.unsigned(u64::from(span.start));
            writer.unsigned(u64::from(span.end));
        }
        None => writer.null(),
    }
}

fn decode_span(reader: &mut CborReader<'_>) -> Result<Option<SourceSpan>, DecodeError> {
    reader.optional(|reader| {
        expect_array(reader, 2, "source span")?;
        let start = u32::try_from(reader.unsigned()?).map_err(|_| DecodeError::IntegerOverflow)?;
        let end = u32::try_from(reader.unsigned()?).map_err(|_| DecodeError::IntegerOverflow)?;
        Ok(SourceSpan { start, end })
    })
}

fn encode_optional_id(writer: &mut CborWriter, value: Option<[u8; 16]>) {
    match value {
        Some(value) => writer.bytes(&value),
        None => writer.null(),
    }
}

fn bounded_array(
    reader: &mut CborReader<'_>,
    maximum: u64,
    label: &'static str,
) -> Result<u64, DecodeError> {
    let count = reader.array_len()?;
    if count > maximum {
        Err(DecodeError::Limit(label))
    } else {
        Ok(count)
    }
}

fn capacity(count: u64) -> Result<usize, DecodeError> {
    usize::try_from(count).map_err(|_| DecodeError::LengthOverflow)
}

fn expect_array(
    reader: &mut CborReader<'_>,
    expected: u64,
    label: &'static str,
) -> Result<(), DecodeError> {
    if reader.array_len()? == expected {
        Ok(())
    } else {
        Err(DecodeError::Schema(label))
    }
}
