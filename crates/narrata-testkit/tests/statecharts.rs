#![allow(clippy::panic, clippy::unwrap_used)]

use std::sync::Arc;

use narrata_core::{
    ActionId, CommitId, EffectResponseV0, EventTypeId, ExecutionId, FlowId, GlobalId, InputId,
    InstructionId, ProgramId, RegionId, StateId, TransitionId, Value, ValueKindV0,
    limits::{MacrostepLimits, ProgramLoadLimits, SnapshotLoadLimits},
    program::{
        FlowV0, GlobalDeclV0, InstructionRecordV0, OpV0, ProgramArtifactV0, ReturnModeV0,
        encode_program_artifact, load_program, validate_program,
    },
    runtime::{
        CheckedRuntimeInput, DraftResult, RuntimeFault, RuntimeStatusV0, SliceBudget, SliceOutcome,
        TransitionDraft, begin_transition_with_parent_commit, new_execution,
    },
    snapshot::{export_snapshot, restore_snapshot, state_digest},
    statechart::{
        ActionRecordV0, GuardExprV0, RegionV0, StateKindV0, StateV0, StatechartActionV0,
        StatechartTraceKindV0, StatechartV0, TransitionKindV0, TransitionTargetV0, TransitionV0,
    },
    version::{PROGRAM_FORMAT_V0, SEMANTICS_V0},
};
use narrata_testkit::generator::statechart_parallel_history_v0;

#[test]
fn checked_in_statechart_semantic_fixture_executes() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/statechart/parallel-history-v0.json"
    ))
    .unwrap();
    let program = checked_program();
    let mut state = Arc::new(new_execution(&program, ExecutionId::from_u128(777)).unwrap());
    for (index, step) in fixture["steps"].as_array().unwrap().iter().enumerate() {
        let input_id = InputId::from_u128((index + 1) as u128);
        let input = match step["kind"].as_str().unwrap() {
            "start" => CheckedRuntimeInput::start(input_id),
            "event" => CheckedRuntimeInput::event(
                input_id,
                narrata_core::EventTypeId::from_u128(step["event"].as_u64().unwrap() as u128),
            ),
            "effect_response" => {
                let pending = state.pending_effect().unwrap();
                CheckedRuntimeInput::effect_response(
                    input_id,
                    pending,
                    &program,
                    EffectResponseV0 {
                        effect: pending.request.id,
                        request_digest: pending.request.request_digest,
                        capability: pending.request.capability.clone(),
                        capability_version: pending.request.capability_version,
                        payload: Value::I64(step["value"].as_i64().unwrap()),
                    },
                )
                .unwrap()
            }
            other => panic!("unknown fixture input {other}"),
        };
        let draft = run(
            Arc::clone(&program),
            state,
            CommitId::from_bytes([(index + 1) as u8; 32]),
            input,
            MacrostepLimits::default(),
            SliceBudget::new((index % 3 + 1) as u64).unwrap(),
        )
        .unwrap();
        match step["expect"].as_str().unwrap() {
            "effect" => assert!(matches!(draft.result(), DraftResult::AwaitEffect(_))),
            "stable" => {
                let active = step["active"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|id| id.as_u64().unwrap() as u128)
                    .collect::<Vec<_>>();
                assert_active(&draft, &active);
            }
            "finished" => assert!(matches!(draft.result(), DraftResult::Finished(_))),
            other => panic!("unknown fixture expectation {other}"),
        }
        let actual_trace = draft
            .statechart_trace()
            .iter()
            .map(|event| format!("{:?}", event.kind))
            .collect::<Vec<_>>();
        if let Some(expected) = step.get("trace").and_then(serde_json::Value::as_array) {
            let expected = expected
                .iter()
                .map(|kind| kind.as_str().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                actual_trace
                    .iter()
                    .take(expected.len())
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                expected
            );
        }
        if let Some(expected) = step
            .get("trace_contains")
            .and_then(serde_json::Value::as_array)
        {
            for kind in expected {
                assert!(
                    actual_trace
                        .iter()
                        .any(|actual| actual == kind.as_str().unwrap())
                );
            }
        }
        state = Arc::new(draft.into_next_state());
    }
}

#[test]
fn statechart_program_codec_and_validation_are_checked() {
    let artifact = statechart_parallel_history_v0().unwrap();
    let bytes = encode_program_artifact(&artifact);
    let checked = load_program(&bytes, &ProgramLoadLimits::default()).unwrap();
    assert_eq!(checked.artifact(), &artifact);
    assert!(checked.state(StateId::from_u128(4)).is_some());

    let mut conflict = artifact.clone();
    let chart = conflict.statechart.as_mut().unwrap();
    let right = chart
        .transitions
        .iter_mut()
        .find(|transition| transition.id == narrata_core::TransitionId::from_u128(6))
        .unwrap();
    let narrata_core::StatechartActionV0::Assign { global, .. } = &mut right.actions[0].action
    else {
        panic!("fixture action kind");
    };
    *global = narrata_core::GlobalId::from_u128(501);
    let diagnostics = validate_program(conflict, &ProgramLoadLimits::default()).unwrap_err();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "NAR-V0007")
    );

    let mut eventless_cycle = artifact;
    let transitions = &mut eventless_cycle.statechart.as_mut().unwrap().transitions;
    transitions[0].event = None;
    transitions[1].event = None;
    transitions[2].event = None;
    transitions[2].targets = vec![narrata_core::TransitionTargetV0::State(StateId::from_u128(
        2,
    ))];
    let diagnostics = validate_program(eventless_cycle, &ProgramLoadLimits::default()).unwrap_err();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "NAR-V0008")
    );
}

#[test]
fn flow_effect_parallel_history_and_completion_are_one_deterministic_model() {
    let program = checked_program();
    let initial = Arc::new(new_execution(&program, ExecutionId::from_u128(500)).unwrap());
    let first = run(
        program.clone(),
        initial,
        CommitId::from_bytes([1; 32]),
        CheckedRuntimeInput::start(InputId::from_u128(1)),
        MacrostepLimits::default(),
        SliceBudget::new(1).unwrap(),
    )
    .unwrap();
    assert!(matches!(first.result(), DraftResult::AwaitEffect(_)));
    assert!(matches!(
        first.next_state().status,
        RuntimeStatusV0::AwaitingEffect { .. }
    ));
    let first_trace = first.statechart_trace();
    assert_eq!(first_trace[0].kind, StatechartTraceKindV0::Entry);
    assert_eq!(first_trace[1].kind, StatechartTraceKindV0::Entry);
    assert_eq!(first_trace[2].kind, StatechartTraceKindV0::Action);
    assert_eq!(first_trace[3].kind, StatechartTraceKindV0::Invoke);

    let first_response = effect_input(&program, &first, 2, Value::I64(42));
    let second = run(
        program.clone(),
        Arc::new(first.into_next_state()),
        CommitId::from_bytes([2; 32]),
        first_response,
        MacrostepLimits::default(),
        SliceBudget::new(2).unwrap(),
    )
    .unwrap();
    assert!(matches!(second.result(), DraftResult::AwaitEffect(_)));
    assert!(matches!(
        second.next_state().status,
        RuntimeStatusV0::AwaitingStatechartEffect { .. }
    ));
    let ordered = second.statechart_trace();
    assert!(
        ordered
            .iter()
            .any(|event| event.kind == StatechartTraceKindV0::Resume)
    );
    assert!(ordered.iter().any(|event| {
        event.kind == StatechartTraceKindV0::Transition
            && event.transition == Some(narrata_core::TransitionId::from_u128(1))
    }));
    assert_eq!(
        second.next_state().globals[&narrata_core::GlobalId::from_u128(500)],
        Value::I64(42)
    );

    let second_response = effect_input(&program, &second, 3, Value::I64(99));
    let third = run(
        program.clone(),
        Arc::new(second.into_next_state()),
        CommitId::from_bytes([3; 32]),
        second_response,
        MacrostepLimits::default(),
        SliceBudget::unlimited(),
    )
    .unwrap();
    assert_active(&third, &[10, 20]);
    assert_eq!(
        third.next_state().globals[&narrata_core::GlobalId::from_u128(500)],
        Value::I64(99)
    );

    let paused = run(
        program.clone(),
        Arc::new(third.into_next_state()),
        CommitId::from_bytes([4; 32]),
        CheckedRuntimeInput::event(
            InputId::from_u128(4),
            narrata_core::EventTypeId::from_u128(3),
        ),
        MacrostepLimits::default(),
        SliceBudget::new(1).unwrap(),
    )
    .unwrap();
    assert_active(&paused, &[5]);
    let paused_state = paused.next_state();
    assert_eq!(
        paused_state.statechart.as_ref().unwrap().history[&narrata_core::HistoryId::from_u128(1)],
        vec![StateId::from_u128(10)]
    );
    let restored = restore_snapshot(
        &export_snapshot(paused_state).unwrap(),
        &program,
        &SnapshotLoadLimits::default(),
    )
    .unwrap();
    assert_eq!(restored, *paused_state);

    let resumed = run(
        program.clone(),
        Arc::new(restored),
        CommitId::from_bytes([5; 32]),
        CheckedRuntimeInput::event(
            InputId::from_u128(5),
            narrata_core::EventTypeId::from_u128(4),
        ),
        MacrostepLimits::default(),
        SliceBudget::unlimited(),
    )
    .unwrap();
    assert_active(&resumed, &[10, 20]);
    let parent = Arc::new(resumed.next_state().clone());
    let parent_digest = state_digest(&parent);

    let fault = run(
        program.clone(),
        parent.clone(),
        CommitId::from_bytes([6; 32]),
        CheckedRuntimeInput::event(
            InputId::from_u128(6),
            narrata_core::EventTypeId::from_u128(5),
        ),
        MacrostepLimits {
            max_microsteps: 1,
            ..MacrostepLimits::default()
        },
        SliceBudget::unlimited(),
    )
    .unwrap_err();
    assert_eq!(fault, RuntimeFault::MicrostepLimit);
    assert_eq!(state_digest(&parent), parent_digest);

    let finished = run(
        program,
        parent,
        CommitId::from_bytes([6; 32]),
        CheckedRuntimeInput::event(
            InputId::from_u128(6),
            narrata_core::EventTypeId::from_u128(5),
        ),
        MacrostepLimits::default(),
        SliceBudget::new(1).unwrap(),
    )
    .unwrap();
    assert!(
        matches!(finished.result(), DraftResult::Finished(Value::Null)),
        "unexpected result: {:?}, state: {:?}",
        finished.result(),
        finished.next_state().statechart
    );
    assert!(matches!(
        finished.next_state().status,
        RuntimeStatusV0::StatechartFinished
    ));
    assert_eq!(
        finished.next_state().globals[&narrata_core::GlobalId::from_u128(501)],
        Value::Bool(true)
    );
    assert_eq!(
        finished.next_state().globals[&narrata_core::GlobalId::from_u128(502)],
        Value::Bool(true)
    );
}

#[test]
fn snapshot_restore_rejects_unknown_active_state() {
    let program = checked_program();
    let mut state = new_execution(&program, ExecutionId::from_u128(501)).unwrap();
    state
        .statechart
        .as_mut()
        .unwrap()
        .active
        .insert(StateId::from_u128(u128::MAX));
    state.turn = narrata_core::runtime::Turn(1);
    let bytes = export_snapshot(&state).unwrap();
    assert!(restore_snapshot(&bytes, &program, &SnapshotLoadLimits::default()).is_err());
}

#[test]
fn eventless_transitions_preempt_and_cancel_exited_entry_work() {
    let mut artifact = statechart_parallel_history_v0().unwrap();
    artifact
        .statechart
        .as_mut()
        .unwrap()
        .transitions
        .iter_mut()
        .find(|transition| transition.id == TransitionId::from_u128(1))
        .unwrap()
        .event = None;
    let program = Arc::new(validate_program(artifact, &ProgramLoadLimits::default()).unwrap());
    let state = Arc::new(new_execution(&program, ExecutionId::from_u128(502)).unwrap());
    let draft = run(
        program,
        state,
        CommitId::from_bytes([7; 32]),
        CheckedRuntimeInput::start(InputId::from_u128(1)),
        MacrostepLimits::default(),
        SliceBudget::unlimited(),
    )
    .unwrap();
    assert!(matches!(
        draft.next_state().status,
        RuntimeStatusV0::AwaitingStatechartEffect { .. }
    ));
    assert!(
        draft
            .statechart_trace()
            .iter()
            .any(|event| event.kind == StatechartTraceKindV0::Cancel)
    );
    assert!(
        draft
            .statechart_trace()
            .iter()
            .all(|event| event.kind != StatechartTraceKindV0::Invoke)
    );
}

#[test]
fn parallel_region_effects_are_serialized_in_canonical_region_order() {
    let mut artifact = statechart_parallel_history_v0().unwrap();
    let capability = artifact.capabilities[0].id.clone();
    for (state_id, action_id, payload) in [(10, 900, 10), (20, 901, 20)] {
        artifact
            .statechart
            .as_mut()
            .unwrap()
            .states
            .iter_mut()
            .find(|state| state.id == StateId::from_u128(state_id))
            .unwrap()
            .on_entry
            .push(ActionRecordV0 {
                id: ActionId::from_u128(action_id),
                action: StatechartActionV0::EmitEffect {
                    capability: capability.clone(),
                    payload: Value::I64(payload),
                    response_to: Some(GlobalId::from_u128(500)),
                    response_event: None,
                },
                source_span: None,
            });
    }
    let program = Arc::new(validate_program(artifact, &ProgramLoadLimits::default()).unwrap());
    let initial = Arc::new(new_execution(&program, ExecutionId::from_u128(504)).unwrap());
    let flow_effect = run(
        Arc::clone(&program),
        initial,
        CommitId::from_bytes([11; 32]),
        CheckedRuntimeInput::start(InputId::from_u128(1)),
        MacrostepLimits::default(),
        SliceBudget::unlimited(),
    )
    .unwrap();
    let direct_effect = run(
        Arc::clone(&program),
        Arc::new(flow_effect.next_state().clone()),
        CommitId::from_bytes([12; 32]),
        effect_input(&program, &flow_effect, 2, Value::I64(1)),
        MacrostepLimits::default(),
        SliceBudget::unlimited(),
    )
    .unwrap();
    let left_effect = run(
        Arc::clone(&program),
        Arc::new(direct_effect.next_state().clone()),
        CommitId::from_bytes([13; 32]),
        effect_input(&program, &direct_effect, 3, Value::I64(2)),
        MacrostepLimits::default(),
        SliceBudget::unlimited(),
    )
    .unwrap();
    assert_eq!(
        left_effect
            .next_state()
            .pending_effect()
            .unwrap()
            .statechart_site(),
        Some(ActionId::from_u128(900))
    );
    let right_effect = run(
        Arc::clone(&program),
        Arc::new(left_effect.next_state().clone()),
        CommitId::from_bytes([14; 32]),
        effect_input(&program, &left_effect, 4, Value::I64(3)),
        MacrostepLimits::default(),
        SliceBudget::unlimited(),
    )
    .unwrap();
    assert_eq!(
        right_effect
            .next_state()
            .pending_effect()
            .unwrap()
            .statechart_site(),
        Some(ActionId::from_u128(901))
    );
    let stable = run(
        program.clone(),
        Arc::new(right_effect.next_state().clone()),
        CommitId::from_bytes([15; 32]),
        effect_input(&program, &right_effect, 5, Value::I64(4)),
        MacrostepLimits::default(),
        SliceBudget::unlimited(),
    )
    .unwrap();
    assert_active(&stable, &[10, 20]);
}

#[test]
fn targetless_internal_and_external_self_transitions_have_distinct_traces() {
    let program = Arc::new(
        validate_program(edge_semantics_program(), &ProgramLoadLimits::default()).unwrap(),
    );
    let mut state = Arc::new(new_execution(&program, ExecutionId::from_u128(503)).unwrap());
    let started = run(
        Arc::clone(&program),
        state,
        CommitId::from_bytes([8; 32]),
        CheckedRuntimeInput::start(InputId::from_u128(1)),
        MacrostepLimits::default(),
        SliceBudget::unlimited(),
    )
    .unwrap();
    assert_active(&started, &[2]);
    state = Arc::new(started.into_next_state());

    let internal = run(
        Arc::clone(&program),
        state,
        CommitId::from_bytes([9; 32]),
        CheckedRuntimeInput::event(InputId::from_u128(2), EventTypeId::from_u128(1)),
        MacrostepLimits::default(),
        SliceBudget::unlimited(),
    )
    .unwrap();
    assert_active(&internal, &[2]);
    assert_eq!(
        internal
            .statechart_trace()
            .iter()
            .filter(|event| matches!(
                event.kind,
                StatechartTraceKindV0::Exit | StatechartTraceKindV0::Entry
            ))
            .count(),
        0
    );
    state = Arc::new(internal.into_next_state());

    let external_self = run(
        program,
        state,
        CommitId::from_bytes([10; 32]),
        CheckedRuntimeInput::event(InputId::from_u128(3), EventTypeId::from_u128(2)),
        MacrostepLimits::default(),
        SliceBudget::unlimited(),
    )
    .unwrap();
    assert_active(&external_self, &[2]);
    assert_eq!(
        external_self
            .statechart_trace()
            .iter()
            .filter(|event| event.kind == StatechartTraceKindV0::Exit)
            .count(),
        1
    );
    assert_eq!(
        external_self
            .statechart_trace()
            .iter()
            .filter(|event| event.kind == StatechartTraceKindV0::Entry)
            .count(),
        1
    );
}

fn edge_semantics_program() -> ProgramArtifactV0 {
    let flow = FlowId::from_u128(700);
    let root = StateId::from_u128(1);
    let active = StateId::from_u128(2);
    let final_state = StateId::from_u128(3);
    let region = RegionId::from_u128(1);
    let global = GlobalId::from_u128(700);
    let action = |id, value| ActionRecordV0 {
        id: ActionId::from_u128(id),
        action: StatechartActionV0::Assign {
            global,
            value: Value::I64(value),
        },
        source_span: None,
    };
    ProgramArtifactV0 {
        format_version: PROGRAM_FORMAT_V0,
        semantics_version: SEMANTICS_V0,
        program_id: ProgramId::from_u128(700),
        entry_flow: flow,
        constants: Vec::new(),
        globals: vec![GlobalDeclV0 {
            id: global,
            kind: ValueKindV0::I64,
            default: Value::I64(0),
        }],
        flows: vec![FlowV0 {
            id: flow,
            parameters: Vec::new(),
            locals: Vec::new(),
            return_kind: None,
            entry: InstructionId::from_u128(1),
            instructions: vec![InstructionRecordV0 {
                id: InstructionId::from_u128(1),
                op: OpV0::Finish {
                    value: ReturnModeV0::None,
                },
            }],
        }],
        capabilities: Vec::new(),
        content: Vec::new(),
        statechart: Some(StatechartV0 {
            root,
            events: (1..=3).map(EventTypeId::from_u128).collect(),
            states: vec![
                StateV0 {
                    id: root,
                    parent: None,
                    region: None,
                    kind: StateKindV0::Compound,
                    on_entry: Vec::new(),
                    on_exit: Vec::new(),
                    completion_event: None,
                    source_span: None,
                },
                StateV0 {
                    id: active,
                    parent: Some(root),
                    region: Some(region),
                    kind: StateKindV0::Atomic,
                    on_entry: Vec::new(),
                    on_exit: Vec::new(),
                    completion_event: None,
                    source_span: None,
                },
                StateV0 {
                    id: final_state,
                    parent: Some(root),
                    region: Some(region),
                    kind: StateKindV0::Final,
                    on_entry: Vec::new(),
                    on_exit: Vec::new(),
                    completion_event: None,
                    source_span: None,
                },
            ],
            regions: vec![RegionV0 {
                id: region,
                parent: root,
                initial: active,
                source_span: None,
            }],
            histories: Vec::new(),
            transitions: vec![
                TransitionV0 {
                    id: TransitionId::from_u128(1),
                    source: active,
                    event: Some(EventTypeId::from_u128(1)),
                    kind: TransitionKindV0::Internal,
                    guard: GuardExprV0::Always,
                    targets: Vec::new(),
                    actions: vec![action(1, 1)],
                    source_span: None,
                },
                TransitionV0 {
                    id: TransitionId::from_u128(2),
                    source: active,
                    event: Some(EventTypeId::from_u128(2)),
                    kind: TransitionKindV0::External,
                    guard: GuardExprV0::Always,
                    targets: vec![TransitionTargetV0::State(active)],
                    actions: vec![action(2, 2)],
                    source_span: None,
                },
                TransitionV0 {
                    id: TransitionId::from_u128(3),
                    source: active,
                    event: Some(EventTypeId::from_u128(3)),
                    kind: TransitionKindV0::External,
                    guard: GuardExprV0::Always,
                    targets: vec![TransitionTargetV0::State(final_state)],
                    actions: Vec::new(),
                    source_span: None,
                },
            ],
            source_span: None,
        }),
    }
}

fn checked_program() -> Arc<narrata_core::CheckedProgram> {
    Arc::new(
        validate_program(
            statechart_parallel_history_v0().unwrap(),
            &ProgramLoadLimits::default(),
        )
        .unwrap(),
    )
}

fn effect_input(
    program: &narrata_core::CheckedProgram,
    draft: &TransitionDraft,
    input: u128,
    payload: Value,
) -> CheckedRuntimeInput {
    let pending = draft.next_state().pending_effect().unwrap();
    CheckedRuntimeInput::effect_response(
        InputId::from_u128(input),
        pending,
        program,
        EffectResponseV0 {
            effect: pending.request.id,
            request_digest: pending.request.request_digest,
            capability: pending.request.capability.clone(),
            capability_version: pending.request.capability_version,
            payload,
        },
    )
    .unwrap()
}

fn assert_active(draft: &TransitionDraft, expected: &[u128]) {
    let DraftResult::StatechartStable(view) = draft.result() else {
        panic!("expected stable Statechart result");
    };
    assert_eq!(
        view.active,
        expected
            .iter()
            .copied()
            .map(StateId::from_u128)
            .collect::<Vec<_>>()
    );
}

fn run(
    program: Arc<narrata_core::CheckedProgram>,
    parent: Arc<narrata_core::RuntimeStateV0>,
    parent_commit: CommitId,
    input: CheckedRuntimeInput,
    limits: MacrostepLimits,
    slice: SliceBudget,
) -> Result<TransitionDraft, RuntimeFault> {
    let mut outcome =
        begin_transition_with_parent_commit(program, parent, parent_commit, input, limits)
            .unwrap()
            .run_slice(slice);
    loop {
        match outcome {
            SliceOutcome::Yielded { runner, .. } => outcome = runner.run_slice(slice),
            SliceOutcome::Completed(draft) => return Ok(draft),
            SliceOutcome::Faulted(fault) => return Err(fault),
        }
    }
}
