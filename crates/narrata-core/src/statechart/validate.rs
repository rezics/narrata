use std::collections::{BTreeMap, BTreeSet};

use crate::{
    ActionId, EventTypeId, FlowId, GlobalId, HistoryId, RegionId, StateId, TransitionId,
    diagnostic::{
        Diagnostic, DiagnosticClass, DiagnosticPath,
        codes::{
            CONTROL_FLOW_INVALID, DUPLICATE_ID, KIND_MISMATCH, LIMIT_EXCEEDED, MISSING_REFERENCE,
            STATECHART_CONFLICT, STATECHART_EVENTLESS_CYCLE, STATECHART_STRUCTURE_INVALID,
            STATECHART_UNREACHABLE,
        },
    },
    limits::ProgramLoadLimits,
    program::{CheckedProgram, OpV0, SlotRefV0},
    value::ValueKindV0,
};

use super::{
    ActionRecordV0, GuardExprV0, HistoryKindV0, StateKindV0, StatechartActionV0, TransitionKindV0,
    TransitionTargetV0,
};

#[derive(Clone, Debug, Default)]
pub(crate) struct StatechartIndices {
    pub states: BTreeMap<StateId, usize>,
    pub regions: BTreeMap<RegionId, usize>,
    pub histories: BTreeMap<HistoryId, usize>,
    pub transitions: BTreeMap<TransitionId, usize>,
    pub actions: BTreeSet<ActionId>,
    pub children: BTreeMap<StateId, Vec<StateId>>,
    pub regions_by_parent: BTreeMap<StateId, Vec<RegionId>>,
    pub transitions_by_source: BTreeMap<StateId, Vec<usize>>,
    pub depths: BTreeMap<StateId, usize>,
}

pub(crate) fn validate_statechart(
    program: &CheckedProgram,
    limits: &ProgramLoadLimits,
    diagnostics: &mut Vec<Diagnostic>,
) -> StatechartIndices {
    let Some(chart) = &program.artifact().statechart else {
        return StatechartIndices::default();
    };
    let path = DiagnosticPath::root().field("statechart");
    let mut indices = StatechartIndices::default();
    let action_count = chart
        .states
        .iter()
        .flat_map(|state| state.on_entry.iter().chain(&state.on_exit))
        .count()
        .saturating_add(
            chart
                .transitions
                .iter()
                .map(|transition| transition.actions.len())
                .sum::<usize>(),
        );
    if chart.states.len() as u64 > limits.program.max_states
        || chart.regions.len() as u64 > limits.program.max_regions
        || chart.transitions.len() as u64 > limits.program.max_transitions
        || action_count as u64 > limits.program.max_statechart_actions
    {
        diagnostics.push(Diagnostic::new(
            DiagnosticClass::LimitExceeded,
            LIMIT_EXCEEDED,
            path.clone(),
            "Statechart exceeds configured limits",
        ));
    }
    let mut previous_event = None;
    for (index, event) in chart.events.iter().copied().enumerate() {
        if previous_event.is_some_and(|previous| previous >= event) {
            diagnostics.push(
                Diagnostic::new(
                    DiagnosticClass::Validation,
                    DUPLICATE_ID,
                    path.clone().field("events").index(index),
                    "event type IDs must be unique and sorted",
                )
                .with_id(event),
            );
        }
        previous_event = Some(event);
    }
    for (index, state) in chart.states.iter().enumerate() {
        if indices.states.insert(state.id, index).is_some() {
            diagnostics.push(duplicate(
                path.clone().field("states").index(index),
                state.id,
                state.source_span,
            ));
        }
        if let Some(parent) = state.parent {
            indices.children.entry(parent).or_default().push(state.id);
        }
        validate_actions(
            program,
            &chart.events,
            &state.on_entry,
            path.clone().field("states").index(index).field("on_entry"),
            &mut indices.actions,
            diagnostics,
        );
        validate_actions(
            program,
            &chart.events,
            &state.on_exit,
            path.clone().field("states").index(index).field("on_exit"),
            &mut indices.actions,
            diagnostics,
        );
    }
    for children in indices.children.values_mut() {
        children.sort_unstable();
    }
    for (index, region) in chart.regions.iter().enumerate() {
        if indices.regions.insert(region.id, index).is_some() {
            diagnostics.push(duplicate(
                path.clone().field("regions").index(index),
                region.id,
                region.source_span,
            ));
        }
        indices
            .regions_by_parent
            .entry(region.parent)
            .or_default()
            .push(region.id);
    }
    for regions in indices.regions_by_parent.values_mut() {
        regions.sort_unstable();
    }
    for (index, history) in chart.histories.iter().enumerate() {
        if indices.histories.insert(history.id, index).is_some() {
            diagnostics.push(duplicate(
                path.clone().field("histories").index(index),
                history.id,
                history.source_span,
            ));
        }
    }
    for (index, transition) in chart.transitions.iter().enumerate() {
        if indices.transitions.insert(transition.id, index).is_some() {
            diagnostics.push(duplicate(
                path.clone().field("transitions").index(index),
                transition.id,
                transition.source_span,
            ));
        }
        indices
            .transitions_by_source
            .entry(transition.source)
            .or_default()
            .push(index);
        validate_guard(
            program,
            &transition.guard,
            1,
            limits.program.max_guard_depth,
            path.clone()
                .field("transitions")
                .index(index)
                .field("guard"),
            diagnostics,
        );
        validate_actions(
            program,
            &chart.events,
            &transition.actions,
            path.clone()
                .field("transitions")
                .index(index)
                .field("actions"),
            &mut indices.actions,
            diagnostics,
        );
    }

    if !indices.states.contains_key(&chart.root) {
        diagnostics.push(missing(path.clone().field("root"), chart.root, None));
    }
    validate_structure(program, &indices, diagnostics);
    compute_depths(program, &mut indices, diagnostics);
    validate_transitions(program, &indices, diagnostics);
    validate_history(program, &indices, diagnostics);
    validate_static_conflicts(program, &indices, diagnostics);
    validate_eventless_cycles(program, &indices, diagnostics);
    validate_reachability(program, &indices, diagnostics);
    indices
}

fn validate_structure(
    program: &CheckedProgram,
    indices: &StatechartIndices,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(chart) = program.artifact().statechart.as_ref() else {
        return;
    };
    let base = DiagnosticPath::root().field("statechart");
    for (index, state) in chart.states.iter().enumerate() {
        let path = base.clone().field("states").index(index);
        if state.id == chart.root {
            if state.parent.is_some() || state.region.is_some() {
                diagnostics.push(structure(
                    path.clone(),
                    "root state cannot have a parent or region",
                    state.source_span,
                ));
            }
        } else {
            let Some(parent) = state.parent else {
                diagnostics.push(structure(
                    path.clone(),
                    "non-root state requires a parent",
                    state.source_span,
                ));
                continue;
            };
            let Some(region) = state.region else {
                diagnostics.push(structure(
                    path.clone(),
                    "non-root state requires a region",
                    state.source_span,
                ));
                continue;
            };
            if !indices.states.contains_key(&parent) {
                diagnostics.push(missing(
                    path.clone().field("parent"),
                    parent,
                    state.source_span,
                ));
            }
            match indices
                .regions
                .get(&region)
                .and_then(|position| chart.regions.get(*position))
            {
                Some(declaration) if declaration.parent == parent => {}
                Some(_) => diagnostics.push(structure(
                    path.clone().field("region"),
                    "state region belongs to another parent",
                    state.source_span,
                )),
                None => diagnostics.push(missing(
                    path.clone().field("region"),
                    region,
                    state.source_span,
                )),
            }
        }
        let region_count = indices.regions_by_parent.get(&state.id).map_or(0, Vec::len);
        let valid_regions = match state.kind {
            StateKindV0::Atomic | StateKindV0::Final => region_count == 0,
            StateKindV0::Compound => region_count == 1,
            StateKindV0::Parallel => region_count >= 2,
        };
        if !valid_regions {
            diagnostics.push(structure(
                path.clone(),
                "state kind has an invalid number of child regions",
                state.source_span,
            ));
        }
        if state.kind == StateKindV0::Final
            && indices
                .transitions_by_source
                .get(&state.id)
                .is_some_and(|items| !items.is_empty())
        {
            diagnostics.push(structure(
                path.clone(),
                "final state cannot have outgoing transitions",
                state.source_span,
            ));
        }
        if state.completion_event.is_some()
            && !matches!(state.kind, StateKindV0::Compound | StateKindV0::Parallel)
        {
            diagnostics.push(structure(
                path.clone().field("completion_event"),
                "only compound or parallel states can raise completion events",
                state.source_span,
            ));
        }
        if let Some(event) = state.completion_event
            && chart.events.binary_search(&event).is_err()
        {
            diagnostics.push(missing(
                path.clone().field("completion_event"),
                event,
                state.source_span,
            ));
        }
    }
    for (index, region) in chart.regions.iter().enumerate() {
        let path = base.clone().field("regions").index(index);
        let Some(parent) = state(program, indices, region.parent) else {
            diagnostics.push(missing(
                path.clone().field("parent"),
                region.parent,
                region.source_span,
            ));
            continue;
        };
        if !matches!(parent.kind, StateKindV0::Compound | StateKindV0::Parallel) {
            diagnostics.push(structure(
                path.clone().field("parent"),
                "region parent must be compound or parallel",
                region.source_span,
            ));
        }
        match state(program, indices, region.initial) {
            Some(initial)
                if initial.parent == Some(region.parent) && initial.region == Some(region.id) => {}
            Some(_) => diagnostics.push(structure(
                path.clone().field("initial"),
                "region initial state must be its direct child",
                region.source_span,
            )),
            None => diagnostics.push(missing(
                path.clone().field("initial"),
                region.initial,
                region.source_span,
            )),
        }
    }
}

fn compute_depths(
    program: &CheckedProgram,
    indices: &mut StatechartIndices,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(chart) = program.artifact().statechart.as_ref() else {
        return;
    };
    for state_record in &chart.states {
        let mut current = state_record.id;
        let mut seen = BTreeSet::new();
        let mut depth = 0_usize;
        loop {
            if !seen.insert(current) {
                diagnostics.push(structure(
                    DiagnosticPath::root().field("statechart").field("states"),
                    "state parent graph contains a cycle",
                    state_record.source_span,
                ));
                break;
            }
            let Some(node) = state(program, indices, current) else {
                break;
            };
            match node.parent {
                Some(parent) => {
                    depth = depth.saturating_add(1);
                    current = parent;
                }
                None => {
                    if current != chart.root {
                        diagnostics.push(structure(
                            DiagnosticPath::root().field("statechart").field("states"),
                            "state parent chain does not end at root",
                            state_record.source_span,
                        ));
                    }
                    indices.depths.insert(state_record.id, depth);
                    break;
                }
            }
        }
    }
}

fn validate_transitions(
    program: &CheckedProgram,
    indices: &StatechartIndices,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(chart) = program.artifact().statechart.as_ref() else {
        return;
    };
    let base = DiagnosticPath::root()
        .field("statechart")
        .field("transitions");
    for (index, transition) in chart.transitions.iter().enumerate() {
        let path = base.clone().index(index);
        if !indices.states.contains_key(&transition.source) {
            diagnostics.push(missing(
                path.clone().field("source"),
                transition.source,
                transition.source_span,
            ));
            continue;
        }
        if let Some(event) = transition.event
            && chart.events.binary_search(&event).is_err()
        {
            diagnostics.push(missing(
                path.clone().field("event"),
                event,
                transition.source_span,
            ));
        }
        let mut resolved = Vec::new();
        for (target_index, target) in transition.targets.iter().enumerate() {
            match target {
                TransitionTargetV0::State(id) if indices.states.contains_key(id) => {
                    resolved.push(*id)
                }
                TransitionTargetV0::History(id) => match history(program, indices, *id) {
                    Some(item) => resolved.extend(item.default_targets.iter().copied()),
                    None => diagnostics.push(missing(
                        path.clone().field("targets").index(target_index),
                        id,
                        transition.source_span,
                    )),
                },
                TransitionTargetV0::State(id) => diagnostics.push(missing(
                    path.clone().field("targets").index(target_index),
                    id,
                    transition.source_span,
                )),
            }
        }
        if transition.kind == TransitionKindV0::Internal
            && resolved
                .iter()
                .any(|target| !is_descendant(program, indices, *target, transition.source))
        {
            diagnostics.push(structure(
                path.clone(),
                "internal transition targets must be descendants of its source",
                transition.source_span,
            ));
        }
        if resolved.len() > 1 && !targets_are_orthogonal(program, indices, &resolved) {
            diagnostics.push(structure(
                path.clone().field("targets"),
                "multi-target transition must target distinct regions of one parallel state",
                transition.source_span,
            ));
        }
    }
}

fn validate_history(
    program: &CheckedProgram,
    indices: &StatechartIndices,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(chart) = program.artifact().statechart.as_ref() else {
        return;
    };
    let base = DiagnosticPath::root()
        .field("statechart")
        .field("histories");
    for (index, item) in chart.histories.iter().enumerate() {
        let path = base.clone().index(index);
        let parent = state(program, indices, item.parent);
        if !parent.is_some_and(|state| {
            matches!(state.kind, StateKindV0::Compound | StateKindV0::Parallel)
        }) {
            diagnostics.push(structure(
                path.clone().field("parent"),
                "history parent must be compound or parallel",
                item.source_span,
            ));
        }
        if !region(program, indices, item.region).is_some_and(|region| region.parent == item.parent)
        {
            diagnostics.push(structure(
                path.clone().field("region"),
                "history region must belong to its parent",
                item.source_span,
            ));
        }
        if item.default_targets.is_empty() {
            diagnostics.push(structure(
                path.clone().field("default_targets"),
                "history requires a default target",
                item.source_span,
            ));
        }
        for target in &item.default_targets {
            let valid = state(program, indices, *target).is_some_and(|target_state| {
                is_descendant(program, indices, *target, item.parent)
                    && (item.kind == HistoryKindV0::Deep
                        || target_state.parent == Some(item.parent))
            });
            if !valid {
                diagnostics.push(structure(
                    path.clone().field("default_targets"),
                    "history default target is outside its legal scope",
                    item.source_span,
                ));
            }
        }
    }
}

fn validate_static_conflicts(
    program: &CheckedProgram,
    indices: &StatechartIndices,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(chart) = program.artifact().statechart.as_ref() else {
        return;
    };
    for (source, transition_indices) in &indices.transitions_by_source {
        for (offset, left_index) in transition_indices.iter().enumerate() {
            for right_index in transition_indices.iter().skip(offset.saturating_add(1)) {
                let left = &chart.transitions[*left_index];
                let right = &chart.transitions[*right_index];
                if left.event == right.event
                    && matches!(left.guard, GuardExprV0::Always)
                    && matches!(right.guard, GuardExprV0::Always)
                {
                    diagnostics.push(
                        Diagnostic::new(
                            DiagnosticClass::Conflict,
                            STATECHART_CONFLICT,
                            DiagnosticPath::root()
                                .field("statechart")
                                .field("transitions")
                                .index(*right_index),
                            "two unconditional transitions at one source match the same event",
                        )
                        .with_id(*source)
                        .with_span(right.source_span),
                    );
                }
            }
        }
    }
    for (left_index, left) in chart.transitions.iter().enumerate() {
        for (right_index, right) in chart
            .transitions
            .iter()
            .enumerate()
            .skip(left_index.saturating_add(1))
        {
            if left.event != right.event
                || !matches!(left.guard, GuardExprV0::Always)
                || !matches!(right.guard, GuardExprV0::Always)
                || !sources_are_orthogonal(program, indices, left.source, right.source)
            {
                continue;
            }
            let left_writes = action_writes(program, &left.actions);
            let right_writes = action_writes(program, &right.actions);
            if left_writes
                .iter()
                .any(|global| right_writes.contains(global))
            {
                diagnostics.push(
                    Diagnostic::new(
                        DiagnosticClass::Conflict,
                        STATECHART_CONFLICT,
                        DiagnosticPath::root()
                            .field("statechart")
                            .field("transitions")
                            .index(right_index),
                        "orthogonal transitions can write the same global in one microstep",
                    )
                    .with_id(left.id)
                    .with_id(right.id)
                    .with_span(right.source_span),
                );
            }
        }
    }
}

fn validate_eventless_cycles(
    program: &CheckedProgram,
    indices: &StatechartIndices,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(chart) = program.artifact().statechart.as_ref() else {
        return;
    };
    let mut graph = BTreeMap::<StateId, Vec<StateId>>::new();
    for transition in &chart.transitions {
        if transition.event.is_some() || !matches!(transition.guard, GuardExprV0::Always) {
            continue;
        }
        for target in &transition.targets {
            match target {
                TransitionTargetV0::State(target) => {
                    graph.entry(transition.source).or_default().push(*target);
                }
                TransitionTargetV0::History(id) => {
                    if let Some(item) = history(program, indices, *id) {
                        graph
                            .entry(transition.source)
                            .or_default()
                            .extend(item.default_targets.iter().copied());
                    }
                }
            }
        }
    }
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for state in graph.keys().copied().collect::<Vec<_>>() {
        if has_cycle(state, &graph, &mut visiting, &mut visited) {
            diagnostics.push(
                Diagnostic::new(
                    DiagnosticClass::Validation,
                    STATECHART_EVENTLESS_CYCLE,
                    DiagnosticPath::root()
                        .field("statechart")
                        .field("transitions"),
                    "unconditional eventless transition graph contains a cycle",
                )
                .with_id(state),
            );
            break;
        }
    }
}

fn validate_reachability(
    program: &CheckedProgram,
    indices: &StatechartIndices,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(chart) = program.artifact().statechart.as_ref() else {
        return;
    };
    if !indices.states.contains_key(&chart.root) {
        return;
    }
    let mut reachable = BTreeSet::new();
    enter_reachable(program, indices, chart.root, &mut reachable);
    loop {
        let before = reachable.len();
        for transition in &chart.transitions {
            if reachable.contains(&transition.source) {
                for target in &transition.targets {
                    match target {
                        TransitionTargetV0::State(target) => {
                            enter_reachable(program, indices, *target, &mut reachable)
                        }
                        TransitionTargetV0::History(id) => {
                            if let Some(item) = history(program, indices, *id) {
                                for target in &item.default_targets {
                                    enter_reachable(program, indices, *target, &mut reachable);
                                }
                            }
                        }
                    }
                }
            }
        }
        if reachable.len() == before {
            break;
        }
    }
    for (index, state) in chart.states.iter().enumerate() {
        if !reachable.contains(&state.id) {
            diagnostics.push(
                Diagnostic::new(
                    DiagnosticClass::Validation,
                    STATECHART_UNREACHABLE,
                    DiagnosticPath::root()
                        .field("statechart")
                        .field("states")
                        .index(index),
                    "state is unreachable from initial configuration or transition targets",
                )
                .with_id(state.id)
                .with_span(state.source_span),
            );
        }
        if state.kind == StateKindV0::Atomic && !has_exit(program, indices, state.id) {
            diagnostics.push(
                Diagnostic::new(
                    DiagnosticClass::Validation,
                    CONTROL_FLOW_INVALID,
                    DiagnosticPath::root()
                        .field("statechart")
                        .field("states")
                        .index(index),
                    "atomic state has no outgoing transition on itself or an ancestor",
                )
                .with_id(state.id)
                .with_span(state.source_span),
            );
        }
    }
}

fn validate_actions(
    program: &CheckedProgram,
    events: &[EventTypeId],
    actions: &[ActionRecordV0],
    path: DiagnosticPath,
    seen: &mut BTreeSet<ActionId>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (index, action) in actions.iter().enumerate() {
        let action_path = path.clone().index(index);
        if !seen.insert(action.id) {
            diagnostics.push(duplicate(
                action_path.clone(),
                action.id,
                action.source_span,
            ));
        }
        match &action.action {
            StatechartActionV0::Assign { global, value } => match program.global(*global) {
                Some(declaration) if declaration.kind == value.kind() => {}
                Some(_) => diagnostics.push(
                    Diagnostic::new(
                        DiagnosticClass::Validation,
                        KIND_MISMATCH,
                        action_path.clone(),
                        "assigned Value does not match global kind",
                    )
                    .with_span(action.source_span),
                ),
                None => diagnostics.push(missing(action_path.clone(), global, action.source_span)),
            },
            StatechartActionV0::Raise { event } => {
                if events.binary_search(event).is_err() {
                    diagnostics.push(missing(action_path.clone(), event, action.source_span));
                }
            }
            StatechartActionV0::StartFlow {
                flow,
                result_to,
                done_event: _,
            } => match program.flow(*flow) {
                Some(declaration) if declaration.parameters.is_empty() => {
                    match (declaration.return_kind, result_to) {
                        (None, None) => {}
                        (Some(kind), Some(global))
                            if program
                                .global(*global)
                                .is_some_and(|item| item.kind == kind) => {}
                        _ => diagnostics.push(
                            Diagnostic::new(
                                DiagnosticClass::Validation,
                                KIND_MISMATCH,
                                action_path.clone(),
                                "invoked Flow return kind does not match result global",
                            )
                            .with_span(action.source_span),
                        ),
                    }
                    if *flow == program.artifact().entry_flow {
                        diagnostics.push(
                            Diagnostic::new(
                                DiagnosticClass::Validation,
                                CONTROL_FLOW_INVALID,
                                action_path.clone(),
                                "Statechart cannot invoke the Program entry Flow",
                            )
                            .with_span(action.source_span),
                        );
                    }
                }
                Some(_) => diagnostics.push(
                    Diagnostic::new(
                        DiagnosticClass::Validation,
                        CONTROL_FLOW_INVALID,
                        action_path.clone(),
                        "Statechart invoked Flow cannot declare parameters",
                    )
                    .with_span(action.source_span),
                ),
                None => diagnostics.push(missing(action_path.clone(), flow, action.source_span)),
            },
            StatechartActionV0::EmitEffect {
                capability,
                payload,
                response_to,
                response_event,
            } => match program.capability(capability) {
                Some(declaration) => {
                    if !declaration.request_schema.accepts(payload) {
                        diagnostics.push(
                            Diagnostic::new(
                                DiagnosticClass::Validation,
                                KIND_MISMATCH,
                                action_path.clone(),
                                "Statechart Effect payload violates request schema",
                            )
                            .with_span(action.source_span),
                        );
                    }
                    match (response_to, declaration.response_schema.static_kind()) {
                        (Some(global), Some(kind))
                            if program
                                .global(*global)
                                .is_some_and(|item| item.kind == kind) => {}
                        (None, _) => {}
                        _ => diagnostics.push(
                            Diagnostic::new(
                                DiagnosticClass::Validation,
                                KIND_MISMATCH,
                                action_path.clone(),
                                "Statechart Effect response global has an incompatible kind",
                            )
                            .with_span(action.source_span),
                        ),
                    }
                    if response_to.is_none() && response_event.is_none() {
                        diagnostics.push(
                            Diagnostic::new(
                                DiagnosticClass::Validation,
                                CONTROL_FLOW_INVALID,
                                action_path.clone(),
                                "response-bearing Statechart Effect needs a response target or event",
                            )
                            .with_span(action.source_span),
                        );
                    }
                }
                None => {
                    diagnostics.push(missing(action_path.clone(), capability, action.source_span))
                }
            },
        }
        let event = match &action.action {
            StatechartActionV0::StartFlow { done_event, .. } => done_event.as_ref(),
            StatechartActionV0::EmitEffect { response_event, .. } => response_event.as_ref(),
            StatechartActionV0::Assign { .. } | StatechartActionV0::Raise { .. } => None,
        };
        if let Some(event) = event
            && events.binary_search(event).is_err()
        {
            diagnostics.push(missing(action_path, *event, action.source_span));
        }
    }
}

fn validate_guard(
    program: &CheckedProgram,
    guard: &GuardExprV0,
    depth: u32,
    maximum: u32,
    path: DiagnosticPath,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if depth > maximum {
        diagnostics.push(Diagnostic::new(
            DiagnosticClass::LimitExceeded,
            LIMIT_EXCEEDED,
            path,
            "Statechart guard exceeds configured depth",
        ));
        return;
    }
    match guard {
        GuardExprV0::Always => {}
        GuardExprV0::Bool { global, .. } => {
            if program.global(*global).map(|item| item.kind) != Some(ValueKindV0::Bool) {
                diagnostics.push(Diagnostic::new(
                    DiagnosticClass::Validation,
                    KIND_MISMATCH,
                    path,
                    "Bool guard requires a Bool global",
                ));
            }
        }
        GuardExprV0::Equals { global, value } => match program.global(*global) {
            Some(item) if item.kind == value.kind() => {}
            Some(_) => diagnostics.push(Diagnostic::new(
                DiagnosticClass::Validation,
                KIND_MISMATCH,
                path,
                "Equals guard Value does not match global kind",
            )),
            None => diagnostics.push(missing(path, global, None)),
        },
        GuardExprV0::Not(inner) => validate_guard(
            program,
            inner,
            depth.saturating_add(1),
            maximum,
            path,
            diagnostics,
        ),
        GuardExprV0::All(items) | GuardExprV0::Any(items) => {
            if items.is_empty() {
                diagnostics.push(Diagnostic::new(
                    DiagnosticClass::Validation,
                    CONTROL_FLOW_INVALID,
                    path.clone(),
                    "All/Any guard requires at least one operand",
                ));
            }
            for (index, item) in items.iter().enumerate() {
                validate_guard(
                    program,
                    item,
                    depth.saturating_add(1),
                    maximum,
                    path.clone().index(index),
                    diagnostics,
                );
            }
        }
    }
}

fn action_writes(program: &CheckedProgram, actions: &[ActionRecordV0]) -> BTreeSet<GlobalId> {
    let mut writes = BTreeSet::new();
    for action in actions {
        match &action.action {
            StatechartActionV0::Assign { global, .. } => {
                writes.insert(*global);
            }
            StatechartActionV0::StartFlow { flow, .. } => {
                collect_flow_writes(program, *flow, &mut BTreeSet::new(), &mut writes);
            }
            StatechartActionV0::Raise { .. } | StatechartActionV0::EmitEffect { .. } => {}
        }
    }
    writes
}

fn collect_flow_writes(
    program: &CheckedProgram,
    flow: FlowId,
    visited: &mut BTreeSet<FlowId>,
    writes: &mut BTreeSet<GlobalId>,
) {
    if !visited.insert(flow) {
        return;
    }
    let Some(flow) = program.flow(flow) else {
        return;
    };
    for instruction in &flow.instructions {
        match instruction.op {
            OpV0::Store {
                slot: SlotRefV0::Global(global),
                ..
            } => {
                writes.insert(global);
            }
            OpV0::Call { flow, .. } => collect_flow_writes(program, flow, visited, writes),
            _ => {}
        }
    }
}

fn has_cycle(
    state: StateId,
    graph: &BTreeMap<StateId, Vec<StateId>>,
    visiting: &mut BTreeSet<StateId>,
    visited: &mut BTreeSet<StateId>,
) -> bool {
    if visiting.contains(&state) {
        return true;
    }
    if !visited.insert(state) {
        return false;
    }
    visiting.insert(state);
    let found = graph.get(&state).is_some_and(|targets| {
        targets
            .iter()
            .any(|target| has_cycle(*target, graph, visiting, visited))
    });
    visiting.remove(&state);
    found
}

fn enter_reachable(
    program: &CheckedProgram,
    indices: &StatechartIndices,
    id: StateId,
    reachable: &mut BTreeSet<StateId>,
) {
    if !reachable.insert(id) {
        return;
    }
    let Some(node) = state(program, indices, id) else {
        return;
    };
    if matches!(node.kind, StateKindV0::Compound | StateKindV0::Parallel)
        && let Some(regions) = indices.regions_by_parent.get(&id)
    {
        for region_id in regions {
            if let Some(region) = region(program, indices, *region_id) {
                enter_reachable(program, indices, region.initial, reachable);
            }
        }
    }
}

fn has_exit(program: &CheckedProgram, indices: &StatechartIndices, id: StateId) -> bool {
    let mut current = Some(id);
    while let Some(id) = current {
        if indices
            .transitions_by_source
            .get(&id)
            .is_some_and(|items| !items.is_empty())
        {
            return true;
        }
        current = state(program, indices, id).and_then(|state| state.parent);
    }
    false
}

fn targets_are_orthogonal(
    program: &CheckedProgram,
    indices: &StatechartIndices,
    targets: &[StateId],
) -> bool {
    let mut parallel = None;
    let mut regions = BTreeSet::new();
    for target in targets {
        let Some((owner, region)) = nearest_parallel_region(program, indices, *target) else {
            return false;
        };
        if parallel.is_some_and(|existing| existing != owner) {
            return false;
        }
        parallel = Some(owner);
        if !regions.insert(region) {
            return false;
        }
    }
    true
}

fn sources_are_orthogonal(
    program: &CheckedProgram,
    indices: &StatechartIndices,
    left: StateId,
    right: StateId,
) -> bool {
    match (
        nearest_parallel_region(program, indices, left),
        nearest_parallel_region(program, indices, right),
    ) {
        (Some((left_owner, left_region)), Some((right_owner, right_region))) => {
            left_owner == right_owner && left_region != right_region
        }
        _ => false,
    }
}

fn nearest_parallel_region(
    program: &CheckedProgram,
    indices: &StatechartIndices,
    id: StateId,
) -> Option<(StateId, RegionId)> {
    let mut current = id;
    loop {
        let node = state(program, indices, current)?;
        let parent = node.parent?;
        let parent_node = state(program, indices, parent)?;
        if parent_node.kind == StateKindV0::Parallel {
            return Some((parent, node.region?));
        }
        current = parent;
    }
}

pub(crate) fn is_descendant(
    program: &CheckedProgram,
    indices: &StatechartIndices,
    descendant: StateId,
    ancestor: StateId,
) -> bool {
    let mut current = Some(descendant);
    while let Some(id) = current {
        if id == ancestor {
            return true;
        }
        current = state(program, indices, id).and_then(|node| node.parent);
    }
    false
}

fn state<'a>(
    program: &'a CheckedProgram,
    indices: &StatechartIndices,
    id: StateId,
) -> Option<&'a super::StateV0> {
    let index = indices.states.get(&id)?;
    program.artifact().statechart.as_ref()?.states.get(*index)
}

fn region<'a>(
    program: &'a CheckedProgram,
    indices: &StatechartIndices,
    id: RegionId,
) -> Option<&'a super::RegionV0> {
    let index = indices.regions.get(&id)?;
    program.artifact().statechart.as_ref()?.regions.get(*index)
}

fn history<'a>(
    program: &'a CheckedProgram,
    indices: &StatechartIndices,
    id: HistoryId,
) -> Option<&'a super::HistoryV0> {
    let index = indices.histories.get(&id)?;
    program
        .artifact()
        .statechart
        .as_ref()?
        .histories
        .get(*index)
}

fn duplicate(
    path: DiagnosticPath,
    id: impl ToString,
    span: Option<crate::diagnostic::SourceSpan>,
) -> Diagnostic {
    Diagnostic::new(
        DiagnosticClass::Validation,
        DUPLICATE_ID,
        path,
        "duplicate Statechart identity",
    )
    .with_id(id)
    .with_span(span)
}

fn missing(
    path: DiagnosticPath,
    id: impl ToString,
    span: Option<crate::diagnostic::SourceSpan>,
) -> Diagnostic {
    Diagnostic::new(
        DiagnosticClass::Validation,
        MISSING_REFERENCE,
        path,
        "Statechart reference does not exist",
    )
    .with_id(id)
    .with_span(span)
}

fn structure(
    path: DiagnosticPath,
    message: &'static str,
    span: Option<crate::diagnostic::SourceSpan>,
) -> Diagnostic {
    Diagnostic::new(
        DiagnosticClass::Validation,
        STATECHART_STRUCTURE_INVALID,
        path,
        message,
    )
    .with_span(span)
}
