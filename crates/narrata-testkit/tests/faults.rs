#![allow(clippy::panic, clippy::unwrap_used)]

use std::sync::Arc;

use narrata_core::{
    ChoiceId, ExecutionId, FlowId, GlobalId, InputId, InstructionId, ProgramId, Value,
    limits::{MacrostepLimits, ProgramLoadLimits, RuntimeLimits},
    program::{
        BinaryOpV0, ChoiceArmV0, ConstIndex, FlowV0, GlobalDeclV0, InstructionRecordV0, OpV0,
        ProgramArtifactV0, ReturnModeV0, SlotRefV0, encode_program_artifact, load_program,
    },
    runtime::{CheckedRuntimeInput, RuntimeFault, SliceBudget, new_execution},
    snapshot::export_snapshot,
    version::{PROGRAM_FORMAT_V0, SEMANTICS_V0},
};
use narrata_testkit::backend::{BackendError, ConformanceBackend, NativeBackend};

#[test]
fn arithmetic_fault_matrix_is_deterministic_and_atomic() {
    for (op, left, right) in [
        (BinaryOpV0::Add, i64::MAX, 1),
        (BinaryOpV0::Sub, i64::MIN, 1),
        (BinaryOpV0::Mul, i64::MAX, 2),
        (BinaryOpV0::Div, 1, 0),
        (BinaryOpV0::Rem, 1, 0),
        (BinaryOpV0::Div, i64::MIN, -1),
        (BinaryOpV0::Rem, i64::MIN, -1),
    ] {
        let program = checked(binary_program(op, left, right));
        let parent = Arc::new(new_execution(&program, ExecutionId::from_u128(1)).unwrap());
        let before = export_snapshot(&parent).unwrap();
        let error = NativeBackend
            .transition(
                program,
                parent.clone(),
                CheckedRuntimeInput::start(InputId::from_u128(1)),
                Default::default(),
                SliceBudget::unlimited(),
            )
            .unwrap_err();
        assert_eq!(error, BackendError::Fault(RuntimeFault::Arithmetic));
        assert_eq!(export_snapshot(&parent).unwrap(), before);
    }
}

#[test]
fn instruction_allocation_and_stack_limits_are_typed_faults() {
    let program = checked(binary_program(BinaryOpV0::Add, 1, 2));
    let parent = Arc::new(new_execution(&program, ExecutionId::from_u128(2)).unwrap());
    let allocation = MacrostepLimits {
        max_logical_alloc_units: 0,
        ..MacrostepLimits::default()
    };
    assert_eq!(
        NativeBackend
            .transition(
                program.clone(),
                parent.clone(),
                CheckedRuntimeInput::start(InputId::from_u128(2)),
                allocation,
                SliceBudget::unlimited(),
            )
            .unwrap_err(),
        BackendError::Fault(RuntimeFault::LogicalAllocationLimit)
    );

    let stack = MacrostepLimits {
        runtime: RuntimeLimits {
            max_stack_depth: 1,
            ..RuntimeLimits::default()
        },
        ..MacrostepLimits::default()
    };
    assert_eq!(
        NativeBackend
            .transition(
                program,
                parent,
                CheckedRuntimeInput::start(InputId::from_u128(3)),
                stack,
                SliceBudget::unlimited(),
            )
            .unwrap_err(),
        BackendError::Fault(RuntimeFault::StackDepthLimit)
    );
}

#[test]
fn loop_choice_call_and_live_value_limits_have_distinct_faults() {
    assert_fault(
        infinite_jump_program(),
        MacrostepLimits {
            max_instructions: 5,
            ..MacrostepLimits::default()
        },
        RuntimeFault::InstructionLimit,
    );
    assert_fault(
        hidden_choice_program(),
        MacrostepLimits::default(),
        RuntimeFault::NoVisibleChoices,
    );
    assert_fault(
        recursive_call_program(),
        MacrostepLimits {
            max_calls: 0,
            ..MacrostepLimits::default()
        },
        RuntimeFault::CallLimit,
    );
    assert_fault(
        recursive_call_program(),
        MacrostepLimits {
            runtime: RuntimeLimits {
                max_call_depth: 2,
                ..RuntimeLimits::default()
            },
            ..MacrostepLimits::default()
        },
        RuntimeFault::CallDepthLimit,
    );
    assert_fault(
        binary_program(BinaryOpV0::Add, 1, 2),
        MacrostepLimits {
            runtime: RuntimeLimits {
                max_total_live_values: 0,
                ..RuntimeLimits::default()
            },
            ..MacrostepLimits::default()
        },
        RuntimeFault::LiveValueLimit,
    );
}

fn binary_program(op: BinaryOpV0, left: i64, right: i64) -> ProgramArtifactV0 {
    let flow = FlowId::from_u128(1);
    ProgramArtifactV0 {
        format_version: PROGRAM_FORMAT_V0,
        semantics_version: SEMANTICS_V0,
        program_id: ProgramId::from_u128(100),
        entry_flow: flow,
        constants: vec![Value::I64(left), Value::I64(right)],
        globals: Vec::new(),
        flows: vec![FlowV0 {
            id: flow,
            parameters: Vec::new(),
            locals: Vec::new(),
            return_kind: None,
            entry: InstructionId::from_u128(1),
            instructions: vec![
                instruction(
                    1,
                    OpV0::Const {
                        constant: ConstIndex(0),
                        next: InstructionId::from_u128(2),
                    },
                ),
                instruction(
                    2,
                    OpV0::Const {
                        constant: ConstIndex(1),
                        next: InstructionId::from_u128(3),
                    },
                ),
                instruction(
                    3,
                    OpV0::Binary {
                        op,
                        next: InstructionId::from_u128(4),
                    },
                ),
                instruction(
                    4,
                    OpV0::Finish {
                        value: ReturnModeV0::Stack,
                    },
                ),
            ],
        }],
        capabilities: Vec::new(),
        external_content: Vec::new(),
    }
}

fn instruction(id: u128, op: OpV0) -> InstructionRecordV0 {
    InstructionRecordV0 {
        id: InstructionId::from_u128(id),
        op,
    }
}

fn infinite_jump_program() -> ProgramArtifactV0 {
    bare_program(
        vec![instruction(
            1,
            OpV0::Jump {
                target: InstructionId::from_u128(1),
            },
        )],
        Vec::new(),
    )
}

fn hidden_choice_program() -> ProgramArtifactV0 {
    let hidden = GlobalId::from_u128(1);
    bare_program(
        vec![
            instruction(
                1,
                OpV0::Choice {
                    prompt: None,
                    choices: vec![ChoiceArmV0 {
                        id: ChoiceId::from_u128(1),
                        label: ConstIndex(0),
                        visible_if: Some(SlotRefV0::Global(hidden)),
                        target: InstructionId::from_u128(2),
                    }],
                },
            ),
            instruction(
                2,
                OpV0::Finish {
                    value: ReturnModeV0::None,
                },
            ),
        ],
        vec![GlobalDeclV0 {
            id: hidden,
            kind: narrata_core::ValueKindV0::Bool,
            default: Value::Bool(false),
        }],
    )
}

fn recursive_call_program() -> ProgramArtifactV0 {
    let entry = FlowId::from_u128(1);
    let recursive = FlowId::from_u128(2);
    ProgramArtifactV0 {
        format_version: PROGRAM_FORMAT_V0,
        semantics_version: SEMANTICS_V0,
        program_id: ProgramId::from_u128(102),
        entry_flow: entry,
        constants: Vec::new(),
        globals: Vec::new(),
        flows: vec![
            FlowV0 {
                id: entry,
                parameters: Vec::new(),
                locals: Vec::new(),
                return_kind: None,
                entry: InstructionId::from_u128(1),
                instructions: vec![
                    instruction(
                        1,
                        OpV0::Call {
                            flow: recursive,
                            argument_count: 0,
                            return_to: InstructionId::from_u128(2),
                        },
                    ),
                    instruction(
                        2,
                        OpV0::Finish {
                            value: ReturnModeV0::None,
                        },
                    ),
                ],
            },
            FlowV0 {
                id: recursive,
                parameters: Vec::new(),
                locals: Vec::new(),
                return_kind: None,
                entry: InstructionId::from_u128(10),
                instructions: vec![
                    instruction(
                        10,
                        OpV0::Call {
                            flow: recursive,
                            argument_count: 0,
                            return_to: InstructionId::from_u128(11),
                        },
                    ),
                    instruction(
                        11,
                        OpV0::Return {
                            value: ReturnModeV0::None,
                        },
                    ),
                ],
            },
        ],
        capabilities: Vec::new(),
        external_content: Vec::new(),
    }
}

fn bare_program(
    instructions: Vec<InstructionRecordV0>,
    globals: Vec<GlobalDeclV0>,
) -> ProgramArtifactV0 {
    let flow = FlowId::from_u128(1);
    ProgramArtifactV0 {
        format_version: PROGRAM_FORMAT_V0,
        semantics_version: SEMANTICS_V0,
        program_id: ProgramId::from_u128(101),
        entry_flow: flow,
        constants: vec![Value::from("hidden")],
        globals,
        flows: vec![FlowV0 {
            id: flow,
            parameters: Vec::new(),
            locals: Vec::new(),
            return_kind: None,
            entry: InstructionId::from_u128(1),
            instructions,
        }],
        capabilities: Vec::new(),
        external_content: Vec::new(),
    }
}

fn assert_fault(artifact: ProgramArtifactV0, limits: MacrostepLimits, expected: RuntimeFault) {
    let program = checked(artifact);
    let parent = Arc::new(new_execution(&program, ExecutionId::from_u128(9)).unwrap());
    let before = export_snapshot(&parent).unwrap();
    let error = NativeBackend
        .transition(
            program,
            parent.clone(),
            CheckedRuntimeInput::start(InputId::from_u128(9)),
            limits,
            SliceBudget::unlimited(),
        )
        .unwrap_err();
    assert_eq!(error, BackendError::Fault(expected));
    assert_eq!(export_snapshot(&parent).unwrap(), before);
}

fn checked(artifact: ProgramArtifactV0) -> Arc<narrata_core::CheckedProgram> {
    load_program(
        &encode_program_artifact(&artifact),
        &ProgramLoadLimits::default(),
    )
    .unwrap()
}
