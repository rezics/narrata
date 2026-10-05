#![allow(clippy::panic, clippy::unwrap_used)]

use std::sync::Arc;

use narrata_core::{
    ActionId, EventTypeId, ExecutionId, FlowId, GlobalId, InputId, InstructionId, ProgramId,
    RegionId, StateId, TransitionId, Value, ValueKindV0,
    limits::{MacrostepLimits, ProgramLoadLimits},
    program::{
        FlowV0, GlobalDeclV0, InstructionRecordV0, OpV0, ProgramArtifactV0, ReturnModeV0,
        validate_program,
    },
    runtime::{
        CheckedRuntimeInput, DraftResult, SliceBudget, SliceOutcome, TransitionDraft,
        begin_transition, new_execution,
    },
    snapshot::state_digest,
    statechart::{
        ActionRecordV0, GuardExprV0, RegionV0, StateKindV0, StateV0, StatechartActionV0,
        StatechartV0, TransitionKindV0, TransitionTargetV0, TransitionV0,
    },
    version::{PROGRAM_FORMAT_V0, SEMANTICS_V0},
};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn generated_compound_machines_match_simple_reference(
        state_count in 2_u8..9,
        events in prop::collection::vec(1_u8..8, 0..24),
    ) {
        let program = Arc::new(validate_program(
            linear_chart(state_count),
            &ProgramLoadLimits::default(),
        ).unwrap());
        let mut parent = Arc::new(new_execution(&program, ExecutionId::from_u128(900)).unwrap());
        let start = CheckedRuntimeInput::start(InputId::from_u128(1));
        let first = run(Arc::clone(&program), Arc::clone(&parent), start.clone(), SliceBudget::new(1).unwrap());
        let first_unlimited = run(Arc::clone(&program), parent, start, SliceBudget::unlimited());
        prop_assert_eq!(first.next_state_digest(), first_unlimited.next_state_digest());
        prop_assert_eq!(first.receipt(), first_unlimited.receipt());
        prop_assert_eq!(first.statechart_trace(), first_unlimited.statechart_trace());
        let mut current = 0_u8;
        prop_assert!(matches!(first.result(), DraftResult::StatechartStable(_)));
        parent = Arc::new(first.into_next_state());

        for (offset, raw_event) in events.iter().copied().enumerate() {
            if current == state_count.saturating_sub(1) {
                break;
            }
            let event = 1 + raw_event % state_count.saturating_sub(1);
            let input = CheckedRuntimeInput::event(
                InputId::from_u128((offset + 2) as u128),
                EventTypeId::from_u128(u128::from(event)),
            );
            let sliced = run(
                Arc::clone(&program),
                Arc::clone(&parent),
                input.clone(),
                SliceBudget::new(1 + (offset % 3) as u64).unwrap(),
            );
            let unlimited = run(
                Arc::clone(&program),
                Arc::clone(&parent),
                input,
                SliceBudget::unlimited(),
            );
            prop_assert_eq!(sliced.next_state_digest(), unlimited.next_state_digest());
            prop_assert_eq!(sliced.receipt(), unlimited.receipt());
            prop_assert_eq!(sliced.statechart_trace(), unlimited.statechart_trace());

            if event == current.saturating_add(1) {
                current = current.saturating_add(1);
            }
            let expected_score = Value::I64(i64::from(current));
            prop_assert_eq!(
                sliced.next_state().globals.get(&GlobalId::from_u128(900)),
                Some(&expected_score),
            );
            if current == state_count.saturating_sub(1) {
                prop_assert!(matches!(sliced.result(), DraftResult::Finished(Value::Null)));
            } else {
                let DraftResult::StatechartStable(view) = sliced.result() else {
                    prop_assert!(false, "reference expected stable configuration");
                    break;
                };
                prop_assert_eq!(
                    view.active.as_slice(),
                    &[StateId::from_u128(100 + u128::from(current))],
                );
            }
            parent = Arc::new(sliced.into_next_state());
        }
    }
}

fn linear_chart(state_count: u8) -> ProgramArtifactV0 {
    let entry_flow = FlowId::from_u128(900);
    let score = GlobalId::from_u128(900);
    let mut states = vec![StateV0 {
        id: StateId::from_u128(1),
        parent: None,
        region: None,
        kind: StateKindV0::Compound,
        on_entry: Vec::new(),
        on_exit: Vec::new(),
        completion_event: None,
        source_span: None,
    }];
    let mut transitions = Vec::new();
    for index in 0..state_count {
        states.push(StateV0 {
            id: StateId::from_u128(100 + u128::from(index)),
            parent: Some(StateId::from_u128(1)),
            region: Some(RegionId::from_u128(1)),
            kind: if index == state_count.saturating_sub(1) {
                StateKindV0::Final
            } else {
                StateKindV0::Atomic
            },
            on_entry: Vec::new(),
            on_exit: Vec::new(),
            completion_event: None,
            source_span: None,
        });
        if index < state_count.saturating_sub(1) {
            transitions.push(TransitionV0 {
                id: TransitionId::from_u128(100 + u128::from(index)),
                source: StateId::from_u128(100 + u128::from(index)),
                event: Some(EventTypeId::from_u128(1 + u128::from(index))),
                kind: TransitionKindV0::External,
                guard: GuardExprV0::Always,
                targets: vec![TransitionTargetV0::State(StateId::from_u128(
                    101 + u128::from(index),
                ))],
                actions: vec![ActionRecordV0 {
                    id: ActionId::from_u128(100 + u128::from(index)),
                    action: StatechartActionV0::Assign {
                        global: score,
                        value: Value::I64(i64::from(index.saturating_add(1))),
                    },
                    source_span: None,
                }],
                source_span: None,
            });
        }
    }
    ProgramArtifactV0 {
        format_version: PROGRAM_FORMAT_V0,
        semantics_version: SEMANTICS_V0,
        program_id: ProgramId::from_u128(900 + u128::from(state_count)),
        entry_flow,
        constants: Vec::new(),
        globals: vec![GlobalDeclV0 {
            id: score,
            kind: ValueKindV0::I64,
            default: Value::I64(0),
        }],
        flows: vec![FlowV0 {
            id: entry_flow,
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
            root: StateId::from_u128(1),
            events: (1..state_count)
                .map(|event| EventTypeId::from_u128(u128::from(event)))
                .collect(),
            states,
            regions: vec![RegionV0 {
                id: RegionId::from_u128(1),
                parent: StateId::from_u128(1),
                initial: StateId::from_u128(100),
                source_span: None,
            }],
            histories: Vec::new(),
            transitions,
            source_span: None,
        }),
    }
}

fn run(
    program: Arc<narrata_core::CheckedProgram>,
    parent: Arc<narrata_core::RuntimeStateV0>,
    input: CheckedRuntimeInput,
    slice: SliceBudget,
) -> TransitionDraft {
    let parent_digest = state_digest(&parent);
    let mut outcome = begin_transition(
        program,
        Arc::clone(&parent),
        input,
        MacrostepLimits::default(),
    )
    .unwrap()
    .run_slice(slice);
    loop {
        match outcome {
            SliceOutcome::Yielded { runner, .. } => outcome = runner.run_slice(slice),
            SliceOutcome::Completed(draft) => {
                assert_eq!(state_digest(&parent), parent_digest);
                return draft;
            }
            SliceOutcome::Faulted(fault) => panic!("unexpected generated-machine fault: {fault}"),
        }
    }
}
