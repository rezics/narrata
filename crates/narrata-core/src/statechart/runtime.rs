use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::{
    ActionId, CapabilityId, EventTypeId, FlowId, GlobalId, HistoryId, StateId, TransitionId, Value,
    effect::EffectResponseV0,
    limits::MacrostepLimits,
    program::CheckedProgram,
    runtime::{PendingEffectV0, RuntimeFault, RuntimeStateV0},
};

use super::{
    ActionRecordV0, GuardExprV0, HistoryKindV0, StateKindV0, StatechartActionV0, TransitionKindV0,
    TransitionTargetV0, validate::is_descendant,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeferredStatechartWorkV0 {
    Flow {
        site: ActionId,
        owner: Option<StateId>,
        flow: FlowId,
        result_to: Option<GlobalId>,
        done_event: Option<EventTypeId>,
    },
    Effect {
        site: ActionId,
        owner: Option<StateId>,
        capability: CapabilityId,
        payload: Value,
        response_to: Option<GlobalId>,
        response_event: Option<EventTypeId>,
    },
}

impl DeferredStatechartWorkV0 {
    pub(crate) fn owner(&self) -> Option<StateId> {
        match self {
            Self::Flow { owner, .. } | Self::Effect { owner, .. } => *owner,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvokedFlowV0 {
    pub site: ActionId,
    pub flow: FlowId,
    pub result_to: Option<GlobalId>,
    pub done_event: Option<EventTypeId>,
    pub deferred_events: VecDeque<EventTypeId>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StatechartStateV0 {
    pub active: BTreeSet<StateId>,
    pub history: BTreeMap<HistoryId, Vec<StateId>>,
    pub completed: BTreeSet<StateId>,
    pub internal_queue: VecDeque<EventTypeId>,
    pub deferred_events: VecDeque<EventTypeId>,
    pub deferred_work: VecDeque<DeferredStatechartWorkV0>,
    pub invocation: Option<InvokedFlowV0>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatechartView {
    pub active: Vec<StateId>,
}

impl From<&StatechartStateV0> for StatechartView {
    fn from(state: &StatechartStateV0) -> Self {
        Self {
            active: state.active.iter().copied().collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatechartTraceKindV0 {
    Event,
    Exit,
    History,
    Transition,
    Action,
    Entry,
    Invoke,
    Await,
    Resume,
    Cancel,
    Stable,
    Finished,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatechartTraceEventV0 {
    pub microstep: u64,
    pub kind: StatechartTraceKindV0,
    pub event: Option<EventTypeId>,
    pub state: Option<StateId>,
    pub transition: Option<TransitionId>,
    pub action: Option<ActionId>,
}

#[derive(Clone, Debug)]
pub(crate) struct ChartRunV0 {
    initialized: bool,
    current_event: Option<EventTypeId>,
    pub microstep_count: u64,
    pub internal_event_count: u64,
}

#[derive(Clone, Debug)]
pub(crate) enum StatechartStepOutcome {
    Continue,
    StartFlow {
        site: ActionId,
        flow: FlowId,
        result_to: Option<GlobalId>,
        done_event: Option<EventTypeId>,
    },
    AwaitEffect {
        site: ActionId,
        capability: CapabilityId,
        payload: Value,
        response_to: Option<GlobalId>,
        response_event: Option<EventTypeId>,
    },
    Stable,
    Finished,
}

pub(crate) fn begin_chart(state: &RuntimeStateV0, event: Option<EventTypeId>) -> ChartRunV0 {
    let initialized = state
        .statechart
        .as_ref()
        .is_some_and(|chart| !chart.active.is_empty());
    ChartRunV0 {
        initialized,
        current_event: event,
        microstep_count: 0,
        internal_event_count: 0,
    }
}

pub(crate) fn begin_chart_effect_response(
    state: &mut RuntimeStateV0,
    pending: &PendingEffectV0,
    response: &EffectResponseV0,
) -> Result<ChartRunV0, RuntimeFault> {
    let (response_to, response_event) = pending
        .statechart_continuation()
        .ok_or(RuntimeFault::InvalidState)?;
    if let Some(global) = response_to {
        let target = state
            .globals
            .get_mut(&global)
            .ok_or(RuntimeFault::InvalidState)?;
        if target.kind() != response.payload.kind() {
            return Err(RuntimeFault::KindMismatch);
        }
        *target = response.payload.clone();
    }
    let chart = state
        .statechart
        .as_mut()
        .ok_or(RuntimeFault::InvalidState)?;
    if let Some(event) = response_event {
        chart.deferred_events.push_back(event);
    }
    Ok(begin_chart(state, None))
}

pub(crate) fn queue_invoked_flow_event(
    state: &mut RuntimeStateV0,
    event: EventTypeId,
) -> Result<(), RuntimeFault> {
    state
        .statechart
        .as_mut()
        .and_then(|chart| chart.invocation.as_mut())
        .ok_or(RuntimeFault::InvalidState)?
        .deferred_events
        .push_back(event);
    Ok(())
}

pub(crate) fn finish_invoked_flow(
    state: &mut RuntimeStateV0,
    result: Option<Value>,
) -> Result<ChartRunV0, RuntimeFault> {
    let chart = state
        .statechart
        .as_mut()
        .ok_or(RuntimeFault::InvalidState)?;
    let invocation = chart.invocation.take().ok_or(RuntimeFault::InvalidState)?;
    match (invocation.result_to, result) {
        (Some(global), Some(value)) => {
            let target = state
                .globals
                .get_mut(&global)
                .ok_or(RuntimeFault::InvalidState)?;
            if target.kind() != value.kind() {
                return Err(RuntimeFault::KindMismatch);
            }
            *target = value;
        }
        (None, None) => {}
        _ => return Err(RuntimeFault::InvalidState),
    }
    chart.deferred_events.extend(invocation.deferred_events);
    if let Some(event) = invocation.done_event {
        chart.deferred_events.push_back(event);
    }
    Ok(begin_chart(state, None))
}

pub(crate) fn execute_chart_step(
    program: &CheckedProgram,
    state: &mut RuntimeStateV0,
    run: &mut ChartRunV0,
    limits: &MacrostepLimits,
    trace: &mut Vec<StatechartTraceEventV0>,
) -> Result<StatechartStepOutcome, RuntimeFault> {
    if run.microstep_count >= limits.max_microsteps {
        return Err(RuntimeFault::MicrostepLimit);
    }
    if !run.initialized {
        run.microstep_count = run.microstep_count.saturating_add(1);
        let root = program
            .statechart()
            .map(|chart| chart.root)
            .ok_or(RuntimeFault::InvalidState)?;
        enter_targets(
            program,
            state,
            &[root],
            &BTreeSet::new(),
            None,
            run.microstep_count,
            trace,
        )?;
        update_completion(program, state, run.microstep_count, trace)?;
        run.initialized = true;
        return Ok(StatechartStepOutcome::Continue);
    }

    if let Some(event) = run.current_event.take() {
        push_trace(
            trace,
            run.microstep_count,
            StatechartTraceKindV0::Event,
            Some(event),
            None,
            None,
            None,
        );
        let selected = select_transitions(program, state, Some(event))?;
        if !selected.is_empty() {
            run.microstep_count = run.microstep_count.saturating_add(1);
            perform_microstep(
                program,
                state,
                &selected,
                Some(event),
                run.microstep_count,
                trace,
            )?;
        }
        return Ok(StatechartStepOutcome::Continue);
    }

    let eventless = select_transitions(program, state, None)?;
    if !eventless.is_empty() {
        run.microstep_count = run.microstep_count.saturating_add(1);
        perform_microstep(program, state, &eventless, None, run.microstep_count, trace)?;
        return Ok(StatechartStepOutcome::Continue);
    }

    let internal = state
        .statechart
        .as_mut()
        .and_then(|chart| chart.internal_queue.pop_front());
    if let Some(event) = internal {
        if run.internal_event_count >= limits.max_internal_events {
            return Err(RuntimeFault::InternalEventLimit);
        }
        run.internal_event_count = run.internal_event_count.saturating_add(1);
        run.current_event = Some(event);
        return Ok(StatechartStepOutcome::Continue);
    }

    loop {
        let work = state
            .statechart
            .as_mut()
            .and_then(|chart| chart.deferred_work.pop_front());
        let Some(work) = work else {
            break;
        };
        if work
            .owner()
            .is_some_and(|owner| !is_active(program, state, owner))
        {
            push_trace(
                trace,
                run.microstep_count,
                StatechartTraceKindV0::Cancel,
                None,
                work.owner(),
                None,
                Some(work_site(&work)),
            );
            continue;
        }
        return Ok(match work {
            DeferredStatechartWorkV0::Flow {
                site,
                flow,
                result_to,
                done_event,
                ..
            } => StatechartStepOutcome::StartFlow {
                site,
                flow,
                result_to,
                done_event,
            },
            DeferredStatechartWorkV0::Effect {
                site,
                capability,
                payload,
                response_to,
                response_event,
                ..
            } => StatechartStepOutcome::AwaitEffect {
                site,
                capability,
                payload,
                response_to,
                response_event,
            },
        });
    }

    let deferred_event = state
        .statechart
        .as_mut()
        .and_then(|chart| chart.deferred_events.pop_front());
    if let Some(event) = deferred_event {
        state
            .statechart
            .as_mut()
            .ok_or(RuntimeFault::InvalidState)?
            .internal_queue
            .push_back(event);
        return Ok(StatechartStepOutcome::Continue);
    }

    if chart_finished(program, state)? {
        push_trace(
            trace,
            run.microstep_count,
            StatechartTraceKindV0::Finished,
            None,
            program.statechart().map(|chart| chart.root),
            None,
            None,
        );
        Ok(StatechartStepOutcome::Finished)
    } else {
        push_trace(
            trace,
            run.microstep_count,
            StatechartTraceKindV0::Stable,
            None,
            None,
            None,
            None,
        );
        Ok(StatechartStepOutcome::Stable)
    }
}

fn select_transitions(
    program: &CheckedProgram,
    state: &RuntimeStateV0,
    event: Option<EventTypeId>,
) -> Result<Vec<usize>, RuntimeFault> {
    let indices = program.statechart_indices();
    let chart = program.statechart().ok_or(RuntimeFault::InvalidState)?;
    let active = state
        .statechart
        .as_ref()
        .ok_or(RuntimeFault::InvalidState)?;
    let mut candidates = BTreeSet::new();
    for leaf in &active.active {
        let mut current = Some(*leaf);
        while let Some(source) = current {
            let mut matched = None;
            if let Some(transitions) = indices.transitions_by_source.get(&source) {
                for index in transitions {
                    let transition = chart
                        .transitions
                        .get(*index)
                        .ok_or(RuntimeFault::InvalidState)?;
                    if transition.event == event && evaluate_guard(state, &transition.guard)? {
                        matched = Some(*index);
                        break;
                    }
                }
            }
            if let Some(index) = matched {
                candidates.insert(index);
                break;
            }
            current = program.state(source).and_then(|node| node.parent);
        }
    }
    let mut ordered = candidates.into_iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        let left_depth = chart
            .transitions
            .get(*left)
            .and_then(|transition| indices.depths.get(&transition.source))
            .copied()
            .unwrap_or_default();
        let right_depth = chart
            .transitions
            .get(*right)
            .and_then(|transition| indices.depths.get(&transition.source))
            .copied()
            .unwrap_or_default();
        right_depth.cmp(&left_depth).then_with(|| left.cmp(right))
    });
    let mut selected = Vec::new();
    let mut occupied = BTreeSet::new();
    for candidate in ordered {
        let exits = transition_exit_states(program, state, candidate)?;
        if exits.iter().all(|state| !occupied.contains(state)) {
            occupied.extend(exits);
            selected.push(candidate);
        }
    }
    selected.sort_unstable();
    check_parallel_writes(program, &selected)?;
    Ok(selected)
}

fn perform_microstep(
    program: &CheckedProgram,
    state: &mut RuntimeStateV0,
    selected: &[usize],
    event: Option<EventTypeId>,
    microstep: u64,
    trace: &mut Vec<StatechartTraceEventV0>,
) -> Result<(), RuntimeFault> {
    let chart = program.statechart().ok_or(RuntimeFault::InvalidState)?;
    let mut exits = BTreeSet::new();
    for transition in selected {
        exits.extend(transition_exit_states(program, state, *transition)?);
    }
    let mut exit_order = exits.iter().copied().collect::<Vec<_>>();
    exit_order.sort_by(|left, right| {
        depth(program, *right)
            .cmp(&depth(program, *left))
            .then_with(|| left.cmp(right))
    });
    record_history(program, state, &exit_order, microstep, trace)?;
    let already_entered = all_active_states(program, state)?
        .difference(&exits)
        .copied()
        .collect::<BTreeSet<_>>();
    for id in &exit_order {
        let node = program
            .state(*id)
            .ok_or(RuntimeFault::InvalidState)?
            .clone();
        push_trace(
            trace,
            microstep,
            StatechartTraceKindV0::Exit,
            event,
            Some(*id),
            None,
            None,
        );
        execute_actions(program, state, &node.on_exit, None, None, microstep, trace)?;
    }
    state
        .statechart
        .as_mut()
        .ok_or(RuntimeFault::InvalidState)?
        .active
        .retain(|leaf| {
            !exits
                .iter()
                .any(|exited| is_descendant(program, program.statechart_indices(), *leaf, *exited))
        });

    let mut target_batches = Vec::new();
    for index in selected {
        let transition = chart
            .transitions
            .get(*index)
            .ok_or(RuntimeFault::InvalidState)?
            .clone();
        push_trace(
            trace,
            microstep,
            StatechartTraceKindV0::Transition,
            event,
            Some(transition.source),
            Some(transition.id),
            None,
        );
        execute_actions(
            program,
            state,
            &transition.actions,
            None,
            Some(transition.id),
            microstep,
            trace,
        )?;
        let mut targets = resolve_targets(program, state, &transition.targets)?;
        targets.sort_unstable();
        targets.dedup();
        target_batches.push((transition.id, targets));
    }
    for (transition, targets) in target_batches {
        enter_targets(
            program,
            state,
            &targets,
            &already_entered,
            Some(transition),
            microstep,
            trace,
        )?;
    }
    update_completion(program, state, microstep, trace)
}

fn execute_actions(
    _program: &CheckedProgram,
    state: &mut RuntimeStateV0,
    actions: &[ActionRecordV0],
    owner: Option<StateId>,
    transition: Option<TransitionId>,
    microstep: u64,
    trace: &mut Vec<StatechartTraceEventV0>,
) -> Result<(), RuntimeFault> {
    for record in actions {
        push_trace(
            trace,
            microstep,
            StatechartTraceKindV0::Action,
            None,
            owner,
            transition,
            Some(record.id),
        );
        match &record.action {
            StatechartActionV0::Assign { global, value } => {
                let target = state
                    .globals
                    .get_mut(global)
                    .ok_or(RuntimeFault::InvalidState)?;
                if target.kind() != value.kind() {
                    return Err(RuntimeFault::KindMismatch);
                }
                *target = value.clone();
            }
            StatechartActionV0::Raise { event } => {
                push_internal(state, *event)?;
            }
            StatechartActionV0::StartFlow {
                flow,
                result_to,
                done_event,
            } => state
                .statechart
                .as_mut()
                .ok_or(RuntimeFault::InvalidState)?
                .deferred_work
                .push_back(DeferredStatechartWorkV0::Flow {
                    site: record.id,
                    owner,
                    flow: *flow,
                    result_to: *result_to,
                    done_event: *done_event,
                }),
            StatechartActionV0::EmitEffect {
                capability,
                payload,
                response_to,
                response_event,
            } => state
                .statechart
                .as_mut()
                .ok_or(RuntimeFault::InvalidState)?
                .deferred_work
                .push_back(DeferredStatechartWorkV0::Effect {
                    site: record.id,
                    owner,
                    capability: capability.clone(),
                    payload: payload.clone(),
                    response_to: *response_to,
                    response_event: *response_event,
                }),
        }
    }
    Ok(())
}

fn enter_targets(
    program: &CheckedProgram,
    state: &mut RuntimeStateV0,
    targets: &[StateId],
    already_entered: &BTreeSet<StateId>,
    transition: Option<TransitionId>,
    microstep: u64,
    trace: &mut Vec<StatechartTraceEventV0>,
) -> Result<(), RuntimeFault> {
    let mut roots = BTreeMap::<StateId, Vec<StateId>>::new();
    for target in targets {
        let path = path_from_root(program, *target)?;
        let root = path
            .iter()
            .copied()
            .find(|id| !already_entered.contains(id) && !is_active(program, state, *id));
        if let Some(root) = root {
            roots.entry(root).or_default().push(*target);
        }
    }
    for (root, explicit) in roots {
        enter_config(
            program,
            state,
            root,
            &explicit,
            already_entered,
            transition,
            microstep,
            trace,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn enter_config(
    program: &CheckedProgram,
    state: &mut RuntimeStateV0,
    id: StateId,
    explicit: &[StateId],
    already_entered: &BTreeSet<StateId>,
    transition: Option<TransitionId>,
    microstep: u64,
    trace: &mut Vec<StatechartTraceEventV0>,
) -> Result<(), RuntimeFault> {
    let node = program.state(id).ok_or(RuntimeFault::InvalidState)?.clone();
    let was_active = already_entered.contains(&id) || is_active(program, state, id);
    if !was_active {
        push_trace(
            trace,
            microstep,
            StatechartTraceKindV0::Entry,
            None,
            Some(id),
            transition,
            None,
        );
        execute_actions(
            program,
            state,
            &node.on_entry,
            Some(id),
            transition,
            microstep,
            trace,
        )?;
    }
    match node.kind {
        StateKindV0::Atomic | StateKindV0::Final => {
            state
                .statechart
                .as_mut()
                .ok_or(RuntimeFault::InvalidState)?
                .active
                .insert(id);
        }
        StateKindV0::Compound | StateKindV0::Parallel => {
            let regions = program
                .statechart_indices()
                .regions_by_parent
                .get(&id)
                .cloned()
                .ok_or(RuntimeFault::InvalidState)?;
            for region_id in regions {
                let region = program
                    .region(region_id)
                    .ok_or(RuntimeFault::InvalidState)?;
                let mut region_targets = explicit
                    .iter()
                    .copied()
                    .filter(|target| in_region(program, *target, region_id))
                    .collect::<Vec<_>>();
                region_targets.sort_unstable();
                let child = region_targets
                    .first()
                    .and_then(|target| direct_child(program, id, *target))
                    .unwrap_or(region.initial);
                if !was_active || !region_targets.is_empty() {
                    enter_config(
                        program,
                        state,
                        child,
                        &region_targets,
                        already_entered,
                        transition,
                        microstep,
                        trace,
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn record_history(
    program: &CheckedProgram,
    state: &mut RuntimeStateV0,
    exits: &[StateId],
    microstep: u64,
    trace: &mut Vec<StatechartTraceEventV0>,
) -> Result<(), RuntimeFault> {
    let chart = program.statechart().ok_or(RuntimeFault::InvalidState)?;
    let active = state
        .statechart
        .as_ref()
        .ok_or(RuntimeFault::InvalidState)?
        .active
        .clone();
    for exited in exits {
        for history in chart.histories.iter().filter(|item| item.parent == *exited) {
            let mut saved = BTreeSet::new();
            for leaf in &active {
                if !in_region(program, *leaf, history.region) {
                    continue;
                }
                match history.kind {
                    HistoryKindV0::Shallow => {
                        if let Some(child) = direct_child(program, history.parent, *leaf) {
                            saved.insert(child);
                        }
                    }
                    HistoryKindV0::Deep => {
                        saved.insert(*leaf);
                    }
                }
            }
            state
                .statechart
                .as_mut()
                .ok_or(RuntimeFault::InvalidState)?
                .history
                .insert(history.id, saved.into_iter().collect());
            push_trace(
                trace,
                microstep,
                StatechartTraceKindV0::History,
                None,
                Some(*exited),
                None,
                None,
            );
        }
    }
    Ok(())
}

fn update_completion(
    program: &CheckedProgram,
    state: &mut RuntimeStateV0,
    microstep: u64,
    trace: &mut Vec<StatechartTraceEventV0>,
) -> Result<(), RuntimeFault> {
    let chart = program.statechart().ok_or(RuntimeFault::InvalidState)?;
    let previous = state
        .statechart
        .as_ref()
        .ok_or(RuntimeFault::InvalidState)?
        .completed
        .clone();
    let mut completed = BTreeSet::new();
    let mut states = chart.states.iter().collect::<Vec<_>>();
    states.sort_by(|left, right| {
        depth(program, right.id)
            .cmp(&depth(program, left.id))
            .then_with(|| left.id.cmp(&right.id))
    });
    for node in states {
        if state_complete(program, state, node.id)? {
            completed.insert(node.id);
            if !previous.contains(&node.id)
                && let Some(event) = node.completion_event
            {
                push_internal(state, event)?;
                push_trace(
                    trace,
                    microstep,
                    StatechartTraceKindV0::Event,
                    Some(event),
                    Some(node.id),
                    None,
                    None,
                );
            }
        }
    }
    state
        .statechart
        .as_mut()
        .ok_or(RuntimeFault::InvalidState)?
        .completed = completed;
    Ok(())
}

fn state_complete(
    program: &CheckedProgram,
    state: &RuntimeStateV0,
    id: StateId,
) -> Result<bool, RuntimeFault> {
    let node = program.state(id).ok_or(RuntimeFault::InvalidState)?;
    match node.kind {
        StateKindV0::Final => Ok(is_active(program, state, id)),
        StateKindV0::Atomic => Ok(false),
        StateKindV0::Compound | StateKindV0::Parallel => {
            let regions = program
                .statechart_indices()
                .regions_by_parent
                .get(&id)
                .ok_or(RuntimeFault::InvalidState)?;
            Ok(regions.iter().all(|region_id| {
                program.statechart().is_some_and(|chart| {
                    chart.states.iter().any(|child| {
                        child.parent == Some(id)
                            && child.region == Some(*region_id)
                            && child.kind == StateKindV0::Final
                            && is_active(program, state, child.id)
                    })
                })
            }))
        }
    }
}

fn chart_finished(program: &CheckedProgram, state: &RuntimeStateV0) -> Result<bool, RuntimeFault> {
    let root = program
        .statechart()
        .map(|chart| chart.root)
        .ok_or(RuntimeFault::InvalidState)?;
    let root_state = program.state(root).ok_or(RuntimeFault::InvalidState)?;
    if root_state.kind == StateKindV0::Final {
        Ok(is_active(program, state, root))
    } else {
        Ok(state
            .statechart
            .as_ref()
            .ok_or(RuntimeFault::InvalidState)?
            .completed
            .contains(&root))
    }
}

fn transition_exit_states(
    program: &CheckedProgram,
    state: &RuntimeStateV0,
    index: usize,
) -> Result<BTreeSet<StateId>, RuntimeFault> {
    let transition = program
        .statechart()
        .and_then(|chart| chart.transitions.get(index))
        .ok_or(RuntimeFault::InvalidState)?;
    if transition.targets.is_empty() {
        return Ok(BTreeSet::new());
    }
    let targets = resolve_targets(program, state, &transition.targets)?;
    if targets.is_empty() {
        return Ok(BTreeSet::new());
    }
    let (domain, include_domain) =
        transition_domain(program, transition.source, &targets, transition.kind)?;
    let active_states = all_active_states(program, state)?;
    let source_region = program
        .state(transition.source)
        .and_then(|node| node.region);
    let restrict_region = source_region.filter(|region| {
        targets
            .iter()
            .all(|target| in_region(program, *target, *region))
    });
    Ok(active_states
        .into_iter()
        .filter(|active| {
            is_descendant(program, program.statechart_indices(), *active, domain)
                && (include_domain || *active != domain)
                && restrict_region.is_none_or(|region| in_region(program, *active, region))
        })
        .collect())
}

fn transition_domain(
    program: &CheckedProgram,
    source: StateId,
    targets: &[StateId],
    kind: TransitionKindV0,
) -> Result<(StateId, bool), RuntimeFault> {
    if kind == TransitionKindV0::Internal {
        return Ok((source, false));
    }
    if targets
        .iter()
        .all(|target| is_descendant(program, program.statechart_indices(), *target, source))
    {
        return Ok((source, true));
    }
    if let Some(ancestor) = targets
        .iter()
        .copied()
        .find(|target| is_descendant(program, program.statechart_indices(), source, *target))
    {
        return Ok((ancestor, true));
    }
    Ok((least_common_ancestor(program, source, targets)?, false))
}

fn least_common_ancestor(
    program: &CheckedProgram,
    source: StateId,
    targets: &[StateId],
) -> Result<StateId, RuntimeFault> {
    let source_path = path_from_root(program, source)?;
    let paths = targets
        .iter()
        .map(|target| path_from_root(program, *target))
        .collect::<Result<Vec<_>, _>>()?;
    source_path
        .into_iter()
        .rev()
        .find(|candidate| paths.iter().all(|path| path.contains(candidate)))
        .ok_or(RuntimeFault::InvalidState)
}

fn resolve_targets(
    program: &CheckedProgram,
    state: &RuntimeStateV0,
    targets: &[TransitionTargetV0],
) -> Result<Vec<StateId>, RuntimeFault> {
    let chart_state = state
        .statechart
        .as_ref()
        .ok_or(RuntimeFault::InvalidState)?;
    let mut resolved = Vec::new();
    for target in targets {
        match target {
            TransitionTargetV0::State(id) => resolved.push(*id),
            TransitionTargetV0::History(id) => {
                let declaration = program.history(*id).ok_or(RuntimeFault::InvalidState)?;
                let saved = chart_state
                    .history
                    .get(id)
                    .filter(|items| !items.is_empty())
                    .unwrap_or(&declaration.default_targets);
                resolved.extend(saved.iter().copied());
            }
        }
    }
    Ok(resolved)
}

fn evaluate_guard(state: &RuntimeStateV0, guard: &GuardExprV0) -> Result<bool, RuntimeFault> {
    match guard {
        GuardExprV0::Always => Ok(true),
        GuardExprV0::Bool { global, expected } => match state.globals.get(global) {
            Some(Value::Bool(value)) => Ok(value == expected),
            Some(_) => Err(RuntimeFault::KindMismatch),
            None => Err(RuntimeFault::InvalidState),
        },
        GuardExprV0::Equals { global, value } => state
            .globals
            .get(global)
            .map(|current| current == value)
            .ok_or(RuntimeFault::InvalidState),
        GuardExprV0::Not(inner) => evaluate_guard(state, inner).map(|value| !value),
        GuardExprV0::All(items) => {
            for item in items {
                if !evaluate_guard(state, item)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        GuardExprV0::Any(items) => {
            for item in items {
                if evaluate_guard(state, item)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
    }
}

fn check_parallel_writes(program: &CheckedProgram, selected: &[usize]) -> Result<(), RuntimeFault> {
    let chart = program.statechart().ok_or(RuntimeFault::InvalidState)?;
    let mut owners = BTreeMap::<GlobalId, TransitionId>::new();
    for index in selected {
        let transition = chart
            .transitions
            .get(*index)
            .ok_or(RuntimeFault::InvalidState)?;
        for action in &transition.actions {
            if let StatechartActionV0::Assign { global, .. } = action.action
                && owners
                    .insert(global, transition.id)
                    .is_some_and(|owner| owner != transition.id)
            {
                return Err(RuntimeFault::ParallelWriteConflict);
            }
        }
    }
    Ok(())
}

fn all_active_states(
    program: &CheckedProgram,
    state: &RuntimeStateV0,
) -> Result<BTreeSet<StateId>, RuntimeFault> {
    let leaves = &state
        .statechart
        .as_ref()
        .ok_or(RuntimeFault::InvalidState)?
        .active;
    let mut result = BTreeSet::new();
    for leaf in leaves {
        let mut current = Some(*leaf);
        while let Some(id) = current {
            result.insert(id);
            current = program.state(id).and_then(|node| node.parent);
        }
    }
    Ok(result)
}

pub(crate) fn is_active(program: &CheckedProgram, state: &RuntimeStateV0, id: StateId) -> bool {
    state.statechart.as_ref().is_some_and(|chart| {
        chart
            .active
            .iter()
            .any(|leaf| is_descendant(program, program.statechart_indices(), *leaf, id))
    })
}

fn in_region(program: &CheckedProgram, id: StateId, region: crate::RegionId) -> bool {
    let mut current = Some(id);
    while let Some(state_id) = current {
        let Some(node) = program.state(state_id) else {
            return false;
        };
        if node.region == Some(region) {
            return true;
        }
        current = node.parent;
    }
    false
}

fn direct_child(program: &CheckedProgram, parent: StateId, descendant: StateId) -> Option<StateId> {
    let mut current = descendant;
    loop {
        let node = program.state(current)?;
        if node.parent == Some(parent) {
            return Some(current);
        }
        current = node.parent?;
    }
}

fn path_from_root(program: &CheckedProgram, id: StateId) -> Result<Vec<StateId>, RuntimeFault> {
    let mut path = Vec::new();
    let mut current = Some(id);
    let mut seen = BTreeSet::new();
    while let Some(state) = current {
        if !seen.insert(state) {
            return Err(RuntimeFault::InvalidState);
        }
        path.push(state);
        current = program
            .state(state)
            .ok_or(RuntimeFault::InvalidState)?
            .parent;
    }
    path.reverse();
    Ok(path)
}

fn depth(program: &CheckedProgram, id: StateId) -> usize {
    program
        .statechart_indices()
        .depths
        .get(&id)
        .copied()
        .unwrap_or_default()
}

fn push_internal(state: &mut RuntimeStateV0, event: EventTypeId) -> Result<(), RuntimeFault> {
    state
        .statechart
        .as_mut()
        .ok_or(RuntimeFault::InvalidState)?
        .internal_queue
        .push_back(event);
    Ok(())
}

fn push_trace(
    trace: &mut Vec<StatechartTraceEventV0>,
    microstep: u64,
    kind: StatechartTraceKindV0,
    event: Option<EventTypeId>,
    state: Option<StateId>,
    transition: Option<TransitionId>,
    action: Option<ActionId>,
) {
    trace.push(StatechartTraceEventV0 {
        microstep,
        kind,
        event,
        state,
        transition,
        action,
    });
}

fn work_site(work: &DeferredStatechartWorkV0) -> ActionId {
    match work {
        DeferredStatechartWorkV0::Flow { site, .. }
        | DeferredStatechartWorkV0::Effect { site, .. } => *site,
    }
}
