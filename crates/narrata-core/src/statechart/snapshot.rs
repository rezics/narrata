use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::{
    ActionId, CapabilityId, EventTypeId, FlowId, GlobalId, HistoryId, StateId,
    codec::{CborReader, CborWriter, DecodeError},
    limits::SnapshotLoadLimits,
};

use super::{DeferredStatechartWorkV0, InvokedFlowV0, StatechartStateV0};

pub(crate) fn encode_statechart_state(writer: &mut CborWriter, state: &StatechartStateV0) {
    writer.array(7);
    writer.array(state.active.len() as u64);
    for id in &state.active {
        writer.bytes(id.as_bytes());
    }
    writer.map(state.history.len() as u64);
    for (id, saved) in &state.history {
        writer.bytes(id.as_bytes());
        writer.array(saved.len() as u64);
        for state in saved {
            writer.bytes(state.as_bytes());
        }
    }
    writer.array(state.completed.len() as u64);
    for id in &state.completed {
        writer.bytes(id.as_bytes());
    }
    encode_events(writer, &state.internal_queue);
    encode_events(writer, &state.deferred_events);
    writer.array(state.deferred_work.len() as u64);
    for work in &state.deferred_work {
        encode_work(writer, work);
    }
    match &state.invocation {
        Some(invocation) => encode_invocation(writer, invocation),
        None => writer.null(),
    }
}

pub(crate) fn decode_statechart_state(
    reader: &mut CborReader<'_>,
    limits: &SnapshotLoadLimits,
    value_nodes: &mut u64,
) -> Result<StatechartStateV0, DecodeError> {
    expect_array(reader, 7, "Statechart state")?;
    let active_count = bounded_array(
        reader,
        limits.runtime.max_active_states,
        "active Statechart configuration",
    )?;
    let mut active = BTreeSet::new();
    let mut previous = None;
    for _ in 0..active_count {
        let raw = reader.bytes_exact::<16>()?;
        if previous.is_some_and(|previous| previous >= raw) {
            return Err(DecodeError::NonCanonical("active StateId order"));
        }
        previous = Some(raw);
        active.insert(StateId::from_bytes(raw));
    }
    let history_count = reader.map_len()?;
    if history_count > limits.runtime.max_history_entries {
        return Err(DecodeError::Limit("Statechart history entries"));
    }
    let mut history = BTreeMap::new();
    let mut previous_history = None;
    for _ in 0..history_count {
        let raw = reader.bytes_exact::<16>()?;
        if previous_history.is_some_and(|previous| previous >= raw) {
            return Err(DecodeError::NonCanonical("HistoryId order"));
        }
        previous_history = Some(raw);
        let saved_count = bounded_array(
            reader,
            limits.runtime.max_active_states,
            "saved history configuration",
        )?;
        let mut saved = Vec::with_capacity(capacity(saved_count)?);
        let mut previous_saved = None;
        for _ in 0..saved_count {
            let state = reader.bytes_exact::<16>()?;
            if previous_saved.is_some_and(|previous| previous >= state) {
                return Err(DecodeError::NonCanonical("saved history StateId order"));
            }
            previous_saved = Some(state);
            saved.push(StateId::from_bytes(state));
        }
        history.insert(HistoryId::from_bytes(raw), saved);
    }
    let completed_count = bounded_array(
        reader,
        limits.runtime.max_active_states,
        "completed Statechart states",
    )?;
    let mut completed = BTreeSet::new();
    let mut previous_completed = None;
    for _ in 0..completed_count {
        let raw = reader.bytes_exact::<16>()?;
        if previous_completed.is_some_and(|previous| previous >= raw) {
            return Err(DecodeError::NonCanonical("completed StateId order"));
        }
        previous_completed = Some(raw);
        completed.insert(StateId::from_bytes(raw));
    }
    let internal_queue = decode_events(reader, limits)?;
    let deferred_events = decode_events(reader, limits)?;
    let work_count = bounded_array(
        reader,
        limits.runtime.max_deferred_work,
        "deferred Statechart work",
    )?;
    let mut deferred_work = VecDeque::with_capacity(capacity(work_count)?);
    for _ in 0..work_count {
        deferred_work.push_back(decode_work(reader, limits, value_nodes)?);
    }
    let invocation = reader.optional(|reader| decode_invocation(reader, limits))?;
    Ok(StatechartStateV0 {
        active,
        history,
        completed,
        internal_queue,
        deferred_events,
        deferred_work,
        invocation,
    })
}

fn encode_work(writer: &mut CborWriter, work: &DeferredStatechartWorkV0) {
    match work {
        DeferredStatechartWorkV0::Flow {
            site,
            owner,
            flow,
            result_to,
            done_event,
        } => {
            writer.array(6);
            writer.unsigned(0);
            writer.bytes(site.as_bytes());
            encode_optional_id(writer, owner.map(|id| *id.as_bytes()));
            writer.bytes(flow.as_bytes());
            encode_optional_id(writer, result_to.map(|id| *id.as_bytes()));
            encode_optional_id(writer, done_event.map(|id| *id.as_bytes()));
        }
        DeferredStatechartWorkV0::Effect {
            site,
            owner,
            capability,
            payload,
            response_to,
            response_event,
        } => {
            writer.array(7);
            writer.unsigned(1);
            writer.bytes(site.as_bytes());
            encode_optional_id(writer, owner.map(|id| *id.as_bytes()));
            writer.text(capability.as_str());
            crate::value::encode_value(writer, payload);
            encode_optional_id(writer, response_to.map(|id| *id.as_bytes()));
            encode_optional_id(writer, response_event.map(|id| *id.as_bytes()));
        }
    }
}

fn decode_work(
    reader: &mut CborReader<'_>,
    limits: &SnapshotLoadLimits,
    value_nodes: &mut u64,
) -> Result<DeferredStatechartWorkV0, DecodeError> {
    let length = reader.array_len()?;
    let tag = reader.unsigned()?;
    let site = ActionId::from_bytes(reader.bytes_exact::<16>()?);
    let owner = reader
        .optional(|reader| reader.bytes_exact::<16>())?
        .map(StateId::from_bytes);
    match (tag, length) {
        (0, 6) => Ok(DeferredStatechartWorkV0::Flow {
            site,
            owner,
            flow: FlowId::from_bytes(reader.bytes_exact::<16>()?),
            result_to: reader
                .optional(|reader| reader.bytes_exact::<16>())?
                .map(GlobalId::from_bytes),
            done_event: reader
                .optional(|reader| reader.bytes_exact::<16>())?
                .map(EventTypeId::from_bytes),
        }),
        (1, 7) => Ok(DeferredStatechartWorkV0::Effect {
            site,
            owner,
            capability: CapabilityId::new(reader.text(limits.decode.max_string_bytes)?)
                .map_err(|_| DecodeError::Schema("deferred Effect capability"))?,
            payload: crate::value::decode_value(reader, &limits.decode, 1, value_nodes)?,
            response_to: reader
                .optional(|reader| reader.bytes_exact::<16>())?
                .map(GlobalId::from_bytes),
            response_event: reader
                .optional(|reader| reader.bytes_exact::<16>())?
                .map(EventTypeId::from_bytes),
        }),
        _ => Err(DecodeError::Schema("deferred Statechart work")),
    }
}

fn encode_invocation(writer: &mut CborWriter, invocation: &InvokedFlowV0) {
    writer.array(5);
    writer.bytes(invocation.site.as_bytes());
    writer.bytes(invocation.flow.as_bytes());
    encode_optional_id(writer, invocation.result_to.map(|id| *id.as_bytes()));
    encode_optional_id(writer, invocation.done_event.map(|id| *id.as_bytes()));
    encode_events(writer, &invocation.deferred_events);
}

fn decode_invocation(
    reader: &mut CborReader<'_>,
    limits: &SnapshotLoadLimits,
) -> Result<InvokedFlowV0, DecodeError> {
    expect_array(reader, 5, "invoked Flow")?;
    Ok(InvokedFlowV0 {
        site: ActionId::from_bytes(reader.bytes_exact::<16>()?),
        flow: FlowId::from_bytes(reader.bytes_exact::<16>()?),
        result_to: reader
            .optional(|reader| reader.bytes_exact::<16>())?
            .map(GlobalId::from_bytes),
        done_event: reader
            .optional(|reader| reader.bytes_exact::<16>())?
            .map(EventTypeId::from_bytes),
        deferred_events: decode_events(reader, limits)?,
    })
}

fn encode_events(writer: &mut CborWriter, events: &VecDeque<EventTypeId>) {
    writer.array(events.len() as u64);
    for event in events {
        writer.bytes(event.as_bytes());
    }
}

fn decode_events(
    reader: &mut CborReader<'_>,
    limits: &SnapshotLoadLimits,
) -> Result<VecDeque<EventTypeId>, DecodeError> {
    let count = bounded_array(
        reader,
        limits.runtime.max_internal_events,
        "Statechart event queue",
    )?;
    let mut events = VecDeque::with_capacity(capacity(count)?);
    for _ in 0..count {
        events.push_back(EventTypeId::from_bytes(reader.bytes_exact::<16>()?));
    }
    Ok(events)
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
